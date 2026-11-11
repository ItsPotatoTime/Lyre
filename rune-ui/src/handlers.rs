//! The third-party COM objects that draw their own context-menu items.
//!
//! 7-Zip, FineReader and friends register no commands under `shell`. They
//! register an in-process DLL under `shellex\ContextMenuHandlers` that builds its
//! menu when the shell asks, which is why their items exist nowhere in the
//! registry and cannot be found by reading keys.
//!
//! Listing what such a handler would draw means loading that DLL into this
//! process and calling `IContextMenu::QueryContextMenu`, exactly as Explorer
//! does. That is third-party code running here, where a broken extension can take
//! the window down, so it only happens for handlers the person switched on: the
//! registry side of this module is safe and always available, and the loading
//! side is asked for one handler at a time.

#[cfg(not(target_os = "windows"))]
use crate::shell::Verb;

/// One registered handler, before it has been asked what it would draw.
#[derive(Clone, Debug)]
pub struct Registration {
    /// The class it was registered under, which is the stable part of its name.
    pub class: String,
    /// The registration key name.
    pub name: String,
    /// What to show in the list.
    pub label: String,
    /// The CLSID it activates, when it is registered as one.
    pub clsid: String,
    /// Its in-process server, when the CLSID resolves to a DLL.
    pub dll: Option<String>,
}

impl Registration {
    /// Stable identity for remembering the person's choice.
    pub fn id(&self) -> String {
        format!("handler|{}|{}", self.class, self.name)
    }
}

#[cfg(target_os = "windows")]
pub use win::{invoke, items, registrations};

#[cfg(not(target_os = "windows"))]
pub fn registrations(_classes: &[String]) -> Vec<Registration> {
    Vec::new()
}

#[cfg(not(target_os = "windows"))]
pub fn items(_clsid: &str, _paths: &[String]) -> Result<Vec<Verb>, String> {
    Err("Third-party menu handlers exist only on Windows.".into())
}

#[cfg(not(target_os = "windows"))]
pub fn invoke(_clsid: &str, _paths: &[String], _command: u32) -> Result<(), String> {
    Err("Third-party menu handlers exist only on Windows.".into())
}

// ---------------------------------------------------------------- probes ----

#[cfg(all(test, target_os = "windows"))]
mod probes {
    /// Temporary: loads a real handler to see whether its items come back.
    #[test]
    #[ignore]
    fn dump_real_handlers() {
        let path = std::env::temp_dir().join("lyre-handler-probe.txt");
        std::fs::write(&path, b"x").unwrap();
        let probe = path.to_string_lossy().into_owned();
        let classes = vec![
            "AllFilesystemObjects".to_string(),
            "*".to_string(),
            ".txt".to_string(),
        ];
        println!("classes: {classes:?}");
        let found = super::registrations(&classes);
        println!("\n{} handlers:", found.len());
        for reg in &found {
            println!("   {:<28} {}  dll={:?}", reg.label, reg.clsid, reg.dll);
        }
        for reg in found
            .iter()
            .filter(|reg| reg.label.to_lowercase().contains("7-zip"))
        {
            println!("\n--- items from {} ({}) ---", reg.label, reg.clsid);
            match super::items(&reg.clsid, std::slice::from_ref(&probe)) {
                Ok(items) => {
                    println!("{} items", items.len());
                    for item in &items {
                        let kind = match &item.run {
                            crate::shell::Run::Handler { command, .. } => format!("#{command}"),
                            _ => "group".to_string(),
                        };
                        println!("   {:<40} {kind}", item.label);
                    }
                }
                Err(err) => println!("failed: {err}"),
            }
        }
        std::fs::remove_file(&path).ok();
    }
}

#[cfg(target_os = "windows")]
mod win {
    use super::Registration;
    use crate::shell::{Run, Verb};
    use std::ffi::c_void;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Shell::CMINVOKECOMMANDINFO;
    use windows::Win32::UI::Shell::{CMINVOKECOMMANDINFOEX, IContextMenu, IShellExtInit};
    use windows::core::{GUID, Interface};

    /// Tell the handler the ordinal field carries a wide string.
    const CMIC_MASK_UNICODE: u32 = 0x0000_4000;

