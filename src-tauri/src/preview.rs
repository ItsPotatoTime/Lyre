//! File previews.
//!
//! The pane shows a picture of the selected file, and the picture is made in
//! this process rather than by handing the webview the original. That
//! distinction matters: letting the webview decode a 50 megapixel photo to show
//! it 300 pixels wide costs a couple of hundred megabytes, which is the opposite
//! of what a file manager should do. Everything here is scaled to fit the pane
//! and cached on disk by content, so re-selecting a file is a file read.
//!
//! The actual decoding is delegated to the tools every desktop already has:
//! `pdftoppm` for PDFs, `ffmpegthumbnailer` for video, and `ffmpeg` or
//! ImageMagick for images. If none of them is installed the pane still shows
//! the file's details, which is a graceful failure rather than a broken one.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Longest edge of a generated thumbnail, in pixels.
const THUMB_EDGE: u32 = 480;
/// How much of a text file is worth reading for a preview.
const TEXT_BYTES: usize = 4096;
const TEXT_LINES: usize = 24;

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
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
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

/// Make the thumbnail if it is not already cached, and return where it is.
fn thumbnail_path(path: &Path) -> Result<PathBuf, String> {
    let meta = std::fs::metadata(path).map_err(|err| format!("{err}"))?;
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
        "video" => run(
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
        ),
        "image" => {
            let scale = format!("scale={THUMB_EDGE}:-1:force_original_aspect_ratio=decrease");
            run("ffmpeg", &["-v", "error", "-y", "-i", &source, "-vf", &scale, &target])
                || run("convert", &[&source, "-thumbnail", &format!("{THUMB_EDGE}x{THUMB_EDGE}"), &target])
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
        std::fs::read(path).ok().map(|bytes| {
            let slice = &bytes[..bytes.len().min(TEXT_BYTES)];
            String::from_utf8_lossy(slice)
                .lines()
                .take(TEXT_LINES)
                .collect::<Vec<_>>()
                .join("\n")
        })
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

    fn have(tool: &str) -> bool {
        Command::new(tool)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    }

    /// The whole path: spawn the tool, write the cache file, read it back.
    /// Skipped rather than failed when the machine has no image tooling.
    #[test]
    fn an_image_produces_a_real_png_thumbnail() {
        if !have("ffmpeg") {
            println!("skipped: ffmpeg not installed");
            return;
        }
        let dir = std::env::temp_dir().join(format!("lyre-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("swatch.png");
        let made = Command::new("ffmpeg")
            .args([
                "-v", "error", "-y", "-f", "lavfi", "-i", "color=c=red:s=64x64",
                "-frames:v", "1",
            ])
            .arg(&image)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        assert!(made, "could not create a test image");

        let bytes = thumbnail(&image.to_string_lossy()).expect("a thumbnail");
        assert!(bytes.len() > 64, "thumbnail is suspiciously small");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");

        // Second call is served from the cache, byte for byte.
        let again = thumbnail(&image.to_string_lossy()).expect("cached thumbnail");
        assert_eq!(bytes, again);

        std::fs::remove_dir_all(&dir).ok();
    }
}
