//! Hold on to files a drag source is about to delete.
//!
//! An archiver does not put an archive on the clipboard when you drag out of
//! it. 7-Zip extracts the selected members into a throwaway folder
//! (`%TEMP%\7zE9A2F\`), WinRAR into `%TEMP%\Rar$EXa0.123\`, and both delete that
//! folder as soon as `DoDragDrop` returns. Browsers and mail clients do the
//! same with attachments. Two things go wrong for a webview-based file manager:
//!
//! * wry reads `CF_HDROP` inside `IDropTarget::Drop`, but the event only reaches
//!   the app by `PostMessageW`. By the time the window asks to paste, the source
//!   has often removed every path it just named.
//! * 7-Zip does not even extract until that `GetData(CF_HDROP)` call. At
//!   `DragEnter` its `CF_HDROP` names only the *empty* temp folder; the real
//!   member paths appear at `Drop`.
//!
//! There is exactly one moment the bytes are guaranteed to exist: inside
//! `IDropTarget::Drop`, after wry has collected the paths and before
//! `DragFinish`. The patched wry in `vendor/wry` calls `on_native_drop` there,
//! synchronously. `stage_early` snapshots at that point; `take` hands the
//! stable paths to the window; `release_after_paste` clears them when the copy
//! ends. Platforms without that hook fall back to `WindowEvent::DragDrop`, and
//! `take` stages what it can if both early hooks were missed.
//!
//! Detection is deliberately generic: **any** path under the system temp
//! directory is treated as ephemeral, not just folder names Lyre recognises.
//! That is what makes an unrecognised archiver, a browser attachment or a mail
//! client work without a new special case. Files outside temp are never copied
//! or moved through staging. The snapshot is made of `std` only: a hard link
//! keeps the bytes alive after the source unlinks its own name and costs no
//! data copy on NTFS, ext4 or APFS; a byte copy covers volumes that cannot link.
//! A source file is never renamed away — the source may still have it open.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
/// How fresh a single unclaimed snapshot must be to stand in for an exact key.
const RECENT: Duration = Duration::from_secs(10);
/// The Windows hook runs on the drop target's own thread. Hard links are cheap,
/// but an app could name a temp tree with tens of thousands of files; bound the
/// walk rather than freezing the window during someone else's drop.
const MAX_STAGED_ITEMS: usize = 4096;

/// Folder numbers are process-global, not per `DropState`: two windows, or two
/// tests in one binary, must never claim the same staging folder.
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
/// The state the wry hook uses. A `fn` pointer cannot capture, so the callback
/// looks its state up here instead.
static HOOK_STATE: OnceLock<DropState> = OnceLock::new();

/// Cheap to clone so the paste thread can own a handle to the same registry.
#[derive(Default, Clone)]
pub struct DropState {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// A drop staged before the window collected it.
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

/// Install the synchronous hook and return the state Tauri should manage.
/// Called once, before the webview is built.
pub fn install() -> DropState {
    let state = HOOK_STATE.get_or_init(DropState::default).clone();
    #[cfg(windows)]
    lyre_drophook::set(on_native_drop);
    state
}

/// The callback wry invokes on the drop target's own thread. It must not panic:
/// it runs inside a COM `extern "system"` boundary.
#[cfg(windows)]
fn on_native_drop(phase: lyre_drophook::Phase, paths: &[PathBuf]) {
    let Some(state) = HOOK_STATE.get() else {
        return;
    };
    match phase {
        // 7-Zip's `DragEnter` names an empty temp folder and has not extracted
        // anything yet. Snapshotting here would stage a directory with no
        // members under a key that will never match the final drop.
        lyre_drophook::Phase::Enter => {}
        lyre_drophook::Phase::Drop => stage_early(paths.to_vec(), state),
        lyre_drophook::Phase::Leave => cancel(state),
    }
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

    /// The synchronous hook already took this snapshot; a second attempt at the
    /// queued event would race the source's cleanup for no reason.
    fn has_pending(&self, key: &str) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        prune(&mut inner);
        inner.pending.contains_key(key)
    }

    /// A drag left without being dropped. Do not free the snapshot outright —
    /// a `drop` can arrive just after `leave` — but let `prune` reclaim it soon.
    /// Returns whether anything was newly marked, so one leave means one prune.
    fn abandon(&self) -> bool {
        if let Ok(mut inner) = self.inner.lock() {
            let now = Instant::now();
            let mut changed = false;
            for pending in inner.pending.values_mut() {
                if pending.abandoned.is_none() {
                    pending.abandoned = Some(now);
                    changed = true;
                }
            }
            return changed;
        }
        false
    }

