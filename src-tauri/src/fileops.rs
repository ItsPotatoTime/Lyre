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
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Copy,
    Cut,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PasteEvent {
    Progress { done: usize, total: usize, name: String },
    Done { pasted: usize, skipped: Vec<String> },
    Failed { message: String },
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
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
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
            fs::copy(from, to)?;
            return Ok(());
        }
    }
    if meta.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }
    fs::copy(from, to)?;
    Ok(())
}

/// Move, falling back to copy-then-remove across filesystems, where a rename
/// is not possible.
fn move_tree(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_err) => {
            // A rename across mounts fails; copying and removing still lands
            // the item where the user asked.
            copy_tree(from, to)?;
            remove_tree(from)
        }
    }
}

fn remove_tree(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
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
    cancel: &std::sync::atomic::AtomicBool,
    mut emit: F,
) where
    F: FnMut(PasteEvent) -> bool,
{
    let destination = Path::new(destination);
    if !destination.is_dir() {
        let _ = emit(PasteEvent::Failed {
            message: "The destination folder is no longer available.".into(),
        });
        return;
    }

    let total = sources.len();
    let mut pasted = 0usize;
    let mut skipped: Vec<String> = Vec::new();

    for (index, source) in sources.iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
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

        if !emit(PasteEvent::Progress {
            done: index,
            total,
            name: name.clone(),
        }) {
            return;
        }

        // A paste into the folder an item already lives in must not become a
        // rename of that item, so the destination name is always freed.
        let target = free_path(&destination.join(&name));

        let result = match mode {
            Mode::Copy => copy_tree(from, &target),
            Mode::Cut => move_tree(from, &target),
        };
        match result {
            Ok(()) => pasted += 1,
            Err(err) => skipped.push(format!("{name}: {err}")),
        }
    }

    let _ = emit(PasteEvent::Done { pasted, skipped });
}

/// The trash folder for this user, following $XDG_DATA_HOME.
fn trash_dir() -> Option<PathBuf> {
    #[cfg(not(unix))]
    {
        return None;
    }
    #[cfg(unix)]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })?;
        Some(base.join("Trash"))
    }
}

/// The freedesktop trash specification: move the item into `files/` and record
/// where it came from in `info/<name>.trashinfo`, which is what lets the desktop
/// put it back.
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

    move_tree(path, &target).map_err(|err| format!("Could not move to the trash: {err}"))?;
    if let Err(err) = fs::write(&info_path, body) {
        // Without the .trashinfo the desktop cannot restore the item, so put it
        // back rather than leaving an orphan in the trash.
        let _ = move_tree(&target, path);
        return Err(format!("Could not record the deletion: {err}"));
    }
    Ok(())
}

/// `YYYY-MM-DDThh:mm:ss`. The specification asks for local time; a timezone
/// crate is not worth a dependency for this, so the value is UTC.
fn format_trash_date(secs: u64) -> String {
    let days = secs / 86_400;
    let time = secs % 86_400;
    let (hour, minute, second) = (time / 3600, (time % 3600) / 60, time % 60);
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

/// Days since the Unix epoch to a calendar date (Howard Hinnant's algorithm).
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

/// Move items to the trash. Returns how many were moved.
pub fn delete_to_trash(paths: &[String]) -> Result<usize, String> {
    let mut moved = 0usize;
    let mut first_error: Option<String> = None;
    for path in paths {
        match trash_one(Path::new(path)) {
            Ok(()) => moved += 1,
            Err(message) => {
                if first_error.is_none() {
                    first_error = Some(message);
                }
            }
        }
    }
    if moved == 0 {
        return Err(first_error.unwrap_or_else(|| "Nothing was deleted.".into()));
    }
    Ok(moved)
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
        let cancel = AtomicBool::new(false);
        let mut pasted = 0;
        let mut skipped = Vec::new();
        paste(
            &destination.to_string_lossy(),
            sources,
            mode,
            &cancel,
            |event| match event {
                PasteEvent::Done { pasted: p, skipped: s } => {
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

    #[test]
    fn dates_convert_to_the_shape_the_trash_specification_wants() {
        // 2024-02-29T12:00:00Z, a leap day, from its Unix seconds.
        assert_eq!(format_trash_date(1_709_208_000), "2024-02-29T12:00:00");
        assert_eq!(format_trash_date(0), "1970-01-01T00:00:00");
    }
}
