//! Hold on to files a drag source is about to delete.
//!
//! An archiver does not put an archive on the clipboard when you drag out of
//! it. 7-Zip and WinRAR extract the selected members into a throwaway folder
//! (`%TEMP%\7zE9A2F\`, `%TEMP%\Rar$EXa0.123\`), name those paths in `CF_HDROP`,
//! and delete the folder as soon as `DoDragDrop` returns. Two things go wrong
//! for a file manager built on Tauri:
//!
//! * The drag can be described by the *parent* folder rather than its members,
//!   so a twenty-file extraction arrives as one folder.
//! * wry reads `CF_HDROP` inside `IDropTarget::Drop`, but the event crosses to
//!   the app by `PostMessageW`. By the time the window asks to paste, the
//!   source has often removed every path it just named.
//!
//! Nothing here can run inside `IDropTarget::Drop` without replacing wry's
//! native drop target, which would break every other drop: wry reads `CF_HDROP`
//! there and its listener only *posts* a message to the event loop. The next
//! best moment is `DragEnter`, which carries the same paths but arrives while
//! the pointer is still over the window and the archiver is still holding its
//! files. `stage_early` snapshots on both `Enter` and `Drop` (the second is a
//! no-op when the first worked), `take` hands the result to the window, and
//! `release_after_paste` clears it once the copy is done. If both early hooks
//! are ever missed, `take` still stages what it can rather than failing the
//! drop.
//!
//! The snapshot is made of `std` only: a hard link keeps the bytes alive after
//! the archiver unlinks its own name and costs no data copy on NTFS, ext4 or
//! APFS. A rename and then a copy cover volumes that cannot link. All of this
//! is path based, so it builds and runs on every platform, and ordinary drops
//! from a file manager are never copied or moved through a staging folder.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// Staging folders are named so a later run can recognise and sweep them.
const PREFIX: &str = "lyre-drop-";
/// An in-process snapshot nobody asked for is abandoned after half an hour.
const MAX_AGE: Duration = Duration::from_secs(30 * 60);
/// A drag that left the window is given a short reprieve: some webviews send
/// `leave` just after `drop`, and the interface has not collected the paths yet.
const CANCEL_GRACE: Duration = Duration::from_secs(15);
/// A folder left by a crashed run is old enough to sweep after six hours.
const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// Folder numbers are process-global, not per `DropState`: two windows, or two
/// tests in one binary, must never claim the same staging folder.
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// Cheap to clone so the paste thread can own a handle to the same registry.
#[derive(Default, Clone)]
pub struct DropState {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// A drop staged on the event loop, waiting for the window to collect it.
    pending: HashMap<String, Pending>,
    /// Every staging folder this process owns, pending or handed over.
    owned: Vec<PathBuf>,
}

struct Pending {
    dir: PathBuf,
    paths: Vec<String>,
    at: Instant,
    /// `DragLeave` without a `Drop`. Kept briefly in case `drop` follows.
    abandoned: Option<Instant>,
}

impl DropState {
    fn next_root(&self) -> Option<PathBuf> {
        let base = std::env::temp_dir();
        for _ in 0..128 {
            let n = NEXT_ROOT.fetch_add(1, Ordering::Relaxed) + 1;
            let dir = base.join(format!("{PREFIX}{}-{n}", std::process::id()));
            // Creating, rather than checking and then creating, is what makes
            // two drops racing for the same number harmless.
            match fs::create_dir(&dir) {
                Ok(()) => return Some(dir),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return None,
            }
        }
        None
    }

    fn own(&self, dir: PathBuf) {
        if let Ok(mut inner) = self.inner.lock() {
            prune(&mut inner);
            inner.owned.push(dir);
        }
    }

