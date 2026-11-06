//! Lyre's Explorer shell, drawn natively by Rune.
//!
//! Additive experiment on `feat/rune-native-ui`: the Windows 11 Explorer shell
//! from the reference captures, rendered by Rune's GPU renderer instead of a
//! Tauri webview. The default window reads a real directory; named scenes
//! reproduce the four reference views so a headless PNG can be compared to them.
//!
//!   cargo run
//!   cargo run -- --snap downloads.png --scene downloads --size 1076x600 --scale 1
//!   cargo run -- --snap home.png      --scene home
//!   cargo run -- --snap pictures.png  --scene pictures
//!   cargo run -- --snap thispc.png    --scene thispc
//!   cargo run -- --snap light.png     --scene light
//!   LYRE_RUNE_PATH=... cargo run
//!
//! Out of scope for this slice: independent tab history, real search, sorting,
//! file operations, OS drag-and-drop, system context menus, thumbnails, the
//! reveal highlight, and the Segoe UI Variable face (Rune ships Inter).

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use rune::color::hex;
use rune::{
    Align, AppSpec, Color, Cx, El, GrowthSpec, LineCap, ScrollSpec, Shape, TextStyle, canvas, col,
    el, row, text,
};

// Metrics sampled from the reference captures (1076x600 window).
const TAB_H: f32 = 40.0;
const ADDRESS_H: f32 = 44.0;
const COMMAND_H: f32 = 44.0;
const STATUS_H: f32 = 24.0;
const RAIL_W: f32 = 155.0;
const HEADER_H: f32 = 32.0;
const ROW_H: f32 = 30.0;
const FIELD_H: f32 = 28.0;
const PREVIEW_W: f32 = 286.0;
const NAME_W: f32 = 262.0;
const DATE_W: f32 = 136.0;
const TYPE_W: f32 = 114.0;
const SIZE_W: f32 = 81.0;

const CAPTION: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
const ROW_TEXT: TextStyle = TextStyle::new(13.0, 400.0).lh(18.0);
const BODY: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0);
const TILE_TEXT: TextStyle = TextStyle::new(14.0, 600.0).lh(18.0);

fn c(s: &str) -> Color {
    hex(s).expect("authored hex literal")
}

#[derive(Clone, Copy)]
struct Paint {
    title: Color,
    chrome: Color,
    tab: Color,
    content: Color,
    status: Color,
    field: Color,
    line: Color,
    soft_line: Color,
    text: Color,
    secondary: Color,
    tertiary: Color,
    disabled: Color,
    accent: Color,
    accent_fill: Color,
    hover: Color,
    rail_sel: Color,
    row_sel: Color,
    hair: f32,
}

