//! What can be said about a file without opening the program that owns it.
//!
//! Everything here comes from the bytes on disk: the directory entry for size
//! and timestamps, the file header for an image's dimensions, and the content
//! itself for a text file's shape. A folder reports what it directly holds.
//! Nothing is guessed from the name alone, and nothing here loads a codec.

use std::path::Path;

/// One labelled fact, as the Properties dialog and the preview pane show it.
pub type Fact = (String, String);

/// Everything worth showing about one item.
pub struct Details {
    pub name: String,
    pub location: String,
    /// The shell's own type name, which the list already computes.
    pub kind: String,
    pub size: u64,
    /// Seconds since the epoch, for the caller to format.
    pub modified: Option<u64>,
    pub created: Option<u64>,
    pub accessed: Option<u64>,
    /// Type-specific facts, in the order they should be read.
    pub extra: Vec<Fact>,
}

impl Details {
    /// The size as Windows writes it, with the exact byte count beside it.
    pub fn size_label(&self) -> String {
        let bytes = self.size;
        let (value, unit) = if bytes >= 1 << 30 {
            (bytes as f64 / (1u64 << 30) as f64, "GB")
        } else if bytes >= 1 << 20 {
            (bytes as f64 / (1u64 << 20) as f64, "MB")
        } else if bytes >= 1 << 10 {
            (bytes as f64 / (1u64 << 10) as f64, "KB")
        } else {
            (bytes as f64, "bytes")
        };
        if unit == "bytes" {
            format!("{bytes} bytes")
        } else {
            format!("{value:.1} {unit} ({bytes} bytes)")
        }
    }
}

fn seconds(time: std::io::Result<std::time::SystemTime>) -> Option<u64> {
    time.ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
}

/// Read one file or folder.
pub fn describe(path: &Path, is_dir: bool, kind: &str) -> Details {
    let shown = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let location = path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut details = Details {
        name: shown,
        location,
        kind: kind.to_string(),
        size: 0,
        modified: None,
        created: None,
        accessed: None,
        extra: Vec::new(),
    };

    let Ok(entry) = std::fs::metadata(path) else {
        details
            .extra
            .push(("Status".into(), "This item can no longer be read".into()));
        return details;
    };
    details.size = entry.len();
    details.modified = seconds(entry.modified());
    details.created = seconds(entry.created());
    details.accessed = seconds(entry.accessed());

    if is_dir {
        details.size = 0;
        details.extra.extend(folder_facts(path));
        return details;
    }

    if let Some((width, height)) = image_size(path) {
        details
            .extra
            .push(("Dimensions".into(), format!("{width} × {height}")));
        details.extra.push((
            "Megapixels".into(),
            format!("{:.1}", megapixels(width, height)),
        ));
        if width > 0 {
            details.extra.push((
                "Aspect ratio".into(),
                format!("{:.2}:1", height as f64 / width as f64),
            ));
        }
    }
    if let Some((lines, words, characters)) = text_shape(path) {
        details.extra.push(("Lines".into(), lines.to_string()));
        details.extra.push(("Words".into(), words.to_string()));
        details
            .extra
            .push(("Characters".into(), characters.to_string()));
    }
    details
}

fn megapixels(width: u32, height: u32) -> f64 {
    width as f64 * height as f64 / 1_000_000.0
}

/// What a folder directly holds. Counting a whole tree would mean walking it,
/// which is why Explorer says "contains" rather than showing a total size.
fn folder_facts(path: &Path) -> Vec<Fact> {
    let Ok(read) = std::fs::read_dir(path) else {
        return vec![("Contents".into(), "This folder cannot be read".into())];
    };
    let mut files = 0usize;
    let mut folders = 0usize;
    for entry in read.flatten() {
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => folders += 1,
            Ok(_) => files += 1,
            Err(_) => {}
        }
    }
    let plural = |count: usize, one: &str, many: &str| {
        if count == 1 {
            format!("1 {one}")
        } else {
            format!("{count} {many}")
        }
    };
    vec![(
        "Contains".into(),
        format!(
            "{}, {}",
            plural(files, "file", "files"),
            plural(folders, "folder", "folders")
        ),
    )]
}

