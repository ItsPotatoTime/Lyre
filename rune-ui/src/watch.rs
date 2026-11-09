//! Notice when the folder on screen changes underneath the window.
//!
//! One blocked kernel wait per watched folder: ReadDirectoryChangesW on
//! Windows, inotify on Linux, a kqueue vnode note on macOS. Nothing polls while
//! the folder is quiet — an idle watch is one sleeping thread and no CPU. A
//! burst of notifications is coalesced into a single rescan off the UI thread,
//! and the window is woken only when there is something new to show. Volumes
//! that refuse a notification (network shares commonly do) fall back to
//! comparing a cheap fingerprint, so no platform loses the feature.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// A finished rescan of the watched folder, drained in `poll`.
pub struct Refreshed {
    pub path: PathBuf,
    pub entries: Vec<crate::Entry>,
    pub scan_ms: f64,
}

/// The quiet gap that ends a burst, and the longest a nonstop burst may
/// postpone the refresh: a copy of a thousand files is one refresh, not a
/// thousand, but it still lands while the copy is running.
const QUIET: Duration = Duration::from_millis(120);
const BURST_CAP: Duration = Duration::from_millis(1000);

/// Stops the watcher thread and interrupts its blocked wait. Dropping the
/// handle is the only way a watch ends; navigation replaces the handle.
pub struct Handle {
    stop: Arc<AtomicBool>,
    cancel: platform::Cancel,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.cancel.wake();
    }
}

/// Watch one folder until the returned handle is dropped. `wake` runs on the
/// watcher thread once a rescan is ready for `poll` to drain.
pub fn spawn(
    path: PathBuf,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> (Handle, mpsc::Receiver<Refreshed>) {
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let (backend, cancel) = match platform::Backend::new(&path) {
        Ok((backend, cancel)) => (Some(backend), cancel),
        // A volume that refuses notifications (network shares commonly do)
        // still gets live refresh, from a fingerprint compared off-thread.
        Err(_) => (None, platform::idle_cancel()),
    };
    let handle = Handle { stop: Arc::clone(&stop), cancel };
    let watched = path.clone();
    let _ = std::thread::Builder::new()
        .name("lyre-watch".into())
        .spawn(move || run(&watched, &stop, backend, &tx, &wake));
    (handle, rx)
}

/// Wait for changes, coalesce each burst, rescan and hand the listing over.
fn run(
    path: &Path,
    stop: &AtomicBool,
    backend: Option<platform::Backend>,
    tx: &mpsc::Sender<Refreshed>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) {
    let Some(mut backend) = backend else {
        return poll_fallback(path, stop, tx, wake);
    };
    let mut first: Option<Instant> = None;
    let mut last = Instant::now();
    while !stop.load(Ordering::Acquire) {
        // Idle: block forever. In a burst: wake at the quiet gap or the cap,
        // whichever comes first, so a nonstop writer cannot starve the refresh.
        let timeout = first.map(|start| {
            let deadline = (start + BURST_CAP).min(last + QUIET);
            deadline.saturating_duration_since(Instant::now())
        });
        let (event, dead) = backend.wait(timeout);
        if stop.load(Ordering::Acquire) {
            return;
        }
        if event {
            let now = Instant::now();
            first.get_or_insert(now);
            last = now;
            if now.duration_since(first.unwrap()) < BURST_CAP && !dead {
                continue;
            }
        } else if first.is_none() {
            return;
        }
        first = None;
        backend.rearm();
        if !flush(path, tx, wake) {
            return;
        }
        if dead {
            // The notification source is gone (folder deleted, watch limit):
            // a fingerprint keeps the folder live until navigation replaces us.
            return poll_fallback(path, stop, tx, wake);
        }
    }
}

/// One rescan and one wake; false when the window side is gone.
fn flush(
    path: &Path,
    tx: &mpsc::Sender<Refreshed>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) -> bool {
    let (entries, scan_ms) = crate::scan(path);
    if tx.send(Refreshed { path: path.to_path_buf(), entries, scan_ms }).is_err() {
        return false;
    }
    wake();
    true
}

/// The identity of a folder's contents: names, sizes and modified times. A
/// file edited in place changes its time or size but not the folder's own
/// metadata, which is why the directory mtime alone was never enough.
fn fingerprint(folder: &Path) -> Option<(u64, usize)> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut rows: Vec<(String, u64, u64, bool)> = Vec::new();
    for item in std::fs::read_dir(folder).ok()? {
        let item = item.ok()?;
        let meta = item.metadata().ok();
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|span| span.as_millis() as u64)
            .unwrap_or(0);
        rows.push((
            item.file_name().to_string_lossy().into_owned(),
            meta.as_ref().map_or(0, |m| m.len()),
            modified,
            meta.as_ref().is_some_and(|m| m.is_dir()),
        ));
    }
    rows.sort_unstable();
    let mut hasher = DefaultHasher::new();
    rows.hash(&mut hasher);
    Some((hasher.finish(), rows.len()))
}

