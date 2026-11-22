//! Cross-platform file operations: paste (copy or move), delete to the trash or
//! permanently, create folder and rename.
//!
//! Delete means trash unless the person asks for permanent (Shift). Trash uses
//! the platform's own recoverable store — the Recycle Bin on Windows and the
//! freedesktop trash specification on Linux/macOS — through native calls, so no
//! extra crate is needed. A paste keeps both items by numbering the newcomer
//! unless the person chooses to overwrite. Operations report progress through a
//! callback that returns `false` to stop.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Cut,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Keep both, numbering the incoming one "name (2)".
    Rename,
    /// Replace what is there.
    Overwrite,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DeleteMode {
    Trash,
    Permanent,
}

/// One progress report; the callback returns `false` to cancel.
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub name: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

#[cfg(windows)]
fn win_file_id(path: &Path, follow_links: bool) -> Option<(u32, u64)> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    struct ByHandleFileInformation {
        file_attributes: u32,
        creation_time: [u32; 2],
        last_access_time: [u32; 2],
        last_write_time: [u32; 2],
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: usize,
        ) -> usize;
        fn GetFileInformationByHandle(handle: usize, info: *mut ByHandleFileInformation) -> i32;
        fn CloseHandle(handle: usize) -> i32;
    }

    const INVALID_HANDLE_VALUE: usize = usize::MAX;
    const FILE_SHARE_ALL: u32 = 0x0001 | 0x0002 | 0x0004;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if !follow_links {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_ALL,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            flags,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut info = unsafe { std::mem::zeroed::<ByHandleFileInformation>() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) != 0 };
    unsafe { CloseHandle(handle) };
    if !ok {
        return None;
    }
    let index = ((info.file_index_high as u64) << 32) | (info.file_index_low as u64);
    Some((info.volume_serial_number, index))
}

fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

fn normalize_lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push(comp);
                }
            }
            _ => out.push(comp),
        }
    }
    out
}

fn is_symlink_path(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Resolve a path for ancestry/equality checks, canonicalizing parents (and
/// non-symlink targets) without dereferencing a leaf symlink to its target.
fn resolve_for_check(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if !is_symlink_path(&abs)
        && let Ok(canon) = fs::canonicalize(&abs)
    {
        return strip_verbatim_prefix(&canon);
    }
    if let (Some(parent), Some(name)) = (abs.parent(), abs.file_name()) {
        let parent_dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if let Ok(canon_parent) = fs::canonicalize(parent_dir) {
            return strip_verbatim_prefix(&canon_parent).join(name);
        }
    }
    strip_verbatim_prefix(&normalize_lexical(&abs))
}

/// Resolve a paste target folder, dereferencing junctions/symlinks so pasting
/// into a junction pointing inside `src` is caught as folder-into-itself.
fn resolve_target_for_check(target: &Path) -> PathBuf {
    let abs = std::path::absolute(target).unwrap_or_else(|_| target.to_path_buf());
    if let Ok(canon) = fs::canonicalize(&abs) {
        return strip_verbatim_prefix(&canon);
    }
    resolve_for_check(&abs)
}

fn component_eq(a: std::path::Component<'_>, b: std::path::Component<'_>) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str().to_string_lossy().to_lowercase()
            == b.as_os_str().to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn norm_eq(a: &Path, b: &Path) -> bool {
    let mut ca = a.components();
    let mut cb = b.components();
    loop {
        match (ca.next(), cb.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if component_eq(x, y) => {}
            _ => return false,
        }
    }
}

fn norm_starts_with(path: &Path, base: &Path) -> bool {
    let mut cp = path.components();
    for b in base.components() {
        let Some(p) = cp.next() else {
            return false;
        };
        if !component_eq(p, b) {
            return false;
        }
    }
    true
}

/// Whether `a` and `b` live in the same directory, differ only in letter case,
/// and name the very same directory entry (without following symlinks).
fn is_case_only_same_entry(a: &Path, b: &Path) -> bool {
    let (Some(pa), Some(pb)) = (a.parent(), b.parent()) else {
        return false;
    };
    let (Some(na), Some(nb)) = (a.file_name(), b.file_name()) else {
        return false;
    };
    let na_str = na.to_string_lossy();
    let nb_str = nb.to_string_lossy();
    if na_str == nb_str || na_str.to_lowercase() != nb_str.to_lowercase() {
        return false;
    }
    let pa_norm = resolve_for_check(if pa.as_os_str().is_empty() { Path::new(".") } else { pa });
    let pb_norm = resolve_for_check(if pb.as_os_str().is_empty() { Path::new(".") } else { pb });
    if !norm_eq(&pa_norm, &pb_norm) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::symlink_metadata(a)
            .ok()
            .zip(fs::symlink_metadata(b).ok())
            .is_some_and(|(x, y)| x.dev() == y.dev() && x.ino() == y.ino())
    }
    #[cfg(windows)]
    {
        win_file_id(a, false)
            .zip(win_file_id(b, false))
            .is_some_and(|(x, y)| x == y)
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Whether two paths name the same underlying file (hard link or resolved
/// symlink), so a copy onto one of them cannot truncate the other.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return fs::metadata(a)
            .ok()
            .zip(fs::metadata(b).ok())
            .is_some_and(|(x, y)| x.dev() == y.dev() && x.ino() == y.ino());
    }
    #[cfg(windows)]
    {
        if win_file_id(a, true)
            .zip(win_file_id(b, true))
            .is_some_and(|(x, y)| x == y)
        {
            return true;
        }
        fs::canonicalize(a)
            .ok()
            .zip(fs::canonicalize(b).ok())
            .is_some_and(|(x, y)| x == y)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (a, b);
        false
    }
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// A destination that does not collide: "name (2).ext", "name (3).ext", …
/// Folders keep dots as part of their name (`v1.2` -> `v1.2 (2)`).
pub fn free_path(dst: &Path) -> PathBuf {
    let is_dir = fs::symlink_metadata(dst).is_ok_and(|m| m.is_dir());
    free_path_for(dst, is_dir)
}

/// Like `free_path`, with the caller stating whether the item is a folder.
pub fn free_path_for(dst: &Path, is_dir: bool) -> PathBuf {
    if fs::symlink_metadata(dst).is_err() {
        return dst.to_path_buf();
    }
    let (stem, ext) = if is_dir {
        (name_of(dst), None)
    } else {
        (
            dst.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            dst.extension().map(|e| format!(".{}", e.to_string_lossy())),
        )
    };
    let parent = dst.parent().map(Path::to_path_buf).unwrap_or_default();
    for n in 2..10_000 {
        let candidate = match &ext {
            Some(ext) => parent.join(format!("{stem} ({n}){ext}")),
            None => parent.join(format!("{stem} ({n})")),
        };
        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    // Every numbered name is taken. The fallback used to be the colliding
    // path itself, so a keep-both paste wrote over the item it was keeping
    // and a delete replaced the copy already in the trash. Keep numbering
    // instead, from a counter no other call in this process reuses, so the
    // first free number is the answer.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(10_000);
    loop {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let candidate = match &ext {
            Some(ext) => parent.join(format!("{stem} ({n}){ext}")),
            None => parent.join(format!("{stem} ({n})")),
        };
        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
}

/// How deep a walk follows folders. A link that points back at a folder above
/// it makes the tree endless, and following it to the end overflows the stack,
/// which takes the whole application down rather than failing the operation.
const MAX_DEPTH: usize = 512;

/// Recursive size in bytes, for the progress bar's total.
fn item_size(path: &Path) -> u64 {
    size_of(path, 0)
}

fn size_of(path: &Path, depth: usize) -> u64 {
    if depth > MAX_DEPTH {
        return 0;
    }
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    // A link is not the folder it names: walking through it is how a tree with
    // one link back upstairs becomes endless.
    if meta.file_type().is_symlink() {
        return 0;
    }
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .fold(0u64, |sum, e| sum.saturating_add(size_of(&e.path(), depth + 1)))
        })
        .unwrap_or(0)
}

fn apply_times(src_meta: &fs::Metadata, dst: &Path, is_dir: bool) {
    let mut times = fs::FileTimes::new();
    let mut any = false;
    if let Ok(accessed) = src_meta.accessed() {
        times = times.set_accessed(accessed);
        any = true;
    }
    if let Ok(modified) = src_meta.modified() {
        times = times.set_modified(modified);
        any = true;
    }
    #[cfg(windows)]
    if let Ok(created) = src_meta.created() {
        use std::os::windows::fs::FileTimesExt;
        times = times.set_created(created);
        any = true;
    }
    if !any {
        return;
    }
    #[cfg(windows)]
    let handle = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        const FILE_SHARE_ALL: u32 = 0x0001 | 0x0002 | 0x0004;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .share_mode(FILE_SHARE_ALL)
            .custom_flags(if is_dir { FILE_FLAG_BACKUP_SEMANTICS } else { 0 })
            .open(dst)
    };
    #[cfg(not(windows))]
    let handle = {
        let _ = is_dir;
        fs::File::open(dst)
    };
    if let Ok(file) = handle {
        let _ = file.set_times(times);
    }
}

fn copy_item(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64),
) -> Result<(), String> {
    copy_at(src, dst, 0, cancel, report)
}

fn copy_at(
    src: &Path,
    dst: &Path,
    depth: usize,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64),
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    if depth > MAX_DEPTH {
        return Err(format!("{}: the folders nest too deeply to copy", src.display()));
    }
    // A link chosen as the source is copied as a link, like the ones inside a
    // folder. Following it here walked into its target, which could include
    // the destination being built and recurse until the depth limit.
    if fs::symlink_metadata(src)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return copy_link(src, dst);
    }
    let meta = fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if meta.is_dir() {
        fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        let entries = fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
        for item in entries {
            let entry = item.map_err(|e| format!("{}: {e}", src.display()))?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            // A link is copied as what it is rather than walked through: a
            // junction back up the tree would otherwise copy forever.
            if entry
                .file_type()
                .map(|kind| kind.is_symlink())
                .unwrap_or(false)
            {
                copy_link(&from, &to)?;
                continue;
            }
            copy_at(&from, &to, depth + 1, cancel, report)?;
        }
        apply_times(&meta, dst, true);
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }
        copy_file_chunked(src, dst, &meta, cancel, report)?;
        apply_times(&meta, dst, false);
        Ok(())
    }
}

/// Copy one regular file in chunks so large transfers report progress in real
/// time and stop promptly when cancelled.
fn copy_file_chunked(
    src: &Path,
    dst: &Path,
    meta: &fs::Metadata,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64),
) -> Result<(), String> {
    use std::io::{Read, Write};
    const CHUNK: usize = 256 * 1024;
    let mut reader = fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut writer = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(dst)
        .map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(writer);
            let _ = fs::remove_file(dst);
            return Err("Cancelled".into());
        }
        let n = reader.read(&mut buf).map_err(|e| format!("{}: {e}", src.display()))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .map_err(|e| format!("{}: {e}", dst.display()))?;
        report(n as u64);
    }
    let _ = fs::set_permissions(dst, meta.permissions());
    Ok(())
}

