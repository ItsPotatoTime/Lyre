//! Becoming the file manager the desktop launches.
//!
//! Linux has a defined answer: a `.desktop` entry declaring `inode/directory`
//! in its MimeType, registered as the handler for that type with `xdg-mime`.
//! That is what the "open a folder" action of every desktop environment looks
//! up, so it is what makes a folder open here instead of in the previous file
//! manager.
//!
//! Windows and macOS do not offer an equivalent: Explorer and Finder are the
//! shell rather than an association, and neither provides a supported way to
//! replace them from an application. The interface says so rather than
//! pretending.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerState {
    /// The desktop's current handler for directories, if any.
    pub current: Option<String>,
    /// Whether that handler is this application.
    pub is_default: bool,
    /// Set when this platform cannot be asked to change it.
    pub unsupported: Option<String>,
    /// Where the desktop entry was, or would be, written.
    pub desktop_file: Option<String>,
    /// Whether the change can be undone from here.
    pub restore_supported: bool,
}

#[cfg(unix)]
mod linux {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    /// `$XDG_DATA_HOME/applications`, which is where a per-user desktop entry
    /// belongs.
    fn applications_dir() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })?;
        Some(base.join("applications"))
    }

    fn desktop_file() -> Option<PathBuf> {
        Some(applications_dir()?.join("lyre.desktop"))
    }

    fn current_handler() -> Option<String> {
        let output = Command::new("xdg-mime")
            .args(["query", "default", "inode/directory"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Quote an argument for an Exec line, which is not a shell but has its own
    /// rules: a path with spaces must be wrapped, and `"` inside it escaped.
    pub(super) fn quote(value: &str) -> String {
        if !value.is_empty()
            && !value
                .chars()
                .any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
        {
            return value.to_string();
        }
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    }

    pub fn state() -> ManagerState {
        let current = current_handler();
        let desktop_file = desktop_file();
        ManagerState {
            is_default: current.as_deref() == Some("lyre.desktop"),
            current,
            unsupported: None,
            desktop_file: desktop_file.map(|path| path.to_string_lossy().into_owned()),
            restore_supported: true,
        }
    }

    /// Hand folders back to whatever handled them before: the desktop's own
    /// default is restored by clearing this application's registration.
    pub fn restore() -> Result<ManagerState, String> {
        if let Some(path) = desktop_file() {
            let _ = std::fs::remove_file(path);
        }
        // `xdg-mime` has no "unset"; an empty handler is the documented way to
        // say that nothing in particular is preferred.
        let _ = Command::new("xdg-mime")
            .args(["default", "", "inode/directory"])
            .status();
        Ok(state())
    }

    pub fn make_default() -> Result<ManagerState, String> {
        let dir = applications_dir()
            .ok_or_else(|| "Could not find where this user's applications live.".to_string())?;
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("Could not create {}: {err}", dir.display()))?;

        let exe = std::env::current_exe()
            .map_err(|err| format!("Could not locate this application: {err}"))?;
        let exe = exe.to_string_lossy();

        // `%U` is the folder the desktop hands us when a directory is opened.
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Lyre\n\
             GenericName=File Manager\n\
             Comment=A fast file manager in the Windows 11 design language\n\
             Exec={} %U\n\
             Icon=system-file-manager\n\
             Terminal=false\n\
             Categories=System;FileManager;\n\
             MimeType=inode/directory;\n\
             StartupNotify=true\n",
            quote(&exe)
        );

        let path = dir.join("lyre.desktop");
        std::fs::write(&path, entry)
            .map_err(|err| format!("Could not write {}: {err}", path.display()))?;

        // Best effort: refreshes the desktop's cache. Not every system has it.
        let _ = Command::new("update-desktop-database").arg(&dir).output();

        let status = Command::new("xdg-mime")
            .args(["default", "lyre.desktop", "inode/directory"])
            .status()
            .map_err(|err| format!("Could not set the handler: {err}"))?;
        if !status.success() {
            return Err("The desktop refused to change the folder handler.".into());
        }

        Ok(state())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// The verb this application registers for folders.
    const VERB: &str = "Lyre.Open";

    /// Windows has no association for folders: Explorer *is* the shell, and
    /// there is no `inode/directory` equivalent to hand to another program.
    ///
    /// What is possible, and what third-party file managers do, is to override
    /// the *default verb* the shell uses for directories and drives. Double
    /// clicking a folder then runs the command named there instead. It is done
    /// per user under HKCU so it needs no administrator rights, and the stock
    /// `open` verb is left in place so restoring is a matter of pointing the
    /// default back at it.
    fn reg(args: &[&str]) -> Result<String, String> {
        let output = std::process::Command::new("reg")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|err| format!("Could not run reg: {err}"))?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if output.status.success() {
            Ok(text)
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }

    fn default_verb(key: &str) -> Option<String> {
        let text = reg(&["query", key, "/ve"]).ok()?;
        text.lines().find_map(|line| {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("(Default)") {
                return None;
            }
            trimmed
                .split("REG_SZ")
                .nth(1)
                .map(|value| value.trim().to_string())
        })
    }

    pub fn state() -> ManagerState {
        let current = default_verb(r"HKCU\Software\Classes\Directory\shell");
        ManagerState {
            is_default: current.as_deref() == Some(VERB),
            current,
            unsupported: None,
            desktop_file: None,
            restore_supported: true,
        }
    }

    pub fn make_default() -> Result<ManagerState, String> {
        let exe = std::env::current_exe()
            .map_err(|err| format!("Could not locate this application: {err}"))?;
        let command = format!("\"{}\" \"%1\"", exe.display());

        reg(&[
            "add",
            &format!(r"HKCU\Software\Classes\{VERB}\command"),
            "/ve",
            "/d",
            &command,
            "/f",
        ])?;

        // Directories and drives are separate verbs in the shell.
        for kind in ["Directory", "Drive"] {
            reg(&[
                "add",
                &format!(r"HKCU\Software\Classes\{kind}\shell"),
                "/ve",
                "/d",
                VERB,
                "/f",
            ])?;
        }

        Ok(state())
    }

    pub fn restore() -> Result<ManagerState, String> {
        // Point the default back at the shell's own verb, then remove ours.
        for kind in ["Directory", "Drive"] {
            reg(&[
                "add",
                &format!(r"HKCU\Software\Classes\{kind}\shell"),
                "/ve",
                "/d",
                "open",
                "/f",
            ])?;
        }
        let _ = reg(&[
            "delete",
            &format!(r"HKCU\Software\Classes\{VERB}"),
            "/f",
        ]);
        Ok(state())
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
mod platform {
    use super::*;

    pub fn state() -> ManagerState {
        ManagerState {
            current: None,
            is_default: false,
            unsupported: Some(
                "macOS opens folders with Finder, which is part of the system rather than an \
                 application that can be registered for the folder type."
                    .into(),
            ),
            desktop_file: None,
            restore_supported: false,
        }
    }

    pub fn make_default() -> Result<ManagerState, String> {
        Err(state()
            .unsupported
            .unwrap_or_else(|| "Not supported on this platform.".into()))
    }

    /// Nothing to restore, because nothing was changed.
    pub fn restore() -> Result<ManagerState, String> {
        Err(state()
            .unsupported
            .unwrap_or_else(|| "Not supported on this platform.".into()))
    }
}

/// The folder this process was asked to open, if it was launched with one.
///
/// Desktops pass the folder as the first argument from the `%U` in the desktop
/// entry, so this is what makes "open this folder in Lyre" work.
pub fn startup_path() -> Option<String> {
    for arg in std::env::args_os().skip(1) {
        let text = arg.to_string_lossy();
        // A flag is not a folder; skip it and keep looking.
        if text.starts_with('-') {
            continue;
        }
        let path = std::path::Path::new(&arg);
        if path.is_dir() {
            return Some(path.to_string_lossy().into_owned());
        }
        // Some desktops hand over a file URL instead of a path.
        if let Some(rest) = text.strip_prefix("file://") {
            let decoded = rest.replace("%20", " ");
            if std::path::Path::new(&decoded).is_dir() {
                return Some(decoded);
            }
        }
    }
    None
}

/// The shape the interface expects when the platform cannot answer at all.
pub fn unavailable() -> ManagerState {
    ManagerState {
        current: None,
        is_default: false,
        unsupported: Some("The folder handler could not be read.".into()),
        desktop_file: None,
        restore_supported: false,
    }
}

#[cfg(target_os = "linux")]
pub use linux::{make_default, restore, state};

#[cfg(not(target_os = "linux"))]
pub use platform::{make_default, restore, state};


/// Test-only access to the Exec quoting, which is otherwise private to the
/// Linux implementation.
#[cfg(all(test, target_os = "linux"))]
fn quote_for_test(value: &str) -> String {
    linux::quote(value)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn the_exec_quoting_survives_a_path_with_spaces() {
        // The rules are simple but getting them wrong breaks the launcher, so
        // they are pinned here rather than discovered later.
        assert_eq!(super::quote_for_test("/usr/bin/lyre"), "/usr/bin/lyre");
        assert_eq!(
            super::quote_for_test("/home/a b/lyre"),
            "\"/home/a b/lyre\""
        );
        assert_eq!(
            super::quote_for_test("/home/a\"b/lyre"),
            "\"/home/a\\\"b/lyre\""
        );
    }

    #[test]
    fn a_startup_argument_that_is_not_a_folder_is_ignored() {
        // No arguments in the test harness, so this only proves it does not
        // panic or invent a path.
        let start = startup_path();
        assert!(start.is_none() || std::path::Path::new(&start.unwrap()).is_dir());
    }
}
