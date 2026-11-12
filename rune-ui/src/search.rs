//! The search engine: fuzzy name matching with the nucleo matcher (the engine
//! behind the old app and Helix), a fast in-memory pass over the folder on
//! screen, then a streaming breadth-first walk of its subfolders.
//!
//! Typing feels like filtering: the local results arrive in single-digit
//! milliseconds and deeper matches stream in behind them. A newer keystroke
//! cancels the walk. Matching is case-insensitive and accent-folded, scored to
//! reward contiguous runs and word boundaries, with the match positions kept so
//! the interface can highlight them.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, UNIX_EPOCH};

/// Past this the list is noise; deeper matches are counted but not shown.
const LIMIT: usize = 1000;
/// The only ceiling on a recursive walk: entries examined (a property of the
/// tree, not of the hardware, so the same query gives the same answer anywhere).
const MAX_SCANNED: usize = 5_000_000;
const MAX_DEPTH: usize = 32;
const BATCH_ITEMS: usize = 24;
const BATCH_MS: u64 = 40;
/// Names held for reuse by the next query. Roughly 40 bytes each with the
/// directory path shared, so this is about 80 MB at the ceiling; past it the
/// cache is abandoned rather than truncated, because a short index would answer
/// later queries with missing results instead of slow ones.
const MAX_CACHED: usize = 2_000_000;

/// The names one walk found, so the next query in the same search session can be
/// answered from memory instead of reading the tree again.
///
/// Only names are kept: the metadata a row needs is still read per match, which
/// keeps a cached answer exactly as fresh as a walked one. It is dropped when the
/// search box is cleared or the folder changes, so its staleness is bounded by
/// one typing session rather than by the life of the window.
pub struct NameCache {
    /// Directory paths, shared by every name inside them.
    dirs: Vec<std::sync::Arc<str>>,
    /// Directory index, name, and whether it is a folder.
    names: Vec<(u32, Box<str>, bool)>,
}

impl NameCache {
    pub fn len(&self) -> usize {
        self.names.len()
    }
}

pub struct Request {
    /// The folder on screen; the walk starts here.
    pub path: String,
    /// A snapshot of the folder's entries for the instant local pass.
    pub local: Vec<crate::Entry>,
    pub query: String,
    pub recursive: bool,
    pub show_hidden: bool,
    pub show_protected: bool,
    /// The names a previous walk gathered, when it finished and covered this
    /// folder. Supplying it answers the query without touching the disk.
    pub cache: Option<std::sync::Arc<NameCache>>,
}

#[derive(Clone)]
pub struct Hit {
    pub entry: crate::Entry,
    pub score: u32,
    /// Match positions as character offsets into the name, for highlighting.
    pub indices: Vec<u32>,
}

pub enum Event {
    /// Results from the folder on screen, emitted first.
    Local { items: Vec<Hit>, matched: usize, elapsed_ms: u64 },
    /// Results found deeper in the tree, as they arrive.
    Batch { items: Vec<Hit>, scanned: usize },
    /// The names this walk gathered, for the next query to answer from memory.
    Index(std::sync::Arc<NameCache>),
    Done { matched: usize, scanned: usize, elapsed_ms: u64, truncated: bool },
}

/// Fold accented letters onto the base letter, one character to one character so
/// highlight offsets stay aligned. ASCII (nearly every filename) is untouched;
/// case is left to the matcher's case-insensitive mode.
fn fold_into(input: &str, out: &mut String) {
    out.clear();
    if input.is_ascii() {
        out.push_str(input);
        return;
    }
    for ch in input.chars() {
        out.push(fold_char(ch));
    }
}

