//! Cross-platform window frame controls for the borderless shell.
//!
//! Rune's `Tx::minimize` refuses a borderless window on every platform and there
//! is no `Tx::maximize`, so the drawn buttons drive the native window directly:
//! - Windows: `ShowWindow` minimizes/maximizes, DWM rounds the frame, and a
//!   subclass gives draggable resize edges (`WM_NCHITTEST`) and runs the
//!   drag-out handoff between winit's event callbacks.
//! - macOS: `NSWindow` miniaturize/performZoom; rounded corners come from Rune's
//!   `MacWindowOptions` (set in `main`).
//! - Linux: X11 uses EWMH (`_NET_WM_STATE`) and `XIconifyWindow`; Hyprland uses
//!   `hyprctl` dispatch; other Wayland compositors fall back gracefully (resize
//!   still works through winit). X11 is loaded at runtime so the crate builds and
//!   runs on Wayland-only systems without libX11.
//!
//! Everything links to system libraries by name (or loads them at runtime), so no
//! extra crate is needed. Only the Windows path is runtime-tested on this machine.

use rune::WindowHandle;

pub fn install(handle: &WindowHandle) {
    imp::install(handle);
}

/// The native window handle, for converting OS drag coordinates.
#[cfg(target_os = "windows")]
pub fn hwnd() -> Option<isize> {
    imp::hwnd()
}

#[cfg(not(target_os = "windows"))]
pub fn hwnd() -> Option<isize> {
    None
}

/// The window's client size in physical pixels, when the platform can be asked.
///
/// Rune exposes no size accessor and no resize event, so the native window is
/// asked directly. Only Windows implements it; elsewhere this returns `None` and
/// the window simply opens at its default size.
#[cfg(target_os = "windows")]
pub fn content_size() -> Option<(u32, u32)> {
    imp::content_size()
}

#[cfg(not(target_os = "windows"))]
pub fn content_size() -> Option<(u32, u32)> {
    None
}

/// The mouse side button pressed since the last call: 1 is back, 2 forward.
#[cfg(target_os = "windows")]
pub fn take_side_button() -> Option<u8> {
    imp::take_side_button()
}

/// The middle button pressed since the last call, as a window point.
#[cfg(target_os = "windows")]
pub fn take_middle_press() -> Option<(f32, f32)> {
    imp::take_middle_press()
}

#[cfg(not(target_os = "windows"))]
pub fn take_middle_press() -> Option<(f32, f32)> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn take_side_button() -> Option<u8> {
    None
}

pub fn minimize() {
    imp::minimize();
}

