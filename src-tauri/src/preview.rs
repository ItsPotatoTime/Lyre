//! File previews.
//!
//! The pane shows a picture of the selected file, and the picture is made in
//! this process rather than by handing the webview the original. That
//! distinction matters: letting the webview decode a 50 megapixel photo to show
//! it 300 pixels wide costs a couple of hundred megabytes, which is the opposite
//! of what a file manager should do. Everything here is scaled to fit the pane
//! and cached on disk by content, so re-selecting a file is a file read.
//!
//! Pictures are decoded here, in process. Delegating that to a tool was wrong
//! for a clean install: a fresh Windows account has no `ffmpeg`, no `magick`,
//! and its `convert.exe` is the FAT to NTFS utility rather than ImageMagick, so
//! every picture came back blank. Video and PDF still go to `ffmpegthumbnailer`
//! and `pdftoppm`, and a file neither can be made for still shows its details,
//! which is a graceful failure rather than a broken one.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// How long Windows is given to make a thumbnail before the pane moves on.
const SHELL_BUDGET: Duration = Duration::from_secs(3);
/// One slow extraction at a time, so a folder of films cannot leave a decoder
/// stacking up behind every row.
#[cfg(windows)]
static SHELL_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Files Windows has already refused a thumbnail for. A folder of films would
/// otherwise pay for the same failed extraction every time a row scrolls back
/// into view, because a failure is not written to the cache.
#[cfg(windows)]
static SHELL_REFUSED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();
/// Enough to cover a folder of films without growing without bound.
#[cfg(windows)]
const SHELL_REFUSED_LIMIT: usize = 512;

/// Windows' own thumbnail providers, the ones Explorer fills its windows with.
///
/// Video and PDF are not decoded here. The system asks whichever handler is
/// registered for the type, in the process Explorer uses for the same job, so a
/// machine that shows a frame of a film in Explorer shows one here too, and a
/// machine with no codec shows nothing in either. That is the whole reason to
/// borrow it: the set of formats is as large as the machine's, without shipping
/// a decoder for each one.
#[cfg(windows)]
mod shell {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    /// `IID_IShellItemImageFactory`, the interface that asks for a picture of a
    /// shell item at a given size.
    const IMAGE_FACTORY: Guid = Guid {
        data1: 0xbcc1_8b79,
        data2: 0xba16,
        data3: 0x442f,
        data4: [0x80, 0xc4, 0x8a, 0x59, 0xc3, 0x0c, 0x46, 0x3b],
    };

    #[repr(C)]
    struct Size {
        cx: i32,
        cy: i32,
    }

    #[repr(C)]
    struct Bitmap {
        kind: u32,
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

    /// The vtable of `IShellItemImageFactory`: three `IUnknown` slots and the
    /// one method worth calling.
    #[repr(C)]
    struct FactoryVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_image: unsafe extern "system" fn(*mut c_void, Size, u32, *mut isize) -> i32,
    }

    /// `SIIGBF_THUMBNAILONLY`: a thumbnail, never the type's icon.
    const THUMBNAIL_ONLY: u32 = 0x0000_0008;
    /// `SIIGBF_INCACHEONLY`: answer from what Windows already has, and do not
    /// make anything. This is what keeps selecting a video instant.
    const CACHE_ONLY: u32 = 0x0000_0010;
    /// `COINIT_APARTMENTTHREADED`.
    const APARTMENT_THREADED: u32 = 0x2;

    #[link(name = "shell32")]
    extern "system" {
        fn SHCreateItemFromParsingName(
            path: *const u16,
            bind_context: *mut c_void,
            interface: *const Guid,
            out: *mut *mut c_void,
        ) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, model: u32) -> i32;
        fn CoUninitialize();
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn GetDIBits(
            dc: isize,
            bitmap: isize,
            start: u32,
            lines: u32,
            bits: *mut c_void,
            info: *mut BitmapInfo,
            usage: u32,
        ) -> i32;
        fn DeleteObject(object: isize) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetDC(window: isize) -> isize;
        fn ReleaseDC(window: isize, dc: isize) -> i32;
        fn GetObjectW(object: isize, size: i32, out: *mut c_void) -> i32;
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// The shell's picture of a file, or nothing.
    ///
    /// `cached_only` refuses to do any work: it answers only if Windows already
    /// holds a thumbnail for this file, which is the difference between a pane
    /// that opens on a click and one that waits for a decoder.
    pub fn thumbnail(path: &Path, edge: u32, cached_only: bool) -> Option<image::RgbaImage> {
        // A positive result means this thread now needs the matching release.
        let initialized = unsafe { CoInitializeEx(std::ptr::null_mut(), APARTMENT_THREADED) } >= 0;
        let picture = unsafe { ask(path, edge, cached_only) };
        if initialized {
            unsafe { CoUninitialize() };
        }
        picture
    }

