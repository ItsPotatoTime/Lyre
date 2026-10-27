//! Directory scanning.
//!
//! The design constraint is that a folder with 100,000 entries must paint its
//! first screen within a frame or two of the user clicking it. That rules out
//! the obvious implementation (collect everything, sort, return) because it
//! defers all feedback until the slowest `stat` in the directory completes.
//!
//! What happens instead:
//! 1. `read_dir` enumerates names only. Names and file types come from the
//!    directory itself on all three platforms, so this pass costs roughly one
//!    syscall per *block* of entries, not per entry.
//! 2. The names are cut into chunks and handed to a fixed pool of workers that
//!    stat them in parallel.
//! 3. Each finished chunk is pushed to the webview immediately over an IPC
//!    channel, so the interface renders while the rest of the directory is
//!    still being read.
//!
//! Sorting is deliberately *not* done here. The interface owns the comparator
//! because the user can change it at any time by clicking a column, and one
//! comparator in one language cannot drift out of sync with itself.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Entries per parallel work unit. Small enough that the first batch lands
/// quickly, large enough that the thread hand-off cost stays negligible.
const CHUNK: usize = 512;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    pub path: String,
    #[serde(default)]
    pub show_hidden: bool,
}

/// One row in the file list. Field names are camelCase on the wire.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Bytes. Always 0 for directories; callers must not display it as a size.
    pub size: u64,
    /// Milliseconds since the Unix epoch, if the platform could supply it.
    pub modified_ms: Option<i64>,
    /// Drives both the glyph shown and the "Type" column.
    pub kind: &'static str,
    pub ext: String,
    pub hidden: bool,
    /// False when the entry was listed but could not be stat'ed (a dangling
    /// symlink, a permission wall, a race with a delete).
    pub readable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ScanEvent {
    /// A partial result. `scanned` is the running total, which the status bar
    /// shows climbing while the rest of the directory arrives.
    Batch { entries: Vec<Entry>, scanned: usize },
    Done { total: usize, elapsed_ms: u64 },
    Failed { message: String },
}

/// A name read from the directory, before `stat`.
struct Raw {
    name: String,
    path: PathBuf,
    is_dir: bool,
    is_symlink: bool,
}

