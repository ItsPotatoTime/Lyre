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

/// Apply the OS window backdrop material and dark/light frame appearance to
/// the installed window (called after [`install`]).
pub fn apply_backdrop(backdrop: crate::theme::BackdropStyle, dark: bool) {
    imp::apply_backdrop(backdrop, dark);
}

/// Apply the OS window backdrop material and dark/light frame appearance to
/// `handle` directly.
pub fn apply_backdrop_to(
    handle: &WindowHandle,
    backdrop: crate::theme::BackdropStyle,
    dark: bool,
) {
    imp::apply_backdrop_to(handle, backdrop, dark);
}

mod backdrop_spec {
    use crate::theme::BackdropStyle;

    pub const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    pub const DWMWA_USE_IMMERSIVE_DARK_MODE_PRE_20H1: u32 = 19;
    pub const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;

    pub const DWMSBT_NONE: u32 = 1;
    pub const DWMSBT_MAINWINDOW: u32 = 2;
    pub const DWMSBT_TRANSIENTWINDOW: u32 = 3;

    pub fn dwm_dark_mode(dark: bool) -> i32 {
        i32::from(dark)
    }

    pub fn dwm_backdrop_type(style: BackdropStyle) -> u32 {
        match style {
            BackdropStyle::Opaque => DWMSBT_NONE,
            BackdropStyle::Mica => DWMSBT_MAINWINDOW,
            BackdropStyle::Acrylic => DWMSBT_TRANSIENTWINDOW,
        }
    }

    /// Client-area DWM margins (`[left, right, top, bottom]`): `-1` on all
    /// edges extends DWM glass/backdrop behind the transparent client area.
    pub fn dwm_margins(style: BackdropStyle) -> [i32; 4] {
        if style == BackdropStyle::Opaque {
            [0, 0, 0, 0]
        } else {
            [-1, -1, -1, -1]
        }
    }

    pub fn x11_blur_behind(style: BackdropStyle) -> bool {
        style != BackdropStyle::Opaque
    }
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

    /// `WINDOWPLACEMENT`: where a window sits when it is not minimised or
    /// maximised, which is the size worth remembering.
    #[repr(C)]
    struct Placement {
        length: u32,
        flags: u32,
        show_cmd: u32,
        min_position: [i32; 2],
        max_position: [i32; 2],
        normal: Rect,
        device: Rect,
    }

    type SubclassProc =
        unsafe extern "system" fn(isize, u32, usize, isize, usize, usize) -> isize;

