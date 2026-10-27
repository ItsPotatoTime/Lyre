//! The places the navigation rail shows, and the shape of a path as the
//! breadcrumb needs it.
//!
//! Known folders are resolved without a helper crate: the platform
//! environment already names them, and a folder that does not exist is simply
//! not offered rather than shown as a dead link.

use serde::Serialize;
use std::path::{Component, Path, PathBuf, MAIN_SEPARATOR};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub name: String,
    pub path: String,
    /// `folder` | `drive` | `removable`
    pub kind: &'static str,
    /// Glyph family the rail draws.
    pub icon: &'static str,
    /// Volume size in bytes; zero for folders. The interface needs both
    /// numbers, not just the formatted summary, to draw a capacity bar.
    pub total: u64,
    pub free: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInfo {
    pub path: String,
    /// Display name: the last segment, or the volume for a filesystem root.
    pub name: String,
    pub parent: Option<String>,
    pub segments: Vec<Segment>,
    pub exists: bool,
    pub is_dir: bool,
}

pub fn home() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home);
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
        return PathBuf::from(profile);
    }
    // Last resort on Windows: assemble it from the two variables that compose it.
    match (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
        (Some(drive), Some(path)) => {
            let mut joined = PathBuf::from(drive);
            joined.push(path);
            joined
        }
        _ => PathBuf::from(MAIN_SEPARATOR.to_string()),
    }
}

pub fn places() -> Vec<Place> {
    let mut out = Vec::with_capacity(12);
    let home = home();

    out.push(Place {
        name: display_name(&home),
        path: home.to_string_lossy().into_owned(),
        kind: "folder",
        icon: "home",
        total: 0,
        free: 0,
    });

    // The XDG user directories are declared in a config file on Linux, and the
    // folder names are localised on all three platforms; the English defaults
    // are what the shells create, and anything missing is dropped below.
    let known: [(&str, &[&str], &str); 6] = [
        ("Desktop", &["Desktop", "Bureau", "Schreibtisch", "Escritorio"], "desktop"),
        ("Downloads", &["Downloads", "Téléchargements", "Herunterladen", "Descargas"], "download"),
        ("Documents", &["Documents", "Dokumente", "Documentos"], "document"),
        ("Pictures", &["Pictures", "Images", "Bilder", "Imágenes"], "image"),
        ("Music", &["Music", "Musique", "Musik", "Música"], "audio"),
        ("Videos", &["Videos", "Vidéos", "Films"], "video"),
    ];

    for (label, candidates, icon) in known {
        let found = candidates
            .iter()
            .map(|c| home.join(c))
            .find(|p| p.is_dir());
        if let Some(path) = found {
            out.push(Place {
                name: label.to_string(),
                path: path.to_string_lossy().into_owned(),
                kind: "folder",
                icon,
                total: 0,
                free: 0,
            });
        }
    }

    for disk in drives() {
        out.push(disk);
    }

    out
}

fn drives() -> Vec<Place> {
    use sysinfo::Disks;

    let disks = Disks::new_with_refreshed_list();
    let mut out = Vec::with_capacity(disks.len());
    for disk in &disks {
        let mount = disk.mount_point();
        if mount.as_os_str().is_empty() {
            continue;
        }
        let total = disk.total_space();
        if total == 0 {
            continue;
        }
        let free = disk.available_space();
        let label = disk.name().to_string_lossy().trim().to_string();
        let name = if label.is_empty() {
            display_name(mount)
        } else {
            label
        };
        out.push(Place {
            name,
            path: mount.to_string_lossy().into_owned(),
            kind: if disk.is_removable() { "removable" } else { "drive" },
            icon: "drive",
            total,
            free,
        });
    }
    // Stable order: fixed drives by path, so the rail does not reshuffle
    // between the first and second enumeration.
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn path_info(raw: &str) -> PathInfo {
    let path = Path::new(raw);

    let mut segments: Vec<Segment> = Vec::with_capacity(8);
    let mut built = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                built = PathBuf::from(prefix.as_os_str());
                let text = built.to_string_lossy().into_owned();
                segments.push(Segment {
                    name: text.clone(),
                    path: text,
                });
            }
            Component::RootDir => {
                built.push(MAIN_SEPARATOR.to_string());
                if segments.is_empty() {
                    // Unix root is its own segment; a Windows root is already
                    // part of the drive prefix.
                    segments.push(Segment {
                        name: MAIN_SEPARATOR.to_string(),
                        path: MAIN_SEPARATOR.to_string(),
                    });
                }
            }
            Component::Normal(part) => {
                built.push(part);
                segments.push(Segment {
                    name: part.to_string_lossy().into_owned(),
                    path: built.to_string_lossy().into_owned(),
                });
            }
            Component::CurDir => {}
            Component::ParentDir => {
                built.push("..");
                segments.push(Segment {
                    name: "..".into(),
                    path: built.to_string_lossy().into_owned(),
                });
            }
        }
    }

    let meta = std::fs::metadata(path).ok();
    PathInfo {
        path: path.to_string_lossy().into_owned(),
        name: display_name(path),
        parent: path.parent().map(|p| {
            let text = p.to_string_lossy().into_owned();
            if text.is_empty() { MAIN_SEPARATOR.to_string() } else { text }
        }),
        segments,
        exists: meta.is_some(),
        is_dir: meta.map(|m| m.is_dir()).unwrap_or(false),
    }
}

/// Last-resort display name for a path: its final component, or the whole path
/// when it has none (a filesystem root).
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

