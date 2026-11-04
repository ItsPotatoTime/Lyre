//! Notice when the folder on screen changes underneath the window.
//!
//! Windows has a real change notification for a directory, and using it means
//! a file saved by another program is on screen without any periodic work.
//! Everywhere else — and on Windows volumes that refuse a notification, which
//! network shares commonly do — the same answer comes from comparing a cheap
//! fingerprint of the entries. That fallback is made of nothing but `std`, so
//! it compiles and is tested on every platform we build for. No new
//! dependency, and no platform is left with a feature that only half works.
//!
//! Only one folder is watched: the one being looked at. Switching tabs or
//! folders replaces the watcher, and a search or a virtual page stops it.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

/// The window listens for this with the folder path as its payload.
pub const EVENT: &str = "folder-changed";

/// The shortest gap between two notices sent to the webview. A large copy can
/// produce hundreds of notifications a second; the window debounces what is
/// left, but there is no reason to send all of them across the bridge.
const NOTICE_EVERY: Duration = Duration::from_millis(100);

/// How long a native watcher waits before checking whether it has been asked to
/// stop. It is also the worst-case delay before the watcher thread goes away.
const WAIT_STEP: Duration = Duration::from_millis(200);

/// Polling bounds. A fingerprint costs one metadata read per entry, so the
/// interval grows with the folder: an empty folder can afford to be checked
/// often, a folder of twenty thousand rows cannot.
const POLL_MIN: Duration = Duration::from_millis(500);
const POLL_MAX: Duration = Duration::from_millis(3000);

#[derive(Default)]
pub struct WatchState(Mutex<Option<Running>>);

struct Running {
    path: String,
    stop: Arc<Stop>,
}

struct Stop(AtomicBool);