    fn prune_now(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            prune(&mut inner);
        }
    }

    fn take(&self, key: &str) -> Option<Vec<String>> {
        let mut inner = self.inner.lock().ok()?;
        prune(&mut inner);
        if let Some(pending) = inner.pending.remove(key) {
            return Some(pending.paths);
        }
        // A path can be spelled differently between wry and the webview event
        // (short names, a junction, a trailing separator). With exactly one
        // fresh snapshot there is no ambiguity about which drop this is.
        if inner.pending.len() == 1 {
            let fresh = inner
                .pending
                .values()
                .next()
                .is_some_and(|p| p.abandoned.is_none() && age(p.at) < RECENT);
            if fresh {
                let only = inner.pending.keys().next().cloned();
                if let Some(only) = only {
                    return inner.pending.remove(&only).map(|p| p.paths);
                }
            }
        }
        None
    }
}

fn age(at: Instant) -> Duration {
    Instant::now().saturating_duration_since(at)
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
        if now.saturating_duration_since(since) < limit {
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

/// Never stage our own staging folder, or a drop from one Lyre window into
/// another would recurse through a new snapshot on every paste.
fn is_ours(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str().to_string_lossy().starts_with(PREFIX))
}

/// Whether a path lives under the system temp directory. The lexical check is
/// the hot path and does no I/O; canonicalising covers Windows 8.3 short names
/// and junctions, where the archiver and this process spell `%TEMP%` differently.
fn under_temp(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    let root = temp.to_string_lossy().to_lowercase();
    let root = root.trim_end_matches(['\\', '/']);
    if !root.is_empty() {
        let candidate = path.to_string_lossy().to_lowercase();
        // Require a separator after the root: `C:\Temp2` is not under `C:\Temp`.
        if candidate == root
            || candidate.starts_with(&format!("{root}\\"))
            || candidate.starts_with(&format!("{root}/"))
        {
            return true;
        }
    }
    match (path.canonicalize(), temp.canonicalize()) {
        (Ok(candidate), Ok(root)) => candidate.starts_with(root),
        _ => false,
    }
}

/// Any path a source may delete when the drag ends. Deliberately not tied to
/// `7zE` or `Rar$`: browsers, mail clients and archivers nobody has heard of
/// all extract to temp, and the point is to work with all of them.
fn ephemeral(path: &Path) -> bool {
    !is_ours(path) && under_temp(path)
}

/// The name an archiver gives its throwaway extraction folder. Used only to
/// decide whether a *lone parent folder* should become its members; staging
/// itself is not restricted to these names.
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

fn is_extraction_dir(path: &Path) -> bool {
    path.is_dir()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(extraction_name)
        && under_temp(path)
}

/// Replace a lone extraction container with the members the archiver put in
/// it. When the folder is empty — 7-Zip at `DragEnter`, before it has extracted
/// anything — keep it so a platform that gets no later event still attempts a
/// drop rather than silently losing it.
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
fn snapshot_into(
    from: &Path,
    to: &Path,
    budget: &mut usize,
) -> std::io::Result<()> {
    if *budget == 0 {
        return Err(std::io::Error::other("staging budget exhausted"));
    }
    *budget -= 1;
    let meta = fs::symlink_metadata(from)?;
    if meta.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)?.flatten() {
            // One member that cannot be read should not cost the other nineteen.
            let _ = snapshot_into(&entry.path(), &to.join(entry.file_name()), budget);
        }
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    // A hard link keeps the original name working for the source. Renaming is
    // deliberately absent: the source may still hold an open handle, and taking
    // its file away is how a drag into one app breaks the other.
    if fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    Ok(())
}

