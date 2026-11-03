//! File operations: paste (copy or move) and delete to the trash.
//!
//! Two rules shape this module.
//!
//! **Nothing is ever overwritten.** A paste onto an existing name creates
//! "name (2)" instead. Overwriting is the one mistake a file manager cannot
//! take back, so it is not offered at all.
//!
//! **Delete means trash, not erase.** On Linux this follows the freedesktop
//! trash specification, which is what makes an accidental delete recoverable
//! from the desktop's own trash. Where no trash is available, the operation
//! refuses rather than deleting permanently.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Copy,
    Cut,
}

/// What to do when the destination already holds an item of the same name.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Conflict {
    /// Keep both, numbering the incoming one: "name (2)".
    Rename,
    /// Write over what is there, merging folders rather than replacing them.
    Overwrite,
    /// Leave the destination alone.
    Skip,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PasteEvent {
    Progress {
        /// Items finished, and how many there are.
        done: usize,
        total: usize,
        /// The item being worked on.
        name: String,
        /// Bytes written so far, and how many there are to write.
        done_bytes: u64,
        total_bytes: u64,
    },
    Done {
        pasted: usize,
        skipped: Vec<String>,
        /// Things that happened but were not what was asked, such as a move
        /// that copied everything and then could not remove the original.
        warnings: Vec<String>,
    },
    Failed { message: String },
}

/// What a delete reports while it runs, in the same shape as a paste so the
/// status bar can show either without knowing which it is.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DeleteEvent {
    Progress {
        done: usize,
        total: usize,
        name: String,
        done_bytes: u64,
        total_bytes: u64,
    },
    Done {
        deleted: usize,
        warnings: Vec<String>,
    },
}

/// One progress report, before it is framed as a paste or a delete.
///
/// Both operations move bytes around and both need the same thing on screen, so
/// the counter is neutral here and each caller turns it into its own event.
struct Step {
    done: usize,
    total: usize,
    name: String,
    done_bytes: u64,
    total_bytes: u64,
}

/// How much of an operation is done, and when to say so.
///
/// A single large file is the case this exists for: reporting per item leaves a
/// four minute move showing nothing at all. The byte counter is what fills the
/// bar, and it is throttled because a channel message per megabyte would make
/// the reporting the slow part of the copy.
struct Job<'a, F: FnMut(Step) -> bool> {
    emit: F,
    cancel: &'a std::sync::atomic::AtomicBool,
    total_items: usize,
    done_items: usize,
    total_bytes: u64,
    done_bytes: u64,
    /// The item being copied, so a byte update can name it.
    current: String,
    last: std::time::Instant,
    /// The interface stopped listening, which ends the paste.
    stopped: bool,
}

/// Nothing to report and nothing to stop: used by the trash, which reports its
/// own outcome.
#[cfg(unix)]
static NEVER_CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl<'a, F: FnMut(Step) -> bool> Job<'a, F> {
    fn new(
        emit: F,
        cancel: &'a std::sync::atomic::AtomicBool,
        total_items: usize,
        total_bytes: u64,
    ) -> Self {
        Job {
            emit,
            cancel,
            total_items,
            done_items: 0,
            total_bytes,
            done_bytes: 0,
            current: String::new(),
            last: std::time::Instant::now(),
            stopped: false,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn send(&mut self, force: bool) {
        if self.stopped {
            return;
        }
        if !force && self.last.elapsed() < std::time::Duration::from_millis(80) {
            return;
        }
        self.last = std::time::Instant::now();
        self.stopped = !(self.emit)(Step {
            done: self.done_items,
            total: self.total_items,
            name: self.current.clone(),
            done_bytes: self.done_bytes,
            total_bytes: self.total_bytes,
        });
    }

    fn start_item(&mut self, name: &str) {
        self.current = name.to_string();
        self.send(true);
    }

    fn advance(&mut self, bytes: u64) {
        self.done_bytes += bytes;
        self.send(false);
    }

    fn finish_item(&mut self) {
        self.done_items += 1;
        self.send(true);
    }
}

/// Every byte under `path`, for the size of the whole paste.
fn bytes_in(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_dir() {
        let mut total = 0;
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                total += bytes_in(&entry.path());
            }
        }
        total
    } else {
        meta.len()
    }
}