impl Stop {
    fn signalled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn signal(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// The identity of a folder's contents, small enough to hold between ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Stamp {
    hash: u64,
    entries: usize,
}

/// Names, sizes and modified times decide the answer. A file edited in place
/// changes its time or size but not the folder's own metadata, which is why
/// watching the directory's mtime alone was never enough.
fn fingerprint(folder: &Path) -> Option<Stamp> {
    let mut rows: Vec<(String, u64, u64, u8)> = Vec::new();
    for item in std::fs::read_dir(folder).ok()? {
        let item = item.ok()?;
        let (length, modified, kind) = match item.metadata() {
            Ok(metadata) => {
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                    .map(|span| span.as_millis() as u64)
                    .unwrap_or(0);
                let kind = if metadata.is_dir() { 1 } else { 0 };
                (metadata.len(), modified, kind)
            }
            // An entry that disappears between read_dir and metadata is a
            // change in its own right; a distinct marker keeps it stable.
            Err(_) => (0, 0, 2),
        };
        rows.push((item.file_name().to_string_lossy().into_owned(), length, modified, kind));
    }
    rows.sort_unstable();
    let mut hasher = DefaultHasher::new();
    rows.hash(&mut hasher);
    Some(Stamp {
        hash: hasher.finish(),
        entries: rows.len(),
    })
}

fn interval_for(entries: usize) -> Duration {
    let millis = POLL_MIN.as_millis() as u64 + entries as u64 / 8;
    Duration::from_millis(millis.min(POLL_MAX.as_millis() as u64))
}

/// Watch one folder, or pass `None` to stop. Calling this with the folder that
/// is already watched does nothing, so navigating within a tab is cheap.
pub fn set_folder(path: Option<String>, app: AppHandle, state: &WatchState) {
    let mut current = state
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let wanted = path.filter(|value| !value.is_empty());
    if current.as_ref().map(|running| running.path.as_str()) == wanted.as_deref() {
        return;
    }
    if let Some(running) = current.take() {
        running.stop.signal();
    }
    let Some(folder) = wanted else { return };

    let stop = Arc::new(Stop(AtomicBool::new(false)));
    let for_thread = Arc::clone(&stop);
    let watched = folder.clone();
    if std::thread::Builder::new()
        .name("lyre-watch".into())
        .spawn(move || run(&app, &watched, &for_thread))
        .is_err()
    {
        return;
    }
    *current = Some(Running { path: folder, stop });
}

fn run(app: &AppHandle, folder: &str, stop: &Arc<Stop>) {
    let mut last_notice = Instant::now() - Duration::from_secs(10);
    let mut notice = || {
        if last_notice.elapsed() < NOTICE_EVERY {
            return;
        }
        last_notice = Instant::now();
        let _ = app.emit(EVENT, folder.to_string());
    };

    #[cfg(windows)]
    let native = win::watch(Path::new(folder), stop, &mut notice);
    #[cfg(not(windows))]
    let native = Err("this platform has no native folder notifications".to_string());

    match native {
        // The watcher ended because it was asked to, not because it failed.
        Ok(()) => {}
        Err(reason) => {
            if std::env::var_os("LYRE_TRACE").is_some() {
                eprintln!("[lyre] polling {folder}: {reason}");
            }
            if !stop.signalled() {
                poll(Path::new(folder), stop, &mut notice);
            }
        }
    }
}

fn poll(folder: &Path, stop: &Arc<Stop>, notice: &mut dyn FnMut()) {
    let mut last = fingerprint(folder);
    while !stop.signalled() {
        let wait = last.as_ref().map_or(POLL_MAX, |stamp| interval_for(stamp.entries));
        // Sleep in slices so a folder change elsewhere is not held up by a long
        // uninterruptible sleep on the way out.
        let slices = (wait.as_millis() / WAIT_STEP.as_millis()).max(1);
        for _ in 0..slices {
            if stop.signalled() {
                return;
            }
            std::thread::sleep(WAIT_STEP);
        }
        if stop.signalled() {
            return;
        }
        let current = fingerprint(folder);
        if current.map(|stamp| stamp.hash) != last.map(|stamp| stamp.hash) {
            last = current;
            notice();
        }
    }
}

// ---------------------------------------------------------------------------
// Windows: ReadDirectoryChangesW, with one outstanding overlapped read.
//
// The stop flag is polled on a 200 ms wait rather than needing a second event
// to interrupt the wait. While a read is still outstanding on a timeout, the
// loop simply keeps waiting for it; it is only cancelled once on the way out,
// which is what keeps the buffer alive until the kernel has finished with it.
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod win {
    use super::{Stop, WAIT_STEP};
    use std::ffi::c_void;
    use std::path::Path;
    use std::sync::Arc;

    const INVALID_HANDLE_VALUE: usize = usize::MAX;
    const FILE_LIST_DIRECTORY: u32 = 0x0001;
    const FILE_SHARE_ALL: u32 = 0x0001 | 0x0002 | 0x0004;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
    // Names, attributes, size, last write and creation cover a file arriving,
    // leaving, being renamed, or being edited in place.
    const NOTIFY_FILTER: u32 = 0x01 | 0x02 | 0x04 | 0x08 | 0x10 | 0x40;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;
    const WAIT_FAILED: u32 = u32::MAX;
    const BUFFER_WORDS: usize = 16 * 1024; // 64 KiB, DWORD aligned

    #[repr(C)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        event: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: usize,
        ) -> usize;
        fn CreateEventW(
            security: *mut c_void,
            manual: i32,
            initial: i32,
            name: *const u16,
        ) -> usize;
        fn ResetEvent(event: usize) -> i32;
        fn CloseHandle(handle: usize) -> i32;
        fn ReadDirectoryChangesW(
            directory: usize,
            buffer: *mut u8,
            length: u32,
            subtree: i32,
            filter: u32,
            returned: *mut u32,
            overlapped: *mut Overlapped,
            routine: *mut c_void,
        ) -> i32;
        fn GetOverlappedResult(
            file: usize,
            overlapped: *mut Overlapped,
            transferred: *mut u32,
            wait: i32,
        ) -> i32;
        fn CancelIoEx(file: usize, overlapped: *mut Overlapped) -> i32;
        fn WaitForSingleObject(handle: usize, milliseconds: u32) -> u32;
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn error(prefix: &str) -> String {
        format!("{prefix} ({})", std::io::Error::last_os_error())
    }

