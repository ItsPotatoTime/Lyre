//! "Open with": which applications declare they can open a file, which one is
//! the default, and changing that choice.
//!
//! Linux is implemented through the freedesktop conventions, because that is
//! where this build runs: the MIME type comes from `xdg-mime`, the candidates
//! come from the `.desktop` entries the desktop itself reads, opening goes
//! through `gio launch`, and the default is set with `xdg-mime default`.
//!
//! On other platforms the list comes back empty with a reason rather than
//! guessing at a registry or Launch Services layout and getting it wrong.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppEntry {
    /// Desktop file name, which is the identity `xdg-mime` uses.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWith {
    pub mime: String,
    pub default_app: Option<String>,
    pub apps: Vec<AppEntry>,
    /// Set when the platform cannot answer, so the interface can explain itself.
    pub unsupported: Option<String>,
}

struct Desktop {
    id: String,
    name: String,
    exec: String,
    mimes: Vec<String>,
}

/// The directories a desktop environment scans for applications.
fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut push_data_dir = |base: PathBuf| {
        let candidate = base.join("applications");
        if candidate.is_dir() {
            dirs.push(candidate);
        }
    };

    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        push_data_dir(PathBuf::from(data_home));
    } else if let Some(home) = std::env::var_os("HOME") {
        push_data_dir(PathBuf::from(home).join(".local/share"));
    }

    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for dir in data_dirs.split(':').filter(|dir| !dir.is_empty()) {
        push_data_dir(PathBuf::from(dir));
    }
    dirs
}

fn parse_desktop(path: &Path) -> Option<Desktop> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut name = None;
    let mut exec = None;
    let mut mimes = Vec::new();
    let mut hidden = false;
    let mut is_application = false;
    let mut in_main_section = false;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // Only the main group describes the application itself; the
            // localised overrides in later groups are ignored on purpose.
            in_main_section = line == "[Desktop Entry]";
            continue;
        }
        if !in_main_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "Name" => name = name.or_else(|| Some(value.to_string())),
            "Exec" => exec = Some(value.to_string()),
            "MimeType" => {
                mimes = value
                    .split(';')
                    .filter(|mime| !mime.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            "NoDisplay" | "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            "Type" => is_application = value.eq_ignore_ascii_case("Application"),
            _ => {}
        }
    }

    if hidden || !is_application {
        return None;
    }
    Some(Desktop {
        id: path.file_name()?.to_string_lossy().into_owned(),
        name: name?,
        exec: exec?,
        mimes,
    })
}

fn all_desktops() -> Vec<Desktop> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    // Data dirs are ordered by preference, so the first entry for an id wins.
    for dir in application_dirs() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
                continue;
            }
            let Some(desktop) = parse_desktop(&path) else {
                continue;
            };
            if seen.insert(desktop.id.clone()) {
                out.push(desktop);
            }
        }
    }
    out
}

fn xdg_mime(args: &[&str]) -> Option<String> {
    let output = Command::new("xdg-mime").args(args).output().ok()?;
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

/// MIME types have historic aliases, and the two sides disagree: `xdg-mime`
/// reports `application/java-archive` for a jar while the archive applications
/// declare `application/x-java-archive`. The shared-mime-info database records
/// which name is canonical, so both sides are normalised before comparing.
fn mime_aliases() -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(PathBuf::from(data_home));
    } else if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    roots.extend(
        data_dirs
            .split(':')
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from),
    );

    for root in roots {
        let Ok(text) = std::fs::read_to_string(root.join("mime/aliases")) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // "alias canonical"
            if let Some((alias, canonical)) = line.split_once(char::is_whitespace) {
                aliases
                    .entry(alias.to_string())
                    .or_insert_with(|| canonical.trim().to_string());
            }
        }
    }
    aliases
}

fn canonical_mime(mime: &str, aliases: &HashMap<String, String>) -> String {
    aliases.get(mime).cloned().unwrap_or_else(|| mime.to_string())
}

/// The MIME type the desktop would use for this file.
fn mime_of(path: &str) -> Option<String> {
    xdg_mime(&["query", "filetype", path])
}