fn fold_char(ch: char) -> char {
    let c = ch.to_lowercase().next().unwrap_or(ch);
    match c {
        'à'|'á'|'â'|'ã'|'ä'|'å'|'ā'|'ă'|'ą'|'ǎ'|'ǻ'|'ạ'|'ả'|'ấ'|'ầ'|'ẩ'|'ẫ'|'ậ'|'ắ'|'ằ'|'ẳ'|'ẵ'|'ặ'|'æ'|'ǽ'|'ǣ' => 'a',
        'è'|'é'|'ê'|'ë'|'ē'|'ĕ'|'ė'|'ę'|'ě'|'ẹ'|'ẻ'|'ế'|'ề'|'ể'|'ễ'|'ệ' => 'e',
        'ì'|'í'|'î'|'ï'|'ĩ'|'ī'|'ĭ'|'į'|'ǐ'|'ị'|'ỉ'|'ı' => 'i',
        'ò'|'ó'|'ô'|'õ'|'ö'|'ō'|'ŏ'|'ő'|'ǒ'|'ǫ'|'ǭ'|'ọ'|'ỏ'|'ố'|'ồ'|'ổ'|'ỗ'|'ộ'|'ớ'|'ờ'|'ở'|'ỡ'|'ợ'|'ø'|'ǿ'|'ơ'|'œ' => 'o',
        'ù'|'ú'|'û'|'ü'|'ũ'|'ū'|'ŭ'|'ů'|'ű'|'ų'|'ǔ'|'ǖ'|'ǘ'|'ǚ'|'ǜ'|'ụ'|'ủ'|'ứ'|'ừ'|'ử'|'ữ'|'ự' => 'u',
        'ý'|'ÿ'|'ŷ'|'ỹ'|'ỳ'|'ỷ' => 'y',
        'ñ'|'ń'|'ň'|'ņ'|'ṅ'|'ṇ'|'ṉ'|'ŋ' => 'n',
        'ç'|'ć'|'ĉ'|'ċ'|'č'|'ḉ' => 'c',
        'ł'|'ľ'|'ļ'|'ḷ'|'ḻ' => 'l',
        'đ'|'ð'|'ď'|'ḋ'|'ḍ'|'ḏ'|'ḑ'|'ḓ' => 'd',
        'ś'|'ŝ'|'ş'|'š'|'ș'|'ṡ'|'ṣ'|'ṥ'|'ṧ'|'ṩ'|'ß' => 's',
        'ŕ'|'ř'|'ŗ'|'ṙ'|'ṛ'|'ṝ'|'ṟ' => 'r',
        'ť'|'ţ'|'ț'|'ṫ'|'ṭ'|'ṯ'|'ṱ'|'þ'|'ŧ' => 't',
        'ĝ'|'ğ'|'ġ'|'ģ'|'ǧ'|'ǵ'|'ḡ' => 'g',
        'ĥ'|'ȟ'|'ḣ'|'ḥ'|'ḧ'|'ḩ'|'ḫ'|'ħ' => 'h',
        'ĵ'|'ǰ' => 'j',
        'ķ'|'ǩ'|'ḱ'|'ḳ'|'ḵ'|'ĸ' => 'k',
        'ŵ'|'ẁ'|'ẃ'|'ẅ'|'ẇ'|'ẉ' => 'w',
        'ź'|'ż'|'ž'|'ẑ'|'ẓ'|'ẕ' => 'z',
        'ƒ' => 'f',
        other => other,
    }
}

/// The query as bytes, for the cheap check that runs before the expensive one.
struct Needle {
    /// The folded query, ASCII-lowercased.
    bytes: Vec<u8>,
}

impl Needle {
    fn new(folded: &str) -> Self {
        Self {
            bytes: folded.bytes().map(|byte| byte.to_ascii_lowercase()).collect(),
        }
    }

    /// Whether every byte of the query appears in order.
    ///
    /// A fuzzy match requires exactly this, so failing it cannot hide a real
    /// result: it only spares the scorer the names it would reject anyway. On a
    /// million entries that is most of them.
    fn is_subsequence(&self, haystack: &str) -> bool {
        if self.bytes.is_empty() {
            return true;
        }
        let hay = haystack.as_bytes();
        if hay.len() < self.bytes.len() {
            return false;
        }
        let mut want = 0;
        for &byte in hay {
            if byte.to_ascii_lowercase() == self.bytes[want] {
                want += 1;
                if want == self.bytes.len() {
                    return true;
                }
            }
        }
        false
    }
}

/// Score one name, doing as little as it can get away with.
///
/// An ASCII name folds to itself, so it is scored in place instead of being
/// copied into the fold buffer first, which is the common case by a wide margin.
/// The prefilter then rules out the names the scorer would reject.
fn score_name(
    pattern: &Pattern,
    needle: &Needle,
    name: &str,
    folded: &mut String,
    buf: &mut Vec<char>,
    matcher: &mut Matcher,
) -> Option<u32> {
    let hay = if name.is_ascii() {
        name
    } else {
        fold_into(name, folded);
        folded.as_str()
    };
    if !needle.is_subsequence(hay) {
        return None;
    }
    pattern.score(Utf32Str::new(hay, buf), matcher)
}

/// Run a search, emitting results as they are known. `emit` returns false to stop.

