//! The volumes This PC lists, and safe eject.
//!
//! Windows asks the kernel for the volume table, the volume label and the free
//! space, and ejects a removable device by locking and dismounting it first.
//! Every other platform lists its mount points as a working fallback; the free
//! space is a Windows-only read here. No extra crate: the Windows calls link to
//! kernel32 by name, as `shell` does.

use std::path::{Path, PathBuf};

use crate::format_size;

/// What kind of device a volume is, which decides whether it can be ejected.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    Fixed,
    Removable,
    Optical,
    Network,
    RamDisk,
}

/// One device This PC shows as a card.
#[derive(Clone)]
pub struct Volume {
    /// The root path, `C:\` on Windows.
    pub path: PathBuf,
    /// The volume label, empty when the device has none.
    pub label: String,
    pub kind: VolumeKind,
    pub free: u64,
    pub total: u64,
}

impl Volume {
    /// The drive letter with its colon (`C:`), for the title. A mount point
    /// with no drive letter falls back to its last component.
    pub fn letter(&self) -> String {
        let text = self.path.to_string_lossy();
        let trimmed = text.trim_end_matches(['\\', '/']);
        let bytes = trimmed.as_bytes();
        if bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return trimmed.to_string();
        }
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(trimmed.to_string())
    }

    /// "Local Disk (C:)", or the label when the device has one.
    pub fn title(&self) -> String {
        let letter = self.letter();
        if self.label.is_empty() {
            let name = match self.kind {
                VolumeKind::Fixed => "Local Disk",
                VolumeKind::Removable => "Removable Disk",
                VolumeKind::Optical => "CD Drive",
                VolumeKind::Network => "Network Drive",
                VolumeKind::RamDisk => "RAM Disk",
            };
            format!("{name} ({letter})")
        } else {
            format!("{} ({letter})", self.label)
        }
    }

    /// "1.55 TB free of 1.81 TB", or a plain admission when it is unknown.
    pub fn space_label(&self) -> String {
        if self.total == 0 {
            "Free space unavailable".to_string()
        } else {
            format!("{} free of {}", format_size(self.free), format_size(self.total))
        }
    }

    /// How full the volume is, 0 to 1, for the card's bar.
    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (1.0 - self.free as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }

    pub fn is_ejectable(&self) -> bool {
        matches!(self.kind, VolumeKind::Removable | VolumeKind::Optical)
    }
}

/// Every volume currently attached, in drive order.
pub fn enumerate() -> Vec<Volume> {
    #[cfg(target_os = "windows")]
    {
        win::enumerate()
    }
    #[cfg(not(target_os = "windows"))]
    {
        other::enumerate()
    }
}

/// Eject a removable volume. The device is locked and dismounted first so a
/// handle another program holds does not block the eject.
pub fn eject(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        win::eject(path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err("Safe eject needs Windows.".into())
    }
}

#[cfg(target_os = "windows")]
mod win {
    use super::{Volume, VolumeKind};
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};

    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;

    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const OPEN_EXISTING: u32 = 3;
    const FSCTL_LOCK_VOLUME: u32 = 0x0009_0018;
    const FSCTL_DISMOUNT_VOLUME: u32 = 0x0009_0020;
    const IOCTL_STORAGE_EJECT_MEDIA: u32 = 0x002D_4808;
    const INVALID_HANDLE_VALUE: usize = usize::MAX;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLogicalDrives() -> u32;
        fn GetDriveTypeW(path: *const u16) -> u32;
        fn GetVolumeInformationW(
            root: *const u16,
            label: *mut u16,
            label_len: u32,
            serial: *mut u32,
            max_component: *mut u32,
            flags: *mut u32,
            fs_name: *mut u16,
            fs_len: u32,
        ) -> i32;
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            free_available: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
        fn CreateFileW(
            path: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            creation: u32,
            flags: u32,
            template: usize,
        ) -> usize;
        fn DeviceIoControl(
            device: usize,
            code: u32,
            input: *mut c_void,
            input_len: u32,
            output: *mut c_void,
            output_len: u32,
            written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn CloseHandle(handle: usize) -> i32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn kind_of(root: &str) -> Option<VolumeKind> {
        let path = wide(root);
        match unsafe { GetDriveTypeW(path.as_ptr()) } {
            DRIVE_REMOVABLE => Some(VolumeKind::Removable),
            DRIVE_FIXED => Some(VolumeKind::Fixed),
            DRIVE_REMOTE => Some(VolumeKind::Network),
            DRIVE_CDROM => Some(VolumeKind::Optical),
            DRIVE_RAMDISK => Some(VolumeKind::RamDisk),
            _ => None,
        }
    }

    fn label_of(root: &str) -> String {
        let path = wide(root);
        let mut label = vec![0u16; 256];
        let ok = unsafe {
            GetVolumeInformationW(
                path.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        if ok == 0 {
            return String::new();
        }
        let end = label.iter().position(|unit| *unit == 0).unwrap_or(label.len());
        String::from_utf16_lossy(&label[..end])
    }

    fn space_of(root: &str) -> (u64, u64) {
        let path = wide(root);
        let mut free = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        let ok = unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut free, &mut total, &mut total_free) };
        if ok == 0 {
            (0, 0)
        } else {
            (free, total)
        }
    }

    pub fn enumerate() -> Vec<Volume> {
        let mut out = Vec::new();
        let mask = unsafe { GetLogicalDrives() };
        for bit in 0..26u32 {
            if mask & (1 << bit) == 0 {
                continue;
            }
            let letter = (b'A' + bit as u8) as char;
            let root = format!("{letter}:\\");
            let Some(kind) = kind_of(&root) else { continue };
            let label = label_of(&root);
            let (free, total) = space_of(&root);
            out.push(Volume { path: PathBuf::from(root), label, kind, free, total });
        }
        out
    }

    pub fn eject(path: &Path) -> Result<(), String> {
        let letter = path
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .split(':')
            .next()
            .unwrap_or("")
            .to_string();
        if letter.is_empty() {
            return Err("That is not a drive.".into());
        }
        let device = wide(&format!("\\\\.\\{letter}:"));
        let handle = unsafe {
            CreateFileW(
                device.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err("Windows would not open the device for ejecting.".into());
        }
        let control = |code| unsafe {
            DeviceIoControl(
                handle,
                code,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        // A busy handle is not fatal: the dismount still lets the eject through.
        let _ = control(FSCTL_LOCK_VOLUME);
        let _ = control(FSCTL_DISMOUNT_VOLUME);
        let status = control(IOCTL_STORAGE_EJECT_MEDIA);
        unsafe { CloseHandle(handle) };
        if status == 0 {
            return Err(
                "Windows refused to eject the device. Close any window using it and try again.".into(),
            );
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod other {
    use super::{Volume, VolumeKind};
    use std::path::PathBuf;

    pub fn enumerate() -> Vec<Volume> {
        // The root and whatever the desktop mounts: /Volumes on macOS, the
        // usual mount points elsewhere. The free space is not read here.
        let mut roots = vec![PathBuf::from("/")];
        for base in ["/Volumes", "/media", "/mnt"] {
            if let Ok(reader) = std::fs::read_dir(base) {
                for item in reader.flatten() {
                    let path = item.path();
                    if path.is_dir() {
                        roots.push(path);
                    }
                }
            }
        }
        roots.sort();
        roots.dedup();
        roots
            .into_iter()
            .map(|path| Volume {
                label: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path,
                kind: VolumeKind::Fixed,
                free: 0,
                total: 0,
            })
            .collect()
    }
}
