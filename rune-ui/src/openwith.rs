//! "Open with": which applications declare they can open a file, which one is
//! the default today, and how to change that.
//!
//! The three platforms answer in their own terms, and the differences are not
//! cosmetic.
//!
//! **Linux** is open. The MIME type comes from `xdg-mime`, the candidates are
//! the `.desktop` entries the desktop itself reads, opening goes through
//! `gio launch`, and the default is a single file rewritten by `xdg-mime
//! default`. Everything can be listed and changed from inside the window.
//!
//! **macOS** keeps the same three answers in LaunchServices: the type is a UTI,
//! the candidates and the default are role handlers for that UTI, and both are
//! readable and settable through its C API. Applications launch by bundle id.
//!
//! **Windows** is closed only where the *default* is concerned. The candidate
//! list is readable: a classic application registers a ProgID under the
//! extension's `OpenWithProgids`, appears in `SystemFileAssociations\.ext\
//! OpenWithList`, or lists the types it accepts under `Applications\<exe>\
//! SupportedTypes`. The default, though, is a `UserChoice` key carrying a hash
//! the system validates, so writing it is tampering the OS reverts; only its own
//! picker may change it. That is what [`OpenWith::default_via_picker`] reports.
//!
//! Nothing here loads a third-party DLL or COM object: every answer comes from
//! the registry, from files, or from a system framework.

use std::path::Path;

/// One application that says it can open this kind of file.
#[derive(Clone, Debug)]
pub struct AppEntry {
    /// Identity used to launch it: a Windows ProgID, a Linux `.desktop` file
    /// name, or a macOS bundle identifier.
    pub id: String,
    pub name: String,
}

/// The answer for one file: what kind it is, what opens it today, and what else
/// could.
#[derive(Clone, Debug)]
pub struct OpenWith {
    /// The kind in the platform's own words: a MIME type on Linux, a dotted
    /// lower-case extension elsewhere. Empty when it could not be worked out.
    pub kind: String,
    /// The id of the application the platform would use today.
    pub default_app: Option<String>,
    pub apps: Vec<AppEntry>,
    /// Set when the platform cannot answer at all, with the reason.
    pub unsupported: Option<String>,
    /// True when only the platform's own picker may change the default, which is
    /// every Windows build. The list is still usable; the *choice* is not ours.
    pub default_via_picker: bool,
}

impl OpenWith {
    fn unsupported(kind: &str, reason: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            default_app: None,
            apps: Vec::new(),
            unsupported: Some(reason.into()),
            default_via_picker: false,
        }
    }
}

/// The dotted, lower-case extension of a path, or empty when it has none.
/// Dotfiles such as `.bashrc` count as having no extension, as the platforms do.
fn extension_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

/// The program a command line starts with, unquoted. Used for the `Exec=` field
/// of a `.desktop` entry and for a Windows `shell\open\command`.
#[cfg_attr(test, allow(dead_code))]
fn program_of(command: &str) -> Option<&str> {
    let rest = command.trim_start();
    let program = if let Some(quoted) = rest.strip_prefix('"') {
        quoted.split('"').next()?
    } else {
        rest.split_whitespace().next()?
    };
    (!program.is_empty()).then_some(program)
}

// ------------------------------------------------------------- Linux ----
//
// The freedesktop conventions, which every desktop environment reads: the type
// is a MIME type, the candidates are `.desktop` files, and the default is the
// `mimeapps.list` that `xdg-mime` maintains.

#[cfg(all(unix, not(target_os = "macos")))]
mod freedesktop {
    use super::AppEntry;
    use super::OpenWith;
    use super::{canonical_mime, parse_aliases, parse_desktop, program_of};
    use std::collections::hash_map::Entry;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::process::Command;

    /// One `.desktop` file's main group, read for the few keys that matter here.
    struct Desktop {
        /// The file name, which is how the desktop names this application.
        id: String,
        path: PathBuf,
        name: String,
        exec: String,
        mimes: Vec<String>,
    }