    #[repr(C)]
    struct Margins {
        left_width: i32,
        right_width: i32,
        top_height: i32,
        bottom_height: i32,
    }

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attribute: u32,
            value: *const core::ffi::c_void,
            size: u32,
        ) -> i32;
        fn DwmExtendFrameIntoClientArea(hwnd: isize, margins: *const Margins) -> i32;
    }
    #[repr(C)]
    struct MonitorInfo {
        cb_size: u32,
        rc_monitor: Rect,
        rc_work: Rect,
        flags: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        fn IsZoomed(hwnd: isize) -> i32;
        fn IsIconic(hwnd: isize) -> i32;
        fn GetWindowPlacement(hwnd: isize, placement: *mut Placement) -> i32;
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn GetClientRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
        fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
        fn SetWindowPos(hwnd: isize, insert_after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
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
    const WM_SIZE: u32 = 0x0005;
    const WM_ACTIVATE: u32 = 0x0006;
    const WM_SYSCOMMAND: u32 = 0x0112;
    const WM_POWERBROADCAST: u32 = 0x0218;
    const PBT_APMRESUMESUSPEND: usize = 0x0007;
    const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
    const SC_RESTORE: usize = 0xF120;
    /// `WM_SIZE`'s wParam when the window is being minimised.
    const SIZE_MINIMIZED: usize = 1;
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
            // Coming back from minimised, the window is not redrawn until
            // something else happens: it came back laid out for the sliver a
            // minimised window reports, and stayed that way until a click. Ask
            // for the paint that draws it at its real size instead.
            if (msg == WM_SIZE && wparam != SIZE_MINIMIZED)
                || (msg == WM_SYSCOMMAND && wparam & 0xFFF0 == SC_RESTORE)
                || (msg == WM_ACTIVATE && (wparam & 0xFFFF) != 0)
                || (msg == WM_POWERBROADCAST
                    && (wparam == PBT_APMRESUMESUSPEND || wparam == PBT_APMRESUMEAUTOMATIC))
            {
                RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INTERNALPAINT);
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
        // A minimised window reports the sliver it collapsed to, and saving that
        // is how a window came back at the smallest size allowed every time it
        // was minimised before closing. Its normal placement is the size worth
        // remembering, and this window is borderless, so that rect is the client.
        if unsafe { IsIconic(hwnd) } != 0 {
            let mut placement = Placement {
                length: core::mem::size_of::<Placement>() as u32,
                flags: 0,
                show_cmd: 0,
                min_position: [0, 0],
                max_position: [0, 0],
                normal: Rect { left: 0, top: 0, right: 0, bottom: 0 },
                device: Rect { left: 0, top: 0, right: 0, bottom: 0 },
            };
            if unsafe { GetWindowPlacement(hwnd, &mut placement) } != 0 {
                let width = (placement.normal.right - placement.normal.left).max(0) as u32;
                let height = (placement.normal.bottom - placement.normal.top).max(0) as u32;
                if width > 0 && height > 0 {
                    return Some((width, height));
                }
            }
            return None;
        }
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
                    clamp_to_work_area(hwnd);
                }
            }
        });
    }

    /// Keep a newly created window completely inside its monitor's work area so
    /// a large saved size never starts partly offscreen or behind the taskbar.
    unsafe fn clamp_to_work_area(hwnd: isize) {
        const MONITOR_DEFAULTTONEAREST: u32 = 2;
        const SWP_NOZORDER: u32 = 0x0004;
        const SWP_NOACTIVATE: u32 = 0x0010;
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        if monitor == 0 {
            return;
        }
        let mut info = MonitorInfo {
            cb_size: core::mem::size_of::<MonitorInfo>() as u32,
            rc_monitor: Rect { left: 0, top: 0, right: 0, bottom: 0 },
            rc_work: Rect { left: 0, top: 0, right: 0, bottom: 0 },
            flags: 0,
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return;
        }
        let mut r = Rect { left: 0, top: 0, right: 0, bottom: 0 };
        if unsafe { GetWindowRect(hwnd, &mut r) } == 0 {
            return;
        }
        let work = info.rc_work;
        let work_w = (work.right - work.left).max(1);
        let work_h = (work.bottom - work.top).max(1);
        let w = (r.right - r.left).min(work_w);
        let h = (r.bottom - r.top).min(work_h);
        let x = r.left.clamp(work.left, work.right - w);
        let y = r.top.clamp(work.top, work.bottom - h);
        if x != r.left || y != r.top || w != (r.right - r.left) || h != (r.bottom - r.top) {
            unsafe {
                SetWindowPos(hwnd, 0, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
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

    fn apply_backdrop_hwnd(hwnd: isize, backdrop: crate::theme::BackdropStyle, dark: bool) {
        use super::backdrop_spec::*;

        let dark_val = dwm_dark_mode(dark);
        let backdrop_val = dwm_backdrop_type(backdrop);
        let [left_width, right_width, top_height, bottom_height] = dwm_margins(backdrop);
        let margins = Margins {
            left_width,
            right_width,
            top_height,
            bottom_height,
        };
        unsafe {
            let hr = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&dark_val as *const i32).cast(),
                core::mem::size_of::<i32>() as u32,
            );
            if hr < 0 {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE_PRE_20H1,
                    (&dark_val as *const i32).cast(),
                    core::mem::size_of::<i32>() as u32,
                );
            }
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                (&backdrop_val as *const u32).cast(),
                core::mem::size_of::<u32>() as u32,
            );
        }
    }

    pub fn apply_backdrop(backdrop: crate::theme::BackdropStyle, dark: bool) {
        if let Some(&hwnd) = HWND.get() {
            apply_backdrop_hwnd(hwnd, backdrop, dark);
        }
    }

    pub fn apply_backdrop_to(
        handle: &WindowHandle,
        backdrop: crate::theme::BackdropStyle,
        dark: bool,
    ) {
        let _ = handle.with_raw_handle(|raw| {
            if let RawWindowHandle::Win32(win) = raw.as_raw() {
                apply_backdrop_hwnd(win.hwnd.get(), backdrop, dark);
            }
        });
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

    pub fn apply_backdrop(_backdrop: crate::theme::BackdropStyle, _dark: bool) {}

    pub fn apply_backdrop_to(
        _handle: &WindowHandle,
        _backdrop: crate::theme::BackdropStyle,
        _dark: bool,
    ) {
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

    pub fn apply_backdrop(backdrop: crate::theme::BackdropStyle, _dark: bool) {
        if let Some(&win) = XWINDOW.get() {
            x11::set_blur_behind(win, super::backdrop_spec::x11_blur_behind(backdrop));
        }
    }

    pub fn apply_backdrop_to(
        handle: &WindowHandle,
        backdrop: crate::theme::BackdropStyle,
        _dark: bool,
    ) {
        let _ = handle.with_raw_handle(|raw| {
            if let RawWindowHandle::Xlib(x) = raw.as_raw() {
                x11::set_blur_behind(
                    x.window as u64,
                    super::backdrop_spec::x11_blur_behind(backdrop),
                );
            }
        });
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
        type ChangeProperty = unsafe extern "C" fn(
            *mut Display,
            Window,
            Atom,
            Atom,
            c_int,
            c_int,
            *const u8,
            c_int,
        ) -> c_int;
        type DeleteProperty = unsafe extern "C" fn(*mut Display, Window, Atom) -> c_int;

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

        pub fn set_blur_behind(window: u64, enable: bool) {
            with_display(|display| unsafe {
                let Some(lib) = lib() else { return };
                let Some(intern) = sym::<InternAtom>(lib, "XInternAtom") else { return };
                let Some(flush) = sym::<Flush>(lib, "XFlush") else { return };
                let c = CString::new("_KDE_NET_WM_BLUR_BEHIND_REGION").unwrap();
                let blur_atom = intern(display, c.as_ptr(), 0);
                if blur_atom == 0 {
                    return;
                }
                if enable {
                    let Some(change) = sym::<ChangeProperty>(lib, "XChangeProperty") else {
                        return;
                    };
                    const XA_CARDINAL: Atom = 6;
                    const PROP_MODE_REPLACE: c_int = 0;
                    change(
                        display,
                        window as Window,
                        blur_atom,
                        XA_CARDINAL,
                        32,
                        PROP_MODE_REPLACE,
                        std::ptr::null(),
                        0,
                    );
                } else {
                    let Some(delete) = sym::<DeleteProperty>(lib, "XDeleteProperty") else {
                        return;
                    };
                    delete(display, window as Window, blur_atom);
                }
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
    pub fn apply_backdrop(_backdrop: crate::theme::BackdropStyle, _dark: bool) {}
    pub fn apply_backdrop_to(
        _handle: &WindowHandle,
        _backdrop: crate::theme::BackdropStyle,
        _dark: bool,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::backdrop_spec::*;
    use crate::theme::BackdropStyle;

    #[test]
    fn dwm_attribute_constants_match_windows_sdk() {
        assert_eq!(DWMWA_USE_IMMERSIVE_DARK_MODE, 20);
        assert_eq!(DWMWA_SYSTEMBACKDROP_TYPE, 38);
        assert_eq!(DWMSBT_NONE, 1);
        assert_eq!(DWMSBT_MAINWINDOW, 2);
        assert_eq!(DWMSBT_TRANSIENTWINDOW, 3);
    }

    #[test]
    fn backdrop_maps_to_dwm_types_margins_and_x11_blur() {
        assert_eq!(dwm_backdrop_type(BackdropStyle::Opaque), DWMSBT_NONE);
        assert_eq!(dwm_backdrop_type(BackdropStyle::Mica), DWMSBT_MAINWINDOW);
        assert_eq!(
            dwm_backdrop_type(BackdropStyle::Acrylic),
            DWMSBT_TRANSIENTWINDOW
        );

        assert_eq!(dwm_margins(BackdropStyle::Opaque), [0, 0, 0, 0]);
        assert_eq!(dwm_margins(BackdropStyle::Mica), [-1, -1, -1, -1]);
        assert_eq!(dwm_margins(BackdropStyle::Acrylic), [-1, -1, -1, -1]);

        assert!(!x11_blur_behind(BackdropStyle::Opaque));
        assert!(x11_blur_behind(BackdropStyle::Mica));
        assert!(x11_blur_behind(BackdropStyle::Acrylic));

        assert_eq!(dwm_dark_mode(false), 0);
        assert_eq!(dwm_dark_mode(true), 1);
    }
}