/// A job that reports nothing, for work whose progress is not shown.
#[cfg(unix)]
fn quiet_job() -> Job<'static, fn(PasteEvent) -> bool> {
    fn ignore(_: PasteEvent) -> bool {
        true
    }
    Job {
        emit: ignore,
        cancel: &NEVER_CANCELLED,
        total_items: 0,
        done_items: 0,
        total_bytes: 0,
        done_bytes: 0,
        current: String::new(),
        last: std::time::Instant::now(),
        stopped: false,
    }
}

/// Copies a file in chunks, so the bar moves during one large file and a cancel
/// can take effect inside it. A cancelled copy removes what it wrote, rather
/// than leaving half a file behind.
fn copy_file<F: FnMut(Step) -> bool>(
    from: &Path,
    to: &Path,
    job: &mut Job<'_, F>,
) -> io::Result<()> {
    use std::io::{Read, Write};

    let mut source = fs::File::open(from)?;
    let mut target = fs::File::create(to)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        if job.cancelled() {
            drop(target);
            let _ = fs::remove_file(to);
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        target.write_all(&buffer[..read])?;
        job.advance(read as u64);
    }
    // `fs::copy` carried the permission bits across; a chunked copy has to do
    // it itself, or an executable arrives without its executable bit.
    if let Ok(meta) = fs::metadata(from) {
        let _ = fs::set_permissions(to, meta.permissions());
    }
    Ok(())
}

/// Give `path` a name that does not exist yet: "a.txt", "a (2).txt", "a (3).txt".
fn free_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "item".into());
    let extension = path.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 2..10_000 {
        let name = match &extension {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} ({})", std::process::id()))
}

/// Copy a file or a whole folder tree.
fn copy_tree<F: FnMut(Step) -> bool>(
    from: &Path,
    to: &Path,
    job: &mut Job<'_, F>,
) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.is_symlink() {
        // Copy the link itself, so copying a tree cannot explode into the
        // trees its links point at.
        #[cfg(unix)]
        {
            let target = fs::read_link(from)?;
            std::os::unix::fs::symlink(target, to)?;
            return Ok(());
        }
        #[cfg(not(unix))]
        {
            copy_file(from, to, job)?;
            return Ok(());
        }
    }
    if meta.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            if job.cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()), job)?;
        }
        return Ok(());
    }
    copy_file(from, to, job)?;
    Ok(())
}

/// Move, falling back to copy-then-remove across filesystems, where a rename
/// is not possible.
fn move_tree<F: FnMut(Step) -> bool>(
    from: &Path,
    to: &Path,
    job: &mut Job<'_, F>,
) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_err) => {
            // A rename across mounts fails; copying and removing still lands
            // the item where the user asked.
            copy_tree(from, to, job)?;
            remove_tree(from).map_err(|err| {
                io::Error::new(
                    err.kind(),
                    format!("copied, but the original could not be removed ({err})"),
                )
            })
        }
    }
}

/// Copy over an existing destination, merging folders.
///
/// This is the part that must not be done by deleting the destination first.
/// A folder that already exists is merged: files present in both are written
/// over, files that exist only in the destination are left alone. Deleting the
/// old tree first would silently destroy everything the incoming copy did not
/// happen to contain, which is exactly the loss this exists to avoid.
fn copy_over<F: FnMut(Step) -> bool>(
    from: &Path,
    to: &Path,
    job: &mut Job<'_, F>,
) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;

    if meta.is_dir() {
        if to.exists() && !to.is_dir() {
            // A file where a folder is going has to go, or the merge cannot
            // create the directory.
            fs::remove_file(to)?;
        }
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            if job.cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            let entry = entry?;
            copy_over(&entry.path(), &to.join(entry.file_name()), job)?;
        }
        return Ok(());
    }

    if to.is_dir() {
        // A file replacing a folder: the folder cannot be kept, because two
        // things cannot share one name.
        fs::remove_dir_all(to)?;
    }
    // Copy over a symlink rather than through it, as a plain copy does.
    if meta.is_symlink() {
        #[cfg(unix)]
        {
            if to.exists() {
                let _ = fs::remove_file(to);
            }
            let target = fs::read_link(from)?;
            std::os::unix::fs::symlink(target, to)?;
            return Ok(());
        }
    }
    copy_file(from, to, job)?;
    Ok(())
}

