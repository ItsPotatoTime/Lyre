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
        .map(|rd| rd.flatten().map(|e| size_of(&e.path(), depth + 1)).sum())
        .unwrap_or(0)
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
    let meta = fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if meta.is_dir() {
        fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        let entries = fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
        for entry in entries.flatten() {
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

/// Make a link at `dst` that points where `src` points.
///
/// A link is never walked through while copying: a junction that points back up
/// the tree would copy until the stack ran out, which takes the whole window
/// down rather than failing one operation.
#[cfg(windows)]
fn copy_link(src: &Path, dst: &Path) -> Result<(), String> {
    let target = fs::read_link(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if target.is_dir() {
        // A junction needs no privilege, where a real symlink does.
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(dst)
            .arg(&target)
            .status();
        if made.is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    // A file link, or a junction this account may not make: the link is left
    // out of the copy rather than failing the rest of it.
    Ok(())
}

#[cfg(unix)]
fn copy_link(src: &Path, dst: &Path) -> Result<(), String> {
    let target = fs::read_link(src).map_err(|e| format!("{}: {e}", src.display()))?;
    std::os::unix::fs::symlink(target, dst).map_err(|e| format!("{}: {e}", dst.display()))
}

#[cfg(not(any(unix, windows)))]
fn copy_link(_src: &Path, _dst: &Path) -> Result<(), String> {
    Ok(())
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
}

/// Put deleted items back where they came from, newest first, so a folder is
/// restored before anything that was inside it.
pub fn restore(items: &[Trashed]) -> Result<(), String> {
    for item in items.iter().rev() {
        restore_one(item)?;
    }
    Ok(())
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
    if item.original.exists() {
        return Err(format!(
            "{} is there again already.",
            item.original.display()
        ));
    }
    if let Some(parent) = item.original.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::rename(trashed, &item.original).map_err(|e| format!("Could not take it out of the trash: {e}"))?;
    let _ = fs::remove_file(info);
    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn restore_one(_item: &Trashed) -> Result<(), String> {
    Err("Taking a deletion back is not available on this system.".into())
}

fn remove_item(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_dir() {
        fs::remove_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
    } else {
        fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// One Recycle Bin record: where the item came from and when it was deleted.
///
/// The file holds an 8-byte version, the size, the deletion time as a FILETIME,
/// then the path: a fixed 260 UTF-16 units in the older format, or a length and
/// then the path in the one Vista and later write.
#[cfg(windows)]
fn parse_i(data: &[u8]) -> Option<(PathBuf, u64)> {
    if data.len() < 28 {
        return None;
    }
    let version = u64::from_le_bytes(data[0..8].try_into().ok()?);
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
    Some((PathBuf::from(String::from_utf16_lossy(&units)), deleted))
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
    copy_item(src, dst, cancel, report)?;
    remove_item(src)
}

/// Move one item to a full destination path, for the steps of undo and
/// redo: a rename where it can, a copy across volumes where it cannot.
/// The destination must be free. A bare rename writes over whatever stands
/// there on Unix without a word, so an undo could land on a file that took
/// the place since; what stands there now is never written over.
pub fn relocate(src: &Path, dst: &Path) -> Result<(), String> {
    if fs::symlink_metadata(dst).is_ok() {
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
}

impl Pasted {
    fn refused(reason: &str) -> Self {
        Pasted { landed: Vec::new(), error: Some(reason.into()) }
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
) -> Result<PathBuf, String> {
    let name = name_of(src);
    let mut dst = target.join(&name);
    // Never paste a folder into itself.
    if src.is_dir() && dst.starts_with(src) && dst != src {
        return Err("Cannot paste a folder into itself.".into());
    }
    if dst.exists() {
        match conflict {
            Conflict::Rename => dst = free_path(&dst),
            Conflict::Overwrite => {
                // A file and a folder cannot stand in for one another in place.
                // Everything else merges, so what is only in the destination
                // survives and only the names in both are written over.
                if dst.is_dir() != src.is_dir() {
                    remove_item(&dst)?;
                }
            }
        }
    }
    // Settled after the conflict: a copy read from the file it is writing leaves
    // it empty, so an item is never written over itself.
    if dst == src {
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
    for src in sources {
        let dst = target.join(name_of(src));
        if !src.is_dir() {
            continue;
        }
        // Strictly inside, where renaming the newcomer cannot rescue it.
        if dst != *src && dst.starts_with(src) {
            return Pasted::refused("Cannot paste a folder into itself.");
        }
        // A folder replaced by itself would be written over itself.
        if dst == *src && conflict == Conflict::Overwrite {
            return Pasted::refused(if sources.len() == 1 {
                "That item is already here."
            } else {
                "Those items are already here."
            });
        }
    }
    let total = sources.len();
    let total_bytes: u64 = sources.iter().map(|p| item_size(p)).sum();
    let mut done_bytes = 0u64;
    let mut completed = 0u64;
    let mut landed: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut error = None;
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
        let outcome = {
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
            paste_one(mode, conflict, src, target, cancel, &mut report)
        };
        match outcome {
            Ok(dst) => {
                // A rename moves bytes without copying them; credit them at the end.
                completed += item_size(src);
                done_bytes = done_bytes.max(completed);
                landed.push((src.clone(), dst));
                emit(Progress { done: i + 1, total, name, done_bytes, total_bytes });
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    Pasted { landed, error }
}

/// Copy `pairs` again, source to destination, for a redo of a copy. A
/// destination that has been taken since is refused rather than written
/// over: the redo recreates what the copy made, it does not replace what
/// is there now.
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

/// Delete `paths` to the trash or permanently, reporting progress.
pub fn delete<F: FnMut(Progress) -> bool>(
    mode: DeleteMode,
    paths: &[PathBuf],
    cancel: &AtomicBool,
    mut emit: F,
) -> Result<Deleted, String> {
    let total = paths.len();
    let total_bytes: u64 = paths.iter().map(|p| item_size(p)).sum();
    let mut done_bytes = 0u64;
    let mut deleted = 0usize;
    let mut trashed: Vec<Trashed> = Vec::new();
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
            DeleteMode::Trash => trashed.push(trash_one(path)?),
        }
        done_bytes += size;
        deleted += 1;
        emit(Progress { done: i + 1, total, name, done_bytes, total_bytes });
    }
    Ok(Deleted { count: deleted, trashed })
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
fn trash_one(path: &Path) -> Result<Trashed, String> {
    let when = std::time::SystemTime::now();
    recycle::send(&[path])?;
    Ok(Trashed { original: path.to_path_buf(), when })
}

#[cfg(windows)]
mod recycle {
    use std::ffi::c_void;
    use std::fs;
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
        let drive = original
            .components()
            .next()
            .map(|part| part.as_os_str().to_string_lossy().trim_end_matches(':').to_string());
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
        if original.exists() {
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
    }

    /// Every record in every bin this account can read.
    fn records(drive: Option<&str>) -> Vec<Record> {
        use std::path::PathBuf;
        let mut out = Vec::new();
        let letters: Vec<char> = match drive.and_then(|d| d.chars().next()) {
            Some(letter) => vec![letter],
            None => ('A'..='Z').collect(),
        };
        for letter in letters {
            let bin = PathBuf::from(format!("{}:\\$Recycle.Bin", letter));
            let Ok(users) = fs::read_dir(&bin) else { continue };
            for user in users.flatten() {
                let Ok(items) = fs::read_dir(user.path()) else { continue };
                for item in items.flatten() {
                    let name = item.file_name();
                    let name = name.to_string_lossy();
                    if !name.starts_with("$I") {
                        continue;
                    }
                    let Ok(data) = fs::read(item.path()) else { continue };
                    let Some((original, deleted)) = super::parse_i(&data) else { continue };
                    let content = item.path().with_file_name(format!("$R{}", &name[2..]));
                    if !content.exists() {
                        continue;
                    }
                    out.push(Record { info: item.path(), content, original, deleted });
                }
            }
        }
        out
    }

    /// A `$I` record's deletion time as a clock time: 100ns steps from 1601.
    fn from_filetime(stamp: u64) -> Option<std::time::SystemTime> {
        const EPOCH: u64 = 11_644_473_600;
        let secs = stamp / 10_000_000;
        if secs < EPOCH {
            return None;
        }
        Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs - EPOCH))
    }
}

#[cfg(unix)]
fn trash_one(path: &Path) -> Result<Trashed, String> {
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
fn trash_one(_path: &Path) -> Result<Trashed, String> {
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
            let (path, deleted) = parse_i(&bytes).expect("the record reads");
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
        let Ok(deleted) = delete(DeleteMode::Trash, std::slice::from_ref(&file), &cancel, noop) else {
            // A machine with the bin turned off cannot delete this way at all.
            let _ = fs::remove_dir_all(&dir);
            return;
        };
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
        delete(DeleteMode::Permanent, std::slice::from_ref(&a), &cancel, noop).unwrap();
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
}