fn paint(dark: bool, scale: f32) -> Paint {
    let hair = 1.0 / scale;
    if dark {
        Paint {
            title: c("#0a0a0a"),
            chrome: c("#1f1f1f"),
            tab: c("#1f1f1f"),
            content: c("#191919"),
            status: c("#1c1c1c"),
            field: c("#2c2c2c"),
            line: c("#3a3a3a"),
            soft_line: c("#2b2b2b"),
            text: c("#ffffff"),
            secondary: c("#c8c8c8"),
            tertiary: c("#9a9a9a"),
            disabled: c("#6b6b6b"),
            accent: c("#60cdff"),
            accent_fill: c("#0078d4"),
            hover: Color::WHITE.alpha(0.055),
            rail_sel: c("#333333"),
            row_sel: c("#60cdff").alpha(0.16),
            hair,
        }
    } else {
        Paint {
            title: c("#e8e8e8"),
            chrome: c("#f5f5f5"),
            tab: c("#ffffff"),
            content: c("#ffffff"),
            status: c("#f5f5f5"),
            field: c("#ffffff"),
            line: c("#dcdcdc"),
            soft_line: c("#ebebeb"),
            text: c("#191a1a"),
            secondary: c("#5c5d5e"),
            tertiary: c("#767778"),
            disabled: c("#9a9b9c"),
            accent: c("#005fb8"),
            accent_fill: c("#0078d4"),
            hover: Color::BLACK.alpha(0.045),
            rail_sel: c("#e4e4e4"),
            row_sel: c("#005fb8").alpha(0.11),
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

fn kind_hex(k: Kind) -> &'static str {
    match k {
        Kind::Folder => "#f0b429",
        Kind::Drive => "#4a90d9",
        Kind::Image => "#1f9e6e",
        Kind::Video => "#8b5cf6",
        Kind::Audio => "#e0518c",
        Kind::Pdf => "#d94848",
        Kind::Document => "#2f6fd0",
        Kind::Sheet => "#1f8f4e",
        Kind::Slides => "#e0642a",
        Kind::Text => "#6b7280",
        Kind::Code => "#4f6ef7",
        Kind::Archive => "#c2861f",
        Kind::App => "#3b82c4",
        Kind::Other => "#8f9296",
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
    selected: Option<usize>,
    focused: Option<usize>,
    nav: usize,
    count: usize,
    selection_detail: String,
    scan_ms: f64,
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

fn load(s: &mut App, f: Fixture) {
    s.groups.clear();
    s.preview = false;
    s.contextual = false;
    s.mode = Mode::Details;
    s.selected = None;
    s.focused = None;
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
    App {
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
        selected: None,
        focused: None,
        nav: 9,
        count,
        selection_detail: String::new(),
        scan_ms,
    }
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
    Shape::new().stroke(points, 1.5, ink)
}

fn closed(points: Vec<[f32; 2]>, ink: Color) -> Shape {
    Shape::new().stroke_with(points, true, 1.5, LineCap::Round, ink)
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
    Home,
    Gallery,
    Cloud,
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
    ThisPc,
    Network,
    Monitor,
    Minimize,
    Maximize,
    Close,
    Info,
    ShareSmall,
    SetBackground,
    RotateLeft,
    RotateRight,
}

fn ico(p: Paint, which: Ico, size: f32, ink: Color) -> El {
    const W: f32 = 1.5;
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
        Ico::Refresh => Shape::new()
            .stroke_with(
                vec![[19.0, 9.0], [19.0, 5.0], [15.0, 5.0]],
                false,
                W,
                LineCap::Round,
                ink,
            )
            .stroke(
                vec![
                    [5.0, 12.0],
                    [5.0, 16.0],
                    [8.0, 19.0],
                    [12.0, 19.0],
                    [17.0, 19.0],
                    [19.0, 15.0],
                    [19.0, 12.0],
                    [19.0, 8.0],
                    [16.0, 5.0],
                    [12.0, 5.0],
                    [8.0, 5.0],
                    [5.0, 8.0],
                    [5.0, 12.0],
                ],
                W,
                ink,
            ),
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
                [8.0, 3.0],
                [16.0, 3.0],
                [17.0, 8.0],
                [13.0, 11.0],
                [13.0, 20.0],
                [11.0, 20.0],
                [11.0, 11.0],
                [7.0, 8.0],
            ],
            ink,
        ),
        Ico::Home => Shape::new().fill(
            vec![
                [3.0, 12.0],
                [12.0, 4.0],
                [21.0, 12.0],
                [19.0, 12.0],
                [19.0, 20.0],
                [13.0, 20.0],
                [13.0, 14.0],
                [11.0, 14.0],
                [11.0, 20.0],
                [5.0, 20.0],
                [5.0, 12.0],
            ],
            ink,
        ),
        Ico::Gallery => closed(
            vec![[3.0, 5.0], [21.0, 5.0], [21.0, 19.0], [3.0, 19.0]],
            ink,
        )
        .fill(circle(8.0, 10.0, 1.6), ink)
        .fill(vec![[4.0, 18.0], [10.0, 12.0], [16.0, 18.0]], ink),
        Ico::Cloud => Shape::new().stroke_with(
            vec![
                [6.0, 18.0],
                [4.0, 14.0],
                [6.0, 10.0],
                [10.0, 9.0],
                [12.0, 5.0],
                [17.0, 6.0],
                [20.0, 10.0],
                [19.0, 14.0],
                [20.0, 18.0],
            ],
            true,
            W,
            LineCap::Round,
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
        Ico::Network => closed(circle(12.0, 12.0, 8.0), ink)
            .line([4.0, 12.0], [20.0, 12.0], W, LineCap::Round, ink)
            .stroke(
                vec![
                    [12.0, 4.0],
                    [9.0, 12.0],
                    [12.0, 20.0],
                    [15.0, 12.0],
                    [12.0, 4.0],
                ],
                W,
                ink,
            ),
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
    };
    let _ = p;
    canvas(sh.view_box(24.0, 24.0)).size(size, size)
}

fn file_glyph(e: &Entry, _p: Paint, size: f32) -> El {
    let color = c(if e.kind == Kind::App {
        app_color(&e.name)
    } else {
        kind_hex(e.kind)
    });
    if e.is_dir {
        let sh = Shape::new()
            .fill(
                vec![
                    [1.0, 4.0],
                    [1.0, 3.0],
                    [9.0, 3.0],
                    [11.0, 6.0],
                    [23.0, 6.0],
                    [23.0, 21.0],
                    [1.0, 21.0],
                ],
                color,
            )
            .fill(
                vec![[1.0, 10.0], [23.0, 10.0], [23.0, 21.0], [1.0, 21.0]],
                color.mix(Color::BLACK, 0.22),
            );
        return canvas(sh.view_box(24.0, 24.0)).size(size, size);
    }
    let sh = Shape::new()
        .fill(
            vec![
                [4.0, 1.0],
                [15.0, 1.0],
                [20.0, 6.0],
                [20.0, 23.0],
                [4.0, 23.0],
            ],
            color,
        )
        .fill(
            vec![[15.0, 1.0], [15.0, 6.0], [20.0, 6.0]],
            Color::WHITE.alpha(0.5),
        );
    let mark = e
        .name
        .rsplit_once('.')
        .and_then(|(_, ext)| ext.chars().next())
        .unwrap_or('?')
        .to_ascii_uppercase();
    el().key("glyph")
        .size(size, size)
        .center()
        .child(
            canvas(sh.view_box(24.0, 24.0))
                .absolute(0.0, 0.0)
                .size(size, size),
        )
        .child(
            text(mark.to_string())
                .font(TextStyle::new(8.0, 600.0))
                .color(Color::WHITE),
        )
}

fn drive_glyph(p: Paint, size: f32) -> El {
    let sh = Shape::new()
        .fill(
            vec![[1.0, 3.0], [23.0, 3.0], [23.0, 17.0], [1.0, 17.0]],
            c("#4a90d9"),
        )
        .fill(
            vec![[1.0, 14.0], [23.0, 14.0], [23.0, 17.0], [1.0, 17.0]],
            c("#2f6ca8"),
        )
        .fill(
            vec![[3.0, 5.0], [10.0, 5.0], [10.0, 9.0], [3.0, 9.0]],
            Color::WHITE.alpha(0.85),
        )
        .fill(
            vec![[3.0, 18.0], [21.0, 18.0], [21.0, 20.0], [3.0, 20.0]],
            c("#8f9296"),
        );
    let _ = p;
    canvas(sh.view_box(24.0, 24.0)).size(size, size)
}

// ----------------------------------------------------------------- view ----

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let p = paint(s.dark, cx.env().scale);
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
}

fn tab_strip(s: &App, p: Paint) -> El {
    let tab = row()
        .key("tab")
        .h(32.0)
        .w(220.0)
        .corners([8.0, 8.0, 0.0, 0.0])
        .fill(p.tab)
        .px(10.0)
        .gap(8.0)
        .items_center()
        .child(ico(p, Ico::Home, 14.0, c(kind_hex(Kind::Folder))))
        .child(text(s.tab.as_str()).font(CAPTION).color(p.text).truncate())
        .child(el().grow(1.0))
        .child(text("×").font(CAPTION).color(p.tertiary));
    let mut strip = row()
        .key("tabs")
        .w_full()
        .h(TAB_H)
        .fill(p.title)
        .items_end()
        .px(8.0)
        .gap(4.0)
        .child(tab)
        .child(
            el().key("new-tab")
                .size(28.0, 28.0)
                .radius(4.0)
                .hover(p.hover)
                .center()
                .child(text("+").font(BODY).color(p.secondary)),
        )
        .child(el().grow(1.0));
    for (i, w) in [Ico::Minimize, Ico::Maximize, Ico::Close]
        .iter()
        .enumerate()
    {
        strip = strip.child(
            el().key(format!("win-{i}"))
                .size(46.0, 32.0)
                .hover(p.hover)
                .center()
                .child(ico(p, *w, 14.0, p.secondary)),
        );
    }
    strip
}

fn nav_button(p: Paint, which: Ico, key: &str) -> El {
    el().key(key)
        .size(30.0, 30.0)
        .radius(4.0)
        .hover(p.hover)
        .center()
        .child(ico(p, which, 15.0, p.secondary))
}

fn address_row(s: &App, p: Paint) -> El {
    let mut crumb = row()
        .key("breadcrumb")
        .grow(1.0)
        .min_w(120.0)
        .h(FIELD_H)
        .radius(4.0)
        .fill(p.field)
        .border(p.hair, p.line)
        .px(10.0)
        .gap(4.0)
        .items_center()
        .child(ico(p, Ico::Desktop, 14.0, p.secondary));
    for (i, part) in s.crumbs.iter().enumerate() {
        crumb = crumb
            .child(ico(p, Ico::ChevronRight, 12.0, p.tertiary))
            .child(text(part.as_str()).font(CAPTION).color(p.text).truncate());
        if i + 1 == s.crumbs.len() {
            crumb = crumb.child(ico(p, Ico::ChevronRight, 12.0, p.tertiary));
        }
    }
    row()
        .key("address")
        .w_full()
        .h(ADDRESS_H)
        .fill(p.chrome)
        .px(8.0)
        .gap(2.0)
        .items_center()
        .child(nav_button(p, Ico::Back, "back"))
        .child(nav_button(p, Ico::Forward, "forward"))
        .child(nav_button(p, Ico::Up, "up"))
        .child(nav_button(p, Ico::Refresh, "refresh"))
        .child(el().w(8.0))
        .child(crumb)
        .child(
            row()
                .key("search")
                .w(220.0)
                .h(FIELD_H)
                .radius(4.0)
                .fill(p.field)
                .border(p.hair, p.line)
                .px(10.0)
                .items_center()
                .child(text(s.search.as_str()).font(CAPTION).color(p.tertiary))
                .child(el().grow(1.0))
                .child(ico(p, Ico::Search, 14.0, p.tertiary)),
        )
}

fn icon_button(p: Paint, which: Ico, dim: bool) -> El {
    let ink = if dim { p.disabled } else { p.secondary };
    el().size(32.0, 32.0)
        .radius(4.0)
        .hover(p.hover)
        .center()
        .child(ico(p, which, 18.0, ink))
}

fn text_button(p: Paint, which: Ico, label: &str, chevron: bool, dim: bool) -> El {
    let ink = if dim { p.disabled } else { p.secondary };
    let mut b = row()
        .h(32.0)
        .px(8.0)
        .gap(6.0)
        .radius(4.0)
        .hover(p.hover)
        .items_center()
        .child(ico(p, which, 16.0, ink))
        .child(
            text(label)
                .font(CAPTION)
                .color(if dim { p.disabled } else { p.text }),
        );
    if chevron {
        b = b.child(ico(p, Ico::ChevronDown, 11.0, ink));
    }
    b
}

fn command_bar(s: &App, p: Paint) -> El {
    let mut bar = row()
        .key("command")
        .w_full()
        .h(COMMAND_H)
        .fill(p.chrome)
        .px(8.0)
        .gap(2.0)
        .items_center()
        .child(text_button(p, Ico::New, "New", true, false))
        .child(el().w(p.hair).h(20.0).fill(p.line));
    for w in [
        Ico::Cut,
        Ico::Copy,
        Ico::Paste,
        Ico::Rename,
        Ico::Share,
        Ico::Delete,
    ] {
        bar = bar.child(icon_button(p, w, true));
    }
    bar = bar
        .child(el().w(p.hair).h(20.0).fill(p.line))
        .child(text_button(p, Ico::Sort, "Sort", true, false))
        .child(text_button(p, Ico::View, "View", true, false))
        .child(text_button(p, Ico::Filter, "Filter", true, false));
    if s.contextual {
        bar = bar
            .child(text_button(
                p,
                Ico::SetBackground,
                "Set as background",
                false,
                false,
            ))
            .child(text_button(p, Ico::RotateLeft, "Rotate left", false, false))
            .child(text_button(
                p,
                Ico::RotateRight,
                "Rotate right",
                false,
                false,
            ));
    }
    bar = bar
        .child(icon_button(p, Ico::More, false))
        .child(el().grow(1.0));
    bar.child(text_button(p, Ico::Details, "Details", false, false))
}

fn rail_item(
    s: &App,
    p: Paint,
    index: usize,
    label: &str,
    which: Ico,
    chevron: bool,
    pin: bool,
) -> El {
    let active = s.nav == index;
    let ink = if active { p.text } else { p.secondary };
    let mut item = row()
        .key(format!("rail-{index}"))
        .w_full()
        .h(30.0)
        .radius(4.0)
        .fill(if active {
            p.rail_sel
        } else {
            Color::TRANSPARENT
        })
        .hover(p.hover)
        .px(8.0)
        .gap(8.0)
        .items_center();
    if active {
        item = item.child(
            el().w(3.0)
                .h(16.0)
                .radius(1.5)
                .fill(p.accent)
                .absolute(0.0, 7.0),
        );
    }
    if chevron {
        item = item.child(ico(p, Ico::ChevronRight, 10.0, p.tertiary));
    } else {
        item = item.child(el().w(10.0));
    }
    item = item
        .child(ico(p, which, 16.0, ink))
        .child(text(label).font(BODY).color(ink));
    if pin {
        item = item
            .child(el().grow(1.0))
            .child(ico(p, Ico::Pin, 10.0, p.tertiary));
    } else {
        item = item.child(el().grow(1.0));
    }
    item.on_press(move |s: &mut App| s.nav = index)
}

fn nav_rail(s: &App, p: Paint) -> El {
    col()
        .key("rail")
        .w(RAIL_W)
        .h_full()
        .fill(p.content)
        .px(4.0)
        .pt(4.0)
        .gap(1.0)
        .child(rail_item(s, p, 0, "Home", Ico::Home, false, false))
        .child(rail_item(s, p, 1, "Gallery", Ico::Gallery, false, false))
        .child(rail_item(s, p, 2, "OneDrive", Ico::Cloud, true, false))
        .child(el().w_full().h(p.hair).fill(p.soft_line).key("rail-sep-1"))
        .child(rail_item(s, p, 3, "Desktop", Ico::Desktop, false, true))
        .child(rail_item(s, p, 4, "Downloads", Ico::Downloads, false, true))
        .child(rail_item(s, p, 5, "Documents", Ico::Documents, false, true))
        .child(rail_item(s, p, 6, "Pictures", Ico::Pictures, false, true))
        .child(rail_item(s, p, 7, "Music", Ico::Music, false, true))
        .child(rail_item(s, p, 8, "Videos", Ico::Videos, false, true))
        .child(el().w_full().h(p.hair).fill(p.soft_line).key("rail-sep-2"))
        .child(rail_item(s, p, 9, "This PC", Ico::ThisPc, true, false))
        .child(rail_item(s, p, 10, "Network", Ico::Network, true, false))
        .child(el().grow(1.0))
}

fn column_header(p: Paint, cols: &[(&str, f32)], right_last: bool) -> El {
    let mut h = row().key("header").w_full().h(HEADER_H).items_center();
    for (i, (label, w)) in cols.iter().enumerate() {
        if i > 0 {
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
        .h(30.0)
        .px(12.0)
        .gap(6.0)
        .items_center()
        .child(ico(p, Ico::ChevronDown, 11.0, p.secondary))
        .child(text(label).font(TILE_TEXT).color(p.text))
}

fn file_row(s: &App, i: usize, p: Paint) -> El {
    let e = &s.entries[i];
    let selected = s.selected == Some(i);
    let focused = s.focused == Some(i);
    let fill = if selected {
        p.row_sel
    } else if focused {
        p.hover
    } else {
        Color::TRANSPARENT
    };
    let path = e.path.clone();
    let mut line = row()
        .key(format!("row-{i}"))
        .w_full()
        .h(ROW_H)
        .items_center()
        .fill(fill)
        .hover(p.hover);
    if focused && selected {
        line = line.border(p.hair, p.accent);
    }
    line.child(
        row()
            .w(NAME_W)
            .pl(12.0)
            .pr(8.0)
            .gap(8.0)
            .items_center()
            .child(file_glyph(e, p, 20.0))
            .child(
                text(e.name.as_str())
                    .font(ROW_TEXT)
                    .color(p.text)
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
    .child(el().grow(1.0))
    .on_press(move |s: &mut App| {
        if let Some(at) = s.entries.iter().position(|entry| entry.path == path) {
            s.selected = Some(at);
            s.focused = Some(at);
        }
    })
}

fn details_rows(s: &App, p: Paint) -> El {
    let viewport = col()
        .key("viewport")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if !s.groups.is_empty() {
        let mut list = col().key("rows").w_full();
        for i in 0..s.entries.len() {
            if let Some((_, label)) = s.groups.iter().find(|(start, _)| *start == i) {
                list = list.child(group_header(p, label));
            }
            list = list.child(file_row(s, i, p));
        }
        return viewport.child(list);
    }
    viewport.child(
        col()
            .key("rows")
            .w_full()
            .growth(GrowthSpec::default())
            .virtual_rows(
                s.entries.len(),
                ROW_H,
                |s: &App, i| Cow::Borrowed(s.entries[i].path.as_str()),
                move |s: &App, i| file_row(s, i, p),
            ),
    )
}

fn grid_tile(s: &App, i: usize, p: Paint) -> El {
    let e = &s.entries[i];
    let selected = s.selected == Some(i);
    let path = e.path.clone();
    col()
        .key(format!("tile-{i}"))
        .w(104.0)
        .h(140.0)
        .radius(4.0)
        .fill(if selected {
            p.rail_sel
        } else {
            Color::TRANSPARENT
        })
        .hover(p.hover)
        .pt(10.0)
        .px(6.0)
        .gap(6.0)
        .items_center()
        .child(
            el().w(88.0)
                .h(72.0)
                .radius(2.0)
                .fill(p.field)
                .border(p.hair, p.line)
                .center()
                .child(file_glyph(e, p, 44.0)),
        )
        .child(
            text(e.name.as_str())
                .font(CAPTION)
                .color(p.text)
                .w(92.0)
                .align(Align::Center)
                .max_lines(2),
        )
        .on_press(move |s: &mut App| {
            if let Some(at) = s.entries.iter().position(|entry| entry.path == path) {
                s.selected = Some(at);
                s.focused = Some(at);
            }
        })
}

fn grid_rows(s: &App, p: Paint) -> El {
    col()
        .key("viewport")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            row()
                .key("tiles")
                .w_full()
                .pt(16.0)
                .px(12.0)
                .gap(8.0)
                .children((0..s.entries.len()).map(|i| grid_tile(s, i, p))),
        )
}

fn quick_tile(p: Paint, label: &str, which: Ico, color: &str) -> El {
    row()
        .w(220.0)
        .h(64.0)
        .gap(12.0)
        .items_center()
        .child(
            el().size(40.0, 40.0)
                .radius(2.0)
                .fill(c(color))
                .center()
                .child(ico(p, which, 22.0, Color::WHITE))
                .child(ico(p, Ico::Pin, 9.0, Color::WHITE).absolute(28.0, 28.0)),
        )
        .child(
            col()
                .gap(2.0)
                .child(text(label).font(TILE_TEXT).color(p.text))
                .child(text("Stored locally").font(CAPTION).color(p.tertiary)),
        )
}

fn home_content(s: &App, p: Paint) -> El {
    let tiles = [
        ("Desktop", Ico::Desktop, kind_hex(Kind::Folder)),
        ("Downloads", Ico::Downloads, "#1f9e6e"),
        ("Documents", Ico::Documents, "#2f6fd0"),
        ("Pictures", Ico::Pictures, "#1f9e6e"),
        ("Music", Ico::Music, "#e0518c"),
        ("Videos", Ico::Videos, "#8b5cf6"),
    ];
    let mut grid = col().w_full().px(16.0).pt(12.0).gap(8.0);
    for pair in tiles.chunks(3) {
        let mut line = row().w_full().gap(8.0);
        for (label, which, color) in pair {
            line = line.child(quick_tile(p, *label, *which, *color));
        }
        grid = grid.child(line);
    }
    let recent = row()
        .w_full()
        .pt(16.0)
        .px(16.0)
        .gap(8.0)
        .child(
            row()
                .h(30.0)
                .px(14.0)
                .radius(15.0)
                .fill(p.accent_fill)
                .gap(8.0)
                .items_center()
                .child(ico(p, Ico::Info, 13.0, Color::WHITE))
                .child(text("Recent").font(CAPTION).color(Color::WHITE)),
        )
        .child(
            row()
                .h(30.0)
                .px(14.0)
                .radius(15.0)
                .fill(p.field)
                .gap(8.0)
                .items_center()
                .child(ico(p, Ico::Gallery, 13.0, p.secondary))
                .child(text("Favorites").font(CAPTION).color(p.text)),
        );
    let mut table = col().w_full().pt(16.0);
    table = table.child(column_header(
        p,
        &[
            ("Name", 440.0),
            ("Date accessed", 220.0),
            ("Activity", 160.0),
        ],
        false,
    ));
    for (i, e) in s.entries.iter().enumerate() {
        table = table.child(
            row()
                .w_full()
                .h(34.0)
                .items_center()
                .child(
                    row()
                        .w(440.0)
                        .pl(12.0)
                        .gap(8.0)
                        .items_center()
                        .child(file_glyph(e, p, 20.0))
                        .child(
                            text(e.name.as_str())
                                .font(ROW_TEXT)
                                .color(p.text)
                                .truncate(),
                        ),
                )
                .child(
                    col()
                        .w(220.0)
                        .pl(12.0)
                        .child(text(e.modified.as_str()).font(ROW_TEXT).color(p.secondary)),
                )
                .child(
                    col().w(160.0).pl(12.0).child(
                        text(e.type_label.as_str())
                            .font(ROW_TEXT)
                            .color(p.secondary),
                    ),
                )
                .on_press(move |st: &mut App| st.selected = Some(i)),
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
                .h(30.0)
                .px(16.0)
                .gap(6.0)
                .items_center()
                .child(ico(p, Ico::ChevronDown, 11.0, p.secondary))
                .child(text("Quick access").font(TILE_TEXT).color(p.text)),
        )
        .child(grid)
        .child(recent)
        .child(table)
}

fn this_pc_content(s: &App, p: Paint) -> El {
    let drive = col()
        .w(240.0)
        .h(64.0)
        .px(8.0)
        .gap(6.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .child(drive_glyph(p, 32.0))
                .child(text("Local Disk (C:)").font(ROW_TEXT).color(p.text)),
        )
        .child(
            el().w_full()
                .h(4.0)
                .radius(2.0)
                .fill(p.field)
                .child(el().w(36.0).h_full().radius(2.0).fill(p.accent_fill)),
        )
        .child(
            text("1.56 TB free of 1.81 TB")
                .font(CAPTION)
                .color(p.tertiary),
        );
    let _ = s;
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
                .child(ico(p, Ico::ChevronDown, 11.0, p.secondary))
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
                    .center()
                    .child(ico(p, Ico::Monitor, 96.0, p.accent_fill)),
            )
            .child(el().w_full().h(p.hair).fill(p.line))
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
                            .h(76.0)
                            .radius(4.0)
                            .border(p.hair, p.line)
                            .px(12.0)
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
        let e = s.entries.get(s.selected.unwrap_or(0));
        let name = e.map(|x| x.name.as_str()).unwrap_or("");
        let mut details = col()
            .w_full()
            .px(24.0)
            .gap(12.0)
            .child(text("Details").font(TILE_TEXT).color(p.text));
        for (k, v) in [
            (
                "Type",
                e.map(|x| x.type_label.as_str()).unwrap_or("PNG File"),
            ),
            ("Size", "131 KB"),
            ("File location", "Pictures / Screenshots"),
            ("Date modified", "11/6/2026 10:39 AM"),
        ] {
            details = details.child(
                row()
                    .w_full()
                    .items_center()
                    .child(
                        col()
                            .w(110.0)
                            .child(text(k).font(CAPTION).color(p.tertiary)),
                    )
                    .child(text(v).font(CAPTION).color(p.secondary).truncate()),
            );
        }
        let thumb = match e {
            Some(entry) => col()
                .w_full()
                .h(210.0)
                .fill(p.field)
                .center()
                .child(file_glyph(entry, p, 64.0)),
            None => col().w_full().h(210.0).fill(p.field),
        };
        col()
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .fill(p.content)
            .child(thumb)
            .child(el().w_full().h(p.hair).fill(p.line))
            .child(
                col()
                    .w_full()
                    .px(24.0)
                    .pt(20.0)
                    .gap(14.0)
                    .child(text(name).font(TILE_TEXT).color(p.text))
                    .child(
                        row()
                            .h(30.0)
                            .px(10.0)
                            .radius(4.0)
                            .border(p.hair, p.line)
                            .gap(8.0)
                            .items_center()
                            .child(ico(p, Ico::ShareSmall, 14.0, p.accent))
                            .child(text("Share").font(CAPTION).color(p.text)),
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
        .child(
            text(s.selection_detail.as_str())
                .font(CAPTION)
                .color(p.secondary),
        )
        .child(el().grow(1.0))
        .child(ico(p, Ico::View, 14.0, p.tertiary))
        .child(ico(p, Ico::Details, 14.0, p.tertiary))
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
        ..AppSpec::new("Lyre", (1076.0, 600.0), init, view)
    };
    let probe = init();
    eprintln!(
        "lyre-rune: {} entries in {} in {:.2} ms",
        probe.entries.len(),
        probe.root.display(),
        probe.scan_ms
    );
    rune::run(spec);
}