    /// The directories a desktop environment scans for applications, in the
    /// order it prefers them.
    fn application_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let mut push = |base: PathBuf| {
            let candidate = base.join("applications");
            if candidate.is_dir() {
                dirs.push(candidate);
            }
        };
        if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
            push(PathBuf::from(data_home));
        } else if let Some(home) = std::env::var_os("HOME") {
            push(PathBuf::from(home).join(".local/share"));
        }
        let data_dirs = std::env::var("XDG_DATA_DIRS")
            .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
        for dir in data_dirs.split(':').filter(|dir| !dir.is_empty()) {
            push(PathBuf::from(dir));
        }
        dirs
    }

    fn all_desktops() -> Vec<Desktop> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        for dir in application_dirs() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(id) = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                else {
                    continue;
                };
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Some(entry) = parse_desktop(&text) else {
                    continue;
                };
                // Data dirs are ordered by preference, so the first entry for an
                // id wins and a later directory's override is not read twice.
                if !seen.insert(id.clone()) {
                    continue;
                }
                out.push(Desktop {
                    id,
                    path,
                    name: entry.name,
                    exec: entry.exec,
                    mimes: entry.mimes,
                });
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
        (!text.is_empty()).then_some(text)
    }

    /// MIME types have historic aliases and the two sides disagree: `xdg-mime`
    /// reports `application/java-archive` for a jar while the archive
    /// applications declare `application/x-java-archive`. The shared-mime-info
    /// database records which spelling is canonical, so both are normalised
    /// before comparing.
    fn mime_aliases() -> HashMap<String, String> {
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

        let mut aliases = HashMap::new();
        for root in roots {
            let Ok(text) = std::fs::read_to_string(root.join("mime/aliases")) else {
                continue;
            };
            for (alias, canonical) in parse_aliases(&text) {
                // The first database to define an alias wins, as with desktop ids.
                if let Entry::Vacant(slot) = aliases.entry(alias) {
                    slot.insert(canonical);
                }
            }
        }
        aliases
    }

    pub fn open_with(path: &str) -> OpenWith {
        let Some(mime) = xdg_mime(&["query", "filetype", path]) else {
            return OpenWith::unsupported(
                "",
                "Could not work out this file's type: xdg-mime is not available.",
            );
        };
        let aliases = mime_aliases();
        let wanted = canonical_mime(&mime, &aliases);
        let default_id = xdg_mime(&["query", "default", &mime]);
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

        // The current default is always offered, even when its entry does not
        // declare the type: the desktop says it opens these, and hiding it would
        // contradict the checkmark this dialog draws.
        if let Some(id) = &default_id {
            if !apps.iter().any(|app| &app.id == id) {
                let name = desktops
                    .iter()
                    .find(|desktop| &desktop.id == id)
                    .map(|desktop| desktop.name.clone())
                    .unwrap_or_else(|| id.clone());
                apps.push(AppEntry {
                    id: id.clone(),
                    name,
                });
            }
        }
        apps.sort_by_cached_key(|app| app.name.to_lowercase());

        let default_app = default_id;

        OpenWith {
            kind: mime,
            default_app,
            apps,
            unsupported: None,
            default_via_picker: false,
        }
    }

    /// Launch one application with the file.
    ///
    /// `gio launch` applies the desktop's own launching rules, including the
    /// `Exec` field codes, so it does the argument handling rather than this
    /// code getting it wrong on a path with a space in it. Only when `gio` is
    /// missing does this fall back to running the program directly.
    pub fn open_with_app(path: &str, app_id: &str) -> Result<(), String> {
        let desktop = all_desktops()
            .into_iter()
            .find(|desktop| desktop.id == app_id)
            .ok_or_else(|| format!("{app_id} is no longer installed."))?;

        if Command::new("gio")
            .arg("launch")
            .arg(&desktop.path)
            .arg(path)
            .spawn()
            .is_ok()
        {
            return Ok(());
        }

        let program = program_of(&desktop.exec)
            .ok_or_else(|| format!("{} has no command to run.", desktop.name))?;
        Command::new(program)
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| format!("Could not start {}: {err}", desktop.name))
    }

    pub fn set_default_app(path: &str, app_id: &str) -> Result<String, String> {
        let mime = xdg_mime(&["query", "filetype", path])
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
}

/// One `.desktop` file's main group, reduced to what "Open with" needs.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
struct DesktopEntry {
    name: String,
    exec: String,
    mimes: Vec<String>,
}