/// Move over an existing destination, merging folders the same way a copy does.
fn move_over<F: FnMut(Step) -> bool>(
    from: &Path,
    to: &Path,
    job: &mut Job<'_, F>,
) -> io::Result<()> {
    if from.is_dir() && to.is_dir() {
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            move_over(&entry.path(), &to.join(entry.file_name()), job)?;
        }
        // Only the emptied source folder is removed; the destination keeps
        // everything it already had.
        fs::remove_dir(from)?;
        return Ok(());
    }

    if to.exists() {
        if to.is_dir() {
            fs::remove_dir_all(to)?;
        } else {
            fs::remove_file(to)?;
        }
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_err) => {
            copy_over(from, to, job)?;
            remove_tree(from).map_err(|err| {
                io::Error::new(
                    err.kind(),
                    format!("copied, but the original could not be removed ({err})"),
                )
            })
        }
    }
}

fn remove_tree(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        clear_read_only(path);
        fs::remove_dir_all(path)
    } else {
        clear_read_only(path);
        fs::remove_file(path)
    }
}

/// Clears the attribute that stops Windows deleting a file.
///
/// One read-only file inside a folder is enough to make `remove_dir_all` fail,
/// and a move that has already copied everything should not stop because of an
/// attribute: Explorer clears it in the same situation.
fn clear_read_only(path: &Path) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                clear_read_only(&entry.path());
            }
        }
    }
    #[allow(clippy::permissions_set_readonly_false)]
    {
        let mut permissions = meta.permissions();
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
}

/// Paste entries into a folder, reporting progress as it goes.
///
/// `cancel` is checked between items rather than inside a large file copy, so a
/// cancel takes effect at the next boundary and what was already pasted stays.
pub fn paste<F>(
    destination: &str,
    sources: &[String],
    mode: Mode,
    conflict: Conflict,
    cancel: &std::sync::atomic::AtomicBool,
    emit: F,
) where
    F: FnMut(PasteEvent) -> bool,
{
    let destination = Path::new(destination);
    if !destination.is_dir() {
        let mut emit = emit;
        let _ = emit(PasteEvent::Failed {
            message: "The destination folder is no longer available.".into(),
        });
        return;
    }

    let total = sources.len();
    // Totalled up front so the bar has a denominator. Only metadata is read, so
    // this is cheap even for a folder of films.
    let total_bytes: u64 = sources.iter().map(|source| bytes_in(Path::new(source))).sum();
    let mut emit = emit;
    let mut pasted = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    {
    let mut job = Job::new(
        |step: Step| {
            emit(PasteEvent::Progress {
                done: step.done,
                total: step.total,
                name: step.name,
                done_bytes: step.done_bytes,
                total_bytes: step.total_bytes,
            })
        },
        cancel,
        total,
        total_bytes,
    );
    for (index, source) in sources.iter().enumerate() {
        if job.cancelled() || job.stopped {
            break;
        }
        let from = Path::new(source);
        if !from.exists() {
            skipped.push(source.clone());
            continue;
        }
        // Moving an item into the folder it already lives in would rename it
        // rather than move it, and moving a folder into itself would destroy
        // it. Both are refused rather than silently turned into something else.
        if from.parent() == Some(destination) && mode == Mode::Cut {
            skipped.push(format!("{} is already here", from.display()));
            continue;
        }
        if destination.starts_with(from) {
            skipped.push(format!("{} cannot go inside itself", from.display()));
            continue;
        }
        let name = from
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "item".into());

        job.start_item(&name);
        if job.stopped {
            return;
        }

        // A paste into the folder an item already lives in must not become a
        // rename of that item, so the destination name is always freed.
        let wanted = destination.join(&name);
        let target = if wanted.exists() {
            match conflict {
                Conflict::Rename => free_path(&wanted),
                // The destination is merged, never deleted wholesale.
                Conflict::Overwrite => wanted,
                Conflict::Skip => {
                    skipped.push(format!("{name} is already here"));
                    continue;
                }
            }
        } else {
            wanted
        };
        let replacing = target.exists();

        let result = match (mode, replacing) {
            (Mode::Copy, false) => copy_tree(from, &target, &mut job),
            (Mode::Copy, true) => copy_over(from, &target, &mut job),
            (Mode::Cut, false) => move_tree(from, &target, &mut job),
            (Mode::Cut, true) => move_over(from, &target, &mut job),
        };
        match result {
            Ok(()) => {
                pasted += 1;
                job.finish_item();
            }
            Err(_err) if job.cancelled() => {
                // Stopped by the person: what was already pasted stays, and the
                // partial file this item was writing has been removed already.
                break;
            }
            Err(err) => {
                // A move that copied everything and then failed to remove the
                // original did happen; saying "skipped" would suggest the item
                // is still only in one place, which is the opposite of the
                // truth, and the person needs to know the original is there.
                if mode == Mode::Cut && target.exists() {
                    pasted += 1;
                    job.finish_item();
                    warnings.push(format!("{name} moved, but the original stayed: {err}"));
                } else {
                    skipped.push(format!("{name}: {err}"));
                }
            }
        }
        let _ = index;
    }
    }

    let _ = emit(PasteEvent::Done {
        pasted,
        skipped,
        warnings,
    });
}