/// Make a link at `dst` that points where `src` points.
///
/// A link is never walked through while copying: a junction that points back up
/// the tree would copy until the stack ran out, which takes the whole window
/// down rather than failing one operation.
#[cfg(windows)]
fn copy_link(src: &Path, dst: &Path) -> Result<(), String> {
    use std::os::windows::fs::{MetadataExt, symlink_dir, symlink_file};
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let target = fs::read_link(src).map_err(|e| format!("{}: {e}", src.display()))?;
    // A junction target is resolved relative to the link's folder, not the
    // process working directory, before deciding whether it is a directory.
    let absolute = if target.is_absolute() {
        target.clone()
    } else {
        src.parent().unwrap_or_else(|| Path::new(".")).join(&target)
    };
    let is_dir_link = fs::symlink_metadata(src)
        .map(|m| m.file_attributes() & 0x10 != 0)
        .unwrap_or(false)
        || absolute.is_dir();
    if let Some(parent) = dst.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if is_dir_link {
        // No shell: paths go straight to mklink, so spaces, `&` and `|` in a
        // name cannot break the command or run a second one. Each argument is
        // quoted only by CreateProcess's exact-argument rule.
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(dst)
            .arg(&absolute)
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if made.as_ref().is_ok_and(|out| out.status.success()) {
            return Ok(());
        }
        symlink_dir(&target, dst)
            .map_err(|e| format!("{}: could not recreate directory link: {e}", src.display()))
    } else {
        // A file symlink can be recreated with the Windows API without
        // elevation when Developer Mode is enabled; otherwise fail rather than
        // claiming success so a cross-volume Cut never deletes the original.
        symlink_file(&target, dst)
            .map_err(|e| format!("{}: could not recreate link: {e}", src.display()))
    }
}

#[cfg(unix)]
fn copy_link(src: &Path, dst: &Path) -> Result<(), String> {
    let target = fs::read_link(src).map_err(|e| format!("{}: {e}", src.display()))?;
    std::os::unix::fs::symlink(target, dst).map_err(|e| format!("{}: {e}", dst.display()))
}

#[cfg(not(any(unix, windows)))]
fn copy_link(_src: &Path, _dst: &Path) -> Result<(), String> {
    Err("Copying links is not supported on this system.".into())
}

/// What a delete left behind, enough to put the item back: where it came from,
/// when it went, and, where the trash is a folder this wrote to, where it is now
/// and the note that names it.
#[derive(Clone)]
pub struct Trashed {
    pub original: PathBuf,
    pub when: std::time::SystemTime,
    /// Where the item was put, and the note that names where it came from, on
    /// systems whose trash is a folder this wrote to. On Windows the bin is the
    /// shell's, and its own records answer both.
    #[cfg(unix)]
    pub trashed: Option<PathBuf>,
    #[cfg(unix)]
    pub info: Option<PathBuf>,
}

/// A finished delete: how many went, and what can be brought back.
pub struct Deleted {
    pub count: usize,
    pub trashed: Vec<Trashed>,
    /// Why the delete stopped, when it did. The items counted and trashed
    /// above still went, and their records are kept, so a delete that
    /// stopped half way can still be taken back for the part that landed.
    pub error: Option<String>,
}

/// Put deleted items back where they came from, newest first, so a folder is
/// restored before anything that was inside it.
#[allow(dead_code)]
pub fn restore(items: &[Trashed]) -> Result<(), String> {
    for item in items.iter().rev() {
        restore_one(item)?;
    }
    Ok(())
}

/// Restore as many as can be brought back, reporting the first error after
/// trying all of them. A single bad record must not strand every other item
/// in the trash with no way back.
pub fn restore_each(items: &[Trashed]) -> Result<Vec<Trashed>, (Vec<Trashed>, String)> {
    let mut left = Vec::new();
    let mut error = None;
    for item in items.iter().rev() {
        match restore_one(item) {
            Ok(()) => {}
            Err(e) => {
                if error.is_none() {
                    error = Some(e);
                }
                left.push(item.clone());
            }
        }
    }
    left.reverse();
    match error {
        None => Ok(Vec::new()),
        Some(e) => Err((left, e)),
    }
}

/// Bring one item back out of the trash.
#[cfg(windows)]
fn restore_one(item: &Trashed) -> Result<(), String> {
    recycle::restore(&item.original, item.when)
}

/// Bring one item back out of the freedesktop trash, whose own note names where
/// it came from.
#[cfg(unix)]
fn restore_one(item: &Trashed) -> Result<(), String> {
    let (Some(trashed), Some(info)) = (item.trashed.as_ref(), item.info.as_ref()) else {
        return Err("That deletion cannot be taken back.".into());
    };
    if fs::symlink_metadata(&item.original).is_ok() {
        return Err(format!(
            "{} is there again already.",
            item.original.display()
        ));
    }
    if let Some(parent) = item.original.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // The trash lives on the home volume, so an item deleted from another
    // volume (a stick, a second drive, a tmpfs) cannot be renamed back: the
    // move falls back to copying, the same one that took it there.
    let cancel = AtomicBool::new(false);
    move_item(trashed, &item.original, &cancel, &mut |_| {})
        .map_err(|e| format!("Could not take it out of the trash: {e}"))?;
    let _ = fs::remove_file(info);
    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn restore_one(_item: &Trashed) -> Result<(), String> {
    Err("Taking a deletion back is not available on this system.".into())
}

fn remove_item(path: &Path) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    remove_item_progress(path, &cancel, &mut |_| {})
}

fn remove_item_progress(
    path: &Path,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64),
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // On Windows a junction or directory symlink is a reparse point with
    // FILE_ATTRIBUTE_DIRECTORY (0x10) set even when broken; DeleteFile cannot
    // remove a directory reparse point, so remove_dir must be used.
    #[cfg(windows)]
    let folder_link = {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() && (meta.file_attributes() & 0x10 != 0)
    };
    #[cfg(not(windows))]
    let folder_link = false;
    if folder_link {
        return fs::remove_dir(path).map_err(|e| format!("{}: {e}", path.display()));
    }
    if meta.is_dir() {
        let entries = fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
        for item in entries {
            let entry = item.map_err(|e| format!("{}: {e}", path.display()))?;
            remove_item_progress(&entry.path(), cancel, report)?;
        }
        fs::remove_dir(path).map_err(|e| format!("{}: {e}", path.display()))
    } else {
        let size = if meta.file_type().is_symlink() { 0 } else { meta.len() };
        fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
        report(size);
        Ok(())
    }
}

/// One Recycle Bin record: where the item came from and when it was deleted.
///
/// The file holds an 8-byte version, the size, the deletion time as a FILETIME,
/// then the path: a fixed 260 UTF-16 units in the older format, or a length and
/// then the path in the one Vista and later write.
#[cfg(windows)]
fn parse_i(data: &[u8]) -> Option<(PathBuf, u64, u64)> {
    if data.len() < 28 {
        return None;
    }
    let version = u64::from_le_bytes(data[0..8].try_into().ok()?);
    // The size is stored in the record. Reading it here avoids a metadata call
    // per item, which made opening a full bin freeze the window.
    let size = u64::from_le_bytes(data[8..16].try_into().ok()?);
    let deleted = u64::from_le_bytes(data[16..24].try_into().ok()?);
    // The path is the rest of the record, ended by a NUL: the length before it
    // is counted in characters by some writers and in bytes by others, so the
    // terminator is the one thing that means the same thing either way.
    let raw: &[u8] = match version {
        1 => &data[24..],
        2 => &data[28..],
        _ => return None,
    };
    if raw.len() < 4 {
        return None;
    }
    let units: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    if units.is_empty() {
        return None;
    }
    Some((PathBuf::from(String::from_utf16_lossy(&units)), deleted, size))
}

/// Take an item away, for a step being undone. A folder goes with everything in
/// it.
pub fn remove(path: &Path) -> Result<(), String> {
    remove_item(path)
}

fn move_item(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64),
) -> Result<(), String> {
    // A rename is instant on the same volume; across volumes fall back to copy.
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    // A copy fallback can succeed while the source cannot be removed (a locked
    // file, a deny-delete ACL). Leaving both behind silently duplicated the
    // item, and because the step returned Err nothing reached the undo stack.
    // Roll the copy back so a failed move leaves the original state intact.
    let dst_existed = fs::symlink_metadata(dst).is_ok();
    if let Err(e) = copy_item(src, dst, cancel, report) {
        if !dst_existed {
            let _ = remove_item(dst);
        }
        return Err(e);
    }
    if let Err(e) = remove_item(src) {
        if !dst_existed {
            if let Err(rollback) = remove_item(dst) {
                return Err(format!(
                    "Could not remove the original: {e}; the copy at {} could not be removed either: {rollback}",
                    dst.display()
                ));
            }
        }
        return Err(format!("Could not remove the original: {e}"));
    }
    Ok(())
}

/// Move one item to a full destination path, for the steps of undo and
/// redo: a rename where it can, a copy across volumes where it cannot.
/// The destination must be free. A bare rename writes over whatever stands
/// there on Unix without a word, so an undo could land on a file that took
/// the place since; what stands there now is never written over.
pub fn relocate(src: &Path, dst: &Path) -> Result<(), String> {
    if fs::symlink_metadata(dst).is_ok() && !is_case_only_same_entry(src, dst) {
        return Err(format!("{} is there already.", dst.display()));
    }
    let cancel = AtomicBool::new(false);
    move_item(src, dst, &cancel, &mut |_| {})
}

/// Copy or move `sources` into the `target` directory, reporting progress.
///
/// Answers where each item landed, source first, and why it stopped, so a paste
/// that broke half way is still a paste that moved things and can be taken back.
pub struct Pasted {
    pub landed: Vec<(PathBuf, PathBuf)>,
    pub error: Option<String>,
    /// Existing items a Replace had to set aside. Both the old bytes and the
    /// bytes that replaced them are kept so undo and redo can swap them back
    /// without losing anything the folder already had.
    pub replaced: Vec<Replaced>,
}

#[derive(Clone)]
pub struct Replaced {
    pub source: PathBuf,
    pub old: PathBuf,
    pub new: PathBuf,
    pub target: PathBuf,
}

static REPLACE_NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A private copy of an item about to be replaced, so undo can put it back.
fn replace_backup(src: &Path) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("lyre-replace-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| format!("Could not keep a copy for undo: {e}"))?;
    let n = REPLACE_NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let leaf = name_of(src);
    let dst = dir.join(format!("{n:016x}-{leaf}"));
    let cancel = AtomicBool::new(false);
    copy_item(src, &dst, &cancel, &mut |_| {})?;
    Ok(dst)
}