    const CMF_NORMAL: u32 = 0;
    /// The first and last command ids handed to a handler. Explorer's range.
    const ID_FIRST: u32 = 1;
    const ID_LAST: u32 = 0x7FFF;
    const MF_BYPOSITION: u32 = 0x0000_0400;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn CreatePopupMenu() -> *mut c_void;
        fn DestroyMenu(menu: *mut c_void) -> i32;
        fn GetMenuItemCount(menu: *mut c_void) -> i32;
        fn GetSubMenu(menu: *mut c_void, pos: i32) -> *mut c_void;
        fn GetMenuItemID(menu: *mut c_void, pos: i32) -> u32;
        fn GetMenuStringW(
            menu: *mut c_void,
            item: u32,
            buffer: *mut u16,
            max: i32,
            flags: u32,
        ) -> i32;
    }

    /// A CLSID from the registry's brace-and-dash spelling.
    fn parse_guid(text: &str) -> Option<GUID> {
        let hex: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if hex.len() != 32 {
            return None;
        }
        let byte = |index: usize| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok();
        let mut bytes = [0u8; 16];
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = byte(index)?;
        }
        Some(GUID {
            data1: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            data2: u16::from_be_bytes([bytes[4], bytes[5]]),
            data3: u16::from_be_bytes([bytes[6], bytes[7]]),
            data4: [
                bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                bytes[15],
            ],
        })
    }

    /// A registration key name turned into something worth reading:
    /// `FineReader16ContextMenu` is the program "FineReader16".
    fn tidy(name: &str) -> String {
        let trimmed = name
            .strip_suffix("ContextMenu")
            .or_else(|| name.strip_suffix("ContextMenuHandler"))
            .or_else(|| name.strip_suffix("Menu"))
            .unwrap_or(name);
        let trimmed = trimmed.trim_end_matches(['1', '2', '3']).trim();
        if trimmed.is_empty() {
            name.to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Every handler registered for these classes, deduplicated by CLSID so one
    /// program that registers under several classes is listed once.
    pub fn registrations(classes: &[String]) -> Vec<Registration> {
        let mut found: Vec<Registration> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for class in classes {
            let base = format!(r"{class}\shellex\ContextMenuHandlers");
            for name in crate::shell::reg_subkeys(&base) {
                let key = format!(r"{base}\{name}");
                let Some(value) = crate::shell::reg_string(&key, None) else {
                    continue;
                };
                let is_guid = value.trim_start().starts_with('{');
                let clsid = if is_guid {
                    value.trim().to_ascii_uppercase()
                } else {
                    // A handler may also name its server directly.
                    value.trim().to_string()
                };
                let identity = clsid.to_ascii_lowercase();
                if seen.contains(&identity) {
                    continue;
                }
                let dll = if is_guid {
                    crate::shell::reg_string(&format!(r"CLSID\{clsid}\InprocServer32"), None)
                } else {
                    Some(clsid.clone())
                };
                // The program names itself better than its key does.
                let label = dll
                    .as_deref()
                    .and_then(crate::openwith::executable_description)
                    .unwrap_or_else(|| tidy(&name));
                seen.push(identity);
                found.push(Registration {
                    class: class.clone(),
                    name: name.clone(),
                    label,
                    clsid,
                    dll,
                });
            }
        }
        found.sort_by_cached_key(|handler| handler.label.to_lowercase());
        found
    }

    /// Windows wants an apartment before any of this.
    fn ensure_com() {
        // SAFETY: a plain COM initialisation; RPC_E_CHANGED_MODE means another
        // apartment already owns this thread, which is fine.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
    }

    /// The handler instance, initialised with the files it is being asked about.
    ///
    /// `IShellExtInit::Initialize` is how a handler learns the selection; without
    /// it, QueryContextMenu draws a menu for nothing, and a handler given one
    /// path cannot act on a multiple selection.
    fn handler(clsid: &str, paths: &[String]) -> Result<IContextMenu, String> {
        let guid = parse_guid(clsid).ok_or_else(|| format!("{clsid} is not a class id."))?;
        ensure_com();
        // SAFETY: the CLSID is well formed and this is the documented activation.
        unsafe {
            let unknown: windows::core::IUnknown =
                CoCreateInstance(&guid, None, CLSCTX_INPROC_SERVER)
                    .map_err(|err| format!("Windows could not load that extension: {err}"))?;
            let files: Vec<std::path::PathBuf> =
                paths.iter().map(std::path::PathBuf::from).collect();
            let data = crate::osdrag::win::data_object_for(&files)
                .ok_or_else(|| "Could not describe that selection to the extension.".to_string())?;
            if let Ok(init) = unknown.cast::<IShellExtInit>() {
                let _ = init.Initialize(None, Some(&data), None);
            }
            unknown
                .cast::<IContextMenu>()
                .map_err(|err| format!("That extension draws no menu: {err}"))
        }
    }

    /// Read one menu level, recursing into the submenus a handler drew.
    fn walk_menu(menu: *mut c_void, clsid: &str, depth: u8, out: &mut Vec<Verb>) {
        // SAFETY: `menu` came from CreatePopupMenu or GetSubMenu and stays alive
        // for the whole walk.
        unsafe {
            let count = GetMenuItemCount(menu);
            for pos in 0..count {
                let mut buffer = [0u16; 512];
                let length = GetMenuStringW(
                    menu,
                    pos as u32,
                    buffer.as_mut_ptr(),
                    buffer.len() as i32,
                    MF_BYPOSITION,
                );
                let text = if length > 0 {
                    String::from_utf16_lossy(&buffer[..length as usize])
                } else {
                    String::new()
                };
                // Accelerator markers are not part of the name.
                let label: String = text.replace('&', "").trim().to_string();

                let submenu = GetSubMenu(menu, pos);
                if !submenu.is_null() {
                    let before = out.len();
                    walk_menu(submenu, clsid, depth + 1, out);
                    // A submenu is a group of items, not an item: name it if it
                    // drew nothing worth showing underneath.
                    if out.len() == before && !label.is_empty() {
                        out.push(Verb {
                            id: format!("handler|{clsid}|{depth}:{label}"),
                            label,
                            source: String::new(),
                            verb: String::new(),
                            background: false,
                            children: Vec::new(),
                            extended: false,
                            run: Run::Command(String::new()),
                        });
                    }
                    continue;
                }

                let command = GetMenuItemID(menu, pos);
                // -1 is what separators and popups report.
                if command == u32::MAX || label.is_empty() {
                    continue;
                }
                out.push(Verb {
                    // Keyed by what it says, not by the ordinal it happened to
                    // get: a handler numbers its items per file, so the ordinal
                    // moves and a remembered choice would land on another item.
                    id: format!("handler|{clsid}|{label}"),
                    label,
                    source: String::new(),
                    verb: String::new(),
                    background: false,
                    children: Vec::new(),
                    extended: false,
                    run: Run::Handler {
                        clsid: clsid.to_string(),
                        command,
                    },
                });
            }
        }
    }

    /// What this handler would add to a menu for this selection.
    pub fn items(clsid: &str, paths: &[String]) -> Result<Vec<Verb>, String> {
        let menu = handler(clsid, paths)?;
        // SAFETY: a menu handle that is destroyed before returning, and the
        // command range Explorer uses.
        unsafe {
            let popup = CreatePopupMenu();
            if popup.is_null() {
                return Err("Windows could not create a menu to ask into.".into());
            }
            let result = menu.QueryContextMenu(
                windows::Win32::UI::WindowsAndMessaging::HMENU(popup),
                0,
                ID_FIRST,
                ID_LAST,
                CMF_NORMAL,
            );
            let mut found = Vec::new();
            if result.is_ok() {
                walk_menu(popup, clsid, 0, &mut found);
            }
            DestroyMenu(popup);
            if result.is_err() {
                return Err("That extension refused to describe its menu.".into());
            }
            Ok(found)
        }
    }

    /// Run one item the handler drew.
    pub fn invoke(clsid: &str, paths: &[String], command: u32) -> Result<(), String> {
        let menu = handler(clsid, paths)?;
        // SAFETY: the ordinal is relative to the same command range the items
        // were read in, which is the documented way to invoke one.
        unsafe {
            let info = CMINVOKECOMMANDINFOEX {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                fMask: CMIC_MASK_UNICODE,
                lpVerb: windows::core::PCSTR((command - ID_FIRST) as usize as *const u8),
                lpVerbW: windows::core::PCWSTR((command - ID_FIRST) as usize as *const u16),
                nShow: windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL.0,
                ..Default::default()
            };
            menu.InvokeCommand(&info as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO)
                .map_err(|err| format!("Windows could not run that item: {err}"))
        }
    }
}