    fn record(&self, key: String, dir: PathBuf, paths: Vec<String>) {
        if let Ok(mut inner) = self.inner.lock() {
            prune(&mut inner);
            let previous = inner.pending.insert(
                key,
                Pending {
                    dir: dir.clone(),
                    paths,
                    at: Instant::now(),
                    abandoned: None,
                },
            );
            // Only a genuinely second drag with the same paths reaches here.
            if let Some(previous) = previous {
                inner.owned.retain(|owned| *owned != previous.dir);
                let _ = fs::remove_dir_all(&previous.dir);
            }
            inner.owned.push(dir);
        }
    }

    /// `DragEnter` already took this snapshot; `Drop` must not make a second
    /// one at the very moment the source starts deleting the files.
    fn has_pending(&self, key: &str) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        prune(&mut inner);
        inner.pending.contains_key(key)
    }

    /// A drag left without being dropped. Do not free the snapshot outright —
    /// a `drop` can arrive just after `leave` — but let `prune` reclaim it soon.
    fn abandon(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            let now = Instant::now();
            for pending in inner.pending.values_mut() {
                pending.abandoned.get_or_insert(now);
            }
        }
    }

    fn prune_now(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            prune(&mut inner);
        }
    }

    fn take(&self, key: &str) -> Option<Vec<String>> {
        let mut inner = self.inner.lock().ok()?;
        prune(&mut inner);
        inner.pending.remove(key).map(|pending| pending.paths)
    }
}

/// Only a snapshot nobody ever collected is safe to throw away. A staging
/// folder already handed to the interface may be read by a paste that takes
/// hours, so it stays until `release_after_paste` or the next startup sweep.
fn prune(inner: &mut Inner) {
    let now = Instant::now();
    let mut expired = Vec::new();
    inner.pending.retain(|_, pending| {
        let (since, limit) = match pending.abandoned {
            Some(left) => (left, CANCEL_GRACE),
            None => (pending.at, MAX_AGE),
        };
        if now.duration_since(since) < limit {
            true
        } else {
            expired.push(pending.dir.clone());
            false
        }
    });
    for dir in expired {
        inner.owned.retain(|owned| *owned != dir);
        let _ = fs::remove_dir_all(dir);
    }
}

/// Remove staging folders left behind by an earlier process. Called once at
/// startup; a folder still in use by this process is far too young to match.
pub fn sweep() {
    let cutoff = SystemTime::now().checked_sub(STALE_AFTER);
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(PREFIX) {
            continue;
        }
        let too_old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .zip(cutoff)
            .is_some_and(|(modified, cutoff)| modified < cutoff);
        if too_old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The name an archiver gives its throwaway extraction folder. Deliberately
/// narrow: a folder of the person's own called `7zE1234` should stay a folder.
fn extraction_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("rar$") {
        return lower.len() > 4;
    }
    lower.strip_prefix("7ze").is_some_and(|rest| {
        // `~` covers the 8.3 short-name spelling Windows can hand back for a
        // longer generated folder, such as `7ZE9A2~1`.
        rest.len() >= 2
            && rest
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '~')
    })
}

fn has_extraction_component(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str().to_str().is_some_and(extraction_name))
}

/// Canonicalising resolves the short/long path spellings of `%TEMP%` that
/// differ between the archiver and this process on Windows.
fn under_temp(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    let base = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    match (base.canonicalize(), temp.canonicalize()) {
        (Ok(candidate), Ok(root)) => candidate.starts_with(root),
        _ => {
            let candidate = base.to_string_lossy().to_lowercase();
            let root = temp.to_string_lossy().to_lowercase();
            let root = root.trim_end_matches(['\\', '/']);
            !root.is_empty() && candidate.starts_with(root)
        }
    }
}

fn is_extraction_dir(path: &Path) -> bool {
    path.is_dir()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(extraction_name)
        && under_temp(path)
}

fn ephemeral(path: &Path) -> bool {
    has_extraction_component(path) && under_temp(path)
}

