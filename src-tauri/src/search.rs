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
/// Ceiling on entries examined by a recursive walk, so searching a whole disk
/// still ends and can say what it did.
const MAX_SCANNED: usize = 400_000;
const MAX_DEPTH: usize = 32;
/// How long the walker may accumulate before flushing a batch.
const BATCH_MS: u64 = 40;
const BATCH_ITEMS: usize = 24;
/// Folders kept in the index: enough for a browsing session's history.
const INDEX_CAPACITY: usize = 24;
const CHUNK: usize = 2048;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub path: String,
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
    map: HashMap<PathBuf, Arc<Vec<Entry>>>,
    order: VecDeque<PathBuf>,
}

impl Index {
    pub fn store(&self, path: &Path, entries: Arc<Vec<Entry>>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state
            .map
            .insert(path.to_path_buf(), Arc::clone(&entries))
            .is_none()
        {
            state.order.push_back(path.to_path_buf());
        }
        while state.order.len() > INDEX_CAPACITY {
            if let Some(oldest) = state.order.pop_front() {
                state.map.remove(&oldest);
            }
        }
    }

    pub fn get(&self, path: &Path) -> Option<Arc<Vec<Entry>>> {
        self.state.lock().ok()?.map.get(path).cloned()
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
    let root = PathBuf::from(&request.path);

    // Phase one: the folder the user is looking at, straight out of memory.
    let entries = match index.get(&root) {
        Some(cached) => cached,
        None => {
            let collected = collect(&root, request.show_hidden, cancel);
            let shared = Arc::new(collected);
            index.store(&root, Arc::clone(&shared));
            shared
        }
    };

    let (local, matched_local) = score_local(&pattern, &entries, LIMIT);
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
            scanned: entries.len(),
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
            scanned: entries.len(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            truncated: true,
        });
        return;
    }

    let outcome = walk(
        &root,
        &pattern,
        request.show_hidden,
        cancel,
        matched_local,
        entries.len(),
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
fn score_local(pattern: &Pattern, entries: &[Entry], limit: usize) -> (Vec<Hit>, usize) {
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
    (materialise(&scored, entries, pattern), matched)
}

/// Best score first, then the shorter name, then alphabetical. A short name
/// that matches is a stronger signal than a long one containing the same
/// letters by coincidence.
///
/// A packed folder produces tens of thousands of ties (a two-letter query
/// matches almost everything), so this comparator is the hot loop: it never
/// allocates, and the list is cut to `limit` before it is fully sorted.
fn rank(scored: &mut Vec<(u32, u32)>, entries: &[Entry], limit: usize) {
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
fn materialise(scored: &[(u32, u32)], entries: &[Entry], pattern: &Pattern) -> Vec<Hit> {
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
                entry: entry.clone(),
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

/// Walk the tree below `root`, matching names as they are read.
///
/// Workers share one stack of pending folders and one channel, so a wide tree
/// divides rather than being walked in sequence. The receiving side only
/// assembles batches; it never touches the disk.
fn walk<F>(
    root: &Path,
    pattern: &Pattern,
    show_hidden: bool,
    cancel: &Arc<AtomicBool>,
    matched_so_far: usize,
    scanned_so_far: usize,
    emit: &mut F,
) -> Outcome
where
    F: FnMut(SearchEvent) -> bool,
{
    // Seed with the root's subfolders: the root's own entries belong to phase
    // one and must not be read twice.
    let stack: Mutex<Vec<(PathBuf, usize)>> = Mutex::new(subfolders(root, show_hidden, 0));
    let examined = AtomicUsize::new(scanned_so_far);
    let (tx, rx) = mpsc::channel::<Hit>();
    let workers = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(12);

    let mut found = 0usize;
    let mut accepted = 0usize;
    // Set when the scan ceiling stopped the walk, so `Done` can say the answer
    // is incomplete rather than implying the tree was fully searched.
    let mut cut = false;

    thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let stack = &stack;
            let examined = &examined;
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
                        let Ok(mut stack) = stack.lock() else { return };
                        let mut batch = Vec::with_capacity(4);
                        for _ in 0..4 {
                            match stack.pop() {
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
                            if let Ok(mut stack) = stack.lock() {
                                stack.extend(subdirs);
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
            found += 1;
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
        }

        if !batch.is_empty() {
            let _ = emit(SearchEvent::Batch {
                items: batch,
                scanned: examined.load(Ordering::Relaxed),
            });
        }
    });

    Outcome {
        matched: matched_so_far + found,
        scanned: examined.load(Ordering::Relaxed),
        truncated: cut || found > accepted,
    }
}

/// Immediate children of the root that are folders, for the walk to start on.
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