    /// Call `changed` for every notification until `stop` is signalled.
    /// `Err` means the notification could not be set up or was lost; the caller
    /// falls back to polling rather than leaving the folder unwatched.
    pub fn watch(
        folder: &Path,
        stop: &Arc<Stop>,
        changed: &mut dyn FnMut(),
    ) -> Result<(), String> {
        let name = wide(&folder.to_string_lossy());
        let directory = unsafe {
            CreateFileW(
                name.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_ALL,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                0,
            )
        };
        if directory == INVALID_HANDLE_VALUE {
            return Err(error("the folder could not be opened for notifications"));
        }
        let event = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, std::ptr::null()) };
        if event == 0 {
            unsafe { CloseHandle(directory) };
            return Err(error("an event for the folder watcher could not be made"));
        }

        // u32 rather than u8: FILE_NOTIFY_INFORMATION needs DWORD alignment.
        let mut buffer = vec![0u32; BUFFER_WORDS];
        let fresh = || Overlapped {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            event,
        };
        let mut overlapped = fresh();
        let mut pending = false;
        let mut outcome = Ok(());

        loop {
            if stop.signalled() {
                break;
            }
            if !pending {
                overlapped = fresh();
                let started = unsafe {
                    ReadDirectoryChangesW(
                        directory,
                        buffer.as_mut_ptr() as *mut u8,
                        (buffer.len() * 4) as u32,
                        0,
                        NOTIFY_FILTER,
                        std::ptr::null_mut(),
                        &mut overlapped,
                        std::ptr::null_mut(),
                    )
                };
                if started == 0 {
                    outcome = Err(error("the folder watcher could not start"));
                    break;
                }
                pending = true;
            }

            let waited = unsafe { WaitForSingleObject(event, WAIT_STEP.as_millis() as u32) };
            match waited {
                WAIT_OBJECT_0 => {
                    let mut transferred = 0u32;
                    let completed = unsafe {
                        GetOverlappedResult(directory, &mut overlapped, &mut transferred, 1)
                    };
                    pending = false;
                    unsafe { ResetEvent(event) };
                    if completed == 0 {
                        if stop.signalled() {
                            break;
                        }
                        outcome = Err(error("the folder watcher lost its handle"));
                        break;
                    }
                    if transferred > 0 {
                        changed();
                    }
                }
                WAIT_TIMEOUT => {} // the stop flag is checked at the top
                WAIT_FAILED => {
                    outcome = Err(error("waiting for the folder failed"));
                    break;
                }
                _ => {}
            }
        }

        // The buffer must outlive any read still in flight. Cancelling and
        // collecting the result is what makes that true before we drop it.
        if pending {
            unsafe {
                CancelIoEx(directory, &mut overlapped);
                let mut transferred = 0u32;
                GetOverlappedResult(directory, &mut overlapped, &mut transferred, 1);
            }
        }
        unsafe {
            CloseHandle(event);
            CloseHandle(directory);
        }
        outcome
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc;
        use std::time::Duration;

        /// A real notification, run by hand because it depends on the machine:
        /// `cargo test a_saved_file_wakes_the_watcher -- --ignored --nocapture`.
        #[test]
        #[ignore]
        fn a_saved_file_wakes_the_watcher() {
            let folder = std::env::temp_dir().join(format!("lyre-watch-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).expect("a temporary folder");
            let stop = Arc::new(Stop(AtomicBool::new(false)));
            let (send, receive) = mpsc::channel();
            let watching = folder.clone();
            let thread_stop = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                let mut notice = || {
                    let _ = send.send(());
                };
                watch(&watching, &thread_stop, &mut notice)
            });

            std::thread::sleep(Duration::from_millis(150));
            std::fs::write(folder.join("new.txt"), b"first").expect("a new file");
            let arrived = receive.recv_timeout(Duration::from_secs(5));
            std::fs::write(folder.join("new.txt"), b"edited, longer").expect("an edit");
            let edited = receive.recv_timeout(Duration::from_secs(5));

            stop.signal();
            let result = thread.join().expect("the watcher thread");
            let _ = std::fs::remove_dir_all(&folder);
            assert!(arrived.is_ok(), "a new file must wake the watcher");
            assert!(edited.is_ok(), "an edit must wake the watcher");
            assert!(result.is_ok(), "the watcher ended on request: {result:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("lyre-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a temporary folder");
        folder
    }

    #[test]
    fn a_new_file_and_an_edit_both_change_the_fingerprint() {
        let folder = temporary("fingerprint");
        let empty = fingerprint(&folder);
        assert_eq!(empty, fingerprint(&folder), "an unchanged folder is stable");

        let file = folder.join("notes.txt");
        std::fs::write(&file, b"first").unwrap();
        let created = fingerprint(&folder);
        assert_ne!(created, empty, "a new file must be noticed");

        std::fs::write(&file, b"edited and longer").unwrap();
        assert_ne!(fingerprint(&folder), created, "an edit in place must be noticed");

        std::fs::remove_file(&file).unwrap();
        assert_eq!(fingerprint(&folder), empty, "removing it returns to the start");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_rename_and_a_new_folder_change_the_fingerprint() {
        let folder = temporary("rename");
        let file = folder.join("a.txt");
        std::fs::write(&file, b"same contents").unwrap();
        let before = fingerprint(&folder);

        std::fs::rename(&file, folder.join("b.txt")).unwrap();
        assert_ne!(fingerprint(&folder), before, "a rename must be noticed");
        let renamed = fingerprint(&folder);

        std::fs::create_dir(folder.join("subfolder")).unwrap();
        assert_ne!(fingerprint(&folder), renamed, "a folder must be noticed");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_missing_folder_is_not_the_same_as_an_empty_one() {
        let folder = temporary("missing");
        let empty = fingerprint(&folder);
        let _ = std::fs::remove_dir_all(&folder);
        assert_ne!(fingerprint(&folder), empty);
        assert!(fingerprint(&folder).is_none());
    }

    #[test]
    fn large_folders_are_checked_less_often() {
        assert_eq!(interval_for(0), POLL_MIN);
        assert_eq!(interval_for(20_000), POLL_MAX);
        assert!(interval_for(2_000) > interval_for(200));
    }
}