pub fn search<F: FnMut(Event) -> bool>(req: &Request, cancel: &AtomicBool, mut emit: F) {
    let started = Instant::now();
    let needle = req.query.trim();
    if needle.is_empty() {
        return;
    }

    let mut folded_needle = String::new();
    fold_into(needle, &mut folded_needle);
    let pattern = Pattern::parse(&folded_needle, CaseMatching::Ignore, Normalization::Smart);

    // Phase one: the folder on screen, straight from the snapshot in memory.
    let needle_bytes = Needle::new(&folded_needle);
    let (local, matched_local) = score_local(
        &pattern,
        &needle_bytes,
        &req.local,
        req.show_hidden,
        req.show_protected,
    );
    let laid_out = req.local.len();
    if !emit(Event::Local { items: local, matched: matched_local, elapsed_ms: started.elapsed().as_millis() as u64 }) {
        return;
    }

    // A walk earlier in this session already read the tree; match against what it
    // found instead of reading the disk again. Metadata is still read for the
    // matches, so the answer is as fresh as a walked one.
    if req.recursive
        && needle.chars().count() >= 2
        && let Some(cache) = &req.cache
    {
        let (hits, matched) = filter_cache(cache, &pattern, &needle_bytes, req.show_hidden);
        if !hits.is_empty() && !emit(Event::Batch { items: hits, scanned: cache.len() }) {
            return;
        }
        let _ = emit(Event::Done {
            matched,
            scanned: cache.len(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            truncated: matched > LIMIT,
        });
        return;
    }

    // A single character matches almost everything; walking a tree for it costs
    // far more than the answer is worth.
    if !req.recursive || needle.chars().count() < 2 || matched_local >= LIMIT || req.path.is_empty() {
        let _ = emit(Event::Done {
            matched: matched_local,
            scanned: laid_out,
            elapsed_ms: started.elapsed().as_millis() as u64,
            truncated: matched_local > LIMIT,
        });
        return;
    }

    let outcome = walk(&folded_needle, Path::new(&req.path), req.show_hidden, cancel, matched_local, laid_out, &mut emit);
    let _ = emit(Event::Done {
        matched: outcome.matched,
        scanned: outcome.scanned,
        elapsed_ms: started.elapsed().as_millis() as u64,
        truncated: outcome.truncated,
    });
}

/// Score the folder's entries and keep the best `LIMIT`, folders first.
fn score_local(
    pattern: &Pattern,
    needle: &Needle,
    entries: &[crate::Entry],
    show_hidden: bool,
    show_protected: bool,
) -> (Vec<Hit>, usize) {
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut folded = String::new();
    let mut scored: Vec<(u32, u32)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let visible = if entry.hidden && entry.system {
            show_hidden && show_protected
        } else if entry.hidden {
            show_hidden
        } else {
            true
        };
        if !visible {
            continue;
        }
        if let Some(score) = score_name(pattern, needle, &entry.name, &mut folded, &mut buf, &mut matcher) {
            scored.push((score, i as u32));
        }
    }
    let matched = scored.len();
    rank(&mut scored, entries);
    let hits = materialise(&scored, entries, pattern);
    (hits, matched)
}

/// Folders first, then best score, then shorter name, then alphabetical.
fn rank(scored: &mut Vec<(u32, u32)>, entries: &[crate::Entry]) {
    let compare = |a: &(u32, u32), b: &(u32, u32)| {
        let left = &entries[a.1 as usize];
        let right = &entries[b.1 as usize];
        right.is_dir.cmp(&left.is_dir)
            .then_with(|| b.0.cmp(&a.0))
            .then_with(|| left.name.len().cmp(&right.name.len()))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    };
    if scored.len() > LIMIT {
        scored.select_nth_unstable_by(LIMIT, compare);
        scored.truncate(LIMIT);
    }
    scored.sort_unstable_by(compare);
}

/// Turn scored pairs into hits, computing match positions only for survivors.
fn materialise(scored: &[(u32, u32)], entries: &[crate::Entry], pattern: &Pattern) -> Vec<Hit> {
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut folded = String::new();
    scored.iter().filter_map(|&(score, i)| {
        let entry = entries.get(i as usize)?;
        let hay = if entry.name.is_ascii() {
            entry.name.as_str()
        } else {
            fold_into(&entry.name, &mut folded);
            folded.as_str()
        };
        let mut indices = Vec::new();
        pattern.indices(Utf32Str::new(hay, &mut buf), &mut matcher, &mut indices);
        indices.sort_unstable();
        indices.dedup();
        Some(Hit { entry: entry.clone(), score, indices })
    }).collect()
}

struct Outcome {
    matched: usize,
    scanned: usize,
    truncated: bool,
}

