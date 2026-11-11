//! Drag sessions with the operating system, and the staging that makes them
//! safe. Archivers and browsers drag from throwaway folders: 7-Zip extracts
//! members into `%TEMP%` and deletes them the moment `DoDragDrop` returns, so
//! paths collected asynchronously name bytes that are already gone. Anything
//! under the system temp directory is therefore snapshotted (hard links where
//! the volume allows, byte copies otherwise) before the source can unlink it;
//! the snapshot is released once the paste finishes.
//!
//! Windows runs its own `IDropTarget` so the snapshot happens inside `Drop`,
//! the one moment the bytes are guaranteed to exist, and understands the
//! virtual-file formats (`FileDescriptorW`/`FileContents`) archivers and mail
//! clients use instead of `CF_HDROP`. A drag that leaves the window hands the
//! same gesture to `DoDragDrop` on the pressing thread, carrying the shell's
//! own data object, which is the one OLE accepts as a drag source.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A drop handed over by the OS thread, with the point it landed at.
pub struct OsDrop {
    pub paths: Vec<PathBuf>,
    pub at: (f32, f32),
}

/// What the OS drag session tells the window: where the drag hovers, and the
/// final drop. Points are screen pixels, the drop's included.
pub enum OsDragMsg {
    Hover(Option<(f32, f32)>),
    Drop(OsDrop),
}

/// The picture an OS drag shows under the pointer, as straight-alpha sRGB
/// RGBA8: the item's own icon. Lyre's in-window ghost cannot follow the pointer
/// past the window edge, so the OS draws this instead once the gesture leaves.
#[cfg(target_os = "windows")]
pub struct DragPreview {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

static SENDER: std::sync::Mutex<Option<std::sync::mpsc::Sender<OsDragMsg>>> =
    std::sync::Mutex::new(None);

/// The window's drop target is registered at native creation, after `init`
/// built the channel; this is how the two meet. The last built app is the one
/// that runs (the startup scan builds an app too), so its sender wins: keeping
/// the first instead left every OS drop on a channel nobody read.
pub fn set_sender(tx: std::sync::mpsc::Sender<OsDragMsg>) {
    if let Ok(mut sender) = SENDER.lock() {
        *sender = Some(tx);
    }
}

pub fn sender() -> Option<std::sync::mpsc::Sender<OsDragMsg>> {
    SENDER.lock().ok()?.clone()
}

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
const PREFIX: &str = "lyre-drop-";
const MAX_STAGED_ITEMS: usize = 4096;

/// Whether a path is ephemeral: any source under the system temp directory may
/// be deleted by its drag source as soon as the drop ends.
pub fn ephemeral(path: &Path) -> bool {
    path.starts_with(std::env::temp_dir())
}

/// Snapshot the ephemeral paths into a private folder and return the paths to
/// copy from (non-temp paths pass through untouched). Hard links keep the bytes
/// alive after the source unlinks its own name without copying data; volumes
/// that cannot link fall back to a byte copy.
pub fn stage(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut root: Option<PathBuf> = None;
    let mut budget = MAX_STAGED_ITEMS;
    for path in paths {
        if !ephemeral(path) {
            out.push(path.clone());
            continue;
        }
        let root = root.get_or_insert_with(|| {
            std::env::temp_dir().join(format!(
                "{PREFIX}{}-{}",
                std::process::id(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ))
        });
        let name = path
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("item"));
        let dst = root.join(name);
        if snapshot(path, &dst, &mut budget) {
            out.push(dst);
        }
    }
    out
}

fn snapshot(src: &Path, dst: &Path, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    let meta = match std::fs::metadata(src) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if meta.is_dir() {
        if std::fs::create_dir_all(dst).is_err() {
            return false;
        }
        let Ok(reader) = std::fs::read_dir(src) else { return false };
        for item in reader.flatten() {
            let name = item.file_name();
            snapshot(&item.path(), &dst.join(name), budget);
        }
        true
    } else {
        if let Some(parent) = dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::hard_link(src, dst).is_ok() {
            return true;
        }
        std::fs::copy(src, dst).is_ok()
    }
}

/// Release a staging root once its paste finished; only folders this module
/// created are ever removed.
pub fn release(paths: &[PathBuf]) {
    for path in paths {
        for ancestor in path.ancestors() {
            if ancestor
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(PREFIX))
            {
                let _ = std::fs::remove_dir_all(ancestor);
                break;
            }
        }
    }
}

/// Sweep staging folders a crashed run left behind.
pub fn sweep_stale() {
    let Ok(reader) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for item in reader.flatten() {
        let name = item.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(PREFIX) {
            continue;
        }
        let old = item
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(6 * 60 * 60));
        if old {
            let _ = std::fs::remove_dir_all(item.path());
        }
    }
}

// ---------------------------------------------------------------------------
// Windows: the drop target and the drag source.
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
pub mod win {
    use super::{DragPreview, OsDragMsg, OsDrop, stage};
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::Sender;

    const CF_HDROP: u16 = 15;
    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;
    const S_OK: i32 = 0;
    const DRAGDROP_S_DROP: i32 = 0x00040100;
    const DRAGDROP_S_CANCEL: i32 = 0x00040101;
    const DRAGDROP_S_USEDEFAULTCURSORS: i32 = 0x00040102;
    const TYMED_ISTREAM: u32 = 4;
    const MK_LBUTTON: u32 = 1;
    const RDW_INTERNALPAINT: u32 = 0x0002;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    pub struct DropFiles {
        pub p_files: u32,
        pt: Point,
        f_nc: i32,
        pub f_wide: i32,
    }

    #[repr(C)]
    struct FormatEtc {
        cf_format: u16,
        /// `DVTARGETDEVICE *`: COM reads it at the pointer's own alignment, so
        /// the clip format is followed by padding, not by the next DWORD.
        _ptd: *mut c_void,
        _aspect: u32,
        _lindex: i32,
        tymed: u32,
    }

    #[repr(C)]
    struct StgMedium {
        tymed: u32,
        _pad: u32,
        data: *mut c_void,
        _unk: *mut c_void,
    }

