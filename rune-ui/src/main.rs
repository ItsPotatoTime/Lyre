//! Lyre's Explorer shell, drawn natively by Rune.
//!
//! Additive experiment on `feat/rune-native-ui`: the Windows 11 Explorer shell
//! from the reference captures, rendered by Rune's GPU renderer instead of a
//! Tauri webview. The default window reads a real directory; named scenes
//! reproduce the four reference views so a headless PNG can be compared to them.
//!
//!   bun dev:rune
//!   bun buildit --rune
//!   cargo run -- --snap downloads.png --scene downloads --size 1076x600 --scale 1
//!
//! Out of scope for this slice: independent tab history, file operations, OS
//! drag-and-drop, system context menus, real shell thumbnails, and the reveal
//! highlight.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use rune::color::hex;
use rune::{
    Align, AppSpec, Color, Cx, El, GrowthSpec, LineCap, Live, Phase, ScrollSpec, Shape, Spring,
    TextEdit, TextStyle, Tx, WindowOptions, WindowSpec, canvas, col, el, row, run_windows, text,
    text_input,
};

// Metrics and colours sampled from the four reference captures. The chrome is
// three distinct bands, not one surface: a near-black tab strip, a Mica-tinted
// address row, and a command bar that shares the work area's neutral charcoal.
const TAB_H: f32 = 38.0;
const TAB_ITEM_H: f32 = 30.0;
const ADDRESS_H: f32 = 46.0;
const COMMAND_H: f32 = 44.0;
const STATUS_H: f32 = 24.0;
const RAIL_W: f32 = 155.0;
const HEADER_H: f32 = 32.0;
const GROUP_H: f32 = 28.0;
const ROW_H: f32 = 28.0;
const FIELD_H: f32 = 30.0;
const PREVIEW_W: f32 = 300.0;
const GUTTER_W: f32 = 12.0;
const NAME_W: f32 = 260.0;
const DATE_W: f32 = 140.0;
const TYPE_W: f32 = 120.0;
const SIZE_W: f32 = 80.0;

// Explorer sets its body and chrome text in 14px Segoe UI; only the quiet
// secondary lines (status, "Stored locally", preview details) drop to 12px.
const CAPTION: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
const UI_TEXT: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0);
const ROW_TEXT: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0);
const TILE_TEXT: TextStyle = TextStyle::new(14.0, 600.0).lh(20.0);
const GROUP_TEXT: TextStyle = TextStyle::new(14.0, 600.0).lh(20.0);

fn c(s: &str) -> Color {
    hex(s).expect("authored hex literal")
}

#[derive(Clone, Copy)]
struct Paint {
    title: Color,
    tab: Color,
    toolbar: Color,
    command: Color,
    content: Color,
    status: Color,
    field: Color,
    field_line: Color,
    line: Color,
    soft_line: Color,
    text: Color,
    secondary: Color,
    tertiary: Color,
    disabled: Color,
    accent: Color,
    accent_fill: Color,
    accent_ink: Color,
    hover: Color,
    press: Color,
    rail_sel: Color,
    row_sel: Color,
    sel_line: Color,
    focus_line: Color,
    scroll_thumb: Color,
    thumb_bg: Color,
    pill_off: Color,
    close_hover: Color,
    hair: f32,
}

fn paint(dark: bool, scale: f32) -> Paint {
    let hair = 1.0 / scale;
    if dark {
        Paint {
            title: c("#000d17"),
            tab: c("#202429"),
            toolbar: c("#1a2127"),
            command: c("#191919"),
            content: c("#191919"),
            status: c("#1c1c1c"),
            field: c("#272e33"),
            field_line: c("#3a4148"),
            line: c("#3a3a3a"),
            soft_line: c("#2b2b2b"),
            text: c("#ffffff"),
            secondary: c("#e3e3e3"),
            tertiary: c("#adadad"),
            disabled: c("#6e6e6e"),
            accent: c("#4cc2ff"),
            accent_fill: c("#0078d4"),
            accent_ink: c("#003048"),
            hover: Color::WHITE.alpha(0.055),
            press: Color::WHITE.alpha(0.028),
            rail_sel: c("#2d2d2d"),
            row_sel: c("#2a2d31"),
            sel_line: c("#454545"),
            focus_line: c("#505050"),
            scroll_thumb: c("#6e6e6e"),
            thumb_bg: c("#222222"),
            pill_off: c("#272727"),
            close_hover: c("#c42b1c"),
            hair,
        }
    } else {
        Paint {
            title: c("#e9edf2"),
            tab: c("#ffffff"),
            toolbar: c("#f4f6f8"),
            command: c("#fbfbfb"),
            content: c("#ffffff"),
            status: c("#f3f3f3"),
            field: c("#ffffff"),
            field_line: c("#dfe3e8"),
            line: c("#e2e2e2"),
            soft_line: c("#ededed"),
            text: c("#1b1b1b"),
            secondary: c("#484848"),
            tertiary: c("#6c6c6c"),
            disabled: c("#a2a2a2"),
            accent: c("#005fb8"),
            accent_fill: c("#0067c0"),
            accent_ink: c("#ffffff"),
            hover: Color::BLACK.alpha(0.045),
            press: Color::BLACK.alpha(0.025),
            rail_sel: c("#eaeaea"),
            row_sel: c("#e9f2fc"),
            sel_line: c("#c8dcef"),
            focus_line: c("#909090"),
            scroll_thumb: c("#b4b4b4"),
            thumb_bg: c("#f0f0f0"),
            pill_off: c("#f0f0f0"),
            close_hover: c("#c42b1c"),
            hair,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Folder,
    Drive,
    Image,
    Video,
    Audio,
    Pdf,
    Document,
    Sheet,
    Slides,
    Code,
    Text,
    Archive,
    App,
    Other,
}

fn classify(name: &str, is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" => Kind::Image,
        "mp4" | "mkv" | "mov" | "avi" | "webm" => Kind::Video,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" => Kind::Audio,
        "pdf" => Kind::Pdf,
        "doc" | "docx" | "odt" | "rtf" => Kind::Document,
        "xls" | "xlsx" | "csv" | "ods" => Kind::Sheet,
        "ppt" | "pptx" | "odp" => Kind::Slides,
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "c" | "h" | "cpp" | "json" | "toml"
        | "yml" | "yaml" | "css" | "html" | "md" | "sh" => Kind::Code,
        "txt" | "log" | "ini" | "cfg" => Kind::Text,
        "zip" | "7z" | "rar" | "gz" | "tar" | "bz2" | "xz" => Kind::Archive,
        "exe" | "msi" | "appx" | "app" | "deb" | "rpm" => Kind::App,
        _ => Kind::Other,
    }
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Folder => "File folder",
        Kind::Drive => "Local Disk",
        Kind::Image => "PNG File",
        Kind::Video => "Video",
        Kind::Audio => "Audio",
        Kind::Pdf => "PDF document",
        Kind::Document => "Document",
        Kind::Sheet => "Spreadsheet",
        Kind::Slides => "Presentation",
        Kind::Code => "Source file",
        Kind::Text => "Text document",
        Kind::Archive => "Compressed Archive",
        Kind::App => "Application",
        Kind::Other => "File",
    }
}

/// Installers are the one place Explorer shows a different mark per file; a
/// stable name hash stands in for each application's own icon.
fn app_color(name: &str) -> &'static str {
    const PALETTE: [&str; 6] = [
        "#3b82c4", "#e0518c", "#1f9e6e", "#8b5cf6", "#e0642a", "#2f6fd0",
    ];
    let h = name
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    PALETTE[(h as usize) % PALETTE.len()]
}

#[derive(Clone)]
struct Entry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    modified: String,
    type_label: String,
    kind: Kind,
    /// Fixtures carry display-ready size strings; real scans carry byte counts.
    fixture: bool,
    fixture_size: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Home,
    Files,
    ThisPc,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Details,
    Grid,
}

#[derive(Clone, Copy)]
enum Fixture {
    Home,
    Downloads,
    Screenshots,
    ThisPc,
}

struct App {
    dark: bool,
    view: View,
    mode: Mode,
    preview: bool,
    contextual: bool,
    root: PathBuf,
    crumbs: Vec<String>,
    search: String,
    tab: String,
    entries: Vec<Entry>,
    /// (row index where the group starts, label).
    groups: Vec<(usize, String)>,
    /// Row indices into `entries` after search and sort. Never built rows.
    visible: Vec<usize>,
    selected: Option<usize>,
    focused: Option<usize>,
    nav: usize,
    count: usize,
    selection_detail: String,
    notice: String,
    search_edit: TextEdit,
    last_query: String,
    history: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    sort: u8,
    /// Fixture views arrive in the authored order of the reference captures;
    /// only a real directory scan is sorted by the active column.
    keep_order: bool,
    /// Home/This PC report the authored item count, not just the rows drawn.
    count_override: Option<usize>,
    zoom: bool,
    scan_ms: f64,
    /// Home's Recent/Favorites pill pair.
    home_tab: u8,
    /// The drawn scrollbar rides Live bindings so a scroll never re-runs the
    /// view: the viewport reports its offset, the thumb follows in-frame.
    scroll_track: Live<[f32; 4]>,
    scroll_thumb: Live<[f32; 4]>,
    scroll_vis: Live<f32>,
    scroll_max: Live<f32>,
    scroll_off: Live<f32>,
}

