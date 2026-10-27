//! Tests for the hot path. These do not need a window: the scanner talks to a
//! plain closure, so what is asserted here is the same code the interface runs.

use crate::scan::{scan, Entry, ScanEvent};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn collect(path: &Path, show_hidden: bool) -> (Vec<Entry>, usize, u64) {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut entries: Vec<Entry> = Vec::new();
    let mut reported = 0usize;
    let mut elapsed = 0u64;
    scan(path, show_hidden, &cancel, |event| {
        match event {
            ScanEvent::Batch { entries: batch, .. } => entries.extend(batch),
            ScanEvent::Done {
                total,
                elapsed_ms,
            } => {
                reported = total;
                elapsed = elapsed_ms;
            }
            ScanEvent::Failed { message } => panic!("scan failed: {message}"),
        }
        true
    });
    (entries, reported, elapsed)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lyre-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn lists_a_directory_and_reports_a_matching_total() {
    let dir = scratch("basic");
    fs::write(dir.join("a.txt"), b"hello").unwrap();
    fs::write(dir.join("b.rs"), b"fn main() {}").unwrap();
    fs::create_dir(dir.join("nested")).unwrap();

    let (entries, total, _) = collect(&dir, false);

    assert_eq!(entries.len(), 3);
    assert_eq!(total, 3, "the reported total must match what was sent");

    let names: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains("nested"));
    assert!(names.contains("a.txt"));

    let nested = entries.iter().find(|e| e.name == "nested").unwrap();
    assert!(nested.is_dir);
    assert_eq!(nested.kind, "folder");
    assert_eq!(nested.size, 0, "folders do not report a byte size");

    let text = entries.iter().find(|e| e.name == "a.txt").unwrap();
    assert_eq!(text.size, 5);
    assert_eq!(text.kind, "text");
    assert_eq!(text.ext, "txt");
    assert!(text.modified_ms.is_some());
    assert!(text.readable);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn hidden_entries_are_opt_in() {
    let dir = scratch("hidden");
    fs::write(dir.join("visible.txt"), b"x").unwrap();
    fs::write(dir.join(".hidden.txt"), b"x").unwrap();

    let (visible, _, _) = collect(&dir, false);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].name, "visible.txt");

    let (all, _, _) = collect(&dir, true);
    assert_eq!(all.len(), 2);
    let hidden = all.iter().find(|e| e.name == ".hidden.txt").unwrap();
    assert!(hidden.hidden, "the flag must survive even when shown");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_large_directory_arrives_in_batches_and_completes() {
    let dir = scratch("large");
    // Enough files to cross several chunks, so the streaming path is exercised
    // rather than the single-batch fast path.
    for i in 0..1500 {
        fs::write(dir.join(format!("file-{i:05}.dat")), b"z").unwrap();
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let mut batches = 0usize;
    let mut seen = HashSet::new();
    let mut total = 0usize;
    let mut done = false;
    scan(&dir, false, &cancel, |event| {
        match event {
            ScanEvent::Batch { entries, scanned } => {
                batches += 1;
                total = scanned;
                for entry in entries {
                    seen.insert(entry.name);
                }
            }
            ScanEvent::Done { total: t, .. } => {
                done = true;
                assert_eq!(t, 1500, "final total must be exact");
            }
            ScanEvent::Failed { message } => panic!("scan failed: {message}"),
        }
        true
    });

    assert!(done, "the scan must report completion");
    assert!(batches > 1, "1500 entries must stream in more than one batch");
    assert_eq!(seen.len(), 1500, "no entry may be dropped or duplicated");
    assert_eq!(total, 1500);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_missing_directory_fails_with_a_readable_message() {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut message = None;
    scan(
        Path::new("/definitely/not/a/real/path/lyre"),
        false,
        &cancel,
        |event| {
            if let ScanEvent::Failed { message: m } = event {
                message = Some(m);
            }
            true
        },
    );
    let message = message.expect("a missing directory must report a failure");
    assert!(
        message.contains("no longer exists"),
        "unexpected message: {message}"
    );
}

#[test]
fn cancellation_stops_the_scan_without_reporting_done() {
    let dir = scratch("cancel");
    for i in 0..2000 {
        fs::write(dir.join(format!("f-{i:05}.dat")), b"z").unwrap();
    }

    let cancel = Arc::new(AtomicBool::new(false));
    cancel.store(true, Ordering::Relaxed);

    let mut done = false;
    scan(&dir, false, &cancel, |event| {
        if matches!(event, ScanEvent::Done { .. }) {
            done = true;
        }
        true
    });

    assert!(!done, "a cancelled scan must not claim to have finished");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn kinds_follow_the_extension() {
    for (name, kind) in [
        ("photo.JPG", "image"),
        ("clip.mp4", "video"),
        ("song.flac", "audio"),
        ("bundle.zip", "archive"),
        ("paper.pdf", "pdf"),
        ("sheet.csv", "spreadsheet"),
        ("main.rs", "code"),
        ("README", "file"),
    ] {
        let dir = scratch(&format!("kind-{}-{}", name, std::process::id()));
        fs::write(dir.join(name), b"x").unwrap();
        let (entries, _, _) = collect(&dir, false);
        let entry = entries.first().expect("one entry");
        assert_eq!(entry.kind, kind, "for {name}");
        assert_eq!(entry.ext, name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default());
        fs::remove_dir_all(&dir).ok();
    }
}

/// Timing run, not an assertion. `cargo test --lib bench -- --ignored
/// --nocapture` prints how long a real 20,000-entry directory takes end to end
/// through the streaming path.
#[test]
#[ignore = "timing run, not an assertion"]
fn bench_twenty_thousand_entries() {
    let dir = scratch("bench");
    for i in 0..20_000 {
        fs::write(dir.join(format!("file-{i:06}.dat")), b"x").unwrap();
    }

    let started = std::time::Instant::now();
    let (entries, total, elapsed) = collect(&dir, false);
    let wall = started.elapsed();

    println!(
        "{} entries reported as {total} in {elapsed} ms internal, {wall:?} including first batch dispatch",
        entries.len()
    );
    fs::remove_dir_all(&dir).ok();
}