/// Replace a lone extraction container with the members the archiver put in
/// it. When the folder is empty — the extraction has not finished yet — keep
/// the folder so the drop is still attempted rather than silently dropped.
pub fn expand(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if is_extraction_dir(path) {
            let mut children = Vec::new();
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    children.push(entry.path());
                }
            }
            children.sort();
            if !children.is_empty() {
                out.extend(children);
                continue;
            }
        }
        out.push(path.clone());
    }
    out
}

/// Make one item durable under `to`. Directories are rebuilt so a member's own
/// name reaches the paste unchanged; each top-level item gets its own index so
/// two members with the same name in different folders cannot collide.
fn snapshot_into(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)?.flatten() {
            // One member that cannot be read should not cost the other nineteen.
            let _ = snapshot_into(&entry.path(), &to.join(entry.file_name()));
        }
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    Ok(())
}

/// Snapshot an extraction drop into a folder Lyre owns. Returns `None` for a
/// normal drop, or when none of the paths could be held: the caller then uses
/// the original paths and loses nothing that working today.
fn snapshot(paths: &[PathBuf], state: &DropState) -> Option<(PathBuf, Vec<String>)> {
    if paths.is_empty() {
        return None;
    }
    // This runs on `DragEnter` for every drag, including ordinary ones. A
    // string-only check keeps a normal drag off the disk entirely.
    if !paths
        .iter()
        .any(|path| has_extraction_component(path))
    {
        return None;
    }
    let expanded = expand(paths);
    if !expanded.iter().any(|path| ephemeral(path)) {
        return None;
    }
    let root = state.next_root()?;
    let mut staged = Vec::new();
    for (index, source) in expanded.iter().enumerate() {
        let Some(name) = source.file_name() else { continue };
        let to = root.join(index.to_string()).join(name);
        if snapshot_into(source, &to).is_err() {
            continue;
        }
        staged.push(to.to_string_lossy().into_owned());
    }
    if staged.is_empty() {
        let _ = fs::remove_dir_all(&root);
        return None;
    }
    Some((root, staged))
}

fn key(paths: &[String]) -> String {
    paths.join("\u{1f}")
}

/// Called from `WindowEvent::DragDrop` for both `Enter` and `Drop`. `Enter` is
/// the reliable one: the archiver cannot have cleaned up while the drag is
/// still in flight. `Drop` covers a platform that delivers no `Enter` paths.
pub fn stage_early(paths: Vec<PathBuf>, state: &DropState) {
    let original: Vec<String> = paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let key = key(&original);
    if state.has_pending(&key) {
        return;
    }
    if let Some((dir, staged)) = snapshot(&paths, state) {
        state.record(key, dir, staged);
    }
}

/// The drag left the window without a drop. The interface never collected the
/// snapshot, so reclaim it after the short grace period. A window may receive
/// no further events at all after a leave, hence the delayed prune rather than
/// waiting for the next drag to come along.
pub fn cancel(state: &DropState) {
    state.abandon();
    let later = state.clone();
    let _ = std::thread::Builder::new()
        .name("lyre-drop-prune".into())
        .spawn(move || {
            std::thread::sleep(CANCEL_GRACE + Duration::from_millis(250));
            later.prune_now();
        });
}

/// Called by the interface as soon as a drop arrives. The early snapshot wins;
/// staging here is a fallback for a platform that routes the event differently.
pub fn take(paths: Vec<String>, state: &DropState) -> Vec<String> {
    if let Some(staged) = state.take(&key(&paths)) {
        return staged;
    }
    let as_paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    match snapshot(&as_paths, state) {
        Some((dir, staged)) => {
            state.own(dir);
            staged
        }
        None => paths,
    }
}

