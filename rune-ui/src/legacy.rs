//! Read the favorites and pins the 0.2 application left behind.
//!
//! 0.2 was a Tauri window, so its settings lived in the webview's own local
//! storage under the page origin `http://tauri.localhost`, keyed
//! `lyre.library.v1`, rather than in a file Lyre owned. 0.3 is a native window
//! and keeps its settings in `prefs.json` (`rune::prefs`), so the first 0.3
//! launch found nothing to read and saved its own empty lists: the favorites and
//! the pinned folders were not lost by a bug in the save, they were never read.
//! This module finds that old value once so they can be carried over.
//!
//! The store's bytes are read directly rather than through a database library,
//! because the platforms that ran 0.2 keep it in two different formats:
//! Chromium's LevelDB log on Windows, and a WebKit SQLite file on Linux and
//! macOS. Both hold the same JSON under the same key, an ASCII string in the
//! first and UTF-16LE in the second, so a scan for the key in each encoding and
//! then for the object after it reads either without a dependency.
//!
//! Nothing here is required for the window to open: a missing, unreadable or
//! unexpected store simply imports nothing.

use std::path::{Path, PathBuf};

/// The identifier the Tauri application was built with, which is also the name
/// of the folder its webview kept its data in.
const APP_ID: &str = "app.lyre.files";

/// The key `src/lib/library.ts` saved under in the old application.
const KEY: &str = "lyre.library.v1";

/// How much of a store file is read, and how many files are tried. A webview
/// profile holds a handful of small files; the caps keep a large or unrelated
/// directory from turning startup into a crawl.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FILES: usize = 64;

/// What the old application remembered about the person's own folders.
#[derive(Default)]
pub struct Library {
    /// Paths on the old Quick access page.
    pub favorites: Vec<String>,
    /// Built-in rail rows the person removed.
    pub hidden: Vec<String>,
    /// Rail rows the person added.
    pub pins: Vec<String>,
}

impl Library {
    fn is_empty(&self) -> bool {
        self.favorites.is_empty() && self.hidden.is_empty() && self.pins.is_empty()
    }
}

/// Read the old application's library, if this machine still has it.
pub fn read() -> Option<Library> {
    for dir in stores() {
        let Some(library) = read_dir(&dir) else { continue };
        if !library.is_empty() {
            return Some(library);
        }
    }
    None
}

/// Where each platform's webview kept local storage for the 0.2 application.
///
/// Windows ran the Chromium backend of WebView2, Linux and macOS the WebKit one
/// that Tauri is built on there. A path that does not exist is skipped, so the
/// list can carry the layouts of every platform without harm.
fn stores() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local").join("share")));
    // WebView2: one profile, one LevelDB directory.
    if let Some(local) = &local {
        out.push(local.join(APP_ID).join("EBWebView").join("Default").join("Local Storage").join("leveldb"));
    }
    // WebKitGTK: one SQLite file per origin.
    if let Some(data) = &data {
        out.push(data.join(APP_ID).join("localstorage"));
    }
    // WKWebView for a bundled application, and its sandboxed container.
    if let Some(home) = &home {
        out.push(home.join("Library").join("WebKit").join(APP_ID).join("WebsiteData").join("LocalStorage"));
        out.push(
            home.join("Library")
                .join("Containers")
                .join(APP_ID)
                .join("Data")
                .join("Library")
                .join("WebKit")
                .join("WebsiteData")
                .join("LocalStorage"),
        );
    }
    out
}

/// Read every file in one store directory, oldest format first, and return the
/// first library that parses.
fn read_dir(dir: &Path) -> Option<Library> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .take(MAX_FILES)
        .collect();
    // LevelDB keeps its most recent writes in .log and moves older ones to
    // .ldb, and a key that was set twice lives in both. Newest first, so the
    // last thing the old application saved is the thing imported.
    files.sort_by_key(|path| {
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
        match extension {
            "log" => 0,
            "ldb" => 1,
            _ => 2,
        }
    });
    for path in files {
        if let Some(library) = read_file(&path) {
            return Some(library);
        }
    }
    None
}

/// Pull the library out of one store file.
fn read_file(path: &Path) -> Option<Library> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() == 0 || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    from_bytes(&bytes)
}

