//! Search.
//!
//! The requirement is that typing feels like filtering, not like querying: a
//! result for a folder with tens of thousands of entries has to appear inside
//! a frame, and a search that walks subfolders must never make the interface
//! wait for it.
//!
//! Three things make that possible.
//!
//! 1. **The folder on screen is already in memory.** Every scan publishes its
//!    entries to an index keyed by path, so the first phase of a search is a
//!    pure memory scan with no syscalls at all.
//! 2. **Names are matched, not metadata.** Walking subfolders reads directory
//!    entries and matches the file name; only the items that match are
//!    stat'ed. A recursive search therefore costs about one `readdir` per
//!    folder instead of one `stat` per file.
//! 3. **The first phase is emitted before the second begins.** Local results
//!    arrive in single-digit milliseconds and matches from deeper in the tree
//!    stream in behind them. A keystroke cancels the work it started.
//!
//! Matching is `nucleo-matcher`, the engine behind Helix: smart case,
//! subsequence matching with a score that rewards contiguous runs and word
//! boundaries, and the match positions so the interface can highlight them.

use crate::scan::{self, Entry};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Results handed to the interface. Past this the list is noise, and every
/// extra row is bytes across the IPC boundary on every keystroke.
const LIMIT: usize = 1000;
/// Ceiling on entries examined by a recursive walk. The walker reads a few
/// million names a second, so this is a guard against a pathological tree
/// rather than a normal stopping point; the time budget below usually binds
/// first on slow disks, and this one on fast ones.
const MAX_SCANNED: usize = 5_000_000;
/// Wall-clock budget for the walk. Results have already been streaming for
/// this long by the time it applies, so the interface is never waiting.
const TIME_BUDGET_MS: u128 = 3000;
const MAX_DEPTH: usize = 32;
/// How long the walker may accumulate before flushing a batch.
const BATCH_MS: u64 = 40;
const BATCH_ITEMS: usize = 24;
/// Folders kept in the index: enough for a browsing session's history.
const INDEX_CAPACITY: usize = 24;
/// Rows held across all cached folders. Roughly 60 bytes each after compaction,
/// so this bounds the cache at a few tens of megabytes however it was filled.
const INDEX_MAX_ENTRIES: usize = 200_000;
const CHUNK: usize = 2048;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    /// The folder on screen. Empty for a place that is not a directory, such
    /// as This PC; the search then begins straight at the roots.
    pub path: String,
    /// Where to walk. Home supplies the home folder, This PC supplies every
    /// volume. Empty means "just `path`", which is the ordinary folder case.
    #[serde(default)]
    pub roots: Vec<String>,
    pub query: String,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub show_hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    #[serde(flatten)]
    pub entry: Entry,
    pub score: u32,
    /// Match positions as character offsets, for highlighting. Sorted.
    pub indices: Vec<u32>,
}

/// What the cache keeps per file: enough to match and rank, not enough to
/// display.
///
/// A full [`Entry`] carries its absolute path and its extension, both of which
/// are derived from the folder and the name it is already holding. For a folder
/// of twenty thousand files that is two extra heap strings per row, which is
/// most of what the cache costs. The display form is rebuilt only for the rows
/// that actually match.
#[derive(Debug, Clone)]
pub struct CompactEntry {
    pub name: Box<str>,
    pub size: u64,
    pub modified_ms: Option<i64>,
    pub kind: &'static str,
    /// is_dir | is_symlink << 1 | hidden << 2 | readable << 3
    flags: u8,
}

impl CompactEntry {
    pub fn is_dir(&self) -> bool {
        self.flags & 1 != 0
    }

    /// Rebuild the row the interface renders, from the folder it lives in.
    pub fn to_entry(&self, folder: &Path) -> Entry {
        let name = self.name.to_string();
        let ext = match name.rfind('.') {
            Some(index) if index > 0 && index + 1 < name.len() => {
                name[index + 1..].to_ascii_lowercase()
            }
            _ => String::new(),
        };
        Entry {
            path: folder.join(&name).to_string_lossy().into_owned(),
            ext,
            is_dir: self.is_dir(),
            is_symlink: self.flags & 2 != 0,
            hidden: self.flags & 4 != 0,
            readable: self.flags & 8 != 0,
            name,
            size: self.size,
            modified_ms: self.modified_ms,
            kind: self.kind,
        }
    }
}

