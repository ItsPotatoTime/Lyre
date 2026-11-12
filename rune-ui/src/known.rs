//! Where each of Lyre's known folders really lives.
//!
//! `$HOME/Desktop` is only the default. A person can move any of these folders,
//! and OneDrive's folder backup moves Desktop, Documents and Pictures without
//! asking. The rewrite joined the English name onto the home directory, so on a
//! machine set up that way the rail drew a row for a folder that is not there,
//! and every press answered "not available here" — Desktop and the Gallery,
//! which reads Pictures, were the two that showed it first. The 0.2 application
//! asked the platform and offered only the folders that answered; this does the
//! same.
//!
//! The answer cannot change while a window is up, and the rail asks on every
//! frame (`place_of` runs per frame), so it is resolved once and kept.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

type Cache = Mutex<HashMap<(PathBuf, String), Option<PathBuf>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Where `name` is for this person, if that folder is really there.
///
/// A folder that does not exist is not offered at all, rather than drawn as a
/// row that can only answer with a notice.
pub fn folder(home: &Path, name: &str) -> Option<PathBuf> {
    let key = (home.to_path_buf(), name.to_string());
    if let Some(kept) = cache().lock().ok().and_then(|kept| kept.get(&key).cloned()) {
        return kept;
    }
    let found = resolve(home, name);
    if let Ok(mut kept) = cache().lock() {
        kept.insert(key, found.clone());
    }
    found
}

