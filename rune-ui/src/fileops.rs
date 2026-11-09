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

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// A destination that does not collide: "name (2).ext", "name (3).ext", …
pub fn free_path(dst: &Path) -> PathBuf {
    if !dst.exists() {
        return dst.to_path_buf();
    }
    let stem = dst.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = dst.extension().map(|e| format!(".{}", e.to_string_lossy()));
    let parent = dst.parent().map(Path::to_path_buf).unwrap_or_default();
    for n in 2..10_000 {
        let candidate = match &ext {
            Some(ext) => parent.join(format!("{stem} ({n}){ext}")),
            None => parent.join(format!("{stem} ({n})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    dst.to_path_buf()
}

/// Recursive size in bytes, for the progress bar's total.
fn item_size(path: &Path) -> u64 {
    match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => fs::read_dir(path)
            .map(|rd| rd.flatten().map(|e| item_size(&e.path())).sum())
            .unwrap_or(0),
        Ok(meta) => meta.len(),
        Err(_) => 0,
    }
}

fn copy_item(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64),
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    let meta = fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if meta.is_dir() {
        fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        let entries = fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
        for entry in entries.flatten() {
            let from = entry.path();
            let to = dst.join(entry.file_name());
            copy_item(&from, &to, cancel, report)?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let copied = fs::copy(src, dst).map_err(|e| format!("{}: {e}", src.display()))?;
        report(copied);
        Ok(())
    }
}

fn remove_item(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_dir() {
        fs::remove_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
    } else {
        fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))
    }
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
    copy_item(src, dst, cancel, report)?;
    remove_item(src)
}

/// Copy or move `sources` into the `target` directory, reporting progress.
pub fn paste<F: FnMut(Progress) -> bool>(
    mode: Mode,
    conflict: Conflict,
    sources: &[PathBuf],
    target: &Path,
    cancel: &AtomicBool,
    mut emit: F,
) -> Result<usize, String> {
    let total = sources.len();
    let total_bytes: u64 = sources.iter().map(|p| item_size(p)).sum();
    let mut done_bytes = 0u64;
    let mut completed = 0u64;
    let mut pasted = 0usize;
    // Bytes land inside the recursive copy, so a folder of ten thousand files
    // climbs smoothly instead of jumping from 0 to 100 at the end. Throttled
    // by time and by a sixteenth of the total so the channel never floods.
    let mut last_emit = std::time::Instant::now();
    let mut last_bytes = 0u64;
    let step = (total_bytes / 16).max(1);
    for (i, src) in sources.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = name_of(src);
        if !emit(Progress { done: i, total, name: name.clone(), done_bytes, total_bytes }) {
            break;
        }
        let mut dst = target.join(&name);
        if dst.exists() {
            match conflict {
                Conflict::Rename => dst = free_path(&dst),
                Conflict::Overwrite => remove_item(&dst)?,
            }
        }
        // Never paste a folder into itself.
        if src.is_dir() && dst.starts_with(src) {
            return Err("Cannot paste a folder into itself.".into());
        }
        {
            let mut report = |add: u64| {
                done_bytes += add;
                if last_emit.elapsed() >= std::time::Duration::from_millis(60)
                    || done_bytes - last_bytes >= step
                {
                    last_emit = std::time::Instant::now();
                    last_bytes = done_bytes;
                    emit(Progress { done: i, total, name: name.clone(), done_bytes, total_bytes });
                }
            };
            match mode {
                Mode::Copy => copy_item(src, &dst, cancel, &mut report)?,
                Mode::Cut => move_item(src, &dst, cancel, &mut report)?,
            }
        }
        // A rename moves bytes without copying them; credit them at the end.
        completed += item_size(src);
        done_bytes = done_bytes.max(completed);
        pasted += 1;
        emit(Progress { done: i + 1, total, name, done_bytes, total_bytes });
    }
    Ok(pasted)
}

/// Delete `paths` to the trash or permanently, reporting progress.
pub fn delete<F: FnMut(Progress) -> bool>(
    mode: DeleteMode,
    paths: &[PathBuf],
    cancel: &AtomicBool,
    mut emit: F,
) -> Result<usize, String> {
    let total = paths.len();
    let total_bytes: u64 = paths.iter().map(|p| item_size(p)).sum();
    let mut done_bytes = 0u64;
    let mut deleted = 0usize;
    for (i, path) in paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = name_of(path);
        if !emit(Progress { done: i, total, name: name.clone(), done_bytes, total_bytes }) {
            break;
        }
        let size = item_size(path);
        match mode {
            DeleteMode::Permanent => remove_item(path)?,
            DeleteMode::Trash => trash_one(path)?,
        }
        done_bytes += size;
        deleted += 1;
        emit(Progress { done: i + 1, total, name, done_bytes, total_bytes });
    }
    Ok(deleted)
}

pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let path = free_path(&parent.join(name));
    fs::create_dir(&path).map_err(|e| format!("Could not create the folder: {e}"))?;
    Ok(path)
}

pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("That item cannot be renamed.")?;
    let target = parent.join(new_name);
    if target == path {
        return Ok(path.to_path_buf());
    }
    if target.exists() {
        return Err("An item with that name already exists.".into());
    }
    fs::rename(path, &target).map_err(|e| format!("Could not rename: {e}"))?;
    Ok(target)
}

// ---------------------------------------------------------------------------
// Trash: the Recycle Bin on Windows, freedesktop trash on Linux/macOS.
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn trash_one(path: &Path) -> Result<(), String> {
    recycle::send(&[path])
}

#[cfg(windows)]
mod recycle {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[repr(C)]
    struct FileOperation {
        window: isize,
        action: u32,
        from: *const u16,
        to: *const u16,
        flags: u16,
        aborted: i32,
        mappings: *mut c_void,
        title: *const u16,
    }

    const DELETE: u32 = 3;
    const ALLOW_UNDO: u16 = 0x0040;
    const NO_CONFIRMATION: u16 = 0x0010;
    const SILENT: u16 = 0x0004;
    const NO_ERROR_UI: u16 = 0x0400;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHFileOperationW(operation: *mut FileOperation) -> i32;
    }

    pub fn send(paths: &[&Path]) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut list: Vec<u16> = Vec::new();
        for path in paths {
            list.extend(path.as_os_str().encode_wide());
            list.push(0);
        }
        list.push(0);
        let mut operation = FileOperation {
            window: 0,
            action: DELETE,
            from: list.as_ptr(),
            to: std::ptr::null(),
            flags: ALLOW_UNDO | NO_CONFIRMATION | SILENT | NO_ERROR_UI,
            aborted: 0,
            mappings: std::ptr::null_mut(),
            title: std::ptr::null(),
        };
        let result = unsafe { SHFileOperationW(&mut operation) };
        if result == 0 {
            Ok(())
        } else {
            Err(format!("Windows would not move it to the Recycle Bin (code {result})."))
        }
    }
}

#[cfg(unix)]
fn trash_one(path: &Path) -> Result<(), String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("Deleting to the trash is not available on this system.")?;
    let trash = base.join("Trash");
    let files = trash.join("files");
    let info = trash.join("info");
    fs::create_dir_all(&files).map_err(|e| format!("Could not reach the trash: {e}"))?;
    fs::create_dir_all(&info).map_err(|e| format!("Could not reach the trash: {e}"))?;

    let name = name_of(path);
    let target = free_path(&files.join(&name));
    let absolute = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
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
    Ok(())
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
fn trash_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", rem / 3600, (rem % 3600) / 60, rem % 60)
}

#[cfg(unix)]
fn civil(z: i64) -> (i64, i64, i64) {
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

#[cfg(not(any(windows, unix)))]
fn trash_one(_path: &Path) -> Result<(), String> {
    Err("Deleting to the trash is not available on this system.".into())
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
        paste(
            Mode::Copy,
            Conflict::Rename,
            &[src],
            &dest,
            &cancel,
            |p| {
                seen.push((p.done_bytes, p.total_bytes));
                true
            },
        )
        .unwrap();
        let total = 32 * 4096;
        assert!(
            seen.iter().any(|&(d, t)| t == total && d > 0 && d < total),
            "progress must climb inside the folder: {seen:?}"
        );
        assert_eq!(seen.last().unwrap().0, total);
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
        paste(Mode::Copy, Conflict::Rename, &[src.clone()], &dst, &cancel, noop).unwrap();
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
        paste(Mode::Cut, Conflict::Rename, &[src.clone()], &dst, &cancel, noop).unwrap();
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
        paste(Mode::Copy, Conflict::Overwrite, &[src], &dst, &cancel, noop).unwrap();
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
        delete(DeleteMode::Permanent, &[a.clone()], &cancel, noop).unwrap();
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
        paste(Mode::Copy, Conflict::Rename, &[src], &dst, &cancel, noop).unwrap();
        assert_eq!(fs::read_to_string(dst.join("src/nested/b.txt")).unwrap(), "deep");
        let _ = fs::remove_dir_all(&dir);
    }
}