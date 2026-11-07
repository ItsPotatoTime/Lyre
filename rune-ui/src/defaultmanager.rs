//! Becoming the file manager the desktop launches.
//!
//! Linux has a defined answer: a `.desktop` entry declaring `inode/directory`
//! in its MimeType, registered as the handler for that type with `xdg-mime`.
//! Windows routes a folder open through shell verbs (`Directory`, `Folder`,
//! `Drive`); overriding each command per user is what every third-party file
//! manager does. Which surface consults which verb was measured on Windows 11,
//! not assumed. Win+E is not one of them (the shell spawns explorer.exe by full
//! path), so it is taken at the keyboard instead — a separate module, later.
//! macOS offers neither: Finder is the shell and no verb is consulted.
//!
//! Ported from the old Tauri backend; the registry round trip is exercised by an
//! ignored test so a normal test run never decides the user's folder handler.

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

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    fn applications_dir() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
        Some(base.join("applications"))
    }

    fn desktop_file() -> Option<PathBuf> {
        Some(applications_dir()?.join("lyre.desktop"))
    }

    fn current_handler() -> Option<String> {
        let output = Command::new("xdg-mime").args(["query", "default", "inode/directory"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }

    /// Quote an argument for an Exec line (not a shell, but its own rules).
    pub(super) fn quote(value: &str) -> String {
        if !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\') {
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

    pub fn restore() -> Result<ManagerState, String> {
        if let Some(path) = desktop_file() {
            let _ = std::fs::remove_file(path);
        }
        let _ = Command::new("xdg-mime").args(["default", "", "inode/directory"]).status();
        Ok(state())
    }

    pub fn make_default() -> Result<ManagerState, String> {
        let dir = applications_dir().ok_or_else(|| "Could not find where this user's applications live.".to_string())?;
        std::fs::create_dir_all(&dir).map_err(|err| format!("Could not create {}: {err}", dir.display()))?;
        let exe = std::env::current_exe().map_err(|err| format!("Could not locate this application: {err}"))?;
        let exe = exe.to_string_lossy();
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
        std::fs::write(&path, entry).map_err(|err| format!("Could not write {}: {err}", path.display()))?;
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
    use serde::Deserialize;
    use std::ffi::c_void;
    use std::path::PathBuf;

    /// Every key the shell consults when a folder is opened, measured on Windows 11.
    const COMMAND_KEYS: [&str; 7] = [
        r"Software\Classes\Directory\shell\open\command",
        r"Software\Classes\Directory\shell\explore\command",
        r"Software\Classes\Folder\shell\open\command",
        r"Software\Classes\Folder\shell\explore\command",
        r"Software\Classes\Drive\shell\open\command",
        r"Software\Classes\Drive\shell\explore\command",
        r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command",
    ];

    const VERB_KEYS: [(&str, &str); 3] = [
        (r"Software\Classes\Directory\shell", "open"),
        (r"Software\Classes\Folder\shell", "open"),
        (r"Software\Classes\Drive\shell", "open"),
    ];

    const LEGACY_PROG_ID: &str = r"Software\Classes\Lyre.Open";
    const LEGACY_VERB: &str = "Lyre.Open";

    const REG_SZ: u32 = 1;
    const KEY_READ: u32 = 0x0002_0019;
    const KEY_SET_VALUE: u32 = 0x0002;
    const REG_OPTION_NON_VOLATILE: u32 = 0;
    const ERROR_SUCCESS: i32 = 0;

    #[cfg(target_pointer_width = "64")]
    const HKEY_CURRENT_USER: usize = 0xFFFF_FFFF_8000_0001;
    #[cfg(target_pointer_width = "32")]
    const HKEY_CURRENT_USER: usize = 0x8000_0001;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegCreateKeyExW(
            key: usize,
            sub_key: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            access: u32,
            security: *mut c_void,
            result: *mut usize,
            disposition: *mut u32,
        ) -> i32;
        fn RegOpenKeyExW(key: usize, sub_key: *const u16, options: u32, access: u32, result: *mut usize) -> i32;
        fn RegQueryValueExW(key: usize, name: *const u16, reserved: *mut u32, kind: *mut u32, data: *mut u8, len: *mut u32) -> i32;
        fn RegSetValueExW(key: usize, name: *const u16, reserved: u32, kind: u32, data: *const u8, len: u32) -> i32;
        fn RegDeleteValueW(key: usize, name: *const u16) -> i32;
        fn RegDeleteKeyW(key: usize, sub_key: *const u16) -> i32;
        fn RegCloseKey(key: usize) -> i32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn name_pointer(wide_name: Option<&Vec<u16>>) -> *const u16 {
        match wide_name {
            Some(name) => name.as_ptr(),
            None => std::ptr::null(),
        }
    }

    fn read_value(path: &str, name: Option<&str>) -> Option<RegValue> {
        let path_wide = wide(path);
        let mut key: usize = 0;
        let opened = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path_wide.as_ptr(), 0, KEY_READ, &mut key) };
        if opened != ERROR_SUCCESS {
            return None;
        }
        let name_wide = name.map(wide);
        let name_ptr = name_pointer(name_wide.as_ref());
        let mut kind = REG_SZ;
        let mut len = 0u32;
        let sized = unsafe { RegQueryValueExW(key, name_ptr, std::ptr::null_mut(), &mut kind, std::ptr::null_mut(), &mut len) };
        if sized != ERROR_SUCCESS || len == 0 {
            unsafe { RegCloseKey(key) };
            return None;
        }
        let mut bytes = vec![0u8; len as usize];
        let read = unsafe { RegQueryValueExW(key, name_ptr, std::ptr::null_mut(), &mut kind, bytes.as_mut_ptr(), &mut len) };
        unsafe { RegCloseKey(key) };
        if read != ERROR_SUCCESS {
            return None;
        }
        bytes.truncate(len as usize);
        let mut units: Vec<u16> = Vec::with_capacity(bytes.len() / 2);
        let mut i = 0;
        while i + 1 < bytes.len() {
            units.push(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
            i += 2;
        }
        while units.last() == Some(&0) {
            units.pop();
        }
        String::from_utf16(&units).ok().map(|text| RegValue { kind, text })
    }

    fn write_value(path: &str, name: Option<&str>, value: &RegValue) -> Result<(), String> {
        let path_wide = wide(path);
        let mut key: usize = 0;
        let mut disposition = 0u32;
        let created = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path_wide.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | KEY_READ,
                std::ptr::null_mut(),
                &mut key,
                &mut disposition,
            )
        };
        if created != ERROR_SUCCESS {
            return Err(format!("Could not open HKEY_CURRENT_USER\\{path} (code {created})."));
        }
        let name_wide = name.map(wide);
        let name_ptr = name_pointer(name_wide.as_ref());
        let units: Vec<u16> = value.text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        let written = unsafe { RegSetValueExW(key, name_ptr, 0, value.kind, bytes.as_ptr(), bytes.len() as u32) };
        unsafe { RegCloseKey(key) };
        if written != ERROR_SUCCESS {
            return Err(format!("Could not write HKEY_CURRENT_USER\\{path} (code {written})."));
        }
        Ok(())
    }

    fn delete_value(path: &str, name: Option<&str>) {
        let path_wide = wide(path);
        let mut key: usize = 0;
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path_wide.as_ptr(), 0, KEY_SET_VALUE, &mut key) } != ERROR_SUCCESS {
            return;
        }
        let name_wide = name.map(wide);
        let name_ptr = name_pointer(name_wide.as_ref());
        unsafe {
            RegDeleteValueW(key, name_ptr);
            RegCloseKey(key);
        }
    }

    fn remove_empty_key(path: &str) {
        let mut current = path.to_string();
        loop {
            let current_wide = wide(&current);
            if unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, current_wide.as_ptr()) } != ERROR_SUCCESS {
                return;
            }
            let Some(index) = current.rfind('\\') else { return };
            current.truncate(index);
            if !current.contains(r"Software\Classes\") {
                return;
            }
        }
    }

    fn backup_path() -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .unwrap_or_else(std::env::temp_dir);
        base.join("Lyre").join("folder-handler.json")
    }

    fn load_backup() -> Option<Backup> {
        let text = std::fs::read_to_string(backup_path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save_backup(backup: &Backup) -> Result<(), String> {
        let path = backup_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("Could not create {}: {err}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(backup).map_err(|err| format!("Could not record the previous handler: {err}"))?;
        std::fs::write(&path, text).map_err(|err| format!("Could not write {}: {err}", path.display()))
    }

    fn wanted(exe: &str) -> Vec<(String, Option<String>, RegValue)> {
        let command = RegValue::string(handler_command(exe));
        let mut values = Vec::with_capacity(COMMAND_KEYS.len() * 2 + VERB_KEYS.len());
        for key in COMMAND_KEYS {
            values.push((key.to_string(), None, command.clone()));
            values.push((key.to_string(), Some("DelegateExecute".to_string()), RegValue::string("")));
        }
        for (key, verb) in VERB_KEYS {
            values.push((key.to_string(), None, RegValue::string(verb)));
        }
        values
    }

    fn previous_value(path: &str, name: Option<&str>) -> Option<RegValue> {
        read_value(path, name).filter(|value| value.text != LEGACY_VERB)
    }

    pub fn state() -> ManagerState {
        let ours = std::env::current_exe().map(|path| handler_command(&path.to_string_lossy())).ok();
        let installed = match &ours {
            Some(command) => COMMAND_KEYS.iter().all(|key| read_value(key, None).map(|value| value.text == *command).unwrap_or(false)),
            None => false,
        };
        ManagerState {
            current: read_value(COMMAND_KEYS[0], None).map(|value| value.text),
            is_default: installed,
            unsupported: None,
            desktop_file: None,
            restore_supported: true,
        }
    }

    #[allow(dead_code)]
    pub fn is_current_handler() -> bool {
        let Ok(exe) = std::env::current_exe() else { return false };
        let ours = handler_command(&exe.to_string_lossy());
        read_value(COMMAND_KEYS[0], None).map(|value| value.text == ours).unwrap_or(false)
    }

    pub fn make_default() -> Result<ManagerState, String> {
        let exe = std::env::current_exe().map_err(|err| format!("Could not locate this application: {err}"))?;
        let exe = exe.to_string_lossy().into_owned();
        let values = wanted(&exe);
        let mut backup = load_backup().unwrap_or_default();
        for (path, name, owned) in &values {
            match backup.entries.iter_mut().find(|entry| &entry.path == path && entry.name == *name) {
                Some(entry) => entry.owned = owned.clone(),
                None => backup.entries.push(Entry {
                    path: path.clone(),
                    name: name.clone(),
                    owned: owned.clone(),
                    before: previous_value(path, name.as_deref()),
                }),
            }
        }
        save_backup(&backup)?;
        for (path, name, value) in &values {
            write_value(path, name.as_deref(), value)?;
        }
        clear_legacy();
        Ok(state())
    }

    pub fn restore() -> Result<ManagerState, String> {
        let entries = match load_backup() {
            Some(backup) if !backup.entries.is_empty() => backup.entries,
            _ => match std::env::current_exe() {
                Ok(exe) => wanted(&exe.to_string_lossy())
                    .into_iter()
                    .map(|(path, name, owned)| Entry { path, name, owned, before: None })
                    .collect(),
                Err(_) => Vec::new(),
            },
        };
        for entry in entries {
            let ours = read_value(&entry.path, entry.name.as_deref()).map(|value| value.text == entry.owned.text).unwrap_or(false);
            if !ours {
                continue;
            }
            match entry.before {
                Some(before) => write_value(&entry.path, entry.name.as_deref(), &before)?,
                None => {
                    delete_value(&entry.path, entry.name.as_deref());
                    remove_empty_key(&entry.path);
                }
            }
        }
        clear_legacy();
        let _ = std::fs::remove_file(backup_path());
        Ok(state())
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct RegValue {
        kind: u32,
        text: String,
    }

    impl RegValue {
        fn string(text: impl Into<String>) -> Self {
            RegValue { kind: REG_SZ, text: text.into() }
        }
    }

    #[derive(Serialize, Deserialize)]
    struct Entry {
        path: String,
        name: Option<String>,
        owned: RegValue,
        before: Option<RegValue>,
    }

    #[derive(Default, Serialize, Deserialize)]
    struct Backup {
        entries: Vec<Entry>,
    }

    fn clear_legacy() {
        let command = format!(r"{LEGACY_PROG_ID}\command");
        delete_value(&command, None);
        remove_empty_key(&command);
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
        Err(state().unsupported.unwrap_or_else(|| "Not supported on this platform.".into()))
    }

    pub fn restore() -> Result<ManagerState, String> {
        Err(state().unsupported.unwrap_or_else(|| "Not supported on this platform.".into()))
    }
}

/// The command line the shell runs when a folder is opened.
#[allow(dead_code)]
fn handler_command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

/// Whether this application is the folder handler the shell would use now.
/// Read by the Win+E hook (a later module); kept here so the handler state lives
/// in one place.
#[allow(dead_code)]
pub fn is_current_handler() -> bool {
    #[cfg(windows)]
    {
        platform::is_current_handler()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The folder this process was asked to open, if it was launched with one.
pub fn startup_path() -> Option<String> {
    std::env::args_os().skip(1).find_map(|arg| openable_folder(&arg.to_string_lossy()))
}

fn openable_folder(arg: &str) -> Option<String> {
    if arg.starts_with('-') {
        return None;
    }
    let candidate = match arg.strip_prefix("file://") {
        Some(rest) => rest.replace("%20", " "),
        None => arg.to_string(),
    };
    let path = std::path::Path::new(&candidate);
    if path.is_dir() {
        return Some(candidate);
    }
    if path.is_file() {
        return path.parent().map(|parent| parent.to_string_lossy().into_owned());
    }
    None
}

#[allow(dead_code)]
fn needs_explorer(args: &[String]) -> bool {
    args.iter().any(|arg| {
        let lower = arg.to_ascii_lowercase();
        lower.starts_with("/select")
            || lower.starts_with("/e,")
            || lower.starts_with("/root,")
            || lower.starts_with("/n,")
            || lower.starts_with("shell:")
            || lower.starts_with("::")
    })
}

/// Hand a launch the window cannot serve to the real Explorer; true means exit.
#[cfg(windows)]
pub fn forward_unhostable_launch() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let args: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    if !needs_explorer(&args) {
        return false;
    }
    let explorer = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows")).join("explorer.exe");
    std::process::Command::new(explorer).args(&args).creation_flags(CREATE_NO_WINDOW).spawn().is_ok()
}

#[cfg(not(windows))]
pub fn forward_unhostable_launch() -> bool {
    false
}

#[cfg(target_os = "linux")]
pub use linux::{make_default, restore, state};

#[cfg(not(target_os = "linux"))]
pub use platform::{make_default, restore, state};

#[cfg(all(test, target_os = "linux"))]
fn quote_for_test(value: &str) -> String {
    linux::quote(value)
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;

    #[test]
    fn the_exec_quoting_survives_a_path_with_spaces() {
        assert_eq!(quote_for_test("/usr/bin/lyre"), "/usr/bin/lyre");
        assert_eq!(quote_for_test("/home/a b/lyre"), "\"/home/a b/lyre\"");
        assert_eq!(quote_for_test("/home/a\"b/lyre"), "\"/home/a\\\"b/lyre\"");
    }
}

#[cfg(test)]
mod handler_tests {
    use super::*;

    #[test]
    fn the_handler_command_quotes_the_path_and_passes_the_folder() {
        assert_eq!(handler_command(r"C:\Program Files\Lyre\lyre.exe"), "\"C:\\Program Files\\Lyre\\lyre.exe\" \"%1\"");
    }

    #[test]
    fn launches_only_the_shell_can_serve_are_handed_over() {
        let args = |list: &[&str]| list.iter().map(|item| item.to_string()).collect::<Vec<_>>();
        assert!(needs_explorer(&args(&[r"/select,C:\Windows\notepad.exe"])));
        assert!(needs_explorer(&args(&["::{52205FD8-5DFB-447D-801A-D0B52F2E83E1}"])));
        assert!(needs_explorer(&args(&["shell:RecycleBinFolder"])));
        assert!(!needs_explorer(&args(&[r"C:\Windows"])));
        assert!(!needs_explorer(&args(&[])));
    }

    #[test]
    fn a_folder_argument_opens_itself_and_a_file_argument_opens_its_folder() {
        let scratch = std::env::temp_dir().join("lyre-handler-openable");
        std::fs::create_dir_all(&scratch).expect("scratch folder");
        let file = scratch.join("note.txt");
        std::fs::write(&file, "x").expect("scratch file");
        let folder = scratch.to_string_lossy().into_owned();
        assert_eq!(openable_folder(&folder), Some(folder.clone()));
        assert_eq!(openable_folder(&file.to_string_lossy()), Some(folder));
        assert_eq!(openable_folder("--path"), None);
        assert_eq!(openable_folder(&scratch.join("gone").to_string_lossy()), None);
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

/// The registry round trip runs against the real thing; ignored by default so a
/// test run never decides the user's folder handler. Run with --ignored.
#[cfg(all(test, target_os = "windows"))]
mod registry_tests {
    use super::*;

    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = super::restore();
        }
    }

    #[test]
    #[ignore = "writes HKEY_CURRENT_USER; run with --ignored to exercise the registry"]
    fn the_handler_round_trips_through_the_registry() {
        let before = state();
        let guard = Restore;
        let installed = make_default().expect("make default");
        assert!(installed.is_default, "every command key should point here");
        let restored = super::restore().expect("restore");
        assert!(!restored.is_default, "nothing should point here after a restore");
        assert_eq!(restored.current, before.current, "the previous handler should be back");
        std::mem::forget(guard);
    }
}