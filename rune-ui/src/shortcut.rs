//! What a shortcut file points at.
//!
//! Handing a shortcut to a folder to the shell asks whichever program opens
//! folders to open it — and that is Lyre, so a second Lyre window appeared on
//! the folder instead of the folder opening in this one. Resolving the shortcut
//! first lets a shortcut to a folder open as a folder, while every other
//! shortcut is still handed to the shell, which is what runs it with its
//! arguments, its working directory and its window state.
//!
//! Only `.lnk` is read here. A `.url` names a web address rather than a place on
//! disk, and a `.desktop` file is a launcher; the shell already runs both.

use std::path::{Path, PathBuf};

/// A shell link starts with its header, whose size is fixed.
const HEADER: usize = 0x4c;

/// LinkFlags: a target ID list follows the header.
const HAS_TARGET_ID_LIST: u32 = 0x1;
/// LinkFlags: a LinkInfo structure follows the target ID list.
const HAS_LINK_INFO: u32 = 0x2;
/// LinkInfoFlags: the link names a place on this machine, so it carries a local
/// base path. A link to a network share carries the share's pieces instead.
const HAS_LOCAL_BASE_PATH: u32 = 0x1;

/// The path a shortcut names, when it is a `.lnk` that names one this machine
/// can build a path from.
pub fn target(path: &str) -> Option<PathBuf> {
    if !is_shortcut(path) {
        return None;
    }
    target_in(&std::fs::read(path).ok()?)
}

/// The folder a shortcut opens, when that folder is really there.
pub fn folder_target(path: &str) -> Option<PathBuf> {
    let target = target(path)?;
    target.is_dir().then_some(target)
}

/// Whether `path` is a Windows shortcut rather than a file of its own.
pub fn is_shortcut(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
}

/// The target named inside a shortcut's own bytes.
///
/// The format is Microsoft's Shell Link Binary File Format: a fixed header, an
/// optional target ID list, then the `LinkInfo` block that records where the
/// target is on disk. That block is the only part read here, because it is the
/// part that answers with a path.
fn target_in(bytes: &[u8]) -> Option<PathBuf> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    if word(0)? != HEADER as u32 {
        return None;
    }
    let flags = word(0x14)?;
    if flags & HAS_LINK_INFO == 0 {
        // A link that only carries an ID list names its target through the
        // shell's namespace rather than a path, which is the shell's job.
        return None;
    }
    let mut at = HEADER;
    if flags & HAS_TARGET_ID_LIST != 0 {
        let size = u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize;
        at += 2 + size;
    }
    // LinkInfo: its size, the header size that says which offsets it carries,
    // its flags, then offsets measured from the start of the block.
    let size = word(at)? as usize;
    let header = word(at + 4)? as usize;
    let link_flags = word(at + 8)?;
    if size == 0 || header < 0x1c || at + size > bytes.len() || link_flags & HAS_LOCAL_BASE_PATH == 0 {
        return None;
    }
    let field = |offset: usize| -> Option<usize> { Some(word(at + offset)? as usize) };
    // A Windows that writes Unicode also writes UTF-16 copies of both strings,
    // which is the only pair that survives a path outside the ANSI code page.
    let unicode = header >= 0x24 && field(0x1c)? != 0 && field(0x20)? != 0;
    let (base, suffix, read): (usize, usize, Reader) = if unicode {
        (field(0x1c)?, field(0x20)?, read_utf16)
    } else {
        (field(0x10)?, field(0x18)?, read_ansi)
    };
    if base == 0 {
        return None;
    }
    let mut text = read(bytes, at + base)?;
    // The path is the base path plus the common suffix; the last component is
    // often what the suffix holds.
    if suffix != 0 && let Some(tail) = read(bytes, at + suffix) {
        text.push_str(&tail);
    }
    Some(PathBuf::from(text))
}

/// Reads one of the two strings a shortcut's `LinkInfo` block carries.
type Reader = fn(&[u8], usize) -> Option<String>;

