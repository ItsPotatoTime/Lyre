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

pub struct Request {
    /// The folder on screen; the walk starts here.
    pub path: String,
    /// A snapshot of the folder's entries for the instant local pass.
    pub local: Vec<crate::Entry>,
    pub query: String,
    pub recursive: bool,
    pub show_hidden: bool,
    pub show_protected: bool,
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
    let (local, matched_local) = score_local(&pattern, &req.local, req.show_hidden, req.show_protected);
    let laid_out = req.local.len();
    if !emit(Event::Local { items: local, matched: matched_local, elapsed_ms: started.elapsed().as_millis() as u64 }) {
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

    let outcome = walk(&pattern, Path::new(&req.path), req.show_hidden, cancel, matched_local, laid_out, &mut emit);
    let _ = emit(Event::Done {
        matched: outcome.matched,
        scanned: outcome.scanned,
        elapsed_ms: started.elapsed().as_millis() as u64,
        truncated: outcome.truncated,
    });
}

/// Score the folder's entries and keep the best `LIMIT`, folders first.
fn score_local(pattern: &Pattern, entries: &[crate::Entry], show_hidden: bool, show_protected: bool) -> (Vec<Hit>, usize) {
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
        fold_into(&entry.name, &mut folded);
        if let Some(score) = pattern.score(Utf32Str::new(&folded, &mut buf), &mut matcher) {
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
        fold_into(&entry.name, &mut folded);
        let mut indices = Vec::new();
        pattern.indices(Utf32Str::new(&folded, &mut buf), &mut matcher, &mut indices);
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
/// about one readdir per folder rather than one stat per file.
fn walk<F: FnMut(Event) -> bool>(
    pattern: &Pattern,
    root: &Path,
    show_hidden: bool,
    cancel: &AtomicBool,
    matched_so_far: usize,
    scanned_so_far: usize,
    emit: &mut F,
) -> Outcome {
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((root.to_path_buf(), 0));
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut folded = String::new();
    let mut examined = scanned_so_far;
    let mut matched = matched_so_far;
    let mut accepted = 0usize;
    let mut batch: Vec<Hit> = Vec::new();
    let mut last_flush = Instant::now();
    let mut cut = false;

    while let Some((dir, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(read) = fs::read_dir(&dir) else { continue };
        let mut subdirs: Vec<(PathBuf, usize)> = Vec::new();
        for item in read.flatten() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            examined += 1;
            let name = item.file_name().to_string_lossy().into_owned();
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
            fold_into(&name, &mut folded);
            let Some(score) = pattern.score(Utf32Str::new(&folded, &mut buf), &mut matcher) else {
                continue;
            };
            matched += 1;
            if matched_so_far + accepted < LIMIT {
                if let Some(entry) = describe(name, item.path(), is_dir) {
                    let mut indices = Vec::new();
                    pattern.indices(Utf32Str::new(&folded, &mut buf), &mut matcher, &mut indices);
                    indices.sort_unstable();
                    indices.dedup();
                    batch.push(Hit { entry, score, indices });
                    accepted += 1;
                }
            }
            if batch.len() >= BATCH_ITEMS || last_flush.elapsed().as_millis() as u64 >= BATCH_MS {
                if !batch.is_empty() {
                    let alive = emit(Event::Batch { items: std::mem::take(&mut batch), scanned: examined });
                    last_flush = Instant::now();
                    if !alive {
                        cancel.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
            if examined >= MAX_SCANNED {
                cut = true;
                cancel.store(true, Ordering::Relaxed);
                break;
            }
        }
        if !batch.is_empty() && (last_flush.elapsed().as_millis() as u64 >= BATCH_MS) {
            let alive = emit(Event::Batch { items: std::mem::take(&mut batch), scanned: examined });
            last_flush = Instant::now();
            if !alive {
                break;
            }
        }
        queue.extend(subdirs);
    }
    if !batch.is_empty() {
        let _ = emit(Event::Batch { items: batch, scanned: examined });
    }
    Outcome { matched, scanned: examined, truncated: cut || matched > matched_so_far + accepted }
}

/// Build a display row for a match found in the walk; metadata is paid for only
/// here, on a match.
fn describe(name: String, path: PathBuf, is_dir: bool) -> Option<crate::Entry> {
    let meta = fs::metadata(&path).ok()?;
    let size = if is_dir { 0 } else { meta.len() };
    let modified = meta.modified().ok()
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
        type_label: crate::kind_label(kind).to_string(),
        kind,
        hidden,
        system,
        fixture: false,
        fixture_size: String::new(),
    })
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
        let req = Request { path: "/folder".into(), local: entries, query: "report".into(), recursive: false, show_hidden: false, show_protected: false };
        let events = collect_events(&req);
        let items = match &events[0] { Event::Local { items, .. } => items, _ => panic!() };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].entry.name, "report.txt", "contiguous match ranks first");
        assert!(!items[0].indices.is_empty(), "match positions are kept for highlighting");
    }

    #[test]
    fn folders_rank_before_files() {
        let entries = vec![entry("report.txt", false), entry("reports", true)];
        let req = Request { path: "/folder".into(), local: entries, query: "report".into(), recursive: false, show_hidden: false, show_protected: false };
        let events = collect_events(&req);
        let items = match &events[0] { Event::Local { items, .. } => items, _ => panic!() };
        assert!(items[0].entry.is_dir, "folders lead even when a file scores similarly");
    }

    #[test]
    fn accent_folding_matches_base_letters() {
        let entries = vec![entry("Țări.txt", false), entry("soseaua.doc", false)];
        let req = Request { path: "/folder".into(), local: entries, query: "tari".into(), recursive: false, show_hidden: false, show_protected: false };
        let events = collect_events(&req);
        let items = match &events[0] { Event::Local { items, .. } => items, _ => panic!() };
        assert!(items.iter().any(|h| h.entry.name == "Țări.txt"), "accent-folded match");
    }

    #[test]
    fn hidden_files_are_skipped_unless_shown() {
        let entries = vec![entry(".hidden.txt", false), entry("visible.txt", false)];
        let req = Request { path: "/folder".into(), local: entries, query: "txt".into(), recursive: false, show_hidden: false, show_protected: false };
        let events = collect_events(&req);
        let items = match &events[0] { Event::Local { items, .. } => items, _ => panic!() };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].entry.name, "visible.txt");
    }

    #[test]
    fn single_character_does_not_walk() {
        let entries = vec![entry("a.txt", false)];
        let req = Request { path: "/folder".into(), local: entries, query: "a".into(), recursive: true, show_hidden: false, show_protected: false };
        let events = collect_events(&req);
        // Local then Done, no Batch (no recursive walk for one character).
        assert!(matches!(events.last(), Some(Event::Done { .. })));
        assert!(!events.iter().any(|e| matches!(e, Event::Batch { .. })));
    }
}