static SWAP_NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn sibling_stage_path(target: &Path, tag: &str) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("{}: missing parent directory", target.display()))?;
    let pid = std::process::id();
    loop {
        let n = SWAP_NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let candidate = parent.join(format!(".lyre-swap-{tag}-{pid}-{n}"));
        if fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
}

/// Replace `target` with a copy of `backup` without ever deleting `target`
/// before `backup` is verified and staged alongside `target`.
fn swap_with_backup(backup: &Path, target: &Path) -> Result<(), String> {
    if backup.as_os_str().is_empty() || fs::symlink_metadata(backup).is_err() {
        return Err("The backup copy for undo/redo is no longer available.".into());
    }
    let staged = sibling_stage_path(target, "stage")?;
    if let Err(e) = copy_item(backup, &staged, &AtomicBool::new(false), &mut |_| {}) {
        let _ = remove_item(&staged);
        return Err(e);
    }
    if fs::symlink_metadata(target).is_ok() {
        let aside = sibling_stage_path(target, "aside")?;
        if let Err(e) = fs::rename(target, &aside) {
            let _ = remove_item(&staged);
            return Err(format!("{}: {e}", target.display()));
        }
        if let Err(e) = fs::rename(&staged, target) {
            let _ = fs::rename(&aside, target);
            let _ = remove_item(&staged);
            return Err(format!("{}: {e}", target.display()));
        }
        let _ = remove_item(&aside);
    } else if let Err(e) = fs::rename(&staged, target) {
        let _ = remove_item(&staged);
        return Err(format!("{}: {e}", target.display()));
    }
    Ok(())
}

/// Undo a Replace: restore the old bytes to `item.target` via a staged swap.
pub fn swap_back(item: &Replaced) -> Result<(), String> {
    swap_with_backup(&item.old, &item.target)
}

/// Redo a Replace: put the new bytes back, keeping the restored old ones for a
/// further undo.
pub fn swap_forward(item: &Replaced) -> Result<(), String> {
    if item.new.as_os_str().is_empty() {
        return Err("The redo backup copy is not available.".into());
    }
    swap_with_backup(&item.new, &item.target)
}

/// Sweep backups left behind by an earlier run. Only the current process's
/// own directory is ever touched: a second instance starting up must not wipe
/// live undo backups that the first instance still needs.
pub fn sweep_stale_backups() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("lyre-replace-{}", std::process::id()));
        let _ = remove_item(&dir);
    });
}

impl Pasted {
    fn refused(reason: &str) -> Self {
        Pasted { landed: Vec::new(), error: Some(reason.into()), replaced: Vec::new() }
    }
}

/// One item of a paste: where it landed, or why it did not go.
fn paste_one(
    mode: Mode,
    conflict: Conflict,
    src: &Path,
    target: &Path,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64),
    replaced: &mut Vec<Replaced>,
) -> Result<PathBuf, String> {
    let name = name_of(src);
    let mut dst = target.join(&name);
    let src_is_link = is_symlink_path(src);
    let src_is_dir = !src_is_link && fs::metadata(src).map(|m| m.is_dir()).unwrap_or(false);
    let src_norm = resolve_for_check(src);
    let target_norm = resolve_target_for_check(target);
    let initial_dst_norm = target_norm.join(&name);
    // Never paste a folder into itself.
    if src_is_dir && !norm_eq(&initial_dst_norm, &src_norm) && norm_starts_with(&initial_dst_norm, &src_norm) {
        return Err("Cannot paste a folder into itself.".into());
    }
    if let Ok(dst_meta) = fs::symlink_metadata(&dst) {
        match conflict {
            Conflict::Rename => dst = free_path_for(&dst, src_is_dir),
            Conflict::Overwrite => {
                // Keep the old bytes before anything is written over. The copy
                // is separate from the destination, so a folder merge can still
                // happen below and undo has the complete earlier state to put
                // back. On failure the caller restores it.
                let old = replace_backup(&dst)?;
                replaced.push(Replaced { source: src.to_path_buf(), old, new: PathBuf::new(), target: dst.clone() });
                // A file and a folder cannot stand in for one another in place,
                // and a symlink cannot be merged into. Everything else merges,
                // so what is only in the destination survives and only the names
                // in both are written over.
                let dst_is_link = dst_meta.file_type().is_symlink();
                let dst_is_dir = !dst_is_link && dst_meta.is_dir();
                if src_is_link || dst_is_link || dst_is_dir != src_is_dir {
                    remove_item(&dst)?;
                }
            }
        }
    }
    // Settled after the conflict: a copy read from the file it is writing leaves
    // it empty. Compare identities, not just path text: a hard link (or a link
    // resolving to the same file) is the same item under a different name and
    // would be truncated by fs::copy.
    let dst_norm = target_norm.join(name_of(&dst));
    if norm_eq(&src_norm, &dst_norm) || same_file(src, &dst) {
        return Err("That item is already here.".into());
    }
    match mode {
        Mode::Copy => copy_item(src, &dst, cancel, report)?,
        Mode::Cut => move_item(src, &dst, cancel, report)?,
    }
    Ok(dst)
}

pub fn paste<F: FnMut(Progress) -> bool>(
    mode: Mode,
    conflict: Conflict,
    sources: &[PathBuf],
    target: &Path,
    cancel: &AtomicBool,
    mut emit: F,
) -> Pasted {
    // A selection that names the destination itself is refused before anything
    // moves: stopping half way leaves the person to work out which half went.
    // A folder and something inside it are the same operation twice: keeping
    // both made the second source vanish after the first moved, reporting a
    // false failure and recording an undo that could not cover the child.
    // Order-independent check: regardless of selection order, any real ancestor
    // directory anywhere in `sources` covers its descendants.
    let norm_sources: Vec<(PathBuf, bool)> = sources
        .iter()
        .map(|src| (resolve_for_check(src), is_symlink_path(src)))
        .collect();
    let covered: Vec<bool> = norm_sources
        .iter()
        .enumerate()
        .map(|(i, (src_norm, _))| {
            norm_sources.iter().enumerate().any(|(j, (other_norm, other_is_link))| {
                if j == i || *other_is_link {
                    return false;
                }
                if norm_eq(src_norm, other_norm) {
                    j < i
                } else {
                    norm_starts_with(src_norm, other_norm)
                }
            })
        })
        .collect();
    let target_norm = resolve_target_for_check(target);
    for (i, src) in sources.iter().enumerate() {
        if covered[i] {
            continue;
        }
        let (src_norm, src_is_link) = &norm_sources[i];
        if *src_is_link || !src.is_dir() {
            continue;
        }
        let dst_norm = target_norm.join(name_of(src));
        // Strictly inside, where renaming the newcomer cannot rescue it.
        if !norm_eq(&dst_norm, src_norm) && norm_starts_with(&dst_norm, src_norm) {
            return Pasted::refused("Cannot paste a folder into itself.");
        }
        // A folder replaced by itself would be written over itself.
        if norm_eq(&dst_norm, src_norm) && conflict == Conflict::Overwrite {
            return Pasted::refused(if sources.len() == 1 {
                "That item is already here."
            } else {
                "Those items are already here."
            });
        }
    }
    // Only sources that are actually attempted count toward the item total;
    // covered descendants are skipped below and must not leave the bar short.
    let total = covered.iter().filter(|&&skipped| !skipped).count();
    let mut shown = 0usize;
    let total_bytes: u64 = sources
        .iter()
        .enumerate()
        .filter(|&(i, _)| !covered[i])
        .fold(0u64, |sum, (_, p)| sum.saturating_add(item_size(p)));
    let mut done_bytes = 0u64;
    let mut completed = 0u64;
    let mut landed: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut replaced: Vec<Replaced> = Vec::new();
    let mut error = None;
    // Bytes land inside the recursive copy, so a folder of ten thousand files
    // climbs smoothly instead of jumping from 0 to 100 at the end. Throttled
    // by time and by a sixteenth of the total so the channel never floods.
    let mut last_emit = std::time::Instant::now();
    let mut last_bytes = 0u64;
    let step = (total_bytes / 16).max(1);
    for (src, &is_covered) in sources.iter().zip(covered.iter()) {
        if is_covered {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = name_of(src);
        if !emit(Progress { done: shown, total, name: name.clone(), done_bytes, total_bytes }) {
            break;
        }
        // Measured before the item goes: after a cut the source is gone and
        // reads as zero, so a move's progress never reached the total.
        let size_before = item_size(src);
        let outcome = {
            let mut report = |add: u64| {
                done_bytes += add;
                if last_emit.elapsed() >= std::time::Duration::from_millis(60)
                    || done_bytes.saturating_sub(last_bytes) >= step
                {
                    last_emit = std::time::Instant::now();
                    last_bytes = done_bytes;
                    emit(Progress { done: shown, total, name: name.clone(), done_bytes, total_bytes });
                }
            };
            paste_one(mode, conflict, src, target, cancel, &mut report, &mut replaced)
        };
        match outcome {
            Ok(dst) => {
                shown += 1;
                // A rename moves bytes without copying them; credit them at the end.
                completed += size_before;
                done_bytes = done_bytes.max(completed);
                landed.push((src.clone(), dst.clone()));
                for r in replaced.iter_mut().rev() {
                    if r.source == *src && r.new.as_os_str().is_empty() {
                        match replace_backup(&dst) {
                            Ok(new) => r.new = new,
                            Err(e) => {
                                error = Some(e);
                                break;
                            }
                        }
                    }
                }
                if error.is_some() {
                    break;
                }
                emit(Progress { done: shown, total, name, done_bytes, total_bytes });
            }
            Err(e) => {
                // Anything set aside for this item goes back rather than
                // stranding the old contents in a backup folder. A custom
                // destination from Conflict::Rename is not in `name`, so match
                // every entry belonging to the source that just failed.
                let failing: Vec<usize> = (0..replaced.len())
                    .rev()
                    .filter(|&k| replaced[k].source == *src)
                    .collect();
                for k in failing {
                    let r = replaced.remove(k);
                    // Put the earlier contents back where they were. Only drop
                    // the backup once that worked — a locked child in the
                    // destination must leave the sole good copy in place.
                    let restore = swap_with_backup(&r.old, &r.target);
                    match restore {
                        Ok(()) => {
                            let _ = remove_item(&r.old);
                        }
                        Err(back) => {
                            // Keep the backup referenced for a later retry.
                            replaced.push(r);
                            error = Some(format!("{e}; the replaced item could not be restored: {back}"));
                            break;
                        }
                    }
                }
                if error.is_none() {
                    error = Some(e);
                }
                break;
            }
        }
    }
    Pasted { landed, error, replaced }
}

/// Copy `pairs` again, source to destination, for a redo of a copy. A
/// destination that has been taken since is refused rather than written
/// over: the redo recreates what the copy made, it does not replace what
/// is there now.
#[allow(dead_code)]
pub fn copy_again(pairs: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    for (from, to) in pairs {
        if fs::symlink_metadata(to).is_ok() {
            return Err(format!("{} is there already.", to.display()));
        }
        copy_item(from, to, &cancel, &mut |_| {})?;
    }
    Ok(())
}

/// Redo a multi-item copy one item at a time, leaving only what failed for
/// the next press. The all-or-nothing version retried from the start and was
/// blocked forever by the copies that had already landed.
type CopiedPairs = Vec<(PathBuf, PathBuf)>;

pub fn copy_again_each(pairs: &[(PathBuf, PathBuf)]) -> Result<CopiedPairs, (CopiedPairs, String)> {
    let cancel = AtomicBool::new(false);
    let mut left = Vec::new();
    let mut error = None;
    for (from, to) in pairs {
        let result = if fs::symlink_metadata(to).is_ok() {
            Err(format!("{} is there already.", to.display()))
        } else {
            copy_item(from, to, &cancel, &mut |_| {})
        };
        if let Err(e) = result {
            if error.is_none() {
                error = Some(e);
            }
            left.push((from.clone(), to.clone()));
        }
    }
    match error {
        None => Ok(Vec::new()),
        Some(e) => Err((left, e)),
    }
}

/// Delete `paths` to the trash or permanently, reporting progress. Like
/// paste, what a delete did is reported even when it stops: the failure
/// rides in the outcome instead of discarding the records of the items
/// that already went, which used to leave them in the trash with no way
/// back through undo.
pub fn delete<F: FnMut(Progress) -> bool>(
    mode: DeleteMode,
    paths: &[PathBuf],
    cancel: &AtomicBool,
    mut emit: F,
) -> Deleted {
    let total = paths.len();
    let total_bytes: u64 = paths.iter().fold(0u64, |sum, p| sum.saturating_add(item_size(p)));
    let mut done_bytes = 0u64;
    let mut deleted = 0usize;
    let mut trashed: Vec<Trashed> = Vec::new();
    let mut error = None;
    let mut last_emit = std::time::Instant::now();
    let mut last_bytes = 0u64;
    let step = (total_bytes / 16).max(1);
    for (i, path) in paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = name_of(path);
        if !emit(Progress { done: i, total, name: name.clone(), done_bytes, total_bytes }) {
            break;
        }
        let size = item_size(path);
        let done_before = done_bytes;
        let failed = match mode {
            DeleteMode::Permanent => {
                let mut completed = done_bytes;
                let mut report = |add: u64| {
                    completed += add;
                    if last_emit.elapsed() >= std::time::Duration::from_millis(60)
                        || completed.saturating_sub(last_bytes) >= step
                    {
                        last_emit = std::time::Instant::now();
                        last_bytes = completed;
                        emit(Progress { done: i, total, name: name.clone(), done_bytes: completed, total_bytes });
                    }
                };
                let res = remove_item_progress(path, cancel, &mut report).err();
                done_bytes = completed.max(done_bytes);
                res
            }
            DeleteMode::Trash => match trash_one(path) {
                Ok(item) => {
                    trashed.push(item);
                    done_bytes += size;
                    None
                }
                Err(e) => Some(e),
            },
        };
        if let Some(e) = failed {
            error = Some(e);
            break;
        }
        if mode == DeleteMode::Permanent {
            done_bytes = done_bytes.max(done_before + size);
        }
        deleted += 1;
        emit(Progress { done: i + 1, total, name, done_bytes, total_bytes });
    }
    Deleted { count: deleted, trashed, error }
}