/// A null-terminated UTF-16LE string.
fn read_utf16(bytes: &[u8], at: usize) -> Option<String> {
    let mut units = Vec::new();
    for pair in bytes.get(at..)?.as_chunks::<2>().0 {
        let unit = u16::from_le_bytes(*pair);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    (!units.is_empty()).then(|| String::from_utf16_lossy(&units))
}

/// A null-terminated string in the ANSI code page, which every character of an
/// ordinary path shares with ASCII.
fn read_ansi(bytes: &[u8], at: usize) -> Option<String> {
    let rest = bytes.get(at..)?;
    let end = rest.iter().position(|byte| *byte == 0)?;
    (end > 0).then(|| String::from_utf8_lossy(&rest[..end]).into_owned())
}

/// A shortcut laid out the way Explorer writes one: a header, an empty target
/// ID list, then a `LinkInfo` block whose strings are UTF-16. Tests build the
/// bytes with this because a real `.lnk` needs the shell to write it.
#[cfg(test)]
pub(crate) fn encoded(base: &str, suffix: &str) -> Vec<u8> {
    let mut bytes = vec![0u8; HEADER];
    bytes[0..4].copy_from_slice(&(HEADER as u32).to_le_bytes());
    bytes[0x14..0x18].copy_from_slice(&(HAS_TARGET_ID_LIST | HAS_LINK_INFO | 0x8).to_le_bytes());
    // The target ID list: a size, then that many bytes of namespace items.
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 4]);
    let base_units: Vec<u16> = base.encode_utf16().chain(std::iter::once(0)).collect();
    let suffix_units: Vec<u16> = suffix.encode_utf16().chain(std::iter::once(0)).collect();
    let size = 0x24 + base_units.len() * 2 + suffix_units.len() * 2;
    let mut block = vec![0u8; 0x24];
    block[0x00..0x04].copy_from_slice(&(size as u32).to_le_bytes());
    block[0x04..0x08].copy_from_slice(&0x24u32.to_le_bytes());
    block[0x08..0x0c].copy_from_slice(&HAS_LOCAL_BASE_PATH.to_le_bytes());
    block[0x1c..0x20].copy_from_slice(&0x24u32.to_le_bytes());
    block[0x20..0x24].copy_from_slice(&((base_units.len() * 2 + 0x24) as u32).to_le_bytes());
    bytes.extend_from_slice(&block);
    for unit in base_units.iter().chain(suffix_units.iter()) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_names_the_path_its_block_records() {
        let bytes = encoded(r"C:\Users\someone\Videos", "");
        assert_eq!(
            target_in(&bytes),
            Some(PathBuf::from(r"C:\Users\someone\Videos"))
        );
    }

    /// The base path and the common suffix are two halves of one path.
    #[test]
    fn the_base_path_and_the_suffix_are_joined() {
        let bytes = encoded(r"C:\Users\someone\", r"Videos");
        assert_eq!(
            target_in(&bytes),
            Some(PathBuf::from(r"C:\Users\someone\Videos"))
        );
    }

    /// A shortcut written before the Unicode fields existed carries the path in
    /// the ANSI one.
    #[test]
    fn an_ansi_only_link_is_read_too() {
        let mut bytes = vec![0u8; HEADER];
        bytes[0..4].copy_from_slice(&(HEADER as u32).to_le_bytes());
        bytes[0x14..0x18].copy_from_slice(&(HAS_LINK_INFO).to_le_bytes());
        let path = b"C:\\Shown\\Videos\0";
        let size = 0x1c + path.len();
        let mut block = vec![0u8; 0x1c];
        block[0..4].copy_from_slice(&(size as u32).to_le_bytes());
        block[4..8].copy_from_slice(&0x1cu32.to_le_bytes());
        block[8..12].copy_from_slice(&HAS_LOCAL_BASE_PATH.to_le_bytes());
        block[0x10..0x14].copy_from_slice(&0x1cu32.to_le_bytes());
        bytes.extend_from_slice(&block);
        bytes.extend_from_slice(path);
        assert_eq!(target_in(&bytes), Some(PathBuf::from(r"C:\Shown\Videos")));
    }

    /// A link whose target is on another machine names no local path, and a file
    /// that is not a shortcut at all has nothing to read.
    #[test]
    fn a_network_link_and_a_plain_file_name_nothing() {
        let mut network = encoded(r"C:\Shown\Videos", "");
        let info = HEADER + 2 + 4;
        network[info + 8..info + 12].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(target_in(&network), None);
        assert_eq!(target_in(b"not a shortcut at all"), None);
        assert_eq!(target_in(&[0u8; 0x50]), None);
    }

    /// Only `.lnk` is a shortcut. The extension is compared without case.
    #[test]
    fn only_the_shortcut_extension_is_read() {
        assert!(is_shortcut(r"C:\Users\someone\Documents\My Videos.lnk"));
        assert!(is_shortcut(r"C:\Users\someone\Documents\My Videos.LNK"));
        assert!(!is_shortcut(r"C:\Users\someone\Documents\My Videos"));
        assert!(!is_shortcut(r"C:\Users\someone\Documents\notes.txt"));
        assert!(!is_shortcut(r".lnk"));
    }

    /// A shortcut whose folder is gone names no folder to open, so the shell
    /// still gets to complain about it in its own words.
    #[test]
    fn a_folder_target_is_only_reported_when_it_is_there() {
        let base = std::env::temp_dir().join(format!("lyre-shortcut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("Shown")).unwrap();
        let shortcut = base.join("shown.lnk");
        std::fs::write(&shortcut, encoded(&base.join("Shown").to_string_lossy(), "")).unwrap();
        assert_eq!(
            folder_target(&shortcut.to_string_lossy()),
            Some(base.join("Shown"))
        );
        // A link left behind by a folder that has since been deleted.
        std::fs::remove_dir_all(base.join("Shown")).unwrap();
        assert_eq!(folder_target(&shortcut.to_string_lossy()), None);
        assert!(target(&shortcut.to_string_lossy()).is_some());
        let _ = std::fs::remove_dir_all(&base);
    }
}
