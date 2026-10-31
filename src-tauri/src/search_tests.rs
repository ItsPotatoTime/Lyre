//! Search behaviour and timing. The scanner's contract is tested in
//! `scan_tests`; this covers ranking, phases, and the promise that a packed
//! folder answers in milliseconds.

use crate::scan::{self, Entry};
use crate::search::{search, CompactEntry, Index, SearchEvent, SearchRequest};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lyre-search-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn scan_into_index(index: &Index, path: &Path) -> Vec<Entry> {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut entries = Vec::new();
    scan::scan(path, false, &cancel, |event| {
        if let scan::ScanEvent::Batch { entries: batch, .. } = event {
            entries.extend(batch);
        }
        true
    });
    index.store(
        path,
        Arc::new(entries.iter().map(CompactEntry::from).collect()),
    );
    entries
}

struct Collected {
    local: Vec<String>,
    batches: Vec<String>,
    matched: usize,
    scanned: usize,
    elapsed_ms: u64,
    truncated: bool,
}

fn run(index: &Index, path: &Path, query: &str, recursive: bool) -> Collected {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut out = Collected {
        local: Vec::new(),
        batches: Vec::new(),
        matched: 0,
        scanned: 0,
        elapsed_ms: 0,
        truncated: false,
    };
    search(
        index,
        &SearchRequest {
            path: path.to_string_lossy().into_owned(),
            roots: Vec::new(),
            query: query.to_string(),
            recursive,
            show_hidden: false,
        },
        &cancel,
        |event| {
            match event {
                SearchEvent::Local { items, matched, elapsed_ms } => {
                    out.local = items.into_iter().map(|hit| hit.entry.name).collect();
                    out.matched = matched;
                    out.elapsed_ms = elapsed_ms;
                }
                SearchEvent::Batch { items, scanned } => {
                    out.batches
                        .extend(items.into_iter().map(|hit| hit.entry.name));
                    out.scanned = scanned;
                }
                SearchEvent::Done {
                    matched,
                    scanned,
                    elapsed_ms,
                    truncated,
                } => {
                    out.matched = matched;
                    out.scanned = scanned;
                    out.elapsed_ms = elapsed_ms;
                    out.truncated = truncated;
                }
                SearchEvent::Failed { message } => panic!("search failed: {message}"),
            }
            true
        },
    );
    out
}

/// A request with no folder of its own and several roots: how This PC searches.
fn run_roots(index: &Index, roots: &[PathBuf], query: &str) -> Collected {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut out = Collected {
        local: Vec::new(),
        batches: Vec::new(),
        matched: 0,
        scanned: 0,
        elapsed_ms: 0,
        truncated: false,
    };
    search(
        index,
        &SearchRequest {
            path: String::new(),
            roots: roots.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
            query: query.to_string(),
            recursive: true,
            show_hidden: false,
        },
        &cancel,
        |event| {
            match event {
                SearchEvent::Local { items, .. } => {
                    out.local = items.into_iter().map(|hit| hit.entry.name).collect();
                }
                SearchEvent::Batch { items, scanned } => {
                    out.batches
                        .extend(items.into_iter().map(|hit| hit.entry.name));
                    out.scanned = scanned;
                }
                SearchEvent::Done { matched, scanned, .. } => {
                    out.matched = matched;
                    out.scanned = scanned;
                }
                SearchEvent::Failed { message } => panic!("search failed: {message}"),
            }
            true
        },
    );
    out
}

/// Every hit from both phases, in the order the engine ranks them, with the
/// flag that decides whether it opens as a folder. Names alone cannot tell a
/// folder from a file that shares its name.
fn run_typed(index: &Index, path: &Path, query: &str, recursive: bool) -> Vec<(String, bool)> {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut found = Vec::new();
    search(
        index,
        &SearchRequest {
            path: path.to_string_lossy().into_owned(),
            roots: Vec::new(),
            query: query.to_string(),
            recursive,
            show_hidden: false,
        },
        &cancel,
        |event| {
            match event {
                SearchEvent::Local { items, .. } | SearchEvent::Batch { items, .. } => {
                    found.extend(
                        items
                            .into_iter()
                            .map(|hit| (hit.entry.name, hit.entry.is_dir)),
                    );
                }
                SearchEvent::Done { .. } => {}
                SearchEvent::Failed { message } => panic!("search failed: {message}"),
            }
            true
        },
    );
    found
}