    unsafe fn ask(path: &Path, edge: u32, cached_only: bool) -> Option<image::RgbaImage> {
        let wide_path = wide(path);
        let mut factory: *mut c_void = std::ptr::null_mut();
        let created = unsafe {
            SHCreateItemFromParsingName(
                wide_path.as_ptr(),
                std::ptr::null_mut(),
                &IMAGE_FACTORY,
                &mut factory,
            )
        };
        if created < 0 || factory.is_null() {
            return None;
        }

        let vtbl = unsafe { *(factory as *const *const FactoryVtbl) };
        let mut flags = THUMBNAIL_ONLY;
        if cached_only {
            flags |= CACHE_ONLY;
        }
        let mut bitmap: isize = 0;
        let asked = unsafe {
            ((*vtbl).get_image)(
                factory,
                Size {
                    cx: edge as i32,
                    cy: edge as i32,
                },
                flags,
                &mut bitmap,
            )
        };
        unsafe { ((*vtbl).release)(factory) };
        if asked < 0 || bitmap == 0 {
            // Visible under `cargo test -- --nocapture`; a GUI process has no
            // console, so this costs nothing in the shipped window.
            eprintln!("shell GetImage failed for {}: hresult {asked:#010x}", path.display());
            return None;
        }

        let picture = unsafe { to_picture(bitmap) };
        unsafe { DeleteObject(bitmap) };
        picture
    }

    /// Copies the bitmap out and turns its pixels the right way round.
    unsafe fn to_picture(bitmap: isize) -> Option<image::RgbaImage> {
        let mut described = Bitmap {
            kind: 0,
            width: 0,
            height: 0,
            width_bytes: 0,
            planes: 0,
            bits_per_pixel: 0,
            bits: std::ptr::null_mut(),
        };
        let described_size = std::mem::size_of::<Bitmap>() as i32;
        if unsafe { GetObjectW(bitmap, described_size, &mut described as *mut Bitmap as *mut c_void) }
            == 0
        {
            return None;
        }
        if described.width <= 0 || described.height <= 0 {
            return None;
        }
        let width = described.width as u32;
        let height = described.height as u32;

        let mut info = BitmapInfo {
            header: BitmapInfoHeader {
                size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                width: width as i32,
                // A negative height asks for the rows top down, which is the
                // order the image crate stores them in.
                height: -(height as i32),
                planes: 1,
                bit_count: 32,
                compression: 0,
                size_image: 0,
                x_pels: 0,
                y_pels: 0,
                colors_used: 0,
                colors_important: 0,
            },
            colors: [0; 3],
        };
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let dc = unsafe { GetDC(0) };
        if dc == 0 {
            return None;
        }
        let lines = unsafe {
            GetDIBits(
                dc,
                bitmap,
                0,
                height,
                pixels.as_mut_ptr() as *mut c_void,
                &mut info,
                0,
            )
        };
        unsafe { ReleaseDC(0, dc) };
        if lines == 0 {
            return None;
        }

        // The bytes arrive as BGRA. Providers that draw an opaque picture often
        // leave alpha at zero, so an image where nothing is transparent gets a
        // solid alpha rather than an invisible thumbnail.
        let mut any_alpha = false;
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            any_alpha |= pixel[3] != 0;
        }
        if !any_alpha {
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }

        image::RgbaImage::from_raw(width, height, pixels)
    }
}