/// Find the library in a store file's bytes.
fn from_bytes(bytes: &[u8]) -> Option<Library> {
    // The key is written as plain ASCII by the Chromium backend and as UTF-16LE
    // by the WebKit one; the value after it is in the same encoding.
    if let Some(library) = scan(bytes, &[KEY.as_bytes().to_vec()], |data| data.to_vec()) {
        return Some(library);
    }
    let utf16: Vec<u8> = KEY.bytes().flat_map(|b| [b, 0]).collect();
    scan(bytes, &[utf16], |data| {
        let units: Vec<u16> = (0..data.len() / 2)
            .map(|i| u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]))
            .collect();
        String::from_utf16_lossy(&units).into_bytes()
    })
}

/// Look for the key anywhere in the file and try the JSON object that follows
/// each occurrence, since a store also holds the key inside its own index.
fn scan(
    bytes: &[u8],
    needles: &[Vec<u8>],
    decode: impl Fn(&[u8]) -> Vec<u8>,
) -> Option<Library> {
    for needle in needles {
        let mut from = 0;
        while let Some(at) = find(&bytes[from..], needle) {
            let start = from + at + needle.len();
            from = start;
            let window = &bytes[start..bytes.len().min(start + 64 * 1024)];
            let text = decode(window);
            let Some(object) = first_object(&text) else { continue };
            if let Some(library) = parse(object) {
                return Some(library);
            }
        }
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// The first balanced `{...}` in `text`, with string literals respected so a
/// brace inside a folder name cannot end the object early.
fn first_object(text: &[u8]) -> Option<&[u8]> {
    let start = text.iter().position(|b| *b == b'{')?;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (i, ch) in text.iter().enumerate().skip(start) {
        if quoted {
            if escaped {
                escaped = false;
            } else if *ch == b'\\' {
                escaped = true;
            } else if *ch == b'"' {
                quoted = false;
            }
            continue;
        }
        match ch {
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse one library object. Anything unreadable is treated as absent, as the
/// old application itself did: a corrupt store must never stop the window.
fn parse(object: &[u8]) -> Option<Library> {
    let value: serde_json::Value = serde_json::from_slice(object).ok()?;
    let object = value.as_object()?;
    // A store index can carry the key without a library beside it.
    if !object.contains_key("favorites") && !object.contains_key("extraPins") {
        return None;
    }
    Some(Library {
        favorites: strings(object.get("favorites")),
        hidden: strings(object.get("hiddenPins")),
        pins: strings(object.get("extraPins")),
    })
}

fn strings(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(|unit| unit.to_le_bytes()).collect()
    }

    /// The shape a Chromium local-storage log holds: the origin, the key, a
    /// length byte and a format byte, then the value as UTF-8.
    fn leveldb_record(value: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\x28_http://tauri.localhost\x00\x01");
        out.extend_from_slice(KEY.as_bytes());
        out.push(value.len() as u8 + 1);
        out.push(0x01);
        out.extend_from_slice(value.as_bytes());
        out
    }

    const SAMPLE: &str = r#"{"favorites":["C:\\Users\\me\\Projects"],"hiddenPins":["C:\\Users\\me\\Music"],"extraPins":["C:\\Projects","C:\\src"]}"#;

    #[test]
    fn a_chromium_log_yields_its_library() {
        let mut bytes = b"other\x00record".to_vec();
        bytes.extend(leveldb_record(SAMPLE));
        let library = from_bytes(&bytes).expect("the library is found");
        assert_eq!(library.favorites, vec![r"C:\Users\me\Projects"]);
        assert_eq!(library.pins, vec![r"C:\Projects", r"C:\src"]);
        assert_eq!(library.hidden, vec![r"C:\Users\me\Music"]);
    }

    #[test]
    fn a_webkit_file_yields_its_library() {
        // WebKit stores the origin, the key and the value as UTF-16LE.
        let mut bytes = Vec::new();
        bytes.extend(utf16("http://tauri.localhost"));
        bytes.extend(utf16(KEY));
        bytes.extend(utf16(SAMPLE));
        let library = from_bytes(&bytes).expect("the library is found");
        assert_eq!(library.pins, vec![r"C:\Projects", r"C:\src"]);
    }

    #[test]
    fn a_store_without_the_key_imports_nothing() {
        assert!(from_bytes(b"http://tauri.localhost\x00\x01lyre.theme\x01system").is_none());
    }

    #[test]
    fn a_brace_in_a_path_does_not_end_the_object() {
        let value = r#"{"favorites":["C:\\odd\\{name}"],"hiddenPins":[],"extraPins":[]}"#;
        let mut bytes = leveldb_record(value);
        bytes.extend_from_slice(b"trailing");
        let library = from_bytes(&bytes).expect("the library is found");
        assert_eq!(library.favorites, vec![r"C:\odd\{name}"]);
    }

}