/// A name typed for a rename or a new folder names one item inside one
/// folder: it may not be empty, be a step to somewhere else, or carry a
/// separator. Without the check the name is joined onto a parent, so a
/// name like `../notes.txt` moves the item out of its folder entirely,
/// and on Windows an absolute name replaces the parent in the join.
fn check_new_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err("That name cannot be used here.".into());
    }
    // Windows device names are reserved even with an extension; creating one
    // fails at the OS with a raw error. Refuse them here with a clear reason.
    if cfg!(windows) {
        let stem = name.split('.').next().unwrap_or(name);
        let stem = stem.trim_end_matches([' ', '.']);
        let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
        if reserved.iter().any(|r| stem.eq_ignore_ascii_case(r)) {
            return Err("That name is reserved by Windows.".into());
        }
    }
    Ok(())
}

pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    check_new_name(name)?;
    let path = free_path_for(&parent.join(name), true);
    fs::create_dir(&path).map_err(|e| format!("Could not create the folder: {e}"))?;
    Ok(path)
}

pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    check_new_name(new_name)?;
    let parent = path.parent().ok_or("That item cannot be renamed.")?;
    let target = parent.join(new_name);
    if target == path {
        return Ok(path.to_path_buf());
    }
    if fs::symlink_metadata(&target).is_ok() && !is_case_only_same_entry(path, &target) {
        return Err("An item with that name already exists.".into());
    }
    fs::rename(path, &target).map_err(|e| format!("Could not rename: {e}"))?;
    Ok(target)
}

// ---------------------------------------------------------------------------
// Trash: the Recycle Bin on Windows, freedesktop trash on Linux/macOS.
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn trash_one(path: &Path) -> Result<Trashed, String> {
    let when = std::time::SystemTime::now();
    recycle::send(&[path])?;
    Ok(Trashed { original: path.to_path_buf(), when })
}

#[cfg(windows)]
mod recycle {
    use std::fs;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }

    fn normalize_for_shell(path: &Path) -> Result<PathBuf, String> {
        let abs = std::path::absolute(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let text = abs.to_string_lossy().replace('/', "\\");
        let cleaned = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else if let Some(rest) = text.strip_prefix(r"\\?\") {
            rest.to_string()
        } else {
            text
        };
        Ok(PathBuf::from(cleaned))
    }

    fn ensure_recyclable_target(path: &Path) -> Result<(), String> {
        let text = path.to_string_lossy();
        if text.starts_with(r"\\") {
            return Err("Items on a network share cannot be moved to the Recycle Bin.".into());
        }
        let mut chars = text.chars();
        if let (Some(drive), Some(':')) = (chars.next(), chars.next())
            && drive.is_ascii_alphabetic()
        {
            let root: Vec<u16> = format!("{drive}:\\")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let dt = unsafe { GetDriveTypeW(root.as_ptr()) };
            const DRIVE_REMOTE: u32 = 4;
            const DRIVE_CDROM: u32 = 5;
            const DRIVE_RAMDISK: u32 = 6;
            if matches!(dt, DRIVE_REMOTE | DRIVE_CDROM | DRIVE_RAMDISK) {
                return Err(
                    "Items on a network or read-only drive cannot be moved to the Recycle Bin."
                        .into(),
                );
            }
        }
        Ok(())
    }

    fn send_with_ifo(abs_paths: &[PathBuf]) -> Result<(), String> {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        };
        use windows::Win32::UI::Shell::{
            FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOFX_RECYCLEONDELETE,
            FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName,
        };

        struct ComGuard(bool);
        impl Drop for ComGuard {
            fn drop(&mut self) {
                if self.0 {
                    unsafe { CoUninitialize() };
                }
            }
        }

        let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if init == RPC_E_CHANGED_MODE {
            return std::thread::scope(|scope| {
                scope
                    .spawn(|| send_with_ifo(abs_paths))
                    .join()
                    .unwrap_or_else(|_| Err("Recycle Bin worker panicked.".into()))
            });
        }
        if init.is_err() {
            return Err(format!("Could not initialize COM for Recycle Bin ({}).", init.0));
        }
        let _guard = ComGuard(true);

        let op: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| format!("Could not open Windows file operation: {e}"))?;
        let flags =
            FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOFX_RECYCLEONDELETE;
        unsafe { op.SetOperationFlags(flags) }
            .map_err(|e| format!("Could not configure Recycle Bin operation: {e}"))?;

        for path in abs_paths {
            let wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let item: IShellItem =
                unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
                    .map_err(|e| format!("{}: {e}", path.display()))?;
            unsafe { op.DeleteItem(&item, None) }
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }

        unsafe { op.PerformOperations() }.map_err(|e| {
            format!("Windows could not move the item to the Recycle Bin without permanently deleting it: {e}")
        })?;
        let aborted = unsafe { op.GetAnyOperationsAborted() }
            .map(|b| b.as_bool())
            .unwrap_or(true);
        if aborted {
            return Err("Windows cancelled moving the item to the Recycle Bin.".into());
        }
        Ok(())
    }

    pub fn send(paths: &[&Path]) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut abs_paths = Vec::with_capacity(paths.len());
        for path in paths {
            let abs = normalize_for_shell(path)?;
            ensure_recyclable_target(&abs)?;
            if fs::symlink_metadata(&abs).is_err() {
                return Err(format!("{}: does not exist", path.display()));
            }
            abs_paths.push(abs);
        }
        send_with_ifo(&abs_paths)
    }

    /// Put one item back where it came from, by finding its record in the bin.
    ///
    /// The bin keeps two files per deletion: `$I…`, which names the original
    /// path and when it went, and `$R…`, which holds the item. Restoring is what
    /// Explorer does when it takes one out: move the content back and drop the
    /// record.
    pub fn restore(original: &Path, when: std::time::SystemTime) -> Result<(), String> {
        let want = original.to_string_lossy().to_lowercase();
        // Only the bin of the drive it came from can hold it: a machine with
        // four drives would otherwise be searched four times over.
        // Only a rooted drive path names a bin drive. A UNC path starts with
        // `\\`, and treating its prefix as a drive letter looked for a folder
        // called `\\:\$Recycle.Bin`, so network deletions could never be undone.
        let drive = original
            .components()
            .next()
            .and_then(|part| part.as_os_str().to_str())
            .and_then(|s| s.strip_suffix(':'))
            .filter(|s| s.len() == 1 && s.chars().all(|c| c.is_ascii_alphabetic()))
            .map(str::to_string);
        let mut best: Option<(i64, std::path::PathBuf, std::path::PathBuf)> = None;
        for record in records(drive.as_deref()) {
            if record.original.to_string_lossy().to_lowercase() != want {
                continue;
            }
            let Some(deleted) = from_filetime(record.deleted) else { continue };
            let delta = match (deleted.duration_since(when), when.duration_since(deleted)) {
                (Ok(later), _) => later.as_secs() as i64,
                (Err(_), Ok(earlier)) => -(earlier.as_secs() as i64),
                _ => continue,
            };
            // The same name can be deleted twice; the deletion moment says which.
            if delta.abs() > 300 {
                continue;
            }
            if best.as_ref().is_none_or(|(seen, ..)| delta.abs() < seen.abs()) {
                best = Some((delta, record.info, record.content));
            }
        }
        let Some((_, info, content)) = best else {
            return Err(format!(
                "{} is not in the Recycle Bin any more.",
                original.display()
            ));
        };
        if fs::symlink_metadata(original).is_ok() {
            return Err(format!("{} is there again already.", original.display()));
        }
        if let Some(parent) = original.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::rename(&content, original)
            .map_err(|e| format!("Could not take it out of the Recycle Bin: {e}"))?;
        let _ = fs::remove_file(&info);
        Ok(())
    }

    /// One `$I` record.
    pub(super) struct Record {
        pub info: std::path::PathBuf,
        pub content: std::path::PathBuf,
        pub original: std::path::PathBuf,
        pub deleted: u64,
        pub size: u64,
        pub is_dir: bool,
    }

    /// Every record in every bin this account can read. The small `$I` files
    /// are parsed on a few worker threads: a full bin has thousands of them and
    /// doing it on one thread was the freeze when the place was opened.
    pub(super) fn records(drive: Option<&str>) -> Vec<Record> {
        use std::os::windows::fs::MetadataExt;
        use std::path::PathBuf;
        let letters: Vec<char> = match drive.and_then(|d| d.chars().next()) {
            Some(letter) => vec![letter],
            // No drive means the whole bin: every volume this account can read.
            None => ('A'..='Z').collect(),
        };
        let mut files = Vec::new();
        for letter in letters {
            let bin = PathBuf::from(format!("{}:\\$Recycle.Bin", letter));
            let Ok(users) = fs::read_dir(&bin) else { continue };
            for user in users.flatten() {
                let Ok(items) = fs::read_dir(user.path()) else { continue };
                for item in items.flatten() {
                    if item.file_name().to_string_lossy().starts_with("$I") {
                        files.push(item.path());
                    }
                }
            }
        }
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(8);
        let per = (files.len() / workers).max(1);
        let mut out = Vec::new();
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for chunk in files.chunks(per) {
                handles.push(scope.spawn(move || {
                    let mut found = Vec::new();
                    for path in chunk {
                        let Ok(data) = fs::read(path) else { continue };
                        let Some((original, deleted, size)) = super::parse_i(&data) else { continue };
                        let suffix = path.file_name().unwrap_or_default().to_string_lossy();
                        let content = path.with_file_name(format!("$R{}", &suffix[2..]));
                        let is_dir = fs::symlink_metadata(&content).is_ok_and(|m| {
                            m.is_dir()
                                || (m.file_type().is_symlink() && (m.file_attributes() & 0x10 != 0))
                        });
                        found.push(Record {
                            info: path.clone(),
                            content,
                            original,
                            deleted,
                            size,
                            is_dir,
                        });
                    }
                    found
                }));
            }
            for handle in handles {
                if let Ok(mut found) = handle.join() {
                    out.append(&mut found);
                }
            }
        });
        out
    }

    /// A `$I` record's deletion time as a clock time: 100ns steps from 1601.
    pub(super) fn from_filetime(stamp: u64) -> Option<std::time::SystemTime> {
        const EPOCH: u64 = 11_644_473_600;
        let secs = stamp / 10_000_000;
        if secs < EPOCH {
            return None;
        }
        Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs - EPOCH))
    }
}