/// A paste has finished reading these sources, so the staging folder can go.
pub fn release_after_paste(sources: &[String], state: &DropState) {
    let Ok(mut inner) = state.inner.lock() else {
        return;
    };
    let Inner { pending, owned } = &mut *inner;
    let mut keep = Vec::new();
    for dir in owned.drain(..) {
        let used = sources
            .iter()
            .any(|source| Path::new(source).starts_with(&dir));
        if used {
            pending.retain(|_, entry| entry.dir != dir);
            let _ = fs::remove_dir_all(&dir);
        } else {
            keep.push(dir);
        }
    }
    *owned = keep;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    static N: AtomicU64 = AtomicU64::new(0);

    fn scratch(label: &str) -> PathBuf {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "lyre-stage-test-{}-{label}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    #[test]
    fn only_archiver_temp_names_count() {
        assert!(extraction_name("7zE9A2F"));
        assert!(extraction_name("7ze0001"));
        assert!(extraction_name("7ZE9A2~1"));
        assert!(extraction_name("Rar$EXa0.123"));
        assert!(!extraction_name("7zE"));
        assert!(!extraction_name("7zE-two-dashes"));
        assert!(!extraction_name("Photos 2024"));
        assert!(!extraction_name("lyre-drop-12-3"));
    }

    #[test]
    fn a_lone_extraction_folder_becomes_its_members() {
        let container = scratch("expand");
        // The container itself has to look like an extraction folder, so it is
        // the child that is expanded here.
        let inner = container.join("7zE00A1");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join("a.txt"), "a").unwrap();
        fs::write(inner.join("b.txt"), "b").unwrap();
        fs::create_dir_all(inner.join("sub")).unwrap();

        let expanded = expand(std::slice::from_ref(&inner));
        assert_eq!(expanded.len(), 3, "all members, not one folder");
        assert!(expanded.contains(&inner.join("a.txt")));
        assert!(expanded.contains(&inner.join("sub")));

        let plain = scratch("plain");
        fs::write(plain.join("a.txt"), "a").unwrap();
        assert_eq!(
            expand(std::slice::from_ref(&plain)),
            vec![plain.clone()]
        );
        let _ = fs::remove_dir_all(&plain);
        let _ = fs::remove_dir_all(&container);
    }

    #[test]
    fn an_empty_container_is_kept_rather_than_losing_the_drop() {
        let container = scratch("empty").join("7zE00B2");
        fs::create_dir_all(&container).unwrap();
        assert_eq!(
            expand(std::slice::from_ref(&container)),
            vec![container.clone()]
        );
        let _ = fs::remove_dir_all(container.parent().unwrap());
    }

    #[test]
    fn a_snapshot_outlives_the_source_being_deleted() {
        let container = scratch("snapshot").join("7zE00C3");
        fs::create_dir_all(&container).unwrap();
        fs::write(container.join("one.txt"), "still here").unwrap();
        fs::create_dir_all(container.join("folder")).unwrap();
        fs::write(container.join("folder/two.txt"), "also here").unwrap();

        let state = DropState::default();
        let paths = vec![container.clone()];
        let (root, staged) = snapshot(&paths, &state).expect("an extraction drop is staged");
        assert_eq!(staged.len(), 2);
        state.own(root);

        // The archiver cleans up its own folder; the snapshot must not care.
        fs::remove_dir_all(&container).unwrap();
        let file = staged
            .iter()
            .find(|path| Path::new(path).file_name() == Some(OsStr::new("one.txt")))
            .expect("the file member was staged");
        assert_eq!(fs::read_to_string(file).unwrap(), "still here");
        let folder = staged
            .iter()
            .find(|path| Path::new(path).is_dir())
            .expect("the folder member was staged");
        assert_eq!(
            fs::read_to_string(Path::new(folder).join("two.txt")).unwrap(),
            "also here"
        );

        release_after_paste(&staged, &state);
        for path in &staged {
            assert!(!Path::new(path).exists(), "the staging folder was released");
        }
        let _ = fs::remove_dir_all(container.parent().unwrap());
    }

    #[test]
    fn an_ordinary_drop_is_left_completely_alone() {
        let folder = scratch("ordinary");
        fs::write(folder.join("photo.jpg"), "jpeg").unwrap();
        let state = DropState::default();
        let paths = vec![folder.join("photo.jpg")];
        assert!(snapshot(&paths, &state).is_none());
        assert_eq!(expand(&paths), paths);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_snapshot_handed_to_a_paste_survives_the_next_drop() {
        let state = DropState::default();
        let held = scratch("held");
        {
            let mut inner = state.inner.lock().unwrap();
            // A collected snapshot has no pending entry left, but its folder is
            // still owned. Pruning must not reclaim it while a paste reads it.
            inner.owned.push(held.clone());
            prune(&mut inner);
            assert_eq!(inner.owned, vec![held.clone()]);
        }
        let source = held.join("0/a.txt").to_string_lossy().into_owned();
        release_after_paste(&[source], &state);
        assert!(!held.exists(), "the paste released it instead");
    }

    #[test]
    fn taking_an_early_snapshot_replaces_the_original_paths() {
        let container = scratch("early").join("7zE00D4");
        fs::create_dir_all(&container).unwrap();
        fs::write(container.join("a.txt"), "a").unwrap();

        let state = DropState::default();
        let original = vec![container.to_string_lossy().into_owned()];
        stage_early(vec![container.clone()], &state);
        let handed = take(original.clone(), &state);
        assert_eq!(handed.len(), 1);
        assert_ne!(handed, original, "the window pastes from a stable path");
        assert!(Path::new(&handed[0]).exists());

        // A second take finds no early snapshot and falls back to staging now.
        let again = take(original.clone(), &state);
        assert!(!again.is_empty());

        release_after_paste(&handed, &state);
        release_after_paste(&again, &state);
        let _ = fs::remove_dir_all(container.parent().unwrap());
    }

    #[test]
    fn a_drop_reuses_the_snapshot_taken_on_enter() {
        let container = scratch("enter").join("7zE00E5");
        fs::create_dir_all(&container).unwrap();
        fs::write(container.join("a.txt"), "saved before the drop").unwrap();

        let state = DropState::default();
        let original = vec![container.to_string_lossy().into_owned()];
        stage_early(vec![container.clone()], &state); // DragEnter

        // The archiver cleans up before the queued Drop is read. A second
        // attempt now would find nothing, so the first snapshot must win.
        fs::remove_dir_all(&container).unwrap();
        stage_early(vec![container.clone()], &state); // Drop, too late

        let handed = take(original, &state);
        assert_eq!(handed.len(), 1);
        assert_eq!(
            fs::read_to_string(&handed[0]).unwrap(),
            "saved before the drop"
        );

        release_after_paste(&handed, &state);
        let _ = fs::remove_dir_all(container.parent().unwrap());
    }

    #[test]
    fn a_leave_does_not_beat_a_late_drop() {
        let container = scratch("leave").join("7zE00F6");
        fs::create_dir_all(&container).unwrap();
        fs::write(container.join("a.txt"), "a").unwrap();

        let state = DropState::default();
        let original = vec![container.to_string_lossy().into_owned()];
        stage_early(vec![container.clone()], &state);
        cancel(&state); // some webviews send leave right after drop
        let handed = take(original, &state);
        assert!(Path::new(&handed[0]).exists(), "the grace period held it");

        release_after_paste(&handed, &state);
        let _ = fs::remove_dir_all(container.parent().unwrap());
    }

    #[test]
    fn an_abandoned_snapshot_is_reclaimed_after_the_grace_period() {
        let state = DropState::default();
        let held = scratch("abandoned");
        fs::create_dir_all(&held).unwrap();
        {
            let mut inner = state.inner.lock().unwrap();
            inner.pending.insert(
                "drag".into(),
                Pending {
                    dir: held.clone(),
                    paths: Vec::new(),
                    at: Instant::now(),
                    abandoned: Some(Instant::now() - CANCEL_GRACE - Duration::from_secs(1)),
                },
            );
            inner.owned.push(held.clone());
            prune(&mut inner);
            assert!(inner.pending.is_empty());
            assert!(inner.owned.is_empty());
        }
        assert!(!held.exists());
    }
}
