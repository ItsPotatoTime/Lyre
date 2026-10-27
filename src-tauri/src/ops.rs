//! File operations the interface can actually perform.
//!
//! Deliberately small: creating a folder and renaming an item. Copy, move and
//! delete are not here because they are not safe to half-build. A rename that
//! silently overwrites, or a delete without a trash, loses a person's work; each
//! needs its own design (conflict handling, undo, progress) before it ships.
//!
//! Every operation validates its own input, because the only caller is a
//! webview and a name arriving from there is just a string.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Characters Windows forbids in a name, plus the separators every platform
/// forbids. Rejecting them everywhere keeps a cross-platform folder usable.
const FORBIDDEN: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Properties {
    pub name: String,
    pub path: String,
    pub parent: Option<String>,
    pub is_dir: bool,
    /// Bytes, for files only. Directories report `None`: totalling a tree is a
    /// walk of its own, and pretending otherwise would be a lie on screen.
    pub size: Option<u64>,
    /// Immediate children, for directories only.
    pub items: Option<u64>,
    pub modified_ms: Option<i64>,
    pub readonly: bool,
}

fn validate(name: &str) -> Result<&str, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("A name is required.".into());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("That name is reserved.".into());
    }
    if trimmed.chars().any(|c| FORBIDDEN.contains(&c)) {
        return Err("A name cannot contain / \\ : * ? \" < > |.".into());
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("A name cannot contain control characters.".into());
    }
    // A leading dot is how a hidden file is named on Unix, so it is allowed.
    // Surrounding whitespace is trimmed rather than refused, as Windows does.
    Ok(trimmed)
}