/// Breadth-first walk of the tree below `root`, matching names as they are read
/// and streaming matches in batches. Only matches are stat'ed, so the walk costs
/// What the walking threads share: the directories still to read, how many are
/// outstanding, and the counters the caller needs back.
struct Walk {
    queue: std::sync::Mutex<VecDeque<(PathBuf, usize)>>,
    /// Directories queued or being read. Zero means the walk is finished, which
    /// is what lets a worker with an empty queue know to stop rather than spin.
    pending: std::sync::atomic::AtomicUsize,
    cancel: AtomicBool,
    scanned: std::sync::atomic::AtomicUsize,
    matched: std::sync::atomic::AtomicUsize,
    accepted: std::sync::atomic::AtomicUsize,
    /// Something stopped the walk early: a limit, or the caller leaving.
    cut: AtomicBool,
    /// Directory paths, interned so each name in one costs only its own text.
    dirs: std::sync::Mutex<Vec<std::sync::Arc<str>>>,
    /// Everything read so far, for the next query.
    names: std::sync::Mutex<Vec<(u32, Box<str>, bool)>>,
    /// The cap was passed, so this walk's names cannot be used as an index.
    overflow: AtomicBool,
}

/// One worker's private matching state, so no lock is needed to score a name.
struct Scratch {
    pattern: Pattern,
    needle: Needle,
    matcher: Matcher,
    buf: Vec<char>,
    folded: String,
    batch: Vec<Hit>,
    last_flush: Instant,
    /// Names read since the last flush into the shared index.
    cached: Vec<(u32, Box<str>, bool)>,
}

impl Scratch {
    fn new(query: &str) -> Self {
        Self {
            pattern: Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart),
            needle: Needle::new(query),
            matcher: Matcher::new(Config::DEFAULT),
            buf: Vec::new(),
            folded: String::new(),
            batch: Vec::new(),
            last_flush: Instant::now(),
            cached: Vec::new(),
        }
    }
}

/// Move a worker's buffered names into the shared index, one lock per chunk
/// rather than one per name. Past the cap the index is abandoned, not truncated.
fn flush_cached(shared: &Walk, buffered: &mut Vec<(u32, Box<str>, bool)>) {
    let mut names = shared.names.lock().unwrap_or_else(|e| e.into_inner());
    if names.len() + buffered.len() > MAX_CACHED {
        shared.overflow.store(true, Ordering::Relaxed);
        buffered.clear();
        return;
    }
    names.append(buffered);
}

/// One less than the cores, so the window keeps a core of its own, and never so
/// many that the walk competes with the renderer.
fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|cores| cores.get())
        .unwrap_or(2)
        .saturating_sub(1)
        .clamp(1, 8)
}