/// Which applications declare support, which one is the default.
pub fn open_with(path: &str) -> OpenWith {
    if !cfg!(unix) {
        return OpenWith {
            mime: String::new(),
            default_app: None,
            apps: Vec::new(),
            unsupported: Some(
                "Choosing an application is implemented on Linux so far.".into(),
            ),
        };
    }

    let Some(mime) = mime_of(path) else {
        return OpenWith {
            mime: String::new(),
            default_app: None,
            apps: Vec::new(),
            unsupported: Some(
                "Could not work out this file's type: xdg-mime is not available.".into(),
            ),
        };
    };

    let aliases = mime_aliases();
    let wanted = canonical_mime(&mime, &aliases);
    let default_app = xdg_mime(&["query", "default", &mime]);
    let desktops = all_desktops();

    let mut apps: Vec<AppEntry> = desktops
        .iter()
        .filter(|desktop| {
            desktop
                .mimes
                .iter()
                .any(|declared| canonical_mime(declared, &aliases) == wanted)
        })
        .map(|desktop| AppEntry {
            id: desktop.id.clone(),
            name: desktop.name.clone(),
        })
        .collect();

    // The current default is always offered, even when its desktop entry does
    // not declare the type, because the desktop says it opens these files.
    if let Some(default_id) = &default_app {
        if !apps.iter().any(|app| &app.id == default_id) {
            let name = desktops
                .iter()
                .find(|desktop| &desktop.id == default_id)
                .map(|desktop| desktop.name.clone())
                .unwrap_or_else(|| default_id.clone());
            apps.push(AppEntry {
                id: default_id.clone(),
                name,
            });
        }
    }
    // Cached so the key is computed once per entry rather than per comparison.
    apps.sort_by_cached_key(|app| app.name.to_lowercase());

    OpenWith {
        mime,
        default_app,
        apps,
        unsupported: None,
    }
}

/// Launch one application with the file.
///
/// `gio launch` uses the desktop's own launch rules, including the Exec field
/// codes, so it does the argument handling rather than this code doing it
/// wrong for a path with a space in it.
pub fn open_with_app(path: &str, app_id: &str) -> Result<(), String> {
    let desktop = all_desktops()
        .into_iter()
        .find(|desktop| desktop.id == app_id)
        .ok_or_else(|| format!("{app_id} is no longer installed."))?;

    let desktop_path = application_dirs()
        .into_iter()
        .map(|dir| dir.join(&desktop.id))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("{} could not be found.", desktop.name))?;

    let launched = Command::new("gio")
        .arg("launch")
        .arg(&desktop_path)
        .arg(path)
        .spawn();

    if launched.is_ok() {
        return Ok(());
    }

    // Fall back to the Exec line with the file appended, which covers systems
    // without `gio`.
    let program = desktop
        .exec
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("{} has no command to run.", desktop.name))?;
    Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Could not start {}: {err}", desktop.name))
}

/// Make an application the default for this file's type.
pub fn set_default_app(path: &str, app_id: &str) -> Result<String, String> {
    let mime = mime_of(path)
        .ok_or_else(|| "Could not work out this file's type.".to_string())?;
    let status = Command::new("xdg-mime")
        .args(["default", app_id, &mime])
        .status()
        .map_err(|err| format!("Could not set the default: {err}"))?;
    if status.success() {
        Ok(mime)
    } else {
        Err("The desktop refused the change.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_desktop_entry_is_read_only_from_its_main_group() {
        let dir = std::env::temp_dir().join(format!("lyre-desktop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("editor.desktop");
        fs::write(
            &file,
            "[Desktop Entry]\nType=Application\nName=Text Editor\nExec=editor %F\nMimeType=text/plain;text/csv;\n\n[Desktop Action new]\nName=New\nExec=editor\n",
        )
        .unwrap();

        let desktop = parse_desktop(&file).expect("should parse");
        assert_eq!(desktop.name, "Text Editor");
        assert_eq!(desktop.exec, "editor %F");
        assert_eq!(desktop.mimes, vec!["text/plain", "text/csv"]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_alias_resolves_to_the_canonical_type() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "application/x-java-archive".to_string(),
            "application/java-archive".to_string(),
        );
        // The desktop reports the canonical name; desktop entries often use
        // the historic alias. Both must compare equal.
        assert_eq!(
            canonical_mime("application/x-java-archive", &aliases),
            "application/java-archive"
        );
        assert_eq!(
            canonical_mime("application/java-archive", &aliases),
            "application/java-archive"
        );
        assert_eq!(
            canonical_mime("text/plain", &aliases),
            "text/plain",
            "unknown types are left alone"
        );
    }

    #[test]
    fn hidden_and_non_application_entries_are_ignored() {
        let dir = std::env::temp_dir().join(format!("lyre-desktop-skip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let hidden = dir.join("hidden.desktop");
        fs::write(
            &hidden,
            "[Desktop Entry]\nType=Application\nName=Hidden\nExec=x\nNoDisplay=true\n",
        )
        .unwrap();
        assert!(parse_desktop(&hidden).is_none(), "NoDisplay must be skipped");

        let link = dir.join("link.desktop");
        fs::write(
            &link,
            "[Desktop Entry]\nType=Link\nName=Link\nExec=x\n",
        )
        .unwrap();
        assert!(parse_desktop(&link).is_none(), "only Application entries count");

        fs::remove_dir_all(&dir).ok();
    }
}