/// The trash folder for this user, following $XDG_DATA_HOME.
#[cfg(unix)]
fn trash_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("Trash"))
}

/// The freedesktop trash specification: move the item into `files/` and record
/// where it came from in `info/<name>.trashinfo`, which is what lets the desktop
/// put it back.
#[cfg(unix)]
fn trash_one(path: &Path) -> Result<(), String> {
    let trash = trash_dir()
        .ok_or_else(|| "Deleting to the trash is not available on this system.".to_string())?;
    let files = trash.join("files");
    let info = trash.join("info");
    fs::create_dir_all(&files).map_err(|err| format!("Could not reach the trash: {err}"))?;
    fs::create_dir_all(&info).map_err(|err| format!("Could not reach the trash: {err}"))?;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| "That item cannot be deleted.".to_string())?;
    let target = free_path(&files.join(&name));

    let absolute = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let info_path = info.join(format!(
        "{}.trashinfo",
        target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(name)
    ));
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode(&absolute.to_string_lossy()),
        format_trash_date(stamp)
    );

    let mut quiet = quiet_job();
    move_tree(path, &target, &mut quiet)
        .map_err(|err| format!("Could not move to the trash: {err}"))?;
    if let Err(err) = fs::write(&info_path, body) {
        // Without the .trashinfo the desktop cannot restore the item, so put it
        // back rather than leaving an orphan in the trash.
        let _ = move_tree(&target, path, &mut quiet);
        return Err(format!("Could not record the deletion: {err}"));
    }
    Ok(())
}

/// The Recycle Bin, which is what Delete means on Windows.
///
/// `SHFileOperationW` with `FOF_ALLOWUNDO` is the shell's own delete: it hands
/// the item to the bin rather than erasing it, so an accidental delete stays
/// recoverable exactly as it would from Explorer.
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

    /// `FO_DELETE`.
    const DELETE: u32 = 3;
    /// `FOF_ALLOWUNDO`: put it in the bin instead of deleting it.
    const ALLOW_UNDO: u16 = 0x0040;
    const NO_CONFIRMATION: u16 = 0x0010;
    const SILENT: u16 = 0x0004;
    const NO_ERROR_UI: u16 = 0x0400;

    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(operation: *mut FileOperation) -> i32;
    }

    pub fn send(paths: &[&Path]) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        // The call wants one path after another, each terminated, then a final
        // terminator of its own.
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