/// Snapshot an ephemeral drop into a folder Lyre owns. Returns `None` for a
/// normal drop, or when none of the paths could be held: the caller then uses
/// the original paths and loses nothing that working today.
fn snapshot(paths: &[PathBuf], state: &DropState) -> Option<(PathBuf, Vec<String>)> {
    if paths.is_empty() || !paths.iter().any(|path| ephemeral(path)) {
        return None;
    }
    let expanded = expand(paths);
    if !expanded.iter().any(|path| ephemeral(path)) {
        return None;
    }
    let root = state.next_root()?;
    let mut staged = Vec::new();
    let mut budget = MAX_STAGED_ITEMS;
    for (index, source) in expanded.iter().enumerate() {
        let Some(name) = source.file_name() else { continue };
        let to = root.join(index.to_string()).join(name);
        if snapshot_into(source, &to, &mut budget).is_err() {
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

/// Snapshot at the earliest point the Rust core sees the drop. On Windows this
/// is the patched wry hook, still inside `IDropTarget::Drop`; elsewhere it is
/// `WindowEvent::DragDrop`. Both may fire for the same drag, so this is
/// idempotent per path set.
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

/// The drag left the window without a drop. The interface never collected the
/// snapshot, so reclaim it after the short grace period. A window may receive
/// no further events at all after a leave, hence the delayed prune rather than
/// waiting for the next drag to come along.
pub fn cancel(state: &DropState) {
    if !state.abandon() {
        return;
    }
    let later = state.clone();
    let _ = std::thread::Builder::new()
        .name("lyre-drop-prune".into())
        .spawn(move || {
            std::thread::sleep(CANCEL_GRACE + Duration::from_millis(250));
            later.prune_now();
        });
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
    fn any_temp_path_is_ephemeral_but_our_own_staging_is_not() {
        let folder = scratch("generic");
        let file = folder.join("attachment.pdf");
        fs::write(&file, "pdf").unwrap();
        assert!(ephemeral(&file), "an unrecognised app's temp file counts");
        assert!(ephemeral(&folder));
        let ours = std::env::temp_dir()
            .join(format!("{PREFIX}9-1"))
            .join("0")
            .join("a.txt");
        assert!(!ephemeral(&ours), "a staged path must not be staged again");
        let sibling = std::env::temp_dir()
            .parent()
            .unwrap_or(&std::env::temp_dir())
            .join("TempSibling")
            .join("a.txt");
        assert!(!ephemeral(&sibling), "a sibling of temp is not under temp");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_lone_extraction_folder_becomes_its_members() {
        let container = scratch("expand");
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
    fn a_snapshot_never_takes_the_source_file_away() {
        let folder = scratch("norename");
        let source = folder.join("keep.txt");
        fs::write(&source, "mine").unwrap();
        let state = DropState::default();
        let (root, staged) = snapshot(std::slice::from_ref(&source), &state)
            .expect("a temp file is staged");
        assert_eq!(staged.len(), 1);
        assert_eq!(
            fs::read_to_string(&source).unwrap(),
            "mine",
            "hard link or copy, never a rename"
        );
        release_after_paste(&staged, &state);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn an_ordinary_drop_is_left_completely_alone() {
        let state = DropState::default();
        // A path outside the temp directory is a normal file manager drag.
        let outside = PathBuf::from(if cfg!(windows) { "C:/Lyre" } else { "/lyre" })
            .join("photo.jpg");
        assert!(snapshot(std::slice::from_ref(&outside), &state).is_none());
        assert_eq!(expand(std::slice::from_ref(&outside)), vec![outside]);
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

    #[cfg(windows)]
    #[test]
    fn the_native_hook_snapshots_into_the_installed_state() {
        let container = scratch("nativehook").join("7zE00E7");
        fs::create_dir_all(&container).unwrap();
        fs::write(container.join("a.txt"), "from the drop target").unwrap();

        let state = install();
        let original = vec![container.to_string_lossy().into_owned()];
        // Exactly what wry calls from inside IDropTarget::Drop.
        on_native_drop(
            lyre_drophook::Phase::Enter,
            std::slice::from_ref(&container),
        );
        on_native_drop(
            lyre_drophook::Phase::Drop,
            std::slice::from_ref(&container),
        );

        // The source cleans up before the queued event and the webview paste.
        fs::remove_dir_all(&container).unwrap();
        let handed = take(original, &state);
        assert_eq!(handed.len(), 1);
        assert_eq!(
            fs::read_to_string(&handed[0]).unwrap(),
            "from the drop target"
        );
        release_after_paste(&handed, &state);
        let _ = fs::remove_dir_all(container.parent().unwrap());
    }

    #[test]
    fn a_hook_snapshot_wins_over_the_queued_event() {
        let container = scratch("hook").join("7zE00E5");
        fs::create_dir_all(&container).unwrap();
        fs::write(container.join("a.txt"), "saved before cleanup").unwrap();

        let state = DropState::default();
        let original = vec![container.to_string_lossy().into_owned()];
        // The hook runs inside IDropTarget::Drop, while the files exist.
        stage_early(vec![container.clone()], &state);
        // The source deletes its temp folder before the queued event is read.
        fs::remove_dir_all(&container).unwrap();
        // The queued event must not overwrite the good snapshot.
        stage_early(vec![container.clone()], &state);

        let handed = take(original, &state);
        assert_eq!(handed.len(), 1);
        assert_eq!(
            fs::read_to_string(&handed[0]).unwrap(),
            "saved before cleanup"
        );
        release_after_paste(&handed, &state);
        let _ = fs::remove_dir_all(container.parent().unwrap());
    }

    #[test]
    fn one_fresh_snapshot_survives_a_differently_spelled_key() {
        let container = scratch("spelling").join("Rar$EXa0.123");
        fs::create_dir_all(&container).unwrap();
        fs::write(container.join("a.txt"), "winrar").unwrap();

        let state = DropState::default();
        let original = [container.to_string_lossy().into_owned()];
        stage_early(vec![container.clone()], &state);
        let spelled: Vec<String> = [format!("{}/", original[0])].to_vec();
        let handed = take(spelled.clone(), &state);
        assert_ne!(handed, spelled);
        assert_eq!(fs::read_to_string(&handed[0]).unwrap(), "winrar");
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