impl From<&Entry> for CompactEntry {
    fn from(entry: &Entry) -> Self {
        let mut flags = 0u8;
        if entry.is_dir {
            flags |= 1;
        }
        if entry.is_symlink {
            flags |= 2;
        }
        if entry.hidden {
            flags |= 4;
        }
        if entry.readable {
            flags |= 8;
        }
        CompactEntry {
            name: entry.name.as_str().into(),
            size: entry.size,
            modified_ms: entry.modified_ms,
            kind: entry.kind,
            flags,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SearchEvent {
    /// Results from the folder on screen. Emitted first, always.
    Local {
        items: Vec<Hit>,
        matched: usize,
        elapsed_ms: u64,
    },
    /// Results found deeper in the tree, as they arrive.
    Batch { items: Vec<Hit>, scanned: usize },
    Done {
        matched: usize,
        scanned: usize,
        elapsed_ms: u64,
        truncated: bool,
    },
    Failed {
        message: String,
    },
}

/// Entries published by the most recent scan of each folder.
#[derive(Default)]
pub struct Index {
    state: Mutex<IndexState>,
}

#[derive(Default)]
struct IndexState {
    map: HashMap<PathBuf, Arc<Vec<CompactEntry>>>,
    order: VecDeque<PathBuf>,
    /// Total rows held. The folder cap alone let a handful of huge folders
    /// pin hundreds of megabytes while the window sat idle.
    entries: usize,
}

impl Index {
    pub fn store(&self, path: &Path, entries: Arc<Vec<CompactEntry>>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let added = entries.len();
        if let Some(previous) = state.map.insert(path.to_path_buf(), Arc::clone(&entries)) {
            // Replacing a cached folder must not count it twice.
            state.entries = state.entries.saturating_sub(previous.len());
            if let Some(position) = state.order.iter().position(|item| item == path) {
                state.order.remove(position);
            }
        }
        state.order.push_back(path.to_path_buf());
        state.entries += added;

        while state.order.len() > INDEX_CAPACITY || state.entries > INDEX_MAX_ENTRIES {
            if let Some(oldest) = state.order.pop_front() {
                if let Some(dropped) = state.map.remove(&oldest) {
                    state.entries = state.entries.saturating_sub(dropped.len());
                }
            } else {
                break;
            }
        }
    }

    pub fn get(&self, path: &Path) -> Option<Arc<Vec<CompactEntry>>> {
        self.state.lock().ok()?.map.get(path).cloned()
    }

    /// Drop a folder's cached listing after something changed on disk, so a
    /// search cannot answer from rows that no longer exist.
    pub fn forget(&self, path: &Path) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if let Some(dropped) = state.map.remove(path) {
            state.entries = state.entries.saturating_sub(dropped.len());
        }
        state.order.retain(|entry| entry != path);
    }
}

/// Run a search, emitting results as they are known.
///
/// `emit` returns false when the consumer has gone away, which also stops the
/// walk. The function is synchronous; the caller owns the thread.
pub fn search<F>(index: &Index, request: &SearchRequest, cancel: &Arc<AtomicBool>, mut emit: F)
where
    F: FnMut(SearchEvent) -> bool,
{
    let started = Instant::now();
    let needle = request.query.trim();

    if needle.is_empty() {
        let _ = emit(SearchEvent::Failed {
            message: "Type something to search for.".into(),
        });
        return;
    }

    // Always case-insensitive. Smart case would make "REPORT" mean *only*
    // upper-case names, which is a trap for anyone with caps lock on or who is
    // used to Windows search matching regardless of case.
    let pattern = Pattern::parse(needle, CaseMatching::Ignore, Normalization::Smart);

    let roots: Vec<PathBuf> = if request.roots.is_empty() {
        if request.path.is_empty() {
            Vec::new()
        } else {
            vec![PathBuf::from(&request.path)]
        }
    } else {
        request.roots.iter().map(PathBuf::from).collect()
    };
    let local_path: Option<PathBuf> = if request.path.is_empty() {
        None
    } else {
        Some(PathBuf::from(&request.path))
    };

    // Phase one: the folder the user is looking at, straight out of memory.
    // A place like This PC has no listing of its own, so this phase is simply
    // empty and the walk carries the whole answer.
    let (local, matched_local, laid_out) = match &local_path {
        Some(path) => {
            let entries = match index.get(path) {
                Some(cached) => cached,
                None => {
                    let collected = collect(path, request.show_hidden, cancel);
                    let compact: Vec<CompactEntry> =
                        collected.iter().map(CompactEntry::from).collect();
                    let shared = Arc::new(compact);
                    index.store(path, Arc::clone(&shared));
                    shared
                }
            };
            let (hits, matched) = score_local(&pattern, &entries, path, LIMIT);
            (hits, matched, entries.len())
        }
        None => (Vec::new(), 0, 0),
    };
    let alive = emit(SearchEvent::Local {
        items: local,
        matched: matched_local,
        elapsed_ms: started.elapsed().as_millis() as u64,
    });
    if !alive {
        return;
    }

    // A single character matches almost everything, and walking a tree for it
    // costs far more than the answer is worth.
    if !request.recursive || needle.chars().count() < 2 {
        let _ = emit(SearchEvent::Done {
            matched: matched_local,
            scanned: laid_out,
            elapsed_ms: started.elapsed().as_millis() as u64,
            truncated: matched_local > LIMIT,
        });
        return;
    }

    // The list is already full from this folder alone; deeper matches could
    // never be shown, so the tree is not walked at all.
    if matched_local >= LIMIT {
        let _ = emit(SearchEvent::Done {
            matched: matched_local,
            scanned: laid_out,
            elapsed_ms: started.elapsed().as_millis() as u64,
            truncated: true,
        });
        return;
    }

    let outcome = walk(
        Walk {
            roots: &roots,
            covered: local_path.as_deref(),
            pattern: &pattern,
            show_hidden: request.show_hidden,
            cancel,
            matched_so_far: matched_local,
            scanned_so_far: laid_out,
        },
        &mut emit,
    );

    let _ = emit(SearchEvent::Done {
        matched: outcome.matched,
        scanned: outcome.scanned,
        elapsed_ms: started.elapsed().as_millis() as u64,
        truncated: outcome.truncated,
    });
}

/// Scan a folder synchronously when the index has nothing for it yet. Rare:
/// the interface scans a folder before showing it, so the cache is warm.
fn collect(path: &Path, show_hidden: bool, cancel: &Arc<AtomicBool>) -> Vec<Entry> {
    let mut out = Vec::new();
    scan::scan(path, show_hidden, cancel, |event| {
        if let scan::ScanEvent::Batch { entries, .. } = event {
            out.extend(entries);
        }
        !cancel.load(Ordering::Relaxed)
    });
    out
}

/// Score one folder's entries in parallel and keep the best `limit`.
///
/// Returns the hits and the total number of matches, which the status bar
/// reports even when the list itself is capped.
fn score_local(
    pattern: &Pattern,
    entries: &[CompactEntry],
    folder: &Path,
    limit: usize,
) -> (Vec<Hit>, usize) {
    if entries.is_empty() {
        return (Vec::new(), 0);
    }

    let workers = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(entries.len().div_ceil(CHUNK).max(1));

    let collected: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());

    thread::scope(|scope| {
        for worker in 0..workers {
            let collected = &collected;
            scope.spawn(move || {
                let mut matcher = Matcher::new(Config::DEFAULT);
                let mut buf = Vec::new();
                let mut mine: Vec<(u32, u32)> = Vec::new();
                let mut index = worker * CHUNK;
                while index < entries.len() {
                    let end = (index + CHUNK).min(entries.len());
                    for (offset, entry) in entries[index..end].iter().enumerate() {
                        if let Some(score) =
                            pattern.score(Utf32Str::new(&entry.name, &mut buf), &mut matcher)
                        {
                            mine.push((score, (index + offset) as u32));
                        }
                    }
                    // Strided chunks: every worker keeps scanning to the end.
                    index += CHUNK * workers;
                }
                if let Ok(mut all) = collected.lock() {
                    all.extend(mine);
                }
            });
        }
    });

    let mut scored = collected.into_inner().unwrap_or_default();
    let matched = scored.len();
    rank(&mut scored, entries, limit);
    (materialise(&scored, entries, folder, pattern), matched)
}

/// Best score first, then the shorter name, then alphabetical. A short name
/// that matches is a stronger signal than a long one containing the same
/// letters by coincidence.
///
/// A packed folder produces tens of thousands of ties (a two-letter query
/// matches almost everything), so this comparator is the hot loop: it never
/// allocates, and the list is cut to `limit` before it is fully sorted.
fn rank(scored: &mut Vec<(u32, u32)>, entries: &[CompactEntry], limit: usize) {
    let compare = |a: &(u32, u32), b: &(u32, u32)| {
        let left = &entries[a.1 as usize].name;
        let right = &entries[b.1 as usize].name;
        b.0.cmp(&a.0)
            .then_with(|| left.len().cmp(&right.len()))
            .then_with(|| compare_ignore_case(left, right))
    };

    if scored.len() > limit {
        // Only the best `limit` are ever shown, so find them in linear time
        // and sort just those.
        scored.select_nth_unstable_by(limit, compare);
        scored.truncate(limit);
    }
    scored.sort_unstable_by(compare);
}

/// Case-insensitive comparison over bytes.
///
/// This is a tie-break that runs tens of thousands of times on a single query,
/// so it is byte-wise rather than character-wise: an iterator chain over
/// `char::to_lowercase` costs about 700ns per comparison here, against roughly
/// 40ns for this. Non-ASCII names therefore sort by byte order among equally
/// scored matches, which no one can see because the primary key is the score.
fn compare_ignore_case(a: &str, b: &str) -> std::cmp::Ordering {
    let left = a.as_bytes();
    let right = b.as_bytes();
    let shared = left.len().min(right.len());
    for i in 0..shared {
        let x = left[i].to_ascii_lowercase();
        let y = right[i].to_ascii_lowercase();
        if x != y {
            return x.cmp(&y);
        }
    }
    left.len().cmp(&right.len())
}

/// Turn scored pairs into hits, computing match positions only for the ones
/// that survive the cut.
fn materialise(
    scored: &[(u32, u32)],
    entries: &[CompactEntry],
    folder: &Path,
    pattern: &Pattern,
) -> Vec<Hit> {
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    scored
        .iter()
        .filter_map(|&(score, index)| {
            let entry = entries.get(index as usize)?;
            let mut indices = Vec::new();
            pattern.indices(
                Utf32Str::new(&entry.name, &mut buf),
                &mut matcher,
                &mut indices,
            );
            indices.sort_unstable();
            indices.dedup();
            Some(Hit {
                entry: entry.to_entry(folder),
                score,
                indices,
            })
        })
        .collect()
}

struct Outcome {
    matched: usize,
    scanned: usize,
    truncated: bool,
}

/// Everything a walk needs, in one place so the call site reads as a sentence.
struct Walk<'a> {
    roots: &'a [PathBuf],
    /// The folder phase one already listed; its own entries are not re-matched.
    covered: Option<&'a Path>,
    pattern: &'a Pattern,
    show_hidden: bool,
    cancel: &'a Arc<AtomicBool>,
    matched_so_far: usize,
    scanned_so_far: usize,
}