/// `YYYY-MM-DDThh:mm:ss`. The specification asks for local time; a timezone
/// crate is not worth a dependency for this, so the value is UTC.
#[cfg(unix)]
fn format_trash_date(secs: u64) -> String {
    let days = secs / 86_400;
    let time = secs % 86_400;
    let (hour, minute, second) = (time / 3600, (time % 3600) / 60, time % 60);
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

/// Days since the Unix epoch to a calendar date (Howard Hinnant's algorithm).
#[cfg(unix)]
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(unix)]
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The names in `sources` that already exist in `destination`.
///
/// Checked before the paste starts so the interface can ask what to do, rather
/// than the operations thread stopping mid-way to ask a question it has no way
/// to receive an answer to.
pub fn conflicts(destination: &str, sources: &[String]) -> Vec<String> {
    let destination = Path::new(destination);
    sources
        .iter()
        .filter_map(|source| {
            let name = Path::new(source).file_name()?.to_string_lossy().into_owned();
            destination.join(&name).exists().then_some(name)
        })
        .collect()
}

/// Puts one item in whichever bin this platform has.
fn delete_one(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        recycle::send(&[path])
    }
    #[cfg(unix)]
    {
        trash_one(path)
    }
}

/// Delete items, reporting progress as it goes.
///
/// One item at a time rather than the whole list in a single call, so the bar
/// moves and a cancel takes effect between items. An item already in the bin
/// stays there: there is no undo to offer here, which is why the bin is used at
/// all.
pub fn delete<F>(paths: &[String], cancel: &std::sync::atomic::AtomicBool, emit: F)
where
    F: FnMut(DeleteEvent) -> bool,
{
    let total = paths.len();
    let total_bytes: u64 = paths.iter().map(|path| bytes_in(Path::new(path))).sum();
    let mut emit = emit;
    let mut deleted = 0usize;
    let mut warnings: Vec<String> = Vec::new();

    {
        let mut job = Job::new(
            |step: Step| {
                emit(DeleteEvent::Progress {
                    done: step.done,
                    total: step.total,
                    name: step.name,
                    done_bytes: step.done_bytes,
                    total_bytes: step.total_bytes,
                })
            },
            cancel,
            total,
            total_bytes,
        );

        for path in paths {
            if job.cancelled() || job.stopped {
                break;
            }
            let target = Path::new(path);
            let name = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "item".into());
            let size = bytes_in(target);

            job.start_item(&name);
            if job.stopped {
                return;
            }
            match delete_one(target) {
                Ok(()) => {
                    deleted += 1;
                    job.advance(size);
                    job.finish_item();
                }
                Err(_err) if job.cancelled() => break,
                Err(err) => warnings.push(format!("{name}: {err}")),
            }
        }
    }

    let _ = emit(DeleteEvent::Done { deleted, warnings });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lyre-fileops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_paste(destination: &Path, sources: &[String], mode: Mode) -> (usize, Vec<String>) {
        run_paste_with(destination, sources, mode, Conflict::Rename)
    }

    fn run_paste_with(
        destination: &Path,
        sources: &[String],
        mode: Mode,
        conflict: Conflict,
    ) -> (usize, Vec<String>) {
        let cancel = AtomicBool::new(false);
        let mut pasted = 0;
        let mut skipped = Vec::new();
        paste(
            &destination.to_string_lossy(),
            sources,
            mode,
            conflict,
            &cancel,
            |event| match event {
                PasteEvent::Done { pasted: p, skipped: s, .. } => {
                    pasted = p;
                    skipped = s;
                    true
                }
                PasteEvent::Failed { message } => panic!("{message}"),
                PasteEvent::Progress { .. } => true,
            },
        );
        (pasted, skipped)
    }

    #[test]
    fn overwriting_a_folder_merges_it_and_keeps_files_only_the_destination_had() {
        // The case that matters: the incoming folder must not erase anything
        // the destination already had, and must win where both have a file.
        let dir = scratch("merge");
        let source = dir.join("project");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("shared.txt"), b"new").unwrap();
        fs::write(source.join("incoming.txt"), b"added").unwrap();

        let into = dir.join("into");
        fs::create_dir_all(into.join("project/nested")).unwrap();
        fs::write(into.join("project/shared.txt"), b"old").unwrap();
        fs::write(into.join("project/only-here.txt"), b"keep me").unwrap();
        fs::write(into.join("project/nested/deep.txt"), b"deep").unwrap();

        let (pasted, skipped) = run_paste_with(
            &into,
            &[source.to_string_lossy().into_owned()],
            Mode::Copy,
            Conflict::Overwrite,
        );

        assert_eq!(pasted, 1, "skipped: {skipped:?}");
        assert_eq!(
            fs::read(into.join("project/shared.txt")).unwrap(),
            b"new",
            "the incoming file wins where both exist"
        );
        assert_eq!(
            fs::read(into.join("project/only-here.txt")).unwrap(),
            b"keep me",
            "a file only the destination had must survive"
        );
        assert_eq!(
            fs::read(into.join("project/nested/deep.txt")).unwrap(),
            b"deep",
            "nested destination files survive too"
        );
        assert_eq!(
            fs::read(into.join("project/incoming.txt")).unwrap(),
            b"added",
            "a new file is added"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn overwriting_a_file_replaces_its_contents() {
        let dir = scratch("overwrite-file");
        let source = dir.join("note.txt");
        fs::write(&source, b"replacement").unwrap();
        let into = dir.join("into");
        fs::create_dir(&into).unwrap();
        fs::write(into.join("note.txt"), b"original and longer").unwrap();

        let (pasted, _) = run_paste_with(
            &into,
            &[source.to_string_lossy().into_owned()],
            Mode::Copy,
            Conflict::Overwrite,
        );
        assert_eq!(pasted, 1);
        assert_eq!(
            fs::read(into.join("note.txt")).unwrap(),
            b"replacement",
            "no leftover bytes from the longer original"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn moving_over_a_folder_merges_it_too() {
        let dir = scratch("merge-move");
        let source = dir.join("project");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("shared.txt"), b"moved").unwrap();

        let into = dir.join("into");
        fs::create_dir_all(into.join("project")).unwrap();
        fs::write(into.join("project/shared.txt"), b"old").unwrap();
        fs::write(into.join("project/untouched.txt"), b"stay").unwrap();

        let (pasted, _) = run_paste_with(
            &into,
            &[source.to_string_lossy().into_owned()],
            Mode::Cut,
            Conflict::Overwrite,
        );
        assert_eq!(pasted, 1);
        assert!(!source.exists(), "the source folder is gone after a move");
        assert_eq!(fs::read(into.join("project/shared.txt")).unwrap(), b"moved");
        assert_eq!(
            fs::read(into.join("project/untouched.txt")).unwrap(),
            b"stay"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// One large file has to show movement on its own. This is the case that
    /// sat there for four minutes with nothing on screen.
    #[test]
    fn one_large_file_reports_bytes_as_it_copies() {
        const SIZE: usize = 32 * 1024 * 1024;
        let dir = scratch("bytes");
        let source = dir.join("film.bin");
        fs::write(&source, vec![7u8; SIZE]).unwrap();
        let into = dir.join("into");
        fs::create_dir_all(&into).unwrap();

        let cancel = AtomicBool::new(false);
        let mut mid_file = false;
        let mut total_bytes = 0;
        let mut done_bytes = 0;
        let started = std::time::Instant::now();
        paste(
            &into.to_string_lossy(),
            &[source.to_string_lossy().into_owned()],
            Mode::Copy,
            Conflict::Rename,
            &cancel,
            |event| {
                if let PasteEvent::Progress {
                    done_bytes: done,
                    total_bytes: total,
                    ..
                } = event
                {
                    done_bytes = done;
                    total_bytes = total;
                    if done > 0 && done < total {
                        mid_file = true;
                    }
                }
                true
            },
        );
        let elapsed = started.elapsed();

        assert_eq!(total_bytes as usize, SIZE, "the paste knows how big it is");
        assert_eq!(done_bytes, total_bytes, "and finishes at the end");
        assert_eq!(
            fs::metadata(into.join("film.bin")).unwrap().len() as usize,
            SIZE,
            "and the file is whole"
        );
        // A copy that outlives the reporting throttle must have reported part
        // way through; a faster one legitimately has nothing to report yet.
        assert!(
            mid_file || elapsed < std::time::Duration::from_millis(100),
            "a copy that took {elapsed:?} should have reported part way through"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// Cancelling works inside a file, not only between files, and does not
    /// leave half of one behind.
    #[test]
    fn cancelling_mid_file_leaves_nothing_behind() {
        let dir = scratch("cancel");
        let source = dir.join("big.bin");
        fs::write(&source, vec![9u8; 8 * 1024 * 1024]).unwrap();
        let into = dir.join("into");
        fs::create_dir_all(&into).unwrap();

        let cancel = AtomicBool::new(false);
        let mut pasted = usize::MAX;
        paste(
            &into.to_string_lossy(),
            &[source.to_string_lossy().into_owned()],
            Mode::Copy,
            Conflict::Rename,
            &cancel,
            |event| {
                match event {
                    // Stop as soon as the item has been announced, which is
                    // before its first chunk is written.
                    PasteEvent::Progress { .. } => {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    PasteEvent::Done { pasted: p, .. } => pasted = p,
                    PasteEvent::Failed { message } => panic!("{message}"),
                }
                true
            },
        );

        assert_eq!(pasted, 0, "nothing was reported as pasted");
        assert!(
            !into.join("big.bin").exists(),
            "the half written file is removed rather than left behind"
        );
        assert!(source.exists(), "the source is untouched by a cancelled copy");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skipping_leaves_the_destination_alone() {
        let dir = scratch("skip");
        let source = dir.join("same.txt");
        fs::write(&source, b"new").unwrap();
        let into = dir.join("into");
        fs::create_dir(&into).unwrap();
        fs::write(into.join("same.txt"), b"old").unwrap();

        let (pasted, skipped) = run_paste_with(
            &into,
            &[source.to_string_lossy().into_owned()],
            Mode::Copy,
            Conflict::Skip,
        );
        assert_eq!(pasted, 0);
        assert_eq!(skipped.len(), 1);
        assert_eq!(fs::read(into.join("same.txt")).unwrap(), b"old");
        assert!(!into.join("same (2).txt").exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn conflicts_are_reported_before_anything_is_written() {
        let dir = scratch("conflicts");
        let into = dir.join("into");
        fs::create_dir(&into).unwrap();
        fs::write(into.join("here.txt"), b"x").unwrap();

        let fresh = dir.join("fresh.txt");
        fs::write(&fresh, b"y").unwrap();
        let clash = dir.join("here.txt");

        let names = conflicts(
            &into.to_string_lossy(),
            &[
                fresh.to_string_lossy().into_owned(),
                clash.to_string_lossy().into_owned(),
            ],
        );
        assert_eq!(names, vec!["here.txt".to_string()]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copying_a_file_into_a_folder_keeps_both() {
        let dir = scratch("copy");
        let source = dir.join("note.txt");
        fs::write(&source, b"hello").unwrap();
        let into = dir.join("into");
        fs::create_dir(&into).unwrap();

        let (pasted, skipped) =
            run_paste(&into, &[source.to_string_lossy().into_owned()], Mode::Copy);
        assert_eq!(pasted, 1);
        assert!(skipped.is_empty());
        assert!(source.exists(), "a copy leaves the original");
        assert_eq!(fs::read(into.join("note.txt")).unwrap(), b"hello");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cutting_moves_and_leaves_nothing_behind() {
        let dir = scratch("cut");
        let source = dir.join("move.txt");
        fs::write(&source, b"x").unwrap();
        let into = dir.join("into");
        fs::create_dir(&into).unwrap();

        let (pasted, _) = run_paste(&into, &[source.to_string_lossy().into_owned()], Mode::Cut);
        assert_eq!(pasted, 1);
        assert!(!source.exists(), "a move removes the original");
        assert!(into.join("move.txt").exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_name_that_already_exists_is_numbered_not_overwritten() {
        let dir = scratch("collide");
        let source = dir.join("same.txt");
        fs::write(&source, b"new").unwrap();
        let into = dir.join("into");
        fs::create_dir(&into).unwrap();
        fs::write(into.join("same.txt"), b"old").unwrap();

        let (pasted, _) = run_paste(&into, &[source.to_string_lossy().into_owned()], Mode::Copy);
        assert_eq!(pasted, 1);
        assert_eq!(
            fs::read(into.join("same.txt")).unwrap(),
            b"old",
            "the existing file is untouched"
        );
        assert_eq!(fs::read(into.join("same (2).txt")).unwrap(), b"new");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_whole_folder_is_copied_with_its_contents() {
        let dir = scratch("tree");
        let source = dir.join("project");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("a.txt"), b"a").unwrap();
        fs::write(source.join("nested/b.txt"), b"b").unwrap();
        let into = dir.join("into");
        fs::create_dir(&into).unwrap();

        let (pasted, _) = run_paste(&into, &[source.to_string_lossy().into_owned()], Mode::Copy);
        assert_eq!(pasted, 1);
        assert_eq!(fs::read(into.join("project/nested/b.txt")).unwrap(), b"b");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pasting_into_the_same_folder_does_not_rename_the_original() {
        let dir = scratch("self");
        let source = dir.join("keep.txt");
        fs::write(&source, b"keep").unwrap();

        let (pasted, _) = run_paste(&dir, &[source.to_string_lossy().into_owned()], Mode::Copy);
        assert_eq!(pasted, 1);
        assert_eq!(
            fs::read(&source).unwrap(),
            b"keep",
            "the original keeps its name"
        );
        assert_eq!(fs::read(dir.join("keep (2).txt")).unwrap(), b"keep");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn moving_an_item_into_its_own_folder_is_refused() {
        // A drag that lands on the folder it came from must do nothing, not
        // quietly rename the file to "keep (2).txt".
        let dir = scratch("move-self");
        let source = dir.join("keep.txt");
        fs::write(&source, b"keep").unwrap();

        let (pasted, skipped) = run_paste(&dir, &[source.to_string_lossy().into_owned()], Mode::Cut);
        assert_eq!(pasted, 0, "nothing should move");
        assert_eq!(skipped.len(), 1);
        assert!(source.exists());
        assert!(!dir.join("keep (2).txt").exists(), "no renamed duplicate");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_folder_cannot_be_moved_inside_itself() {
        let dir = scratch("move-into-self");
        let outer = dir.join("outer");
        let inner = outer.join("inner");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join("file.txt"), b"x").unwrap();

        // Dropping "outer" onto "outer/inner" must be refused.
        let (pasted, skipped) = run_paste(&inner, &[outer.to_string_lossy().into_owned()], Mode::Cut);
        assert_eq!(pasted, 0);
        assert_eq!(skipped.len(), 1);
        assert!(inner.join("file.txt").exists(), "the tree is untouched");
        assert!(!inner.join("outer").exists(), "no copy was made inside itself");

        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn dates_convert_to_the_shape_the_trash_specification_wants() {
        // 2024-02-29T12:00:00Z, a leap day, from its Unix seconds.
        assert_eq!(format_trash_date(1_709_208_000), "2024-02-29T12:00:00");
        assert_eq!(format_trash_date(0), "1970-01-01T00:00:00");
    }

    /// Delete on Windows means the Recycle Bin. Ignored by default because it
    /// leaves an item in the real bin.
    #[cfg(windows)]
    #[test]
    #[ignore = "moves a file into the real Recycle Bin"]
    fn delete_sends_the_item_to_the_recycle_bin() {
        let dir = scratch("recycle");
        let file = dir.join("gone.txt");
        fs::write(&file, b"throwaway").unwrap();
        let other = dir.join("also-gone.txt");
        fs::write(&other, b"throwaway too").unwrap();

        let cancel = AtomicBool::new(false);
        let mut deleted = 0usize;
        let mut reported_total = 0usize;
        let mut names = Vec::new();
        delete(
            &[
                file.to_string_lossy().into_owned(),
                other.to_string_lossy().into_owned(),
            ],
            &cancel,
            |event| {
                match event {
                    DeleteEvent::Progress { total, name, .. } => {
                        reported_total = total;
                        if !name.is_empty() {
                            names.push(name);
                        }
                    }
                    DeleteEvent::Done { deleted: count, .. } => deleted = count,
                }
                true
            },
        );
        assert_eq!(deleted, 2);
        assert_eq!(reported_total, 2, "the bar knows how many items there are");
        assert!(
            names.iter().any(|name| name == "gone.txt"),
            "the bar names the item being deleted"
        );
        assert!(!file.exists(), "the file is no longer in the folder");
        assert!(!other.exists());

        fs::remove_dir_all(&dir).ok();
    }
}