#[cfg(unix)]
fn absolutize_without_following_leaf(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if let (Some(parent), Some(name)) = (abs.parent(), abs.file_name()) {
        let parent_dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if let Ok(canon_parent) = fs::canonicalize(parent_dir) {
            return canon_parent.join(name);
        }
    }
    normalize_lexical(&abs)
}

#[cfg(unix)]
fn trash_one(path: &Path) -> Result<Trashed, String> {
    use std::time::SystemTime;

    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("Deleting to the trash is not available on this system.")?;
    let trash = base.join("Trash");
    let files = trash.join("files");
    let info = trash.join("info");
    fs::create_dir_all(&files).map_err(|e| format!("Could not reach the trash: {e}"))?;
    fs::create_dir_all(&info).map_err(|e| format!("Could not reach the trash: {e}"))?;

    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = name_of(path);
    let target = free_path_for(&files.join(&name), meta.is_dir());
    let absolute = absolutize_without_following_leaf(path);
    let stamp = crate::metadata::system_time_to_secs(SystemTime::now()).unwrap_or(0);
    let target_name = name_of(&target);
    let info_path = info.join(format!("{target_name}.trashinfo"));
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode(&absolute.to_string_lossy()),
        trash_date(stamp)
    );

    let cancel = AtomicBool::new(false);
    move_item(path, &target, &cancel, &mut |_| {})
        .map_err(|e| format!("Could not move to the trash: {e}"))?;
    if let Err(e) = fs::write(&info_path, body) {
        let _ = move_item(&target, path, &cancel, &mut |_| {});
        return Err(format!("Could not record the deletion: {e}"));
    }
    Ok(Trashed {
        original: path.to_path_buf(),
        when: SystemTime::now(),
        trashed: Some(target),
        info: Some(info_path),
    })
}

#[cfg(unix)]
fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(unix)]
fn trash_date(secs: i64) -> String {
    let (y, m, d, hh, mm, ss) = crate::metadata::local_civil(secs);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}")
}

#[cfg(not(any(windows, unix)))]
fn trash_one(_path: &Path) -> Result<Trashed, String> {
    Err("Deleting to the trash is not available on this system.".into())
}

// ---------------------------------------------------------------------------
// The Recycle Bin as a folder: list, restore, delete forever, empty.
// ---------------------------------------------------------------------------

/// One item the platform's Recycle Bin is holding. `content` is the bytes on
/// disk (a `$R` file on Windows, a file under `Trash/files` on Unix) and `info`
/// is the record that names where it came from.
#[derive(Clone)]
pub struct BinItem {
    pub original: PathBuf,
    pub when: std::time::SystemTime,
    pub content: PathBuf,
    pub info: PathBuf,
    pub size: u64,
    pub is_dir: bool,
}

#[cfg(windows)]
pub fn bin_list() -> Vec<BinItem> {
    recycle::records(None)
        .into_iter()
        .filter_map(|r| {
            let when = recycle::from_filetime(r.deleted)?;
            Some(BinItem {
                original: r.original,
                when,
                content: r.content,
                info: r.info,
                size: r.size,
                is_dir: r.is_dir,
            })
        })
        .collect()
}

#[cfg(unix)]
pub fn bin_list() -> Vec<BinItem> {
    let Some(base) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    else {
        return Vec::new();
    };
    bin_list_in(&base)
}

#[cfg(unix)]
fn bin_list_in(base: &Path) -> Vec<BinItem> {
    let info_dir = base.join("Trash").join("info");
    let files_dir = base.join("Trash").join("files");
    let mut out = Vec::new();
    for item in fs::read_dir(info_dir).into_iter().flatten().flatten() {
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("trashinfo") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let mut original = None;
        let mut when = None;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("Path=") {
                original = Some(PathBuf::from(percent_decode(v.trim())));
            } else if let Some(v) = line.strip_prefix("DeletionDate=") {
                when = parse_trash_date(v.trim());
            }
        }
        let (Some(original), Some(when)) = (original, when) else { continue };
        let leaf = path.file_stem().unwrap_or_default().to_os_string();
        let content = files_dir.join(&leaf);
        let Ok(meta) = fs::symlink_metadata(&content) else {
            continue;
        };
        let is_dir = meta.is_dir();
        let size = if is_dir { 0 } else { meta.len() };
        out.push(BinItem { original, when, content, info: path, size, is_dir });
    }
    out.sort_by(|a, b| b.when.cmp(&a.when));
    out
}

#[cfg(not(any(windows, unix)))]
pub fn bin_list() -> Vec<BinItem> {
    Vec::new()
}

#[cfg(unix)]
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(unix)]
fn parse_trash_date(text: &str) -> Option<std::time::SystemTime> {
    use std::time::{Duration, UNIX_EPOCH};
    let bytes = text.as_bytes();
    if bytes.len() != 19 || !text.is_ascii() {
        return None;
    }
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let digits = |slice: &str| slice.bytes().all(|b| b.is_ascii_digit());
    let y_str = text.get(0..4)?;
    let m_str = text.get(5..7)?;
    let d_str = text.get(8..10)?;
    let h_str = text.get(11..13)?;
    let mi_str = text.get(14..16)?;
    let s_str = text.get(17..19)?;
    if !(digits(y_str)
        && digits(m_str)
        && digits(d_str)
        && digits(h_str)
        && digits(mi_str)
        && digits(s_str))
    {
        return None;
    }
    let y: i32 = y_str.parse().ok()?;
    let m: u32 = m_str.parse().ok()?;
    let d: u32 = d_str.parse().ok()?;
    let h: u32 = h_str.parse().ok()?;
    let mi: u32 = mi_str.parse().ok()?;
    let s: u32 = s_str.parse().ok()?;
    let secs = crate::metadata::local_to_epoch_secs(y, m, d, h, mi, s)?;
    if secs >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(secs as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(secs.unsigned_abs()))
    }
}