/// Breadth-first walk of the tree below `root`, on several threads, matching
/// names as they are read and streaming matches in batches. Only matches are
/// stat'ed, so the walk costs about one readdir per folder rather than one stat
/// per file.
///
/// The queue is shared and every worker takes the next directory, which keeps
/// the work balanced whatever shape the tree has. Only the caller emits, on its
/// own thread, so the interface sees one ordered stream of batches while the
/// reading happens in parallel. Which batch arrives first is not defined, and
/// does not matter: the caller ranks the whole result once the walk ends.
fn walk<F: FnMut(Event) -> bool>(
    query: &str,
    root: &Path,
    show_hidden: bool,
    cancel: &AtomicBool,
    matched_so_far: usize,
    scanned_so_far: usize,
    emit: &mut F,
) -> Outcome {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    let threads = worker_count();
    let mut seed = VecDeque::new();
    seed.push_back((root.to_path_buf(), 0));
    let shared = std::sync::Arc::new(Walk {
        queue: std::sync::Mutex::new(seed),
        pending: AtomicUsize::new(1),
        cancel: AtomicBool::new(false),
        scanned: AtomicUsize::new(scanned_so_far),
        matched: AtomicUsize::new(matched_so_far),
        accepted: AtomicUsize::new(0),
        cut: AtomicBool::new(false),
        dirs: std::sync::Mutex::new(Vec::new()),
        names: std::sync::Mutex::new(Vec::new()),
        overflow: AtomicBool::new(false),
    });

    let (tx, rx) = mpsc::channel::<Vec<Hit>>();
    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let shared = shared.clone();
        let tx = tx.clone();
        let query = query.to_string();
        handles.push(std::thread::spawn(move || {
            work(&shared, &tx, &query, show_hidden, matched_so_far);
        }));
    }
    // The caller's own sender must go, or the channel never closes.
    drop(tx);

    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(items) => {
                let scanned = shared.scanned.load(Ordering::Relaxed);
                if !emit(Event::Batch { items, scanned }) {
                    shared.cancel.store(true, Ordering::Relaxed);
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::Relaxed) {
                    shared.cancel.store(true, Ordering::Relaxed);
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    for handle in handles {
        let _ = handle.join();
    }
    // Keep the names for the next query, unless this walk stopped short: a short
    // index would answer later queries with missing results rather than slow ones.
    if !shared.overflow.load(Ordering::Relaxed) && !shared.cancel.load(Ordering::Relaxed) {
        let dirs = std::mem::take(&mut *shared.dirs.lock().unwrap_or_else(|e| e.into_inner()));
        let names = std::mem::take(&mut *shared.names.lock().unwrap_or_else(|e| e.into_inner()));
        if !names.is_empty() {
            let _ = emit(Event::Index(std::sync::Arc::new(NameCache { dirs, names })));
        }
    }
    let scanned = shared.scanned.load(Ordering::Relaxed);
    let matched = shared.matched.load(Ordering::Relaxed);
    let accepted = shared.accepted.load(Ordering::Relaxed);
    Outcome {
        matched,
        scanned,
        truncated: shared.cut.load(Ordering::Relaxed) || matched > matched_so_far + accepted,
    }
}

/// Read directories until the queue is empty and nothing is outstanding.
fn work(
    shared: &Walk,
    tx: &std::sync::mpsc::Sender<Vec<Hit>>,
    query: &str,
    show_hidden: bool,
    matched_so_far: usize,
) {
    use std::sync::atomic::Ordering;

    let mut scratch = Scratch::new(query);
    loop {
        if shared.cancel.load(Ordering::Relaxed) {
            break;
        }
        let next = {
            let mut queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.pop_front()
        };
        let Some((dir, depth)) = next else {
            if shared.pending.load(Ordering::Relaxed) == 0 {
                break;
            }
            // Another worker is still reading a directory it took; it may yet
            // add more, so wait rather than finish early.
            std::thread::yield_now();
            continue;
        };
        let Ok(read) = fs::read_dir(&dir) else {
            shared.pending.fetch_sub(1, Ordering::Relaxed);
            continue;
        };
        let dir_index = {
            let mut dirs = shared.dirs.lock().unwrap_or_else(|e| e.into_inner());
            let index = dirs.len() as u32;
            dirs.push(std::sync::Arc::from(dir.to_string_lossy().as_ref()));
            index
        };
        let mut subdirs: Vec<(PathBuf, usize)> = Vec::new();
        for item in read.flatten() {
            if shared.cancel.load(Ordering::Relaxed) {
                break;
            }
            let examined = shared.scanned.fetch_add(1, Ordering::Relaxed) + 1;
            let raw = item.file_name();
            // Borrowed for valid UTF-8, which nearly every name is; the owned
            // string is only made for something that matched.
            let name = raw.to_string_lossy();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let file_type = item.file_type().ok();
            let is_dir = file_type.as_ref().map(|t| t.is_dir()).unwrap_or(false);
            let is_symlink = file_type.as_ref().map(|t| t.is_symlink()).unwrap_or(false);
            // Follow real folders only (a link can point at its own parent).
            if is_dir && !is_symlink && depth < MAX_DEPTH {
                subdirs.push((item.path(), depth + 1));
            }
            if !shared.overflow.load(Ordering::Relaxed) {
                scratch.cached.push((dir_index, Box::from(&*name), is_dir));
                if scratch.cached.len() >= 512 {
                    flush_cached(shared, &mut scratch.cached);
                }
            }
            let Some(score) = score_name(
                &scratch.pattern,
                &scratch.needle,
                &name,
                &mut scratch.folded,
                &mut scratch.buf,
                &mut scratch.matcher,
            ) else {
                continue;
            };
            shared.matched.fetch_add(1, Ordering::Relaxed);
            let accepted_now = shared.accepted.load(Ordering::Relaxed);
            if matched_so_far + accepted_now < LIMIT {
                // Same shortcut as scoring: an ASCII name needs no folded copy.
                let hay = if name.is_ascii() {
                    name.as_ref()
                } else {
                    fold_into(&name, &mut scratch.folded);
                    scratch.folded.as_str()
                };
                let mut indices = Vec::new();
                scratch.pattern.indices(
                    Utf32Str::new(hay, &mut scratch.buf),
                    &mut scratch.matcher,
                    &mut indices,
                );
                indices.sort_unstable();
                indices.dedup();
                if let Some(entry) = describe(name.into_owned(), item.path(), is_dir) {
                    shared.accepted.fetch_add(1, Ordering::Relaxed);
                    scratch.batch.push(Hit { entry, score, indices });
                }
            }
            if scratch.batch.len() >= BATCH_ITEMS
                || scratch.last_flush.elapsed().as_millis() as u64 >= BATCH_MS
            {
                if !scratch.batch.is_empty()
                    && tx.send(std::mem::take(&mut scratch.batch)).is_err()
                {
                    shared.cancel.store(true, Ordering::Relaxed);
                    break;
                }
                scratch.last_flush = Instant::now();
            }
            if examined >= MAX_SCANNED {
                shared.cut.store(true, Ordering::Relaxed);
                shared.cancel.store(true, Ordering::Relaxed);
                break;
            }
        }
        if !scratch.batch.is_empty()
            && scratch.last_flush.elapsed().as_millis() as u64 >= BATCH_MS
            && tx.send(std::mem::take(&mut scratch.batch)).is_err()
        {
            shared.cancel.store(true, Ordering::Relaxed);
        }
        if !subdirs.is_empty() {
            let added = subdirs.len();
            let mut queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.extend(subdirs);
            shared.pending.fetch_add(added, Ordering::Relaxed);
        }
        shared.pending.fetch_sub(1, Ordering::Relaxed);
    }
    if !scratch.batch.is_empty() {
        let _ = tx.send(scratch.batch);
    }
    if !scratch.cached.is_empty() {
        flush_cached(shared, &mut scratch.cached);
    }
}

/// Answer a query from names already gathered, reading metadata only for what
/// ends up shown. This is the walk's match path with the directory reads removed.
fn filter_cache(
    cache: &NameCache,
    pattern: &Pattern,
    needle: &Needle,
    show_hidden: bool,
) -> (Vec<Hit>, usize) {
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut folded = String::new();
    let mut hits = Vec::new();
    let mut matched = 0usize;
    for (dir, name, is_dir) in &cache.names {
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let Some(score) = score_name(pattern, needle, name, &mut folded, &mut buf, &mut matcher)
        else {
            continue;
        };
        matched += 1;
        if hits.len() >= LIMIT {
            continue;
        }
        let hay = if name.is_ascii() {
            &**name
        } else {
            fold_into(name, &mut folded);
            folded.as_str()
        };
        let mut indices = Vec::new();
        pattern.indices(Utf32Str::new(hay, &mut buf), &mut matcher, &mut indices);
        indices.sort_unstable();
        indices.dedup();
        let folder = cache.dirs.get(*dir as usize).map(|path| &**path).unwrap_or("");
        let path = Path::new(folder).join(&**name);
        if let Some(entry) = describe(name.to_string(), path, *is_dir) {
            hits.push(Hit { entry, score, indices });
        }
    }
    (hits, matched)
}

/// Build a display row for a match found in the walk; metadata is paid for only
/// here, on a match.
fn describe(name: String, path: PathBuf, is_dir: bool) -> Option<crate::Entry> {
    let meta = fs::metadata(&path).ok()?;
    let size = if is_dir { 0 } else { meta.len() };
    let modified_at = meta.modified().ok();
    let modified = modified_at
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| crate::format_time(d.as_secs()))
        .unwrap_or_default();
    let kind = crate::classify(&name, is_dir);
    let (hidden, system) = {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            let attrs = meta.file_attributes();
            (attrs & 0x2 != 0, attrs & 0x4 != 0)
        }
        #[cfg(not(windows))]
        {
            (name.starts_with('.'), false)
        }
    };
    Some(crate::Entry {
        path: path.to_string_lossy().into_owned(),
        name,
        is_dir,
        size,
        modified,
        modified_at,
        type_label: crate::kind_label(kind).to_string(),
        kind,
        hidden,
        system,
        fixture: false,
        fixture_size: String::new(),
    })
}