/// The folder on screen is answered from memory, and folders in it are
/// results rather than only places to go next.
#[test]
fn a_folder_in_the_current_folder_is_found_by_its_name() {
    let dir = scratch("local-folder");
    fs::create_dir_all(dir.join("Invoices")).unwrap();
    fs::write(dir.join("invoices.txt"), b"x").unwrap();

    let index = Index::default();
    scan_into_index(&index, &dir);
    let found = run_typed(&index, &dir, "invoices", true);

    assert!(
        found.contains(&("Invoices".to_string(), true)),
        "the folder must be a hit marked as a folder: {found:?}"
    );
    assert!(
        found.contains(&("invoices.txt".to_string(), false)),
        "the file beside it must stay a hit: {found:?}"
    );

    fs::remove_dir_all(&dir).ok();
}

/// The walk reports folders as results, and still descends into them.
#[test]
fn a_recursive_search_finds_folders_below_the_starting_folder() {
    let dir = scratch("walk-folders");
    fs::create_dir_all(dir.join("projects/lyre-archive")).unwrap();
    fs::write(dir.join("projects/lyre-archive/lyre-notes.txt"), b"x").unwrap();

    let index = Index::default();
    scan_into_index(&index, &dir);
    let found = run_typed(&index, &dir, "lyre", true);

    assert!(
        found.contains(&("lyre-archive".to_string(), true)),
        "a folder below the starting folder must be found as a folder: {found:?}"
    );
    assert!(
        found.contains(&("lyre-notes.txt".to_string(), false)),
        "matching the folder must not stop the walk entering it: {found:?}"
    );

    fs::remove_dir_all(&dir).ok();
}

/// A folder is a place rather than a hit, so it leads even when a file beside
/// it matched better: "reports" is the file's whole name and only a fragment of
/// the folder's.
#[test]
fn a_folder_leads_a_file_that_matched_better() {
    let dir = scratch("folders-first");
    fs::create_dir_all(dir.join("reports-archive-2019")).unwrap();
    fs::write(dir.join("reports.txt"), b"x").unwrap();

    let index = Index::default();
    scan_into_index(&index, &dir);
    let found = run_typed(&index, &dir, "reports", false);

    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        found[0],
        ("reports-archive-2019".to_string(), true),
        "the folder must lead: {found:?}"
    );
    assert_eq!(found[1], ("reports.txt".to_string(), false));

    fs::remove_dir_all(&dir).ok();
}

