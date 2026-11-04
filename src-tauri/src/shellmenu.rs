//! The menu items other programs put in the system's own context menu.
//!
//! Windows lets an installer add itself in two ways: a *verb*, which is a
//! registry key holding a command line, and a *handler*, which is a DLL that
//! draws its own items when it is asked to. "Scan with Bitdefender" arrives as a
//! handler; "Open in Windows Terminal" arrives as a verb. Lyre paints its own
//! menu, so none of them can appear unless the registrations are read and the
//! person has asked for them.
//!
//! Reading is cheap: verbs are keys, and a handler is asked what it would offer
//! through `IContextMenu`, which is the same call the shell makes before drawing
//! its menu. Running one is handed back to the handler, because only it knows
//! what its items mean and only it can carry them out.
//!
//! A handler is third-party code and loads into this process, exactly as it does
//! into the shell. A broken one can take the window with it. There is no way to
//! ask a DLL what it would draw without loading it.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Stable identity. This is also what the interface remembers when an item
    /// is turned off, so it must not carry a label or a menu position.
    pub id: String,
    pub label: String,
    /// Which menu the item belongs to: a file, a folder, or the folder itself.
    pub origin: String,
    /// The program that registered it, for the customise list.
    pub source: String,
    /// `verb` for a registry command, `handler` for a menu extension.
    pub kind: String,
    /// Whether the item shows before anyone has chosen anything. The items Lyre
    /// already offers a better version of start off.
    pub default_on: bool,
    /// The items of a submenu. A parent with children is not itself invokable.
    pub children: Vec<Entry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    /// False where the platform has no such menu to read.
    pub supported: bool,
    /// Why there is nothing to read, in that platform's own terms.
    pub note: Option<String>,
    pub entries: Vec<Entry>,
    /// Registrations that were found but could not be read. An item that fails
    /// to load and one that does not exist look identical to the person looking
    /// for it, so the difference is kept and shown rather than swallowed.
    pub warnings: Vec<String>,
}