/// Polling bounds: a fingerprint costs one metadata read per entry, so the
/// interval grows with the folder instead of taxing a twenty-thousand-row one.
fn poll_fallback(
    path: &Path,
    stop: &AtomicBool,
    tx: &mpsc::Sender<Refreshed>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) {
    let mut last = fingerprint(path);
    while !stop.load(Ordering::Acquire) {
        let entries = last.map_or(0, |(_, n)| n);
        let wait = Duration::from_millis((500 + entries as u64 / 8).min(3000));
        let deadline = Instant::now() + wait;
        // Sleep in slices so a drop is never held up by a long sleep.
        while Instant::now() < deadline {
            if stop.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100).min(deadline - Instant::now()));
        }
        let current = fingerprint(path);
        if current != last {
            last = current;
            if !flush(path, tx, wake) {
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Windows: one outstanding overlapped ReadDirectoryChangesW. The contents of
// the notification do not matter — every burst ends in a full rescan — so the
// buffer is never parsed, only drained.
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    const INVALID_HANDLE_VALUE: usize = usize::MAX;
    const FILE_LIST_DIRECTORY: u32 = 0x0001;
    const FILE_SHARE_ALL: u32 = 0x0001 | 0x0002 | 0x0004;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
    const NOTIFY_FILTER: u32 = 0x01 | 0x02 | 0x04 | 0x08 | 0x10 | 0x40;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;
    const INFINITE: u32 = u32::MAX;

    #[repr(C)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        event: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: usize,
        ) -> usize;
        fn CreateEventW(security: *mut c_void, manual: i32, initial: i32, name: *const u16) -> usize;
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

    /// Interrupts the blocked wait from the dropping thread.
    pub struct Cancel(Arc<AtomicUsize>);

    impl Cancel {
        pub fn wake(&self) {
            let directory = self.0.load(Ordering::Acquire);
            if directory != INVALID_HANDLE_VALUE {
                unsafe { CancelIoEx(directory, std::ptr::null_mut()) };
            }
        }
    }

    /// A cancel with nothing to interrupt, for the fingerprint fallback.
    pub fn idle_cancel() -> Cancel {
        Cancel(Arc::new(AtomicUsize::new(INVALID_HANDLE_VALUE)))
    }

    pub struct Backend {
        directory: usize,
        event: usize,
        buffer: Vec<u32>,
        overlapped: Overlapped,
        pending: bool,
        cancel: Arc<AtomicUsize>,
    }

    impl Backend {
        pub fn new(path: &Path) -> Result<(Self, Cancel), String> {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            let directory = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    FILE_LIST_DIRECTORY,
                    FILE_SHARE_ALL,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                    0,
                )
            };
            if directory == INVALID_HANDLE_VALUE {
                return Err("the folder could not be opened for notifications".into());
            }
            let event = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, std::ptr::null()) };
            if event == 0 {
                unsafe { CloseHandle(directory) };
                return Err("a watcher event could not be created".into());
            }
            let cancel = Arc::new(AtomicUsize::new(directory));
            let mut backend = Backend {
                directory,
                event,
                // DWORD-aligned: FILE_NOTIFY_INFORMATION would need it.
                buffer: vec![0u32; 16 * 1024],
                overlapped: Overlapped { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event },
                pending: false,
                cancel: Arc::clone(&cancel),
            };
            // Arm before spawn returns: a change racing the thread start must
            // not fall between the folder opening and the first read.
            if !backend.issue() {
                unsafe {
                    CloseHandle(event);
                    CloseHandle(directory);
                }
                return Err("the folder could not be read for notifications".into());
            }
            Ok((backend, Cancel(cancel)))
        }

        fn issue(&mut self) -> bool {
            self.overlapped.internal = 0;
            self.overlapped.internal_high = 0;
            let started = unsafe {
                ReadDirectoryChangesW(
                    self.directory,
                    self.buffer.as_mut_ptr().cast(),
                    (self.buffer.len() * 4) as u32,
                    0,
                    NOTIFY_FILTER,
                    std::ptr::null_mut(),
                    &mut self.overlapped,
                    std::ptr::null_mut(),
                )
            };
            self.pending = started != 0;
            self.pending
        }

        /// Put the next read outstanding before the rescan is handed over, so
        /// changes during the scan are not lost either.
        pub fn rearm(&mut self) {
            if !self.pending {
                self.issue();
            }
        }

        /// True when a notification landed; the bool beside it marks the
        /// source dead (cancelled reads included, which the stop flag owns).
        pub fn wait(&mut self, timeout: Option<Duration>) -> (bool, bool) {
            if !self.pending && !self.issue() {
                return (false, true);
            }
            let millis = timeout.map_or(INFINITE, |d| d.as_millis().min(u32::MAX as u128) as u32);
            let waited = unsafe { WaitForSingleObject(self.event, millis) };
            if waited == WAIT_TIMEOUT {
                return (false, false);
            }
            if waited != WAIT_OBJECT_0 {
                return (false, true);
            }
            let mut returned = 0u32;
            let ok = unsafe {
                GetOverlappedResult(self.directory, &mut self.overlapped, &mut returned, 0)
            };
            self.pending = false;
            unsafe { ResetEvent(self.event) };
            if ok == 0 {
                // ERROR_OPERATION_ABORTED from a drop is the stop flag's job;
                // anything else (buffer overflow included) still means change.
                let aborted = std::io::Error::last_os_error().raw_os_error() == Some(995);
                return (!aborted, aborted);
            }
            // returned == 0 means the buffer overflowed: changes happened.
            (true, false)
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            self.cancel.store(INVALID_HANDLE_VALUE, Ordering::Release);
            unsafe {
                CancelIoEx(self.directory, std::ptr::null_mut());
                CloseHandle(self.event);
                CloseHandle(self.directory);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: inotify, with an eventfd beside it so a drop interrupts the poll()
// without a timeout wakeup. Idle cost is one thread blocked in the kernel.
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::{c_char, c_void, CString};
    use std::os::raw::c_ulong;
    use std::path::Path;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    const IN_MODIFY: u32 = 0x0000_0002;
    const IN_ATTRIB: u32 = 0x0000_0004;
    const IN_MOVED_FROM: u32 = 0x0000_0040;
    const IN_MOVED_TO: u32 = 0x0000_0080;
    const IN_CREATE: u32 = 0x0000_0100;
    const IN_DELETE: u32 = 0x0000_0200;
    const IN_DELETE_SELF: u32 = 0x0000_0400;
    const IN_MOVE_SELF: u32 = 0x0000_0800;
    const IN_NONBLOCK: i32 = 0x800;
    const IN_CLOEXEC: i32 = 0x80000;
    const EFD_NONBLOCK: i32 = 0x800;
    const EFD_CLOEXEC: i32 = 0x80000;
    const POLLIN: i16 = 0x001;

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    unsafe extern "C" {
        fn inotify_init1(flags: i32) -> i32;
        fn inotify_add_watch(fd: i32, path: *const c_char, mask: u32) -> i32;
        fn eventfd(initial: u32, flags: i32) -> i32;
        fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: i32) -> i32;
        fn read(fd: i32, buf: *mut c_void, count: usize) -> isize;
        fn write(fd: i32, buf: *const c_void, count: usize) -> isize;
        fn close(fd: i32) -> i32;
    }

    pub struct Cancel(Arc<AtomicI32>);

    impl Cancel {
        pub fn wake(&self) {
            let fd = self.0.load(Ordering::Acquire);
            if fd >= 0 {
                unsafe { write(fd, [1u64].as_ptr().cast(), 8) };
            }
        }
    }

    /// A cancel with nothing to interrupt, for the fingerprint fallback.
    pub fn idle_cancel() -> Cancel {
        Cancel(Arc::new(AtomicI32::new(-1)))
    }

    pub struct Backend {
        notify: i32,
        stop_fd: Arc<AtomicI32>,
    }

    impl Backend {
        pub fn new(path: &Path) -> Result<(Self, Cancel), String> {
            let notify = unsafe { inotify_init1(IN_NONBLOCK | IN_CLOEXEC) };
            if notify < 0 {
                return Err("inotify could not be opened".into());
            }
            let name = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
            let mask = IN_CREATE | IN_DELETE | IN_MODIFY | IN_ATTRIB
                | IN_MOVED_FROM | IN_MOVED_TO | IN_DELETE_SELF | IN_MOVE_SELF;
            if unsafe { inotify_add_watch(notify, name.as_ptr(), mask) } < 0 {
                unsafe { close(notify) };
                return Err("the folder could not be watched".into());
            }
            let stop_fd = unsafe { eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC) };
            if stop_fd < 0 {
                unsafe { close(notify) };
                return Err("a watcher stop channel could not be opened".into());
            }
            let stop_fd = Arc::new(AtomicI32::new(stop_fd));
            Ok((Backend { notify, stop_fd: Arc::clone(&stop_fd) }, Cancel(stop_fd)))
        }

        /// The inotify watch stays armed across rescans; nothing to re-issue.
        pub fn rearm(&mut self) {}

        pub fn wait(&mut self, timeout: Option<Duration>) -> (bool, bool) {
            let stop = self.stop_fd.load(Ordering::Acquire);
            let mut fds = [
                PollFd { fd: self.notify, events: POLLIN, revents: 0 },
                PollFd { fd: stop, events: POLLIN, revents: 0 },
            ];
            let millis = timeout.map_or(-1, |d| d.as_millis().min(i32::MAX as u128) as i32);
            let ready = unsafe { poll(fds.as_mut_ptr(), 2, millis) };
            if ready <= 0 {
                return (false, ready < 0);
            }
            if fds[1].revents != 0 {
                return (false, false);
            }
            if fds[0].revents == 0 {
                return (false, false);
            }
            // Drain the queue; the names do not matter (the rescan reads the
            // whole folder), but IN_IGNORED means the watch itself is gone.
            let mut buf = [0u8; 4096];
            let mut dead = false;
            loop {
                let got = unsafe { read(self.notify, buf.as_mut_ptr().cast(), buf.len()) };
                if got <= 0 {
                    break;
                }
                let mut at = 0usize;
                while at + 16 <= got as usize {
                    let mask = u32::from_ne_bytes(buf[at + 4..at + 8].try_into().unwrap());
                    let len = u32::from_ne_bytes(buf[at + 12..at + 16].try_into().unwrap()) as usize;
                    dead |= mask & 0x0000_8000 != 0; // IN_IGNORED
                    at += 16 + len;
                }
            }
            (true, dead || fds[0].revents & !POLLIN != 0)
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            let stop = self.stop_fd.swap(-1, Ordering::AcqRel);
            unsafe {
                close(self.notify);
                if stop >= 0 {
                    close(stop);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// macOS: a kqueue vnode note on the directory. NOTE_WRITE fires when entries
// are linked or unlinked, which is exactly when a listing goes stale. An
// EVFILT_USER note interrupts the wait from the dropping thread.
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_void, CString};
    use std::path::Path;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    const O_RDONLY: i32 = 0;
    const O_EVTONLY: i32 = 0x8000;
    const EVFILT_VNODE: i16 = -4;
    const EVFILT_USER: i16 = -10;
    const EV_ADD: u16 = 0x0001;
    const EV_ENABLE: u16 = 0x0010;
    const NOTE_WRITE: u32 = 0x0000_0002;
    const NOTE_DELETE: u32 = 0x0000_0001;
    const NOTE_ATTRIB: u32 = 0x0000_0008;
    const NOTE_LINK: u32 = 0x0000_0010;
    const NOTE_RENAME: u32 = 0x0000_0004;
    const NOTE_TRIGGER: u32 = 0x0100_0000;

    #[repr(C)]
    struct Kevent {
        ident: usize,
        filter: i16,
        flags: u16,
        fflags: u32,
        data: isize,
        udata: *mut c_void,
    }

    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }

    unsafe extern "C" {
        fn open(path: *const c_char, flags: i32, ...) -> i32;
        fn close(fd: i32) -> i32;
        fn kqueue() -> i32;
        fn kevent(
            kq: i32,
            changelist: *const Kevent,
            nchanges: i32,
            eventlist: *mut Kevent,
            nevents: i32,
            timeout: *const Timespec,
        ) -> i32;
    }

    pub struct Cancel(Arc<AtomicI32>);

    impl Cancel {
        pub fn wake(&self) {
            let kq = self.0.load(Ordering::Acquire);
            if kq >= 0 {
                let trigger = Kevent {
                    ident: 1,
                    filter: EVFILT_USER,
                    flags: EV_ADD | EV_ENABLE,
                    fflags: NOTE_TRIGGER,
                    data: 0,
                    udata: std::ptr::null_mut(),
                };
                unsafe { kevent(kq, &trigger, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
            }
        }
    }

    /// A cancel with nothing to interrupt, for the fingerprint fallback.
    pub fn idle_cancel() -> Cancel {
        Cancel(Arc::new(AtomicI32::new(-1)))
    }

    pub struct Backend {
        dir: i32,
        kq: Arc<AtomicI32>,
    }

    impl Backend {
        pub fn new(path: &Path) -> Result<(Self, Cancel), String> {
            let name = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
            let dir = unsafe { open(name.as_ptr(), O_RDONLY | O_EVTONLY) };
            if dir < 0 {
                return Err("the folder could not be opened for notifications".into());
            }
            let kq = unsafe { kqueue() };
            if kq < 0 {
                unsafe { close(dir) };
                return Err("a kqueue could not be opened".into());
            }
            let note = Kevent {
                ident: dir as usize,
                filter: EVFILT_VNODE,
                flags: EV_ADD | EV_ENABLE,
                fflags: NOTE_WRITE | NOTE_DELETE | NOTE_ATTRIB | NOTE_LINK | NOTE_RENAME,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            let user = Kevent {
                ident: 1,
                filter: EVFILT_USER,
                flags: EV_ADD | EV_ENABLE,
                fflags: 0,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            let changes = [note, user];
            if unsafe { kevent(kq, changes.as_ptr(), 2, std::ptr::null_mut(), 0, std::ptr::null()) } < 0 {
                unsafe {
                    close(kq);
                    close(dir);
                }
                return Err("the folder could not be watched".into());
            }
            let kq = Arc::new(AtomicI32::new(kq));
            Ok((Backend { dir, kq: Arc::clone(&kq) }, Cancel(kq)))
        }

        /// The vnode note stays armed across rescans; nothing to re-issue.
        pub fn rearm(&mut self) {}

        pub fn wait(&mut self, timeout: Option<Duration>) -> (bool, bool) {
            let kq = self.kq.load(Ordering::Acquire);
            let spec = timeout.map(|d| Timespec {
                sec: d.as_secs() as i64,
                nsec: d.subsec_nanos() as i64,
            });
            let mut out = Kevent { ident: 0, filter: 0, flags: 0, fflags: 0, data: 0, udata: std::ptr::null_mut() };
            let ready = unsafe {
                kevent(
                    kq,
                    std::ptr::null(),
                    0,
                    &mut out,
                    1,
                    spec.as_ref().map_or(std::ptr::null(), |s| s),
                )
            };
            if ready <= 0 {
                return (false, ready < 0);
            }
            // NOTE_DELETE: the directory itself is gone; the vnode can say
            // nothing more, so the fingerprint takes over after the rescan.
            (out.filter == EVFILT_VNODE, out.fflags & NOTE_DELETE != 0)
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            let kq = self.kq.swap(-1, Ordering::AcqRel);
            unsafe {
                if kq >= 0 {
                    close(kq);
                }
                close(self.dir);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fallback for volumes with no notification source: compare a fingerprint of
// names, sizes and modified times, at an interval that grows with the folder.
// ---------------------------------------------------------------------------
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod platform {
    use std::path::Path;
    use std::time::Duration;

    pub struct Cancel;

    impl Cancel {
        pub fn wake(&self) {}
    }

    pub fn idle_cancel() -> Cancel {
        Cancel
    }

    pub struct Backend;

    impl Backend {
        pub fn new(_path: &Path) -> Result<(Self, Cancel), String> {
            Err("no native folder notifications on this platform".into())
        }

        pub fn rearm(&mut self) {}

        pub fn wait(&mut self, _timeout: Option<Duration>) -> (bool, bool) {
            (false, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lyre-watch-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn recv(rx: &mpsc::Receiver<Refreshed>) -> Refreshed {
        rx.recv_timeout(Duration::from_secs(10)).expect("the watch must report the change")
    }

    #[test]
    fn creates_and_deletes_arrive_and_dropping_stops_the_thread() {
        let dir = temp_dir("basic");
        let (handle, rx) = spawn(dir.clone(), Arc::new(|| {}));
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        let msg = recv(&rx);
        assert_eq!(msg.path, dir);
        assert!(msg.entries.iter().any(|e| e.name == "a.txt"));
        std::fs::remove_file(dir.join("a.txt")).unwrap();
        let msg = recv(&rx);
        assert!(!msg.entries.iter().any(|e| e.name == "a.txt"));
        drop(handle);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ), "the watcher thread must retire when its handle drops");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_burst_of_writes_is_coalesced_not_one_notice_per_file() {
        let dir = temp_dir("burst");
        let (_handle, rx) = spawn(dir.clone(), Arc::new(|| {}));
        for i in 0..40 {
            std::fs::write(dir.join(format!("f{i:02}.txt")), b"x").unwrap();
        }
        let mut names = std::collections::HashSet::new();
        let mut notices = 0;
        while let Ok(msg) = rx.recv_timeout(Duration::from_millis(600)) {
            notices += 1;
            names.extend(msg.entries.iter().map(|e| e.name.clone()));
        }
        assert_eq!(names.len(), 40, "every file of the burst must land");
        assert!(notices <= 4, "40 writes were {notices} notices, not a coalesced burst");
        let _ = std::fs::remove_dir_all(&dir);
    }
}