/// Longest edge of a generated thumbnail, in pixels.
const THUMB_EDGE: u32 = 480;
/// How much of a text file is worth reading for a preview.
const TEXT_BYTES: usize = 4096;
const TEXT_LINES: usize = 24;
/// Beyond this, decoding costs more than a preview is worth: a 300 MB image or
/// a film would hold the decoder for a long time and produce a 480 pixel square.
const MAX_PREVIEW_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewInfo {
    /// image, video, audio, pdf, text, folder or other.
    pub kind: String,
    pub size: u64,
    pub modified_ms: Option<i64>,
    /// First lines of a text file, if it is one.
    pub excerpt: Option<String>,
    /// Whether a thumbnail can be produced for this file.
    pub has_thumbnail: bool,
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn kind_of(path: &Path, is_dir: bool) -> &'static str {
    if is_dir {
        return "folder";
    }
    match extension(path).as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "tif" | "avif" | "heic"
        | "svg" | "ico" => "image",
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg" => "video",
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "opus" | "aac" | "wma" => "audio",
        "pdf" => "pdf",
        "txt" | "md" | "rs" | "ts" | "tsx" | "js" | "json" | "toml" | "yaml" | "yml" | "css"
        | "html" | "sh" | "py" | "go" | "c" | "h" | "cpp" | "log" | "ini" | "conf" | "csv" => "text",
        _ => "other",
    }
}

fn cache_dir() -> Option<PathBuf> {
    // A Windows account has neither XDG_CACHE_HOME nor HOME, so the lookup used
    // to come back empty and the thumbnailer gave up before it looked for any
    // decoder at all.
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".cache"))
        });
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")));
    let base = base?;
    Some(base.join("lyre").join("thumbnails"))
}