    #[repr(C)]
    pub struct FileDescriptor {
        flags: u32,
        _clsid: [u8; 16],
        _sizel: [i32; 2],
        _pointl: [i32; 2],
        _attributes: u32,
        _created: u64,
        _accessed: u64,
        _written: u64,
        size_high: u32,
        size_low: u32,
        name: [u16; 260],
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn OleInitialize(reserved: *mut c_void) -> i32;
        fn RegisterDragDrop(hwnd: isize, target: *mut c_void) -> i32;
        fn RevokeDragDrop(hwnd: isize) -> i32;
        fn ReleaseStgMedium(medium: *mut StgMedium);
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalLock(handle: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(handle: *mut c_void) -> i32;
        fn GlobalSize(handle: *mut c_void) -> usize;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClipboardFormatW(name: *const u16) -> u16;
        fn ReleaseCapture() -> i32;
        fn ScreenToClient(hwnd: isize, pt: *mut Point) -> i32;
        fn GetForegroundWindow() -> isize;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn GetAsyncKeyState(vkey: i32) -> i16;
        fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn RedrawWindow(hwnd: isize, rect: *const c_void, region: *mut c_void, flags: u32) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    /// OLE tracks the pointer itself; the UI thread must let go of the Win32
    /// capture first, or the drag loop never sees a move and the drag dies.
    pub fn release_capture() {
        unsafe { ReleaseCapture() };
    }

    /// Whether the left button is still down. A handoff deferred by one frame
    /// has to check: a release that arrives first must cancel the drag, not
    /// start one that drops on whatever window is under the pointer.
    pub fn button_held() -> bool {
        unsafe { GetAsyncKeyState(0x01) as u16 & 0x8000 != 0 }
    }

    /// IDropTarget points are screen pixels; the hit test wants window points.
    pub fn to_window(hwnd: isize, x: f32, y: f32, scale: f32) -> (f32, f32) {
        let mut pt = Point { x: x as i32, y: y as i32 };
        unsafe { ScreenToClient(hwnd, &mut pt) };
        (pt.x as f32 / scale, pt.y as f32 / scale)
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn format_name(name: &str) -> u16 {
        unsafe { RegisterClipboardFormatW(wide(name).as_ptr()) }
    }

    /// Paths out of a `CF_HDROP` global, as the shell's own drop format carries
    /// them: a DROPFILES header then double-null-terminated names.
    pub fn parse_hdrop(bytes: &[u8]) -> Vec<PathBuf> {
        if bytes.len() < std::mem::size_of::<DropFiles>() {
            return Vec::new();
        }
        let header: &DropFiles = unsafe { &*(bytes.as_ptr() as *const DropFiles) };
        let mut out = Vec::new();
        let mut at = header.p_files as usize;
        if header.f_wide != 0 {
            while at + 2 <= bytes.len() {
                let mut unit = [0u16; 1];
                unit[0] = u16::from_ne_bytes([bytes[at], bytes[at + 1]]);
                if unit[0] == 0 {
                    break;
                }
                let start = at;
                while at + 2 <= bytes.len() {
                    let u = u16::from_ne_bytes([bytes[at], bytes[at + 1]]);
                    at += 2;
                    if u == 0 {
                        break;
                    }
                }
                let units: Vec<u16> = bytes[start..at - 2]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_ne_bytes(*c))
                    .collect();
                if let Ok(name) = String::from_utf16(&units) {
                    out.push(PathBuf::from(name));
                }
            }
        }
        out
    }

    #[repr(C)]
    struct Target {
        // COM reads the vtable pointer at offset zero; the refcount follows.
        #[allow(dead_code)]
        vtable: &'static TargetVtable,
        refs: AtomicUsize,
        tx: Sender<OsDragMsg>,
        descriptor: u16,
        contents: u16,
        hwnd: isize,
        /// What DragEnter accepted; DragOver carries no data object to re-ask.
        accepted: bool,
    }

    #[repr(C)]
    struct TargetVtable {
        query: unsafe extern "system" fn(*mut Target, *const c_void, *mut *mut c_void) -> i32,
        add: unsafe extern "system" fn(*mut Target) -> u32,
        release: unsafe extern "system" fn(*mut Target) -> u32,
        enter: unsafe extern "system" fn(*mut Target, *mut c_void, u32, Point, *mut u32) -> i32,
        over: unsafe extern "system" fn(*mut Target, u32, Point, *mut u32) -> i32,
        leave: unsafe extern "system" fn(*mut Target) -> i32,
        drop: unsafe extern "system" fn(*mut Target, *mut c_void, u32, Point, *mut u32) -> i32,
    }

    unsafe extern "system" fn query(this: *mut Target, _iid: *const c_void, out: *mut *mut c_void) -> i32 {
        unsafe {
            if std::env::var_os("LYRE_TRACE").is_some() {
                eprintln!("[drop] QueryInterface");
            }
            *out = this.cast();
            (*this).refs.fetch_add(1, Ordering::Relaxed);
        }
        S_OK
    }
    unsafe extern "system" fn add(this: *mut Target) -> u32 {
        unsafe {
            if std::env::var_os("LYRE_TRACE").is_some() {
                eprintln!("[drop] AddRef");
            }
            (*this).refs.fetch_add(1, Ordering::Relaxed) as u32 + 1
        }
    }
    unsafe extern "system" fn release(this: *mut Target) -> u32 {
        unsafe {
            if std::env::var_os("LYRE_TRACE").is_some() {
                eprintln!("[drop] Release");
            }
            let left = (*this).refs.fetch_sub(1, Ordering::Relaxed) as u32 - 1;
            if left == 0 {
                drop(Box::from_raw(this));
            }
            left
        }
    }
    /// Tell the window where the drag is, and wake it: the message alone is
    /// only drained by a frame, and OLE's loop asks for one through this
    /// redraw. A drag the target refused lights nothing.
    unsafe fn hover(this: *mut Target, pt: Option<Point>) {
        unsafe {
            let at = pt.map(|p| (p.x as f32, p.y as f32));
            let _ = (*this).tx.send(OsDragMsg::Hover(at));
            RedrawWindow((*this).hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INTERNALPAINT);
        }
    }

    unsafe extern "system" fn enter(
        this: *mut Target,
        data: *mut c_void,
        _keys: u32,
        pt: Point,
        effect: *mut u32,
    ) -> i32 {
        unsafe {
            (*this).accepted = accepts(data, (*this).descriptor);
            *effect = if (*this).accepted { DROPEFFECT_COPY } else { 0 };
            hover(this, (*this).accepted.then_some(pt));
            if std::env::var_os("LYRE_TRACE").is_some() {
                eprintln!("[drop] DragEnter effect={} pt=({},{})", *effect, pt.x, pt.y);
            }
        }
        S_OK
    }
    unsafe extern "system" fn over(this: *mut Target, _keys: u32, pt: Point, effect: *mut u32) -> i32 {
        unsafe {
            *effect = if (*this).accepted { DROPEFFECT_COPY } else { 0 };
            hover(this, (*this).accepted.then_some(pt));
        }
        if std::env::var_os("LYRE_TRACE").is_some() {
            eprintln!("[drop] DragOver");
        }
        S_OK
    }
    unsafe extern "system" fn leave(this: *mut Target) -> i32 {
        unsafe { hover(this, None) };
        if std::env::var_os("LYRE_TRACE").is_some() {
            eprintln!("[drop] DragLeave");
        }
        S_OK
    }

    /// The one moment the bytes are guaranteed to exist: inside Drop, before
    /// the source's DoDragDrop returns and deletes its temp tree.
    unsafe extern "system" fn drop_it(
        this: *mut Target,
        data: *mut c_void,
        _keys: u32,
        pt: Point,
        effect: *mut u32,
    ) -> i32 {
        unsafe {
            let paths = collect(data, (*this).descriptor, (*this).contents);
            *effect = if paths.is_empty() { 0 } else { DROPEFFECT_COPY };
            hover(this, None);
            if std::env::var_os("LYRE_TRACE").is_some() {
                eprintln!("[drop] Drop pt=({},{}) paths={} first={:?}", pt.x, pt.y, paths.len(), paths.first());
            }
            if !paths.is_empty() {
                let staged = stage(&paths);
                if std::env::var_os("LYRE_TRACE").is_some() {
                    eprintln!("[drop] staged={} first={:?}", staged.len(), staged.first());
                }
                let _ = (*this).tx.send(OsDragMsg::Drop(OsDrop { paths: staged, at: (pt.x as f32, pt.y as f32) }));
            }
        }
        S_OK
    }

    static TARGET_VTABLE: TargetVtable = TargetVtable {
        query,
        add,
        release,
        enter,
        over,
        leave,
        drop: drop_it,
    };

    /// Whether a data object carries something we can take.
    unsafe fn accepts(data: *mut c_void, descriptor: u16) -> bool {
        unsafe { has_format(data, CF_HDROP) || has_format(data, descriptor) }
    }

    unsafe fn has_format(data: *mut c_void, format: u16) -> bool {
        unsafe {
            let mut etc = FormatEtc { cf_format: format, _ptd: std::ptr::null_mut(), tymed: TYMED_ISTREAM, _aspect: 1, _lindex: -1 };
            let vtable = *(data as *const *const DataVtable);
            ((*vtable).query_get_data)(data, &mut etc) == S_OK
                || {
                    etc.tymed = 1; // TYMED_HGLOBAL
                    ((*vtable).query_get_data)(data, &mut etc) == S_OK
                }
        }
    }

    unsafe fn get_medium(data: *mut c_void, format: u16, index: i32, tymed: u32) -> Option<StgMedium> {
        unsafe {
            let mut etc = FormatEtc { cf_format: format, _ptd: std::ptr::null_mut(), tymed, _aspect: 1, _lindex: index };
            let mut medium: StgMedium = std::mem::zeroed();
            let vtable = *(data as *const *const DataVtable);
            if ((*vtable).get_data)(data, &mut etc, &mut medium) != S_OK {
                return None;
            }
            Some(medium)
        }
    }

    /// Member names out of a `FILEGROUP_DESCRIPTORW` global. The descriptor
    /// array starts at offset 8: the DWORD count is followed by padding to
    /// the FILE_DESCRIPTORW's eight-byte alignment.
    pub fn descriptor_names(bytes: &[u8]) -> Vec<String> {
        if bytes.len() < 8 {
            return Vec::new();
        }
        let count = u32::from_ne_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let mut out = Vec::new();
        let stride = std::mem::size_of::<FileDescriptor>();
        for i in 0..count.min(super::MAX_STAGED_ITEMS) {
            let at = 8 + i * stride;
            if at + stride > bytes.len() {
                break;
            }
            let d: &FileDescriptor = unsafe { &*(bytes.as_ptr().add(at) as *const FileDescriptor) };
            let units: Vec<u16> = d.name.iter().take_while(|&&u| u != 0).copied().collect();
            if let Ok(name) = String::from_utf16(&units)
                && !name.is_empty()
            {
                out.push(name);
            }
        }
        out
    }

    /// Every path or virtual member the source offers, extracted in full.
    unsafe fn collect(data: *mut c_void, descriptor: u16, contents: u16) -> Vec<PathBuf> {
        unsafe {
            if let Some(medium) = get_medium(data, CF_HDROP, -1, 1) {
                let ptr = GlobalLock(medium.data);
                let size = GlobalSize(medium.data);
                let paths = if !ptr.is_null() && size > 0 {
                    parse_hdrop(std::slice::from_raw_parts(ptr as *const u8, size))
                } else {
                    Vec::new()
                };
                GlobalUnlock(medium.data);
                ReleaseStgMedium(&mut { medium });
                if !paths.is_empty() {
                    return paths;
                }
            }
            // Virtual files: one stream per member, read to the end so an
            // archive arrives whole, not just its first folder.
            let Some(group) = get_medium(data, descriptor, -1, 1) else { return Vec::new() };
            let ptr = GlobalLock(group.data);
            if ptr.is_null() {
                ReleaseStgMedium(&mut { group });
                return Vec::new();
            }
            let count_bytes = GlobalSize(group.data);
            let names = descriptor_names(std::slice::from_raw_parts(ptr as *const u8, count_bytes));
            let root = std::env::temp_dir().join(format!(
                "lyre-virtual-{}",
                super::NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut out = Vec::new();
            for (i, name) in names.iter().enumerate() {
                let name = name.replace('/', "\\");
                let dst = root.join(name.trim_start_matches('\\'));
                let Some(stream_medium) = get_medium(data, contents, i as i32, TYMED_ISTREAM) else { continue };
                let stream = stream_medium.data as *mut *const StreamVtable;
                if let Some(parent) = dst.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let Ok(mut file) = std::fs::File::create(&dst) else { continue };
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let mut read = 0u32;
                    let hr = ((*(*stream)).read)(stream as *mut c_void, buf.as_mut_ptr().cast(), buf.len() as u32, &mut read);
                    if hr != S_OK || read == 0 {
                        break;
                    }
                    use std::io::Write;
                    if file.write_all(&buf[..read as usize]).is_err() {
                        break;
                    }
                }
                ReleaseStgMedium(&mut { stream_medium });
                out.push(dst);
            }
            GlobalUnlock(group.data);
            ReleaseStgMedium(&mut { group });
            out
        }
    }

    #[repr(C)]
    struct DataVtable {
        query: *const c_void,
        add: *const c_void,
        release: *const c_void,
        get_data: unsafe extern "system" fn(*mut c_void, *mut FormatEtc, *mut StgMedium) -> i32,
        get_data_here: *const c_void,
        query_get_data: unsafe extern "system" fn(*mut c_void, *mut FormatEtc) -> i32,
        _rest: [*const c_void; 3],
    }

    #[repr(C)]
    struct StreamVtable {
        query: *const c_void,
        add: *const c_void,
        release: *const c_void,
        read: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> i32,
        _rest: [*const c_void; 10],
    }

    /// Take over the window's drop target from winit's, so staging happens
    /// inside Drop rather than a frame later.
    /// Whether OLE still keeps our target on the window, and which window the
    /// cursor is over: the two facts that decide whether DragEnter can fire.
    pub fn trace_registration(hwnd: isize) {
        unsafe {
            let prop = GetPropW(hwnd, wide("OleDropTargetInterface").as_ptr());
            let mut pt = Point { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            let over = WindowFromPoint(pt);
            eprintln!(
                "[drop] check hwnd={hwnd} prop={prop:?} cursor=({},{}) over={over}",
                pt.x, pt.y
            );
        }
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetPropW(hwnd: isize, name: *const u16) -> *mut c_void;
        fn GetCursorPos(pt: *mut Point) -> i32;
        fn WindowFromPoint(pt: Point) -> isize;
    }

    pub fn register(hwnd: isize, tx: Sender<OsDragMsg>) {
        unsafe {
            // OLE drag and drop needs OleInitialize, not plain CoInitializeEx:
            // without it DoDragDrop returns CO_E_NOT_SUPPORTED and a registered
            // target is never consulted by the drag loop.
            let ole = OleInitialize(std::ptr::null_mut());
            if std::env::var_os("LYRE_TRACE").is_some() {
                eprintln!("[drop] OleInitialize={ole:#x}");
            }
            let target = Box::new(Target {
                vtable: &TARGET_VTABLE,
                refs: AtomicUsize::new(1),
                tx,
                descriptor: format_name("FileDescriptorW"),
                contents: format_name("FileContents"),
                hwnd,
                accepted: false,
            });
            let ptr = Box::into_raw(target);
            let revoked = RevokeDragDrop(hwnd);
            let registered = RegisterDragDrop(hwnd, ptr.cast());
            if std::env::var_os("LYRE_TRACE").is_some() {
                eprintln!("[drop] RevokeDragDrop={revoked:#x} RegisterDragDrop={registered:#x}");
            }
            if registered != S_OK {
                drop(Box::from_raw(ptr));
            }
        }
    }

    // -- drag source ---------------------------------------------------------
    //
    // The data object is the shell's own, built from the dragged paths: the
    // same one Explorer hands OLE, carrying CF_HDROP and every other format a
    // target understands, an optimised move included. A hand-written vtable
    // cannot serve here: OLE queries interfaces beyond its slots (INoMarshal,
    // the drag helpers), and answers that were not real made it refuse the
    // operation with CO_E_NOT_SUPPORTED, then fault once the queries were
    // honest about what the object implements.

    static PENDING: std::sync::Mutex<Option<(Vec<PathBuf>, Option<DragPreview>)>> =
        std::sync::Mutex::new(None);
    static DRAG_MSG: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

    /// The registered message the frame subclass runs the handoff on.
    pub fn drag_message() -> u32 {
        *DRAG_MSG.get_or_init(|| unsafe { RegisterWindowMessageW(wide("LyreDragOut").as_ptr()) })
    }

    /// Queue the drag for the window's message pump instead of starting it
    /// inside the frame callback: winit buffers redraws raised while its event
    /// handler runs, so a DoDragDrop started there freezes the window for the
    /// whole session. Between callbacks its modal loop pumps frames again.
    pub fn request_start(hwnd: isize, paths: Vec<PathBuf>, preview: Option<DragPreview>) {
        if let Ok(mut pending) = PENDING.lock() {
            *pending = Some((paths, preview));
        }
        unsafe { PostMessageW(hwnd, drag_message(), 0, 0) };
    }

    /// Run the queued handoff on the window thread, from the subclass. A
    /// release that beat the handoff cancels instead of dropping the files on
    /// whatever window is under the pointer.
    pub fn run_pending() {
        let payload = PENDING.lock().ok().and_then(|mut p| p.take());
        if let Some((paths, preview)) = payload
            && button_held()
        {
            // The UI thread owns the Win32 capture from the press; OLE cannot
            // take it from another thread, so let go first.
            release_capture();
            start_drag_out(&paths, preview);
        }
    }

    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Com::IDataObject;
    use windows::Win32::System::Ole::{DROPEFFECT, IDropSource, IDropSource_Impl};
    use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
    use windows::Win32::Foundation::{COLORREF, POINT, SIZE};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
    };
    use windows::Win32::System::Com::CLSCTX_ALL;
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        BHID_DataObject, CLSID_DragDropHelper, IDragSourceHelper, ILCreateFromPathW, ILFree,
        IShellItemArray, SHDRAGIMAGE, SHCreateShellItemArrayFromIDLists,
    };
    use windows::core::{BOOL, HRESULT, PCWSTR};

    /// The button state decides the session: release drops, Escape cancels,
    /// anything else carries on. Default cursors show what the target allows.
    #[windows::core::implement(IDropSource)]
    struct DropSource;

    #[allow(non_snake_case)]
    impl IDropSource_Impl for DropSource_Impl {
        fn QueryContinueDrag(
            &self,
            escape: BOOL,
            keys: MODIFIERKEYS_FLAGS,
        ) -> windows::core::HRESULT {
            if escape.as_bool() {
                HRESULT(DRAGDROP_S_CANCEL)
            } else if keys & MODIFIERKEYS_FLAGS(MK_LBUTTON) == MODIFIERKEYS_FLAGS(0) {
                HRESULT(DRAGDROP_S_DROP)
            } else {
                HRESULT(S_OK)
            }
        }

        fn GiveFeedback(&self, _effect: DROPEFFECT) -> windows::core::HRESULT {
            HRESULT(DRAGDROP_S_USEDEFAULTCURSORS)
        }
    }

    /// The shell's `IDataObject` for these paths. The item ids are ours and are
    /// freed once the array holds them.
    unsafe fn file_data_object(paths: &[PathBuf]) -> Option<IDataObject> {
        unsafe {
            let wide: Vec<Vec<u16>> = paths
                .iter()
                .map(|p| p.as_os_str().encode_wide().chain(std::iter::once(0)).collect())
                .collect();
            let ids: Vec<*const ITEMIDLIST> = wide
                .iter()
                .map(|w| ILCreateFromPathW(PCWSTR::from_raw(w.as_ptr())).cast_const())
                .collect();
            let array: Option<IShellItemArray> = SHCreateShellItemArrayFromIDLists(&ids).ok();
            for id in &ids {
                if !id.is_null() {
                    ILFree(Some(*id));
                }
            }
            array?.BindToHandler(None, &BHID_DataObject).ok()
        }
    }

    /// The same data object for one path, for code that hands a file to a shell
    /// extension rather than to a drag session.
    pub(crate) fn data_object_for(path: &std::path::Path) -> Option<IDataObject> {
        unsafe { file_data_object(&[path.to_path_buf()]) }
    }

    /// A 32-bit top-down DIB of the preview, with premultiplied alpha: the
    /// shell's drag image is drawn with `AlphaBlend`, which expects it.
    unsafe fn hbitmap(preview: &DragPreview) -> Option<HBITMAP> {
        unsafe {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: preview.width as i32,
                    biHeight: -(preview.height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            if bits.is_null() {
                let _ = DeleteObject(bitmap.into());
                return None;
            }
            let pixels = (preview.width as usize * preview.height as usize * 4).min(preview.rgba.len());
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, pixels);
            for (px, out) in preview.rgba[..pixels]
                .as_chunks::<4>()
                .0
                .iter()
                .zip(dst.as_chunks_mut::<4>().0.iter_mut())
            {
                let alpha = px[3] as u32;
                out[0] = (px[2] as u32 * alpha / 255) as u8;
                out[1] = (px[1] as u32 * alpha / 255) as u8;
                out[2] = (px[0] as u32 * alpha / 255) as u8;
                out[3] = alpha as u8;
            }
            Some(bitmap)
        }
    }

    /// Hand the preview to the shell's drag helper, which owns the copy it puts
    /// under the pointer; without one, a drag shows only the OLE cursor.
    unsafe fn set_drag_image(data: &IDataObject, preview: &DragPreview) {
        unsafe {
            let Some(bitmap) = hbitmap(preview) else { return };
            if let Ok(helper) = windows::Win32::System::Com::CoCreateInstance::<_, IDragSourceHelper>(
                &CLSID_DragDropHelper,
                None,
                CLSCTX_ALL,
            ) {
                let image = SHDRAGIMAGE {
                    sizeDragImage: SIZE {
                        cx: preview.width as i32,
                        cy: preview.height as i32,
                    },
                    ptOffset: POINT {
                        x: preview.width as i32 / 2,
                        y: preview.height as i32 / 2,
                    },
                    hbmpDragImage: bitmap,
                    // CLR_NONE: blend with the image's own alpha channel.
                    crColorKey: COLORREF(0xFFFF_FFFF),
                };
                let _ = helper.InitializeFromBitmap(&image, data);
            }
            let _ = DeleteObject(bitmap.into());
        }
    }

    /// The gesture left the window: carry the same files into an OS drag so it
    /// can land in a browser, Explorer or anything else that takes files.
    pub fn start_drag_out(paths: &[PathBuf], preview: Option<DragPreview>) {
        let trace = std::env::var_os("LYRE_TRACE").is_some();
        // OLE's modal drag loop must run on the thread that owns the press and
        // the foreground window: it captures the mouse there and pumps that
        // thread's messages, so the window keeps painting.
        unsafe {
            let Some(data) = file_data_object(paths) else {
                if trace {
                    eprintln!("[drag] the shell gave no data object for {} path(s)", paths.len());
                }
                return;
            };
            if let Some(preview) = &preview {
                set_drag_image(&data, preview);
            }
            let source: IDropSource = DropSource.into();
            let mut effect = DROPEFFECT(0);
            let started = std::time::Instant::now();
            if trace {
                let fg = GetForegroundWindow();
                let mut fg_pid = 0u32;
                let fg_thread = GetWindowThreadProcessId(fg, &mut fg_pid);
                eprintln!(
                    "[drag] pre-DoDragDrop fg={fg} fg_thread={fg_thread} self_thread={} lbutton={} items={}",
                    GetCurrentThreadId(),
                    GetAsyncKeyState(0x01),
                    paths.len()
                );
            }
            let hr = windows::Win32::System::Ole::DoDragDrop(
                &data,
                &source,
                DROPEFFECT(DROPEFFECT_COPY | DROPEFFECT_MOVE),
                &mut effect,
            );
            if trace {
                eprintln!(
                    "[drag] DoDragDrop hr={hr:?} effect={} after {} ms",
                    effect.0,
                    started.elapsed().as_millis()
                );
            }
            // The session is over wherever it ended: a cancel over this window
            // need not have called DragLeave, so clear the hover explicitly.
            if let Some(tx) = super::sender() {
                let _ = tx.send(OsDragMsg::Hover(None));
            }
        }
    }

    #[cfg(test)]
    mod com_tests {
        use super::*;

        /// OLE calls through the pointer at offset zero; a refcount there is an
        /// access violation the moment RegisterDragDrop addrefs the target.
        #[test]
        fn the_target_starts_with_its_vtable() {
            assert_eq!(std::mem::offset_of!(Target, vtable), 0);
        }

        /// COM reads `FORMATETC` by its own layout: the target device pointer
        /// follows the clip format at pointer alignment. Getting this wrong
        /// makes every `GetData` land on another field and collect nothing.
        #[cfg(target_pointer_width = "64")]
        #[test]
        fn format_etc_matches_the_com_layout() {
            assert_eq!(std::mem::offset_of!(FormatEtc, _ptd), 8);
            assert_eq!(std::mem::offset_of!(FormatEtc, _aspect), 16);
            assert_eq!(std::mem::offset_of!(FormatEtc, _lindex), 20);
            assert_eq!(std::mem::offset_of!(FormatEtc, tymed), 24);
            assert_eq!(std::mem::size_of::<FormatEtc>(), 32);
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: an XDND source over X11, and a wl_data_source on Wayland. The Wayland
// tracker thread binds its own seat and pointer (every pointer object of a
// seat hears its events) so the press serial start_drag demands is known; all
// protocol calls stay on that one thread and queue.
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
pub mod linux {
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::os::raw::{c_ulong, c_uint};
    use std::path::PathBuf;
    use std::sync::mpsc::{channel, Sender};
    use std::sync::OnceLock;

    static REQUEST: OnceLock<Sender<Vec<PathBuf>>> = OnceLock::new();

    /// Start the Wayland tracker once the window proves it is on Wayland.
    pub fn set_wayland() {
        if REQUEST.get().is_none() {
            let (tx, rx) = channel::<Vec<PathBuf>>();
            if REQUEST.set(tx).is_ok() {
                std::thread::spawn(move || wayland_tracker(rx));
            }
        }
    }

    /// The gesture left the window: hand it to whichever session the window
    /// belongs to.
    pub fn start_drag_out(paths: &[PathBuf]) {
        if REQUEST.get().is_some() {
            if let Some(tx) = REQUEST.get() {
                let _ = tx.send(paths.to_vec());
            }
            return;
        }
        let paths = paths.to_vec();
        std::thread::spawn(move || xdnd_drag(&paths));
    }

    fn uri_list(paths: &[PathBuf]) -> String {
        let mut out = String::new();
        for path in paths {
            out.push_str("file://");
            for byte in path.to_string_lossy().bytes() {
                match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                        out.push(byte as char)
                    }
                    _ => out.push_str(&format!("%{byte:02X}")),
                }
            }
            out.push_str("\r\n");
        }
        out
    }

    // -- X11 / XDND ----------------------------------------------------------

    type Display = *mut c_void;
    type Window = c_ulong;
    type Atom = c_ulong;
    type Bool = c_int;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XClientMessage {
        type_: c_int,
        serial: c_ulong,
        send_event: Bool,
        display: Display,
        window: Window,
        message_type: Atom,
        format: c_int,
        data: [c_ulong; 5],
    }

    #[repr(C)]
    union XEvent {
        type_: c_int,
        client: XClientMessage,
        selection: XSelectionEvent,
        request: XSelectionRequest,
        pad: [c_ulong; 24],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XSelectionEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: Bool,
        display: Display,
        requestor: Window,
        selection: Atom,
        target: Atom,
        property: Atom,
        time: c_ulong,
    }

    #[link(name = "X11")]
    unsafe extern "C" {
        fn XOpenDisplay(name: *const c_char) -> Display;
        fn XCloseDisplay(d: Display) -> c_int;
        fn XDefaultRootWindow(d: Display) -> Window;
        fn XInternAtom(d: Display, name: *const c_char, only_if_exists: Bool) -> Atom;
        fn XSetSelectionOwner(d: Display, selection: Atom, owner: Window, time: c_ulong);
        fn XCreateSimpleWindow(
            d: Display,
            parent: Window,
            x: c_int,
            y: c_int,
            w: c_uint,
            h: c_uint,
            border: c_uint,
            border_pixel: c_ulong,
            background: c_ulong,
        ) -> Window;
        fn XDestroyWindow(d: Display, w: Window);
        fn XQueryPointer(
            d: Display,
            w: Window,
            root: *mut Window,
            child: *mut Window,
            rx: *mut c_int,
            ry: *mut c_int,
            wx: *mut c_int,
            wy: *mut c_int,
            mask: *mut c_uint,
        ) -> Bool;
        fn XGetWindowProperty(
            d: Display,
            w: Window,
            property: Atom,
            offset: c_long,
            length: c_long,
            delete: Bool,
            req_type: Atom,
            actual_type: *mut Atom,
            actual_format: *mut c_int,
            nitems: *mut c_ulong,
            bytes_after: *mut c_ulong,
            prop: *mut *mut c_void,
        ) -> c_int;
        fn XFree(data: *mut c_void) -> c_int;
        fn XSendEvent(d: Display, w: Window, propagate: Bool, mask: c_long, event: *mut XEvent) -> c_int;
        fn XNextEvent(d: Display, event: *mut XEvent) -> c_int;
        fn XPending(d: Display) -> c_int;
        fn XFlush(d: Display) -> c_int;
        fn XChangeProperty(
            d: Display,
            w: Window,
            property: Atom,
            type_: Atom,
            format: c_int,
            mode: c_int,
            data: *const c_void,
            nelements: c_int,
        ) -> c_int;
    }
    use std::os::raw::c_long;

    const CURRENT_TIME: c_ulong = 0;
    const SELECTION_REQUEST: c_int = 22;
    const CLIENT_MESSAGE: c_int = 33;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XSelectionRequest {
        type_: c_int,
        serial: c_ulong,
        send_event: Bool,
        display: Display,
        owner: Window,
        requestor: Window,
        selection: Atom,
        target: Atom,
        property: Atom,
        time: c_ulong,
    }

    fn atom(d: Display, name: &str) -> Atom {
        let c = CString::new(name).unwrap();
        unsafe { XInternAtom(d, c.as_ptr(), 0) }
    }

    /// XDND version 5 source: own the selection, announce to the window under
    /// the pointer, answer its data requests, finish on release.
    fn xdnd_drag(paths: &[PathBuf]) {
        unsafe {
            let d = XOpenDisplay(std::ptr::null());
            if d.is_null() {
                return;
            }
            let root = XDefaultRootWindow(d);
            let win = XCreateSimpleWindow(d, root, -100, -100, 1, 1, 0, 0, 0);
            let aware = atom(d, "XdndAware");
            let selection = atom(d, "XdndSelection");
            let enter = atom(d, "XdndEnter");
            let position = atom(d, "XdndPosition");
            let leave = atom(d, "XdndLeave");
            let drop = atom(d, "XdndDrop");
            let finished = atom(d, "XdndFinished");
            let status = atom(d, "XdndStatus");
            let action_copy = atom(d, "XdndActionCopy");
            let action_move = atom(d, "XdndActionMove");
            let action_ask = atom(d, "XdndActionAsk");
            let uri = atom(d, "text/uri-list");
            let targets = atom(d, "TARGETS");
            XSetSelectionOwner(d, selection, win, CURRENT_TIME);
            let list = uri_list(paths);
            let mut target_win: Window = 0;
            let mut accepted = false;
            let mut entered = false;
            let mut last_pos = std::time::Instant::now();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            loop {
                if std::time::Instant::now() > deadline {
                    break;
                }
                while XPending(d) > 0 {
                    let mut ev: XEvent = std::mem::zeroed();
                    XNextEvent(d, &mut ev);
                    match ev.type_ {
                        SELECTION_REQUEST => {
                            let req = &ev.request;
                            let prop = if req.property != 0 { req.property } else { req.target };
                            if req.target == targets {
                                let list_atoms = [targets, uri];
                                XChangeProperty(
                                    d,
                                    req.requestor,
                                    prop,
                                    4, // XA_ATOM
                                    32,
                                    0,
                                    list_atoms.as_ptr().cast(),
                                    2,
                                );
                            } else if req.target == uri {
                                XChangeProperty(
                                    d,
                                    req.requestor,
                                    prop,
                                    uri,
                                    8,
                                    0,
                                    list.as_ptr().cast(),
                                    list.len() as c_int,
                                );
                            }
                            let mut reply: XEvent = std::mem::zeroed();
                            reply.selection.type_ = 31; // SelectionNotify
                            reply.selection.display = d;
                            reply.selection.requestor = req.requestor;
                            reply.selection.selection = req.selection;
                            reply.selection.target = req.target;
                            reply.selection.property = prop;
                            reply.selection.time = req.time;
                            XSendEvent(d, req.requestor, 0, 0, &mut reply);
                        }
                        CLIENT_MESSAGE => {
                            let msg = &ev.client;
                            if msg.message_type == status {
                                accepted = msg.data[1] & 1 != 0;
                            } else if msg.message_type == finished {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let mut root_ret = 0;
                let mut child = 0;
                let mut rx = 0;
                let mut ry = 0;
                let mut wx = 0;
                let mut wy = 0;
                let mut mask = 0u32;
                XQueryPointer(d, root, &mut root_ret, &mut child, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask);
                if mask & 0x100 == 0 {
                    // Button released: drop on the accepted target, if any.
                    if entered && target_win != 0 {
                        let mut ev: XEvent = std::mem::zeroed();
                        ev.client.type_ = CLIENT_MESSAGE;
                        ev.client.display = d;
                        ev.client.window = target_win;
                        ev.client.message_type = drop;
                        ev.client.format = 32;
                        ev.client.data[0] = win as c_ulong;
                        ev.client.data[2] = CURRENT_TIME;
                        XSendEvent(d, target_win, 0, 0, &mut ev);
                        XFlush(d);
                        // Give the target a moment to finish; its Finished
                        // message ends the loop above.
                        let until = std::time::Instant::now() + std::time::Duration::from_millis(500);
                        while std::time::Instant::now() < until && XPending(d) == 0 {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                    break;
                }
                let over = if child != 0 { child } else { root };
                if over != target_win {
                    if entered && target_win != 0 {
                        let mut ev: XEvent = std::mem::zeroed();
                        ev.client.type_ = CLIENT_MESSAGE;
                        ev.client.display = d;
                        ev.client.window = target_win;
                        ev.client.message_type = leave;
                        ev.client.format = 32;
                        ev.client.data[0] = win as c_ulong;
                        XSendEvent(d, target_win, 0, 0, &mut ev);
                    }
                    target_win = over;
                    accepted = false;
                    entered = false;
                    let mut ty = 0;
                    let mut fmt = 0;
                    let mut n = 0;
                    let mut after = 0;
                    let mut prop: *mut c_void = std::ptr::null_mut();
                    if XGetWindowProperty(d, over, aware, 0, 1, 0, 0, &mut ty, &mut fmt, &mut n, &mut after, &mut prop) == 0
                        && !prop.is_null()
                    {
                        let version = *(prop as *const c_ulong);
                        XFree(prop);
                        if version >= 3 {
                            entered = true;
                            let mut ev: XEvent = std::mem::zeroed();
                            ev.client.type_ = CLIENT_MESSAGE;
                            ev.client.display = d;
                            ev.client.window = over;
                            ev.client.message_type = enter;
                            ev.client.format = 32;
                            ev.client.data[0] = win as c_ulong;
                            ev.client.data[1] = (5u64 << 24) as c_ulong | 1; // version 5, more types flag
                            ev.client.data[2] = uri;
                            XSendEvent(d, over, 0, 0, &mut ev);
                        }
                    }
                }
                if entered && last_pos.elapsed() >= std::time::Duration::from_millis(50) {
                    last_pos = std::time::Instant::now();
                    let mut ev: XEvent = std::mem::zeroed();
                    ev.client.type_ = CLIENT_MESSAGE;
                    ev.client.display = d;
                    ev.client.window = target_win;
                    ev.client.message_type = position;
                    ev.client.format = 32;
                    ev.client.data[0] = win as c_ulong;
                    ev.client.data[2] = ((rx as c_ulong) << 16) | (ry as c_ulong & 0xffff);
                    ev.client.data[3] = CURRENT_TIME;
                    ev.client.data[4] = if accepted { action_copy } else { action_ask };
                    let _ = action_move;
                    XSendEvent(d, target_win, 0, 0, &mut ev);
                }
                XFlush(d);
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            if entered && target_win != 0 {
                let mut ev: XEvent = std::mem::zeroed();
                ev.client.type_ = CLIENT_MESSAGE;
                ev.client.display = d;
                ev.client.window = target_win;
                ev.client.message_type = leave;
                ev.client.format = 32;
                ev.client.data[0] = win as c_ulong;
                XSendEvent(d, target_win, 0, 0, &mut ev);
            }
            XSetSelectionOwner(d, selection, 0, CURRENT_TIME);
            XDestroyWindow(d, win);
            XCloseDisplay(d);
        }
    }

    // -- Wayland -------------------------------------------------------------
    //
    // One tracker thread owns one event queue: it binds its own seat and
    // pointer (every pointer object of a seat hears its events) so the press
    // serial start_drag demands is known, and runs any drag the window hands
    // over as a wl_data_source offering text/uri-list.

    use wayland_client::protocol::{
        wl_data_device::WlDataDevice,
        wl_data_device_manager::{DndAction, WlDataDeviceManager},
        wl_data_source::WlDataSource,
        wl_pointer::{self, WlPointer},
        wl_registry::{self, WlRegistry},
        wl_seat::{self, WlSeat},
    };
    use wayland_client::{Connection, Dispatch, QueueHandle};

    struct Tracker {
        serial: u32,
        seat: Option<WlSeat>,
        manager: Option<WlDataDeviceManager>,
        device: Option<WlDataDevice>,
        origin: Option<wayland_client::protocol::wl_surface::WlSurface>,
        done: bool,
        payload: String,
    }

    impl Dispatch<WlRegistry, ()> for Tracker {
        fn event(
            state: &mut Self,
            registry: &WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global { name, interface, version } = event {
                match interface.as_str() {
                    "wl_seat" if state.seat.is_none() => {
                        state.seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(5), qh, ()));
                    }
                    "wl_data_device_manager" if state.manager.is_none() => {
                        state.manager =
                            Some(registry.bind::<WlDataDeviceManager, _, _>(name, version.min(3), qh, ()));
                    }
                    "wl_compositor" if state.origin.is_none() => {
                        let comp = registry.bind::<wayland_client::protocol::wl_compositor::WlCompositor, _, _>(
                            name,
                            version.min(4),
                            qh,
                            (),
                        );
                        state.origin = Some(comp.create_surface(qh, ()));
                    }
                    _ => {}
                }
            }
        }
    }

    impl Dispatch<WlSeat, ()> for Tracker {
        fn event(_: &mut Self, _: &WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<WlDataDeviceManager, ()> for Tracker {
        fn event(_: &mut Self, _: &WlDataDeviceManager, _: wayland_client::protocol::wl_data_device_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<wayland_client::protocol::wl_compositor::WlCompositor, ()> for Tracker {
        fn event(_: &mut Self, _: &wayland_client::protocol::wl_compositor::WlCompositor, _: wayland_client::protocol::wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<wayland_client::protocol::wl_surface::WlSurface, ()> for Tracker {
        fn event(_: &mut Self, _: &wayland_client::protocol::wl_surface::WlSurface, _: wayland_client::protocol::wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<WlPointer, ()> for Tracker {
        fn event(
            state: &mut Self,
            _: &WlPointer,
            event: wl_pointer::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_pointer::Event::Button { serial, state: btn, .. } = event {
                if btn == wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed) {
                    state.serial = serial;
                }
            }
        }
    }

    impl Dispatch<WlDataDevice, ()> for Tracker {
        fn event(
            _: &mut Self,
            _: &WlDataDevice,
            _: wayland_client::protocol::wl_data_device::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<WlDataSource, ()> for Tracker {
        fn event(
            state: &mut Self,
            _: &WlDataSource,
            event: wayland_client::protocol::wl_data_source::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use wayland_client::protocol::wl_data_source::Event;
            match event {
                Event::Send { mime_type, fd } => {
                    // We own the write end; taking it as a File closes it on
                    // drop, which is what the compositor expects.
                    let mut file = std::fs::File::from(fd);
                    if mime_type == "text/uri-list" {
                        use std::io::Write;
                        let _ = file.write_all(state.payload.as_bytes());
                    }
                }
                Event::Cancelled | Event::DndFinished => state.done = true,
                _ => {}
            }
        }
    }

    /// Dispatch what arrived, then sleep: an idle tracker costs a sleeping
    /// thread, and a handoff is picked up within the timeout.
    fn pump(
        queue: &mut wayland_client::EventQueue<Tracker>,
        conn: &Connection,
        state: &mut Tracker,
        timeout_ms: u64,
    ) {
        let _ = queue.dispatch_pending(state);
        let _ = conn.flush();
        std::thread::sleep(std::time::Duration::from_millis(timeout_ms));
        let _ = queue.dispatch_pending(state);
    }

    fn wayland_tracker(rx: std::sync::mpsc::Receiver<Vec<PathBuf>>) {
        let Ok(conn) = Connection::connect_to_env() else { return };
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut state = Tracker {
            serial: 0,
            seat: None,
            manager: None,
            device: None,
            origin: None,
            done: false,
            payload: String::new(),
        };
        let _registry = conn.display().get_registry(&qh, ());
        if queue.roundtrip(&mut state).is_err() {
            return;
        }
        let (Some(seat), Some(manager)) = (state.seat.clone(), state.manager.clone()) else {
            return;
        };
        let _pointer = seat.get_pointer(&qh, ());
        state.device = Some(manager.get_data_device(&seat, &qh, ()));
        if queue.roundtrip(&mut state).is_err() {
            return;
        }
        loop {
            pump(&mut queue, &conn, &mut state, 100);
            let Ok(paths) = rx.try_recv() else { continue };
            if state.serial == 0 {
                continue;
            }
            state.payload = uri_list(&paths);
            state.done = false;
            let source = manager.create_data_source(&qh, ());
            source.offer("text/uri-list".to_string());
            source.set_actions(DndAction::Copy | DndAction::Move);
            if let (Some(device), Some(origin)) = (&state.device, &state.origin) {
                device.start_drag(Some(&source), origin, None, state.serial);
            }
            let _ = conn.flush();
            let until = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while !state.done && std::time::Instant::now() < until {
                pump(&mut queue, &conn, &mut state, 20);
            }
            source.destroy();
            let _ = conn.flush();
            state.payload.clear();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_survives_the_source_deleting_its_temp_tree() {
        let src = std::env::temp_dir().join(format!("lyre-stage-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/a.txt"), b"payload").unwrap();
        let staged = stage(&[src.clone()]);
        assert_eq!(staged.len(), 1);
        let _ = std::fs::remove_dir_all(&src);
        assert_eq!(std::fs::read(staged[0].join("sub/a.txt")).unwrap(), b"payload");
        release(&staged);
        assert!(!staged[0].exists());
    }

    #[test]
    fn paths_outside_temp_pass_through_unstaged() {
        let dir = std::env::current_dir().unwrap();
        let staged = stage(&[dir.clone()]);
        assert_eq!(staged, vec![dir]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn filegroup_descriptor_names_start_after_the_alignment_padding() {
        let stride = std::mem::size_of::<win::FileDescriptor>();
        let mut bytes = vec![0u8; 8 + stride];
        bytes[0..4].copy_from_slice(&1u32.to_ne_bytes());
        let name_at = 8 + 72; // DWORD count, padding, then the fixed fields
        for (i, u) in "sub\\deep.txt".encode_utf16().enumerate() {
            bytes[name_at + i * 2..name_at + i * 2 + 2].copy_from_slice(&u.to_ne_bytes());
        }
        assert_eq!(win::descriptor_names(&bytes), vec!["sub\\deep.txt".to_string()]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn hdrop_header_and_double_null_names_parse() {
        let mut bytes = vec![0u8; std::mem::size_of::<win::DropFiles>()];
        let header: &mut win::DropFiles = unsafe { &mut *(bytes.as_mut_ptr() as *mut win::DropFiles) };
        header.p_files = bytes.len() as u32;
        header.f_wide = 1;
        for name in ["C:\\a.txt", "C:\\b.txt"] {
            for u in name.encode_utf16() {
                bytes.extend_from_slice(&u.to_ne_bytes());
            }
            bytes.extend_from_slice(&0u16.to_ne_bytes());
        }
        bytes.extend_from_slice(&0u16.to_ne_bytes());
        assert_eq!(
            win::parse_hdrop(&bytes),
            vec![PathBuf::from("C:\\a.txt"), PathBuf::from("C:\\b.txt")]
        );
    }
}