/// Put one bin item back where it came from.
pub fn bin_restore(item: &BinItem) -> Result<(), String> {
    #[cfg(windows)]
    {
        recycle::restore(&item.original, item.when)
    }
    #[cfg(unix)]
    {
        restore_one(&Trashed {
            original: item.original.clone(),
            when: item.when,
            trashed: Some(item.content.clone()),
            info: Some(item.info.clone()),
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = item;
        Err("Taking an item out of the bin is not available on this system.".into())
    }
}

/// Delete one bin item forever. This bypasses the trash on purpose.
pub fn bin_delete_forever(item: &BinItem) -> Result<(), String> {
    remove_item(&item.content)?;
    let _ = remove_item(&item.info);
    Ok(())
}

pub fn bin_empty() -> Result<(), String> {
    let mut first = None;
    for item in bin_list() {
        if let Err(e) = bin_delete_forever(&item) && first.is_none() {
            first = Some(e);
        }
    }
    match first {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lyre-fileops-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn noop(_: Progress) -> bool {
        true
    }

    /// A Recycle Bin record as the shell writes one, in either format.
    #[cfg(windows)]
    fn record(version: u64, path: &str, deleted: u64) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&7u64.to_le_bytes()); // size
        out.extend_from_slice(&deleted.to_le_bytes());
        let units: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        if version == 1 {
            let mut raw = vec![0u8; 520];
            for (i, unit) in units.iter().take(260).enumerate() {
                raw[i * 2..i * 2 + 2].copy_from_slice(&unit.to_le_bytes());
            }
            out.extend_from_slice(&raw);
        } else {
            let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        out
    }

    /// Both record formats name the path that was deleted, which is how the item
    /// we deleted is found again.
    #[cfg(windows)]
    #[test]
    fn a_recycle_bin_record_names_where_the_item_came_from() {
        for version in [1u64, 2] {
            let bytes = record(version, r"C:\Users\someone\gone.txt", 133_000_000_000_000_000);
            let (path, deleted, _) = parse_i(&bytes).expect("the record reads");
            assert_eq!(path, PathBuf::from(r"C:\Users\someone\gone.txt"));
            assert_eq!(deleted, 133_000_000_000_000_000);
        }
        // Something that is not a record says so rather than guessing.
        assert!(parse_i(&[0u8; 4]).is_none());
        assert!(parse_i(&record(9, r"C:\x", 0)).is_none());
    }

    /// The reported wish: a delete can be taken back. This goes through the real
    /// bin of this machine.
    #[test]
    fn a_delete_to_the_trash_can_be_taken_back() {
        let dir = scratch("trash");
        let file = dir.join("gone.txt");
        fs::write(&file, b"content").unwrap();
        let cancel = AtomicBool::new(false);
        let deleted = delete(DeleteMode::Trash, std::slice::from_ref(&file), &cancel, noop);
        if deleted.error.is_some() {
            // A machine with the bin turned off cannot delete this way at all.
            let _ = fs::remove_dir_all(&dir);
            return;
        }
        assert_eq!(deleted.count, 1);
        assert!(!file.exists(), "it went to the trash");
        assert_eq!(deleted.trashed.len(), 1, "and the trash recorded where it came from");

        restore(&deleted.trashed).expect("it can be taken back out");
        assert!(file.exists(), "and it is back where it was");
        assert_eq!(fs::read_to_string(&file).unwrap(), "content");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A folder holding a link back to a folder above it is endless if a copy
    /// walks through links: the walk ran until the stack gave out, which ends
    /// the whole application rather than failing one operation.
    #[cfg(windows)]
    #[test]
    fn copying_a_folder_that_links_back_up_finishes() {
        let base = scratch("loop");
        let folder = base.join("folder");
        fs::create_dir_all(folder.join("inner")).unwrap();
        fs::write(folder.join("inner/kept.txt"), b"x").unwrap();
        // A junction from inside the folder back to the folder itself.
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(folder.join("inner").join("back"))
            .arg(&folder)
            .output()
            .expect("cmd runs");
        assert!(
            output.status.success(),
            "the test needs a junction to make: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            folder.join("inner/back").is_dir(),
            "the junction points at the folder"
        );

        // The size walk the progress bar uses has to come back too.
        let _ = item_size(&folder);

        let dest = base.join("copy");
        fs::create_dir_all(&dest).unwrap();
        let cancel = AtomicBool::new(false);
        let pasted = paste(Mode::Copy, Conflict::Rename, std::slice::from_ref(&folder), &dest, &cancel, noop);
        if let Some(error) = pasted.error {
            panic!("the copy must finish, it said: {error}");
        }
        assert!(
            dest.join("folder/inner/kept.txt").exists(),
            "the files inside are copied"
        );
        let _ = fs::remove_dir_all(&base);
    }

    /// A folder copy must report bytes while it runs, not only at the end:
    /// the status bar's percentage climbs through the transfer.
    #[test]
    fn folder_copy_reports_bytes_while_it_runs() {
        let base = scratch("progress");
        let src = base.join("src");
        fs::create_dir_all(&src).unwrap();
        for i in 0..32 {
            fs::write(src.join(format!("f{i:02}.bin")), vec![7u8; 4096]).unwrap();
        }
        let dest = base.join("dest");
        fs::create_dir_all(&dest).unwrap();
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let pasted = paste(
            Mode::Copy,
            Conflict::Rename,
            &[src],
            &dest,
            &cancel,
            |p| {
                seen.push((p.done_bytes, p.total_bytes));
                true
            },
        );
        assert!(pasted.error.is_none(), "the copy must finish: {:?}", pasted.error);
        assert_eq!(pasted.landed.len(), 1, "and one item landed");
        let total = 32 * 4096;
        assert!(
            seen.iter().any(|&(d, t)| t == total && d > 0 && d < total),
            "progress must climb inside the folder: {seen:?}"
        );
        assert_eq!(seen.last().unwrap().0, total);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn single_large_file_copy_reports_bytes_while_it_runs() {
        let base = scratch("single-file-progress");
        let src = base.join("big.bin");
        let total: u64 = 6 * 1024 * 1024;
        fs::write(&src, vec![9u8; total as usize]).unwrap();
        let dest = base.join("dest");
        fs::create_dir_all(&dest).unwrap();
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let pasted = paste(
            Mode::Copy,
            Conflict::Rename,
            &[src],
            &dest,
            &cancel,
            |p| {
                seen.push((p.done, p.total, p.done_bytes, p.total_bytes));
                true
            },
        );
        assert!(pasted.error.is_none(), "the copy must finish: {:?}", pasted.error);
        assert!(
            seen.iter().any(|&(done, tot, d, t)| done == 0 && tot == 1 && t == total && d > 0 && d < total),
            "single-file copy must report intermediate bytes while done == 0: {seen:?}"
        );
        assert_eq!(seen.last().copied(), Some((1, 1, total, total)));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn free_path_numbers_collisions() {
        let dir = scratch("free");
        let a = dir.join("file.txt");
        fs::write(&a, "x").unwrap();
        assert_eq!(free_path(&a), dir.join("file (2).txt"));
        fs::write(dir.join("file (2).txt"), "x").unwrap();
        assert_eq!(free_path(&a), dir.join("file (3).txt"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_keeps_the_original() {
        let dir = scratch("copy");
        let dst = dir.join("dst");
        fs::create_dir_all(&dst).unwrap();
        let src = dir.join("a.txt");
        fs::write(&src, "hello").unwrap();
        let cancel = AtomicBool::new(false);
        let pasted = paste(Mode::Copy, Conflict::Rename, std::slice::from_ref(&src), &dst, &cancel, noop);
        assert!(pasted.error.is_none(), "the copy must finish: {:?}", pasted.error);
        assert_eq!(pasted.landed, vec![(src.clone(), dst.join("a.txt"))], "it says where it landed");
        assert!(src.exists());
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cut_moves_and_keeps_both_on_conflict() {
        let dir = scratch("cut");
        let dst = dir.join("dst");
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("a.txt"), "existing").unwrap();
        let src = dir.join("a.txt");
        fs::write(&src, "incoming").unwrap();
        let cancel = AtomicBool::new(false);
        let pasted = paste(Mode::Cut, Conflict::Rename, std::slice::from_ref(&src), &dst, &cancel, noop);
        assert!(pasted.error.is_none(), "the move must finish: {:?}", pasted.error);
        assert_eq!(pasted.landed.len(), 1, "one item landed");
        assert!(!src.exists(), "cut removes the original");
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "existing");
        assert_eq!(fs::read_to_string(dst.join("a (2).txt")).unwrap(), "incoming");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn overwrite_replaces_the_destination() {
        let dir = scratch("over");
        let dst = dir.join("dst");
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("a.txt"), "old").unwrap();
        let src = dir.join("a.txt");
        fs::write(&src, "new").unwrap();
        let cancel = AtomicBool::new(false);
        let pasted = paste(Mode::Copy, Conflict::Overwrite, &[src], &dst, &cancel, noop);
        assert!(pasted.error.is_none(), "the copy must finish: {:?}", pasted.error);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "new");
        assert!(!dst.join("a (2).txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn permanent_delete_removes() {
        let dir = scratch("del");
        let a = dir.join("a.txt");
        fs::write(&a, "x").unwrap();
        let cancel = AtomicBool::new(false);
        delete(DeleteMode::Permanent, std::slice::from_ref(&a), &cancel, noop);
        assert!(!a.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_folder_and_rename() {
        let dir = scratch("rename");
        let made = create_folder(&dir, "New folder").unwrap();
        assert!(made.is_dir());
        let renamed = rename(&made, "Renamed").unwrap();
        assert!(renamed.is_dir());
        assert_eq!(renamed.file_name().unwrap(), "Renamed");
        assert!(!made.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copies_a_folder_tree() {
        let dir = scratch("tree");
        let src = dir.join("src");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("nested/b.txt"), "deep").unwrap();
        let dst = dir.join("dst");
        let cancel = AtomicBool::new(false);
        let pasted = paste(Mode::Copy, Conflict::Rename, &[src], &dst, &cancel, noop);
        assert!(pasted.error.is_none(), "the copy must finish: {:?}", pasted.error);
        assert_eq!(fs::read_to_string(dst.join("src/nested/b.txt")).unwrap(), "deep");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A paste that stops part way says what it already moved, so the window can
    /// put that on the undo stack instead of leaving the folder's birth there.
    #[test]
    fn a_paste_that_stops_part_way_reports_what_it_moved() {
        let dir = scratch("partway");
        let dst = dir.join("dst");
        fs::create_dir_all(&dst).unwrap();
        let first = dir.join("first.txt");
        fs::write(&first, "one").unwrap();
        let second = dir.join("second.txt");
        fs::write(&second, "two").unwrap();
        let cancel = AtomicBool::new(false);
        // The destination itself is one of the sources: refused before anything
        // moves, so no half-done paste to explain.
        let pasted = paste(Mode::Cut, Conflict::Rename, std::slice::from_ref(&dst), &dst, &cancel, noop);
        assert!(pasted.landed.is_empty(), "nothing moves when the target is the source");
        assert_eq!(
            pasted.error.as_deref(),
            Some("Cannot paste a folder into itself."),
            "and it says why"
        );
        assert!(dst.is_dir(), "the folder is still there");

        // A cancel in the middle keeps what already landed in the answer.
        let pasted = paste(Mode::Cut, Conflict::Rename, &[first.clone(), second.clone()], &dst, &cancel, |_| false);
        assert!(pasted.error.is_none(), "stopping by hand is not a failure");
        assert!(pasted.landed.is_empty(), "the first report stops the paste before it moves");

        let mut steps = 0;
        let pasted = paste(Mode::Cut, Conflict::Rename, &[first.clone(), second.clone()], &dst, &cancel, |_| {
            steps += 1;
            steps < 2
        });
        assert!(pasted.error.is_none(), "stopping by hand is not a failure");
        assert_eq!(pasted.landed, vec![(first.clone(), dst.join("first.txt"))], "the one that went is named");
        assert!(dst.join("first.txt").exists() && second.exists(), "and that is the disk's truth");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Replacing keeps what only the destination has: a folder is merged into,
    /// not deleted and written again.
    #[test]
    fn replacing_a_folder_merges_instead_of_deleting_it() {
        let dir = scratch("merge");
        let dst = dir.join("dst");
        let src = dir.join("package");
        fs::create_dir_all(dst.join("package")).unwrap();
        fs::write(dst.join("package/only-here.txt"), "keep me").unwrap();
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("shared.txt"), "new").unwrap();
        let cancel = AtomicBool::new(false);
        let pasted = paste(Mode::Copy, Conflict::Overwrite, std::slice::from_ref(&src), &dst, &cancel, noop);
        assert!(pasted.error.is_none(), "the copy must finish: {:?}", pasted.error);
        assert_eq!(
            fs::read_to_string(dst.join("package/only-here.txt")).unwrap(),
            "keep me",
            "a file only the destination has survives"
        );
        assert_eq!(fs::read_to_string(dst.join("package/shared.txt")).unwrap(), "new");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Replacing an item with itself must leave it alone: the copy would read the
    /// file it had just truncated, so the item came back empty.
    #[test]
    fn replacing_an_item_with_itself_leaves_it() {
        let dir = scratch("self");
        let folder = dir.join("Presenter");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("index.html"), "<h1>hi</h1>").unwrap();
        let cancel = AtomicBool::new(false);
        let pasted = paste(Mode::Copy, Conflict::Overwrite, std::slice::from_ref(&folder), &dir, &cancel, noop);
        assert_eq!(pasted.error.as_deref(), Some("That item is already here."));
        assert!(pasted.landed.is_empty(), "nothing was written");
        assert_eq!(
            fs::read_to_string(folder.join("index.html")).unwrap(),
            "<h1>hi</h1>",
            "the item is untouched"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Keeping both inside the folder an item already lives in still numbers the
    /// newcomer, for a file and for a folder.
    #[test]
    fn keep_both_in_the_same_folder_numbers_the_newcomer() {
        let dir = scratch("dup");
        fs::write(dir.join("a.txt"), "one").unwrap();
        let folder = dir.join("Presenter");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("index.html"), "<h1>hi</h1>").unwrap();
        let cancel = AtomicBool::new(false);
        for src in [dir.join("a.txt"), folder.clone()] {
            let pasted = paste(Mode::Copy, Conflict::Rename, std::slice::from_ref(&src), &dir, &cancel, noop);
            assert!(pasted.error.is_none(), "the copy must finish: {:?}", pasted.error);
        }
        assert_eq!(fs::read_to_string(dir.join("a (2).txt")).unwrap(), "one");
        assert!(dir.join("Presenter (2)").join("index.html").exists(), "the folder was copied too");
        assert_eq!(fs::read_to_string(folder.join("index.html")).unwrap(), "<h1>hi</h1>");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_free_path_never_falls_back_to_the_taken_one() {
        let dir = scratch("full");
        let dst = dir.join("a.txt");
        fs::write(&dst, "taken").unwrap();
        for n in 2..10_000 {
            fs::write(dir.join(format!("a ({n}).txt")), "").unwrap();
        }
        let free = free_path(&dst);
        assert_ne!(free, dst, "the taken path itself is never the answer");
        assert!(!free.exists(), "the answer is free: {}", free.display());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A rename or a new folder name that carries a step or a separator would
    /// land outside the folder on screen; both are refused before anything
    /// moves, and nothing appears outside.
    #[test]
    fn names_with_separators_are_refused() {
        let dir = scratch("names");
        let file = dir.join("a.txt");
        fs::write(&file, "one").unwrap();
        for bad in ["../b.txt", "..\\b.txt", "sub/b.txt", "..", ".", ""] {
            assert!(rename(&file, bad).is_err(), "rename to {bad:?} is refused");
            assert!(create_folder(&dir, bad).is_err(), "a folder named {bad:?} is refused");
        }
        assert!(file.exists(), "the file never moved");
        assert!(!dir.parent().unwrap().join("b.txt").exists(), "nothing landed outside");
        let _ = fs::remove_dir_all(&dir);
    }

    /// An ordinary rename and an ordinary new folder still work.
    #[test]
    fn plain_names_still_work() {
        let dir = scratch("plain");
        let file = dir.join("a.txt");
        fs::write(&file, "one").unwrap();
        let renamed = rename(&file, "b.txt").unwrap();
        assert_eq!(renamed, dir.join("b.txt"));
        assert!(renamed.exists());
        let folder = create_folder(&dir, "Notes").unwrap();
        assert!(folder.is_dir());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A delete that stops half way keeps the records of what already went,
    /// so the part that landed in the trash can still be taken back, and the
    /// failure is reported rather than swallowing both.
    #[test]
    fn a_stopped_delete_keeps_what_went() {
        let dir = scratch("delstop");
        let gone = dir.join("gone.txt");
        fs::write(&gone, "one").unwrap();
        let missing = dir.join("missing.txt");
        let outcome = delete(DeleteMode::Trash, &[gone.clone(), missing], &AtomicBool::new(false), noop);
        assert_eq!(outcome.count, 1, "the first item went");
        assert_eq!(outcome.trashed.len(), 1, "its record survived the failure");
        assert!(outcome.error.is_some(), "the failure is reported");
        assert!(!gone.exists(), "the first item still went to the trash");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A locked child makes rename fail. The copy fallback must not leave two
    /// copies behind when the source then refuses to be removed.
    #[cfg(windows)]
    #[test]
    fn a_move_that_cannot_remove_the_source_leaves_no_duplicate() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = scratch("locked-move");
        let src = dir.join("folder");
        fs::create_dir_all(&src).unwrap();
        let locked = src.join("inuse.txt");
        fs::write(&locked, b"payload").unwrap();
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(3) // FILE_SHARE_READ | FILE_SHARE_WRITE, not DELETE
            .open(&locked)
            .unwrap();

        let pasted = paste(
            Mode::Cut,
            Conflict::Rename,
            std::slice::from_ref(&src),
            &out,
            &AtomicBool::new(false),
            noop,
        );
        drop(holder);
        assert!(pasted.error.is_some(), "the move reports why it stopped");
        assert!(locked.exists(), "the original stays");
        assert!(!out.join("folder").exists(), "and no duplicate was left behind");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Replace keeps the old bytes so undo can restore them instead of
    /// deleting the destination wholesale, and a failed copy puts them back.
    #[test]
    fn a_replace_preserves_what_it_replaced() {
        let dir = scratch("replace-undo");
        let target = dir.join("target");
        fs::create_dir_all(&target).unwrap();
        let existing = target.join("notes");
        fs::create_dir_all(&existing).unwrap();
        fs::write(existing.join("keep.txt"), b"precious").unwrap();
        let source = dir.join("incoming").join("notes");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("new.txt"), b"new").unwrap();

        let pasted = paste(Mode::Copy, Conflict::Overwrite, std::slice::from_ref(&source), &target, &AtomicBool::new(false), noop);
        assert!(pasted.error.is_none());
        assert_eq!(pasted.replaced.len(), 1, "the old folder was kept for undo");
        assert!(existing.join("keep.txt").exists(), "the merge kept the old file");

        for r in pasted.replaced.iter().rev() {
            swap_back(r).unwrap();
        }
        assert!(existing.join("keep.txt").exists(), "undo restores the old content");

        for r in pasted.replaced.iter() {
            swap_forward(r).unwrap();
        }
        assert!(existing.join("new.txt").exists(), "redo puts the new content back");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A file replaced by another file: undo restores the earlier contents
    /// rather than deleting the file.
    #[test]
    fn undoing_a_file_replace_restores_the_old_contents() {
        let dir = scratch("replace-file");
        let target = dir.join("target");
        fs::create_dir_all(&target).unwrap();
        let existing = target.join("report.txt");
        fs::write(&existing, b"old").unwrap();
        let source = dir.join("incoming").join("report.txt");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"new").unwrap();

        let pasted = paste(Mode::Copy, Conflict::Overwrite, std::slice::from_ref(&source), &target, &AtomicBool::new(false), noop);
        assert!(pasted.error.is_none());
        assert_eq!(fs::read_to_string(&existing).unwrap(), "new");
        for r in pasted.replaced.iter().rev() {
            swap_back(r).unwrap();
        }
        assert_eq!(fs::read_to_string(&existing).unwrap(), "old", "undo restores the old bytes");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A cancelled Replace puts the destination back instead of leaving it gone.
    #[test]
    fn a_cancelled_replace_restores_the_destination() {
        let dir = scratch("replace-cancel");
        let dst = dir.join("target");
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("report"), b"precious").unwrap();
        let src = dir.join("incoming").join("report");
        fs::create_dir_all(&src).unwrap();
        for i in 0..20 {
            fs::write(src.join(format!("f{i}.txt")), vec![b'x'; 1000]).unwrap();
        }
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let mut first = true;
        let pasted = paste(
            Mode::Copy,
            Conflict::Overwrite,
            std::slice::from_ref(&src),
            &dst,
            &cancel,
            |_| {
                if first {
                    first = false;
                    stop.store(true, Ordering::Relaxed);
                }
                true
            },
        );
        assert!(pasted.error.is_some(), "the copy stopped");
        assert_eq!(
            fs::read_to_string(dst.join("report")).ok().as_deref(),
            Some("precious"),
            "the replaced file was restored"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// One bad trash record must not strand the rest of a multi-delete undo.
    #[cfg(unix)]
    #[test]
    fn restoring_a_multi_delete_brings_back_every_item_it_can() {
        let dir = scratch("restore-each");
        let first = dir.join("one.txt");
        let second = dir.join("two.txt");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let outcome = delete(DeleteMode::Trash, &[first.clone(), second.clone()], &AtomicBool::new(false), noop);
        assert_eq!(outcome.trashed.len(), 2);
        fs::write(&first, b"recreated").unwrap(); // this restore will fail
        let left = match restore_each(&outcome.trashed) {
            Ok(_) => panic!("one item cannot be restored"),
            Err((left, _)) => left,
        };
        assert_eq!(left.len(), 1, "only the failed item is left for the next press");
        assert!(second.exists(), "the other item came back");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A folder and a child inside it are one operation, not two, and a link
    /// chosen as the source is not walked into its own target.
    #[test]
    fn a_nested_selection_is_moved_once() {
        let dir = scratch("nested-once");
        let folder = dir.join("folder");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("child.txt"), b"child").unwrap();
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        let pasted = paste(
            Mode::Cut,
            Conflict::Rename,
            &[folder.clone(), folder.join("child.txt")],
            &out,
            &AtomicBool::new(false),
            noop,
        );
        assert!(pasted.error.is_none(), "no false failure: {:?}", pasted.error);
        assert_eq!(pasted.landed.len(), 1, "the ancestor covers the child");
        assert!(out.join("folder").join("child.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_pasted_into_its_target_is_not_walked() {
        let dir = scratch("link-self");
        let folder = dir.join("folder");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("payload.txt"), b"x").unwrap();
        let link = dir.join("shortcut");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        let pasted = paste(Mode::Copy, Conflict::Rename, std::slice::from_ref(&link), &folder, &AtomicBool::new(false), noop);
        assert!(pasted.error.is_none(), "the link is copied as a link: {:?}", pasted.error);
        assert!(fs::symlink_metadata(folder.join("shortcut")).unwrap().file_type().is_symlink());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A cut's progress reaches the total: the size is read before the item
    /// goes, not after the source has disappeared.
    #[test]
    fn a_cut_reports_all_its_bytes() {
        let dir = scratch("cut-bytes");
        let src = dir.join("payload.txt");
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(&src, vec![b'x'; 5000]).unwrap();
        let mut seen = Vec::new();
        let pasted = paste(
            Mode::Cut,
            Conflict::Rename,
            std::slice::from_ref(&src),
            &out,
            &AtomicBool::new(false),
            |p| {
                seen.push((p.done, p.done_bytes, p.total_bytes));
                true
            },
        );
        assert!(pasted.error.is_none());
        let last = seen.last().unwrap();
        assert_eq!(last.0, last.0, "done is reported");
        assert_eq!(last.1, last.2, "cut progress reaches the total: {:?}", last);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A hard link to the source is the same file; pasting onto it must not
    /// truncate the bytes through the alias.
    #[cfg(windows)]
    #[test]
    fn a_paste_onto_a_hard_link_is_refused() {
        let dir = scratch("hardlink-win");
        let target = dir.join("target");
        fs::create_dir_all(&target).unwrap();
        let src = dir.join("source.txt");
        fs::write(&src, b"precious").unwrap();
        fs::hard_link(&src, target.join("source.txt")).unwrap();
        let pasted = paste(
            Mode::Copy,
            Conflict::Overwrite,
            std::slice::from_ref(&src),
            &target,
            &AtomicBool::new(false),
            noop,
        );
        assert!(pasted.error.is_some(), "the self-paste is refused: {:?}", pasted.error);
        assert_eq!(fs::read_to_string(&src).unwrap(), "precious", "the bytes survive");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_paste_onto_a_hard_link_is_refused() {
        let dir = scratch("hardlink");
        let target = dir.join("target");
        fs::create_dir_all(&target).unwrap();
        let src = dir.join("source.txt");
        fs::write(&src, b"precious").unwrap();
        fs::hard_link(&src, target.join("source.txt")).unwrap();
        let pasted = paste(
            Mode::Copy,
            Conflict::Overwrite,
            std::slice::from_ref(&src),
            &target,
            &AtomicBool::new(false),
            noop,
        );
        assert!(pasted.error.is_some(), "the self-paste is refused");
        assert_eq!(fs::read_to_string(&src).unwrap(), "precious", "the bytes survive");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A bin listing recovers the original path, date and bytes, and restoring
    /// puts them back while deleting forever bypasses the trash.
    #[cfg(unix)]
    #[test]
    fn a_freedesktop_bin_lists_and_restores_items() {
        let base = scratch("binlist");
        let info = base.join("Trash").join("info");
        let files = base.join("Trash").join("files");
        fs::create_dir_all(&info).unwrap();
        fs::create_dir_all(&files).unwrap();
        let original = base.join("gone.txt");
        let content = files.join("gone.txt");
        fs::write(&content, b"precious").unwrap();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let record = info.join("gone.txt.trashinfo");
        fs::write(
            &record,
            format!(
                "[Trash Info]\nPath={}\nDeletionDate={}\n",
                percent_encode(&original.to_string_lossy()),
                trash_date(stamp as i64)
            ),
        )
        .unwrap();

        let items = bin_list_in(&base);
        assert_eq!(items.len(), 1, "the bin lists the item");
        assert_eq!(items[0].original, original);
        assert_eq!(fs::read_to_string(&items[0].content).unwrap(), "precious");

        let restored = bin_restore(&items[0]);
        assert!(restored.is_ok(), "restore works: {restored:?}");
        assert_eq!(fs::read_to_string(&original).unwrap(), "precious");
        assert!(!content.exists());
        assert!(!record.exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn copying_again_refuses_a_destination_that_was_taken() {
        let dir = scratch("again");
        let src = dir.join("src.txt");
        let dst = dir.join("copy.txt");
        fs::write(&src, "original").unwrap();
        fs::write(&dst, "taken since").unwrap();
        let pairs = vec![(src.clone(), dst.clone())];
        assert!(copy_again(&pairs).is_err(), "a taken destination is refused");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "taken since", "and left as it was");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn swap_back_and_forward_never_delete_target_when_backup_is_missing_or_empty() {
        let dir = scratch("swap-safe");
        let target = dir.join("precious.txt");
        fs::write(&target, b"keep-me").unwrap();

        let empty_redo = Replaced {
            source: dir.join("src.txt"),
            old: dir.join("missing-old.txt"),
            new: PathBuf::new(),
            target: target.clone(),
        };
        assert!(swap_back(&empty_redo).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep-me");
        assert!(swap_forward(&empty_redo).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep-me");

        let missing_redo = Replaced {
            source: dir.join("src.txt"),
            old: dir.join("missing-old.txt"),
            new: dir.join("missing-new.txt"),
            target: target.clone(),
        };
        assert!(swap_forward(&missing_redo).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep-me");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_selection_child_before_parent_is_not_split_on_cut() {
        let dir = scratch("nested-reverse");
        let folder = dir.join("folder");
        fs::create_dir_all(&folder).unwrap();
        let child = folder.join("child.txt");
        fs::write(&child, b"inside").unwrap();
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();

        let pasted = paste(
            Mode::Cut,
            Conflict::Rename,
            &[child, folder.clone()],
            &out,
            &AtomicBool::new(false),
            noop,
        );
        assert!(pasted.error.is_none(), "paste succeeds: {:?}", pasted.error);
        assert_eq!(pasted.landed.len(), 1, "parent covers child regardless of order");
        assert!(out.join("folder/child.txt").exists());
        assert!(!out.join("child.txt").exists(), "child was not split out of folder");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn free_path_and_create_folder_keep_dots_in_folder_names() {
        let dir = scratch("dot-folder");
        let v1 = create_folder(&dir, "v1.2").unwrap();
        assert_eq!(v1, dir.join("v1.2"));
        let v2 = create_folder(&dir, "v1.2").unwrap();
        assert_eq!(v2, dir.join("v1.2 (2)"));
        assert_eq!(free_path(&v1), dir.join("v1.2 (3)"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_preserves_file_and_folder_timestamps() {
        use std::time::{Duration, UNIX_EPOCH};
        let dir = scratch("copy-times");
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        let src_file = src_dir.join("retro.txt");
        let pre1970 = UNIX_EPOCH - Duration::from_secs(86_400 * 5);
        {
            let f = fs::File::create(&src_file).unwrap();
            f.set_times(fs::FileTimes::new().set_modified(pre1970)).unwrap();
        }
        let dir_mtime = UNIX_EPOCH + Duration::from_secs(1_577_836_800);
        apply_times(
            &{
                let f = fs::File::create(dir.join("stamp_ref")).unwrap();
                f.set_times(fs::FileTimes::new().set_modified(dir_mtime)).unwrap();
                fs::metadata(dir.join("stamp_ref")).unwrap()
            },
            &src_dir,
            true,
        );

        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        let pasted = paste(
            Mode::Copy,
            Conflict::Rename,
            std::slice::from_ref(&src_dir),
            &out,
            &AtomicBool::new(false),
            noop,
        );
        assert!(pasted.error.is_none(), "{:?}", pasted.error);
        let copied_file_mtime = fs::metadata(out.join("src/retro.txt")).unwrap().modified().unwrap();
        assert_eq!(copied_file_mtime, pre1970);
        let copied_dir_mtime = fs::metadata(out.join("src")).unwrap().modified().unwrap();
        assert_eq!(copied_dir_mtime, dir_mtime);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_only_rename_and_relocate_and_junction_guards() {
        use std::os::windows::process::CommandExt;
        let dir = scratch("win-case-junc");

        // Case-only rename + undo relocate
        let lower = dir.join("alpha.txt");
        fs::write(&lower, b"data").unwrap();
        let upper = rename(&lower, "Alpha.txt").unwrap();
        assert_eq!(upper.file_name().unwrap(), "Alpha.txt");
        relocate(&upper, &lower).expect("case-only relocate succeeds for Undo::Rename");
        assert_eq!(fs::read_to_string(&lower).unwrap(), "data");

        // Folder into itself with case difference + via junction
        let parent = dir.join("MyFolder");
        let sub = parent.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let lower_sub = PathBuf::from(sub.to_string_lossy().to_lowercase());
        let self_paste = paste(
            Mode::Copy,
            Conflict::Rename,
            std::slice::from_ref(&parent),
            &lower_sub,
            &AtomicBool::new(false),
            noop,
        );
        assert_eq!(self_paste.error.as_deref(), Some("Cannot paste a folder into itself."));

        let junction = dir.join("junc");
        let mk = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&sub)
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        if mk.status.success() {
            let via_junc = paste(
                Mode::Copy,
                Conflict::Rename,
                std::slice::from_ref(&parent),
                &junction,
                &AtomicBool::new(false),
                noop,
            );
            assert_eq!(via_junc.error.as_deref(), Some("Cannot paste a folder into itself."));

            // Broken junction removal
            fs::remove_dir_all(&sub).unwrap();
            assert!(fs::symlink_metadata(&junction).is_ok());
            remove_item(&junction).expect("broken junction removes cleanly");
            assert!(fs::symlink_metadata(&junction).is_err());
        }

        // Network UNC path refused by recycle::send
        let unc = PathBuf::from(r"\\127.0.0.1\nonexistent_share\file.txt");
        assert!(recycle::send(&[&unc]).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn unix_trash_broken_symlink_and_pre1970_trashinfo_do_not_panic() {
        let dir = scratch("unix-trash");
        let link = dir.join("broken-link");
        std::os::unix::fs::symlink(dir.join("missing"), &link).unwrap();

        // Absolutize does not follow leaf symlink
        let abs = absolutize_without_following_leaf(&link);
        assert_eq!(abs.file_name().unwrap(), "broken-link");

        // free_path and rename see broken symlinks
        assert_ne!(free_path(&link), link);
        assert!(rename(&dir.join("other"), "broken-link").is_err());

        // parse_trash_date handles pre-1970 and malformed dates without panicking
        assert!(parse_trash_date("1969-12-31T23:59:59").is_some());
        assert!(parse_trash_date("1970-01-01 00:00:00").is_none());
        assert!(parse_trash_date("2024-02-30T12:00:00").is_none());
        assert!(parse_trash_date("9999-13-01T00:00:00").is_none());
        assert!(parse_trash_date("+202-01-01T00:00:00").is_none());

        // bin_list_in lists broken symlinks and folder is_dir accurately
        let base = dir.join("xdg");
        let info = base.join("Trash/info");
        let files = base.join("Trash/files");
        fs::create_dir_all(&info).unwrap();
        fs::create_dir_all(&files).unwrap();
        std::os::unix::fs::symlink(dir.join("missing"), files.join("broken-link")).unwrap();
        fs::write(
            info.join("broken-link.trashinfo"),
            "[Trash Info]\nPath=/tmp/broken-link\nDeletionDate=1969-12-31T23:59:59\n",
        )
        .unwrap();
        fs::create_dir_all(files.join("folder")).unwrap();
        fs::write(
            info.join("folder.trashinfo"),
            "[Trash Info]\nPath=/tmp/folder\nDeletionDate=2024-01-01T12:00:00\n",
        )
        .unwrap();

        let items = bin_list_in(&base);
        assert_eq!(items.len(), 2);
        let folder_item = items.iter().find(|i| i.original == Path::new("/tmp/folder")).unwrap();
        assert!(folder_item.is_dir);
        assert!(items.iter().any(|i| i.original == Path::new("/tmp/broken-link")));

        let _ = fs::remove_dir_all(&dir);
    }
}