/// Matching more than a list of rows can hold must not cost a folder its place:
/// folders are ranked first, and that ranking is what the cut to the row limit
/// uses, so the folders survive and the files fill what is left.
///
/// The folder names are deliberately the weakest matches here, scattered
/// letters with long names, so they can only survive the cut on being folders.
#[test]
fn every_folder_survives_the_row_limit() {
    let dir = scratch("folders-under-cap");
    for i in 0..1_200 {
        fs::write(dir.join(format!("report-{i:04}.txt")), b"x").unwrap();
    }
    for name in ["r-e-p-o-r-t-alpha", "r-e-p-o-r-t-beta", "r-e-p-o-r-t-gamma"] {
        fs::create_dir_all(dir.join(name)).unwrap();
    }

    let index = Index::default();
    scan_into_index(&index, &dir);
    let found = run_typed(&index, &dir, "report", false);

    assert!(
        found.len() < 1_203,
        "the answer must still be capped, not 1203 rows: {}",
        found.len()
    );
    let leading: Vec<&str> = found.iter().take(3).map(|(name, _)| name.as_str()).collect();
    assert!(
        leading.iter().all(|name| name.starts_with("r-e-p-o-r-t-")),
        "the folders must lead the capped list: {leading:?}"
    );
    assert!(
        found.iter().take(3).all(|(_, is_dir)| *is_dir),
        "those rows must be the folders: {leading:?}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn subsequences_match_and_rank_above_scattered_letters() {
    let dir = scratch("rank");
    for name in [
        "horizon-windows-x64.zip",
        "hazelnut-notes.txt",
        "zzz-unrelated.dat",
        "HORIZON.md",
    ] {
        fs::write(dir.join(name), b"x").unwrap();
    }
    let index = Index::default();
    scan_into_index(&index, &dir);

    let result = run(&index, &dir, "hzn", false);
    assert!(
        result.local.contains(&"horizon-windows-x64.zip".to_string()),
        "subsequence h-z-n must match: {:?}",
        result.local
    );
    assert!(
        result.local.contains(&"hazelnut-notes.txt".to_string()),
        "hazelnut-notes also contains h..z..n"
    );
    assert!(
        !result.local.iter().any(|name| name.contains("zzz")),
        "unrelated names must not match"
    );

    // A contiguous run beats scattered letters.
    let contiguous = run(&index, &dir, "notes", false);
    assert_eq!(
        contiguous.local.first().map(String::as_str),
        Some("hazelnut-notes.txt")
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn matching_is_case_insensitive_in_both_directions() {
    let dir = scratch("case");
    fs::write(dir.join("Report.docx"), b"x").unwrap();
    fs::write(dir.join("other.txt"), b"x").unwrap();
    let index = Index::default();
    scan_into_index(&index, &dir);

    let lower = run(&index, &dir, "report", false);
    assert_eq!(lower.local.len(), 1, "lowercase query must match mixed case");

    // Caps lock must not turn the search into a filter for upper-case names.
    let upper = run(&index, &dir, "REPORT", false);
    assert_eq!(upper.local.len(), 1, "uppercase query must still match");
    let mixed = run(&index, &dir, "RePoRt", false);
    assert_eq!(mixed.local.len(), 1, "mixed case must still match");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_first_phase_answers_from_the_index_without_touching_the_disk() {
    let dir = scratch("memory");
    for i in 0..50 {
        fs::write(dir.join(format!("note-{i:02}.txt")), b"x").unwrap();
    }
    let index = Index::default();
    scan_into_index(&index, &dir);

    // Delete the folder: if the first phase still answers, it came from memory.
    // A sibling, not a child: a folder cannot be renamed into itself.
    let gone = dir.with_file_name(format!("lyre-search-moved-{}", std::process::id()));
    let _ = fs::remove_dir_all(&gone);
    fs::rename(&dir, &gone).unwrap();

    let result = run(&index, &dir, "note", false);
    assert_eq!(result.matched, 50, "index must outlive the directory");
    assert_eq!(result.local.len(), 50);

    fs::remove_dir_all(&gone).ok();
}

#[test]
fn a_recursive_search_finds_matches_below_the_starting_folder() {
    let dir = scratch("recursive");
    fs::create_dir_all(dir.join("alpha/deep/deeper")).unwrap();
    fs::create_dir_all(dir.join("beta")).unwrap();
    fs::write(dir.join("alpha/deep/deeper/target-file.txt"), b"x").unwrap();
    fs::write(dir.join("beta/another-target.txt"), b"x").unwrap();
    fs::write(dir.join("beta/unrelated.bin"), b"y").unwrap();

    let index = Index::default();
    scan_into_index(&index, &dir);

    let result = run(&index, &dir, "target", true);
    let mut all = result.local;
    all.extend(result.batches);

    assert!(
        all.iter().any(|name| name == "target-file.txt"),
        "deep match missing: {all:?}"
    );
    assert!(
        all.iter().any(|name| name == "another-target.txt"),
        "second folder's match missing: {all:?}"
    );
    assert_eq!(result.matched, 2, "exactly the two matching files");
    assert!(
        !result.truncated,
        "a small tree must not be reported as truncated"
    );
    assert!(result.scanned >= 2, "the walk must report what it examined");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_single_character_does_not_walk_the_tree() {
    let dir = scratch("single");
    fs::create_dir_all(dir.join("nested")).unwrap();
    fs::write(dir.join("nested/needle.txt"), b"x").unwrap();
    let index = Index::default();
    scan_into_index(&index, &dir);

    let result = run(&index, &dir, "n", true);
    assert!(
        result.batches.is_empty(),
        "one character must not trigger a walk"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn searching_with_no_current_folder_covers_every_root() {
    // This PC has no listing of its own: both volumes must be searched, and
    // files sitting directly at a root must be found, not only those in
    // subfolders.
    let first = scratch("volume-one");
    let second = scratch("volume-two");
    fs::create_dir_all(first.join("nested")).unwrap();
    fs::write(first.join("at-the-root-report.txt"), b"x").unwrap();
    fs::write(first.join("nested/deep-report.txt"), b"x").unwrap();
    fs::write(second.join("second-volume-report.txt"), b"x").unwrap();

    let index = Index::default();
    let result = run_roots(&index, &[first.clone(), second.clone()], "report");

    let mut all = result.local;
    all.extend(result.batches);
    assert!(
        all.iter().any(|name| name == "at-the-root-report.txt"),
        "a file at the root itself must be found: {all:?}"
    );
    assert!(
        all.iter().any(|name| name == "deep-report.txt"),
        "a subfolder match must be found: {all:?}"
    );
    assert!(
        all.iter().any(|name| name == "second-volume-report.txt"),
        "the second volume must be searched: {all:?}"
    );
    assert_eq!(result.matched, 3);

    fs::remove_dir_all(&first).ok();
    fs::remove_dir_all(&second).ok();
}

#[test]
fn a_root_inside_another_root_is_only_walked_once() {
    // Volumes routinely list a folder that already lives inside another one.
    let outer = scratch("outer");
    let inner = outer.join("inner");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join("only-report.txt"), b"x").unwrap();

    let index = Index::default();
    let result = run_roots(
        &index,
        &[outer.clone(), inner.clone(), inner.clone()],
        "report",
    );

    let mut all = result.local;
    all.extend(result.batches);
    assert_eq!(
        all.iter().filter(|name| *name == "only-report.txt").count(),
        1,
        "a nested root must not produce a second copy: {all:?}"
    );
    assert_eq!(result.matched, 1);

    fs::remove_dir_all(&outer).ok();
}

#[test]
fn a_folder_search_does_not_read_its_own_entries_twice() {
    // Phase one lists the folder; the walk must only descend from it, or a
    // match in the current folder would be reported twice.
    let dir = scratch("no-double");
    fs::create_dir_all(dir.join("child")).unwrap();
    fs::write(dir.join("here-report.txt"), b"x").unwrap();
    fs::write(dir.join("child/there-report.txt"), b"x").unwrap();

    let index = Index::default();
    scan_into_index(&index, &dir);
    let result = run(&index, &dir, "report", true);

    let local_hits = result.local.iter().filter(|n| *n == "here-report.txt").count();
    let walked_hits = result
        .batches
        .iter()
        .filter(|n| *n == "here-report.txt")
        .count();
    assert_eq!(local_hits, 1, "the current folder's match appears once");
    assert_eq!(walked_hits, 0, "the walk must not repeat it");
    assert_eq!(result.matched, 2, "exactly two matches in the tree");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_request_decodes_from_exactly_what_the_interface_sends() {
    // `src/lib/ipc.ts` posts { path, query, recursive, showHidden }. If the
    // serde rename ever drifts, this is the test that catches it rather than a
    // search box that silently does nothing.
    let wire = serde_json::json!({
        "path": "/tmp/example",
        "roots": ["/tmp/example"],
        "query": "report",
        "recursive": true,
        "showHidden": false
    });
    let request: SearchRequest =
        serde_json::from_value(wire).expect("the interface's payload must decode");
    assert_eq!(request.path, "/tmp/example");
    assert_eq!(request.query, "report");
    assert_eq!(request.roots, vec!["/tmp/example".to_string()]);
    assert!(request.recursive);
    assert!(!request.show_hidden);
}

#[test]
fn events_serialise_to_the_shape_the_interface_reads() {
    // The interface narrows on `type` and reads camelCase fields, so the wire
    // shape is part of the contract rather than an implementation detail.
    let done = serde_json::to_value(SearchEvent::Done {
        matched: 3,
        scanned: 10,
        elapsed_ms: 2,
        truncated: false,
    })
    .unwrap();
    assert_eq!(done["type"], "done");
    assert_eq!(done["elapsedMs"], 2);
    assert_eq!(done["scanned"], 10);
    assert_eq!(done["truncated"], false);

    let hit = crate::search::Hit {
        entry: Entry {
            name: "report.docx".into(),
            path: "/tmp/report.docx".into(),
            is_dir: false,
            is_symlink: false,
            size: 12,
            modified_ms: Some(1),
            kind: "document",
            ext: "docx".into(),
            hidden: false,
            readable: true,
        },
        score: 104,
        indices: vec![0, 1, 2],
    };
    let json = serde_json::to_value(hit).unwrap();
    assert_eq!(json["name"], "report.docx", "entry fields are flattened in");
    assert_eq!(json["isDir"], false);
    assert_eq!(json["score"], 104);
    assert_eq!(json["indices"][0], 0);
}

#[test]
fn hidden_items_are_skipped_when_the_interface_hides_them() {
    let dir = scratch("hidden");
    fs::write(dir.join("visible-note.txt"), b"x").unwrap();
    fs::write(dir.join(".hidden-note.txt"), b"x").unwrap();
    let index = Index::default();
    scan_into_index(&index, &dir);

    let result = run(&index, &dir, "note", false);
    assert_eq!(result.local, vec!["visible-note.txt".to_string()]);

    fs::remove_dir_all(&dir).ok();
}

/// Timing run, not an assertion: proves the first phase is a memory scan.
/// `cargo test --lib search -- --ignored --nocapture`
#[test]
#[ignore = "timing run, not an assertion"]
fn bench_packed_folder() {
    let dir = scratch("bench");
    for i in 0..30_000 {
        fs::write(dir.join(format!("document-{i:05}-report.pdf")), b"x").unwrap();
    }

    let index = Index::default();
    let started = Instant::now();
    scan_into_index(&index, &dir);
    println!("index built in {:?}", started.elapsed());

    for query in ["re", "report", "doc7", "dcmnt"] {
        let started = Instant::now();
        let result = run(&index, &dir, query, false);
        println!(
            "  {query:>8}: {} matches in {:.2} ms (engine reported {} ms)",
            result.matched,
            started.elapsed().as_secs_f64() * 1000.0,
            result.elapsed_ms
        );
    }

    fs::remove_dir_all(&dir).ok();
}




/// What a keystroke costs when the answer lives below the folder on screen.
///
/// `cargo test --lib bench_recursive -- --ignored --nocapture`
#[test]
#[ignore = "timing run, not an assertion"]
fn bench_recursive_tree() {
    let root = scratch("bench-tree");
    // 150 folders of 150 files: enough that a walk is real work, small enough
    // to build quickly.
    for d in 0..150 {
        let dir = root.join(format!("section-{d:03}"));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..150 {
            fs::write(dir.join(format!("item-{d:03}-{f:03}.txt")), b"x").unwrap();
        }
    }

    let index = Index::default();
    println!("tree: 22,500 files across 150 folders");

    for query in ["report", "item-042", "zzz"] {
        let started = Instant::now();
        let first = run(&index, &root, query, true);
        let cold = started.elapsed().as_secs_f64() * 1000.0;

        let started = Instant::now();
        let second = run(&index, &root, query, true);
        let warm = started.elapsed().as_secs_f64() * 1000.0;

        println!(
            "  {query:>9}: cold {cold:7.1} ms ({} hits, {} scanned) | again {warm:7.1} ms",
            first.matched, first.scanned
        );
        assert_eq!(first.matched, second.matched, "the answer must not change");
    }

    fs::remove_dir_all(&root).ok();
}

/// A real tree, of the size where searching from This PC feels slow.
///
/// `cargo test --lib bench_real -- --ignored --nocapture`
#[test]
#[ignore = "timing run against the real filesystem"]
fn bench_real_tree() {
    let index = Index::default();
    for root in ["/usr", "/home"] {
        if !Path::new(root).exists() {
            continue;
        }
        for query in ["libc", "zzzzzzz", "cargo"] {
            let started = Instant::now();
            let result = run(&index, Path::new(root), query, true);
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            println!(
                "{root:>6} {query:>8}: {ms:8.1} ms | {} hits | {} scanned | truncated {}",
                result.matched, result.scanned, result.truncated
            );
        }
    }
}

/// Which accented names a plain-letter query finds, through the real engine.
///
/// `cargo test --lib bench_diacritics -- --ignored --nocapture`
#[test]
#[ignore = "diagnostic: prints what folds"]
fn bench_diacritics() {
    let dir = scratch("diacritics-bench");
    let names = [
        ("Țări", "tari"),
        ("Șoseaua", "soseaua"),
        ("Cărți", "carti"),
        ("Câmp", "camp"),
        ("Înalt", "inalt"),
        ("Łódź", "lodz"),
        ("Ørsted", "orsted"),
        ("Tiếng", "tieng"),
        ("Ёлка", "елка"),
        ("Йошкар", "иошкар"),
        ("Άλφα", "αλφα"),
        ("Müller", "muller"),
    ];
    for (name, _) in names {
        fs::write(dir.join(format!("{name}.txt")), b"x").unwrap();
    }
    let index = Index::default();
    scan_into_index(&index, &dir);

    for (name, needle) in names {
        let result = run(&index, &dir, needle, false);
        let found = result.local.iter().any(|hit| hit.starts_with(name));
        println!("{needle:>9} -> {name:<9} {}", if found { "found" } else { "MISSED" });
    }

    fs::remove_dir_all(&dir).ok();
}

/// Accents must not stop a search, in either direction.
#[test]
fn accented_names_are_found_by_plain_letters() {
    let dir = scratch("accents");
    for name in [
        "Țări și orașe.txt",
        "Șoseaua de centură.txt",
        "Łódź.txt",
        "Tiếng Việt.txt",
        "Ёлка.txt",
        "Straße.txt",
        "Müller.txt",
    ] {
        fs::write(dir.join(name), b"x").unwrap();
    }
    let index = Index::default();
    scan_into_index(&index, &dir);

    let cases = [
        ("tari", "Țări și orașe.txt"),
        ("soseaua", "Șoseaua de centură.txt"),
        ("lodz", "Łódź.txt"),
        ("tieng", "Tiếng Việt.txt"),
        ("елка", "Ёлка.txt"),
        ("strase", "Straße.txt"),
        ("muller", "Müller.txt"),
        // And the accented spelling still works.
        ("țari", "Țări și orașe.txt"),
    ];

    for (query, expected) in cases {
        let result = run(&index, &dir, query, false);
        assert_eq!(
            result.local,
            vec![expected.to_string()],
            "query {query:?} should find {expected:?}"
        );
    }

    fs::remove_dir_all(&dir).ok();
}

/// Highlighting must land on the letters the person can see.
#[test]
fn accents_do_not_shift_the_highlights() {
    let dir = scratch("accent-indices");
    fs::write(dir.join("Țări.txt"), b"x").unwrap();
    let index = Index::default();
    scan_into_index(&index, &dir);

    let cancel = Arc::new(AtomicBool::new(false));
    let mut found: Option<(String, Vec<u32>)> = None;
    search(
        &index,
        &SearchRequest {
            query: "tari".into(),
            path: dir.to_string_lossy().into_owned(),
            roots: Vec::new(),
            recursive: false,
            show_hidden: false,
        },
        &cancel,
        |event| {
            if let SearchEvent::Local { items, .. } = event {
                if let Some(hit) = items.into_iter().next() {
                    found = Some((hit.entry.name, hit.indices));
                }
            }
            true
        },
    );

    let (name, indices) = found.expect("a hit");
    let chars: Vec<char> = name.chars().collect();
    let lit: String = indices.iter().filter_map(|i| chars.get(*i as usize)).collect();
    assert_eq!(lit, "Țări", "the highlighted characters spell the query");

    fs::remove_dir_all(&dir).ok();
}