/// A cache key that changes when the file does. Not cryptographic: it only
/// needs to separate one file's thumbnail from another's.
fn cache_key(path: &Path, size: u64, modified: Option<i64>) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    size.hash(&mut hasher);
    modified.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn run(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Decodes a picture here and writes it scaled to the cache.
///
/// This is the path that has to work on a machine nobody has prepared: no
/// ffmpeg, no ImageMagick, nothing to install first.
fn native_image(path: &Path, target: &Path) -> bool {
    let Ok(reader) = image::ImageReader::open(path) else {
        return false;
    };
    let Ok(reader) = reader.with_guessed_format() else {
        return false;
    };
    let Ok(picture) = reader.decode() else {
        return false;
    };
    // `thumbnail` keeps the aspect ratio and is much cheaper than a general
    // resize, which is the whole point of doing this instead of the webview.
    picture
        .thumbnail(THUMB_EDGE, THUMB_EDGE)
        .save(target)
        .is_ok()
}

/// ImageMagick, under the name current releases use.
///
/// `convert` is ImageMagick 6's name and also a Windows system tool that turns a
/// volume into NTFS, so it is only tried where that collision cannot happen.
fn imagemagick(source: &str, target: &str) -> bool {
    let size = format!("{THUMB_EDGE}x{THUMB_EDGE}");
    if run("magick", &[source, "-thumbnail", &size, target]) {
        return true;
    }
    #[cfg(not(windows))]
    {
        if run("convert", &[source, "-thumbnail", &size, target]) {
            return true;
        }
    }
    false
}

/// Asks Windows for a thumbnail, waiting no longer than the budget.
///
/// The shell answers immediately when it already holds a thumbnail for the file,
/// which is the case for anything Explorer has shown, so that is asked first and
/// costs nothing when it hits. Only a miss goes to a worker thread, and only one
/// of those runs at a time. The result is cached like any other thumbnail, so the
/// next visit to the file does not pay for it again.
fn shell_picture(path: &Path) -> Option<image::RgbaImage> {
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
    #[cfg(windows)]
    {
        use std::sync::atomic::Ordering;

        if let Some(picture) = shell::thumbnail(path, THUMB_EDGE, true) {
            return Some(picture);
        }
        if shell_refused(path) {
            return None;
        }
        if SHELL_BUSY.swap(true, Ordering::AcqRel) {
            // Something is already decoding; answering nothing is better than
            // piling a second decoder on top of it.
            return None;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let owned = path.to_path_buf();
        std::thread::spawn(move || {
            let picture = shell::thumbnail(&owned, THUMB_EDGE, false);
            SHELL_BUSY.store(false, Ordering::Release);
            let _ = sender.send(picture);
        });
        match receiver.recv_timeout(SHELL_BUDGET) {
            Ok(Some(picture)) => Some(picture),
            // The provider looked at the file and could not decode it. Worth
            // remembering, unlike a timeout, which may simply have been busy.
            Ok(None) => {
                remember_refusal(path);
                None
            }
            Err(_) => None,
        }
    }
}

/// Whether Windows has already failed to make a thumbnail for this file.
#[cfg(windows)]
fn shell_refused(path: &Path) -> bool {
    let refused = SHELL_REFUSED.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    let set = refused.lock().unwrap_or_else(|poison| poison.into_inner());
    set.contains(&path.to_string_lossy().into_owned())
}

#[cfg(windows)]
fn remember_refusal(path: &Path) {
    let refused = SHELL_REFUSED.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    let mut set = refused.lock().unwrap_or_else(|poison| poison.into_inner());
    if set.len() >= SHELL_REFUSED_LIMIT {
        set.clear();
    }
    set.insert(path.to_string_lossy().into_owned());
}

/// Writes the shell's picture into the cache, if it gives one.
fn save_shell_picture(path: &Path, target: &Path) -> bool {
    match shell_picture(path) {
        Some(picture) => picture.save(target).is_ok(),
        None => false,
    }
}

/// Make the thumbnail if it is not already cached, and return where it is.
fn thumbnail_path(path: &Path) -> Result<PathBuf, String> {
    let meta = std::fs::metadata(path).map_err(|err| format!("{err}"))?;
    if meta.len() > MAX_PREVIEW_BYTES {
        return Err("File is too large to preview.".into());
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64);
    let dir = cache_dir().ok_or("No cache directory.")?;
    std::fs::create_dir_all(&dir).map_err(|err| format!("{err}"))?;
    let out = dir.join(format!("{}.png", cache_key(path, meta.len(), modified)));
    if out.is_file() && out.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(out);
    }

    let source = path.to_string_lossy().into_owned();
    let target = out.to_string_lossy().into_owned();
    let made = match kind_of(path, false) {
        "pdf" => {
            // Windows draws PDF pages through the handler the system already
            // has, which is the one Explorer uses.
            if save_shell_picture(path, &out) {
                true
            } else {
                // `-singlefile` writes exactly `<target>` with a .png extension.
                let stem = target.trim_end_matches(".png").to_string();
                run(
                    "pdftoppm",
                    &[
                        "-png",
                        "-f",
                        "1",
                        "-l",
                        "1",
                        "-singlefile",
                        "-scale-to",
                        &THUMB_EDGE.to_string(),
                        &source,
                        &stem,
                    ],
                )
            }
        }
        "video" => {
            if save_shell_picture(path, &out) {
                true
            } else {
                run(
                    "ffmpegthumbnailer",
                    &[
                        "-i",
                        &source,
                        "-o",
                        &target,
                        "-s",
                        &THUMB_EDGE.to_string(),
                        "-q",
                        "8",
                    ],
                )
            }
        }
        "image" => {
            // In process first: it is the only path a clean install has.
            if native_image(path, &out) {
                return Ok(out);
            }
            // Formats with no decoder here, such as HEIC or AVIF, may still have
            // one registered with the system.
            if save_shell_picture(path, &out) {
                return Ok(out);
            }
            let scale = format!("scale={THUMB_EDGE}:-1:force_original_aspect_ratio=decrease");
            run("ffmpeg", &["-v", "error", "-y", "-i", &source, "-vf", &scale, &target])
                || imagemagick(&source, &target)
        }
        _ => false,
    };

    if made && out.is_file() {
        Ok(out)
    } else {
        // A zero byte file left behind by a failed attempt would be served as
        // a broken image next time, so it is removed.
        let _ = std::fs::remove_file(&out);
        Err("No tool could preview this file.".into())
    }
}

/// Read only as much as the preview can show.
///
/// This used to read the entire file and then take the first few kilobytes,
/// which meant selecting a large log read the whole thing into memory to
/// display two dozen lines of it.
fn excerpt_of(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut buffer = Vec::with_capacity(TEXT_BYTES);
    file.take(TEXT_BYTES as u64).read_to_end(&mut buffer).ok()?;
    Some(
        String::from_utf8_lossy(&buffer)
            .lines()
            .take(TEXT_LINES)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The shape the interface expects when a preview cannot be produced at all.
pub fn unavailable() -> PreviewInfo {
    PreviewInfo {
        kind: "other".into(),
        size: 0,
        modified_ms: None,
        excerpt: None,
        has_thumbnail: false,
    }
}

fn modified_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

pub fn info(path: &str) -> PreviewInfo {
    let path = Path::new(path);
    let meta = std::fs::metadata(path).ok();
    let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let kind = kind_of(path, is_dir);

    let excerpt = if kind == "text" {
        excerpt_of(path)
    } else {
        None
    };

    PreviewInfo {
        kind: kind.to_string(),
        size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
        modified_ms: modified_ms(path),
        excerpt,
        has_thumbnail: matches!(kind, "image" | "video" | "pdf"),
    }
}

/// The thumbnail's PNG bytes, for the interface to display.
pub fn thumbnail(path: &str) -> Result<Vec<u8>, String> {
    let file = thumbnail_path(Path::new(path))?;
    std::fs::read(&file).map_err(|err| format!("{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_classified_by_extension() {
        assert_eq!(kind_of(Path::new("/a/photo.JPG"), false), "image");
        assert_eq!(kind_of(Path::new("/a/film.mkv"), false), "video");
        assert_eq!(kind_of(Path::new("/a/song.flac"), false), "audio");
        assert_eq!(kind_of(Path::new("/a/manual.pdf"), false), "pdf");
        assert_eq!(kind_of(Path::new("/a/notes.md"), false), "text");
        assert_eq!(kind_of(Path::new("/a/thing.bin"), false), "other");
        assert_eq!(kind_of(Path::new("/a/folder"), true), "folder");
    }

    #[test]
    fn a_text_file_is_summarised_by_its_first_lines() {
        let dir = std::env::temp_dir().join(format!("lyre-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "first line\nsecond line\nthird line\n").unwrap();

        let info = info(&file.to_string_lossy());
        assert_eq!(info.kind, "text");
        let excerpt = info.excerpt.unwrap();
        assert!(excerpt.starts_with("first line\nsecond line"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_reports_instead_of_panicking() {
        let info = info("/definitely/not/here.txt");
        assert_eq!(info.kind, "text");
        assert_eq!(info.size, 0);
        assert!(info.excerpt.is_none());
    }

    /// The cache key has to change when the file does, or an edited picture
    /// keeps showing its old thumbnail.
    #[test]
    fn the_cache_key_follows_the_file() {
        let first = cache_key(Path::new("/a.png"), 10, Some(1));
        assert_eq!(first, cache_key(Path::new("/a.png"), 10, Some(1)));
        assert_ne!(first, cache_key(Path::new("/a.png"), 11, Some(1)));
        assert_ne!(first, cache_key(Path::new("/a.png"), 10, Some(2)));
        assert_ne!(first, cache_key(Path::new("/b.png"), 10, Some(1)));
    }
}

#[cfg(test)]
mod thumbnail_tests {
    use super::*;

    /// The whole path: decode, scale, write the cache file, read it back, and
    /// get the same bytes the second time. Nothing external is involved, which
    /// is the point: a clean install has no image tooling at all.
    #[test]
    fn an_image_produces_a_real_png_thumbnail() {
        let dir = std::env::temp_dir().join(format!("lyre-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("swatch.png");
        let picture = image::RgbImage::from_fn(640, 480, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 160])
        });
        picture.save(&source).expect("write a test picture");

        let bytes = thumbnail(&source.to_string_lossy()).expect("a thumbnail");
        assert!(bytes.len() > 64, "thumbnail is suspiciously small");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");

        // Second call is served from the cache, byte for byte.
        let again = thumbnail(&source.to_string_lossy()).expect("cached thumbnail");
        assert_eq!(bytes, again);

        // Scaled to the long edge, with the picture's shape kept.
        let made = image::load_from_memory(&bytes).expect("the thumbnail decodes");
        assert!(made.width() <= THUMB_EDGE && made.height() <= THUMB_EDGE);
        assert_eq!(
            made.width() * 3,
            made.height() * 4,
            "the aspect ratio should survive"
        );

        // The same path for a JPEG, which is what photographs actually are.
        let photo = dir.join("photo.jpg");
        image::RgbImage::from_fn(320, 200, |x, y| image::Rgb([x as u8, y as u8, 90]))
            .save(&photo)
            .expect("write a test photo");
        let from_photo = thumbnail(&photo.to_string_lossy()).expect("a jpeg thumbnail");
        assert_eq!(&from_photo[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The bug that made every preview blank on Windows: no XDG_CACHE_HOME and
    /// no HOME, so there was nowhere to put a thumbnail and no decoder was ever
    /// reached.
    #[test]
    fn the_cache_is_found_on_every_desktop() {
        let dir = cache_dir().expect("a cache directory");
        assert_eq!(
            dir.file_name().and_then(|name| name.to_str()),
            Some("thumbnails")
        );
    }

    /// The COM plumbing for Windows' own providers. Ignored by default: it asks
    /// the running shell for a picture, which is not something a test run should
    /// depend on everywhere.
    #[cfg(windows)]
    #[test]
    #[ignore = "asks the running Windows shell for a thumbnail"]
    fn the_shell_hands_back_a_picture() {
        let dir = std::env::temp_dir().join("lyre-shell-thumb");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("swatch.png");
        image::RgbImage::from_fn(512, 384, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 60])
        })
        .save(&source)
        .expect("write a test picture");

        let picture = shell_picture(&source).expect("the shell should give a picture");
        assert!(picture.width() > 0 && picture.height() > 0);
        assert!(picture.width() <= THUMB_EDGE || picture.height() <= THUMB_EDGE);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Times the shell path on a real file, which is how the budget above was
    /// chosen. Run it with `LYRE_THUMB_SAMPLE=<path>` and `--ignored --nocapture`.
    #[cfg(windows)]
    #[test]
    #[ignore = "measures a real file; set LYRE_THUMB_SAMPLE"]
    fn how_long_the_shell_takes() {
        let Ok(sample) = std::env::var("LYRE_THUMB_SAMPLE") else {
            return;
        };
        let path = PathBuf::from(&sample);

        let started = std::time::Instant::now();
        let first = shell_picture(&path);
        println!(
            "cold: {:?} -> {:?}",
            started.elapsed(),
            first.as_ref().map(|picture| (picture.width(), picture.height()))
        );

        let started = std::time::Instant::now();
        let second = shell_picture(&path);
        println!(
            "warm: {:?} -> {:?}",
            started.elapsed(),
            second.as_ref().map(|picture| (picture.width(), picture.height()))
        );

        let started = std::time::Instant::now();
        match thumbnail(&sample) {
            Ok(bytes) => println!("through the cache: {:?} -> {} bytes", started.elapsed(), bytes.len()),
            Err(err) => println!("through the cache: {:?} -> {err}", started.elapsed()),
        }
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;

    /// A preview of a large file must not read the large file.
    #[test]
    fn a_huge_text_file_is_read_only_at_the_head() {
        let dir = std::env::temp_dir().join(format!("lyre-huge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("server.log");
        // 8 MB of one long line, then a marker far past the preview window.
        let mut body = vec![b'x'; 8 * 1024 * 1024];
        body.extend_from_slice(b"\nMARKER-BEYOND-THE-HEAD\n");
        std::fs::write(&file, &body).unwrap();

        let started = std::time::Instant::now();
        let info = info(&file.to_string_lossy());
        let elapsed = started.elapsed();

        let excerpt = info.excerpt.expect("an excerpt");
        assert!(excerpt.len() <= TEXT_BYTES, "excerpt is bounded");
        assert!(
            !excerpt.contains("MARKER-BEYOND-THE-HEAD"),
            "nothing past the head should be read"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "reading 8 MB took {elapsed:?}, so the whole file is still being slurped"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