/// Lines, words and characters, for a file that is plainly text. A NUL byte in
/// the first block means it is not, which is the same test the shell uses.
fn text_shape(path: &Path) -> Option<(usize, usize, usize)> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    file.by_ref().take(8192).read_to_end(&mut head).ok()?;
    if head.is_empty() || head.contains(&0) {
        return None;
    }
    if std::str::from_utf8(&head).is_err() {
        return None;
    }

    // The whole file, so a long one is not counted from its first block only.
    let Ok(bytes) = std::fs::read(path) else {
        return None;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return None;
    };
    let lines = if text.is_empty() {
        0
    } else {
        text.lines().count().max(1)
    };
    let words = text.split_whitespace().count();
    Some((lines, words, text.chars().count()))
}

/// An image's pixel dimensions, read from its header. Uncompressed decoders are
/// not involved: every format states its size near the front.
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; 64 * 1024];
    let read = file.read(&mut head).ok()?;
    let head = &head[..read];

    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        // The first chunk is always IHDR, and its width and height lead it.
        if head.len() >= 24 && &head[12..16] == b"IHDR" {
            let width = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
            let height = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
            return Some((width, height));
        }
        return None;
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        if head.len() >= 10 {
            let width = u16::from_le_bytes([head[6], head[7]]) as u32;
            let height = u16::from_le_bytes([head[8], head[9]]) as u32;
            return Some((width, height));
        }
        return None;
    }
    if head.starts_with(b"BM") && head.len() >= 26 {
        let width = i32::from_le_bytes([head[18], head[19], head[20], head[21]]);
        let height = i32::from_le_bytes([head[22], head[23], head[24], head[25]]);
        return Some((width.unsigned_abs(), height.unsigned_abs()));
    }
    if head.starts_with(&[0xFF, 0xD8]) {
        return jpeg_size(head);
    }
    None
}

/// Walk the JPEG segment chain to a start-of-frame, which holds the size.
fn jpeg_size(head: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 9 < head.len() {
        if head[at] != 0xFF {
            at += 1;
            continue;
        }
        let marker = head[at + 1];
        // Standalone markers carry no length.
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            at += 2;
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            return None;
        }
        let length = u16::from_be_bytes([head[at + 2], head[at + 3]]) as usize;
        // Every start-of-frame marker except the arithmetic-coded set's gaps.
        let is_frame = matches!(
            marker,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
        );
        if is_frame {
            let height = u16::from_be_bytes([head[at + 5], head[at + 6]]) as u32;
            let width = u16::from_be_bytes([head[at + 7], head[at + 8]]) as u32;
            return Some((width, height));
        }
        if length < 2 {
            return None;
        }
        at += 2 + length;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    #[test]
    fn a_png_states_its_size_in_the_first_chunk() {
        let dir = std::env::temp_dir().join(format!("lyre-meta-png-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shot.png");
        std::fs::write(&file, png(1920, 1080)).unwrap();
        assert_eq!(image_size(&file), Some((1920, 1080)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_gif_states_its_size_after_the_signature() {
        let dir = std::env::temp_dir().join(format!("lyre-meta-gif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tiny.gif");
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend_from_slice(&320u16.to_le_bytes());
        bytes.extend_from_slice(&200u16.to_le_bytes());
        std::fs::write(&file, bytes).unwrap();
        assert_eq!(image_size(&file), Some((320, 200)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_jpeg_size_comes_from_its_start_of_frame() {
        let dir = std::env::temp_dir().join(format!("lyre-meta-jpg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("photo.jpg");
        let mut bytes = vec![0xFF, 0xD8];
        // An APP0 segment to be skipped, then a baseline frame header.
        bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]);
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&1080u16.to_be_bytes());
        bytes.extend_from_slice(&1920u16.to_be_bytes());
        bytes.push(0);
        std::fs::write(&file, bytes).unwrap();
        assert_eq!(image_size(&file), Some((1920, 1080)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn text_is_counted_and_binary_is_not() {
        let dir = std::env::temp_dir().join(format!("lyre-meta-txt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let text = dir.join("notes.txt");
        std::fs::write(&text, "one two\nthree\n").unwrap();
        assert_eq!(text_shape(&text), Some((2, 3, 14)));

        let binary = dir.join("blob.bin");
        std::fs::write(&binary, [0u8, 1, 2, 3]).unwrap();
        assert_eq!(text_shape(&binary), None, "a NUL byte means it is not text");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_folder_reports_what_it_directly_holds() {
        let dir = std::env::temp_dir().join(format!("lyre-meta-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let details = describe(&dir, true, "File folder");
        let contains = details
            .extra
            .iter()
            .find(|(label, _)| label == "Contains")
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        assert_eq!(contains, "1 file, 1 folder");
        std::fs::remove_dir_all(&dir).ok();
    }
}