/// Walk the tree below `root`, matching names as they are read.
///
/// Workers share one stack of pending folders and one channel, so a wide tree
/// divides rather than being walked in sequence. The receiving side only
/// assembles batches; it never touches the disk.
fn walk<F>(walk: Walk<'_>, emit: &mut F) -> Outcome
where
    F: FnMut(SearchEvent) -> bool,
{
    let Walk {
        roots,
        covered,
        pattern,
        show_hidden,
        cancel,
        matched_so_far,
        scanned_so_far,
    } = walk;

    // Every root is queued at depth zero. A root that phase one already listed
    // is walked for its subfolders only, so its own entries are not read, and
    // counted, twice.
    // Breadth-first, not depth-first. A depth-first walk of a whole volume
    // disappears into the first large system tree it meets (/opt, /usr) and
    // spends the entire budget there before reaching the user's own folders.
    // Taking folders from the front spreads the walk across the top of the
    // tree first, which is where the user's files are.
    // A volume list usually contains folders that live inside other volumes
    // (/ and /home, C:\ and C:\Users), so walking every root as given examines
    // the same files twice.
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    for root in without_nested(roots) {
        if !is_virtual_filesystem(&root) {
            queue.push_back((root, 0));
        }
    }
    let queue: Mutex<VecDeque<(PathBuf, usize)>> = Mutex::new(queue);
    let examined = AtomicUsize::new(scanned_so_far);
    let (tx, rx) = mpsc::channel::<Hit>();
    let workers = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(12);

    let mut accepted = 0usize;
    let walk_started = Instant::now();
    // Matches are counted by every worker, but only the first `LIMIT` are
    // turned into rows. Describing a file costs a `stat` and two allocations;
    // doing that for a query like "libc" across a home folder meant 43,000 of
    // them to display 1,000, and that is where four seconds went.
    let matched = AtomicUsize::new(matched_so_far);
    // Set when the scan ceiling stopped the walk, so `Done` can say the answer
    // is incomplete rather than implying the tree was fully searched.
    let mut cut = false;

    thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let queue = &queue;
            let examined = &examined;
            let matched = &matched;
            scope.spawn(move || {
                let mut matcher = Matcher::new(Config::DEFAULT);
                let mut buf = Vec::new();
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    // Several folders at a time, so the lock is not the
                    // bottleneck on a wide tree.
                    let next = {
                        let Ok(mut queue) = queue.lock() else { return };
                        let mut batch = Vec::with_capacity(4);
                        for _ in 0..4 {
                            match queue.pop_front() {
                                Some(item) => batch.push(item),
                                None => break,
                            }
                        }
                        batch
                    };
                    if next.is_empty() {
                        return;
                    }

                    for (dir, depth) in next {
                        // Phase one already answered for this folder's own
                        // entries; only its children are still unexamined.
                        if covered.is_some_and(|path| path == dir) {
                            let children = subfolders(&dir, show_hidden, depth);
                            if !children.is_empty() {
                                if let Ok(mut queue) = queue.lock() {
                                    queue.extend(children);
                                }
                            }
                            continue;
                        }

                        let Ok(read) = fs::read_dir(&dir) else {
                            continue;
                        };
                        let mut subdirs: Vec<(PathBuf, usize)> = Vec::new();
                        for item in read {
                            if cancel.load(Ordering::Relaxed) {
                                return;
                            }
                            let Ok(item) = item else { continue };
                            examined.fetch_add(1, Ordering::Relaxed);
                            let name = item.file_name().to_string_lossy().into_owned();
                            if !show_hidden && name.starts_with('.') {
                                continue;
                            }
                            let file_type = item.file_type().ok();
                            let is_dir = file_type.as_ref().map(|t| t.is_dir()).unwrap_or(false);
                            let is_symlink =
                                file_type.as_ref().map(|t| t.is_symlink()).unwrap_or(false);

                            if is_dir {
                                // A link can point at its own parent, so linked
                                // folders are never followed.
                                if !is_symlink && depth < MAX_DEPTH {
                                    subdirs.push((item.path(), depth + 1));
                                }
                                continue;
                            }

                            let Some(score) =
                                pattern.score(Utf32Str::new(&name, &mut buf), &mut matcher)
                            else {
                                continue;
                            };

                            // Counted first, so the total stays exact even once
                            // the answer is full and rows stop being built.
                            if matched.fetch_add(1, Ordering::Relaxed) >= LIMIT {
                                continue;
                            }

                            // Metadata is paid for only here, on a match.
                            let Some(entry) =
                                scan::describe_entry(name, item.path(), is_dir, is_symlink)
                            else {
                                continue;
                            };
                            let mut indices = Vec::new();
                            pattern.indices(
                                Utf32Str::new(&entry.name, &mut buf),
                                &mut matcher,
                                &mut indices,
                            );
                            indices.sort_unstable();
                            indices.dedup();

                            if tx
                                .send(Hit {
                                    entry,
                                    score,
                                    indices,
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                        if !subdirs.is_empty() {
                            if let Ok(mut queue) = queue.lock() {
                                queue.extend(subdirs);
                            }
                        }
                    }
                }
            });
        }
        drop(tx);

        let mut batch: Vec<Hit> = Vec::new();
        let mut last_flush = Instant::now();

        for hit in rx {
            if matched_so_far + accepted < LIMIT {
                batch.push(hit);
                accepted += 1;
            }

            let scanned = examined.load(Ordering::Relaxed);
            let due = batch.len() >= BATCH_ITEMS
                || last_flush.elapsed() >= Duration::from_millis(BATCH_MS);
            if due && !batch.is_empty() {
                let alive = emit(SearchEvent::Batch {
                    items: std::mem::take(&mut batch),
                    scanned,
                });
                last_flush = Instant::now();
                if !alive {
                    cancel.store(true, Ordering::Relaxed);
                    break;
                }
            }

            if scanned >= MAX_SCANNED {
                cut = true;
                cancel.store(true, Ordering::Relaxed);
                break;
            }

            if walk_started.elapsed().as_millis() >= TIME_BUDGET_MS {
                cut = true;
                cancel.store(true, Ordering::Relaxed);
                break;
            }
        }

        if !batch.is_empty() {
            let _ = emit(SearchEvent::Batch {
                items: batch,
                scanned: examined.load(Ordering::Relaxed),
            });
        }
    });

    Outcome {
        matched: matched.load(Ordering::Relaxed),
        scanned: examined.load(Ordering::Relaxed),
        // Truncated when there were more matches than rows shown, or when the
        // ceiling stopped the walk before the tree was finished.
        truncated: cut || matched.load(Ordering::Relaxed) > matched_so_far + accepted,
    }
}