/// Maximize when restored, restore when maximized; returns the new zoom state.
pub fn toggle_maximize() -> bool {
    imp::toggle_maximize()
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
mod imp {
    use rune::WindowHandle;
    use rune::raw_window_handle::RawWindowHandle;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

    static HWND: OnceLock<isize> = OnceLock::new();
    /// The last side button pressed, for `poll` to take: 1 is back, 2 forward.
    static SIDE_BUTTON: AtomicU8 = AtomicU8::new(0);
    const WM_XBUTTONUP: u32 = 0x020C;
    /// The middle button released since the last call, as a window point packed
    /// into one word: 0 is none, otherwise x in the low half and y in the high.
    static MIDDLE_PRESS: AtomicU64 = AtomicU64::new(0);
    const WM_MBUTTONUP: u32 = 0x0208;
    /// A browser Back or Forward key, or a mouse that reports its side buttons
    /// this way rather than as an X button.
    const WM_APPCOMMAND: u32 = 0x0319;
    /// `GET_APPCOMMAND_LPARAM` of the high word: the command, without its flags.
    const APPCOMMAND_BROWSER_BACKWARD: usize = 1;
    const APPCOMMAND_BROWSER_FORWARD: usize = 2;

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    type SubclassProc =
        unsafe extern "system" fn(isize, u32, usize, isize, usize, usize) -> isize;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attribute: u32,
            value: *const core::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        fn IsZoomed(hwnd: isize) -> i32;
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn GetClientRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn DefSubclassProc(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn RedrawWindow(hwnd: isize, rect: *const core::ffi::c_void, region: *mut core::ffi::c_void, flags: u32) -> i32;
        fn LoadImageW(
            instance: *mut core::ffi::c_void,
            name: *const u16,
            kind: u32,
            width: i32,
            height: i32,
            flags: u32,
        ) -> *mut core::ffi::c_void;
        fn SendMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut core::ffi::c_void;
    }
    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn SetWindowSubclass(hwnd: isize, proc: SubclassProc, id: usize, data: usize) -> i32;
    }

    const WM_NCHITTEST: u32 = 0x0084;
    const SW_MINIMIZE: i32 = 6;
    const SW_MAXIMIZE: i32 = 3;
    const SW_RESTORE: i32 = 9;
    const HTLEFT: isize = 10;
    const HTRIGHT: isize = 11;
    const HTTOP: isize = 12;
    const HTTOPLEFT: isize = 13;
    const HTTOPRIGHT: isize = 14;
    const HTBOTTOM: isize = 15;
    const HTBOTTOMLEFT: isize = 16;
    const HTBOTTOMRIGHT: isize = 17;
    const BORDER: i32 = 8;
    const RDW_INTERNALPAINT: u32 = 0x0002;

    unsafe extern "system" fn subclass(
        hwnd: isize,
        msg: u32,
        wparam: usize,
        lparam: isize,
        _id: usize,
        _data: usize,
    ) -> isize {
        unsafe {
            // The drag-out handoff runs here, between winit's event callbacks:
            // OLE's modal loop then pumps frames re-entrantly, so the window
            // keeps painting (and hover highlights) for the whole drag.
            if msg == crate::osdrag::win::drag_message() {
                crate::osdrag::win::run_pending();
                RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INTERNALPAINT);
                return 0;
            }
            if msg == WM_NCHITTEST && IsZoomed(hwnd) == 0 {
                let x = (lparam as u16) as i16 as i32;
                let y = ((lparam >> 16) as u16) as i16 as i32;
                let mut r = Rect { left: 0, top: 0, right: 0, bottom: 0 };
                if GetWindowRect(hwnd, &mut r) != 0 {
                    let left = x < r.left + BORDER;
                    let right = x >= r.right - BORDER;
                    let top = y < r.top + BORDER;
                    let bottom = y >= r.bottom - BORDER;
                    let code = match (top, bottom, left, right) {
                        (true, _, true, _) => HTTOPLEFT,
                        (true, _, _, true) => HTTOPRIGHT,
                        (_, true, true, _) => HTBOTTOMLEFT,
                        (_, true, _, true) => HTBOTTOMRIGHT,
                        (true, _, _, _) => HTTOP,
                        (_, true, _, _) => HTBOTTOM,
                        (_, _, true, _) => HTLEFT,
                        (_, _, _, true) => HTRIGHT,
                        _ => 0,
                    };
                    if code != 0 {
                        return code;
                    }
                }
            }
            // The side buttons on a mouse. Windows delivers them as
            // WM_XBUTTONUP with the button in the high word, and Rune has no
            // notion of them, so they are caught here and left for `poll`.
            if msg == WM_XBUTTONUP {
                let which = ((wparam >> 16) & 0xFFFF) as u8;
                if which == 1 || which == 2 {
                    SIDE_BUTTON.store(which, Ordering::Relaxed);
                    // A taken message does not wake the runtime, and an idle
                    // window then never runs `poll`: the press sat unread until
                    // the pointer happened to move over the window, which is why
                    // the side buttons felt like they did nothing. Asking for a
                    // paint is how this file already wakes the window from a
                    // message it takes (see the drag-out handoff above).
                    RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INTERNALPAINT);
                }
                // Handled: returning zero would let the default proc beep.
                return 1;
            }
            // Not every device reports the browser buttons as an X button:
            // A middle press is a browser's "close this tab", and Rune has no
            // handler for the middle button at all, so the window takes it and
            // `poll` closes whatever tab it landed on.
            if msg == WM_MBUTTONUP {
                let x = (lparam as u16) as i16 as i64;
                let y = ((lparam >> 16) as u16) as i16 as i64;
                let packed = ((x + 0x8000) | ((y + 0x8000) << 16)) as u64;
                MIDDLE_PRESS.store(packed, Ordering::Relaxed);
                RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INTERNALPAINT);
                return 1;
            }
            // Not every device reports the browser buttons as an X button:
            // keyboards with Back/Forward keys, and some mice, send them as an
            // app command instead. Both land on the same history step.
            if msg == WM_APPCOMMAND {
                let command = ((lparam >> 16) & 0x0FFF) as usize;
                let which = match command {
                    APPCOMMAND_BROWSER_BACKWARD => 1,
                    APPCOMMAND_BROWSER_FORWARD => 2,
                    _ => 0,
                };
                if which != 0 {
                    SIDE_BUTTON.store(which, Ordering::Relaxed);
                    RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INTERNALPAINT);
                    // Handled, so Windows does not pass it on to the shell.
                    return 1;
                }
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
    }

    pub fn hwnd() -> Option<isize> {
        HWND.get().copied()
    }

    /// The side button pressed since the last call, if any.
    pub fn take_side_button() -> Option<u8> {
        match SIDE_BUTTON.swap(0, std::sync::atomic::Ordering::Relaxed) {
            0 => None,
            which => Some(which),
        }
    }

    /// The middle press since the last call, in client pixels.
    pub fn take_middle_press() -> Option<(f32, f32)> {
        let packed = MIDDLE_PRESS.swap(0, Ordering::Relaxed);
        if packed == 0 {
            return None;
        }
        let x = ((packed & 0xFFFF) as i64 - 0x8000) as f32;
        let y = (((packed >> 16) & 0xFFFF) as i64 - 0x8000) as f32;
        Some((x, y))
    }

    /// The client area, which is what the layout sees and what a saved size has
    /// to describe.
    pub fn content_size() -> Option<(u32, u32)> {
        let &hwnd = HWND.get()?;
        let mut rect = Rect { left: 0, top: 0, right: 0, bottom: 0 };
        if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
            return None;
        }
        let width = (rect.right - rect.left).max(0) as u32;
        let height = (rect.bottom - rect.top).max(0) as u32;
        (width > 0 && height > 0).then_some((width, height))
    }

    pub fn install(handle: &WindowHandle) {
        let _ = handle.with_raw_handle(|raw| {
            if let RawWindowHandle::Win32(win) = raw.as_raw() {
                let hwnd = win.hwnd.get();
                let _ = HWND.set(hwnd);
                if let Some(tx) = crate::osdrag::sender() {
                    crate::osdrag::win::register(hwnd, tx);
                }
                const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
                const DWMWCP_ROUND: u32 = 2;
                let preference = DWMWCP_ROUND;
                unsafe {
                    DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_WINDOW_CORNER_PREFERENCE,
                        (&preference as *const u32).cast(),
                        core::mem::size_of::<u32>() as u32,
                    );
                    SetWindowSubclass(hwnd, subclass, 1, 0);
                    apply_icon(hwnd);
                }
            }
        });
    }

    /// Put the embedded icon in the titlebar, taskbar and alt-tab list.
    ///
    /// A window class with no icon of its own gets the generic one, so it is set
    /// explicitly afterwards. The handle comes from the resource the build script
    /// compiled in, which is the same icon Explorer draws for the file, so the
    /// two can never disagree.
    unsafe fn apply_icon(hwnd: isize) {
        const WM_SETICON: u32 = 0x0080;
        const ICON_SMALL: usize = 0;
        const ICON_BIG: usize = 1;
        const IMAGE_ICON: u32 = 1;
        /// The id `lyre.rc` gives the icon.
        const ICON_RESOURCE: usize = 1;

        let module = unsafe { GetModuleHandleW(core::ptr::null()) };
        if module.is_null() {
            return;
        }
        for (which, size) in [(ICON_SMALL, 16), (ICON_BIG, 32)] {
            // The resource id is passed where a name would go, as the API expects.
            let name = ICON_RESOURCE as *const u16;
            let icon = unsafe { LoadImageW(module, name, IMAGE_ICON, size, size, 0) };
            if icon.is_null() {
                continue;
            }
            unsafe { SendMessageW(hwnd, WM_SETICON, which, icon as isize) };
        }
    }

    pub fn minimize() {
        if let Some(&hwnd) = HWND.get() {
            unsafe {
                ShowWindow(hwnd, SW_MINIMIZE);
            }
        }
    }

    pub fn toggle_maximize() -> bool {
        let Some(&hwnd) = HWND.get() else {
            return false;
        };
        unsafe {
            if IsZoomed(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
                false
            } else {
                ShowWindow(hwnd, SW_MAXIMIZE);
                true
            }
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
mod imp {
    use rune::WindowHandle;
    use rune::raw_window_handle::RawWindowHandle;
    use std::ffi::{CString, c_void};
    use std::sync::OnceLock;

    static NSW: OnceLock<usize> = OnceLock::new();

    unsafe extern "C" {
        fn sel_registerName(name: *const i8) -> *mut c_void;
        fn objc_msgSend(obj: *mut c_void, sel: *mut c_void, ...) -> usize;
    }

    fn sel(name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        unsafe { sel_registerName(c.as_ptr()) }
    }

    pub fn install(handle: &WindowHandle) {
        let _ = handle.with_raw_handle(|raw| {
            if let RawWindowHandle::AppKit(w) = raw.as_raw() {
                // The raw handle gives the view; ask it for its window.
                let ns_window = unsafe { objc_msgSend(w.ns_view.as_ptr(), sel("window")) };
                let _ = NSW.set(ns_window);
            }
        });
    }

    fn window() -> Option<*mut c_void> {
        NSW.get().map(|&p| p as *mut c_void)
    }

    pub fn minimize() {
        if let Some(w) = window() {
            unsafe {
                objc_msgSend(w, sel("miniaturize:"), std::ptr::null_mut::<c_void>());
            }
        }
    }

    pub fn toggle_maximize() -> bool {
        let Some(w) = window() else {
            return false;
        };
        unsafe {
            let zoomed = objc_msgSend(w, sel("isZoomed")) != 0;
            objc_msgSend(w, sel("performZoom:"), std::ptr::null_mut::<c_void>());
            !zoomed
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: X11 (EWMH), Hyprland (hyprctl), other Wayland (graceful fallback)
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
mod imp {
    use rune::WindowHandle;
    use rune::raw_window_handle::RawWindowHandle;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    static XWINDOW: OnceLock<u64> = OnceLock::new();
    static ZOOM: AtomicBool = AtomicBool::new(false);

    pub fn install(handle: &WindowHandle) {
        let _ = handle.with_raw_handle(|raw| {
            if let RawWindowHandle::Xlib(x) = raw.as_raw() {
                let _ = XWINDOW.set(x.window as u64);
            }
            if let RawWindowHandle::Wayland(_) = raw.as_raw() {
                crate::osdrag::linux::set_wayland();
            }
        });
    }

    fn hyprland() -> bool {
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
    }

    fn hyprctl(dispatch: &str) {
        let _ = std::process::Command::new("hyprctl")
            .arg("dispatch")
            .arg(dispatch)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    pub fn minimize() {
        if hyprland() {
            hyprctl("movetoworkspacesilent special:minimized");
            return;
        }
        if let Some(&win) = XWINDOW.get() {
            x11::minimize(win);
        }
    }

    pub fn toggle_maximize() -> bool {
        if hyprland() {
            // Hyprland fullscreen mode 1 is maximize; the dispatch toggles it.
            hyprctl("fullscreen 1");
            let now = !ZOOM.load(Ordering::Relaxed);
            ZOOM.store(now, Ordering::Relaxed);
            return now;
        }
        if let Some(&win) = XWINDOW.get() {
            x11::toggle_maximize(win);
            let now = !ZOOM.load(Ordering::Relaxed);
            ZOOM.store(now, Ordering::Relaxed);
            return now;
        }
        false
    }

    /// X11 loaded at runtime so Wayland-only systems need no libX11 at build time.
    mod x11 {
        use std::ffi::{CString, c_void};
        use std::os::raw::{c_int, c_ulong, c_long};
        use std::sync::OnceLock;

        type Display = c_void;
        type Atom = c_ulong;
        type Window = c_ulong;

        #[link(name = "dl")]
        unsafe extern "C" {
            fn dlopen(filename: *const i8, flag: c_int) -> *mut c_void;
            fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
        }

        static LIB: OnceLock<usize> = OnceLock::new();

        fn lib() -> Option<*mut c_void> {
            let handle = *LIB.get_or_init(|| {
                let name = CString::new("libX11.so.6").unwrap();
                unsafe { dlopen(name.as_ptr(), 2 /* RTLD_NOW */) as usize }
            });
            if handle == 0 {
                None
            } else {
                Some(handle as *mut c_void)
            }
        }

        unsafe fn sym<F>(lib: *mut c_void, name: &str) -> Option<F> {
            let c = CString::new(name).unwrap();
            let p = unsafe { dlsym(lib, c.as_ptr()) };
            if p.is_null() {
                None
            } else {
                Some(unsafe { std::mem::transmute_copy(&p) })
            }
        }

        type OpenDisplay = unsafe extern "C" fn(*const i8) -> *mut Display;
        type CloseDisplay = unsafe extern "C" fn(*mut Display) -> c_int;
        type InternAtom = unsafe extern "C" fn(*mut Display, *const i8, c_int) -> Atom;
        type DefaultScreen = unsafe extern "C" fn(*mut Display) -> c_int;
        type RootWindow = unsafe extern "C" fn(*mut Display, c_int) -> Window;
        type IconifyWindow = unsafe extern "C" fn(*mut Display, Window, c_int) -> c_int;
        type SendEvent = unsafe extern "C" fn(*mut Display, Window, c_int, c_long, *mut c_void) -> c_int;
        type Flush = unsafe extern "C" fn(*mut Display) -> c_int;

        #[repr(C)]
        struct ClientMessage {
            type_: c_int,
            serial: c_ulong,
            send_event: c_int,
            display: *mut Display,
            window: Window,
            message_type: Atom,
            format: c_int,
            data: [c_long; 5],
        }

        fn with_display<F: FnOnce(*mut Display)>(f: F) {
            let Some(lib) = lib() else { return };
            unsafe {
                let Some(open) = sym::<OpenDisplay>(lib, "XOpenDisplay") else { return };
                let Some(close) = sym::<CloseDisplay>(lib, "XCloseDisplay") else { return };
                let display = open(std::ptr::null());
                if display.is_null() {
                    return;
                }
                f(display);
                close(display);
            }
        }

        pub fn minimize(window: u64) {
            with_display(|display| unsafe {
                let Some(lib) = lib() else { return };
                let Some(default_screen) = sym::<DefaultScreen>(lib, "XDefaultScreen") else { return };
                let Some(iconify) = sym::<IconifyWindow>(lib, "XIconifyWindow") else { return };
                let screen = default_screen(display);
                iconify(display, window as Window, screen);
            });
        }

        pub fn toggle_maximize(window: u64) {
            with_display(|display| unsafe {
                let Some(lib) = lib() else { return };
                let Some(intern) = sym::<InternAtom>(lib, "XInternAtom") else { return };
                let Some(default_screen) = sym::<DefaultScreen>(lib, "XDefaultScreen") else { return };
                let Some(root) = sym::<RootWindow>(lib, "XRootWindow") else { return };
                let Some(send) = sym::<SendEvent>(lib, "XSendEvent") else { return };
                let Some(flush) = sym::<Flush>(lib, "XFlush") else { return };
                let atom = |name: &str| {
                    let c = CString::new(name).unwrap();
                    intern(display, c.as_ptr(), 0)
                };
                let wm_state = atom("_NET_WM_STATE");
                let vert = atom("_NET_WM_STATE_MAXIMIZED_VERT");
                let horiz = atom("_NET_WM_STATE_MAXIMIZED_HORZ");
                let screen = default_screen(display);
                let root_window = root(display, screen);
                // XEvent is a 24-long union; place the client message at its start.
                let mut event = [0 as c_long; 24];
                let msg = event.as_mut_ptr() as *mut ClientMessage;
                (*msg).type_ = 33; // ClientMessage
                (*msg).send_event = 1;
                (*msg).display = display;
                (*msg).window = window as Window;
                (*msg).message_type = wm_state;
                (*msg).format = 32;
                (*msg).data[0] = 2; // _NET_WM_STATE_TOGGLE
                (*msg).data[1] = vert as c_long;
                (*msg).data[2] = horiz as c_long;
                (*msg).data[3] = 1; // source: application
                let mask = ((1 as c_long) << 19) | ((1 as c_long) << 20); // SubstructureRedirect | SubstructureNotify
                send(display, root_window, 0, mask, event.as_mut_ptr() as *mut c_void);
                flush(display);
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Other platforms: graceful no-op (resize still works through winit).
// ---------------------------------------------------------------------------
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod imp {
    use rune::WindowHandle;

    pub fn install(_handle: &WindowHandle) {}
    pub fn minimize() {}
    pub fn toggle_maximize() -> bool {
        false
    }
}
