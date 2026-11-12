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

    let outcome = walk(&pattern, &needle_bytes, Path::new(&req.path), req.show_hidden, cancel, matched_local, laid_out, &mut emit);
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
/// about one readdir per folder rather than one stat per file.
fn walk<F: FnMut(Event) -> bool>(
    pattern: &Pattern,
    needle: &Needle,
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
            let Some(score) = score_name(pattern, needle, &name, &mut folded, &mut buf, &mut matcher) else {
                continue;
            };
            matched += 1;
            if matched_so_far + accepted < LIMIT {
                // Same shortcut as scoring: an ASCII name needs no folded copy.
                let hay = if name.is_ascii() {
                    name.as_ref()
                } else {
                    fold_into(&name, &mut folded);
                    folded.as_str()
                };
                let mut indices = Vec::new();
                pattern.indices(Utf32Str::new(hay, &mut buf), &mut matcher, &mut indices);
                indices.sort_unstable();
                indices.dedup();
                if let Some(entry) = describe(name.into_owned(), item.path(), is_dir) {
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
            };
            let started = Instant::now();
            let mut matched = 0;
            let mut scanned = 0;
            search(&req, &AtomicBool::new(false), |event| {
                match event {
                    Event::Batch { items, .. } => matched += items.len(),
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