#[cfg(test)]
mod speed_tests {
    use super::*;
    use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo_matcher::{Config, Matcher, Utf32Str};

    /// Names chosen to exercise the parts that could break: ASCII, accents,
    /// mixed case, a non-Latin script, and characters that only match out of
    /// order.
    const NAMES: &[&str] = &[
        "report.docx",
        "README.md",
        "Résumé.pdf",
        "a_b_c.txt",
        "ABC",
        "photos",
        "İstanbul.txt",
        "日本語.txt",
        "x",
        "Quarterly Report 2026.xlsx",
    ];

    const QUERIES: &[&str] = &["rep", "abc", "resume", "md", "AB", "pho", "日本", "z", "R", "qr"];

    /// The whole point of the prefilter is that it cannot change an answer, so
    /// it must never reject a name the scorer accepts.
    #[test]
    fn the_prefilter_never_rejects_a_match() {
        for query in QUERIES {
            let mut folded_needle = String::new();
            fold_into(query, &mut folded_needle);
            let needle = Needle::new(&folded_needle);
            let pattern = Pattern::parse(&folded_needle, CaseMatching::Ignore, Normalization::Smart);
            let mut matcher = Matcher::new(Config::DEFAULT);
            let mut buf = Vec::new();
            let mut folded = String::new();
            for name in NAMES {
                let hay = if name.is_ascii() {
                    *name
                } else {
                    fold_into(name, &mut folded);
                    folded.as_str()
                };
                if pattern.score(Utf32Str::new(hay, &mut buf), &mut matcher).is_some() {
                    assert!(
                        needle.is_subsequence(hay),
                        "{name:?} matches {query:?} but the prefilter would drop it"
                    );
                }
            }
        }
    }