/// Drop any root that another root already contains. Sorted by depth first, so
/// a parent is always seen before its children.
fn without_nested(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut ordered: Vec<&PathBuf> = roots.iter().collect();
    ordered.sort_by_key(|path| path.components().count());

    let mut kept: Vec<PathBuf> = Vec::with_capacity(ordered.len());
    for root in ordered {
        if kept.iter().any(|parent| root.starts_with(parent)) {
            continue;
        }
        kept.push(root.clone());
    }
    kept
}

/// Filesystems that are not real storage. Walking them is slow and their
/// contents are kernel state, not the user's files.
fn is_virtual_filesystem(path: &Path) -> bool {
    #[cfg(unix)]
    {
        matches!(
            path.to_str(),
            Some("/proc") | Some("/sys") | Some("/dev") | Some("/run")
        )
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Immediate children of a folder that are folders, for the walk to start on.
fn subfolders(dir: &Path, show_hidden: bool, depth: usize) -> Vec<(PathBuf, usize)> {
    let mut out = Vec::new();
    if depth >= MAX_DEPTH {
        return out;
    }
    let Ok(read) = fs::read_dir(dir) else {
        return out;
    };
    for item in read.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        if file_type.is_dir() && !file_type.is_symlink() {
            out.push((item.path(), depth + 1));
        }
    }
    out
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    fn entry(i: usize, folder: &str) -> Entry {
        Entry {
            name: format!("file_{i:05}.txt"),
            path: format!("{folder}/file_{i:05}.txt"),
            is_dir: false,
            is_symlink: false,
            size: i as u64,
            modified_ms: Some(1_700_000_000_000),
            kind: "Text document",
            ext: "txt".into(),
            hidden: false,
            readable: true,
        }
    }

    /// What the cache costs per row, in both representations, measured rather
    /// than estimated. The compact form exists because the full one carries an
    /// absolute path and an extension per row that are both derivable.
    #[test]
    fn the_compact_form_costs_far_less_per_row() {
        const ROWS: usize = 20_000;
        let folder = "/home/someone/Downloads";

        let before = crate::counting_alloc::live_bytes();
        let full: Vec<Entry> = (0..ROWS).map(|i| entry(i, folder)).collect();
        let full_bytes = crate::counting_alloc::live_bytes() - before;
        let compact: Vec<CompactEntry> = full.iter().map(CompactEntry::from).collect();
        let both_bytes = crate::counting_alloc::live_bytes() - before;
        let compact_bytes = both_bytes - full_bytes;
        drop(full);

        println!(
            "20k rows: full {:.1} MB, compact {:.1} MB ({:.0}% smaller), {:.0} B/row vs {:.0} B/row",
            full_bytes as f64 / 1e6,
            compact_bytes as f64 / 1e6,
            100.0 * (1.0 - compact_bytes as f64 / full_bytes as f64),
            full_bytes as f64 / ROWS as f64,
            compact_bytes as f64 / ROWS as f64,
        );

        assert!(
            compact_bytes * 2 < full_bytes,
            "compaction should at least halve the cost: {compact_bytes} vs {full_bytes}"
        );
        drop(compact);
    }

    /// The rebuilt row must equal the one the scan produced, or the saving is
    /// being paid for with wrong data.
    #[test]
    fn a_rebuilt_row_matches_the_original() {
        let original = entry(7, "/home/someone/Downloads");
        let rebuilt = CompactEntry::from(&original)
            .to_entry(Path::new("/home/someone/Downloads"));
        assert_eq!(rebuilt.name, original.name);
        assert_eq!(rebuilt.path, original.path);
        assert_eq!(rebuilt.ext, original.ext);
        assert_eq!(rebuilt.size, original.size);
        assert_eq!(rebuilt.kind, original.kind);
        assert_eq!(rebuilt.modified_ms, original.modified_ms);
        assert_eq!(rebuilt.readable, original.readable);
    }
}
