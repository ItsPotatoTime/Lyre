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
    /// The thing this volume actually is: `/dev/sda1` on Linux, `C:` on
    /// Windows. Shown small under the name, because a volume label alone does
    /// not tell two similar drives apart.
    pub detail: Option<String>,
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
        detail: None,
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
                detail: None,
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
    use std::collections::HashMap;

    let disks = Disks::new_with_refreshed_list();
    let labels = volume_labels();

    // One row per volume, not per mount. On a btrfs system the same device is
    // mounted at /, /home, /root, /srv and more, and listing each of those as a
    // separate drive is simply wrong. The shortest mount path wins, which is
    // the one a person means when they say "the disk".
    let mut best: HashMap<String, Place> = HashMap::new();
    for disk in &disks {
        let mount = disk.mount_point();
        if mount.as_os_str().is_empty() {
            continue;
        }
        // Pseudo and image filesystems report no usable space and are not
        // storage: overlays, AppImage squashfs mounts, fuse and friends.
        if disk.total_space() == 0 || is_pseudo_filesystem(&disk.file_system().to_string_lossy()) {
            continue;
        }

        let mount_path = mount.to_string_lossy().into_owned();
        let key = volume_key(disk, &mount_path);
        let total = disk.total_space();
        let free = disk.available_space();

        let candidate = Place {
            name: volume_name(
                &key,
                &mount_path,
                total,
                &labels,
                &disk.name().to_string_lossy(),
            ),
            path: mount_path.clone(),
            kind: if disk.is_removable() { "removable" } else { "drive" },
            icon: "drive",
            total,
            free,
            detail: Some(volume_detail(&key, &mount_path)),
        };

        match best.get(&key) {
            Some(existing) if existing.path.len() <= mount_path.len() => {}
            _ => {
                best.insert(key, candidate);
            }
        }
    }

    let mut out: Vec<Place> = best.into_values().collect();
    // Fixed drives first, then by name, so the order does not shuffle between
    // the first and second enumeration.
    out.sort_by(|a, b| {
        (a.kind == "removable")
            .cmp(&(b.kind == "removable"))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// What makes two mounts the same volume.
#[cfg(target_os = "linux")]
fn volume_key(disk: &sysinfo::Disk, _mount: &str) -> String {
    disk.name().to_string_lossy().into_owned()
}

/// On Windows the name is a volume label, which repeats; the drive letter is
/// the identity.
#[cfg(not(target_os = "linux"))]
fn volume_key(_disk: &sysinfo::Disk, mount: &str) -> String {
    mount.to_string()
}

#[cfg(target_os = "linux")]
fn volume_detail(key: &str, _mount: &str) -> String {
    key.to_string()
}

#[cfg(not(target_os = "linux"))]
fn volume_detail(_key: &str, mount: &str) -> String {
    // "C:\" reads as "C:" everywhere else in the interface.
    mount.trim_end_matches(['\\', '/']).to_string()
}

#[cfg(target_os = "linux")]
fn volume_name(
    key: &str,
    mount: &str,
    total: u64,
    labels: &std::collections::HashMap<String, String>,
    _disk_label: &str,
) -> String {
    // The label is what a person calls the stick; the device path goes on the
    // smaller line underneath.
    if let Some(label) = labels.get(key) {
        return label.clone();
    }
    if mount == "/" {
        return "Filesystem".into();
    }
    format!("{} Volume", gib(total))
}

#[cfg(not(target_os = "linux"))]
fn volume_name(
    _key: &str,
    _mount: &str,
    _total: u64,
    _labels: &std::collections::HashMap<String, String>,
    disk_label: &str,
) -> String {
    // On Windows sysinfo's name is the volume label, which is what Explorer
    // shows above the drive letter.
    let label = disk_label.trim();
    if label.is_empty() {
        "Local Disk".into()
    } else {
        label.to_string()
    }
}

fn gib(bytes: u64) -> String {
    let value = bytes as f64 / 1_073_741_824.0;
    if value >= 100.0 {
        format!("{value:.0} GB")
    } else {
        format!("{value:.1} GB")
    }
}

/// Filesystems that are not storage.
fn is_pseudo_filesystem(name: &str) -> bool {
    matches!(
        name,
        "overlay"
            | "tmpfs"
            | "devtmpfs"
            | "devpts"
            | "proc"
            | "sysfs"
            | "cgroup"
            | "cgroup2"
            | "autofs"
            | "mqueue"
            | "hugetlbfs"
            | "debugfs"
            | "tracefs"
            | "securityfs"
            | "pstore"
            | "bpf"
            | "configfs"
            | "fusectl"
            | "binfmt_misc"
            | "nsfs"
            | "ramfs"
            | "efivarfs"
            | "squashfs"
    ) || name.starts_with("fuse")
}

/// Volume labels for each block device, read from the udev links rather than
/// shelling out to `blkid`.
///
/// `/dev/disk/by-label/SANDISK` is a symlink to `/dev/sda1`, so the label can be
/// recovered without a subprocess. Udev escapes a space as `\\x20`.
#[cfg(target_os = "linux")]
fn volume_labels() -> std::collections::HashMap<String, String> {
    let mut labels = std::collections::HashMap::new();
    let Ok(entries) = std::fs::read_dir("/dev/disk/by-label") else {
        return labels;
    };
    for entry in entries.flatten() {
        let Ok(resolved) = std::fs::canonicalize(entry.path()) else {
            continue;
        };
        let label = entry
            .file_name()
            .to_string_lossy()
            .replace("\\x20", " ")
            .replace("\\x27", "'")
            .replace("\\x5c", "\\");
        labels.insert(resolved.to_string_lossy().into_owned(), label);
    }
    labels
}

#[cfg(not(target_os = "linux"))]
fn volume_labels() -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
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



#[cfg(test)]
mod volume_tests {
    use super::*;

    /// The volume list must describe *volumes*, not mounts.
    ///
    /// A btrfs system mounts one device at /, /home, /root, /srv and more; the
    /// page used to list each of those as its own drive. Image and overlay
    /// filesystems used to appear as well, with no space at all.
    #[test]
    fn each_volume_appears_once_and_has_space() {
        let volumes: Vec<Place> = places()
            .into_iter()
            .filter(|place| place.kind != "folder")
            .collect();

        let mut seen = std::collections::HashSet::new();
        for volume in &volumes {
            assert!(
                seen.insert(volume.path.clone()),
                "{} is listed twice",
                volume.path
            );
            assert!(
                volume.total > 0,
                "{} has no size, so it is not storage",
                volume.path
            );
            assert!(
                volume.detail.is_some(),
                "{} has no device to show under its name",
                volume.path
            );
        }
    }

    /// Unlabelled volumes still need a name a person can read.
    #[test]
    fn an_unlabelled_volume_is_named_by_its_size() {
        let labels = std::collections::HashMap::new();
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                volume_name("/dev/sdz9", "/run/media/you/0A1B", 64_000_000_000, &labels, ""),
                "59.6 GB Volume"
            );
            assert_eq!(
                volume_name("/dev/sdz9", "/", 64_000_000_000, &labels, ""),
                "Filesystem"
            );
        }
    }
}