    /// The byte prefilter must agree with the scorer on what is worth scoring,
    /// including for names that are not ASCII.
    #[test]
    fn a_name_shorter_than_the_query_never_matches() {
        let mut folded = String::new();
        fold_into("ab", &mut folded);
        let needle = Needle::new(&folded);
        assert!(!needle.is_subsequence("a"));
        assert!(needle.is_subsequence("abc"));
        assert!(needle.is_subsequence("A-B"));
        assert!(!needle.is_subsequence("ax"));
    }

    fn probe(name: &str, is_dir: bool) -> crate::Entry {
        crate::Entry {
            name: name.to_string(),
            path: format!("C:/probe/{name}"),
            is_dir,
            size: if is_dir { 0 } else { 1 },
            modified: String::new(),
            modified_at: None,
            type_label: String::new(),
            kind: if is_dir { crate::Kind::Folder } else { crate::Kind::Text },
            hidden: false,
            system: false,
            fixture: false,
            fixture_size: String::new(),
        }
    }

    /// Runs the local pass end to end and returns the names in the order the
    /// interface would show them, so an optimisation that reordered results
    /// would fail here.
    fn local_order(query: &str, entries: Vec<crate::Entry>) -> Vec<String> {
        let req = Request {
            path: String::new(),
            local: entries,
            query: query.to_string(),
            recursive: false,
            show_hidden: false,
            show_protected: false,
            cache: None,
        };
        let mut out = Vec::new();
        search(&req, &AtomicBool::new(false), |event| {
            if let Event::Local { items, .. } = event {
                out = items.into_iter().map(|hit| hit.entry.name).collect();
            }
            true
        });
        out
    }

    #[test]
    fn the_interface_order_is_unchanged() {
        let entries = vec![
            probe("report.docx", false),
            probe("Quarterly Report 2026.xlsx", false),
            probe("Reports", true),
            probe("my-report.txt", false),
            probe("unrelated.md", false),
        ];
        // Folders first, then the score. "Report" on a word boundary outranks
        // the same letters inside "my-report", which is the scorer's judgement
        // and is not touched by the prefilter in front of it.
        assert_eq!(
            local_order("report", entries),
            ["Reports", "report.docx", "Quarterly Report 2026.xlsx", "my-report.txt"]
        );
    }

    #[test]
    fn accents_and_case_still_match() {
        let entries = vec![probe("Résumé.pdf", false), probe("README.md", false)];
        assert_eq!(local_order("resume", entries), ["Résumé.pdf"]);
        let entries = vec![probe("README.md", false)];
        assert_eq!(local_order("read", entries), ["README.md"]);
    }