/// The platform's answer, and failing that the plain join, whichever is really
/// a folder.
///
/// Both are tried because either can be the one that exists: a machine whose
/// recorded location is stale still has the folder where it started, and a
/// machine whose folder was moved has nothing at the default place.
fn resolve(home: &Path, name: &str) -> Option<PathBuf> {
    platform::resolve(home, name)
        .filter(|path| path.is_dir())
        .or_else(|| Some(home.join(name)).filter(|path| path.is_dir()))
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    /// The value name Explorer keeps each folder under in `User Shell Folders`.
    ///
    /// Four of them are named after legends rather than after themselves, and
    /// Downloads was added long after the key was designed, so the shell records
    /// it by its KNOWNFOLDERID.
    pub(super) fn value_name(name: &str) -> Option<&'static str> {
        Some(match name {
            "Desktop" => "Desktop",
            "Downloads" => "{374DE290-123F-4565-9164-39C4925E467B}",
            "Documents" => "Personal",
            "Pictures" => "My Pictures",
            "Music" => "My Music",
            "Videos" => "My Video",
            _ => return None,
        })
    }

    pub fn resolve(_home: &Path, name: &str) -> Option<PathBuf> {
        // This is the key Explorer itself reads, so a redirected folder answers
        // with the folder in use — which is why it is worth more than the plain
        // join. Reading it expands the `%USERPROFILE%` the shell writes.
        let value = value_name(name)?;
        let key = r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";
        let found = crate::shell::reg_user_string(key, Some(value))?;
        Some(PathBuf::from(found))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::*;

    /// The variable each folder is recorded under in `user-dirs.dirs`.
    fn variable(name: &str) -> Option<&'static str> {
        Some(match name {
            "Desktop" => "XDG_DESKTOP_DIR",
            // XDG spells this one without its S.
            "Downloads" => "XDG_DOWNLOAD_DIR",
            "Documents" => "XDG_DOCUMENTS_DIR",
            "Pictures" => "XDG_PICTURES_DIR",
            "Music" => "XDG_MUSIC_DIR",
            "Videos" => "XDG_VIDEOS_DIR",
            _ => return None,
        })
    }

    pub fn resolve(home: &Path, name: &str) -> Option<PathBuf> {
        let variable = variable(name)?;
        // xdg-user-dirs writes this file; the desktop's settings app edits it.
        // A path relative to XDG_CONFIG_HOME is not a base for it, so an odd
        // value falls back to the place the spec names.
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let text = std::fs::read_to_string(config.join("user-dirs.dirs")).ok()?;
        named_in(&text, variable, home)
    }

    /// The folder `variable` names in a `user-dirs.dirs`, `$HOME` expanded and
    /// the quotes taken off.
    ///
    /// A variable set to the home folder itself is how the desktop disables that
    /// folder: it means "no such place", not "the home folder".
    fn named_in(text: &str, variable: &str, home: &Path) -> Option<PathBuf> {
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim() != variable {
                continue;
            }
            let value = value.trim().trim_matches('"');
            let path = match value.strip_prefix("$HOME") {
                Some(rest) => home.join(rest.trim_start_matches('/')),
                None => PathBuf::from(value),
            };
            // Empty, relative, or the home folder itself names no folder.
            if path.as_os_str().is_empty() || path == home || !path.is_absolute() {
                return None;
            }
            return Some(path);
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The value is quoted and written with `$HOME`, which is not a path
        /// until it is expanded.
        #[test]
        fn a_quoted_home_relative_value_is_expanded() {
            let home = Path::new("/home/someone");
            let text = "\
# This file is written by xdg-user-dirs-update
XDG_DESKTOP_DIR=\"$HOME/Desktop\"
XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"
";
            assert_eq!(
                named_in(text, "XDG_DESKTOP_DIR", home),
                Some(home.join("Desktop"))
            );
            assert_eq!(
                named_in(text, "XDG_DOWNLOAD_DIR", home),
                Some(home.join("Downloads"))
            );
        }

        /// A folder moved to an absolute path elsewhere is read as that path,
        /// and a commented-out line is not read at all.
        #[test]
        fn an_absolute_value_wins_and_comments_are_skipped() {
            let home = Path::new("/home/someone");
            let text = "\
#XDG_DESKTOP_DIR=\"$HOME/Desktop\"
XDG_DESKTOP_DIR=\"/mnt/data/Desk\"
XDG_PICTURES_DIR=\"/home/someone/Pictures\"
";
            assert_eq!(
                named_in(text, "XDG_DESKTOP_DIR", home),
                Some(PathBuf::from("/mnt/data/Desk"))
            );
            // The variable asked for is the one answered, not the first line.
            assert_eq!(
                named_in(text, "XDG_PICTURES_DIR", home),
                Some(home.join("Pictures"))
            );
        }

        /// Setting a folder to the home directory is how the desktop turns it
        /// off; it must not become a place that opens the home folder.
        #[test]
        fn a_value_of_the_home_folder_names_nothing() {
            let home = Path::new("/home/someone");
            assert_eq!(named_in("XDG_DESKTOP_DIR=\"$HOME\"\n", "XDG_DESKTOP_DIR", home), None);
            assert_eq!(
                named_in("XDG_DESKTOP_DIR=\"$HOME/\"\n", "XDG_DESKTOP_DIR", home),
                None
            );
            assert_eq!(
                named_in("XDG_DESKTOP_DIR=\"relative/Desk\"\n", "XDG_DESKTOP_DIR", home),
                None
            );
            assert_eq!(named_in("", "XDG_DESKTOP_DIR", home), None);
        }

        /// A file that names one folder says nothing about the others.
        #[test]
        fn a_missing_variable_is_absent() {
            let home = Path::new("/home/someone");
            let text = "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\n";
            assert_eq!(named_in(text, "XDG_MUSIC_DIR", home), None);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    /// macOS has no per-folder setting to read: Finder shows these at their
    /// default places under the home folder, which `resolve` already tries.
    pub fn resolve(_home: &Path, _name: &str) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lyre-known-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A place is only offered when the folder behind it is there: that is what
    /// stops a row from being drawn that can only answer with a notice.
    #[test]
    fn a_place_is_only_offered_when_its_folder_is_there() {
        let Some(home) = crate::home_dir() else { return };
        for (_, name) in crate::PLACE_FOLDERS {
            if let Some(path) = folder(&home, name) {
                assert!(
                    path.is_dir(),
                    "{name} resolved to {}, which is not a folder",
                    path.display()
                );
            }
        }
    }

    /// With nothing recorded, the folder with its English name under the home
    /// folder is the place — which is what macOS and a default Linux install
    /// rely on.
    #[test]
    fn the_plain_join_is_used_when_the_platform_names_nothing() {
        let home = scratch("plain");
        std::fs::create_dir_all(home.join("Music")).unwrap();
        let found = folder(&home, "Music");
        assert!(found.is_some(), "an existing Music folder is offered");
        // A machine that records a folder of its own for Music (this one does,
        // and a Windows machine always can) is answered with that instead.
        if platform::resolve(&home, "Music").is_none() {
            assert_eq!(found, Some(home.join("Music")));
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A folder that is not there is not offered: that is what stops the rail
    /// from drawing a row that can only answer with a notice.
    #[test]
    fn a_folder_that_is_not_there_is_not_offered() {
        let empty = scratch("empty");
        // Where the platform names a folder of its own, that answer is the
        // place and this case says nothing about it.
        if platform::resolve(&empty, "Videos").is_none() {
            assert!(
                folder(&empty, "Videos").is_none(),
                "Videos is not in this home folder, so it is not offered"
            );
        }
        let _ = std::fs::remove_dir_all(&empty);
    }

    /// Asking twice is answered from the first answer: `place_of` asks once per
    /// row per frame, and the six registry or file reads behind it are not worth
    /// repeating 60 times a second.
    #[test]
    fn the_answer_is_kept_between_frames() {
        let home = scratch("cache");
        std::fs::create_dir_all(home.join("Music")).unwrap();
        let first = folder(&home, "Music");
        assert_eq!(first, folder(&home, "Music"), "the second ask agrees");
        // The kept answer is still a folder that exists, whatever it named.
        assert!(first.is_none_or(|path| path.is_dir()));
        let _ = std::fs::remove_dir_all(&home);
    }
}

#[cfg(target_os = "windows")]
#[cfg(test)]
mod windows_tests {
    use super::*;

    /// Downloads is recorded by its KNOWNFOLDERID, not by its name; the other
    /// five are recorded under legends that are not their names either.
    #[test]
    fn the_key_names_are_the_ones_explorer_writes() {
        assert_eq!(platform::value_name("Desktop"), Some("Desktop"));
        assert_eq!(
            platform::value_name("Downloads"),
            Some("{374DE290-123F-4565-9164-39C4925E467B}")
        );
        assert_eq!(platform::value_name("Documents"), Some("Personal"));
        assert_eq!(platform::value_name("Pictures"), Some("My Pictures"));
        assert_eq!(platform::value_name("Music"), Some("My Music"));
        assert_eq!(platform::value_name("Videos"), Some("My Video"));
        assert_eq!(platform::value_name("Screenshots"), None);
    }

    /// The key is readable and names the folders that exist, which is what the
    /// rail offers. A machine with a redirected folder answers with that one.
    #[test]
    fn the_shell_key_names_the_folders_that_exist() {
        let Some(home) = crate::home_dir() else { return };
        for (_, name) in crate::PLACE_FOLDERS {
            if let Some(path) = platform::resolve(&home, name) {
                assert!(
                    path.is_dir(),
                    "the key named {name} at {} which is not a folder",
                    path.display()
                );
                assert_eq!(folder(&home, name), Some(path));
            }
        }
    }
}