/// Walk `path` and hand every batch to `emit`.
///
/// The sink is a plain closure rather than a Tauri channel so the scanner can
/// be exercised without a window: `emit` returning false means the consumer
/// went away and the scan should stop.
pub fn scan<F>(path: &Path, show_hidden: bool, cancel: &Arc<AtomicBool>, mut emit: F)
where
    F: FnMut(ScanEvent) -> bool,
{
    let started = Instant::now();

    let read = match fs::read_dir(path) {
        Ok(r) => r,
        Err(err) => {
            emit(ScanEvent::Failed {
                message: describe(&err),
            });
            return;
        }
    };

    let mut raw: Vec<Raw> = Vec::with_capacity(64);
    for item in read {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        if !show_hidden && is_hidden_name(&name) {
            continue;
        }
        // `file_type` is free on Unix (d_type) and on Windows (find data); on
        // macOS it falls back to a stat only for filesystems without d_type.
        let (is_dir, is_symlink) = match item.file_type() {
            Ok(t) => (t.is_dir(), t.is_symlink()),
            Err(_) => (false, false),
        };
        raw.push(Raw {
            name,
            // Rebuild the path from the parent rather than calling `path()`,
            // which allocates the same string but goes through an OsString
            // copy on Windows.
            path: path.join(item.file_name()),
            is_dir,
            is_symlink,
        });
    }

    let total = raw.len();
    if total == 0 {
        emit(ScanEvent::Done {
            total: 0,
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
        return;
    }

    let chunks = total.div_ceil(CHUNK);
    let workers = worker_count().min(chunks);
    let shared = Arc::new(raw);
    let next = AtomicUsize::new(0);
    let (ready_tx, ready_rx) = mpsc::channel::<Vec<Entry>>();

    thread::scope(|scope| {
        for _ in 0..workers {
            let ready_tx = ready_tx.clone();
            let shared = Arc::clone(&shared);
            let next = &next;
            scope.spawn(move || loop {
                // `index` counts chunks, not entries.
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= chunks {
                    break;
                }
                let start = index * CHUNK;
                let end = (start + CHUNK).min(total);
                let mut batch = Vec::with_capacity(end - start);
                for item in &shared[start..end] {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(entry) = stat(item) {
                        batch.push(entry);
                    }
                }
                if ready_tx.send(batch).is_err() {
                    return;
                }
            });
        }
        drop(ready_tx);

        let mut scanned = 0usize;
        for batch in ready_rx {
            scanned += batch.len();
            if !emit(ScanEvent::Batch {
                entries: batch,
                scanned,
            }) {
                // The window went away; stop paying for the remaining chunks.
                cancel.store(true, Ordering::Relaxed);
                break;
            }
        }

        if !cancel.load(Ordering::Relaxed) {
            emit(ScanEvent::Done {
                total: scanned,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
    });
}

fn stat(item: &Raw) -> Option<Entry> {
    let meta = fs::symlink_metadata(&item.path).ok();

    // A symlink's own metadata describes the link, which is never what the
    // user wants to see: display the target's size and type when it resolves.
    let target = if item.is_symlink {
        fs::metadata(&item.path).ok()
    } else {
        None
    };

    let effective = target.as_ref().or(meta.as_ref());
    let is_dir = effective.map(|m| m.is_dir()).unwrap_or(item.is_dir);
    let size = match effective {
        Some(m) if !is_dir => m.len(),
        _ => 0,
    };
    let modified_ms = effective
        .and_then(|m| m.modified().ok())
        .and_then(to_epoch_ms);

    let ext = item
        .name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();

    Some(Entry {
        name: item.name.clone(),
        path: item.path.to_string_lossy().into_owned(),
        is_dir,
        is_symlink: item.is_symlink,
        size,
        modified_ms,
        kind: if is_dir {
            "folder"
        } else if item.is_symlink {
            "shortcut"
        } else {
            classify(&ext)
        },
        ext,
        hidden: is_hidden_name(&item.name) || meta.as_ref().map(is_hidden_meta).unwrap_or(false),
        readable: meta.is_some(),
    })
}

/// Milliseconds since the epoch. Uses `duration_since` against the epoch and
/// falls back to `None` rather than panicking on clocks before 1970.
fn to_epoch_ms(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => Some(d.as_millis() as i64),
        Err(err) => Some(-(err.duration().as_millis() as i64)),
    }
}

fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(windows)]
fn is_hidden_meta(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn is_hidden_meta(_meta: &fs::Metadata) -> bool {
    false
}

/// Map an extension to a glyph family. This runs once per entry and allocates
/// nothing, which matters: it is the only per-file work besides the `stat`.
fn classify(ext: &str) -> &'static str {
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tif" | "tiff" | "avif"
        | "heic" | "raw" | "psd" => "image",
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "m4v" | "mpg" | "mpeg" => "video",
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" | "aiff" => "audio",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "iso" | "dmg" | "tgz" => {
            "archive"
        }
        "pdf" => "pdf",
        "doc" | "docx" | "odt" | "rtf" | "pages" => "document",
        "xls" | "xlsx" | "csv" | "ods" | "tsv" | "numbers" => "spreadsheet",
        "ppt" | "pptx" | "odp" | "key" => "presentation",
        "txt" | "md" | "markdown" | "log" | "rst" | "org" | "tex" => "text",
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "go" | "c" | "h" | "cpp"
        | "hpp" | "cc" | "java" | "kt" | "swift" | "rb" | "php" | "cs" | "sh" | "bash" | "zsh"
        | "fish" | "lua" | "sql" | "html" | "htm" | "css" | "scss" | "sass" | "less" | "vue"
        | "svelte" | "json" | "yaml" | "yml" | "toml" | "xml" | "ini" | "conf" | "env" | "lock"
        | "gradle" | "cmake" | "make" | "mk" | "dockerfile" | "gitignore" => "code",
        "exe" | "msi" | "app" | "deb" | "rpm" | "apk" | "appimage" | "bat" | "cmd" | "com"
        | "pkg" | "snap" | "flatpak" => "executable",
        "ttf" | "otf" | "woff" | "woff2" | "ttc" => "font",
        "db" | "sqlite" | "sqlite3" | "mdb" | "parquet" | "duckdb" => "database",
        "dll" | "so" | "dylib" | "a" | "lib" | "o" | "obj" | "class" | "pyc" | "wasm" => "binary",
        "lnk" | "desktop" | "url" | "alias" | "webloc" => "shortcut",
        _ => "file",
    }
}

fn worker_count() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        // Two per core: `stat` is IO-bound more often than it is CPU-bound.
        .map(|n| (n * 2).min(32))
        .unwrap_or(4)
}

fn describe(err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::NotFound => "That folder no longer exists.".into(),
        std::io::ErrorKind::PermissionDenied => "You do not have permission to open that folder.".into(),
        _ => err.to_string(),
    }
}