    /// The whole point of the index is that it answers exactly what the walk
    /// would have. If these ever differ, the cache is a lie.
    #[test]
    fn a_cached_answer_equals_a_walked_one() {
        let dir = std::env::temp_dir().join(format!("lyre-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for name in [
            "report.docx",
            "notes.txt",
            "Résumé.pdf",
            "unrelated.bin",
            "sub/report-final.txt",
            "sub/other.md",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let root = dir.to_string_lossy().into_owned();

        let request = |cache| Request {
            path: root.clone(),
            local: Vec::new(),
            query: "report".into(),
            recursive: true,
            show_hidden: false,
            show_protected: false,
            cache,
        };

        let mut walked = Vec::new();
        let mut index = None;
        search(&request(None), &AtomicBool::new(false), |event| {
            match event {
                Event::Batch { items, .. } => walked.extend(items),
                Event::Index(built) => index = Some(built),
                _ => {}
            }
            true
        });
        assert!(walked.len() >= 2, "the walk should find both reports");
        let index = index.expect("a completed walk hands back its names");

        let mut cached = Vec::new();
        search(&request(Some(index)), &AtomicBool::new(false), |event| {
            if let Event::Batch { items, .. } = event {
                cached.extend(items);
            }
            true
        });

        // Compare what the interface would actually show: the path, the score and
        // the highlight positions.
        let shape = |hits: &[Hit]| {
            let mut out: Vec<(String, u32, Vec<u32>)> = hits
                .iter()
                .map(|hit| (hit.entry.path.clone(), hit.score, hit.indices.clone()))
                .collect();
            out.sort();
            out
        };
        assert_eq!(shape(&walked), shape(&cached), "cached and walked answers differ");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Not a pass/fail check: point `LYRE_SEARCH_BENCH` at a real tree and read
    /// the throughput. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn benchmark_a_real_walk() {
        let Ok(root) = std::env::var("LYRE_SEARCH_BENCH") else {
            return;
        };
        for query in ["zzzzzz", "report", "sys"] {
            let req = Request {
                path: root.clone(),
                local: Vec::new(),
                query: query.to_string(),
                recursive: true,
                show_hidden: false,
                show_protected: false,
                cache: None,
            };
            let started = Instant::now();
            let mut matched = 0;
            let mut scanned = 0;
            search(&req, &AtomicBool::new(false), |event| {
                match event {
                    Event::Batch { items, .. } => matched += items.len(),
                    Event::Index(_) => {}
                    Event::Done { matched: m, scanned: s, .. } => {
                        matched = m;
                        scanned = s;
                    }
                    Event::Local { .. } => {}
                }
                true
            });
            let elapsed = started.elapsed().as_secs_f64();
            println!(
                "{query:>8}: {matched:>6} matches, {scanned:>9} scanned, {elapsed:>6.2}s, {:>9.0} entries/s",
                scanned as f64 / elapsed.max(0.001)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> crate::Entry {
        let kind = crate::classify(name, is_dir);
        crate::Entry {
            name: name.into(),
            path: format!("/folder/{name}"),
            is_dir,
            size: 0,
            modified: String::new(),
            modified_at: None,
            type_label: crate::kind_label(kind).into(),
            kind,
            hidden: name.starts_with('.'),
            system: false,
            fixture: false,
            fixture_size: String::new(),
        }
    }

    fn collect_events(req: &Request) -> Vec<Event> {
        let cancel = AtomicBool::new(false);
        let mut out = Vec::new();
        search(req, &cancel, |ev| { out.push(ev); true });
        out
    }

    #[test]
    fn fuzzy_matches_subsequence_and_scores_contiguous_higher() {
        let entries = vec![entry("report.txt", false), entry("r_e_p_o_r_t.txt", false), entry("unrelated.md", false)];
        let req = Request { path: "/folder".into(), local: entries, query: "report".into(), recursive: false, show_hidden: false, show_protected: false, cache: None };
        let events = collect_events(&req);
        let items = match &events[0] { Event::Local { items, .. } => items, _ => panic!() };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].entry.name, "report.txt", "contiguous match ranks first");
        assert!(!items[0].indices.is_empty(), "match positions are kept for highlighting");
    }

    #[test]
    fn folders_rank_before_files() {
        let entries = vec![entry("report.txt", false), entry("reports", true)];
        let req = Request { path: "/folder".into(), local: entries, query: "report".into(), recursive: false, show_hidden: false, show_protected: false, cache: None };
        let events = collect_events(&req);
        let items = match &events[0] { Event::Local { items, .. } => items, _ => panic!() };
        assert!(items[0].entry.is_dir, "folders lead even when a file scores similarly");
    }

    #[test]
    fn accent_folding_matches_base_letters() {
        let entries = vec![entry("Țări.txt", false), entry("soseaua.doc", false)];
        let req = Request { path: "/folder".into(), local: entries, query: "tari".into(), recursive: false, show_hidden: false, show_protected: false, cache: None };
        let events = collect_events(&req);
        let items = match &events[0] { Event::Local { items, .. } => items, _ => panic!() };
        assert!(items.iter().any(|h| h.entry.name == "Țări.txt"), "accent-folded match");
    }

    #[test]
    fn hidden_files_are_skipped_unless_shown() {
        let entries = vec![entry(".hidden.txt", false), entry("visible.txt", false)];
        let req = Request { path: "/folder".into(), local: entries, query: "txt".into(), recursive: false, show_hidden: false, show_protected: false, cache: None };
        let events = collect_events(&req);
        let items = match &events[0] { Event::Local { items, .. } => items, _ => panic!() };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].entry.name, "visible.txt");
    }

    #[test]
    fn single_character_does_not_walk() {
        let entries = vec![entry("a.txt", false)];
        let req = Request { path: "/folder".into(), local: entries, query: "a".into(), recursive: true, show_hidden: false, show_protected: false, cache: None };
        let events = collect_events(&req);
        // Local then Done, no Batch (no recursive walk for one character).
        assert!(matches!(events.last(), Some(Event::Done { .. })));
        assert!(!events.iter().any(|e| matches!(e, Event::Batch { .. })));
    }
}