fn format_size(n: u64) -> String {
    if n < 1024 {
        format!("{n} bytes")
    } else if n < 1024 * 1024 {
        format!("{:.0} KB", n as f64 / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

fn format_time(secs: u64) -> String {
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let within = secs % 86_400;
    let h = within / 3600;
    let ampm = if h >= 12 { "PM" } else { "AM" };
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{m}/{d}/{y}  {h12}:{:02} {ampm}", (within % 3600) / 60)
}

fn scan(path: &Path) -> (Vec<Entry>, f64) {
    let start = Instant::now();
    let mut entries = Vec::new();
    if let Ok(reader) = fs::read_dir(path) {
        for item in reader.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(meta) = item.metadata() else { continue };
            let is_dir = meta.is_dir();
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| format_time(d.as_secs()))
                .unwrap_or_default();
            let kind = classify(&name, is_dir);
            entries.push(Entry {
                name: name.clone(),
                path: item.path().to_string_lossy().into_owned(),
                is_dir,
                size: if is_dir { 0 } else { meta.len() },
                modified,
                type_label: kind_label(kind).to_string(),
                kind,
                fixture: false,
                fixture_size: String::new(),
            });
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    (entries, start.elapsed().as_secs_f64() * 1000.0)
}

impl Entry {
    fn size_label(&self) -> String {
        if self.is_dir {
            String::new()
        } else if self.fixture {
            self.fixture_size.clone()
        } else {
            format_size(self.size)
        }
    }
}

fn make_entry(name: &str, modified: &str, type_label: &str, size: &str, kind: Kind) -> Entry {
    Entry {
        name: name.to_string(),
        path: name.to_string(),
        is_dir: kind == Kind::Folder,
        size: 0,
        modified: modified.to_string(),
        type_label: type_label.to_string(),
        kind,
        fixture: true,
        fixture_size: size.to_string(),
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn place_path(index: usize) -> Option<PathBuf> {
    let home = home_dir()?;
    let sub = match index {
        1 | 6 => "Pictures",
        3 => "Desktop",
        4 => "Downloads",
        5 => "Documents",
        7 => "Music",
        8 => "Videos",
        _ => return None,
    };
    Some(home.join(sub))
}

/// Recompute the row order after a scan, a search keystroke, or a sort click.
/// The list stays virtualised: this only holds indices, never built rows.
fn refresh_visible(s: &mut App) {
    let q = s.search_edit.text.to_lowercase();
    let mut order: Vec<usize> = (0..s.entries.len())
        .filter(|&i| q.is_empty() || s.entries[i].name.to_lowercase().contains(&q))
        .collect();
    if !s.keep_order {
        match s.sort {
            1 => order.sort_by(|&a, &b| s.entries[a].modified.cmp(&s.entries[b].modified)),
            2 => order.sort_by(|&a, &b| s.entries[a].type_label.cmp(&s.entries[b].type_label)),
            3 => order.sort_by_key(|&i| s.entries[i].size),
            _ => order.sort_by(|&a, &b| {
                (!s.entries[a].is_dir, s.entries[a].name.to_lowercase())
                    .cmp(&(!s.entries[b].is_dir, s.entries[b].name.to_lowercase()))
            }),
        }
    }
    s.count = s.count_override.unwrap_or(order.len());
    s.visible = order;
}

/// Rune calls this before each frame. Search edits land in `TextEdit` through
/// `on_edit`, which only binds the field, so the row order is recomputed here.
fn poll(s: &mut App, _t: f64, _tx: &mut Tx) -> bool {
    if s.search_edit.text == s.last_query {
        return false;
    }
    s.last_query = s.search_edit.text.clone();
    refresh_visible(s);
    true
}

fn set_location(s: &mut App, path: PathBuf) {
    let (entries, scan_ms) = scan(&path);
    let crumbs: Vec<String> = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .filter(|part| part != "\\" && part != "/")
        .collect();
    let tab = crumbs.last().cloned().unwrap_or_else(|| "Lyre".into());
    s.root = path;
    s.entries = entries;
    s.scan_ms = scan_ms;
    s.crumbs = crumbs;
    s.tab = tab.clone();
    s.search = format!("Search {tab}");
    s.search_edit.text.clear();
    s.view = View::Files;
    s.mode = Mode::Details;
    s.preview = false;
    s.contextual = false;
    s.groups.clear();
    s.selected = None;
    s.focused = None;
    s.selection_detail.clear();
    s.notice.clear();
    s.sort = 0;
    s.keep_order = false;
    s.count_override = None;
    refresh_visible(s);
}

fn go(s: &mut App, path: PathBuf) {
    // Home and This PC are fixture overlays that keep the last real root, so a
    // return to that same folder must still reload; only a no-op while already
    // showing it as a file list.
    if path == s.root && s.view == View::Files {
        return;
    }
    s.history.push(s.root.clone());
    s.forward.clear();
    set_location(s, path);
}

fn go_back(s: &mut App) {
    if let Some(previous) = s.history.pop() {
        s.forward.push(s.root.clone());
        set_location(s, previous);
    } else {
        s.notice = "Nothing to go back to".into();
    }
}

fn go_forward(s: &mut App) {
    if let Some(next) = s.forward.pop() {
        s.history.push(s.root.clone());
        set_location(s, next);
    } else {
        s.notice = "Nothing to go forward to".into();
    }
}

fn go_up(s: &mut App) {
    if let Some(parent) = s.root.parent().map(Path::to_path_buf) {
        go(s, parent);
    } else {
        s.notice = "Already at the root".into();
    }
}

fn navigate(s: &mut App, index: usize) {
    match index {
        0 => load(s, Fixture::Home),
        9 => load(s, Fixture::ThisPc),
        10 => s.notice = "Network browsing is not implemented in this slice".into(),
        i => {
            s.nav = i;
            match place_path(i) {
                Some(path) if path.is_dir() => go(s, path),
                Some(path) => s.notice = format!("{} is not available here", path.display()),
                None => s.notice = "This place is not available here".into(),
            }
        }
    }
}

fn load(s: &mut App, f: Fixture) {
    s.groups.clear();
    s.preview = false;
    s.contextual = false;
    s.mode = Mode::Details;
    s.selected = None;
    s.focused = None;
    s.notice.clear();
    s.search_edit.text.clear();
    match f {
        Fixture::Home => {
            s.view = View::Home;
            s.tab = "Home".into();
            s.crumbs = vec!["Home".into()];
            s.search = "Search Home".into();
            s.nav = 0;
            s.count = 8;
            s.selection_detail.clear();
            s.entries = vec![
                make_entry(
                    "cairn-export-20261101.tar",
                    "11/6/2026  9:45 AM",
                    "Downloads",
                    "",
                    Kind::Archive,
                ),
                make_entry(
                    "HonorWorkStation 5.5.1.54(C233HONORSP5)_Client Softw...",
                    "11/5/2026 10:12 PM",
                    "Downloads",
                    "",
                    Kind::Archive,
                ),
            ];
        }
        Fixture::Downloads => {
            s.view = View::Files;
            s.tab = "Downloads".into();
            s.crumbs = vec!["Downloads".into()];
            s.search = "Search Downloads".into();
            s.nav = 4;
            s.count = 16;
            s.selection_detail.clear();
            let today = "11/6/2026  9:47 AM";
            let yest = "11/5/2026  10:33 PM";
            s.entries = vec![
                make_entry(
                    "Brisk-v2.3.9-windows-x86_64",
                    today,
                    "Application",
                    "21,774 KB",
                    Kind::App,
                ),
                make_entry(
                    "Firefox Installer",
                    today,
                    "Application",
                    "497 KB",
                    Kind::App,
                ),
                make_entry(
                    "cairn-export-20261101.tar",
                    today,
                    "Compressed Archi...",
                    "1,813 KB",
                    Kind::Archive,
                ),
                make_entry(
                    "Brisk",
                    "11/6/2026 10:27 AM",
                    "File folder",
                    "",
                    Kind::Folder,
                ),
                make_entry(
                    "WorkBuddy-win32-x64-user-5.9.4.425936...",
                    yest,
                    "Application",
                    "495,154 KB",
                    Kind::App,
                ),
                make_entry(
                    "WarpSetup",
                    "11/5/2026 10:32 PM",
                    "Application",
                    "132,857 KB",
                    Kind::App,
                ),
                make_entry(
                    "VencordInstaller",
                    "11/5/2026 10:27 PM",
                    "Application",
                    "11,295 KB",
                    Kind::App,
                ),
                make_entry(
                    "windowsdesktop-runtime-6.0.36-win-x64",
                    "11/5/2026 10:24 PM",
                    "Application",
                    "56,036 KB",
                    Kind::App,
                ),
                make_entry(
                    "Bootstrap-v2.11.4",
                    "11/5/2026 10:24 PM",
                    "Application",
                    "12,295 KB",
                    Kind::App,
                ),
                make_entry(
                    "DiscordSetup",
                    "11/5/2026 10:18 PM",
                    "Application",
                    "156,756 KB",
                    Kind::App,
                ),
                make_entry(
                    "Heroic-2.2.4-Setup-x64",
                    "11/5/2026 10:09 PM",
                    "Application",
                    "151,202 KB",
                    Kind::App,
                ),
                make_entry(
                    "AnkerLauncher-win-Setup",
                    "11/5/2026 10:09 PM",
                    "Application",
                    "37,290 KB",
                    Kind::App,
                ),
                make_entry(
                    "HonorWorkStation 5.5.1.54(C233HONOR...",
                    "11/5/2026 10:08 PM",
                    "ZIP File",
                    "962,092 KB",
                    Kind::Archive,
                ),
                make_entry(
                    "fdm_x64_setup",
                    "11/5/2026 10:06 PM",
                    "Application",
                    "50,923 KB",
                    Kind::App,
                ),
                make_entry(
                    "Git-2.51.0-64-bit",
                    "11/5/2026  9:58 PM",
                    "Application",
                    "68,412 KB",
                    Kind::App,
                ),
                make_entry(
                    "node-v24.11.0-x64",
                    "11/5/2026  9:51 PM",
                    "Application",
                    "31,026 KB",
                    Kind::App,
                ),
            ];
            s.groups = vec![(0, "Today".into()), (4, "Yesterday".into())];
        }
        Fixture::Screenshots => {
            s.view = View::Files;
            s.mode = Mode::Grid;
            s.preview = true;
            s.contextual = true;
            s.tab = "Screenshots".into();
            s.crumbs = vec!["Pictures".into(), "Screenshots".into()];
            s.search = "Search Screenshots".into();
            s.nav = 6;
            s.count = 5;
            s.selected = Some(1);
            s.focused = Some(1);
            s.selection_detail = "1 item selected  131 KB".into();
            let d = "11/6/2026 10:39 AM";
            s.entries = vec![
                make_entry(
                    "Screenshot 2026-11-06 095523",
                    d,
                    "PNG File",
                    "131 KB",
                    Kind::Image,
                ),
                make_entry(
                    "Screenshot 2026-11-06 103957",
                    d,
                    "PNG File",
                    "131 KB",
                    Kind::Image,
                ),
                make_entry(
                    "Screenshot 2026-11-06 104522",
                    d,
                    "PNG File",
                    "131 KB",
                    Kind::Image,
                ),
                make_entry(
                    "Screenshot 2026-11-06 111606",
                    d,
                    "PNG File",
                    "131 KB",
                    Kind::Image,
                ),
                make_entry(
                    "Screenshot 2026-11-06 111619",
                    d,
                    "PNG File",
                    "131 KB",
                    Kind::Image,
                ),
            ];
        }
        Fixture::ThisPc => {
            s.view = View::ThisPc;
            s.preview = true;
            s.tab = "This PC".into();
            s.crumbs = vec!["This PC".into()];
            s.search = "Search This PC".into();
            s.nav = 9;
            s.count = 1;
            s.selection_detail.clear();
            s.entries = vec![make_entry(
                "Local Disk (C:)",
                "",
                "Local Disk",
                "",
                Kind::Drive,
            )];
        }
    }
    let authored_count = s.count;
    s.keep_order = true;
    s.count_override = Some(authored_count);
    s.sort = 0;
    refresh_visible(s);
}

fn init() -> App {
    let root = std::env::var("LYRE_RUNE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let (entries, scan_ms) = scan(&root);
    let crumbs: Vec<String> = root
        .components()
        .map(|p| p.as_os_str().to_string_lossy().into_owned())
        .filter(|s| s != "\\" && s != "/")
        .collect();
    let tab = crumbs.last().cloned().unwrap_or_else(|| "Lyre".into());
    let count = entries.len();
    let mut app = App {
        dark: true,
        view: View::Files,
        mode: Mode::Details,
        preview: false,
        contextual: false,
        root,
        crumbs,
        search: format!("Search {tab}"),
        tab,
        entries,
        groups: Vec::new(),
        visible: Vec::new(),
        selected: None,
        focused: None,
        nav: 9,
        count,
        selection_detail: String::new(),
        notice: String::new(),
        search_edit: TextEdit::default(),
        last_query: String::new(),
        history: Vec::new(),
        forward: Vec::new(),
        sort: 0,
        keep_order: false,
        count_override: None,
        zoom: false,
        scan_ms,
        home_tab: 0,
        scroll_track: Live::new([0.0; 4]),
        scroll_thumb: Live::new([0.0; 4]),
        scroll_vis: Live::new(0.0),
        scroll_max: Live::new(0.0),
        scroll_off: Live::new(0.0),
    };
    refresh_visible(&mut app);
    app
}

// ---------------------------------------------------------------- icons ----

fn circle(cx: f32, cy: f32, r: f32) -> Vec<[f32; 2]> {
    (0..16)
        .map(|i| {
            let a = i as f32 / 16.0 * std::f32::consts::TAU;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect()
}

fn stroke(points: Vec<[f32; 2]>, ink: Color) -> Shape {
    Shape::new().stroke(points, 1.3, ink)
}

fn closed(points: Vec<[f32; 2]>, ink: Color) -> Shape {
    Shape::new().stroke_with(points, true, 1.3, LineCap::Round, ink)
}

#[derive(Clone, Copy)]
enum Ico {
    New,
    Cut,
    Copy,
    Paste,
    Rename,
    Share,
    Delete,
    Sort,
    View,
    Filter,
    More,
    Details,
    Back,
    Forward,
    Up,
    Refresh,
    Search,
    ChevronRight,
    ChevronDown,
    Pin,
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
    ThisPc,
    Monitor,
    Minimize,
    Maximize,
    Restore,
    Close,
    Info,
    ShareSmall,
    SetBackground,
    RotateLeft,
    RotateRight,
    Star,
    Clock,
    Grid,
    List,
}

fn ico(p: Paint, which: Ico, size: f32, ink: Color) -> El {
    const W: f32 = 1.3;
    let sh = match which {
        Ico::New => closed(circle(12.0, 12.0, 8.0), ink)
            .line([12.0, 8.0], [12.0, 16.0], W, LineCap::Round, ink)
            .line([8.0, 12.0], [16.0, 12.0], W, LineCap::Round, ink),
        Ico::Cut => Shape::new()
            .stroke_with(circle(7.0, 17.0, 2.4), true, W, LineCap::Round, ink)
            .stroke_with(circle(17.0, 17.0, 2.4), true, W, LineCap::Round, ink)
            .stroke(vec![[9.0, 6.0], [16.0, 18.0]], W, ink)
            .stroke(vec![[15.0, 6.0], [8.0, 18.0]], W, ink),
        Ico::Copy => closed(
            vec![[4.0, 8.0], [14.0, 8.0], [14.0, 20.0], [4.0, 20.0]],
            ink,
        )
        .stroke(vec![[8.0, 4.0], [20.0, 4.0], [20.0, 16.0]], W, ink),
        Ico::Paste => closed(
            vec![[5.0, 6.0], [19.0, 6.0], [19.0, 20.0], [5.0, 20.0]],
            ink,
        )
        .stroke(vec![[9.0, 3.0], [15.0, 3.0]], W, ink)
        .line([9.0, 11.0], [15.0, 11.0], W, LineCap::Round, ink),
        Ico::Rename => closed(
            vec![[4.0, 5.0], [20.0, 5.0], [20.0, 19.0], [4.0, 19.0]],
            ink,
        )
        .line([8.0, 12.0], [16.0, 12.0], W, LineCap::Round, ink),
        Ico::Share => closed(
            vec![[4.0, 10.0], [4.0, 20.0], [20.0, 20.0], [20.0, 10.0]],
            ink,
        )
        .line([12.0, 4.0], [12.0, 14.0], W, LineCap::Round, ink)
        .stroke(vec![[9.0, 7.0], [12.0, 4.0], [15.0, 7.0]], W, ink),
        Ico::Delete => stroke(vec![[4.0, 6.0], [20.0, 6.0]], ink)
            .stroke(
                vec![[6.0, 6.0], [7.0, 20.0], [17.0, 20.0], [18.0, 6.0]],
                W,
                ink,
            )
            .line([10.0, 10.0], [10.0, 17.0], W, LineCap::Round, ink)
            .line([14.0, 10.0], [14.0, 17.0], W, LineCap::Round, ink),
        Ico::Sort => stroke(
            vec![[7.0, 19.0], [7.0, 5.0], [4.0, 8.0], [7.0, 5.0], [10.0, 8.0]],
            ink,
        )
        .stroke(
            vec![
                [17.0, 5.0],
                [17.0, 19.0],
                [14.0, 16.0],
                [17.0, 19.0],
                [20.0, 16.0],
            ],
            W,
            ink,
        ),
        Ico::View => Shape::new()
            .stroke_with(
                vec![[4.0, 5.0], [8.0, 5.0], [8.0, 9.0], [4.0, 9.0]],
                true,
                W,
                LineCap::Round,
                ink,
            )
            .stroke(vec![[11.0, 7.0], [20.0, 7.0]], W, ink)
            .stroke_with(
                vec![[4.0, 14.0], [8.0, 14.0], [8.0, 18.0], [4.0, 18.0]],
                true,
                W,
                LineCap::Round,
                ink,
            )
            .stroke(vec![[11.0, 16.0], [20.0, 16.0]], W, ink),
        Ico::Filter => Shape::new().fill(
            vec![
                [3.0, 5.0],
                [21.0, 5.0],
                [14.0, 13.0],
                [14.0, 20.0],
                [10.0, 18.0],
                [10.0, 13.0],
            ],
            ink,
        ),
        Ico::More => Shape::new()
            .fill(circle(5.0, 12.0, 1.6), ink)
            .fill(circle(12.0, 12.0, 1.6), ink)
            .fill(circle(19.0, 12.0, 1.6), ink),
        Ico::Details => closed(
            vec![[3.0, 5.0], [21.0, 5.0], [21.0, 19.0], [3.0, 19.0]],
            ink,
        )
        .line([14.0, 5.0], [14.0, 19.0], W, LineCap::Round, ink),
        Ico::Back => stroke(vec![[20.0, 12.0], [5.0, 12.0]], ink).stroke(
            vec![[11.0, 6.0], [5.0, 12.0], [11.0, 18.0]],
            W,
            ink,
        ),
        Ico::Forward => stroke(vec![[4.0, 12.0], [19.0, 12.0]], ink).stroke(
            vec![[13.0, 6.0], [19.0, 12.0], [13.0, 18.0]],
            W,
            ink,
        ),
        Ico::Up => stroke(vec![[12.0, 20.0], [12.0, 4.0]], ink).stroke(
            vec![[6.0, 10.0], [12.0, 4.0], [18.0, 10.0]],
            W,
            ink,
        ),
        Ico::Refresh => {
            let arc: Vec<[f32; 2]> = (0..=24)
                .map(|k| {
                    let a = (-60.0 + 330.0 * k as f32 / 24.0).to_radians();
                    [12.0 + 7.0 * a.cos(), 12.0 + 7.0 * a.sin()]
                })
                .collect();
            let tip = arc[arc.len() - 1];
            Shape::new().stroke(arc, W, ink).fill(
                vec![
                    [tip[0] + 3.4, tip[1] + 0.6],
                    [tip[0] - 1.2, tip[1] - 2.9],
                    [tip[0] - 0.4, tip[1] + 3.0],
                ],
                ink,
            )
        }
        Ico::Search => closed(circle(10.5, 10.5, 6.0), ink).line(
            [15.0, 15.0],
            [20.0, 20.0],
            W,
            LineCap::Round,
            ink,
        ),
        Ico::ChevronRight => stroke(vec![[9.0, 6.0], [15.0, 12.0], [9.0, 18.0]], ink),
        Ico::ChevronDown => stroke(vec![[6.0, 9.0], [12.0, 15.0], [18.0, 9.0]], ink),
        Ico::Pin => Shape::new().fill(
            vec![
                [6.0, 3.0],
                [18.0, 3.0],
                [16.0, 9.0],
                [13.0, 11.0],
                [13.0, 17.0],
                [12.0, 21.0],
                [11.0, 17.0],
                [11.0, 11.0],
                [8.0, 9.0],
            ],
            ink,
        ),
        Ico::Desktop => closed(
            vec![[3.0, 5.0], [21.0, 5.0], [21.0, 16.0], [3.0, 16.0]],
            ink,
        )
        .line([12.0, 16.0], [12.0, 20.0], W, LineCap::Round, ink)
        .line([8.0, 20.0], [16.0, 20.0], W, LineCap::Round, ink),
        Ico::Downloads => stroke(vec![[12.0, 3.0], [12.0, 14.0]], ink)
            .stroke(vec![[8.0, 10.0], [12.0, 14.0], [16.0, 10.0]], W, ink)
            .stroke(
                vec![[4.0, 17.0], [4.0, 20.0], [20.0, 20.0], [20.0, 17.0]],
                W,
                ink,
            ),
        Ico::Documents => Shape::new().fill(
            vec![
                [5.0, 3.0],
                [14.0, 3.0],
                [19.0, 8.0],
                [19.0, 21.0],
                [5.0, 21.0],
            ],
            ink,
        ),
        Ico::Pictures => closed(
            vec![[3.0, 5.0], [21.0, 5.0], [21.0, 19.0], [3.0, 19.0]],
            ink,
        )
        .fill(circle(8.0, 10.0, 1.6), ink)
        .fill(vec![[4.0, 18.0], [10.0, 12.0], [16.0, 18.0]], ink),
        Ico::Music => Shape::new()
            .fill(
                vec![[10.0, 4.0], [19.0, 4.0], [19.0, 8.0], [10.0, 8.0]],
                ink,
            )
            .fill(circle(7.0, 18.0, 3.0), ink)
            .fill(circle(17.0, 16.0, 3.0), ink)
            .line([10.0, 6.0], [10.0, 18.0], W, LineCap::Round, ink)
            .line([19.0, 6.0], [19.0, 16.0], W, LineCap::Round, ink),
        Ico::Videos => Shape::new().fill(vec![[8.0, 5.0], [19.0, 12.0], [8.0, 19.0]], ink),
        Ico::ThisPc => closed(
            vec![[3.0, 5.0], [21.0, 5.0], [21.0, 16.0], [3.0, 16.0]],
            ink,
        )
        .line([12.0, 16.0], [12.0, 20.0], W, LineCap::Round, ink)
        .line([8.0, 20.0], [16.0, 20.0], W, LineCap::Round, ink),
        Ico::Monitor => Shape::new()
            .fill(
                vec![[2.0, 3.0], [22.0, 3.0], [22.0, 17.0], [2.0, 17.0]],
                ink,
            )
            .fill(
                vec![[9.0, 17.0], [15.0, 17.0], [15.0, 20.0], [9.0, 20.0]],
                ink,
            )
            .fill(
                vec![[6.0, 20.0], [18.0, 20.0], [18.0, 21.0], [6.0, 21.0]],
                ink,
            ),
        Ico::Minimize => stroke(vec![[6.0, 12.0], [18.0, 12.0]], ink),
        Ico::Maximize => closed(
            vec![[6.0, 6.0], [18.0, 6.0], [18.0, 18.0], [6.0, 18.0]],
            ink,
        ),
        Ico::Restore => closed(
            vec![[5.0, 9.0], [15.0, 9.0], [15.0, 19.0], [5.0, 19.0]],
            ink,
        )
        .stroke(
            vec![[9.0, 9.0], [9.0, 5.0], [19.0, 5.0], [19.0, 15.0], [15.0, 15.0]],
            W,
            ink,
        ),
        Ico::Close => stroke(vec![[6.0, 6.0], [18.0, 18.0]], ink).stroke(
            vec![[18.0, 6.0], [6.0, 18.0]],
            W,
            ink,
        ),
        Ico::Info => closed(circle(12.0, 12.0, 8.0), ink)
            .fill(circle(12.0, 8.0, 1.1), ink)
            .line([12.0, 11.0], [12.0, 16.0], W, LineCap::Round, ink),
        Ico::ShareSmall => closed(
            vec![[4.0, 10.0], [4.0, 20.0], [20.0, 20.0], [20.0, 10.0]],
            ink,
        )
        .line([12.0, 4.0], [12.0, 14.0], W, LineCap::Round, ink)
        .stroke(vec![[9.0, 7.0], [12.0, 4.0], [15.0, 7.0]], W, ink),
        Ico::SetBackground => closed(
            vec![[3.0, 5.0], [21.0, 5.0], [21.0, 17.0], [3.0, 17.0]],
            ink,
        )
        .line([8.0, 20.0], [16.0, 20.0], W, LineCap::Round, ink),
        Ico::RotateLeft => stroke(vec![[19.0, 9.0], [19.0, 5.0], [15.0, 5.0]], ink).stroke(
            vec![
                [5.0, 12.0],
                [5.0, 16.0],
                [8.0, 19.0],
                [12.0, 19.0],
                [17.0, 19.0],
                [19.0, 15.0],
                [19.0, 12.0],
            ],
            W,
            ink,
        ),
        Ico::RotateRight => stroke(vec![[5.0, 9.0], [5.0, 5.0], [9.0, 5.0]], ink).stroke(
            vec![
                [19.0, 12.0],
                [19.0, 16.0],
                [16.0, 19.0],
                [12.0, 19.0],
                [7.0, 19.0],
                [5.0, 15.0],
                [5.0, 12.0],
            ],
            W,
            ink,
        ),
        Ico::Star => closed(
            vec![
                [12.0, 3.5],
                [14.7, 9.0],
                [20.5, 9.6],
                [16.2, 13.6],
                [17.6, 19.5],
                [12.0, 16.4],
                [6.4, 19.5],
                [7.8, 13.6],
                [3.5, 9.6],
                [9.3, 9.0],
            ],
            ink,
        ),
        Ico::Clock => closed(circle(12.0, 12.0, 8.5), ink)
            .stroke(vec![[12.0, 7.0], [12.0, 12.0], [15.5, 14.0]], W, ink),
        Ico::Grid => Shape::new()
            .stroke_with(
                vec![[4.0, 4.0], [10.0, 4.0], [10.0, 10.0], [4.0, 10.0]],
                true,
                W,
                LineCap::Round,
                ink,
            )
            .stroke_with(
                vec![[14.0, 4.0], [20.0, 4.0], [20.0, 10.0], [14.0, 10.0]],
                true,
                W,
                LineCap::Round,
                ink,
            )
            .stroke_with(
                vec![[4.0, 14.0], [10.0, 14.0], [10.0, 20.0], [4.0, 20.0]],
                true,
                W,
                LineCap::Round,
                ink,
            )
            .stroke_with(
                vec![[14.0, 14.0], [20.0, 14.0], [20.0, 20.0], [14.0, 20.0]],
                true,
                W,
                LineCap::Round,
                ink,
            ),
        Ico::List => Shape::new()
            .fill(circle(5.0, 6.0, 1.2), ink)
            .fill(circle(5.0, 12.0, 1.2), ink)
            .fill(circle(5.0, 18.0, 1.2), ink)
            .line([9.0, 6.0], [20.0, 6.0], W, LineCap::Round, ink)
            .line([9.0, 12.0], [20.0, 12.0], W, LineCap::Round, ink)
            .line([9.0, 18.0], [20.0, 18.0], W, LineCap::Round, ink),
    };
    let _ = p;
    canvas(sh.view_box(24.0, 24.0)).size(size, size)
}

/// A Windows 11 folder: a darker back panel with a raised tab, then a gradient
/// front face. `mark` is the white place glyph pinned to the front, if any.
fn folder(size: f32, top: &str, bottom: &str, back: &str, mark: Option<Ico>, p: Paint) -> El {
    let silhouette = Shape::new()
        .fill(
            vec![
                [1.0, 7.0],
                [1.0, 4.0],
                [8.5, 4.0],
                [10.5, 6.5],
                [23.0, 6.5],
                [23.0, 20.0],
                [1.0, 20.0],
            ],
            c(back),
        )
        .view_box(24.0, 24.0);
    let mut front = el()
        .absolute(0.0, size * 0.34)
        .size(size, size * 0.58)
        .radius(size * 0.09)
        .gradient(c(top), c(bottom));
    if let Some(which) = mark {
        front = front.center().child(ico(p, which, size * 0.42, Color::WHITE));
    }
    el().size(size, size)
        .child(canvas(silhouette).absolute(0.0, 0.0).size(size, size))
        .child(front)
}

fn yellow_folder(size: f32, p: Paint) -> El {
    folder(size, "#ffd66b", "#f0b232", "#d89427", None, p)
}

/// The compressed-archive folder: a yellow folder with a zipper down its face.
fn archive_folder(size: f32, p: Paint) -> El {
    let mut teeth = Shape::new();
    for i in 0..6 {
        let y = 9.5 + i as f32 * 1.6;
        let x = if i % 2 == 0 { 10.6 } else { 12.2 };
        teeth = teeth.fill(
            vec![[x, y], [x + 1.3, y], [x + 1.3, y + 1.1], [x, y + 1.1]],
            c("#a5761f"),
        );
    }
    teeth = teeth
        .fill(
            vec![[10.9, 18.6], [13.1, 18.6], [13.1, 20.6], [10.9, 20.6]],
            c("#8a6118"),
        )
        .view_box(24.0, 24.0);
    el().size(size, size)
        .child(yellow_folder(size, p))
        .child(canvas(teeth).absolute(0.0, 0.0).size(size, size))
}

/// Installers carry each application's own mark in Explorer; these are drawn
/// stand-ins keyed off the names in the reference captures, with a stable
/// geometric fallback for anything else.
fn app_glyph(name: &str, size: f32, p: Paint) -> El {
    let n = name.to_ascii_lowercase();
    let sh = if n.contains("firefox") {
        Shape::new()
            .fill(circle(12.0, 12.0, 9.5), c("#f08c1e"))
            .fill(circle(12.5, 12.5, 5.2), c("#3b6fd4"))
            .fill(circle(10.0, 9.0, 2.6), c("#ffd24d"))
    } else if n.contains("discord") {
        Shape::new()
            .fill(
                vec![[3.0, 4.0], [21.0, 4.0], [21.0, 20.0], [3.0, 20.0]],
                c("#5865f2"),
            )
            .fill(circle(9.0, 12.0, 1.8), Color::WHITE)
            .fill(circle(15.0, 12.0, 1.8), Color::WHITE)
            .fill(
                vec![[6.0, 15.0], [18.0, 15.0], [17.0, 18.0], [7.0, 18.0]],
                Color::WHITE,
            )
    } else if n.contains("git-") {
        Shape::new()
            .fill(
                vec![[12.0, 2.0], [22.0, 12.0], [12.0, 22.0], [2.0, 12.0]],
                c("#f05033"),
            )
            .line([9.0, 8.0], [9.0, 15.0], 1.4, LineCap::Round, Color::WHITE)
            .line([9.0, 11.0], [15.0, 11.0], 1.4, LineCap::Round, Color::WHITE)
            .line([15.0, 11.0], [15.0, 14.0], 1.4, LineCap::Round, Color::WHITE)
            .fill(circle(9.0, 7.0, 1.7), Color::WHITE)
            .fill(circle(15.0, 15.5, 1.7), Color::WHITE)
    } else if n.contains("node") {
        Shape::new()
            .fill(
                vec![[12.0, 2.0], [21.0, 7.0], [21.0, 17.0], [12.0, 22.0], [3.0, 17.0], [3.0, 7.0]],
                c("#3c873a"),
            )
            .stroke_with(
                vec![[12.0, 6.0], [17.5, 9.0], [17.5, 15.0], [12.0, 18.0], [6.5, 15.0], [6.5, 9.0]],
                true,
                1.4,
                LineCap::Round,
                Color::WHITE,
            )
    } else if n.contains("warp") {
        Shape::new()
            .fill(circle(12.0, 12.0, 9.5), c("#f2f2f2"))
            .fill(
                vec![[7.0, 16.0], [13.0, 7.0], [17.0, 7.0], [11.0, 16.0]],
                c("#1b1b1b"),
            )
    } else if n.contains("brisk") {
        Shape::new()
            .fill(circle(12.0, 12.0, 9.5), c("#1fae9b"))
            .fill(
                vec![[13.5, 5.0], [8.0, 13.0], [11.5, 13.0], [10.5, 19.0], [16.0, 11.0], [12.5, 11.0]],
                Color::WHITE,
            )
    } else if n.contains("workbuddy") {
        Shape::new()
            .fill(
                vec![[3.0, 3.0], [21.0, 3.0], [21.0, 21.0], [3.0, 21.0]],
                c("#22b07d"),
            )
            .stroke(
                vec![[6.5, 8.0], [9.0, 16.0], [12.0, 10.0], [15.0, 16.0], [17.5, 8.0]],
                1.8,
                Color::WHITE,
            )
    } else if n.contains("vencord") {
        Shape::new()
            .fill(circle(12.0, 12.0, 9.5), c("#b48cf2"))
            .stroke(vec![[7.5, 8.0], [12.0, 16.0], [16.5, 8.0]], 2.0, Color::WHITE)
    } else if n.contains("windowsdesktop") {
        Shape::new()
            .fill(vec![[3.0, 3.0], [11.0, 3.0], [11.0, 11.0], [3.0, 11.0]], c("#0078d4"))
            .fill(vec![[13.0, 3.0], [21.0, 3.0], [21.0, 11.0], [13.0, 11.0]], c("#0078d4"))
            .fill(vec![[3.0, 13.0], [11.0, 13.0], [11.0, 21.0], [3.0, 21.0]], c("#0078d4"))
            .fill(vec![[13.0, 13.0], [21.0, 13.0], [21.0, 21.0], [13.0, 21.0]], c("#0078d4"))
    } else if n.contains("bootstrap") || n.contains("bloxstrap") {
        Shape::new()
            .fill(
                vec![[3.0, 3.0], [21.0, 3.0], [21.0, 21.0], [3.0, 21.0]],
                c("#7952b3"),
            )
            .fill(vec![[8.0, 7.0], [13.0, 7.0], [13.0, 11.0], [8.0, 11.0]], Color::WHITE)
            .fill(vec![[8.0, 13.0], [15.0, 13.0], [15.0, 17.0], [8.0, 17.0]], Color::WHITE)
    } else if n.contains("heroic") {
        Shape::new()
            .fill(circle(12.0, 12.0, 9.5), c("#8b5cf6"))
            .fill(vec![[12.0, 5.0], [16.0, 15.0], [12.0, 13.0], [8.0, 15.0]], Color::WHITE)
            .fill(vec![[10.5, 16.0], [13.5, 16.0], [12.0, 19.5]], c("#ffd24d"))
    } else if n.contains("anker") {
        Shape::new()
            .fill(circle(12.0, 12.0, 9.5), c("#2fa8e0"))
            .stroke_with(circle(12.0, 8.0, 2.2), true, 1.5, LineCap::Round, Color::WHITE)
            .line([12.0, 10.0], [12.0, 17.0], 1.5, LineCap::Round, Color::WHITE)
            .stroke(vec![[7.5, 13.0], [7.5, 17.0], [16.5, 17.0], [16.5, 13.0]], 1.5, Color::WHITE)
    } else if n.contains("fdm") {
        Shape::new()
            .fill(circle(12.0, 12.0, 9.5), c("#3b82c4"))
            .line([12.0, 6.0], [12.0, 14.0], 1.8, LineCap::Round, Color::WHITE)
            .stroke(vec![[8.5, 11.0], [12.0, 14.5], [15.5, 11.0]], 1.8, Color::WHITE)
            .line([7.5, 17.5], [16.5, 17.5], 1.8, LineCap::Round, Color::WHITE)
    } else {
        let base = c(app_color(name));
        let h = name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        let body = Shape::new().fill(
            vec![[3.0, 3.0], [21.0, 3.0], [21.0, 21.0], [3.0, 21.0]],
            base,
        );
        match h % 3 {
            0 => body.fill(circle(12.0, 12.0, 4.0), Color::WHITE),
            1 => body.fill(vec![[12.0, 6.0], [18.0, 17.0], [6.0, 17.0]], Color::WHITE),
            _ => body
                .fill(vec![[7.0, 7.0], [17.0, 7.0], [17.0, 10.0], [7.0, 10.0]], Color::WHITE)
                .fill(vec![[7.0, 13.0], [14.0, 13.0], [14.0, 16.0], [7.0, 16.0]], Color::WHITE),
        }
    };
    let _ = p;
    canvas(sh.view_box(24.0, 24.0)).size(size, size)
}

/// A generic document page with a folded corner and a kind-coloured mark.
fn page_glyph(kind: Kind, size: f32) -> El {
    let paper = c("#f4f4f4");
    let fold = c("#c9c9c9");
    let mut sh = Shape::new()
        .fill(
            vec![[5.0, 1.0], [14.0, 1.0], [19.0, 6.0], [19.0, 23.0], [5.0, 23.0]],
            paper,
        )
        .fill(vec![[14.0, 1.0], [14.0, 6.0], [19.0, 6.0]], fold);
    sh = match kind {
        Kind::Pdf => sh
            .fill(vec![[5.0, 14.0], [19.0, 14.0], [19.0, 20.0], [5.0, 20.0]], c("#d94848"))
            .line([7.0, 9.0], [15.0, 9.0], 1.2, LineCap::Round, c("#9a9a9a"))
            .line([7.0, 11.5], [17.0, 11.5], 1.2, LineCap::Round, c("#9a9a9a")),
        Kind::Code => sh
            .stroke(vec![[10.0, 10.0], [7.5, 13.0], [10.0, 16.0]], 1.4, c("#2f6fd0"))
            .stroke(vec![[14.0, 10.0], [16.5, 13.0], [14.0, 16.0]], 1.4, c("#2f6fd0")),
        Kind::Sheet => sh
            .fill(vec![[7.0, 9.0], [17.0, 9.0], [17.0, 18.0], [7.0, 18.0]], c("#1f8f4e"))
            .line([10.5, 9.0], [10.5, 18.0], 1.0, LineCap::Butt, paper)
            .line([14.0, 9.0], [14.0, 18.0], 1.0, LineCap::Butt, paper)
            .line([7.0, 12.5], [17.0, 12.5], 1.0, LineCap::Butt, paper)
            .line([7.0, 15.5], [17.0, 15.5], 1.0, LineCap::Butt, paper),
        Kind::Slides => sh
            .fill(circle(12.0, 13.0, 4.5), c("#e0642a"))
            .fill(vec![[12.0, 13.0], [12.0, 8.5], [16.5, 13.0]], paper),
        Kind::Image => sh
            .fill(circle(9.5, 9.5, 1.6), c("#f0b232"))
            .fill(vec![[6.0, 18.0], [10.5, 12.0], [14.0, 16.0], [16.0, 13.5], [18.0, 18.0]], c("#2fa8e0")),
        Kind::Video => sh
            .fill(vec![[7.0, 9.0], [17.0, 9.0], [17.0, 18.0], [7.0, 18.0]], c("#3a3f4a"))
            .fill(vec![[10.5, 11.0], [14.5, 13.5], [10.5, 16.0]], c("#8b5cf6")),
        Kind::Audio => sh
            .fill(circle(9.0, 16.0, 2.4), c("#e0518c"))
            .line([11.4, 16.0], [11.4, 8.0], 1.4, LineCap::Round, c("#e0518c"))
            .line([11.4, 8.0], [16.0, 9.5], 1.4, LineCap::Round, c("#e0518c")),
        Kind::Document => sh
            .line([7.0, 9.0], [17.0, 9.0], 1.2, LineCap::Round, c("#2f6fd0"))
            .line([7.0, 12.0], [17.0, 12.0], 1.2, LineCap::Round, c("#9a9a9a"))
            .line([7.0, 15.0], [17.0, 15.0], 1.2, LineCap::Round, c("#9a9a9a"))
            .line([7.0, 18.0], [13.0, 18.0], 1.2, LineCap::Round, c("#9a9a9a")),
        _ => sh
            .line([7.0, 9.0], [17.0, 9.0], 1.2, LineCap::Round, c("#9a9a9a"))
            .line([7.0, 12.0], [17.0, 12.0], 1.2, LineCap::Round, c("#9a9a9a"))
            .line([7.0, 15.0], [14.0, 15.0], 1.2, LineCap::Round, c("#9a9a9a")),
    };
    canvas(sh.view_box(24.0, 24.0)).size(size, size)
}

/// The 7-Zip shell mark the captures show for ZIP files: a white page, black 7z.
fn seven_zip(size: f32) -> El {
    let sh = Shape::new()
        .fill(
            vec![[4.0, 1.0], [15.0, 1.0], [20.0, 6.0], [20.0, 23.0], [4.0, 23.0]],
            c("#f4f4f4"),
        )
        .fill(vec![[15.0, 1.0], [15.0, 6.0], [20.0, 6.0]], c("#c9c9c9"))
        .view_box(24.0, 24.0);
    el().size(size, size)
        .child(canvas(sh).absolute(0.0, 0.0).size(size, size))
        .child(
            text("7z")
                .font(TextStyle::new(size * 0.42, 700.0))
                .color(c("#1b1b1b"))
                .absolute(size * 0.22, size * 0.34),
        )
}

/// Synthetic stand-ins for the five screenshot thumbnails in the reference
/// capture, drawn as tiny scenes so the grid and preview read as real previews.
fn screenshot_thumb(index: usize, w: f32, h: f32) -> El {
    let mut sh = Shape::new();
    match index % 5 {
        0 => {
            sh = sh
                .fill(vec![[0.0, 0.0], [90.0, 0.0], [90.0, 72.0], [0.0, 72.0]], c("#c9ccd1"))
                .fill(vec![[10.0, 12.0], [80.0, 12.0], [80.0, 62.0], [10.0, 62.0]], c("#f4f5f7"))
                .fill(vec![[10.0, 12.0], [80.0, 12.0], [80.0, 20.0], [10.0, 20.0]], c("#e2e5ea"))
                .fill(circle(15.0, 16.0, 1.6), c("#d94848"))
                .fill(vec![[16.0, 28.0], [60.0, 28.0], [60.0, 31.0], [16.0, 31.0]], c("#b9bec7"))
                .fill(vec![[16.0, 36.0], [72.0, 36.0], [72.0, 39.0], [16.0, 39.0]], c("#cdd2d9"))
                .fill(vec![[16.0, 44.0], [50.0, 44.0], [50.0, 47.0], [16.0, 47.0]], c("#cdd2d9"));
        }
        1 => {
            sh = sh
                .fill(vec![[0.0, 0.0], [90.0, 0.0], [90.0, 72.0], [0.0, 72.0]], c("#0d1015"))
                .fill(vec![[62.0, 0.0], [90.0, 0.0], [90.0, 72.0], [62.0, 72.0]], c("#151b26"))
                .fill(circle(76.0, 20.0, 9.0), c("#1e2735"))
                .fill(vec![[8.0, 8.0], [56.0, 8.0], [56.0, 12.0], [8.0, 12.0]], c("#e8ecf2"))
                .fill(vec![[8.0, 16.0], [40.0, 16.0], [40.0, 18.5], [8.0, 18.5]], c("#5d6672"));
            for i in 0..4 {
                let y = 24.0 + i as f32 * 8.0;
                sh = sh
                    .fill(circle(10.0, y + 1.5, 1.8), c("#3fbf6f"))
                    .fill(vec![[15.0, y], [48.0, y], [48.0, y + 2.5], [15.0, y + 2.5]], c("#8b93a0"))
                    .fill(vec![[15.0, y + 3.5], [36.0, y + 3.5], [36.0, y + 5.0], [15.0, y + 5.0]], c("#4a525e"));
            }
            sh = sh.fill(vec![[24.0, 60.0], [52.0, 60.0], [52.0, 67.0], [24.0, 67.0]], c("#2f6fd0"));
        }
        2 => {
            sh = sh
                .fill(vec![[0.0, 0.0], [90.0, 0.0], [90.0, 72.0], [0.0, 72.0]], c("#101318"))
                .fill(vec![[10.0, 26.0], [80.0, 26.0], [80.0, 46.0], [10.0, 46.0]], c("#1c222c"))
                .fill(vec![[16.0, 32.0], [56.0, 32.0], [56.0, 35.0], [16.0, 35.0]], c("#d5dae2"))
                .fill(vec![[16.0, 39.0], [44.0, 39.0], [44.0, 41.5], [16.0, 41.5]], c("#5d6672"));
        }
        3 => {
            sh = sh
                .fill(vec![[0.0, 0.0], [90.0, 0.0], [90.0, 72.0], [0.0, 72.0]], c("#0e1218"))
                .fill(vec![[0.0, 0.0], [18.0, 0.0], [18.0, 72.0], [0.0, 72.0]], c("#161c26"))
                .fill(circle(26.0, 10.0, 2.2), c("#2fa8e0"))
                .fill(circle(34.0, 10.0, 2.2), c("#8b5cf6"))
                .fill(circle(42.0, 10.0, 2.2), c("#3fbf6f"));
            for i in 0..6 {
                let y = 20.0 + i as f32 * 7.0;
                let w = 30.0 + ((i * 17) % 26) as f32;
                sh = sh.fill(vec![[24.0, y], [24.0 + w, y], [24.0 + w, y + 2.4], [24.0, y + 2.4]], c("#6b7482"));
            }
        }
        _ => {
            sh = sh.fill(vec![[0.0, 0.0], [90.0, 0.0], [90.0, 72.0], [0.0, 72.0]], c("#0c0f14"));
            for i in 0..8 {
                let y = 8.0 + i as f32 * 7.0;
                let w = 24.0 + ((i * 23) % 40) as f32;
                let ink = if i % 3 == 0 { c("#3fbf6f") } else { c("#7d8590") };
                sh = sh.fill(vec![[8.0, y], [8.0 + w, y], [8.0 + w, y + 2.4], [8.0, y + 2.4]], ink);
            }
        }
    }
    canvas(sh.view_box(90.0, 72.0)).size(w, h)
}

fn file_glyph(e: &Entry, p: Paint, size: f32) -> El {
    if e.is_dir {
        return yellow_folder(size, p);
    }
    match e.kind {
        Kind::App => app_glyph(&e.name, size, p),
        Kind::Archive => {
            if e.type_label == "ZIP File" {
                seven_zip(size)
            } else {
                archive_folder(size, p)
            }
        }
        Kind::Image if e.fixture => {
            let idx = e.name.rsplit(' ').next().and_then(|t| t.parse::<usize>().ok());
            match idx {
                Some(_) => {
                    let at = ["095523", "103957", "104522", "111606", "111619"]
                        .iter()
                        .position(|t| e.name.ends_with(t))
                        .unwrap_or(0);
                    screenshot_thumb(at, size, size * 0.8)
                }
                None => page_glyph(Kind::Image, size),
            }
        }
        kind => page_glyph(kind, size),
    }
}

/// The Windows 11 local-disk mark: the four-square logo over a grey drive.
fn drive_glyph(p: Paint, size: f32) -> El {
    let sh = Shape::new()
        .fill(vec![[2.0, 1.0], [9.0, 1.0], [9.0, 8.0], [2.0, 8.0]], c("#0078d4"))
        .fill(vec![[11.0, 1.0], [18.0, 1.0], [18.0, 8.0], [11.0, 8.0]], c("#0078d4"))
        .fill(vec![[2.0, 10.0], [9.0, 10.0], [9.0, 17.0], [2.0, 17.0]], c("#0078d4"))
        .fill(vec![[11.0, 10.0], [18.0, 10.0], [18.0, 17.0], [11.0, 17.0]], c("#0078d4"))
        .fill(vec![[3.0, 12.0], [21.0, 12.0], [23.0, 15.0], [1.0, 15.0]], c("#e4e8ed"))
        .fill(vec![[1.0, 15.0], [23.0, 15.0], [23.0, 21.0], [1.0, 21.0]], c("#b9c0c9"))
        .fill(vec![[1.0, 19.0], [23.0, 19.0], [23.0, 21.0], [1.0, 21.0]], c("#9aa1ab"))
        .fill(vec![[12.0, 16.5], [21.0, 16.5], [21.0, 17.8], [12.0, 17.8]], c("#6e7681"))
        .fill(circle(4.5, 17.2, 1.0), c("#3fbf6f"))
        .view_box(24.0, 24.0);
    let _ = p;
    canvas(sh).size(size, size)
}

/// Colourful place marks for the navigation rail and tab, matching the
/// captures: each place owns a hue, drawn as a small filled glyph.
fn place_glyph(index: usize, size: f32) -> El {
    let sh = match index {
        0 => Shape::new()
            .fill(vec![[2.0, 12.0], [12.0, 3.0], [22.0, 12.0], [19.0, 12.0], [19.0, 21.0], [5.0, 21.0], [5.0, 12.0]], c("#f5a623"))
            .fill(vec![[10.0, 14.0], [14.0, 14.0], [14.0, 21.0], [10.0, 21.0]], c("#c77f0a")),
        1 | 6 => Shape::new()
            .fill(vec![[2.0, 4.0], [22.0, 4.0], [22.0, 20.0], [2.0, 20.0]], c("#2fa8e0"))
            .fill(circle(8.0, 9.0, 1.8), Color::WHITE)
            .fill(vec![[4.0, 18.0], [10.0, 11.0], [14.0, 15.0], [17.0, 12.0], [21.0, 18.0]], Color::WHITE),
        2 => Shape::new()
            .fill(circle(8.0, 14.0, 5.0), c("#29a3ec"))
            .fill(circle(14.0, 11.0, 6.0), c("#29a3ec"))
            .fill(circle(18.0, 15.0, 4.0), c("#29a3ec"))
            .fill(vec![[4.0, 15.0], [21.0, 15.0], [21.0, 19.0], [4.0, 19.0]], c("#1b86d8")),
        3 | 9 => Shape::new()
            .fill(vec![[2.0, 3.0], [22.0, 3.0], [22.0, 16.0], [2.0, 16.0]], c("#3e9bdd"))
            .fill(vec![[3.0, 4.0], [21.0, 4.0], [21.0, 12.0], [3.0, 12.0]], c("#6cb8e8"))
            .fill(vec![[10.0, 16.0], [14.0, 16.0], [14.0, 19.0], [10.0, 19.0]], c("#8a939e"))
            .fill(vec![[7.0, 19.0], [17.0, 19.0], [17.0, 21.0], [7.0, 21.0]], c("#8a939e")),
        4 => Shape::new()
            .line([12.0, 3.0], [12.0, 13.0], 2.2, LineCap::Round, c("#1fae7c"))
            .stroke(vec![[7.0, 9.0], [12.0, 14.0], [17.0, 9.0]], 2.2, c("#1fae7c"))
            .stroke(vec![[4.0, 16.0], [4.0, 20.0], [20.0, 20.0], [20.0, 16.0]], 2.0, c("#1fae7c")),
        5 => Shape::new()
            .fill(vec![[5.0, 2.0], [14.0, 2.0], [19.0, 7.0], [19.0, 22.0], [5.0, 22.0]], c("#8fa6bc"))
            .fill(vec![[14.0, 2.0], [14.0, 7.0], [19.0, 7.0]], c("#b9c9d9"))
            .line([8.0, 11.0], [16.0, 11.0], 1.4, LineCap::Round, Color::WHITE)
            .line([8.0, 14.5], [16.0, 14.5], 1.4, LineCap::Round, Color::WHITE)
            .line([8.0, 18.0], [13.0, 18.0], 1.4, LineCap::Round, Color::WHITE),
        7 => Shape::new()
            .fill(circle(12.0, 12.0, 10.0), c("#e8734a"))
            .fill(circle(9.0, 16.0, 2.4), Color::WHITE)
            .line([11.4, 16.0], [11.4, 7.0], 1.6, LineCap::Round, Color::WHITE)
            .line([11.4, 7.0], [17.0, 8.8], 1.6, LineCap::Round, Color::WHITE),
        8 => Shape::new()
            .fill(vec![[2.0, 4.0], [22.0, 4.0], [22.0, 20.0], [2.0, 20.0]], c("#8b5cf6"))
            .fill(vec![[4.0, 6.0], [7.0, 6.0], [7.0, 8.0], [4.0, 8.0]], c("#c4a8f8"))
            .fill(vec![[4.0, 16.0], [7.0, 16.0], [7.0, 18.0], [4.0, 18.0]], c("#c4a8f8"))
            .fill(vec![[10.0, 8.0], [18.0, 12.0], [10.0, 16.0]], Color::WHITE),
        _ => Shape::new()
            .fill(circle(12.0, 10.0, 8.0), c("#3b9bdd"))
            .stroke_with(circle(12.0, 10.0, 8.0), true, 1.2, LineCap::Round, c("#7cc4ee"))
            .line([4.0, 10.0], [20.0, 10.0], 1.2, LineCap::Round, c("#7cc4ee"))
            .stroke(vec![[12.0, 2.0], [8.5, 10.0], [12.0, 18.0], [15.5, 10.0], [12.0, 2.0]], 1.2, c("#7cc4ee"))
            .fill(vec![[10.0, 18.0], [14.0, 18.0], [14.0, 21.0], [10.0, 21.0]], c("#8a939e"))
            .fill(vec![[7.0, 21.0], [17.0, 21.0], [17.0, 22.5], [7.0, 22.5]], c("#8a939e")),
    };
    canvas(sh.view_box(24.0, 24.0)).size(size, size)
}

// ----------------------------------------------------------------- view ----

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let p = paint(s.dark, cx.env().scale);
    // The thumb is a window-level overlay driven by the Files viewport; when
    // that viewport is not mounted nothing refreshes it, so hide it here.
    if s.view != View::Files {
        s.scroll_vis.set(0.0);
    }
    col()
        .key("window")
        .w_full()
        .h_full()
        .fill(p.title)
        .child(tab_strip(s, p))
        .child(address_row(s, p))
        .child(command_bar(s, p))
        .child(body(s, p))
        .child(status_bar(s, p))
        .child(scroll_thumb(s, p))
}

/// The drawn scrollbar thumb. Its rect and visibility arrive from the scroll
/// viewport's `on_scroll_offset` through Live bindings, so scrolling never
/// re-runs the view; dragging it scrolls the viewport back.
fn scroll_thumb(s: &App, p: Paint) -> El {
    el().key("scroll-thumb")
        .pin(Some(0.0), Some(0.0), None, None)
        .size(6.0, 28.0)
        .live_rect(&s.scroll_thumb)
        .live_opacity(&s.scroll_vis)
        .radius(3.0)
        .fill(p.scroll_thumb)
        .hover(Color::WHITE.alpha(0.25))
        .cursor(rune::Cursor::Default)
        .on_drag(|s: &mut App, d: &mut rune::Drag, tx: &mut Tx| {
            let max = s.scroll_max.get();
            if max <= 0.0 {
                return;
            }
            match d.phase {
                Phase::Down => d.data = s.scroll_off.get(),
                Phase::Move => {
                    let track = s.scroll_track.get();
                    let thumb = s.scroll_thumb.get();
                    let range = (track[3] - thumb[3]).max(1.0);
                    let off = (d.data + d.dy / range * max).clamp(0.0, max);
                    tx.scroll_to_instant("viewport", 0.0, off);
                }
                Phase::Up => {}
            }
        })
}

fn tab_strip(s: &App, p: Paint) -> El {
    let tab = row()
        .key("tab")
        .h(TAB_ITEM_H)
        .w(230.0)
        .corners([8.0, 8.0, 0.0, 0.0])
        .fill(p.tab)
        .pl(12.0)
        .pr(6.0)
        .gap(8.0)
        .items_center()
        .child(place_glyph(s.nav, 16.0))
        .child(
            text(s.tab.as_str())
                .font(CAPTION)
                .color(p.text)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(
            el().key("tab-close")
                .size(22.0, 22.0)
                .radius(4.0)
                .hover(p.hover)
                .center()
                .child(ico(p, Ico::Close, 11.0, p.secondary))
                .help("Close tab")
                .on_press(|s: &mut App| {
                    s.notice = "The last tab cannot be closed".into();
                }),
        );
    let mut strip = row()
        .key("tabs")
        .w_full()
        .h(TAB_H)
        .fill(p.title)
        .items_end()
        .pl(8.0)
        .gap(2.0)
        // A borderless window has no system titlebar. This band is it: drag to
        // move, double press to zoom, and the drawn controls below to close.
        .window_drag()
        .child(tab)
        .child(
            el().key("new-tab")
                .size(30.0, 30.0)
                .radius(4.0)
                .hover(p.hover)
                .center()
                .child(text("+").font(TextStyle::new(16.0, 400.0)).color(p.secondary))
                .help("New tab")
                .on_press(|s: &mut App| {
                    s.notice = "New tabs are not implemented in this slice".into()
                }),
        )
        .child(el().grow(1.0));
    let controls = [
        (Ico::Minimize, "win-min", "Minimize"),
        (Ico::Maximize, "win-max", "Maximize"),
        (Ico::Close, "win-close", "Close"),
    ];
    for (i, (which, key, tip)) in controls.iter().enumerate() {
        let close = i == 2;
        let zoomed = i == 1 && s.zoom;
        let glyph = if zoomed { Ico::Restore } else { *which };
        let label = if zoomed { "Restore" } else { *tip };
        strip = strip.child(
            el().key(*key)
                .size(46.0, TAB_H)
                .hover(if close { p.close_hover } else { p.hover })
                .center()
                .child(ico(p, glyph, 13.0, p.secondary))
                .help(label)
                .on_press_tx(move |s: &mut App, tx: &mut Tx| match i {
                    0 => winframe::minimize(),
                    1 => s.zoom = winframe::toggle_maximize(),
                    _ => tx.close_window(),
                }),
        );
    }
    strip
}

fn nav_button(p: Paint, which: Ico, key: &str, action: usize, enabled: bool, tip: &str) -> El {
    let ink = if enabled { p.secondary } else { p.disabled };
    el().key(key)
        .size(30.0, 30.0)
        .radius(4.0)
        .hover(p.hover)
        .press(p.press)
        .center()
        .child(ico(p, which, 15.0, ink))
        .help(tip)
        .disabled(!enabled)
        .on_press(move |s: &mut App| match action {
            0 => go_back(s),
            1 => go_forward(s),
            2 => go_up(s),
            _ => {
                let path = s.root.clone();
                set_location(s, path);
            }
        })
}

fn address_row(s: &App, p: Paint) -> El {
    let mut crumb = row()
        .key("breadcrumb")
        .grow(1.0)
        .min_w(120.0)
        .h(FIELD_H)
        .radius(4.0)
        .fill(p.field)
        .border(p.hair, p.field_line)
        .px(10.0)
        .gap(6.0)
        .items_center()
        .child(if s.view == View::Home {
            place_glyph(0, 15.0)
        } else {
            ico(p, Ico::ThisPc, 15.0, p.secondary)
        });
    for (i, part) in s.crumbs.iter().enumerate() {
        crumb = crumb
            .child(ico(p, Ico::ChevronRight, 10.0, p.tertiary))
            .child(text(part.as_str()).font(UI_TEXT).color(p.text).truncate());
        if i + 1 == s.crumbs.len() {
            crumb = crumb.child(ico(p, Ico::ChevronRight, 10.0, p.tertiary));
        }
    }
    row()
        .key("address")
        .w_full()
        .h(ADDRESS_H)
        .fill(p.toolbar)
        .px(10.0)
        .gap(4.0)
        .items_center()
        .child(nav_button(p, Ico::Back, "back", 0, !s.history.is_empty(), "Back"))
        .child(nav_button(p, Ico::Forward, "forward", 1, !s.forward.is_empty(), "Forward"))
        .child(nav_button(p, Ico::Up, "up", 2, s.root.parent().is_some(), "Up"))
        .child(nav_button(p, Ico::Refresh, "refresh", 3, true, "Refresh"))
        .child(el().w(6.0))
        .child(crumb)
        .child(el().w(6.0))
        .child(
            row()
                .key("search")
                .w(260.0)
                .h(FIELD_H)
                .radius(4.0)
                .fill(p.field)
                .border(p.hair, p.field_line)
                .px(10.0)
                .gap(6.0)
                .items_center()
                .child(
                    text_input(&s.search_edit, UI_TEXT)
                        .key("search-field")
                        .grow(1.0)
                        .min_w(0.0)
                        .single_line()
                        .color(p.text)
                        .placeholder(s.search.as_str(), p.tertiary)
                        .on_edit(|s: &mut App| &mut s.search_edit),
                )
                .child(ico(p, Ico::Search, 14.0, p.secondary)),
        )
}

fn icon_button(p: Paint, which: Ico, dim: bool, tip: &str) -> El {
    let ink = if dim { p.disabled } else { p.secondary };
    el().size(32.0, 32.0)
        .radius(4.0)
        .hover(p.hover)
        .press(p.press)
        .center()
        .child(ico(p, which, 17.0, ink))
        .help(tip)
}

fn text_button(p: Paint, which: Ico, label: &str, chevron: bool, dim: bool) -> El {
    let ink = if dim { p.disabled } else { p.secondary };
    let mut b = row()
        .h(32.0)
        .px(8.0)
        .gap(6.0)
        .radius(4.0)
        .hover(p.hover)
        .press(p.press)
        .items_center()
        .child(ico(p, which, 16.0, ink))
        .child(
            text(label)
                .font(UI_TEXT)
                .color(if dim { p.disabled } else { p.text }),
        );
    if chevron {
        b = b.child(ico(p, Ico::ChevronDown, 10.0, ink));
    }
    b
}

fn accent_button(p: Paint, which: Ico, label: &str) -> El {
    row()
        .h(32.0)
        .px(8.0)
        .gap(6.0)
        .radius(4.0)
        .hover(p.hover)
        .press(p.press)
        .items_center()
        .child(ico(p, which, 16.0, p.accent))
        .child(text(label).font(UI_TEXT).color(p.text))
}

fn command_bar(s: &App, p: Paint) -> El {
    let has_sel = s.selected.is_some();
    let mut bar = row()
        .key("command")
        .w_full()
        .h(COMMAND_H)
        .fill(p.command)
        .px(10.0)
        .gap(2.0)
        .items_center()
        .child(
            text_button(p, Ico::New, "New", true, false)
                .help("New item")
                .on_press(|s: &mut App| {
                    s.notice = "New is not implemented in this slice".into();
                }),
        )
        .child(el().w(p.hair).h(20.0).fill(p.line).mx(6.0));
    for (which, label) in [
        (Ico::Cut, "Cut"),
        (Ico::Copy, "Copy"),
        (Ico::Paste, "Paste"),
        (Ico::Rename, "Rename"),
        (Ico::Share, "Share"),
        (Ico::Delete, "Delete"),
    ] {
        let dim = !has_sel && !matches!(which, Ico::Paste);
        bar = bar.child(icon_button(p, which, dim, label).on_press(move |s: &mut App| {
            s.notice = format!("{label} is not implemented in this slice");
        }));
    }
    bar = bar
        .child(el().w(p.hair).h(20.0).fill(p.line).mx(6.0))
        .child(
            text_button(p, Ico::Sort, "Sort", true, s.view == View::Home)
                .help("Sort")
                .on_press(|s: &mut App| {
                    s.sort = (s.sort + 1) % 4;
                    s.groups.clear();
                    s.keep_order = false;
                    refresh_visible(s);
                    s.notice = format!(
                        "Sorted by {}",
                        ["name", "date", "type", "size"][s.sort as usize]
                    );
                }),
        )
        .child(
            text_button(p, Ico::View, "View", true, false)
                .help("View")
                .on_press(|s: &mut App| {
                    s.mode = if s.mode == Mode::Details {
                        Mode::Grid
                    } else {
                        Mode::Details
                    };
                }),
        );
    if s.view == View::Home {
        bar = bar.child(
            text_button(p, Ico::Filter, "Filter", true, false)
                .help("Filter")
                .on_press(|s: &mut App| {
                    s.notice = "Filter is not implemented in this slice".into();
                }),
        );
    }
    if s.contextual {
        bar = bar
            .child(el().w(p.hair).h(20.0).fill(p.line).mx(6.0))
            .child(
                accent_button(p, Ico::SetBackground, "Set as background").on_press(
                    |s: &mut App| {
                        s.notice = "Set as background is not implemented in this slice".into();
                    },
                ),
            )
            .child(
                accent_button(p, Ico::RotateLeft, "Rotate left").on_press(|s: &mut App| {
                    s.notice = "Rotate left is not implemented in this slice".into();
                }),
            )
            .child(
                accent_button(p, Ico::RotateRight, "Rotate right").on_press(|s: &mut App| {
                    s.notice = "Rotate right is not implemented in this slice".into();
                }),
            );
    }
    bar = bar
        .child(el().w(p.hair).h(20.0).fill(p.line).mx(6.0))
        .child(
            icon_button(p, Ico::More, false, "More")
                .on_press(|s: &mut App| {
                    s.notice = "More commands are not implemented in this slice".into();
                }),
        )
        .child(el().grow(1.0));
    let details = text_button(p, Ico::Details, "Details", false, false)
        .help("Toggle the details pane")
        .on_press(|s: &mut App| {
            s.preview = !s.preview;
        });
    bar.child(if s.preview {
        details.fill(p.rail_sel)
    } else {
        details
    })
}

fn rail_item(
    s: &App,
    p: Paint,
    index: usize,
    label: &str,
    chevron: bool,
    pin: bool,
) -> El {
    let active = s.nav == index;
    let ink = if active { p.text } else { p.secondary };
    let mut item = row()
        .key(format!("rail-{index}"))
        .w_full()
        .h(28.0)
        .radius(4.0)
        .fill(if active {
            p.rail_sel
        } else {
            Color::TRANSPARENT
        })
        .hover(p.hover)
        .px(8.0)
        .gap(8.0)
        .items_center()
        // Explorer's rail selection is a state, not a transition. Rune gives
        // every element a default opacity entrance and springs colour changes;
        // both read as a jump here, so this subtree lands immediately.
        .motion(Spring::NONE)
        .instant_changes();
    if active {
        item = item.border(p.hair, p.sel_line);
    }
    if chevron {
        item = item.child(ico(p, Ico::ChevronRight, 9.0, p.tertiary));
    } else {
        item = item.child(el().w(9.0));
    }
    item = item
        .child(place_glyph(index, 16.0))
        .child(text(label).font(ROW_TEXT).color(ink).grow(1.0).min_w(0.0).truncate());
    if pin {
        item = item.child(ico(p, Ico::Pin, 10.0, p.tertiary));
    }
    item.on_press_tx(move |s: &mut App, tx: &mut Tx| {
        navigate(s, index);
        tx.instant();
    })
}

fn nav_rail(s: &App, p: Paint) -> El {
    col()
        .key("rail")
        .w(RAIL_W)
        .h_full()
        .fill(p.content)
        .px(4.0)
        .pt(6.0)
        .gap(1.0)
        .motion(Spring::NONE)
        .instant_changes()
        .child(rail_item(s, p, 0, "Home", false, false))
        .child(rail_item(s, p, 1, "Gallery", false, false))
        .child(el().w_full().h(p.hair).fill(p.soft_line).my(5.0).key("rail-sep-1"))
        .child(rail_item(s, p, 3, "Desktop", false, true))
        .child(rail_item(s, p, 4, "Downloads", false, true))
        .child(rail_item(s, p, 5, "Documents", false, true))
        .child(rail_item(s, p, 6, "Pictures", false, true))
        .child(rail_item(s, p, 7, "Music", false, true))
        .child(rail_item(s, p, 8, "Videos", false, true))
        .child(el().w_full().h(p.hair).fill(p.soft_line).my(5.0).key("rail-sep-2"))
        .child(rail_item(s, p, 9, "This PC", true, false))
        .child(rail_item(s, p, 10, "Network", true, false))
        .child(el().grow(1.0))
}

fn column_header(p: Paint, cols: &[(&str, f32)], right_last: bool, separators: bool) -> El {
    let mut h = row().key("header").w_full().h(HEADER_H).items_center();
    for (i, (label, w)) in cols.iter().enumerate() {
        if i > 0 && separators {
            h = h.child(el().w(p.hair).h(16.0).fill(p.line));
        }
        let t = text(*label).font(CAPTION).color(p.secondary);
        h = h.child(
            col()
                .w(*w)
                .pl(12.0)
                .pr(8.0)
                .child(if right_last && i + 1 == cols.len() {
                    t.w_full().align(Align::End)
                } else {
                    t
                }),
        );
    }
    h.child(el().grow(1.0))
}

fn group_header(p: Paint, label: &str) -> El {
    row()
        .key(format!("group-{label}"))
        .w_full()
        .h(GROUP_H)
        .px(12.0)
        .gap(6.0)
        .items_center()
        .child(ico(p, Ico::ChevronDown, 10.0, p.secondary))
        .child(text(label).font(GROUP_TEXT).color(p.text))
}

/// Record a selection and the status-bar detail Explorer shows beside the count.
fn select_entry(s: &mut App, ei: usize) {
    s.selected = Some(ei);
    s.focused = Some(ei);
    s.notice.clear();
    let size = s.entries[ei].size_label();
    s.selection_detail = if size.is_empty() {
        "1 item selected".into()
    } else {
        format!("1 item selected  {size}")
    };
}

fn file_row(s: &App, i: usize, p: Paint) -> El {
    let ei = s.visible.get(i).copied().unwrap_or(i);
    let e = &s.entries[ei];
    let selected = s.selected == Some(ei);
    let focused = s.focused == Some(ei);
    let path = e.path.clone();
    let mut line = row()
        .key(format!("row-{i}"))
        .w_full()
        .h(ROW_H)
        .items_center()
        .fill(if selected { p.row_sel } else { Color::TRANSPARENT })
        .hover(p.hover)
        .motion(Spring::NONE);
    if focused && selected {
        line = line.border(p.hair, p.focus_line);
    }
    let line = line
        .child(
            row()
                .w(NAME_W)
                .pl(12.0)
                .pr(8.0)
                .gap(8.0)
                .items_center()
                .child(file_glyph(e, p, 16.0))
                .child(
                    text(e.name.as_str())
                        .font(ROW_TEXT)
                        .color(p.text)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                ),
        )
        .child(el().w(p.hair).h(16.0).fill(p.soft_line))
        .child(
            col()
                .w(DATE_W)
                .pl(12.0)
                .child(text(e.modified.as_str()).font(ROW_TEXT).color(p.secondary)),
        )
        .child(el().w(p.hair).h(16.0).fill(p.soft_line))
        .child(
            col().w(TYPE_W).pl(12.0).pr(8.0).child(
                text(e.type_label.as_str())
                    .font(ROW_TEXT)
                    .color(p.secondary)
                    .w_full()
                    .truncate(),
            ),
        )
        .child(el().w(p.hair).h(16.0).fill(p.soft_line))
        .child(
            col().w(SIZE_W).pl(8.0).pr(12.0).child(
                text(e.size_label())
                    .font(ROW_TEXT)
                    .color(p.secondary)
                    .w_full()
                    .align(Align::End),
            ),
        )
        .child(el().grow(1.0));
    let double_path = path.clone();
    line.on_press(move |s: &mut App| {
        if let Some(at) = s.entries.iter().position(|entry| entry.path == path) {
            select_entry(s, at);
        }
    })
    .on_double_press(move |s: &mut App| {
        if let Some(at) = s.entries.iter().position(|entry| entry.path == double_path) {
            if s.entries[at].is_dir {
                let next = PathBuf::from(&s.entries[at].path);
                go(s, next);
            }
        }
    })
}

/// Wrap list content in a scroll viewport plus a drawn scrollbar gutter. The
/// viewport reports its offset into Live bindings each frame; the window-level
/// thumb follows without a view re-run, and the gutter reserves its lane.
fn scrollable(s: &App, content: El) -> El {
    let track = s.scroll_track.clone();
    let thumb = s.scroll_thumb.clone();
    let vis = s.scroll_vis.clone();
    let max = s.scroll_max.clone();
    let off = s.scroll_off.clone();
    let viewport = col()
        .key("viewport")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(move |at| {
            max.set(at.max[1]);
            off.set(at.offset[1]);
            let t = track.get();
            if at.max[1] <= 0.5 || t[3] <= 0.0 {
                vis.set(0.0);
                return;
            }
            vis.set(1.0);
            let h = (at.viewport[1] / (at.viewport[1] + at.max[1]) * t[3]).max(28.0);
            let y = t[1] + (at.offset[1] / at.max[1]) * (t[3] - h);
            thumb.set([t[0] + (GUTTER_W - 6.0) / 2.0, y, 6.0, h]);
        });
    row()
        .key("list-area")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(viewport.child(content))
        .child(
            el().key("scroll-track")
                .w(GUTTER_W)
                .h_full()
                .drawn_rect_into(&s.scroll_track)
                .on_drag(|s: &mut App, d: &mut rune::Drag, tx: &mut Tx| {
                    let max = s.scroll_max.get();
                    if max <= 0.0 || !matches!(d.phase, Phase::Down) {
                        return;
                    }
                    let thumb = s.scroll_thumb.get();
                    let range = (d.h - thumb[3]).max(1.0);
                    let off = ((d.y - thumb[3] / 2.0) / range * max).clamp(0.0, max);
                    tx.scroll_to_instant("viewport", 0.0, off);
                }),
        )
}

fn details_rows(s: &App, p: Paint) -> El {
    let list = if !s.groups.is_empty() {
        let mut list = col().key("rows").w_full();
        for i in 0..s.visible.len() {
            if let Some((_, label)) = s.groups.iter().find(|(start, _)| *start == i) {
                list = list.child(group_header(p, label));
            }
            list = list.child(file_row(s, i, p));
        }
        list
    } else {
        col()
            .key("rows")
            .w_full()
            .growth(GrowthSpec::default())
            .virtual_rows(
                s.visible.len(),
                ROW_H,
                |s: &App, i| Cow::Borrowed(s.entries[s.visible[i]].path.as_str()),
                move |s: &App, i| file_row(s, i, p),
            )
    };
    scrollable(s, list)
}

fn grid_tile(s: &App, i: usize, p: Paint) -> El {
    let ei = s.visible.get(i).copied().unwrap_or(i);
    let e = &s.entries[ei];
    let selected = s.selected == Some(ei);
    let path = e.path.clone();
    let preview = if e.kind == Kind::Image && e.fixture {
        let at = ["095523", "103957", "104522", "111606", "111619"]
            .iter()
            .position(|t| e.name.ends_with(t))
            .unwrap_or(0);
        screenshot_thumb(at, 90.0, 72.0)
    } else {
        el().w(90.0).h(72.0).center().child(file_glyph(e, p, 44.0))
    };
    let mut tile = col()
        .key(format!("tile-{i}"))
        .w(106.0)
        .radius(4.0)
        .fill(if selected { p.rail_sel } else { Color::TRANSPARENT })
        .hover(p.hover)
        .pt(8.0)
        .pb(8.0)
        .px(6.0)
        .gap(6.0)
        .items_center()
        .motion(Spring::NONE)
        .child(preview)
        .child(
            text(e.name.as_str())
                .font(CAPTION)
                .color(p.text)
                .w(94.0)
                .align(Align::Center)
                .max_lines(3),
        );
    if selected {
        tile = tile.border(p.hair, p.focus_line);
    }
    tile.on_press(move |s: &mut App| {
        if let Some(at) = s.entries.iter().position(|entry| entry.path == path) {
            select_entry(s, at);
        }
    })
}

fn grid_rows(s: &App, p: Paint) -> El {
    let tiles = row()
        .key("tiles")
        .w_full()
        .flex_wrap()
        .pt(12.0)
        .px(16.0)
        .gap(8.0)
        .children((0..s.visible.len()).map(|i| grid_tile(s, i, p)));
    scrollable(s, tiles)
}

fn quick_tile(p: Paint, label: &str, mark: Ico, top: &str, bottom: &str, back: &str, nav: usize) -> El {
    row()
        .w(272.0)
        .h(60.0)
        .radius(4.0)
        .hover(p.hover)
        .px(12.0)
        .gap(12.0)
        .items_center()
        .child(folder(40.0, top, bottom, back, Some(mark), p))
        .child(
            col()
                .gap(1.0)
                .child(text(label).font(TILE_TEXT).color(p.text))
                .child(text("Stored locally").font(CAPTION).color(p.tertiary))
                .child(ico(p, Ico::Pin, 9.0, p.tertiary)),
        )
        .on_press_tx(move |s: &mut App, tx: &mut Tx| {
            navigate(s, nav);
            tx.instant();
        })
}

fn home_content(s: &App, p: Paint) -> El {
    let tiles = [
        ("Desktop", Ico::Desktop, "#2bbad8", "#046cbd", "#0a5a9e", 3usize),
        ("Downloads", Ico::Downloads, "#2bc79a", "#0e9f76", "#0b7a5a", 4),
        ("Documents", Ico::Documents, "#b9c9d9", "#8fa6bc", "#7488a0", 5),
        ("Pictures", Ico::Pictures, "#2bb3e8", "#0e7fc4", "#0a6298", 6),
        ("Music", Ico::Music, "#f2a06b", "#e06a3c", "#b8542e", 7),
        ("Videos", Ico::Videos, "#a86fe8", "#7b3fd0", "#6230a8", 8),
    ];
    let mut grid = col().w_full().pt(10.0).gap(6.0);
    for chunk in tiles.chunks(3) {
        let mut line = row().w_full().justify_center().gap(12.0);
        for (label, mark, top, bottom, back, nav) in chunk {
            line = line.child(quick_tile(p, label, *mark, top, bottom, back, *nav));
        }
        grid = grid.child(line);
    }
    let recent_on = s.home_tab == 0;
    let pills = row()
        .w_full()
        .pt(14.0)
        .px(16.0)
        .gap(8.0)
        .items_center()
        .child(ico(p, Ico::ChevronDown, 10.0, p.secondary))
        .child(
            row()
                .h(30.0)
                .px(14.0)
                .radius(15.0)
                .fill(if recent_on { p.accent } else { p.pill_off })
                .hover(p.hover)
                .gap(7.0)
                .items_center()
                .child(ico(p, Ico::Clock, 14.0, if recent_on { p.accent_ink } else { p.secondary }))
                .child(text("Recent").font(UI_TEXT).color(if recent_on { p.accent_ink } else { p.text }))
                .on_press(|s: &mut App| s.home_tab = 0),
        )
        .child(
            row()
                .h(30.0)
                .px(14.0)
                .radius(15.0)
                .fill(if recent_on { p.pill_off } else { p.accent })
                .hover(p.hover)
                .gap(7.0)
                .items_center()
                .child(ico(p, Ico::Star, 14.0, if recent_on { p.secondary } else { p.accent_ink }))
                .child(text("Favorites").font(UI_TEXT).color(if recent_on { p.text } else { p.accent_ink }))
                .on_press(|s: &mut App| s.home_tab = 1),
        );
    let mut table = col().w_full().pt(14.0);
    if recent_on {
        table = table.child(column_header(
            p,
            &[("Name", 440.0), ("Date accessed", 240.0), ("Activity", 160.0)],
            false,
            false,
        ));
        for &ei in s.visible.iter() {
            let e = &s.entries[ei];
            table = table.child(
                row()
                    .w_full()
                    .h(46.0)
                    .items_center()
                    .hover(p.hover)
                    .child(
                        row()
                            .w(440.0)
                            .pl(16.0)
                            .gap(10.0)
                            .items_center()
                            .child(file_glyph(e, p, 24.0))
                            .child(
                                col()
                                    .gap(1.0)
                                    .child(
                                        text(e.name.as_str())
                                            .font(ROW_TEXT)
                                            .color(p.text)
                                            .truncate(),
                                    )
                                    .child(text(e.type_label.as_str()).font(CAPTION).color(p.tertiary)),
                            ),
                    )
                    .child(
                        col()
                            .w(240.0)
                            .pl(12.0)
                            .child(text(e.modified.as_str()).font(ROW_TEXT).color(p.secondary)),
                    )
                    .child(col().w(160.0))
                    .on_press(move |st: &mut App| select_entry(st, ei)),
            );
        }
    } else {
        table = table.child(
            col()
                .w_full()
                .h(120.0)
                .center()
                .child(ico(p, Ico::Star, 28.0, p.disabled))
                .child(text("No favorites yet").font(CAPTION).color(p.tertiary)),
        );
    }
    col()
        .key("home")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .fill(p.content)
        .child(
            row()
                .w_full()
                .h(34.0)
                .px(16.0)
                .gap(6.0)
                .items_center()
                .child(ico(p, Ico::ChevronDown, 10.0, p.secondary))
                .child(text("Quick access").font(TILE_TEXT).color(p.text)),
        )
        .child(grid)
        .child(pills)
        .child(table)
}

fn this_pc_content(s: &App, p: Paint) -> El {
    let selected = s.selected == Some(0);
    let drive = row()
        .w(250.0)
        .h(64.0)
        .radius(4.0)
        .px(8.0)
        .gap(10.0)
        .items_center()
        .fill(if selected { p.row_sel } else { Color::TRANSPARENT })
        .hover(p.hover)
        .child(drive_glyph(p, 40.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(text("Local Disk (C:)").font(ROW_TEXT).color(p.text))
                .child(
                    el().w_full()
                        .h(10.0)
                        .radius(2.0)
                        .fill(c("#e6e6e6"))
                        .child(el().w_frac(0.145).h_full().radius(2.0).fill(c("#0070cb"))),
                )
                .child(
                    text("1.55 TB free of 1.81 TB")
                        .font(CAPTION)
                        .color(p.tertiary),
                ),
        )
        .on_press(|s: &mut App| select_entry(s, 0))
        .on_double_press(|s: &mut App| {
            if let Some(path) = std::env::var_os("SystemDrive").map(|d| PathBuf::from(format!("{}\\", d.to_string_lossy()))) {
                go(s, path);
            }
        });
    col()
        .key("thispc")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .fill(p.content)
        .px(16.0)
        .pt(8.0)
        .child(
            row()
                .w_full()
                .h(30.0)
                .gap(6.0)
                .items_center()
                .child(ico(p, Ico::ChevronDown, 10.0, p.secondary))
                .child(text("Devices and drives").font(TILE_TEXT).color(p.text)),
        )
        .child(drive)
}

fn preview_pane(s: &App, p: Paint) -> El {
    let inner = if s.view == View::ThisPc {
        col()
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .fill(p.content)
            .child(
                col()
                    .w_full()
                    .h(210.0)
                    .fill(p.thumb_bg)
                    .center()
                    .child(ico(p, Ico::Monitor, 92.0, p.accent_fill)),
            )
            .child(
                col()
                    .w_full()
                    .px(24.0)
                    .pt(20.0)
                    .gap(16.0)
                    .child(text("This PC (1 item)").font(TILE_TEXT).color(p.text))
                    .child(
                        row()
                            .w_full()
                            .radius(4.0)
                            .border(p.hair, p.line)
                            .px(12.0)
                            .py(14.0)
                            .gap(10.0)
                            .items_center()
                            .child(ico(p, Ico::Info, 16.0, p.secondary))
                            .child(
                                text("Select a single file to get more information and share your cloud content.")
                                    .font(CAPTION)
                                    .color(p.secondary)
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .max_lines(3),
                            ),
                    ),
            )
    } else {
        let e = s.selected.and_then(|i| s.entries.get(i)).or_else(|| s.entries.first());
        let name = e.map(|x| x.name.as_str()).unwrap_or("");
        let thumb = match e {
            Some(entry) if entry.kind == Kind::Image && entry.fixture => {
                let at = ["095523", "103957", "104522", "111606", "111619"]
                    .iter()
                    .position(|t| entry.name.ends_with(t))
                    .unwrap_or(0);
                col()
                    .w_full()
                    .h(210.0)
                    .fill(p.thumb_bg)
                    .center()
                    .child(screenshot_thumb(at, PREVIEW_W - 2.0, 210.0))
            }
            Some(entry) => col()
                .w_full()
                .h(210.0)
                .fill(p.thumb_bg)
                .center()
                .child(file_glyph(entry, p, 64.0)),
            None => col().w_full().h(210.0).fill(p.thumb_bg),
        };
        let location = s.crumbs.join(" / ");
        let mut details = col()
            .w_full()
            .px(24.0)
            .pt(18.0)
            .gap(10.0)
            .child(text("Details").font(TILE_TEXT).color(p.text));
        let rows: [(&str, String); 4] = [
            ("Type", e.map(|x| x.type_label.clone()).unwrap_or_default()),
            ("Size", e.map(|x| x.size_label()).unwrap_or_default()),
            ("File location", location),
            ("Date modified", e.map(|x| x.modified.clone()).unwrap_or_default()),
        ];
        for (k, v) in rows {
            details = details.child(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_center()
                    .child(col().w(96.0).child(text(k).font(CAPTION).color(p.tertiary)))
                    .child(
                        text(v)
                            .font(CAPTION)
                            .color(p.secondary)
                            .grow(1.0)
                            .min_w(0.0)
                            .align(Align::End)
                            .truncate(),
                    ),
            );
        }
        col()
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .fill(p.content)
            .child(thumb)
            .child(
                col()
                    .w_full()
                    .px(24.0)
                    .pt(18.0)
                    .gap(12.0)
                    .child(
                        row()
                            .w_full()
                            .gap(10.0)
                            .items_center()
                            .child(ico(p, Ico::Pictures, 22.0, p.secondary))
                            .child(
                                text(name)
                                    .font(TextStyle::new(13.0, 600.0).lh(18.0))
                                    .color(p.text)
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .max_lines(2),
                            ),
                    )
                    .child(
                        row()
                            .h(30.0)
                            .px(10.0)
                            .radius(4.0)
                            .fill(p.pill_off)
                            .hover(p.hover)
                            .gap(8.0)
                            .items_center()
                            .child(ico(p, Ico::ShareSmall, 14.0, p.accent))
                            .child(text("Share").font(CAPTION).color(p.text))
                            .on_press(|s: &mut App| {
                                s.notice = "Share is not implemented in this slice".into();
                            }),
                    ),
            )
            .child(details)
    };
    row()
        .key("preview")
        .w(PREVIEW_W)
        .h_full()
        .fill(p.content)
        .child(el().w(p.hair).h_full().fill(p.line))
        .child(inner)
}

fn content(s: &App, p: Paint) -> El {
    let main = match s.view {
        View::Home => home_content(s, p),
        View::ThisPc => this_pc_content(s, p),
        View::Files => col()
            .key("content")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .fill(p.content)
            .child(if s.mode == Mode::Details {
                column_header(
                    p,
                    &[
                        ("Name", NAME_W),
                        ("Date modified", DATE_W),
                        ("Type", TYPE_W),
                        ("Size", SIZE_W),
                    ],
                    true,
                    true,
                )
            } else {
                el().w_full().h(p.hair)
            })
            .child(el().w_full().h(p.hair).fill(p.line))
            .child(if s.mode == Mode::Details {
                details_rows(s, p)
            } else {
                grid_rows(s, p)
            }),
    };
    if s.preview {
        row()
            .key("body")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(nav_rail(s, p))
            .child(main)
            .child(preview_pane(s, p))
    } else {
        row()
            .key("body")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(nav_rail(s, p))
            .child(main)
    }
}

fn body(s: &App, p: Paint) -> El {
    content(s, p)
}

fn status_bar(s: &App, p: Paint) -> El {
    let middle = if s.notice.is_empty() {
        s.selection_detail.as_str()
    } else {
        s.notice.as_str()
    };
    let toggle = |which: Ico, active: bool, grid: bool| {
        el().size(24.0, 18.0)
            .radius(3.0)
            .fill(if active { p.rail_sel } else { Color::TRANSPARENT })
            .hover(p.hover)
            .center()
            .child(ico(p, which, 13.0, if active { p.text } else { p.tertiary }))
            .on_press(move |s: &mut App| {
                s.mode = if grid { Mode::Grid } else { Mode::Details };
            })
    };
    row()
        .key("status")
        .w_full()
        .h(STATUS_H)
        .fill(p.status)
        .px(12.0)
        .gap(8.0)
        .items_center()
        .child(
            text(if s.count == 1 {
                "1 item".to_string()
            } else {
                format!("{} items", s.count)
            })
            .font(CAPTION)
            .color(p.secondary),
        )
        .child(el().w(p.hair).h(12.0).fill(p.line))
        .child(text(middle).font(CAPTION).color(if s.notice.is_empty() {
            p.secondary
        } else {
            p.accent
        }))
        .child(el().grow(1.0))
        .child(
            row()
                .gap(1.0)
                .px(2.0)
                .py(1.0)
                .radius(4.0)
                .border(p.hair, p.line)
                .child(toggle(Ico::List, s.mode == Mode::Details, false))
                .child(toggle(Ico::Grid, s.mode == Mode::Grid, true)),
        )
}

/// Register the system Segoe UI faces so the shell reads in Explorer's own
/// type rather than Rune's bundled Inter. Rune does not load system fonts, so
/// the bytes are read once and leaked for the process lifetime; when the faces
/// are absent the family list falls back to Inter.
fn load_fonts() -> &'static [&'static [u8]] {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let mut out: Vec<&'static [u8]> = Vec::new();
    for name in ["segoeui.ttf", "seguisb.ttf", "segoeuib.ttf", "segoeuil.ttf", "consola.ttf"] {
        if let Ok(bytes) = fs::read(format!("{windir}\\Fonts\\{name}")) {
            out.push(Box::leak(bytes.into_boxed_slice()));
        }
    }
    Box::leak(out.into_boxed_slice())
}

/// A borderless window has no system frame, so Rune's `Tx::minimize` refuses it
/// and there is no `Tx::maximize`. Drive the HWND directly: DWM rounds the frame
/// (Rune's corner radius is AppKit-only), `ShowWindow` minimizes and truly
/// maximizes/restores, and a `WM_NCHITTEST` subclass hands the outer band back to
/// the system as resize edges so the border drags smoothly. Everything links to
/// system libraries by name, so this needs no extra crate.
#[cfg(target_os = "windows")]
mod winframe {
    use rune::WindowHandle;
    use rune::raw_window_handle::RawWindowHandle;
    use std::sync::OnceLock;

    static HWND: OnceLock<isize> = OnceLock::new();

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    type SubclassProc =
        unsafe extern "system" fn(isize, u32, usize, isize, usize, usize) -> isize;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attribute: u32,
            value: *const core::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        fn IsZoomed(hwnd: isize) -> i32;
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn DefSubclassProc(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    }
    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn SetWindowSubclass(hwnd: isize, proc: SubclassProc, id: usize, data: usize) -> i32;
    }

    const WM_NCHITTEST: u32 = 0x0084;
    const SW_MINIMIZE: i32 = 6;
    const SW_MAXIMIZE: i32 = 3;
    const SW_RESTORE: i32 = 9;
    const HTLEFT: isize = 10;
    const HTRIGHT: isize = 11;
    const HTTOP: isize = 12;
    const HTTOPLEFT: isize = 13;
    const HTTOPRIGHT: isize = 14;
    const HTBOTTOM: isize = 15;
    const HTBOTTOMLEFT: isize = 16;
    const HTBOTTOMRIGHT: isize = 17;
    const BORDER: i32 = 8;

    /// Claim the outer band as resize edges; leave the rest to Rune's own proc.
    unsafe extern "system" fn hit_test(
        hwnd: isize,
        msg: u32,
        wparam: usize,
        lparam: isize,
        _id: usize,
        _data: usize,
    ) -> isize {
        unsafe {
            if msg == WM_NCHITTEST && IsZoomed(hwnd) == 0 {
                let x = (lparam as u16) as i16 as i32;
                let y = ((lparam >> 16) as u16) as i16 as i32;
                let mut r = Rect { left: 0, top: 0, right: 0, bottom: 0 };
                if GetWindowRect(hwnd, &mut r) != 0 {
                    let left = x < r.left + BORDER;
                    let right = x >= r.right - BORDER;
                    let top = y < r.top + BORDER;
                    let bottom = y >= r.bottom - BORDER;
                    let code = match (top, bottom, left, right) {
                        (true, _, true, _) => HTTOPLEFT,
                        (true, _, _, true) => HTTOPRIGHT,
                        (_, true, true, _) => HTBOTTOMLEFT,
                        (_, true, _, true) => HTBOTTOMRIGHT,
                        (true, _, _, _) => HTTOP,
                        (_, true, _, _) => HTBOTTOM,
                        (_, _, true, _) => HTLEFT,
                        (_, _, _, true) => HTRIGHT,
                        _ => 0,
                    };
                    if code != 0 {
                        return code;
                    }
                }
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
    }

    /// Round the frame and install the resize-edge subclass once the HWND exists.
    pub fn install(handle: &WindowHandle) {
        let _ = handle.with_raw_handle(|raw| {
            if let RawWindowHandle::Win32(win) = raw.as_raw() {
                let hwnd = win.hwnd.get();
                let _ = HWND.set(hwnd);
                const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
                const DWMWCP_ROUND: u32 = 2;
                let preference = DWMWCP_ROUND;
                unsafe {
                    DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_WINDOW_CORNER_PREFERENCE,
                        (&preference as *const u32).cast(),
                        core::mem::size_of::<u32>() as u32,
                    );
                    SetWindowSubclass(hwnd, hit_test, 1, 0);
                }
            }
        });
    }

    pub fn minimize() {
        if let Some(&hwnd) = HWND.get() {
            unsafe {
                ShowWindow(hwnd, SW_MINIMIZE);
            }
        }
    }

    /// Maximize when restored, restore when maximized; returns the new zoom state.
    pub fn toggle_maximize() -> bool {
        let Some(&hwnd) = HWND.get() else {
            return false;
        };
        unsafe {
            if IsZoomed(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
                false
            } else {
                ShowWindow(hwnd, SW_MAXIMIZE);
                true
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod winframe {
    pub fn install(_handle: &rune::WindowHandle) {}
    pub fn minimize() {}
    pub fn toggle_maximize() -> bool {
        false
    }
}

fn main() {
    let spec = AppSpec::<App> {
        scenes: &[
            ("dark", |s| {
                s.dark = true;
                load(s, Fixture::Downloads);
            }),
            ("light", |s| {
                s.dark = false;
                load(s, Fixture::Downloads);
            }),
            ("home", |s| load(s, Fixture::Home)),
            ("downloads", |s| load(s, Fixture::Downloads)),
            ("pictures", |s| load(s, Fixture::Screenshots)),
            ("thispc", |s| load(s, Fixture::ThisPc)),
        ],
        poll: Some(poll),
        fonts: load_fonts(),
        families: ("Segoe UI, Inter", "Consolas, IBM Plex Mono"),
        ..AppSpec::new("Lyre", (1076.0, 600.0), init, view)
    };
    let probe = init();
    eprintln!(
        "lyre-rune: {} entries in {} in {:.2} ms",
        probe.entries.len(),
        probe.root.display(),
        probe.scan_ms
    );
    // One window, no system frame. The tab strip is the titlebar now, so the
    // drawn minimize/maximize/close controls and window_drag own that job;
    // leaving decorations on would stack a second, native bar above it.
    run_windows(|_displays| {
        let window = WindowSpec::new("main", spec).options(WindowOptions {
            borderless: true,
            resizable: true,
            min_size: Some((760.0, 480.0)),
            ..Default::default()
        });
        let window = window.on_created(|handle| winframe::install(&handle));
        [window.boxed()]
    });
}
