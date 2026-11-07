//! Cross-platform window frame controls for the borderless shell.
//!
//! Rune's `Tx::minimize` refuses a borderless window on every platform and there
//! is no `Tx::maximize`, so the drawn buttons drive the native window directly:
//! - Windows: `ShowWindow` minimizes/maximizes, DWM rounds the frame, and a
//!   `WM_NCHITTEST` subclass gives draggable resize edges.
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

    static HWND: OnceLock<isize> = OnceLock::new();

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
        fn DefSubclassProc(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
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

    unsafe extern "system" fn hit_test(
        hwnd: isize,
        msg: u32,
        wparam: usize,
        lparam: isize,
        _id: usize,
        _data: usize,
    ) -> isize {
        unsafe {
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
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
    }

    pub fn install(handle: &WindowHandle) {
        let _ = handle.with_raw_handle(|raw| {
            if let RawWindowHandle::Win32(win) = raw.as_raw() {
                let hwnd = win.hwnd.get();
                let _ = HWND.set(hwnd);
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
                    SetWindowSubclass(hwnd, hit_test, 1, 0);
                }
            }
        });
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
                let atom = |name: &str| unsafe {
                    let c = CString::new(name).unwrap();
                    intern(display, c.as_ptr(), 0)
                };
                let wm_state = atom("_NET_WM_STATE");
                let vert = atom("_NET_WM_STATE_MAXIMIZED_VERT");
                let horiz = atom("_NET_WM_STATE_MAXIMIZED_HORZ");
                let screen = default_screen(display);
                let root_window = root(display, screen);
                // XEvent is a 24-long union; place the client message at its start.
                let mut event = [0c_long; 24];
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
                let mask = (1c_long << 19) | (1c_long << 20); // SubstructureRedirect | SubstructureNotify
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