#[allow(dead_code)]
impl Catalog {
    fn unsupported(note: &str) -> Catalog {
        Catalog {
            supported: false,
            note: Some(note.to_string()),
            entries: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

// Each desktop has its own registry for these items. The shape of the answer is
// shared, so the window never needs to know which one it is running on, and an
// unsupported platform says what is missing there rather than naming Windows.

#[cfg(windows)]
pub fn catalog(path: Option<&str>, background: bool) -> Catalog {
    win::catalog(path, background)
}

#[cfg(windows)]
pub fn run(id: &str, paths: &[String]) -> Result<(), String> {
    win::run(id, paths)
}

#[cfg(target_os = "linux")]
pub fn catalog(_path: Option<&str>, _background: bool) -> Catalog {
    Catalog::unsupported(
        "Other programs offer their menu items through .desktop actions on Linux. \
         Lyre does not read those yet; its own menu items are all available.",
    )
}

#[cfg(target_os = "macos")]
pub fn catalog(_path: Option<&str>, _background: bool) -> Catalog {
    Catalog::unsupported(
        "Finder keeps its menu extensions to itself. Another application cannot \
         list the items other programs add to macOS menus.",
    )
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn catalog(_path: Option<&str>, _background: bool) -> Catalog {
    Catalog::unsupported("Other programs' menu items are not available on this system.")
}

#[cfg(not(windows))]
pub fn run(_id: &str, _paths: &[String]) -> Result<(), String> {
    Err("There are no system menu items to run on this platform.".to_string())
}

// ---------------------------------------------------------------------------
// Windows: the registry, then COM
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod win {
    use super::{Catalog, Entry};
    use std::collections::HashSet;
    use std::ffi::c_void;
    use std::path::Path;

    #[cfg(target_pointer_width = "64")]
    const HKEY_CLASSES_ROOT: usize = 0xFFFF_FFFF_8000_0000;
    #[cfg(target_pointer_width = "32")]
    const HKEY_CLASSES_ROOT: usize = 0x8000_0000;

    const KEY_READ: u32 = 0x0002_0019;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;

    /// The verbs the menu already offers a better version of. They stay in the
    /// catalogue so the customise list can show them, but they arrive off.
    const ALREADY_HANDLED: [&str; 16] = [
        "open",
        "opennew",
        "openas",
        "openwith",
        // The shell's "Open in new tab" and Lyre's own read the same, and two
        // identical rows in one menu is worse than one.
        "opennewtab",
        "print",
        "printto",
        "find",
        "runas",
        "explore",
        "properties",
        "cut",
        "copy",
        "paste",
        "delete",
        "rename",
    ];

    /// The menu handlers the menu already offers a better version of.
    const ALREADY_HANDLED_HANDLERS: [&str; 2] = ["Open With", "Open with"];

    // -----------------------------------------------------------------------
    // The registry
    // -----------------------------------------------------------------------
    #[link(name = "advapi32")]
    extern "system" {
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
    extern "system" {
        fn SHLoadIndirectString(
            source: *const u16,
            out: *mut u16,
            out_len: u32,
            reserved: *mut *mut c_void,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn ExpandEnvironmentStringsW(source: *const u16, out: *mut u16, out_len: u32) -> u32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A resource reference (`@%SystemRoot%\system32\shell32.dll,-8964`) turned
    /// into the text it stands for. Windows stores most of its own menu labels
    /// this way, and so do the installers that follow the rules.
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
        let written = unsafe { ExpandEnvironmentStringsW(source.as_ptr(), buffer.as_mut_ptr(), 1024) };
        if written == 0 || written as usize > 1024 {
            return text.to_string();
        }
        String::from_utf16_lossy(&buffer[..written as usize - 1])
    }

    /// The text of one value under `subkey`, with both forms of indirection the
    /// shell uses resolved. `None` for a missing key, a missing value, or a
    /// value that is not a string.
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
        let mut units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
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

    /// `{23170F69-40C1-278A-1000-000100020000}`, which is what a registration
    /// stores as the identity of the object to create.
    fn is_guid(text: &str) -> bool {
        parse_guid(text).is_some()
    }

    /// `&` marks the access key in a menu label and is not part of the text.
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

    /// How a registration names itself in a menu. `MUIVerb` is the modern way;
    /// the key's own default value is the older one, and the key name is the
    /// fallback the shell uses when neither is there.
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

    /// The program a verb belongs to, taken from the command it would run.
    /// Windows' own verbs are credited to Windows; anything else is named after
    /// the executable, because "Upload with ShareX" filed under Windows would be
    /// a lie in the customise list.
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
        // A command that is only `"%1"` names the item itself, not a program,
        // and says nothing about who registered it.
        let lower = executable.to_ascii_lowercase();
        if !lower.ends_with(".exe") && !lower.ends_with(".com") {
            return WINDOWS.to_string();
        }
        let Some(stem) = Path::new(executable)
            .file_stem()
            .and_then(|stem| stem.to_str())
        else {
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

    /// Which menu a set of registrations belongs to. This is also the invocation
    /// route: a background command is run against the folder, not against an
    /// item inside it.
    fn classes(origin: &str, path: Option<&Path>) -> Vec<String> {
        let mut roots = Vec::new();
        match origin {
            "background" => roots.push("Directory\\Background".to_string()),
            "folder" => {
                roots.push("AllFilesystemObjects".to_string());
                roots.push("Directory".to_string());
                roots.push("Folder".to_string());
                // Only a volume root has a volume menu.
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
                        // A ProgID covers the whole family a file type belongs
                        // to, and is registered less specifically than the
                        // extension itself.
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

    /// The verbs registered for a target, read from the registry alone.
    fn verbs(
        origin: &str,
        scope: &str,
        path: Option<&Path>,
        warnings: &mut Vec<String>,
    ) -> Vec<Entry> {
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        for class in classes(origin, path) {
            for name in subkeys(&format!("{class}\\shell")) {
                let key = format!("{class}\\shell\\{name}");
                // `LegacyDisable` is the shell's own way of saying "registered,
                // never shown", which is how a program retires an item without
                // losing the key. `Extended` items are the ones Explorer keeps
                // behind Shift.
                if key_exists(&format!("{key}\\LegacyDisable"))
                    || key_exists(&format!("{key}\\Extended"))
                {
                    continue;
                }
                let Some(text) = label(&key, &name) else {
                    continue;
                };
                // A key with neither a command nor a delegate does nothing when
                // chosen, so it is not offered. `SubCommands` is different: it
                // is a real cascading menu, and Windows 11 installers use it a
                // lot. Lyre cannot run one yet, but someone looking for it is
                // told it exists instead of finding nothing at all.
                if !key_exists(&format!("{key}\\command"))
                    && !key_exists(&format!("{key}\\DelegateExecute"))
                {
                    if value(&key, Some("SubCommands")).is_some()
                        || key_exists(&format!("{key}\\shell"))
                    {
                        warnings.push(format!(
                            "\"{text}\" is a cascading menu; Lyre cannot read its \
                             commands yet."
                        ));
                    }
                    continue;
                }
                let id = format!("verb|{scope}|{name}");
                if !seen.insert(id.clone()) {
                    continue;
                }
                found.push(Entry {
                    id,
                    label: text,
                    origin: origin.to_string(),
                    source: source_of(&key),
                    kind: "verb".to_string(),
                    default_on: !ALREADY_HANDLED
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(&name)),
                    children: Vec::new(),
                });
            }
        }
        found
    }

    /// The menu handlers registered for a target, as (class id, name to credit).
    fn handlers(origin: &str, path: Option<&Path>) -> Vec<(String, String)> {
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        for class in classes(origin, path) {
            for name in subkeys(&format!("{class}\\shellex\\ContextMenuHandlers")) {
                let key = format!("{class}\\shellex\\ContextMenuHandlers\\{name}");
                let Some(clsid) = value(&key, None) else {
                    continue;
                };
                if !is_guid(&clsid) || !seen.insert(clsid.clone()) {
                    continue;
                }
                // The class id's own description is the closest thing to the
                // product name; the handler's key name is the fallback, and a
                // bare class id the last resort.
                let source = value(&format!("CLSID\\{clsid}"), None)
                    .map(|text| clean(&text))
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| {
                        if is_guid(&name) {
                            clsid.clone()
                        } else {
                            name.clone()
                        }
                    });
                found.push((clsid, source));
            }
        }
        found
    }

    // -----------------------------------------------------------------------
    // COM
    // -----------------------------------------------------------------------
    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Guid {
        first: u32,
        second: u16,
        third: u16,
        rest: [u8; 8],
    }

    /// The shell's interfaces all share the `0002xxxx-0000-0000-C000-000000000046`
    /// tail, so only the first word differs.
    const fn shell_iid(first: u32) -> Guid {
        Guid {
            first,
            second: 0,
            third: 0,
            rest: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
        }
    }

    const IID_SHELL_EXT_INIT: Guid = shell_iid(0x0002_14E8);
    const IID_CONTEXT_MENU: Guid = shell_iid(0x0002_14E4);
    const IID_SHELL_FOLDER: Guid = shell_iid(0x0002_14E6);
    const IID_DATA_OBJECT: Guid = Guid {
        first: 0x0000_010E,
        second: 0,
        third: 0,
        rest: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
    };

    const IID_IUNKNOWN: Guid = shell_iid(0);
    /// A handler written before Vista takes the whole selection through
    /// `IShellExtInit`. Newer ones take a single item, and answer
    /// `QueryInterface` for one of these instead. Asking for only the first is
    /// how a handler can be installed, registered, and still draw nothing.
    /// The literals are checked against `parse_guid` in the tests.
    const IID_INIT_WITH_ITEM: Guid = Guid {
        first: 0x7F73_BE3F,
        second: 0xFB79,
        third: 0x493C,
        rest: [0xA6, 0xC7, 0x7E, 0xE1, 0x4E, 0x24, 0x58, 0x41],
    };
    const IID_INIT_WITH_FILE: Guid = Guid {
        first: 0xB7D1_4566,
        second: 0x0509,
        third: 0x4CCE,
        rest: [0xA7, 0x1F, 0x0A, 0x55, 0x42, 0x33, 0xBD, 0x9B],
    };
    const IID_ISHELL_ITEM: Guid = Guid {
        first: 0x4382_6D1E,
        second: 0xE718,
        third: 0x42EE,
        rest: [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE],
    };

    const CLSCTX_INPROC_SERVER: u32 = 0x1;
    const CLSCTX_LOCAL_SERVER: u32 = 0x4;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const RPC_E_CHANGED_MODE: i32 = -2_147_417_082; // 0x8001_0106
    const CMF_NORMAL: u32 = 0x0;
    const CMF_EXPLORE: u32 = 0x4;
    /// The menu id range a handler is given, which is also the shell's own limit.
    const MENU_ID_END: u32 = 0x7FFF;
    const SW_SHOWNORMAL: i32 = 1;

    const MIIM_ID: u32 = 0x2;
    const MIIM_SUBMENU: u32 = 0x4;
    const MIIM_STRING: u32 = 0x40;
    const MIIM_FTYPE: u32 = 0x100;
    const MFT_SEPARATOR: u32 = 0x800;

    #[repr(C)]
    struct Unknown {
        query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct ShellExtInit {
        base: Unknown,
        initialize: unsafe extern "system" fn(*mut c_void, *const c_void, *mut c_void, usize) -> i32,
    }

    #[repr(C)]
    struct ContextMenu {
        base: Unknown,
        query_context_menu: unsafe extern "system" fn(*mut c_void, usize, u32, u32, u32, u32) -> i32,
        invoke_command: unsafe extern "system" fn(*mut c_void, *const CmdInfo) -> i32,
        get_command_string: usize,
    }

    #[repr(C)]
    struct InitializeWithItem {
        base: Unknown,
        initialize: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32,
    }

    #[repr(C)]
    struct InitializeWithFile {
        base: Unknown,
        initialize: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> i32,
    }

    /// Only the calls up to `CreateViewObject` are reached here, but the
    /// placeholders are what keep that one at the right offset in the table.
    #[repr(C)]
    struct ShellFolder {
        base: Unknown,
        parse_display_name: usize,
        enum_objects: usize,
        bind_to_object:
            unsafe extern "system" fn(*mut c_void, *const c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32,
        bind_to_storage: usize,
        compare_ids: usize,
        create_view_object:
            unsafe extern "system" fn(*mut c_void, usize, *const Guid, *mut *mut c_void) -> i32,
    }

    /// `CMINVOKECOMMANDINFO`, not the `EX` form: the item is named by an integer
    /// offset packed into a pointer, which is the only way to ask for an item a
    /// handler drew itself rather than one registered under a name.
    #[repr(C)]
    struct CmdInfo {
        size: u32,
        mask: u32,
        window: usize,
        verb: *const u8,
        parameters: *const u8,
        directory: *const u8,
        show: i32,
        hot_key: u32,
        icon: usize,
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

    #[repr(C)]
    struct MenuItemInfo {
        size: u32,
        mask: u32,
        kind: u32,
        state: u32,
        id: u32,
        submenu: usize,
        checked: usize,
        unchecked: usize,
        data: usize,
        text: *mut u16,
        text_len: u32,
        bitmap: usize,
    }

    #[repr(C)]
    struct StartupInfo {
        size: u32,
        reserved: *mut u16,
        desktop: *mut u16,
        title: *mut u16,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        columns: u32,
        rows: u32,
        fill: u32,
        flags: u32,
        show: u16,
        reserved2: u16,
        reserved3: *mut u8,
        stdin: usize,
        stdout: usize,
        stderr: usize,
    }

    #[repr(C)]
    struct ProcessInfo {
        process: usize,
        thread: usize,
        process_id: u32,
        thread_id: u32,
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, model: u32) -> i32;
        fn CoUninitialize();
        fn CoCreateInstance(
            clsid: *const Guid,
            outer: *mut c_void,
            context: u32,
            iid: *const Guid,
            out: *mut *mut c_void,
        ) -> i32;
        fn CoTaskMemFree(block: *mut c_void);
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHParseDisplayName(
            name: *const u16,
            bind: *mut c_void,
            pidl: *mut *mut c_void,
            attributes: u32,
            out_attributes: *mut u32,
        ) -> i32;
        fn SHBindToParent(
            pidl: *const c_void,
            iid: *const Guid,
            out: *mut *mut c_void,
            last: *mut *mut c_void,
        ) -> i32;
        fn SHCreateDataObject(
            folder: *const c_void,
            count: u32,
            items: *const *const c_void,
            inner: *mut c_void,
            iid: *const Guid,
            out: *mut *mut c_void,
        ) -> i32;
        fn SHCreateItemFromIDList(
            pidl: *const c_void,
            iid: *const Guid,
            out: *mut *mut c_void,
        ) -> i32;
        fn ILClone(pidl: *const c_void) -> *mut c_void;
        fn ILRemoveLastID(pidl: *mut c_void) -> i32;
        fn ILIsEqual(first: *const c_void, second: *const c_void) -> i32;
        fn ShellExecuteExW(info: *mut ExecuteInfo) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateProcessW(
            application: *const u16,
            command: *mut u16,
            process_attributes: *mut c_void,
            thread_attributes: *mut c_void,
            inherit: i32,
            flags: u32,
            environment: *mut c_void,
            directory: *const u16,
            startup: *mut c_void,
            info: *mut c_void,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CloseHandle(handle: usize) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn CreatePopupMenu() -> usize;
        fn DestroyMenu(menu: usize) -> i32;
        fn GetMenuItemCount(menu: usize) -> i32;
        fn GetMenuItemInfoW(menu: usize, item: u32, by_position: i32, info: *mut MenuItemInfo) -> i32;
    }

    /// The apartment a call runs in. A handler draws its items with the shell's
    /// own controls, which want a single-threaded apartment; a thread that has
    /// already been claimed by someone else is used as it is rather than failing
    /// the whole menu.
    struct Apartment {
        owned: bool,
    }

    impl Apartment {
        fn new() -> Result<Apartment, String> {
            let status = unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
            if status >= 0 {
                return Ok(Apartment { owned: true });
            }
            if status == RPC_E_CHANGED_MODE {
                return Ok(Apartment { owned: false });
            }
            Err(format!("COM could not start (0x{:08X}).", status as u32))
        }
    }

    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.owned {
                unsafe { CoUninitialize() };
            }
        }
    }

    unsafe fn vtable<T>(object: *mut c_void) -> *const T {
        *(object as *mut *const T)
    }

    unsafe fn release(object: *mut c_void) {
        if object.is_null() {
            return;
        }
        ((*vtable::<Unknown>(object)).release)(object);
    }

    unsafe fn query(object: *mut c_void, iid: &Guid) -> Option<*mut c_void> {
        let mut out: *mut c_void = std::ptr::null_mut();
        let status = ((*vtable::<Unknown>(object)).query_interface)(object, iid, &mut out);
        if status < 0 || out.is_null() {
            None
        } else {
            Some(out)
        }
    }

    fn parse(path: &str) -> Result<*mut c_void, String> {
        let name = wide(path);
        let mut pidl: *mut c_void = std::ptr::null_mut();
        let status = unsafe {
            SHParseDisplayName(
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut pidl,
                0,
                std::ptr::null_mut(),
            )
        };
        if status < 0 || pidl.is_null() {
            return Err(format!("The shell cannot reach {path}."));
        }
        Ok(pidl)
    }

    /// `{23170F69-40C1-278A-1000-000100020000}` as the object COM wants.
    fn parse_guid(text: &str) -> Option<Guid> {
        let trimmed = text.trim().trim_matches(|ch| ch == '{' || ch == '}');
        let parts: Vec<&str> = trimmed.split('-').collect();
        if parts.len() != 5 {
            return None;
        }
        let first = u32::from_str_radix(parts[0], 16).ok()?;
        let second = u16::from_str_radix(parts[1], 16).ok()?;
        let third = u16::from_str_radix(parts[2], 16).ok()?;
        let high = u16::from_str_radix(parts[3], 16).ok()?;
        let low = u64::from_str_radix(parts[4], 16).ok()?;
        let mut rest = [0u8; 8];
        rest[..2].copy_from_slice(&high.to_be_bytes());
        rest[2..].copy_from_slice(&low.to_be_bytes()[2..]);
        Some(Guid {
            first,
            second,
            third,
            rest,
        })
    }

    /// A menu built by someone else, ready to be read or used. Everything it
    /// holds is released when it goes out of scope, so a caller never has to
    /// think about the order the shell wants it torn down in.
    struct Menu {
        context: *mut c_void,
        init: *mut c_void,
        data: *mut c_void,
        folder: *mut c_void,
        pidls: Vec<*mut c_void>,
        parents: Vec<*mut c_void>,
        hmenu: usize,
        // Declared last so the apartment is released after everything COM handed
        // out has been given back. It is only ever read by its own destructor.
        #[allow(dead_code)]
        apartment: Apartment,
    }

    impl Drop for Menu {
        fn drop(&mut self) {
            unsafe {
                release(self.context);
                release(self.init);
                release(self.data);
                release(self.folder);
                for pidl in self.pidls.drain(..) {
                    CoTaskMemFree(pidl);
                }
                for parent in self.parents.drain(..) {
                    CoTaskMemFree(parent);
                }
                if self.hmenu != 0 {
                    DestroyMenu(self.hmenu);
                }
            }
        }
    }

    fn draw(context: *mut c_void) -> Result<usize, String> {
        let hmenu = unsafe { CreatePopupMenu() };
        if hmenu == 0 {
            return Err("Windows would not open a menu.".to_string());
        }
        let status = unsafe {
            ((*vtable::<ContextMenu>(context)).query_context_menu)(
                context,
                hmenu,
                0,
                1,
                MENU_ID_END,
                CMF_NORMAL | CMF_EXPLORE,
            )
        };
        if status < 0 {
            unsafe { DestroyMenu(hmenu) };
            return Err(format!(
                "That menu would not draw its items (0x{:08X}).",
                status as u32
            ));
        }
        Ok(hmenu)
    }

    /// Tell a handler what is selected, using whichever of the three standard
    /// initialisers it actually speaks.
    ///
    /// `IShellExtInit` is the classic one and takes the whole selection; the
    /// newer pair take one item, as an `IShellItem` or as a path. A handler may
    /// expose more than one, and a failure through one is worth retrying through
    /// the next rather than losing the item. The returned name goes into a
    /// warning, where "which one failed" is the difference between a fixable
    /// report and a shrug.
    unsafe fn initialize_handler(
        object: *mut c_void,
        parent_pidl: *mut c_void,
        item_pidl: *mut c_void,
        data: *mut c_void,
        path: &str,
    ) -> Result<&'static str, String> {
        let mut tried = Vec::new();

        if let Some(ext) = query(object, &IID_SHELL_EXT_INIT) {
            let status = ((*vtable::<ShellExtInit>(ext)).initialize)(ext, parent_pidl, data, 0);
            release(ext);
            if status >= 0 {
                return Ok("IShellExtInit");
            }
            tried.push(format!("IShellExtInit (0x{:08X})", status as u32));
        }

        if let Some(with_item) = query(object, &IID_INIT_WITH_ITEM) {
            let mut item: *mut c_void = std::ptr::null_mut();
            let made = SHCreateItemFromIDList(item_pidl, &IID_ISHELL_ITEM, &mut item);
            let status = if made >= 0 && !item.is_null() {
                let status = ((*vtable::<InitializeWithItem>(with_item)).initialize)(
                    with_item, item, 0,
                );
                release(item);
                status
            } else {
                made
            };
            release(with_item);
            if status >= 0 {
                return Ok("IInitializeWithItem");
            }
            tried.push(format!("IInitializeWithItem (0x{:08X})", status as u32));
        }

        if let Some(with_file) = query(object, &IID_INIT_WITH_FILE) {
            let wide_path = wide(path);
            let status = ((*vtable::<InitializeWithFile>(with_file)).initialize)(
                with_file,
                wide_path.as_ptr(),
                0,
            );
            release(with_file);
            if status >= 0 {
                return Ok("IInitializeWithFile");
            }
            tried.push(format!("IInitializeWithFile (0x{:08X})", status as u32));
        }

        if tried.is_empty() {
            Err(
                "the handler speaks none of the initialisers Lyre knows; \
                 Windows 11 IExplorerCommand handlers are not supported yet"
                    .to_string(),
            )
        } else {
            Err(format!(
                "the handler refused the selection through {}",
                tried.join(", ")
            ))
        }
    }

    /// Ask one handler what it would put in the menu for these items.
    fn handler_menu(clsid: &str, paths: &[String]) -> Result<Menu, String> {
        let apartment = Apartment::new()?;
        let first = paths.first().ok_or("Nothing is selected.")?;
        let full = parse(first)?;

        let mut folder: *mut c_void = std::ptr::null_mut();
        let mut child: *mut c_void = std::ptr::null_mut();
        let status =
            unsafe { SHBindToParent(full, &IID_SHELL_FOLDER, &mut folder, &mut child) };
        if status < 0 || folder.is_null() {
            unsafe { CoTaskMemFree(full) };
            return Err(format!("The shell cannot open the folder holding {first}."));
        }

        // The parent is what identifies the group: a data object may carry
        // several items, but they have to be children of one folder.
        let parent = unsafe { ILClone(full) };
        unsafe { ILRemoveLastID(parent) };

        let mut pidls = vec![full];
        let mut selected: Vec<*const c_void> = vec![child];
        for path in paths.iter().skip(1) {
            let Ok(other) = parse(path) else { continue };
            let other_parent = unsafe { ILClone(other) };
            unsafe { ILRemoveLastID(other_parent) };
            let same = unsafe { ILIsEqual(parent, other_parent) } != 0;
            unsafe { CoTaskMemFree(other_parent) };
            if !same {
                unsafe { CoTaskMemFree(other) };
                continue;
            }
            let mut other_folder: *mut c_void = std::ptr::null_mut();
            let mut other_child: *mut c_void = std::ptr::null_mut();
            let status = unsafe {
                SHBindToParent(other, &IID_SHELL_FOLDER, &mut other_folder, &mut other_child)
            };
            unsafe { release(other_folder) };
            if status < 0 {
                unsafe { CoTaskMemFree(other) };
                continue;
            }
            selected.push(other_child);
            pidls.push(other);
        }

        let mut data: *mut c_void = std::ptr::null_mut();
        let status = unsafe {
            SHCreateDataObject(
                parent,
                selected.len() as u32,
                selected.as_ptr(),
                std::ptr::null_mut(),
                &IID_DATA_OBJECT,
                &mut data,
            )
        };
        if status < 0 || data.is_null() {
            unsafe {
                release(folder);
                for pidl in pidls {
                    CoTaskMemFree(pidl);
                }
                CoTaskMemFree(parent);
            }
            return Err("The shell could not describe the selected items.".to_string());
        }

        let Some(handler_clsid) = parse_guid(clsid) else {
            unsafe {
                release(data);
                release(folder);
                for pidl in pidls {
                    CoTaskMemFree(pidl);
                }
                CoTaskMemFree(parent);
            }
            return Err(format!("{clsid} is not a class id."));
        };

        // Ask for IUnknown rather than one fixed interface: which initialiser a
        // handler implements is discovered, not assumed.
        let mut unknown: *mut c_void = std::ptr::null_mut();
        let status = unsafe {
            CoCreateInstance(
                &handler_clsid,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER,
                &IID_IUNKNOWN,
                &mut unknown,
            )
        };
        if status < 0 || unknown.is_null() {
            unsafe {
                release(data);
                release(folder);
                for pidl in pidls {
                    CoTaskMemFree(pidl);
                }
                CoTaskMemFree(parent);
            }
            return Err(format!(
                "The {clsid} menu handler could not be loaded (0x{:08X}).",
                status as u32
            ));
        }

        let mut menu = Menu {
            context: std::ptr::null_mut(),
            init: unknown,
            data,
            folder,
            pidls,
            parents: vec![parent],
            hmenu: 0,
            apartment,
        };

        // From here on every error path drops `menu`, which hands everything
        // back; the shell's rules for that order are all in `Menu::drop`.
        if let Err(reason) = unsafe {
            initialize_handler(menu.init, menu.parents[0], menu.pidls[0], menu.data, first)
        } {
            return Err(format!("The menu handler could not be used: {reason}."));
        }

        match unsafe { query(menu.init, &IID_CONTEXT_MENU) } {
            Some(context) => menu.context = context,
            None => return Err("The menu handler offers no items.".to_string()),
        }

        menu.hmenu = draw(menu.context)?;
        Ok(menu)
    }

    /// The menu of a folder's own background, which is a different object: the
    /// folder is asked for its view's menu rather than for an item's. One call
    /// covers every handler registered for that background.
    fn background_menu(folder_path: &str) -> Result<Menu, String> {
        let apartment = Apartment::new()?;
        let full = parse(folder_path)?;

        // The folder has to be reached through its parent: the shell hands out
        // an item's folder and its own id, and the folder binds itself from
        // there.
        let mut parent: *mut c_void = std::ptr::null_mut();
        let mut child: *mut c_void = std::ptr::null_mut();
        let status = unsafe { SHBindToParent(full, &IID_SHELL_FOLDER, &mut parent, &mut child) };
        if status < 0 || parent.is_null() {
            unsafe { CoTaskMemFree(full) };
            return Err(format!("The shell cannot open {folder_path}."));
        }
        let mut folder: *mut c_void = std::ptr::null_mut();
        let status = unsafe {
            ((*vtable::<ShellFolder>(parent)).bind_to_object)(
                parent,
                child,
                std::ptr::null_mut(),
                &IID_SHELL_FOLDER,
                &mut folder,
            )
        };
        unsafe { release(parent) };
        if status < 0 || folder.is_null() {
            unsafe { CoTaskMemFree(full) };
            return Err(format!("The shell cannot read {folder_path} as a folder."));
        }

        let mut context: *mut c_void = std::ptr::null_mut();
        let status = unsafe {
            ((*vtable::<ShellFolder>(folder)).create_view_object)(
                folder,
                0,
                &IID_CONTEXT_MENU,
                &mut context,
            )
        };
        if status < 0 || context.is_null() {
            unsafe {
                release(folder);
                CoTaskMemFree(full);
            }
            return Err("Windows offers no menu for that folder.".to_string());
        }

        let mut menu = Menu {
            context,
            init: std::ptr::null_mut(),
            data: std::ptr::null_mut(),
            folder,
            pidls: vec![full],
            parents: Vec::new(),
            hmenu: 0,
            apartment,
        };
        menu.hmenu = draw(menu.context)?;
        Ok(menu)
    }

    /// Everything a handler drew, as entries. Items a handler owns start at id
    /// 1; anything below that is not ours to name.
    fn walk(hmenu: usize, id_for: &dyn Fn(u32) -> String, origin: &str, depth: u32) -> Vec<Entry> {
        let count = unsafe { GetMenuItemCount(hmenu) };
        if count <= 0 || depth > 2 {
            return Vec::new();
        }
        let mut entries = Vec::new();
        for index in 0..count as u32 {
            let mut info: MenuItemInfo = unsafe { std::mem::zeroed() };
            info.size = std::mem::size_of::<MenuItemInfo>() as u32;
            info.mask = MIIM_ID | MIIM_STRING | MIIM_SUBMENU | MIIM_FTYPE;
            let mut text = vec![0u16; 512];
            info.text = text.as_mut_ptr();
            info.text_len = text.len() as u32;
            if unsafe { GetMenuItemInfoW(hmenu, index, 1, &mut info) } == 0 {
                continue;
            }
            if info.kind & MFT_SEPARATOR != 0 || info.id == 0 {
                continue;
            }
            let end = text.iter().position(|unit| *unit == 0).unwrap_or(0);
            let label = clean(&String::from_utf16_lossy(&text[..end]));
            let children = if info.submenu != 0 {
                walk(info.submenu, id_for, origin, depth + 1)
            } else {
                Vec::new()
            };
            if info.submenu != 0 && children.is_empty() {
                continue;
            }
            // A submenu's own text can be empty, and an empty row with an arrow
            // on it says nothing.
            if label.is_empty() && children.is_empty() {
                continue;
            }
            entries.push(Entry {
                id: id_for(info.id),
                label,
                origin: origin.to_string(),
                source: String::new(),
                kind: "handler".to_string(),
                default_on: true,
                children,
            });
        }
        entries
    }

    /// Stamp the name of the program on everything a handler produced, so the
    /// customise list can group by it however deep the submenus go.
    fn credit(entries: &mut [Entry], source: &str) {
        for entry in entries {
            entry.source = source.to_string();
            if ALREADY_HANDLED_HANDLERS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(source))
            {
                entry.default_on = false;
            }
            credit(&mut entry.children, source);
        }
    }

    fn handler_entries(
        clsid: &str,
        source: &str,
        origin: &str,
        scope: &str,
        paths: &[String],
        warnings: &mut Vec<String>,
    ) -> Vec<Entry> {
        let menu = match handler_menu(clsid, paths) {
            Ok(menu) => menu,
            Err(error) => {
                // The registration exists. If it cannot be read, that is a
                // fact the person looking for its item needs, not a reason to
                // make the item look absent.
                warnings.push(format!("{source} ({clsid}): {error}"));
                return Vec::new();
            }
        };
        let id = |command: u32| format!("handler|{scope}|{clsid}|{command}");
        let mut entries = walk(menu.hmenu, &id, origin, 0);
        credit(&mut entries, source);
        entries
    }

    /// The background's own menu, which arrives whole from the folder. The verbs
    /// read from the registry are already in that menu, so the ones that repeat
    /// are dropped rather than shown twice.
    fn background_entries(
        folder: &str,
        verbs: &[Entry],
        warnings: &mut Vec<String>,
    ) -> Vec<Entry> {
        let menu = match background_menu(folder) {
            Ok(menu) => menu,
            Err(error) => {
                warnings.push(format!("The folder's own menu: {error}"));
                return Vec::new();
            }
        };
        let id = |command: u32| format!("background|{command}");
        let mut entries = walk(menu.hmenu, &id, "background", 0);
        entries.retain(|entry| {
            !verbs
                .iter()
                .any(|verb| verb.label.eq_ignore_ascii_case(&entry.label))
        });
        // This menu is drawn by the folder as a whole, so there is no honest way
        // to say which program owns which row. Crediting all of them to Windows
        // used to make a third party's item look like a system one.
        for entry in &mut entries {
            if ALREADY_HANDLED_HANDLERS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(&entry.label))
            {
                entry.default_on = false;
            }
        }
        entries
    }

    pub(super) fn catalog(path: Option<&str>, background: bool) -> Catalog {
        let target = path.filter(|value| !value.is_empty());
        let is_background = background || target.is_none();
        let origin = if is_background {
            "background"
        } else if target.is_some_and(|item| Path::new(item).is_dir()) {
            "folder"
        } else {
            "file"
        };
        // Somebody who turns "Copy path" off for files means it for folders
        // too: the menu is one menu, even though the registry keeps the two
        // registrations apart.
        let scope = if is_background { "background" } else { "item" };
        let target_path = target.map(Path::new);

        let mut warnings = Vec::new();
        let root_verbs = verbs(origin, scope, target_path, &mut warnings);
        let mut entries = root_verbs.clone();

        match (is_background, target) {
            (true, Some(folder)) => entries.extend(background_entries(
                folder,
                &root_verbs,
                &mut warnings,
            )),
            (false, Some(item)) => {
                for (clsid, source) in handlers(origin, target_path) {
                    let paths = vec![item.to_string()];
                    entries.extend(handler_entries(
                        &clsid,
                        &source,
                        origin,
                        scope,
                        &paths,
                        &mut warnings,
                    ));
                }
            }
            _ => {}
        }

        // A verb and a handler can offer the same thing; the first registration
        // wins, which keeps the order the classes were read in.
        let mut seen = HashSet::new();
        entries.retain(|entry| seen.insert(entry.id.clone()));

        Catalog {
            supported: true,
            note: None,
            entries,
            warnings,
        }
    }

    pub(super) fn run(id: &str, paths: &[String]) -> Result<(), String> {
        // The id arrives from the webview. A verb names a registry key and a
        // class id is parsed before it is used, but neither should be able to
        // walk out of the key it was read from.
        if id.split('|').any(|part| part.contains('\\') || part.contains('/')) {
            return Err("That menu item is not one Lyre can run.".to_string());
        }
        let parts: Vec<&str> = id.split('|').collect();
        match parts.as_slice() {
            ["verb", scope, verb] => run_verb(scope, verb, paths),
            ["background", command] => {
                let folder = paths.first().ok_or("There is no folder to work in.")?;
                let command: u32 = command
                    .parse()
                    .map_err(|_| "That menu item is not one Lyre can run.".to_string())?;
                let menu = background_menu(folder)?;
                invoke(&menu, command)
            }
            ["handler", _origin, clsid, command] => {
                let command: u32 = command
                    .parse()
                    .map_err(|_| "That menu item is not one Lyre can run.".to_string())?;
                let menu = handler_menu(clsid, paths)?;
                invoke(&menu, command)
            }
            _ => Err("That menu item is not one Lyre can run.".to_string()),
        }
    }

    /// The handler assigns ids as it draws, so the chosen one is looked up in a
    /// freshly drawn menu rather than trusted to repeat from last time.
    fn invoke(menu: &Menu, command: u32) -> Result<(), String> {
        if !contains(menu.hmenu, command) {
            return Err("That item is not in the menu any more.".to_string());
        }
        let info = CmdInfo {
            size: std::mem::size_of::<CmdInfo>() as u32,
            mask: 0,
            window: 0,
            // The offset into the handler's own range, packed into the pointer.
            verb: (command - 1) as usize as *const u8,
            parameters: std::ptr::null(),
            directory: std::ptr::null(),
            show: SW_SHOWNORMAL,
            hot_key: 0,
            icon: 0,
        };
        let status =
            unsafe { ((*vtable::<ContextMenu>(menu.context)).invoke_command)(menu.context, &info) };
        if status < 0 {
            return Err(format!(
                "That menu item could not be run (0x{:08X}).",
                status as u32
            ));
        }
        Ok(())
    }

    fn contains(hmenu: usize, command: u32) -> bool {
        let count = unsafe { GetMenuItemCount(hmenu) };
        for index in 0..count.max(0) as u32 {
            let mut info: MenuItemInfo = unsafe { std::mem::zeroed() };
            info.size = std::mem::size_of::<MenuItemInfo>() as u32;
            info.mask = MIIM_ID | MIIM_SUBMENU;
            if unsafe { GetMenuItemInfoW(hmenu, index, 1, &mut info) } == 0 {
                continue;
            }
            if info.id == command || (info.submenu != 0 && contains(info.submenu, command)) {
                return true;
            }
        }
        false
    }

    fn run_verb(origin: &str, verb: &str, paths: &[String]) -> Result<(), String> {
        if origin == "background" {
            let folder = paths.first().ok_or("There is no folder to work in.")?;
            return run_background_command(verb, folder);
        }
        if paths.is_empty() {
            return Err("Nothing is selected.".to_string());
        }
        let verb = wide(verb);
        for path in paths {
            let file = wide(path);
            let mut info: ExecuteInfo = unsafe { std::mem::zeroed() };
            info.size = std::mem::size_of::<ExecuteInfo>() as u32;
            // No dialog of the shell's own: a failure belongs in the status bar,
            // where the rest of this application reports them.
            info.mask = 0x400;
            info.verb = verb.as_ptr();
            info.file = file.as_ptr();
            info.show = SW_SHOWNORMAL;
            if unsafe { ShellExecuteExW(&mut info) } == 0 {
                return Err(last_error("Windows could not run that item"));
            }
        }
        Ok(())
    }

    /// A background verb is a command line written for a folder, so it is run the
    /// way the shell runs it: the folder takes the place of `%V` (or `%1`) and
    /// the result is a command line.
    fn run_background_command(verb: &str, folder: &str) -> Result<(), String> {
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
        let mut command = wide(&line);
        let startup = StartupInfo {
            size: std::mem::size_of::<StartupInfo>() as u32,
            reserved: std::ptr::null_mut(),
            desktop: std::ptr::null_mut(),
            title: std::ptr::null_mut(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            columns: 0,
            rows: 0,
            fill: 0,
            flags: 0,
            show: 0,
            reserved2: 0,
            reserved3: std::ptr::null_mut(),
            stdin: 0,
            stdout: 0,
            stderr: 0,
        };
        let mut info = ProcessInfo {
            process: 0,
            thread: 0,
            process_id: 0,
            thread_id: 0,
        };
        let mut startup = startup;
        let started = unsafe {
            CreateProcessW(
                std::ptr::null(),
                command.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null(),
                &mut startup as *mut StartupInfo as *mut c_void,
                &mut info as *mut ProcessInfo as *mut c_void,
            )
        };
        if started == 0 {
            return Err(last_error("Windows could not run that item"));
        }
        unsafe {
            CloseHandle(info.process);
            CloseHandle(info.thread);
        }
        Ok(())
    }

    fn last_error(prefix: &str) -> String {
        let code = std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or_default();
        format!("{prefix} (code {code}).")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_menu_label_loses_its_access_key_marker() {
            assert_eq!(clean("&Scan with Bitdefender"), "Scan with Bitdefender");
            assert_eq!(clean("Extract && add"), "Extract & add");
            assert_eq!(clean("  Added to archive  "), "Added to archive");
        }

        #[test]
        fn a_class_guid_survives_the_trip_through_its_bytes() {
            let guid = parse_guid("{23170F69-40C1-278A-1000-000100020000}")
                .expect("a well-formed class id");
            assert_eq!(guid.first, 0x2317_0F69);
            assert_eq!(guid.second, 0x40C1);
            assert_eq!(guid.third, 0x278A);
            assert_eq!(guid.rest, [0x10, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00]);
            assert!(parse_guid("7-Zip").is_none());
            assert!(parse_guid("{23170F69-40C1-278A-1000}").is_none());
        }

        #[test]
        fn a_menu_id_is_read_back_as_the_route_to_run_it() {
            let id = "handler|file|{23170F69-40C1-278A-1000-000100020000}|7";
            let parts: Vec<&str> = id.split('|').collect();
            assert_eq!(parts, ["handler", "file", "{23170F69-40C1-278A-1000-000100020000}", "7"]);
        }

        /// These are copied by hand from ShObjIdl_core.h / Propsys.h in the
        /// Windows SDK. A transposed digit here is invisible at compile time and
        /// loses every handler that speaks the newer initialisers.
        #[test]
        fn the_interface_ids_are_the_ones_in_the_sdk_headers() {
            assert_eq!(
                parse_guid("00000000-0000-0000-C000-000000000046"),
                Some(IID_IUNKNOWN)
            );
            assert_eq!(
                parse_guid("000214E8-0000-0000-C000-000000000046"),
                Some(IID_SHELL_EXT_INIT)
            );
            assert_eq!(
                parse_guid("7F73BE3F-FB79-493C-A6C7-7EE14E245841"),
                Some(IID_INIT_WITH_ITEM)
            );
            assert_eq!(
                parse_guid("B7D14566-0509-4CCE-A71F-0A554233BD9B"),
                Some(IID_INIT_WITH_FILE)
            );
            assert_eq!(
                parse_guid("43826D1E-E718-42EE-BC55-A1E261C37BFE"),
                Some(IID_ISHELL_ITEM)
            );
        }

        #[test]
        fn an_id_cannot_walk_out_of_the_key_it_names() {
            let folder = vec!["C:\\Windows".to_string()];
            for id in [
                "verb|background|..\\..\\shell\\open",
                "verb|item|open/command",
                "handler|item|{23170F69-40C1-278A-1000-000100020000}|1/2",
            ] {
                let error = run(id, &folder).expect_err("a path in an id must be refused");
                assert!(error.contains("not one Lyre can run"), "unexpected: {error}");
            }
        }

        #[test]
        fn the_system_always_registers_at_least_one_menu_handler() {
            // Windows itself puts "Open With" on every file, so an empty answer
            // means the registry walk is broken rather than that nothing is
            // installed.
            let found = subkeys("*\\shellex\\ContextMenuHandlers");
            assert!(!found.is_empty(), "no menu handlers were found at all");
        }

        #[test]
        fn a_file_of_a_known_type_reports_where_its_registrations_live() {
            let classes = classes("file", Some(Path::new("C:\\Windows\\System32\\kernel32.dll")));
            assert_eq!(classes[0], "AllFilesystemObjects");
            assert_eq!(classes[1], "*");
            assert!(classes.iter().any(|class| class == ".dll"));
            assert!(classes.iter().any(|class| class == "SystemFileAssociations\\.dll"));
        }

        #[test]
        fn a_folder_and_a_volume_root_ask_different_classes() {
            let folder = classes("folder", Some(Path::new("C:\\Windows")));
            assert!(!folder.contains(&"Drive".to_string()));
            let volume = classes("folder", Some(Path::new("C:\\")));
            assert!(volume.contains(&"Drive".to_string()));
            assert_eq!(classes("background", None), ["Directory\\Background"]);
        }

        #[test]
        fn the_verbs_lyre_already_offers_are_listed_but_off() {
            let found = verbs(
                "file",
                "item",
                Some(Path::new("C:\\Windows\\System32\\kernel32.dll")),
                &mut Vec::new(),
            );
            for entry in found {
                if entry.label.eq_ignore_ascii_case("open") {
                    assert!(!entry.default_on, "open is Lyre's own item");
                }
            }
        }

        #[test]
        fn a_verb_that_is_no_longer_registered_says_so() {
            let error = run(
                "verb|background|lyre-no-such-verb",
                &["C:\\Windows".to_string()],
            )
            .expect_err("a verb with no command must not quietly do nothing");
            assert!(error.contains("no longer has a command"), "unexpected: {error}");
        }

        #[test]
        fn an_id_that_names_nothing_is_refused_rather_than_guessed() {
            let folder = vec!["C:\\Windows".to_string()];
            assert!(run("handler|item|not-a-class|1", &folder).is_err());
            assert!(run("handler|item|{23170F69-40C1-278A-1000-000100020000}|soon", &folder).is_err());
            assert!(run("background|soon", &folder).is_err());
            assert!(run("nonsense", &folder).is_err());
        }

        /// Loading a real handler is the only way to know the COM side works,
        /// and it depends on what happens to be installed, so it is run by hand:
        /// `cargo test print_what_the_installed_handlers_offer -- --ignored --nocapture`.
        /// The warnings are the point: a registration that exists but could not
        /// be read is otherwise indistinguishable from one that is not there.
        #[test]
        #[ignore]
        fn print_what_the_installed_handlers_offer() {
            fn show(entries: &[Entry], depth: usize) {
                for entry in entries {
                    println!(
                        "{:indent$}{}  [{} / {}]{}",
                        "",
                        entry.label,
                        entry.source,
                        entry.id,
                        if entry.default_on { "" } else { "  (off)" },
                        indent = depth * 2
                    );
                    show(&entry.children, depth + 1);
                }
            }
            // A background menu belongs to a folder, never to a file: asking the
            // shell for a file's background is a diagnostic of its own, not a
            // useful line in this report.
            for (path, background) in [
                ("C:\\Windows\\System32\\notepad.exe", false),
                ("C:\\Windows", false),
                ("C:\\Windows", true),
                ("C:\\", false),
                ("C:\\", true),
            ] {
                let answer = catalog(Some(path), background);
                println!(
                    "== {path}{}",
                    if background { " (background)" } else { "" }
                );
                show(&answer.entries, 0);
                for warning in &answer.warnings {
                    println!("  ! {warning}");
                }
            }
        }
    }
}