/// Parse a `.desktop` file, reading only its `[Desktop Entry]` group: the
/// localised groups that follow describe the same application again.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn parse_desktop(text: &str) -> Option<DesktopEntry> {
    let mut name = None;
    let mut exec = None;
    let mut mimes = Vec::new();
    let mut hidden = false;
    let mut is_application = false;
    let mut in_main = false;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Name" => name = name.or_else(|| Some(value.trim().to_string())),
            "Exec" => exec = Some(value.trim().to_string()),
            "MimeType" => {
                mimes = value
                    .split(';')
                    .map(str::trim)
                    .filter(|mime| !mime.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            "NoDisplay" | "Hidden" => hidden = value.trim().eq_ignore_ascii_case("true"),
            "Type" => is_application = value.trim().eq_ignore_ascii_case("Application"),
            _ => {}
        }
    }

    if hidden || !is_application {
        return None;
    }
    Some(DesktopEntry {
        name: name?,
        exec: exec?,
        mimes,
    })
}

/// Read a shared-mime-info `aliases` database into `(alias, canonical)` pairs.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn parse_aliases(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((alias, canonical)) = line.split_once(char::is_whitespace) {
            let canonical = canonical.trim();
            if !canonical.is_empty() {
                pairs.push((alias.trim().to_string(), canonical.to_string()));
            }
        }
    }
    pairs
}

/// The canonical spelling of a MIME type: aliases are resolved, unknown types are
/// left exactly as they came in.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn canonical_mime<'a>(
    mime: &'a str,
    aliases: &'a std::collections::HashMap<String, String>,
) -> &'a str {
    aliases.get(mime).map_or(mime, String::as_str)
}

// -------------------------------------------------------------- macOS ----
//
// LaunchServices holds three answers per UTI: every bundle that can act on it at
// all, the bundle that acts by default, and the bundle it was last set to. All
// three are reachable from C, so this needs no crate and no spawned helper.

#[cfg(target_os = "macos")]
mod launch_services {
    use super::AppEntry;
    use super::OpenWith;
    use super::extension_of;
    use std::ffi::{CString, c_char, c_long, c_void};
    use std::path::Path;
    use std::process::Command;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFArrayRef = *const c_void;
    type CFURLRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type CFIndex = c_long;
    type OSStatus = i32;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: CFAllocatorRef,
            text: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetLength(text: CFStringRef) -> CFIndex;
        fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
        fn CFStringGetCString(
            text: CFStringRef,
            buffer: *mut c_char,
            size: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> *const c_void;
        fn CFURLCopyFileSystemPath(url: CFURLRef, style: CFIndex) -> CFStringRef;
        fn CFRelease(value: CFTypeRef);
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn UTTypeCreatePreferredIdentifierForTag(
            tag_class: CFStringRef,
            tag: CFStringRef,
            conforming_to: CFStringRef,
        ) -> CFStringRef;
        fn LSCopyAllRoleHandlersForContentType(content: CFStringRef, roles: u32) -> CFArrayRef;
        fn LSCopyDefaultRoleHandlerForContentType(content: CFStringRef, roles: u32) -> CFStringRef;
        fn LSSetDefaultRoleHandlerForContentType(
            content: CFStringRef,
            roles: u32,
            handler: CFStringRef,
        ) -> OSStatus;
        fn LSCopyApplicationURLsForBundleIdentifier(
            bundle: CFStringRef,
            error: *mut CFTypeRef,
        ) -> CFArrayRef;
    }

    const UTF8: u32 = 0x0800_0100;
    /// `kLSRolesAll`: anything that opens or edits the type.
    const ROLES_ALL: u32 = 0xFFFF_FFFF;
    /// `kLSRolesViewer`: what would open it.
    const ROLES_VIEWER: u32 = 1;
    /// `kCFURLPOSIXPathStyle`.
    const POSIX_PATH: CFIndex = 0;
    /// The tag class a file extension belongs to, as the UTI system spells it.
    const TAG_CLASS_EXTENSION: &str = "public.filename-extension";

    /// A Core Foundation object this code owns, released when it drops. Every
    /// `Create`/`Copy` call returns one; every `Get` does not.
    struct Owned(CFTypeRef);

