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
    /// Signed seconds since the Unix epoch, supporting pre-1970 timestamps.
    pub modified_secs: Option<i64>,
    pub created_secs: Option<i64>,
    pub accessed_secs: Option<i64>,
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

    #[allow(dead_code)]
    pub fn modified_label(&self) -> Option<String> {
        self.modified_secs.map(format_local_datetime)
    }

    #[allow(dead_code)]
    pub fn created_label(&self) -> Option<String> {
        self.created_secs.map(format_local_datetime)
    }

    #[allow(dead_code)]
    pub fn accessed_label(&self) -> Option<String> {
        self.accessed_secs.map(format_local_datetime)
    }
}

/// Convert a `SystemTime` to signed seconds since `UNIX_EPOCH` (flooring
/// sub-second pre-1970 values toward negative infinity).
pub fn system_time_to_secs(t: std::time::SystemTime) -> Option<i64> {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).ok(),
        Err(err) => {
            let before = err.duration();
            let neg = i64::try_from(before.as_secs()).ok()?.checked_neg()?;
            if before.subsec_nanos() > 0 {
                neg.checked_sub(1)
            } else {
                Some(neg)
            }
        }
    }
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

fn utc_civil(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    (
        y.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        m as u32,
        d as u32,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60,
    )
}

#[cfg(all(unix, target_pointer_width = "64"))]
#[repr(C)]
struct Tm {
    tm_sec: std::ffi::c_int,
    tm_min: std::ffi::c_int,
    tm_hour: std::ffi::c_int,
    tm_mday: std::ffi::c_int,
    tm_mon: std::ffi::c_int,
    tm_year: std::ffi::c_int,
    tm_wday: std::ffi::c_int,
    tm_yday: std::ffi::c_int,
    tm_isdst: std::ffi::c_int,
    tm_gmtoff: std::ffi::c_long,
    tm_zone: *const std::ffi::c_char,
}

#[cfg(all(unix, target_pointer_width = "64"))]
unsafe extern "C" {
    fn localtime_r(timep: *const std::ffi::c_long, result: *mut Tm) -> *mut Tm;
    fn mktime(tm: *mut Tm) -> std::ffi::c_long;
}

/// Convert signed Unix epoch seconds into local `(year, month, day, hour, minute, second)`.
pub fn local_civil(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        #[repr(C)]
        struct SystemTimeWin {
            year: u16,
            month: u16,
            day_of_week: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            milliseconds: u16,
        }
        unsafe extern "system" {
            fn FileTimeToLocalFileTime(ft: *const FileTime, local: *mut FileTime) -> i32;
            fn FileTimeToSystemTime(ft: *const FileTime, st: *mut SystemTimeWin) -> i32;
        }
        const WINDOWS_EPOCH_DIFF_SECS: i64 = 11_644_473_600;
        if let Some(ft_secs) = secs.checked_add(WINDOWS_EPOCH_DIFF_SECS)
            && ft_secs >= 0
            && let Some(ticks) = (ft_secs as u64).checked_mul(10_000_000)
        {
            let utc = FileTime {
                low: ticks as u32,
                high: (ticks >> 32) as u32,
            };
            let mut local = FileTime { low: 0, high: 0 };
            let mut st = SystemTimeWin {
                year: 0,
                month: 0,
                day_of_week: 0,
                day: 0,
                hour: 0,
                minute: 0,
                second: 0,
                milliseconds: 0,
            };
            let ok = unsafe {
                FileTimeToLocalFileTime(&utc, &mut local) != 0
                    && FileTimeToSystemTime(&local, &mut st) != 0
            };
            if ok {
                return (
                    st.year as i32,
                    st.month as u32,
                    st.day as u32,
                    st.hour as u32,
                    st.minute as u32,
                    st.second as u32,
                );
            }
        }
    }
    #[cfg(all(unix, target_pointer_width = "64"))]
    {
        let t = secs as std::ffi::c_long;
        let mut out = Tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 0,
            tm_mon: 0,
            tm_year: 0,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: std::ptr::null(),
        };
        let ptr = unsafe { localtime_r(&t, &mut out) };
        if !ptr.is_null() {
            return (
                out.tm_year + 1900,
                (out.tm_mon + 1).max(1) as u32,
                out.tm_mday.max(1) as u32,
                out.tm_hour.clamp(0, 23) as u32,
                out.tm_min.clamp(0, 59) as u32,
                out.tm_sec.clamp(0, 60) as u32,
            );
        }
    }
    utc_civil(secs)
}

/// Format signed Unix epoch seconds as `YYYY-MM-DD HH:MM` in local time.
pub fn format_local_datetime(secs: i64) -> String {
    let (y, m, d, hh, mm, _) = local_civil(secs);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}")
}

/// Format signed Unix epoch seconds as `YYYY-MM-DD` in local time.
pub fn format_local_date(secs: i64) -> String {
    let (y, m, d, _, _, _) = local_civil(secs);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(unix)]
pub(crate) fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = year.rem_euclid(4) == 0
                && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0);
            if leap { 29 } else { 28 }
        }
        _ => 0,
    }
}

