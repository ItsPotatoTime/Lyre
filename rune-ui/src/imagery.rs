//! Real platform icons for files, folders and apps, as straight-alpha sRGB RGBA8.
//!
//! Windows reads the icon Explorer itself shows — the folder icon, a document's
//! default-app icon, or an executable's own embedded icon — through `SHGetFileInfoW`.
//! macOS and Linux use Rune's cross-platform `app_icon_rgba` (NSWorkspace / the
//! freedesktop icon theme). Image *previews* are decoded separately through Rune's
//! cross-platform `ImageCache`, so they work the same on every platform.

use std::path::Path;

/// The shell icon for `path` at about `px` pixels, or `None` to draw a fallback.
#[cfg(target_os = "windows")]
pub fn icon_rgba(path: &str, is_dir: bool, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    win::shell_icon(path, is_dir, px)
}

#[cfg(not(target_os = "windows"))]
pub fn icon_rgba(path: &str, _is_dir: bool, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    rune::image::app_icon_rgba(Path::new(path), px.max(1)).ok()
}

/// Prepare a worker thread for shell icon calls. Windows shell functions need
/// COM on the calling thread; the UI thread gets it from the window system, a
/// bare worker does not.
#[cfg(target_os = "windows")]
pub fn prepare_thread() {
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut std::ffi::c_void, concurrency: u32) -> i32;
    }
    // COINIT_APARTMENTTHREADED, matching the UI thread's shell apartment.
    unsafe { CoInitializeEx(std::ptr::null_mut(), 0x2) };
}

#[cfg(not(target_os = "windows"))]
pub fn prepare_thread() {}

/// Undo `prepare_thread` when the worker retires.
#[cfg(target_os = "windows")]
pub fn release_thread() {
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoUninitialize();
    }
    unsafe { CoUninitialize() };
}

#[cfg(not(target_os = "windows"))]
pub fn release_thread() {}