    impl Owned {
        fn new(value: CFTypeRef) -> Option<Self> {
            (!value.is_null()).then_some(Owned(value))
        }
        fn get(&self) -> CFTypeRef {
            self.0
        }
    }

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the pointer is non-null and came from a Create/Copy call.
            unsafe { CFRelease(self.0) };
        }
    }

    fn cfstring(text: &str) -> Option<Owned> {
        let c = CString::new(text).ok()?;
        // SAFETY: the string is NUL-terminated and outlives the call.
        Owned::new(unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) })
    }

    fn to_string(value: CFStringRef) -> Option<String> {
        if value.is_null() {
            return None;
        }
        // SAFETY: `value` is a live CFString for the whole call.
        unsafe {
            let length = CFStringGetLength(value);
            let capacity = CFStringGetMaximumSizeForEncoding(length, UTF8) + 1;
            let mut buffer = vec![0u8; capacity as usize];
            if CFStringGetCString(value, buffer.as_mut_ptr() as *mut c_char, capacity, UTF8) == 0 {
                return None;
            }
            let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
            String::from_utf8(buffer[..end].to_vec()).ok()
        }
    }

    /// The UTI the system resolves for this file's extension.
    fn uti_of(path: &str) -> Option<Owned> {
        let extension = extension_of(path);
        let bare = extension.strip_prefix('.').unwrap_or(&extension);
        if bare.is_empty() {
            return None;
        }
        let class = cfstring(TAG_CLASS_EXTENSION)?;
        let tag = cfstring(bare)?;
        // SAFETY: both strings are live and the third argument is optional.
        Owned::new(unsafe {
            UTTypeCreatePreferredIdentifierForTag(
                class.get() as CFStringRef,
                tag.get() as CFStringRef,
                std::ptr::null(),
            )
        })
    }

    /// The display name of a bundle: the `.app`'s own file name, which is what
    /// the Finder shows.
    fn bundle_name(bundle_id: &str) -> Option<String> {
        let bundle = cfstring(bundle_id)?;
        let mut error: CFTypeRef = std::ptr::null();
        // SAFETY: the id is live, and the error out-parameter may be written to.
        let urls = unsafe {
            LSCopyApplicationURLsForBundleIdentifier(bundle.get() as CFStringRef, &mut error)
        };
        let urls = Owned::new(urls)?;
        if !error.is_null() {
            // SAFETY: a CFError this call created; it is ours to release.
            unsafe { CFRelease(error) };
        }
        // SAFETY: the array is live and non-empty is checked by the index bound.
        let first = unsafe { CFArrayGetValueAtIndex(urls.get() as CFArrayRef, 0) };
        if first.is_null() {
            return None;
        }
        // SAFETY: the element is a CFURL owned by the array.
        let path = Owned::new(unsafe { CFURLCopyFileSystemPath(first as CFURLRef, POSIX_PATH) })?;
        let path = to_string(path.get() as CFStringRef)?;
        Path::new(&path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
    }

    pub fn open_with(path: &str) -> OpenWith {
        let kind = extension_of(path);
        let Some(uti) = uti_of(path) else {
            return OpenWith::unsupported(
                &kind,
                "This file has no extension, so the system has no type to ask about.",
            );
        };
        let Some(uti_name) = to_string(uti.get() as CFStringRef) else {
            return OpenWith::unsupported(&kind, "The system could not name this file's type.");
        };

        // SAFETY: the UTI is live for both calls.
        let handlers =
            unsafe { LSCopyAllRoleHandlersForContentType(uti.get() as CFStringRef, ROLES_ALL) };
        let mut apps = Vec::new();
        if let Some(handlers) = Owned::new(handlers) {
            // SAFETY: the bound comes from the same array.
            let count = unsafe { CFArrayGetCount(handlers.get() as CFArrayRef) };
            for index in 0..count {
                // SAFETY: `index` is inside the array.
                let element =
                    unsafe { CFArrayGetValueAtIndex(handlers.get() as CFArrayRef, index) };
                let Some(id) = to_string(element as CFStringRef) else {
                    continue;
                };
                let name = bundle_name(&id).unwrap_or_else(|| id.clone());
                apps.push(AppEntry { id, name });
            }
        }
        apps.sort_by_cached_key(|app| app.name.to_lowercase());

        let default_id = {
            // SAFETY: the UTI is live.
            let handler = unsafe {
                LSCopyDefaultRoleHandlerForContentType(uti.get() as CFStringRef, ROLES_VIEWER)
            };
            Owned::new(handler).and_then(|owned| to_string(owned.get() as CFStringRef))
        };
        let default_app = default_id;

        OpenWith {
            kind: if kind.is_empty() { uti_name } else { kind },
            default_app,
            apps,
            unsupported: None,
            default_via_picker: false,
        }
    }

    /// Launch one bundle with the file. `open -b` is the same activation the
    /// Finder does, so the application gets the document the way it expects.
    pub fn open_with_app(path: &str, app_id: &str) -> Result<(), String> {
        let status = Command::new("open")
            .arg("-b")
            .arg(app_id)
            .arg(path)
            .status()
            .map_err(|err| format!("Could not start that application: {err}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("Could not open the file with {app_id}."))
        }
    }

    pub fn set_default_app(path: &str, app_id: &str) -> Result<String, String> {
        let uti = uti_of(path).ok_or_else(|| "This file has no type to set.".to_string())?;
        let bundle = cfstring(app_id)
            .ok_or_else(|| "That application is no longer installed.".to_string())?;
        // SAFETY: both strings are live for the call.
        let status = unsafe {
            LSSetDefaultRoleHandlerForContentType(
                uti.get() as CFStringRef,
                ROLES_ALL,
                bundle.get() as CFStringRef,
            )
        };
        if status == 0 {
            Ok(to_string(uti.get() as CFStringRef).unwrap_or_default())
        } else {
            Err(format!("The system refused the change ({status})."))
        }
    }
}

// ------------------------------------------------------------ Windows ----
//
// The list is readable from the registry; the default is not writable.

#[cfg(target_os = "windows")]
mod win {
    use super::AppEntry;
    use super::OpenWith;
    use super::extension_of;
    use super::program_of;
    use crate::shell;
    use std::collections::HashSet;
    use std::ffi::c_void;
    use std::path::Path;

    #[link(name = "version")]
    unsafe extern "system" {
        fn GetFileVersionInfoSizeW(file: *const u16, handle: *mut u32) -> u32;
        fn GetFileVersionInfoW(file: *const u16, handle: u32, len: u32, data: *mut c_void) -> i32;
        fn VerQueryValueW(
            block: *const c_void,
            sub: *const u16,
            out: *mut *mut c_void,
            len: *mut u32,
        ) -> i32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The name an executable gives itself: `FileDescription` in its version
    /// resource, which is the string the shell puts in its own menus. Most
    /// applications name themselves only here, so without it the list would read
    /// "firefox" where the desktop says "Firefox".
    pub(crate) fn description(program: &str) -> Option<String> {
        let file = wide(program);
        let mut ignored = 0u32;
        let size = unsafe { GetFileVersionInfoSizeW(file.as_ptr(), &mut ignored) };
        if size == 0 {
            return None;
        }
        let mut block = vec![0u8; size as usize];
        if unsafe { GetFileVersionInfoW(file.as_ptr(), 0, size, block.as_mut_ptr() as *mut c_void) }
            == 0
        {
            return None;
        }
        let data = block.as_ptr() as *const c_void;

        // The translation table names the language/code-page block to read.
        let translations_key = wide(r"\VarFileInfo\Translation");
        let mut translations: *mut c_void = std::ptr::null_mut();
        let mut translations_len = 0u32;
        if unsafe {
            VerQueryValueW(
                data,
                translations_key.as_ptr(),
                &mut translations,
                &mut translations_len,
            )
        } == 0
            || translations.is_null()
            || translations_len < 4
        {
            return None;
        }
        // SAFETY: the query reported at least one two-word pair at this address.
        let pair = unsafe { std::slice::from_raw_parts(translations as *const u16, 2) };
        let key = wide(&format!(
            r"\StringFileInfo\{:04x}{:04x}\FileDescription",
            pair[0], pair[1]
        ));
        let mut text: *mut c_void = std::ptr::null_mut();
        let mut text_len = 0u32;
        if unsafe { VerQueryValueW(data, key.as_ptr(), &mut text, &mut text_len) } == 0
            || text.is_null()
            || text_len == 0
        {
            return None;
        }
        // SAFETY: the query reported `text_len` UTF-16 units at this address.
        let units = unsafe { std::slice::from_raw_parts(text as *const u16, text_len as usize) };
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        let name = String::from_utf16_lossy(&units[..end]).trim().to_string();
        (!name.is_empty()).then_some(name)
    }

    /// The registry key that records the per-user default for an extension.
    const USER_CHOICE: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts";

    /// What a ProgID calls the application behind it.
    ///
    /// `HKCR\<ProgID>`'s own default value names the *document type* ("Text
    /// Document"), not the program, so it is the wrong answer here. The
    /// application block, then the command line, do name the program.
    fn app_name(prog_id: &str) -> String {
        if let Some(name) =
            shell::reg_string(&format!(r"{prog_id}\Application"), Some("ApplicationName"))
        {
            return name;
        }
        if let Some(command) = shell::reg_string(&format!(r"{prog_id}\shell\open\command"), None)
            && let Some(program) = program_of(&command)
        {
            if let Some(name) = description(program) {
                return name;
            }
            if let Some(stem) = Path::new(program)
                .file_stem()
                .and_then(|stem| stem.to_str())
            {
                return stem.to_string();
            }
        }
        prog_id.to_string()
    }

    /// The executable a ProgID runs, for spotting the same program registered
    /// twice under two ProgIDs.
    fn app_program(prog_id: &str) -> Option<String> {
        let command = shell::reg_string(&format!(r"{prog_id}\shell\open\command"), None)?;
        program_of(&command).map(|program| program.to_ascii_lowercase())
    }

    /// The ProgID the shell would use today, from the per-user choice.
    fn user_choice(extension: &str) -> Option<String> {
        shell::reg_user_string(
            &format!(r"{USER_CHOICE}\{extension}\UserChoice"),
            Some("ProgId"),
        )
    }

    pub fn open_with(path: &str) -> OpenWith {
        let extension = extension_of(path);
        if extension.is_empty() {
            // Windows associates by extension, so a file without one has no type
            // to put to the shell.
            return OpenWith::unsupported(
                "",
                "This file has no extension, so Windows has no type to ask about.",
            );
        }
        let default_prog_id = user_choice(&extension);

        let mut candidates: Vec<String> = Vec::new();
        // What the machine associates with the type, then everything that
        // declared itself for it.
        candidates.extend(shell::reg_string(&extension, None));
        candidates.extend(shell::reg_subkeys(&format!(r"{extension}\OpenWithProgids")));
        candidates.extend(shell::reg_subkeys(&format!(
            r"SystemFileAssociations\{extension}\OpenWithList"
        )));
        // Applications that accept the type list it under SupportedTypes.
        for exe in shell::reg_subkeys("Applications") {
            let key = format!(r"Applications\{exe}\SupportedTypes");
            if shell::reg_has_value(&key, &extension) {
                candidates.push(format!(r"Applications\{exe}"));
            }
        }
        candidates.extend(default_prog_id.clone());

        let mut apps: Vec<AppEntry> = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();
        // Two ProgIDs can point at one program; the list must not show it twice.
        let mut seen_programs: HashSet<String> = HashSet::new();
        for prog_id in candidates {
            if prog_id.is_empty() || !seen_ids.insert(prog_id.to_ascii_lowercase()) {
                continue;
            }
            if let Some(program) = app_program(&prog_id)
                && !seen_programs.insert(program)
            {
                continue;
            }
            // A ProgID with no command anywhere cannot be started, so it is not a
            // candidate: it is usually a document class, not an application.
            if !shell::reg_key_exists(&format!(r"{prog_id}\shell\open\command"))
                && !shell::reg_key_exists(&format!(r"{prog_id}\Application"))
            {
                continue;
            }
            apps.push(AppEntry {
                id: prog_id.clone(),
                name: app_name(&prog_id),
            });
        }
        apps.sort_by_cached_key(|app| app.name.to_lowercase());

        let default_app = default_prog_id;

        OpenWith {
            kind: extension,
            default_app,
            apps,
            unsupported: None,
            // UserChoice carries a hash the shell validates, so only its own
            // picker may rewrite it.
            default_via_picker: true,
        }
    }

    pub fn open_with_app(path: &str, app_id: &str) -> Result<(), String> {
        shell::open_as(app_id, path)
    }

    pub fn set_default_app(_path: &str, _app_id: &str) -> Result<String, String> {
        Err("Windows protects file associations; use its own picker to change the default.".into())
    }
}

// ------------------------------------------------------- other systems ----

#[cfg(not(any(unix, target_os = "windows")))]
mod other {
    use super::OpenWith;

    pub fn open_with(_path: &str) -> OpenWith {
        OpenWith::unsupported("", "This system has no application list to read.")
    }

    pub fn open_with_app(_path: &str, _app_id: &str) -> Result<(), String> {
        Err("This system cannot launch a chosen application.".into())
    }

    pub fn set_default_app(_path: &str, _app_id: &str) -> Result<String, String> {
        Err("This system cannot change a file association.".into())
    }
}

// ------------------------------------------------ the surface the UI calls ----

/// Which applications can open this file, and which one does today. This touches
/// the filesystem and, on Linux, runs `xdg-mime`, so call it off the UI thread.
#[cfg(target_os = "windows")]
pub(crate) use win::description as executable_description;

#[cfg(all(unix, not(target_os = "macos")))]
pub fn open_with(path: &str) -> OpenWith {
    freedesktop::open_with(path)
}

#[cfg(target_os = "macos")]
pub fn open_with(path: &str) -> OpenWith {
    launch_services::open_with(path)
}

#[cfg(target_os = "windows")]
pub fn open_with(path: &str) -> OpenWith {
    win::open_with(path)
}

#[cfg(not(any(unix, target_os = "windows")))]
pub fn open_with(path: &str) -> OpenWith {
    other::open_with(path)
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn open_with_app(path: &str, app_id: &str) -> Result<(), String> {
    freedesktop::open_with_app(path, app_id)
}

#[cfg(target_os = "macos")]
pub fn open_with_app(path: &str, app_id: &str) -> Result<(), String> {
    launch_services::open_with_app(path, app_id)
}

#[cfg(target_os = "windows")]
pub fn open_with_app(path: &str, app_id: &str) -> Result<(), String> {
    win::open_with_app(path, app_id)
}

#[cfg(not(any(unix, target_os = "windows")))]
pub fn open_with_app(path: &str, app_id: &str) -> Result<(), String> {
    other::open_with_app(path, app_id)
}

/// Make one application the default for this file's type. Returns the type it
/// changed, for the confirmation the status bar shows.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn set_default_app(path: &str, app_id: &str) -> Result<String, String> {
    freedesktop::set_default_app(path, app_id)
}

#[cfg(target_os = "macos")]
pub fn set_default_app(path: &str, app_id: &str) -> Result<String, String> {
    launch_services::set_default_app(path, app_id)
}

#[cfg(target_os = "windows")]
pub fn set_default_app(path: &str, app_id: &str) -> Result<String, String> {
    win::set_default_app(path, app_id)
}

#[cfg(not(any(unix, target_os = "windows")))]
pub fn set_default_app(path: &str, app_id: &str) -> Result<String, String> {
    other::set_default_app(path, app_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_entry_is_read_only_from_its_main_group() {
        let entry = parse_desktop(
            "[Desktop Entry]\nType=Application\nName=Text Editor\nExec=editor %F\nMimeType=text/plain;text/csv;\n\n[Desktop Action new]\nName=New\nExec=editor\n",
        )
        .expect("should parse");
        assert_eq!(entry.name, "Text Editor");
        assert_eq!(entry.exec, "editor %F");
        assert_eq!(entry.mimes, vec!["text/plain", "text/csv"]);
    }

    #[test]
    fn hidden_and_non_application_entries_are_skipped() {
        assert!(
            parse_desktop(
                "[Desktop Entry]\nType=Application\nName=Hidden\nExec=x\nNoDisplay=true\n"
            )
            .is_none(),
            "NoDisplay must be skipped"
        );
        assert!(
            parse_desktop("[Desktop Entry]\nType=Link\nName=Link\nExec=x\n").is_none(),
            "only Application entries count"
        );
        assert!(
            parse_desktop("[Desktop Entry]\nType=Application\nName=No Exec\n").is_none(),
            "an entry with no command cannot be launched"
        );
    }

    #[test]
    fn an_alias_resolves_to_the_canonical_type() {
        let aliases = parse_aliases(
            "# comment\napplication/x-java-archive application/java-archive\n\ntext/x-vcard text/vcard\n",
        )
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            canonical_mime("application/x-java-archive", &aliases),
            "application/java-archive"
        );
        assert_eq!(
            canonical_mime("text/plain", &aliases),
            "text/plain",
            "unknown types are left alone"
        );
    }

    #[test]
    fn the_program_is_the_first_token_even_when_quoted() {
        assert_eq!(program_of("editor %F"), Some("editor"));
        assert_eq!(
            program_of("\"/opt/My App/bin/run\" %f"),
            Some("/opt/My App/bin/run")
        );
        assert_eq!(program_of("   "), None);
        assert_eq!(program_of(""), None);
    }

    #[test]
    fn an_extension_is_dotted_and_lower_case() {
        assert_eq!(extension_of(r"C:\Users\me\Notes.TXT"), ".txt");
        assert_eq!(extension_of("/home/me/notes.TXT"), ".txt");
        assert_eq!(extension_of("/home/me/README"), "");
        assert_eq!(extension_of("/home/me/.bashrc"), "");
    }
}