#[cfg(unix)]
pub(crate) fn local_to_epoch_secs(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    min: u32,
    sec: u32,
) -> Option<i64> {
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year as i64, month)
        || hour > 23
        || min > 59
        || sec > 59
    {
        return None;
    }
    #[cfg(target_pointer_width = "64")]
    {
        let tm_year = year.checked_sub(1900)?;
        let mut tm = Tm {
            tm_sec: sec as std::ffi::c_int,
            tm_min: min as std::ffi::c_int,
            tm_hour: hour as std::ffi::c_int,
            tm_mday: day as std::ffi::c_int,
            tm_mon: (month - 1) as std::ffi::c_int,
            tm_year: tm_year as std::ffi::c_int,
            tm_wday: -1,
            tm_yday: 0,
            tm_isdst: -1,
            tm_gmtoff: 0,
            tm_zone: std::ptr::null(),
        };
        let res = unsafe { mktime(&mut tm) };
        if res == -1 && tm.tm_wday == -1 {
            return None;
        }
        Some(res as i64)
    }
    #[cfg(not(target_pointer_width = "64"))]
    {
        let y = if month <= 2 { year as i64 - 1 } else { year as i64 };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = if month > 2 { month as i64 - 3 } else { month as i64 + 9 };
        let doy = (153 * mp + 2) / 5 + day as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        days.checked_mul(86_400)?
            .checked_add(hour as i64 * 3600 + min as i64 * 60 + sec as i64)
    }
}

fn signed_seconds(time: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    time.ok().and_then(system_time_to_secs)
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
        modified_secs: None,
        created_secs: None,
        accessed_secs: None,
        extra: Vec::new(),
    };

    let Ok(entry) = std::fs::symlink_metadata(path) else {
        details
            .extra
            .push(("Status".into(), "This item can no longer be read".into()));
        return details;
    };
    details.size = entry.len();
    details.modified_secs = signed_seconds(entry.modified());
    details.created_secs = signed_seconds(entry.created());
    details.accessed_secs = signed_seconds(entry.accessed());
    details.modified = details.modified_secs.and_then(|s| u64::try_from(s).ok());
    details.created = details.created_secs.and_then(|s| u64::try_from(s).ok());
    details.accessed = details.accessed_secs.and_then(|s| u64::try_from(s).ok());

    if entry.file_type().is_symlink() {
        if let Ok(target) = std::fs::read_link(path) {
            details
                .extra
                .push(("Target".into(), target.to_string_lossy().into_owned()));
        }
        return details;
    }

    if is_dir || entry.is_dir() {
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

    #[test]
    fn system_time_to_secs_handles_pre_1970_and_subsec() {
        use std::time::{Duration, UNIX_EPOCH};
        assert_eq!(system_time_to_secs(UNIX_EPOCH), Some(0));
        assert_eq!(system_time_to_secs(UNIX_EPOCH + Duration::new(123, 900_000_000)), Some(123));
        assert_eq!(system_time_to_secs(UNIX_EPOCH - Duration::from_secs(1)), Some(-1));
        assert_eq!(
            system_time_to_secs(UNIX_EPOCH - Duration::from_millis(500)),
            Some(-1),
            "sub-second pre-1970 floors toward negative infinity"
        );
        assert_eq!(
            system_time_to_secs(UNIX_EPOCH - Duration::new(1, 500_000_000)),
            Some(-2)
        );
    }

    #[test]
    fn local_civil_and_formatters_handle_pre_1970() {
        let (y, m, d, hh, mm, ss) = local_civil(-86_400);
        assert!(y == 1969 && (m == 12 || m == 1) && (1..=31).contains(&d));
        assert!(hh < 24 && mm < 60 && ss < 60);
        let dt = format_local_datetime(-86_400);
        assert_eq!(dt.len(), 16, "YYYY-MM-DD HH:MM: {dt}");
        assert_eq!(&dt[4..5], "-");
        assert_eq!(&dt[7..8], "-");
        assert_eq!(&dt[10..11], " ");
        assert_eq!(&dt[13..14], ":");
        let date = format_local_date(-86_400);
        assert_eq!(date, &dt[..10]);
    }

    #[test]
    fn describe_preserves_pre_1970_mtime_and_handles_broken_links() {
        use std::time::{Duration, UNIX_EPOCH};
        let dir = std::env::temp_dir().join(format!("lyre-meta-pre1970-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let file = dir.join("old.txt");
        {
            let f = std::fs::File::create(&file).unwrap();
            let pre1970 = UNIX_EPOCH - Duration::from_secs(86_400 * 10);
            let times = std::fs::FileTimes::new().set_modified(pre1970);
            f.set_times(times).unwrap();
        }
        let details = describe(&file, false, "Text Document");
        assert_eq!(details.modified_secs, Some(-86_400 * 10));
        assert!(details.modified_label().is_some());

        #[cfg(unix)]
        {
            let broken = dir.join("broken-link");
            std::os::unix::fs::symlink(dir.join("missing"), &broken).unwrap();
            let link_details = describe(&broken, false, "Symbolic link");
            assert!(
                !link_details.extra.iter().any(|(k, _)| k == "Status"),
                "broken symlink must not fail as unreadable"
            );
            assert!(link_details.modified_secs.is_some());
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let target = dir.join("gone_target");
            std::fs::create_dir_all(&target).unwrap();
            let junction = dir.join("broken-junction");
            let out = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&junction)
                .arg(&target)
                .creation_flags(0x0800_0000)
                .output()
                .unwrap();
            if out.status.success() {
                std::fs::remove_dir(&target).unwrap();
                let link_details = describe(&junction, true, "Junction");
                assert!(
                    !link_details.extra.iter().any(|(k, _)| k == "Status"),
                    "broken junction must not fail as unreadable"
                );
                assert!(link_details.modified_secs.is_some());
            }
        }

        std::fs::remove_dir_all(&dir).ok();
    }
}
