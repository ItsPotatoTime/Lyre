//! Search behaviour and timing. The scanner's contract is tested in
//! `scan_tests`; this covers ranking, phases, and the promise that a packed
//! folder answers in milliseconds.

use crate::scan::{self, Entry};
use crate::search::{search, Index, SearchEvent, SearchRequest};
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
    index.store(path, Arc::new(entries.clone()));
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
fn the_request_decodes_from_exactly_what_the_interface_sends() {
    // `src/lib/ipc.ts` posts { path, query, recursive, showHidden }. If the
    // serde rename ever drifts, this is the test that catches it rather than a
    // search box that silently does nothing.
    let wire = serde_json::json!({
        "path": "/tmp/example",
        "query": "report",
        "recursive": true,
        "showHidden": false
    });
    let request: SearchRequest =
        serde_json::from_value(wire).expect("the interface's payload must decode");
    assert_eq!(request.path, "/tmp/example");
    assert_eq!(request.query, "report");
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