/// Make `base`, `base (2)`, `base (3)` … available inside `parent`.
fn unique_child(parent: &Path, base: &str) -> PathBuf {
    let first = parent.join(base);
    if !first.exists() {
        return first;
    }
    for n in 2..1000 {
        let candidate = parent.join(format!("{base} ({n})"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{base} ({})", std::process::id()))
}

/// Create a folder, choosing a free name rather than failing on a collision.
/// Returns the path it created, so the interface can select it and start
/// renaming, which is what Explorer does.
pub fn create_folder(parent: &str, name: &str) -> Result<String, String> {
    let parent = Path::new(parent);
    if !parent.is_dir() {
        return Err("This folder is no longer available.".into());
    }
    let base = validate(name)?;
    let target = unique_child(parent, base);
    fs::create_dir(&target).map_err(|err| format!("Could not create the folder: {err}"))?;
    Ok(target.to_string_lossy().into_owned())
}

/// Rename an item in place. Refuses to overwrite anything, and keeps the
/// extension when only the stem is given, as Explorer does.
pub fn rename_path(path: &str, name: &str) -> Result<String, String> {
    let source = Path::new(path);
    if !source.exists() {
        return Err("That item no longer exists.".into());
    }
    let parent = source
        .parent()
        .ok_or_else(|| "That item cannot be renamed.".to_string())?;

    let mut target_name = validate(name)?.to_string();
    // Typing a new name without an extension keeps the old one, so a file does
    // not lose its type by accident.
    let had_extension = source.extension().is_some();
    if had_extension && !target_name.contains('.') {
        if let Some(extension) = source.extension() {
            target_name.push('.');
            target_name.push_str(&extension.to_string_lossy());
        }
    }

    let target = parent.join(&target_name);
    if target == source {
        return Ok(target.to_string_lossy().into_owned());
    }
    if target.exists() {
        return Err(format!("There is already an item named {target_name}."));
    }
    fs::rename(source, &target).map_err(|err| format!("Could not rename: {err}"))?;
    Ok(target.to_string_lossy().into_owned())
}

/// Describe one item for the properties dialog.
pub fn properties(path: &str) -> Result<Properties, String> {
    let target = Path::new(path);
    let meta = fs::symlink_metadata(target)
        .map_err(|_| "That item no longer exists.".to_string())?;
    let is_dir = meta.is_dir();
    let size = if is_dir { None } else { Some(meta.len()) };
    let items = if is_dir {
        fs::read_dir(target)
            .ok()
            .map(|read| read.filter_map(Result::ok).count() as u64)
    } else {
        None
    };
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64);

    Ok(Properties {
        name: target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        path: path.to_string(),
        parent: target
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned()),
        is_dir,
        size,
        items,
        modified_ms,
        readonly: meta.permissions().readonly(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lyre-ops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_new_folder_gets_a_free_name_instead_of_failing() {
        let dir = scratch("new");
        let parent = dir.to_string_lossy().into_owned();

        let first = create_folder(&parent, "New folder").unwrap();
        assert!(Path::new(&first).is_dir());
        assert!(first.ends_with("New folder"));

        let second = create_folder(&parent, "New folder").unwrap();
        assert!(second.ends_with("New folder (2)"));
        assert_ne!(first, second);

        let third = create_folder(&parent, "New folder").unwrap();
        assert!(third.ends_with("New folder (3)"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_names_are_refused_before_touching_the_disk() {
        let dir = scratch("invalid");
        let parent = dir.to_string_lossy().into_owned();
        for name in ["", "   ", "a/b", "a\\b", "..", ".", "bad:name", "pipe|name"] {
            assert!(
                create_folder(&parent, name).is_err(),
                "{name:?} should be refused"
            );
        }
        // Nothing was created by the refusals.
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_hidden_name_and_padded_input_are_both_accepted() {
        let dir = scratch("hidden-name");
        let parent = dir.to_string_lossy().into_owned();

        let hidden = create_folder(&parent, ".config").unwrap();
        assert!(hidden.ends_with(".config"), "a leading dot names a hidden file");

        let padded = create_folder(&parent, "  spaced  ").unwrap();
        assert!(padded.ends_with("spaced"), "padding is trimmed");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn renaming_moves_the_item_and_keeps_the_extension() {
        let dir = scratch("rename");
        let file = dir.join("draft.txt");
        fs::write(&file, b"x").unwrap();
        let path = file.to_string_lossy().into_owned();

        let renamed = rename_path(&path, "final").unwrap();
        assert!(renamed.ends_with("final.txt"), "got {renamed}");
        assert!(!file.exists());
        assert!(Path::new(&renamed).exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn renaming_onto_an_existing_name_is_refused() {
        let dir = scratch("collide");
        fs::write(dir.join("one.txt"), b"1").unwrap();
        fs::write(dir.join("two.txt"), b"2").unwrap();
        let path = dir.join("one.txt").to_string_lossy().into_owned();

        let error = rename_path(&path, "two.txt").expect_err("must not overwrite");
        assert!(error.contains("already an item"), "got: {error}");
        assert!(dir.join("one.txt").exists(), "the original must survive");
        assert!(dir.join("two.txt").exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unchanged_name_is_not_an_error() {
        let dir = scratch("same");
        let file = dir.join("keep.txt");
        fs::write(&file, b"x").unwrap();
        let path = file.to_string_lossy().into_owned();
        assert_eq!(rename_path(&path, "keep.txt").unwrap(), path);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn properties_describe_a_file_and_a_folder() {
        let dir = scratch("props");
        let file = dir.join("note.txt");
        fs::write(&file, b"hello").unwrap();
        fs::create_dir(dir.join("inner")).unwrap();
        fs::write(dir.join("inner/a.txt"), b"x").unwrap();

        let file_props = properties(&file.to_string_lossy()).unwrap();
        assert_eq!(file_props.name, "note.txt");
        assert_eq!(file_props.size, Some(5));
        assert!(!file_props.is_dir);
        assert!(file_props.modified_ms.is_some());

        let dir_props = properties(&dir.to_string_lossy()).unwrap();
        assert!(dir_props.is_dir);
        assert_eq!(dir_props.size, None, "a folder does not report a byte size");
        assert_eq!(dir_props.items, Some(2));

        fs::remove_dir_all(&dir).ok();
    }
}