/// Whether a file carries its own icon (so the shell must read the file rather
/// than look the type up by extension).
#[cfg(target_os = "windows")]
fn carries_own_icon(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| matches!(e.as_str(), "exe" | "ico" | "lnk" | "scr" | "dll" | "icl" | "cpl"))
}

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::c_void;

    const SHGFI_ICON: u32 = 0x0000_0100;
    const SHGFI_LARGEICON: u32 = 0x0000_0000;
    const SHGFI_USEFILEATTRIBUTES: u32 = 0x0000_0010;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const BI_RGB: u32 = 0;
    const DIB_RGB_COLORS: u32 = 0;

    #[repr(C)]
    struct ShFileInfoW {
        hicon: *mut c_void,
        i_icon: i32,
        dw_attributes: u32,
        sz_display_name: [u16; 260],
        sz_type_name: [u16; 80],
    }

    #[repr(C)]
    struct IconInfo {
        is_icon: i32,
        x_hotspot: u32,
        y_hotspot: u32,
        hbm_mask: *mut c_void,
        hbm_color: *mut c_void,
    }

    #[repr(C)]
    struct Bitmap {
        kind: i32,
        width: i32,
        height: i32,
        width_bytes: i32,
        planes: u16,
        bits_per_pixel: u16,
        bits: *mut c_void,
    }

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_pels: i32,
        y_pels: i32,
        colors_used: u32,
        colors_important: u32,
    }

    #[repr(C)]
    struct BitmapInfo {
        header: BitmapInfoHeader,
        colors: [u32; 3],
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetFileInfoW(
            path: *const u16,
            attributes: u32,
            info: *mut ShFileInfoW,
            info_size: u32,
            flags: u32,
        ) -> usize;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetIconInfo(icon: *mut c_void, info: *mut IconInfo) -> i32;
        fn DestroyIcon(icon: *mut c_void) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn GetObjectW(object: *mut c_void, size: i32, out: *mut c_void) -> i32;
        fn GetDIBits(
            dc: *mut c_void,
            bitmap: *mut c_void,
            start: u32,
            lines: u32,
            bits: *mut c_void,
            info: *mut BitmapInfo,
            usage: u32,
        ) -> i32;
        fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
        fn DeleteDC(dc: *mut c_void) -> i32;
        fn DeleteObject(object: *mut c_void) -> i32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The icon Explorer shows for a path. Folders and ordinary files are looked
    /// up by attribute/extension (no disk read); executables, icon files and
    /// shortcuts are read so their own icon comes back.
    pub fn shell_icon(path: &str, is_dir: bool, _px: u32) -> Option<(u32, u32, Vec<u8>)> {
        let attributes = if is_dir {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        if super::carries_own_icon(path)
            && let Some(picture) = query(path, attributes, SHGFI_ICON | SHGFI_LARGEICON)
        {
            return Some(picture);
        }
        // A shortcut whose target has gone has nothing to resolve, so it lands
        // here and draws the mark of its type rather than nothing at all.
        query(
            path,
            attributes,
            SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
        )
    }

    /// One `SHGetFileInfoW` call, as RGBA.
    fn query(path: &str, attributes: u32, flags: u32) -> Option<(u32, u32, Vec<u8>)> {
        let wide_path = wide(path);
        if wide_path.len() > 260 {
            return None;
        }
        let mut info: ShFileInfoW = unsafe { std::mem::zeroed() };
        let result = unsafe {
            SHGetFileInfoW(
                wide_path.as_ptr(),
                attributes,
                &mut info,
                std::mem::size_of::<ShFileInfoW>() as u32,
                flags,
            )
        };
        if result == 0 || info.hicon.is_null() {
            return None;
        }
        let picture = unsafe { hicon_rgba(info.hicon) };
        unsafe { DestroyIcon(info.hicon) };
        picture
    }

    /// Copy an HICON's colour bitmap out as top-down straight-alpha RGBA8.
    unsafe fn hicon_rgba(icon: *mut c_void) -> Option<(u32, u32, Vec<u8>)> {
        let mut info: IconInfo = unsafe { std::mem::zeroed() };
        if unsafe { GetIconInfo(icon, &mut info) } == 0 {
            return None;
        }
        let result = (|| {
            if info.hbm_color.is_null() {
                return None;
            }
            let mut bitmap: Bitmap = unsafe { std::mem::zeroed() };
            if unsafe {
                GetObjectW(
                    info.hbm_color,
                    std::mem::size_of::<Bitmap>() as i32,
                    (&mut bitmap as *mut Bitmap).cast(),
                )
            } == 0
            {
                return None;
            }
            let (w, h) = (bitmap.width, bitmap.height);
            if w <= 0 || h <= 0 || w > 1024 || h > 1024 {
                return None;
            }
            let mut header = BitmapInfo {
                header: BitmapInfoHeader {
                    size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                    width: w,
                    height: -h,
                    planes: 1,
                    bit_count: 32,
                    compression: BI_RGB,
                    size_image: 0,
                    x_pels: 0,
                    y_pels: 0,
                    colors_used: 0,
                    colors_important: 0,
                },
                colors: [0; 3],
            };
            let mut pixels = vec![0u8; w as usize * h as usize * 4];
            let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
            let rows = unsafe {
                GetDIBits(
                    dc,
                    info.hbm_color,
                    0,
                    h as u32,
                    pixels.as_mut_ptr().cast(),
                    &mut header,
                    DIB_RGB_COLORS,
                )
            };
            unsafe { DeleteDC(dc) };
            if rows == 0 {
                return None;
            }
            // BGRA; icons without an alpha channel leave it zero and draw opaque.
            let opaque = pixels.chunks_exact(4).all(|p| p[3] == 0);
            for p in pixels.chunks_exact_mut(4) {
                p.swap(0, 2);
                if opaque {
                    p[3] = 255;
                }
            }
            Some((w as u32, h as u32, pixels))
        })();
        unsafe {
            if !info.hbm_color.is_null() {
                DeleteObject(info.hbm_color);
            }
            if !info.hbm_mask.is_null() {
                DeleteObject(info.hbm_mask);
            }
        }
        result
    }
}
