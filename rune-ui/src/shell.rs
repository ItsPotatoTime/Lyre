//! The stable subset of the old app's shell context menu.
//!
//! Other programs register command-line *verbs* in the registry ("Open in
//! Windows Terminal", "Upload with ShareX"); those are cheap to read and safe to
//! run through the shell. This module reads them and opens files with their
//! default app or the system "Open with" picker. It deliberately does NOT load
//! in-process `IContextMenu` handler DLLs: a broken extension could take the
//! window down, and there is no way to ask a DLL what it would draw without
//! loading it. Everything links to system libraries by name, so no extra crate.

/// One registered command verb, as the context menu and customize list show it.
#[derive(Clone)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub struct Verb {
    /// Stable identity, remembered when the person turns the item on or off.
    pub id: String,
    pub label: String,
    /// The program that registered it, for the customize list.
    pub source: String,
    /// The registry key name the shell invokes.
    pub verb: String,
    /// A folder-background command, run against the folder rather than an item.
    pub background: bool,
}

#[cfg(target_os = "windows")]
mod win {
    use super::Verb;
    use std::collections::HashSet;
    use std::ffi::c_void;
    use std::path::Path;

    const HKEY_CLASSES_ROOT: usize = 0xFFFF_FFFF_8000_0000;
    const KEY_READ: u32 = 0x0002_0019;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const SW_SHOWNORMAL: i32 = 1;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegOpenKeyExW(
            key: usize,
            sub_key: *const u16,
            options: u32,
            access: u32,
            result: *mut usize,
        ) -> i32;
        fn RegEnumKeyExW(
            key: usize,
            index: u32,
            name: *mut u16,
            name_len: *mut u32,
            reserved: *mut u32,
            class: *mut u16,
            class_len: *mut u32,
            written: *mut u64,
        ) -> i32;
        fn RegQueryValueExW(
            key: usize,
            name: *const u16,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
        fn RegCloseKey(key: usize) -> i32;
    }
    #[link(name = "shlwapi")]
    unsafe extern "system" {
        fn SHLoadIndirectString(
            source: *const u16,
            out: *mut u16,
            out_len: u32,
            reserved: *mut *mut c_void,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ExpandEnvironmentStringsW(source: *const u16, out: *mut u16, out_len: u32) -> u32;
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteExW(info: *mut ExecuteInfo) -> i32;
    }

    #[repr(C)]
    struct ExecuteInfo {
        size: u32,
        mask: u32,
        window: usize,
        verb: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
        instance: usize,
        id_list: *mut c_void,
        class: *const u16,
        class_key: usize,
        hot_key: u32,
        icon: usize,
        process: usize,
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A resource reference (`@%SystemRoot%\system32\shell32.dll,-8964`) resolved
    /// to the text it stands for; most shell and installer labels are stored so.
    fn indirect(text: &str) -> String {
        if !text.starts_with('@') {
            return text.to_string();
        }
        let source = wide(text);
        let mut buffer = vec![0u16; 512];
        let status = unsafe {
            SHLoadIndirectString(
                source.as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                std::ptr::null_mut(),
            )
        };
        if status < 0 {
            return text.to_string();
        }
        let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
        String::from_utf16_lossy(&buffer[..end])
    }

    fn expand(text: &str) -> String {
        let source = wide(text);
        let mut buffer = vec![0u16; 1024];
        let written =
            unsafe { ExpandEnvironmentStringsW(source.as_ptr(), buffer.as_mut_ptr(), 1024) };
        if written == 0 || written as usize > 1024 {
            return text.to_string();
        }
        String::from_utf16_lossy(&buffer[..written as usize - 1])
    }

    /// `&` marks a menu access key and is not part of the text; `&&` is a literal.
    fn clean(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '&' {
                if chars.peek() == Some(&'&') {
                    out.push('&');
                    chars.next();
                }
                continue;
            }
            out.push(ch);
        }
        out.trim().to_string()
    }

    /// One string value under `subkey`, with the shell's two indirections resolved.
    fn value(subkey: &str, name: Option<&str>) -> Option<String> {
        let path = wide(subkey);
        let mut key: usize = 0;
        if unsafe { RegOpenKeyExW(HKEY_CLASSES_ROOT, path.as_ptr(), 0, KEY_READ, &mut key) } != 0 {
            return None;
        }
        let name_wide = name.map(wide);
        let name_ptr = match &name_wide {
            Some(units) => units.as_ptr(),
            None => std::ptr::null(),
        };
        let mut kind = 0u32;
        let mut len = 0u32;
        let sized = unsafe {
            RegQueryValueExW(
                key,
                name_ptr,
                std::ptr::null_mut(),
                &mut kind,
                std::ptr::null_mut(),
                &mut len,
            )
        };
        if sized != 0 || len == 0 {
            unsafe { RegCloseKey(key) };
            return None;
        }
        let mut bytes = vec![0u8; len as usize];
        let read = unsafe {
            RegQueryValueExW(
                key,
                name_ptr,
                std::ptr::null_mut(),
                &mut kind,
                bytes.as_mut_ptr(),
                &mut len,
            )
        };
        unsafe { RegCloseKey(key) };
        if read != 0 || (kind != REG_SZ && kind != REG_EXPAND_SZ) {
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
        let text = String::from_utf16(&units).ok()?;
        let text = if kind == REG_EXPAND_SZ {
            expand(&text)
        } else {
            text
        };
        let text = indirect(&text);
        let text = text.trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    }

    fn key_exists(subkey: &str) -> bool {
        let path = wide(subkey);
        let mut key: usize = 0;
        if unsafe { RegOpenKeyExW(HKEY_CLASSES_ROOT, path.as_ptr(), 0, KEY_READ, &mut key) } != 0 {
            return false;
        }
        unsafe { RegCloseKey(key) };
        true
    }

    fn subkeys(subkey: &str) -> Vec<String> {
        let path = wide(subkey);
        let mut key: usize = 0;
        if unsafe { RegOpenKeyExW(HKEY_CLASSES_ROOT, path.as_ptr(), 0, KEY_READ, &mut key) } != 0 {
            return Vec::new();
        }
        let mut names = Vec::new();
        let mut index = 0u32;
        loop {
            let mut buffer = vec![0u16; 512];
            let mut len = buffer.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    buffer.as_mut_ptr(),
                    &mut len,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status != 0 {
                break;
            }
            names.push(String::from_utf16_lossy(&buffer[..len as usize]));
            index += 1;
        }
        unsafe { RegCloseKey(key) };
        names
    }

    /// How a registration names itself: `MUIVerb`, then the key's default value,
    /// then the key name with underscores spaced.
    fn label(key: &str, fallback: &str) -> Option<String> {
        let text = value(key, Some("MUIVerb"))
            .or_else(|| value(key, None))
            .unwrap_or_else(|| fallback.replace('_', " "));
        let text = clean(&text);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// The program a verb belongs to, from the executable in its command. Windows'
    /// own verbs are credited to Windows; anything else to its executable stem.
    fn source_of(key: &str) -> String {
        const WINDOWS: &str = "Windows";
        let Some(command) = value(&format!("{key}\\command"), None) else {
            return WINDOWS.to_string();
        };
        let command = expand(&command);
        let rest = command.trim_start();
        let executable = if let Some(quoted) = rest.strip_prefix('"') {
            quoted.split('"').next().unwrap_or(quoted)
        } else {
            rest.split_whitespace().next().unwrap_or(rest)
        };
        let lower = executable.to_ascii_lowercase();
        if !lower.ends_with(".exe") && !lower.ends_with(".com") {
            return WINDOWS.to_string();
        }
        let Some(stem) = Path::new(executable).file_stem().and_then(|s| s.to_str()) else {
            return WINDOWS.to_string();
        };
        let root = std::env::var("SystemRoot").unwrap_or_default();
        let system = !root.is_empty()
            && executable
                .to_ascii_lowercase()
                .starts_with(&root.to_ascii_lowercase());
        if system
            || ["rundll32", "explorer", "shell32", "cmd"]
                .iter()
                .any(|known| known.eq_ignore_ascii_case(stem))
        {
            return WINDOWS.to_string();
        }
        stem.to_string()
    }

    /// Which registry classes a target's menu reads, most specific first.
    fn classes(origin: &str, path: Option<&Path>) -> Vec<String> {
        let mut roots = Vec::new();
        match origin {
            "background" => roots.push("Directory\\Background".to_string()),
            "folder" => {
                roots.push("AllFilesystemObjects".to_string());
                roots.push("Directory".to_string());
                roots.push("Folder".to_string());
                let depth = path.map_or(0, |target| target.components().count());
                if depth <= 2 {
                    roots.push("Drive".to_string());
                }
            }
            _ => {
                roots.push("AllFilesystemObjects".to_string());
                roots.push("*".to_string());
                if let Some(target) = path {
                    if let Some(extension) = target.extension().and_then(|ext| ext.to_str()) {
                        let ext_key = format!(".{extension}");
                        if let Some(prog_id) = value(&ext_key, None) {
                            roots.insert(2, prog_id);
                        }
                        roots.insert(2, format!("SystemFileAssociations\\{ext_key}"));
                        roots.insert(2, ext_key);
                    }
                }
            }
        }
        roots
    }

    /// The runnable command verbs registered for a target, from the registry alone.
    pub fn verbs(path: Option<&str>, is_dir: bool, background: bool) -> Vec<Verb> {
        let origin = if background {
            "background"
        } else if is_dir {
            "folder"
        } else {
            "file"
        };
        let target = path.map(Path::new);
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        for class in classes(origin, target) {
            for name in subkeys(&format!("{class}\\shell")) {
                let key = format!("{class}\\shell\\{name}");
                // LegacyDisable retires an item; Extended hides it behind Shift.
                if key_exists(&format!("{key}\\LegacyDisable"))
                    || key_exists(&format!("{key}\\Extended"))
                {
                    continue;
                }
                let Some(text) = label(&key, &name) else {
                    continue;
                };
                // Neither a command nor a delegate means the item does nothing.
                if !key_exists(&format!("{key}\\command"))
                    && !key_exists(&format!("{key}\\DelegateExecute"))
                {
                    continue;
                }
                let id = format!("verb|{origin}|{name}");
                if !seen.insert(id.clone()) {
                    continue;
                }
                found.push(Verb {
                    id,
                    label: text,
                    source: source_of(&key),
                    verb: name,
                    background,
                });
            }
        }
        found
    }

    fn execute(verb: &str, file: &str, parameters: Option<&str>) -> Result<(), String> {
        let verb_w = wide(verb);
        let file_w = wide(file);
        let params_w = parameters.map(wide);
        let mut info: ExecuteInfo = unsafe { std::mem::zeroed() };
        info.size = std::mem::size_of::<ExecuteInfo>() as u32;
        info.verb = verb_w.as_ptr();
        info.file = file_w.as_ptr();
        info.parameters = params_w.as_ref().map_or(std::ptr::null(), |p| p.as_ptr());
        info.show = SW_SHOWNORMAL;
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            return Err("Windows could not run that item".into());
        }
        Ok(())
    }

    /// A background verb is a command line written for a folder: `%V`/`%1` take
    /// the folder's place, and the result runs through the command interpreter.
    fn run_background(verb: &str, folder: &str) -> Result<(), String> {
        let key = format!("Directory\\Background\\shell\\{verb}\\command");
        let template =
            value(&key, None).ok_or_else(|| format!("{verb} no longer has a command to run."))?;
        let quoted = format!("\"{folder}\"");
        let mut line = template;
        for token in ["%V", "%1"] {
            let surround = format!("\"{token}\"");
            if line.contains(&surround) {
                line = line.replace(&surround, &quoted);
            } else {
                line = line.replace(token, &quoted);
            }
        }
        execute("open", "cmd.exe", Some(&format!("/c {line}")))
    }

    pub fn run(verb: &Verb, path: &str) -> Result<(), String> {
        if verb.background {
            run_background(&verb.verb, path)
        } else {
            execute(&verb.verb, path, None)
        }
    }

    pub fn open_default(path: &str) -> Result<(), String> {
        execute("open", path, None)
    }

    pub fn open_with_picker(path: &str) -> Result<(), String> {
        execute("openas", path, None)
    }
}

#[cfg(target_os = "windows")]
pub use win::{open_default, open_with_picker, run, verbs};

#[cfg(not(target_os = "windows"))]
mod other {
    use super::Verb;

    pub fn verbs(_path: Option<&str>, _is_dir: bool, _background: bool) -> Vec<Verb> {
        Vec::new()
    }
    pub fn run(_verb: &Verb, _path: &str) -> Result<(), String> {
        Err("System menu items need Windows.".into())
    }
    pub fn open_default(_path: &str) -> Result<(), String> {
        Err("Opening files needs Windows.".into())
    }
    pub fn open_with_picker(_path: &str) -> Result<(), String> {
        Err("Open with needs Windows.".into())
    }
}

#[cfg(not(target_os = "windows"))]
pub use other::{open_default, open_with_picker, run, verbs};