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
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use rune::color::hex;
use rune::{
    Align, AppSpec, Color, Cursor, Cx, El, GrowthSpec, Image, ImageCache, Key, KeyEvent, LineCap,
    Live, Phase, Pixels, ScrollSpec, Shape, Span, Spring, SurfaceFit, TextEdit, TextStyle, Tx,
    WindowOptions, WindowSpec, canvas, col, el, row, run_windows, text, text_input,
};

mod defaultmanager;
mod fileops;
mod frame;
mod imagery;
mod search;
mod shell;
mod updates;
mod volumes;

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
/// The column-divider grab zone: a hairline centred in a wider drag target.
const COL_SEP_W: f32 = 8.0;
const MENU_W: f32 = 248.0;
/// The More and context menus carry longer labels ("Stop opening folders with
/// Lyre", shell verbs) than the command-bar flyouts, so they get extra room.
const MENU_W_WIDE: f32 = 288.0;
const MENU_ROW_H: f32 = 30.0;

/// Every context-menu item the customize dialog can turn off, by stable id.
const CUSTOM_ITEMS: &[(&str, &str)] = &[
    ("open", "Open"),
    ("open-tab", "Open in new tab"),
    ("open-with", "Open with…"),
    ("favorite", "Add to favorites"),
    ("pin", "Pin to sidebar"),
    ("copy-path", "Copy path"),
    ("rename", "Rename"),
    ("cut", "Cut"),
    ("copy", "Copy"),
    ("delete", "Delete"),
    ("properties", "Properties"),
    ("new-folder", "New folder"),
    ("paste", "Paste"),
    ("refresh", "Refresh"),
    ("select-all", "Select all"),
];

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
    menu: Color,
    menu_line: Color,
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
            menu: c("#2b2b2b"),
            menu_line: c("#454545"),
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
            menu: c("#ffffff"),
            menu_line: c("#e5e5e5"),
            hair,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Folder,
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
    /// The raw modified time, for sorting and grouping a photo library.
    modified_at: Option<std::time::SystemTime>,
    type_label: String,
    kind: Kind,
    /// The hidden attribute (a dotfile on Unix); `system` marks protected OS
    /// files on Windows, which stay hidden unless both toggles allow them.
    hidden: bool,
    system: bool,
    /// Fixtures carry display-ready size strings; real scans carry byte counts.
    fixture: bool,
    fixture_size: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Home,
    Files,
    ThisPc,
    /// The Pictures library as a date-grouped photo grid.
    Gallery,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Details,
    List,
    Tiles,
    Grid,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Folders,
    Documents,
    Images,
    Video,
    Audio,
    Archives,
    Code,
}

impl Filter {
    fn label(self) -> &'static str {
        match self {
            Filter::All => "All items",
            Filter::Folders => "Folders",
            Filter::Documents => "Documents",
            Filter::Images => "Images",
            Filter::Video => "Video",
            Filter::Audio => "Audio",
            Filter::Archives => "Compressed archives",
            Filter::Code => "Code and programs",
        }
    }

    fn matches(self, e: &Entry) -> bool {
        match self {
            Filter::All => true,
            Filter::Folders => e.is_dir,
            Filter::Documents => matches!(
                e.kind,
                Kind::Pdf | Kind::Document | Kind::Sheet | Kind::Slides | Kind::Text
            ),
            Filter::Images => e.kind == Kind::Image,
            Filter::Video => e.kind == Kind::Video,
            Filter::Audio => e.kind == Kind::Audio,
            Filter::Archives => e.kind == Kind::Archive,
            Filter::Code => matches!(e.kind, Kind::Code | Kind::App),
        }
    }
}

/// A command-bar flyout, anchored below its trigger in window points.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DropMenu {
    New,
    Sort,
    View,
    Filter,
    More,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CtxScope {
    Item,
    Background,
    Place,
    Favorite,
    Pinned,
    Volume,
}

/// The one open menu: a command-bar flyout or a right-click context menu.
#[derive(Clone)]
enum Menu {
    Drop { kind: DropMenu, x: f32, y: f32 },
    Context { scope: CtxScope, target: usize, x: f32, y: f32 },
}

#[derive(Clone, Copy)]
enum Fixture {
    Home,
    Downloads,
    Screenshots,
    ThisPc,
}

/// The file operation running in the background, as the status bar shows it.
#[derive(Clone)]
struct Transfer {
    label: &'static str,
    name: String,
    done: usize,
    total: usize,
    done_bytes: u64,
    total_bytes: u64,
    stopping: bool,
}

/// Messages from the operation worker thread, drained in `poll`.
enum OpMsg {
    Progress(fileops::Progress),
    Done(Result<usize, String>),
}

/// Messages from the update worker thread, drained in `poll`.
enum UpdateMsg {
    Checked(Result<Option<updates::Offer>, String>),
    Progress(u64, Option<u64>),
    Installing,
    Done(Result<(), String>),
}

/// A modal dialog: delete confirmation, paste conflict, rename, new folder, default manager, update.
enum Dialog {
    Delete { permanent: bool, paths: Vec<PathBuf> },
    PasteConflict { mode: fileops::Mode, sources: Vec<PathBuf>, target: PathBuf },
    Rename { path: PathBuf, edit: TextEdit },
    NewFolder { parent: PathBuf, edit: TextEdit },
    DefaultManager,
    Update,
}

/// A rubber-band selection in the rows' content coordinates. `base` is the
/// selection kept when the drag adds to it with Ctrl/Cmd held, and `auto_scroll`
/// is the direction the band is pushing the viewport (-1 up, 1 down, 0 none).
#[derive(Clone)]
struct Marquee {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    base: Vec<usize>,
    auto_scroll: i8,
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
    selected: Vec<usize>,
    focused: Option<usize>,
    /// The entry a Shift-click or Shift-arrow range extends from.
    anchor: Option<usize>,
    /// A row briefly flashed after keyboard navigation, and when it was lit.
    reveal: Option<usize>,
    reveal_at: Option<std::time::Instant>,
    /// The rubber-band selection drag, in the rows' own content coordinates.
    marquee: Option<Marquee>,
    /// Details column widths (Name, Date, Type, Size) and the nav rail width.
    col_w: [f32; 4],
    rail_w: f32,
    nav: usize,
    count: usize,
    selection_detail: String,
    notice: String,
    search_edit: TextEdit,
    history: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    sort: u8,
    sort_desc: bool,
    group_by_date: bool,
    filter: Filter,
    compact: bool,
    show_hidden: bool,
    /// Show hidden+system (protected OS) files; meaningful on Windows only.
    show_protected: bool,
    /// Show file extensions in names; off hides the suffix for files.
    show_extensions: bool,
    nav_open: bool,
    /// The single open flyout or context menu, if any.
    menu: Option<Menu>,
    /// Favorite paths (Home's Favorites tab) and pinned folder paths (the rail).
    favorites: Vec<String>,
    pinned: Vec<String>,
    /// Files opened in Lyre, most recent first, rebuilt into `entries` on the
    /// Home view so the rows behave like any other list.
    recent: Vec<String>,
    /// Context-menu item ids the person turned off in the customize dialog.
    menu_hidden: Vec<String>,
    /// Default-off shell verbs the person turned on in the customize dialog.
    menu_shown: Vec<String>,
    /// Built-in rail places (3..=8) the person unpinned from the sidebar.
    hidden_places: Vec<usize>,
    customize_open: bool,
    /// The target the customize dialog lists shell verbs for.
    customize_path: Option<String>,
    customize_is_dir: bool,
    customize_background: bool,
    /// Signature of the persisted state; poll writes to disk when it changes.
    prefs_sig: String,
    /// Real shell icons keyed by type/path, and decoded image previews by path.
    icon_cache: RefCell<HashMap<String, Pixels>>,
    thumb_cache: RefCell<HashMap<String, Image>>,
    /// Cut/copy clipboard, the running transfer, its worker channel and cancel.
    clipboard: Option<(Vec<PathBuf>, fileops::Mode)>,
    transfer: Option<Transfer>,
    op_rx: Option<std::sync::mpsc::Receiver<OpMsg>>,
    op_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// The open modal dialog (delete confirm, paste conflict, rename, new folder).
    dialog: Option<Dialog>,
    /// Live search: results stream from a worker into these, replacing the list.
    searching: bool,
    search_hits: Vec<search::Hit>,
    search_matched: usize,
    search_scanned: usize,
    search_elapsed: u64,
    search_truncated: bool,
    search_busy: bool,
    search_query: String,
    search_rx: Option<std::sync::mpsc::Receiver<search::Event>>,
    search_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Auto-update: the worker channel, the offered build, and download progress.
    update_rx: Option<std::sync::mpsc::Receiver<UpdateMsg>>,
    update_offer: Option<updates::Offer>,
    update_busy: bool,
    update_progress: Option<(u64, Option<u64>)>,
    update_installing: bool,
    /// Whether to check for updates at launch; persisted so the user can opt out.
    update_check_startup: bool,
    /// True once the first-frame startup check has been considered.
    startup_check_done: bool,
    /// Whether the running check stays quiet (startup) or reports (manual).
    update_silent: bool,
    /// Whether to offer to become the default file manager at launch; persisted.
    prompt_default_manager: bool,
    /// Trigger and window rects in window points, read to anchor and clamp menus.
    rect_new: Live<[f32; 4]>,
    rect_sort: Live<[f32; 4]>,
    rect_view: Live<[f32; 4]>,
    rect_filter: Live<[f32; 4]>,
    rect_more: Live<[f32; 4]>,
    rect_window: Live<[f32; 4]>,
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
    /// The scroll viewport's drawn rect and offset, for the rubber band: a
    /// drag's local point is the viewport's origin less the scroll offset.
    viewport_rect: Live<[f32; 4]>,
    /// Each marquee-able item's drawn rect in window points, recorded by its
    /// `on_layout`. The index is the visible row, matching `visible`.
    marquee_rects: RefCell<Vec<[f32; 4]>>,
    /// This PC's volumes, and which card is selected. Refreshed on entry and
    /// every few seconds, so a stick plugged in appears on its own.
    volumes: Vec<volumes::Volume>,
    volumes_selected: Option<usize>,
    volumes_at: Instant,
}

pub(crate) fn format_size(n: u64) -> String {
    if n < 1024 {
        format!("{n} bytes")
    } else if n < 1024 * 1024 {
        format!("{:.0} KB", n as f64 / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else if n < 1024 * 1024 * 1024 * 1024 {
        format!("{:.2} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if n < 1024 * 1024 * 1024 * 1024 * 1024 {
        format!("{:.2} TB", n as f64 / (1024.0 * 1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.2} PB", n as f64 / (1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0))
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
            let Ok(meta) = item.metadata() else { continue };
            let is_dir = meta.is_dir();
            let modified_at = meta.modified().ok();
            let modified = modified_at
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| format_time(d.as_secs()))
                .unwrap_or_default();
            let kind = classify(&name, is_dir);
            let (hidden, system) = {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    let attrs = meta.file_attributes();
                    (attrs & 0x2 != 0, attrs & 0x4 != 0)
                }
                #[cfg(not(windows))]
                {
                    (name.starts_with('.'), false)
                }
            };
            entries.push(Entry {
                name: name.clone(),
                path: item.path().to_string_lossy().into_owned(),
                is_dir,
                size: if is_dir { 0 } else { meta.len() },
                modified,
                modified_at,
                type_label: kind_label(kind).to_string(),
                kind,
                hidden,
                system,
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
        modified_at: None,
        type_label: type_label.to_string(),
        kind,
        hidden: false,
        system: false,
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

/// Details/List row height: Compact view tightens the rows Explorer-style.
fn row_h(s: &App) -> f32 {
    if s.compact { 22.0 } else { ROW_H }
}

fn mode_to_u8(m: Mode) -> u8 {
    match m {
        Mode::Details => 0,
        Mode::List => 1,
        Mode::Tiles => 2,
        Mode::Grid => 3,
    }
}

fn u8_to_mode(v: u8) -> Mode {
    match v {
        1 => Mode::List,
        2 => Mode::Tiles,
        3 => Mode::Grid,
        _ => Mode::Details,
    }
}

fn filter_to_u8(f: Filter) -> u8 {
    match f {
        Filter::All => 0,
        Filter::Folders => 1,
        Filter::Documents => 2,
        Filter::Images => 3,
        Filter::Video => 4,
        Filter::Audio => 5,
        Filter::Archives => 6,
        Filter::Code => 7,
    }
}

fn u8_to_filter(v: u8) -> Filter {
    match v {
        1 => Filter::Folders,
        2 => Filter::Documents,
        3 => Filter::Images,
        4 => Filter::Video,
        5 => Filter::Audio,
        6 => Filter::Archives,
        7 => Filter::Code,
        _ => Filter::All,
    }
}

/// The display name of a favorite or pinned path: its last component.
fn path_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.to_string())
}

const PREFS_ID: &str = "lyre-rune";

/// Restore the view state and collections saved by a previous run.
fn load_prefs(s: &mut App) {
    let prefs = rune::prefs::Prefs::open(PREFS_ID);
    if let Some((sort, sort_desc, preview, mode, compact, filter, group, nav, hidden, dark)) =
        prefs.get::<(u8, bool, bool, u8, bool, u8, bool, bool, bool, bool)>("settings")
    {
        s.sort = sort.min(3);
        s.sort_desc = sort_desc;
        s.preview = preview;
        s.mode = u8_to_mode(mode);
        s.compact = compact;
        s.filter = u8_to_filter(filter);
        s.group_by_date = group;
        s.nav_open = nav;
        s.show_hidden = hidden;
        s.dark = dark;
    }
    if let Some(f) = prefs.get::<Vec<String>>("favorites") {
        s.favorites = f;
    }
    if let Some(r) = prefs.get::<Vec<String>>("recent") {
        s.recent = r;
    }
    if let Some(p) = prefs.get::<Vec<String>>("pinned") {
        s.pinned = p;
    }
    if let Some(m) = prefs.get::<Vec<String>>("menu_hidden") {
        s.menu_hidden = m;
    }
    if let Some(m) = prefs.get::<Vec<String>>("menu_shown") {
        s.menu_shown = m;
    }
    if let Some(h) = prefs.get::<Vec<usize>>("hidden_places") {
        s.hidden_places = h;
    }
    if let Some(v) = prefs.get::<bool>("update_check_startup") {
        s.update_check_startup = v;
    }
    if let Some(v) = prefs.get::<bool>("prompt_default_manager") {
        s.prompt_default_manager = v;
    }
    if let Some(v) = prefs.get::<bool>("show_protected") {
        s.show_protected = v;
    }
    if let Some(v) = prefs.get::<bool>("show_extensions") {
        s.show_extensions = v;
    }
    if let Some(w) = prefs.get::<[f32; 4]>("col_w") {
        s.col_w = w;
    }
    if let Some(w) = prefs.get::<f32>("rail_w") {
        s.rail_w = w.clamp(120.0, 400.0);
    }
}

/// Write the whole view state and collections; poll calls this once per change.
fn persist(s: &App) {
    let mut prefs = rune::prefs::Prefs::open(PREFS_ID);
    let settings = (
        s.sort,
        s.sort_desc,
        s.preview,
        mode_to_u8(s.mode),
        s.compact,
        filter_to_u8(s.filter),
        s.group_by_date,
        s.nav_open,
        s.show_hidden,
        s.dark,
    );
    let _ = prefs.set("settings", &settings);
    let _ = prefs.set("favorites", &s.favorites);
    let _ = prefs.set("recent", &s.recent);
    let _ = prefs.set("pinned", &s.pinned);
    let _ = prefs.set("menu_hidden", &s.menu_hidden);
    let _ = prefs.set("menu_shown", &s.menu_shown);
    let _ = prefs.set("hidden_places", &s.hidden_places);
    let _ = prefs.set("update_check_startup", &s.update_check_startup);
    let _ = prefs.set("prompt_default_manager", &s.prompt_default_manager);
    let _ = prefs.set("show_protected", &s.show_protected);
    let _ = prefs.set("show_extensions", &s.show_extensions);
    let _ = prefs.set("col_w", &s.col_w);
    let _ = prefs.set("rail_w", &s.rail_w);
}

/// A cheap fingerprint of everything persisted, so poll saves only on change.
fn prefs_signature(s: &App) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{}|{:?}|{}",
        s.sort,
        s.sort_desc,
        s.preview,
        mode_to_u8(s.mode),
        s.compact,
        filter_to_u8(s.filter),
        s.group_by_date,
        s.nav_open,
        s.show_hidden,
        s.dark,
        s.favorites,
        s.recent,
        s.pinned,
        s.menu_hidden,
        s.menu_shown,
        s.hidden_places,
        s.update_check_startup,
        s.prompt_default_manager,
        s.show_protected,
        s.show_extensions,
        s.col_w,
        s.rail_w
    )
}

fn is_favorite(s: &App, path: &str) -> bool {
    s.favorites.iter().any(|p| p == path)
}

fn is_pinned(s: &App, path: &str) -> bool {
    s.pinned.iter().any(|p| p == path)
}

fn toggle_favorite(s: &mut App, path: &str) {
    if let Some(i) = s.favorites.iter().position(|p| p == path) {
        s.favorites.remove(i);
    } else if !path.is_empty() {
        s.favorites.push(path.to_string());
    }
}

/// Remember a file the user opened, most recent first, capped so the list
/// cannot grow without bound.
fn record_recent(s: &mut App, path: &str) {
    if path.is_empty() {
        return;
    }
    s.recent.retain(|p| p != path);
    s.recent.insert(0, path.to_string());
    s.recent.truncate(50);
}

/// Rebuild the Recent rows from the remembered paths, dropping any that have
/// since disappeared.
fn refresh_recent(s: &mut App) {
    let mut entries = Vec::new();
    for path in s.recent.clone() {
        let Ok(meta) = fs::metadata(&path) else { continue };
        if meta.is_dir() {
            continue;
        }
        let name = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let kind = classify(&name, false);
        let modified_at = meta.modified().ok();
        let modified = modified_at
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| format_time(d.as_secs()))
            .unwrap_or_default();
        entries.push(Entry {
            name,
            path,
            is_dir: false,
            size: meta.len(),
            modified,
            modified_at,
            type_label: kind_label(kind).to_string(),
            kind,
            hidden: false,
            system: false,
            fixture: false,
            fixture_size: String::new(),
        });
    }
    s.count = entries.len();
    s.entries = entries;
    s.visible = (0..s.entries.len()).collect();
}

fn toggle_pin(s: &mut App, path: &str) {
    if let Some(i) = s.pinned.iter().position(|p| p == path) {
        s.pinned.remove(i);
    } else if !path.is_empty() {
        s.pinned.push(path.to_string());
    }
}

/// Whether a context-menu item survives the person's customize choices. Lyre's
/// own items start on; another program's verbs start off and are turned on one
/// at a time, so installing software never silently changes a right-click.
fn item_shown(s: &App, id: &str, default_on: bool) -> bool {
    if s.menu_hidden.iter().any(|x| x == id) {
        false
    } else if s.menu_shown.iter().any(|x| x == id) {
        true
    } else {
        default_on
    }
}

fn menu_visible(s: &App, id: &str) -> bool {
    item_shown(s, id, true)
}

/// Toggle a customize choice: default-on items go into menu_hidden, default-off
/// shell verbs into menu_shown.
fn toggle_menu_item(s: &mut App, id: &str, default_on: bool) {
    let shown = item_shown(s, id, default_on);
    if default_on {
        if shown {
            s.menu_hidden.push(id.to_string());
        } else {
            s.menu_hidden.retain(|x| x != id);
        }
    } else if shown {
        s.menu_shown.retain(|x| x != id);
    } else {
        s.menu_shown.push(id.to_string());
    }
}

/// Whether an entry passes the hidden/protected toggles. Protected OS files
/// (hidden and system, a Windows notion) need both switches; ordinary hidden
/// items need only the hidden one.
fn entry_visible(s: &App, e: &Entry) -> bool {
    if e.hidden && e.system {
        s.show_hidden && s.show_protected
    } else if e.hidden {
        s.show_hidden
    } else {
        true
    }
}

/// The list stays virtualised: this only holds indices, never built rows.
fn refresh_visible(s: &mut App) {
    let q = s.search_edit.text.to_lowercase();
    let mut order: Vec<usize> = (0..s.entries.len())
        .filter(|&i| {
            let e = &s.entries[i];
            (q.is_empty() || e.name.to_lowercase().contains(&q))
                && entry_visible(s, e)
                && s.filter.matches(e)
        })
        .collect();
    if !s.keep_order {
        // Folders always lead, whatever the column or direction; the key orders
        // entries within the folder and file groups.
        order.sort_by(|&a, &b| {
            let ea = &s.entries[a];
            let eb = &s.entries[b];
            let base = match s.sort {
                1 => ea.modified.cmp(&eb.modified),
                2 => ea.type_label.cmp(&eb.type_label),
                3 => ea.size.cmp(&eb.size),
                _ => ea.name.to_lowercase().cmp(&eb.name.to_lowercase()),
            };
            eb.is_dir
                .cmp(&ea.is_dir)
                .then_with(|| if s.sort_desc { base.reverse() } else { base })
        });
    }
    // Grouping: fixtures keep their authored blocks; a real folder groups by the
    // day each entry was modified when asked, sorting by date so days stay whole.
    if s.keep_order {
        // authored groups already set by the fixture
    } else if s.group_by_date && s.view == View::Files {
        order.sort_by(|&a, &b| s.entries[b].modified.cmp(&s.entries[a].modified));
        order.sort_by(|&a, &b| s.entries[b].is_dir.cmp(&s.entries[a].is_dir));
        let mut groups: Vec<(usize, String)> = Vec::new();
        let mut last = String::new();
        for (i, &ei) in order.iter().enumerate() {
            let day = s.entries[ei]
                .modified
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if day != last {
                groups.push((i, day.clone()));
                last = day;
            }
        }
        s.groups = groups;
    } else {
        s.groups.clear();
    }
    s.count = s.count_override.unwrap_or(order.len());
    s.visible = order;
}

/// Rune calls this before each frame. Search edits land in `TextEdit` through
/// `on_edit`, which only binds the field, so the row order is recomputed here.
fn poll(s: &mut App, _t: f64, tx: &mut Tx) -> bool {
    let sig = prefs_signature(s);
    if sig != s.prefs_sig {
        persist(s);
        s.prefs_sig = sig;
    }
    // Once at launch, unless this is a headless scene render: check quietly for
    // a newer build, and offer to take over folder opening until the person
    // accepts or declines for good. The window is already up; the check is
    // off-thread and the offer only appears where it is actually possible.
    if !s.startup_check_done {
        s.startup_check_done = true;
        let headless = std::env::args().any(|a| a == "--snap" || a == "--scene");
        if !headless {
            if s.update_check_startup {
                start_update_check(s, true);
            }
            if s.prompt_default_manager {
                let ms = defaultmanager::state();
                if !ms.is_default && ms.unsupported.is_none() {
                    s.dialog = Some(Dialog::DefaultManager);
                }
            }
        }
    }
    let mut changed = drain_op(s);
    // Keep frames coming while a transfer runs so its progress drains promptly.
    if s.transfer.is_some() {
        tx.wake_after(std::time::Duration::from_millis(60));
    }
    changed |= drain_search(s);
    if s.search_busy {
        tx.wake_after(std::time::Duration::from_millis(40));
    }
    changed |= drain_update(s);
    if s.update_busy {
        tx.wake_after(std::time::Duration::from_millis(60));
    }
    // The keyboard-navigation flash fades after a moment; keep frames coming
    // until it clears so the redraw lands.
    if let Some(at) = s.reveal_at {
        if at.elapsed() >= std::time::Duration::from_millis(600) {
            s.reveal = None;
            s.reveal_at = None;
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_millis(50));
        }
    }
    // A rubber band held near the viewport's edge keeps scrolling the list and
    // extends itself, so one drag can cross a long folder. The band's local y
    // moves with the scroll: the pointer's window point stays put.
    let auto_scroll = s.marquee.as_ref().map_or(0, |m| m.auto_scroll);
    if auto_scroll != 0 {
        let max = s.scroll_max.get();
        let off = s.scroll_off.get();
        let next = (off + auto_scroll as f32 * 28.0).clamp(0.0, max);
        if next != off {
            tx.scroll_to_instant("viewport", 0.0, next);
            if let Some(m) = s.marquee.as_mut() {
                m.y1 += next - off;
            }
            apply_marquee_at(s, next);
            changed = true;
        }
        tx.wake_after(std::time::Duration::from_millis(16));
    }
    // This PC notices a stick plugged in while the window is open.
    if s.view == View::ThisPc && s.volumes_at.elapsed() >= std::time::Duration::from_secs(3) {
        refresh_volumes(s);
        changed = true;
    }
    if s.search_edit.text != s.search_query {
        let q = s.search_edit.text.clone();
        if q.trim().is_empty() {
            clear_search(s);
            refresh_visible(s);
        } else {
            start_search(s, q);
        }
        changed = true;
    }
    changed
}

fn set_location(s: &mut App, path: PathBuf) {
    clear_search(s);
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
    s.contextual = false;
    s.groups.clear();
    clear_selection(s);
    s.notice.clear();
    s.menu = None;
    s.keep_order = false;
    s.count_override = None;
    refresh_visible(s);
}

fn go(s: &mut App, path: PathBuf) {
    s.menu = None;
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
    s.menu = None;
    match index {
        0 => load(s, Fixture::Home),
        9 => {
            load(s, Fixture::ThisPc);
            refresh_volumes(s);
        }
        10 => s.notice = "Network browsing is not implemented in this slice".into(),
        1 => go_gallery(s),
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

/// Read the attached volumes again, keeping the selected card when it is still
/// there so a refresh does not drop the selection.
fn refresh_volumes(s: &mut App) {
    let selected = s.volumes_selected.and_then(|i| s.volumes.get(i)).map(|v| v.path.clone());
    s.volumes = volumes::enumerate();
    s.volumes_at = Instant::now();
    s.volumes_selected = selected.and_then(|path| s.volumes.iter().position(|v| v.path == path));
    if s.view == View::ThisPc {
        s.count = s.volumes.len();
        s.count_override = Some(s.volumes.len());
    }
}

fn load(s: &mut App, f: Fixture) {
    s.groups.clear();
    s.preview = false;
    s.contextual = false;
    s.mode = Mode::Details;
    clear_selection(s);
    s.notice.clear();
    s.search_edit.text.clear();
    match f {
        Fixture::Home => {
            s.view = View::Home;
            s.tab = "Home".into();
            s.crumbs = vec!["Home".into()];
            s.search = "Search Home".into();
            s.nav = 0;
            s.count = 0;
            s.selection_detail.clear();
            refresh_recent(s);
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
            s.selected = vec![1];
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
            s.entries.clear();
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
        .ok()
        .or_else(|| defaultmanager::startup_path().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
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
        selected: Vec::new(),
        focused: None,
        anchor: None,
        reveal: None,
        reveal_at: None,
        marquee: None,
        col_w: [NAME_W, DATE_W, TYPE_W, SIZE_W],
        rail_w: RAIL_W,
        nav: 9,
        count,
        selection_detail: String::new(),
        notice: String::new(),
        search_edit: TextEdit::default(),
        history: Vec::new(),
        forward: Vec::new(),
        sort: 0,
        sort_desc: false,
        group_by_date: false,
        filter: Filter::All,
        compact: false,
        show_hidden: false,
        show_protected: false,
        show_extensions: true,
        nav_open: true,
        menu: None,
        favorites: Vec::new(),
        recent: Vec::new(),
        pinned: Vec::new(),
        menu_hidden: Vec::new(),
        menu_shown: Vec::new(),
        hidden_places: Vec::new(),
        customize_open: false,
        customize_path: None,
        customize_is_dir: false,
        customize_background: false,
        prefs_sig: String::new(),
        icon_cache: RefCell::new(HashMap::new()),
        thumb_cache: RefCell::new(HashMap::new()),
        clipboard: None,
        transfer: None,
        op_rx: None,
        op_cancel: None,
        dialog: None,
        searching: false,
        search_hits: Vec::new(),
        search_matched: 0,
        search_scanned: 0,
        search_elapsed: 0,
        search_truncated: false,
        search_busy: false,
        search_query: String::new(),
        search_rx: None,
        search_cancel: None,
        update_rx: None,
        update_offer: None,
        update_busy: false,
        update_progress: None,
        update_installing: false,
        update_check_startup: true,
        startup_check_done: false,
        update_silent: false,
        prompt_default_manager: true,
        rect_new: Live::new([0.0; 4]),
        rect_sort: Live::new([0.0; 4]),
        rect_view: Live::new([0.0; 4]),
        rect_filter: Live::new([0.0; 4]),
        rect_more: Live::new([0.0; 4]),
        rect_window: Live::new([0.0; 4]),
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
        viewport_rect: Live::new([0.0; 4]),
        marquee_rects: RefCell::new(Vec::new()),
        volumes: Vec::new(),
        volumes_selected: None,
        volumes_at: Instant::now(),
    };
    load_prefs(&mut app);
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
    Tiles,
    Check,
    Open,
    Eject,
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
        Ico::Tiles => Shape::new()
            .fill(vec![[4.0, 5.0], [9.0, 5.0], [9.0, 10.0], [4.0, 10.0]], ink)
            .line([11.0, 6.0], [20.0, 6.0], W, LineCap::Round, ink)
            .line([11.0, 9.0], [17.0, 9.0], W, LineCap::Round, ink)
            .fill(vec![[4.0, 14.0], [9.0, 14.0], [9.0, 19.0], [4.0, 19.0]], ink)
            .line([11.0, 15.0], [20.0, 15.0], W, LineCap::Round, ink)
            .line([11.0, 18.0], [17.0, 18.0], W, LineCap::Round, ink),
        Ico::Check => stroke(vec![[5.0, 12.0], [10.0, 17.0], [19.0, 7.0]], ink),
        Ico::Open => stroke(
            vec![[4.0, 11.0], [4.0, 20.0], [20.0, 20.0], [20.0, 11.0]],
            ink,
        )
        .line([12.0, 15.0], [12.0, 4.0], W, LineCap::Round, ink)
        .stroke(vec![[8.0, 8.0], [12.0, 4.0], [16.0, 8.0]], W, ink),
        Ico::Eject => Shape::new()
            .fill(vec![[12.0, 4.5], [19.5, 15.0], [4.5, 15.0]], ink)
            .line([6.0, 19.0], [18.0, 19.0], W, LineCap::Round, ink),
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

// ----------------------------------------------------- icons & previews ----

/// Rune's cross-platform image decoder, shared app-wide. A decoded image's Pixels
/// self-wake the windows showing it, so a no-op waker is enough to stream previews.
fn image_cache() -> &'static ImageCache {
    static CACHE: std::sync::OnceLock<ImageCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| ImageCache::new(std::sync::Arc::new(|| {})))
}

/// Folders share one icon; executables/icon files key by path (their own icon);
/// everything else keys by extension (its default-app/type icon).
fn icon_key(e: &Entry) -> String {
    if e.is_dir {
        return "folder".into();
    }
    let ext = Path::new(&e.path)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "exe" | "ico" | "lnk" | "scr" | "dll" | "icl" | "cpl") {
        format!("file:{}", e.path)
    } else if ext.is_empty() {
        "file:none".into()
    } else {
        format!("ext:{ext}")
    }
}

/// The entry's real shell icon at `size`, falling back to the drawn glyph when the
/// platform supplies none (or the path is only a fixture).
fn entry_icon(s: &App, e: &Entry, p: Paint, size: f32) -> El {
    let key = icon_key(e);
    let pixels = {
        let mut cache = s.icon_cache.borrow_mut();
        if let Some(px) = cache.get(&key) {
            px.clone()
        } else {
            let px = Pixels::new();
            if let Some((w, h, rgba)) = imagery::icon_rgba(&e.path, e.is_dir, (size.ceil() as u32).max(32)) {
                px.set(w, h, rgba);
            }
            cache.insert(key, px.clone());
            px
        }
    };
    if pixels.size() == (0, 0) {
        return file_glyph(e, p, size);
    }
    el().size(size, size).pixels(&pixels)
}

/// Whether an entry is a raster image worth decoding into a preview.
fn previewable(e: &Entry) -> bool {
    if e.is_dir || e.kind != Kind::Image {
        return false;
    }
    let ext = Path::new(&e.path)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "avif" | "heic")
}

/// A decoded preview of an image file, cached by path and size. The handle is live:
/// it draws nothing until the decoder lands, then wakes the window showing it.
fn entry_thumb(s: &App, e: &Entry, px: u32) -> Image {
    let key = format!("{}:{px}", e.path);
    let mut cache = s.thumb_cache.borrow_mut();
    if let Some(img) = cache.get(&key) {
        return img.clone();
    }
    let img = image_cache().file(&e.path, px as f32, 1.0);
    cache.insert(key, img.clone());
    img
}

/// An image preview rect: the decoded picture fills it once ready, over a quiet
/// placeholder so the tile is never a hole while the decoder works.
fn thumb_rect(s: &App, e: &Entry, p: Paint, w: f32, h: f32, px: u32) -> El {
    let img = entry_thumb(s, e, px);
    el().size(w, h)
        .radius(3.0)
        .fill(p.thumb_bg)
        .child(el().absolute(0.0, 0.0).size(w, h).center().child(file_glyph(e, p, h * 0.5)))
        .child(el().absolute(0.0, 0.0).size(w, h).radius(3.0).pixels_fit(img.pixels(), SurfaceFit::Fill))
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
        .drawn_rect_into(&s.rect_window)
        .on_press(|s: &mut App| s.menu = None)
        .child(tab_strip(s, p))
        .child(address_row(s, p))
        .child(command_bar(s, p))
        .child(body(s, p))
        .child(status_bar(s, p))
        .child(scroll_thumb(s, p))
        .child(menu_overlay(s, p))
        .child(customize_dialog(s, p))
        .child(dialog_overlay(s, p))
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
                    0 => frame::minimize(),
                    1 => s.zoom = frame::toggle_maximize(),
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
    let has_sel = !s.selected.is_empty();
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
                .key("cmd-new")
                .drawn_rect_into(&s.rect_new)
                .help("New item")
                .on_press(|s: &mut App| {
                    let r = s.rect_new.get();
                    open_drop(s, DropMenu::New, r);
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
            match which {
                Ico::Cut => cut_selection(s),
                Ico::Copy => copy_selection(s),
                Ico::Paste => begin_paste(s),
                Ico::Rename => begin_rename(s),
                Ico::Delete => begin_delete(s, false),
                _ => s.notice = "Sharing arrives with the share sheet".into(),
            }
        }));
    }
    bar = bar
        .child(el().w(p.hair).h(20.0).fill(p.line).mx(6.0))
        .child(
            text_button(p, Ico::Sort, "Sort", true, false)
                .key("cmd-sort")
                .drawn_rect_into(&s.rect_sort)
                .help("Sort")
                .on_press(|s: &mut App| {
                    let r = s.rect_sort.get();
                    open_drop(s, DropMenu::Sort, r);
                }),
        )
        .child(
            text_button(p, Ico::View, "View", true, false)
                .key("cmd-view")
                .drawn_rect_into(&s.rect_view)
                .help("View")
                .on_press(|s: &mut App| {
                    let r = s.rect_view.get();
                    open_drop(s, DropMenu::View, r);
                }),
        );
    bar = bar.child(
        text_button(p, Ico::Filter, "Filter", true, false)
            .key("cmd-filter")
            .drawn_rect_into(&s.rect_filter)
            .help("Filter by type")
            .on_press(|s: &mut App| {
                let r = s.rect_filter.get();
                open_drop(s, DropMenu::Filter, r);
            }),
    );
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
                .key("cmd-more")
                .drawn_rect_into(&s.rect_more)
                .on_press(|s: &mut App| {
                    let r = s.rect_more.get();
                    open_drop(s, DropMenu::More, r);
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

// ----------------------------------------------------------------- menus ----

/// One row of a flyout or context menu. `leading` is the check/icon column; the
/// caller attaches the press action. Disabled rows draw dimmed and inert.
fn menu_item(p: Paint, label: &str, leading: Option<Ico>, hint: Option<&str>, disabled: bool) -> El {
    let ink = if disabled { p.disabled } else { p.text };
    let mark = match leading {
        Some(w) => ico(p, w, 15.0, if disabled { p.disabled } else { p.secondary }),
        None => el().size(15.0, 15.0),
    };
    let mut r = row()
        .w_full()
        .h(MENU_ROW_H)
        .px(8.0)
        .gap(8.0)
        .radius(4.0)
        .fill(Color::TRANSPARENT)
        .items_center()
        .child(el().w(18.0).center().child(mark))
        .child(text(label).font(UI_TEXT).color(ink).grow(1.0).min_w(0.0).truncate());
    if let Some(h) = hint {
        r = r.child(text(h).font(CAPTION).color(p.tertiary));
    }
    if !disabled {
        r = r.hover(p.hover).press(p.press);
    }
    r
}

fn menu_sep(p: Paint) -> El {
    el().w_full().h(p.hair).fill(p.menu_line).my(4.0)
}

fn menu_heading(p: Paint, label: &str) -> El {
    row()
        .w_full()
        .h(24.0)
        .px(12.0)
        .items_center()
        .child(text(label).font(CAPTION).color(p.tertiary))
}

/// The other-program verbs the person has turned on, as menu rows credited to
/// their source. Empty (and no separator) when none are enabled.
fn shell_verb_rows(
    s: &App,
    p: Paint,
    path: Option<&str>,
    is_dir: bool,
    background: bool,
) -> (Vec<El>, f32) {
    let mut rows = Vec::new();
    let mut h = 0.0;
    let enabled: Vec<shell::Verb> = shell::verbs(path, is_dir, background)
        .into_iter()
        .filter(|v| item_shown(s, &v.id, false))
        .collect();
    if enabled.is_empty() {
        return (rows, h);
    }
    rows.push(menu_sep(p));
    h += 9.0;
    for v in enabled {
        let verb = v.clone();
        let target = path.unwrap_or("").to_string();
        rows.push(
            menu_item(p, &v.label, None, Some(v.source.as_str()), false).on_press(move |s: &mut App| {
                if let Err(e) = shell::run(&verb, &target) {
                    s.notice = e;
                }
                s.menu = None;
            }),
        );
        h += MENU_ROW_H;
    }
    (rows, h)
}

/// Anchor a flyout just below its trigger, from the trigger's drawn rect.
/// Pressing the same trigger again closes it, as a menu button should.
fn open_drop(s: &mut App, kind: DropMenu, r: [f32; 4]) {
    if matches!(&s.menu, Some(Menu::Drop { kind: open, .. }) if *open == kind) {
        s.menu = None;
        return;
    }
    s.menu = Some(Menu::Drop { kind, x: r[0], y: r[1] + r[3] + 2.0 });
}

/// The open menu as a window-level overlay: a full-window scrim that closes it
/// on any outside press, and the panel itself, clamped to the window edges.
fn menu_overlay(s: &App, p: Paint) -> El {
    let Some(menu) = s.menu.clone() else {
        return el().key("menu-none");
    };
    let win = s.rect_window.get();
    let (ww, wh) = (win[2], win[3]);
    let mut rows: Vec<El> = Vec::new();
    let mut h = 8.0;
    let (ax, ay) = match &menu {
        Menu::Drop { x, y, .. } | Menu::Context { x, y, .. } => (*x, *y),
    };

    match &menu {
        Menu::Drop { kind, .. } => {
            match kind {
                DropMenu::New => {
                    rows.push(
                        menu_item(p, "Folder", Some(Ico::New), None, false)
                            .on_press(|s: &mut App| begin_new_folder(s)),
                    );
                    rows.push(menu_item(p, "Shortcut", Some(Ico::New), None, true));
                    h += MENU_ROW_H * 2.0;
                }
                DropMenu::Sort => {
                    for (k, label) in [(0u8, "Name"), (1, "Date modified"), (2, "Type"), (3, "Size")] {
                        let leading = (s.sort == k).then_some(Ico::Check);
                        rows.push(
                            menu_item(p, label, leading, None, false).on_press(move |s: &mut App| {
                                s.sort = k;
                                s.keep_order = false;
                                refresh_visible(s);
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    rows.push(menu_sep(p));
                    h += 9.0;
                    for (desc, label) in [(false, "Ascending"), (true, "Descending")] {
                        let leading = (s.sort_desc == desc).then_some(Ico::Check);
                        rows.push(
                            menu_item(p, label, leading, None, false).on_press(move |s: &mut App| {
                                s.sort_desc = desc;
                                s.keep_order = false;
                                refresh_visible(s);
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    rows.push(menu_sep(p));
                    h += 9.0;
                    let g = s.group_by_date;
                    rows.push(
                        menu_item(p, "Group by date", g.then_some(Ico::Check), None, false).on_press(
                            move |s: &mut App| {
                                s.group_by_date = !g;
                                refresh_visible(s);
                                s.menu = None;
                            },
                        ),
                    );
                    h += MENU_ROW_H;
                }
                DropMenu::View => {
                    for (m, label, icon) in [
                        (Mode::Details, "Details", Ico::Details),
                        (Mode::List, "List", Ico::List),
                        (Mode::Tiles, "Tiles", Ico::Tiles),
                        (Mode::Grid, "Large icons", Ico::Grid),
                    ] {
                        let leading = if s.mode == m { Some(Ico::Check) } else { Some(icon) };
                        rows.push(
                            menu_item(p, label, leading, None, false)
                                .on_press(move |s: &mut App| {
                                    s.mode = m;
                                    s.menu = None;
                                }),
                        );
                        h += MENU_ROW_H;
                    }
                    rows.push(menu_sep(p));
                    h += 9.0;
                    let c = s.compact;
                    rows.push(
                        menu_item(p, "Compact view", c.then_some(Ico::Check), None, false).on_press(
                            move |s: &mut App| {
                                s.compact = !c;
                                s.menu = None;
                            },
                        ),
                    );
                    h += MENU_ROW_H;
                    rows.push(menu_sep(p));
                    h += 9.0;
                    let sh = s.show_hidden;
                    rows.push(
                        menu_item(p, "Show hidden items", sh.then_some(Ico::Check), None, false)
                            .on_press(move |s: &mut App| {
                                s.show_hidden = !sh;
                                refresh_visible(s);
                                s.menu = None;
                            }),
                    );
                    h += MENU_ROW_H;
                    let se = s.show_extensions;
                    rows.push(
                        menu_item(p, "File name extensions", se.then_some(Ico::Check), None, false)
                            .on_press(move |s: &mut App| {
                                s.show_extensions = !se;
                                s.menu = None;
                            }),
                    );
                    h += MENU_ROW_H;
                    if cfg!(target_os = "windows") {
                        let sp = s.show_protected;
                        rows.push(
                            menu_item(p, "Show protected OS files", sp.then_some(Ico::Check), None, false)
                                .on_press(move |s: &mut App| {
                                    s.show_protected = !sp;
                                    refresh_visible(s);
                                    s.menu = None;
                                }),
                        );
                        h += MENU_ROW_H;
                    }
                    let pr = s.preview;
                    rows.push(
                        menu_item(p, "Preview pane", pr.then_some(Ico::Check), None, false).on_press(
                            move |s: &mut App| {
                                s.preview = !pr;
                                s.menu = None;
                            },
                        ),
                    );
                    h += MENU_ROW_H;
                    let nv = s.nav_open;
                    rows.push(
                        menu_item(p, "Navigation pane", nv.then_some(Ico::Check), None, false)
                            .on_press(move |s: &mut App| {
                                s.nav_open = !nv;
                                s.menu = None;
                            }),
                    );
                    h += MENU_ROW_H;
                }
                DropMenu::Filter => {
                    for f in [
                        Filter::All,
                        Filter::Folders,
                        Filter::Documents,
                        Filter::Images,
                        Filter::Video,
                        Filter::Audio,
                        Filter::Archives,
                        Filter::Code,
                    ] {
                        let leading = (s.filter == f).then_some(Ico::Check);
                        rows.push(
                            menu_item(p, f.label(), leading, None, false).on_press(move |s: &mut App| {
                                s.filter = f;
                                refresh_visible(s);
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                }
                DropMenu::More => {
                    rows.push(
                        menu_item(p, "Refresh", Some(Ico::Refresh), Some("F5"), false).on_press(
                            |s: &mut App| {
                                if s.view == View::Files {
                                    let r = s.root.clone();
                                    set_location(s, r);
                                } else {
                                    refresh_visible(s);
                                }
                                s.menu = None;
                            },
                        ),
                    );
                    h += MENU_ROW_H;
                    rows.push(
                        menu_item(p, "Select all", None, Some("Ctrl+A"), false).on_press(
                            |s: &mut App| {
                                s.notice = "Select all arrives with multi-selection".into();
                                s.menu = None;
                            },
                        ),
                    );
                    h += MENU_ROW_H;
                    let path = s.selected.first().map(|&i| s.entries[i].path.clone()).unwrap_or_default();
                    let has_sel = !s.selected.is_empty();
                    let copy = path.clone();
                    rows.push(
                        menu_item(p, "Copy path", None, None, !has_sel).on_press_tx(
                            move |s: &mut App, tx: &mut Tx| {
                                if !copy.is_empty() {
                                    tx.copy(copy.clone());
                                    s.notice = "Path copied to clipboard".into();
                                }
                                s.menu = None;
                            },
                        ),
                    );
                    h += MENU_ROW_H;
                    rows.push(
                        menu_item(p, "Restore sidebar folders", Some(Ico::Pin), None, s.hidden_places.is_empty())
                            .on_press(|s: &mut App| {
                                s.hidden_places.clear();
                                s.menu = None;
                            }),
                    );
                    h += MENU_ROW_H;
                    rows.push(menu_sep(p));
                    h += 9.0;
                    let ms = defaultmanager::state();
                    if ms.unsupported.is_some() {
                        rows.push(menu_item(p, "Open folders with Lyre", Some(Ico::ThisPc), None, true));
                        h += MENU_ROW_H;
                    } else if ms.is_default {
                        rows.push(
                            menu_item(p, "Stop opening folders with Lyre", Some(Ico::ThisPc), None, false)
                                .on_press(|s: &mut App| {
                                    match defaultmanager::restore() {
                                        Ok(_) => s.notice = "Folders open with the previous file manager again".into(),
                                        Err(e) => s.notice = e,
                                    }
                                    s.menu = None;
                                }),
                        );
                        h += MENU_ROW_H;
                    } else {
                        rows.push(
                            menu_item(p, "Open folders with Lyre…", Some(Ico::ThisPc), None, false)
                                .on_press(|s: &mut App| {
                                    s.dialog = Some(Dialog::DefaultManager);
                                    s.menu = None;
                                }),
                        );
                        h += MENU_ROW_H;
                    }
                    if ms.unsupported.is_none() {
                        rows.push(
                            menu_item(p, "Ask to be default at startup", s.prompt_default_manager.then_some(Ico::Check), None, false)
                                .on_press(|s: &mut App| {
                                    s.prompt_default_manager = !s.prompt_default_manager;
                                }),
                        );
                        h += MENU_ROW_H;
                    }
                    rows.push(
                        menu_item(p, "Check for updates", Some(Ico::Refresh), None, s.update_busy)
                            .on_press(|s: &mut App| {
                                start_update_check(s, false);
                                s.menu = None;
                            }),
                    );
                    h += MENU_ROW_H;
                    rows.push(
                        menu_item(p, "Check for updates at startup", s.update_check_startup.then_some(Ico::Check), None, false)
                            .on_press(|s: &mut App| {
                                s.update_check_startup = !s.update_check_startup;
                            }),
                    );
                    h += MENU_ROW_H;
                    rows.push(menu_sep(p));
                    h += 9.0;
                    rows.push(menu_heading(p, "Theme"));
                    h += 24.0;
                    let dark = s.dark;
                    rows.push(
                        menu_item(p, "Light", (!dark).then_some(Ico::Check), None, false).on_press(
                            move |s: &mut App| {
                                s.dark = false;
                                s.menu = None;
                            },
                        ),
                    );
                    h += MENU_ROW_H;
                    rows.push(
                        menu_item(p, "Dark", dark.then_some(Ico::Check), None, false).on_press(
                            move |s: &mut App| {
                                s.dark = true;
                                s.menu = None;
                            },
                        ),
                    );
                    h += MENU_ROW_H;
                }
            }
        }
        Menu::Context { scope, target, .. } => {
            let target = *target;
            match scope {
                CtxScope::Item => {
                    let entry = if s.searching {
                        s.search_hits.get(target).map(|h| &h.entry)
                    } else {
                        s.entries.get(target)
                    };
                    let is_dir = entry.map(|e| e.is_dir).unwrap_or(false);
                    let path = entry.map(|e| e.path.clone()).unwrap_or_default();
                    if menu_visible(s, "open") {
                        let open = path.clone();
                        let open_label = if is_dir { "Open" } else { "Open with default app" };
                        rows.push(
                            menu_item(p, open_label, Some(Ico::Open), Some("Enter"), false).on_press(move |s: &mut App| {
                                if is_dir {
                                    go(s, PathBuf::from(&open));
                                } else if let Err(e) = shell::open_default(&open) {
                                    s.notice = e;
                                }
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    if is_dir {
                        if menu_visible(s, "open-tab") {
                            rows.push(menu_item(p, "Open in new tab", None, None, true));
                            h += MENU_ROW_H;
                        }
                    } else if menu_visible(s, "open-with") {
                        let open = path.clone();
                        rows.push(
                            menu_item(p, "Open with…", None, None, false).on_press(move |s: &mut App| {
                                if let Err(e) = shell::open_with_picker(&open) {
                                    s.notice = e;
                                }
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    rows.push(menu_sep(p));
                    h += 9.0;
                    let fav = is_favorite(s, &path);
                    let pin = is_pinned(s, &path);
                    if menu_visible(s, "favorite") {
                        let fp = path.clone();
                        rows.push(
                            menu_item(p, if fav { "Remove from favorites" } else { "Add to favorites" }, Some(Ico::Star), None, false)
                                .on_press(move |s: &mut App| {
                                    toggle_favorite(s, &fp);
                                    s.menu = None;
                                }),
                        );
                        h += MENU_ROW_H;
                    }
                    if menu_visible(s, "pin") {
                        let pp = path.clone();
                        rows.push(
                            menu_item(p, if pin { "Remove from sidebar" } else { "Pin to sidebar" }, Some(Ico::Pin), None, false)
                                .on_press(move |s: &mut App| {
                                    toggle_pin(s, &pp);
                                    s.menu = None;
                                }),
                        );
                        h += MENU_ROW_H;
                    }
                    if menu_visible(s, "copy-path") {
                        let copy = path.clone();
                        rows.push(
                            menu_item(p, "Copy path", None, None, false).on_press_tx(move |s: &mut App, tx: &mut Tx| {
                                tx.copy(copy.clone());
                                s.notice = "Path copied to clipboard".into();
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    for (id, label, icon) in [
                        ("rename", "Rename", Ico::Rename),
                        ("cut", "Cut", Ico::Cut),
                        ("copy", "Copy", Ico::Copy),
                        ("delete", "Delete", Ico::Delete),
                    ] {
                        if menu_visible(s, id) {
                            rows.push(
                                menu_item(p, label, Some(icon), None, false).on_press(move |s: &mut App| {
                                    match id {
                                        "rename" => begin_rename(s),
                                        "cut" => {
                                            cut_selection(s);
                                            s.menu = None;
                                        }
                                        "copy" => {
                                            copy_selection(s);
                                            s.menu = None;
                                        }
                                        "delete" => begin_delete(s, false),
                                        _ => {}
                                    }
                                }),
                            );
                            h += MENU_ROW_H;
                        }
                    }
                    let (shell_rows, shell_h) = shell_verb_rows(s, p, Some(&path), is_dir, false);
                    rows.extend(shell_rows);
                    h += shell_h;
                    rows.push(menu_sep(p));
                    h += 9.0;
                    if menu_visible(s, "properties") {
                        rows.push(
                            menu_item(p, "Properties", Some(Ico::Info), None, false).on_press(|s: &mut App| {
                                s.preview = true;
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    let ctx_path = path.clone();
                    rows.push(
                        menu_item(p, "Customize context menu…", Some(Ico::More), None, false).on_press(move |s: &mut App| {
                            s.customize_open = true;
                            s.customize_path = Some(ctx_path.clone());
                            s.customize_is_dir = is_dir;
                            s.customize_background = false;
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                }
                CtxScope::Background => {
                    if menu_visible(s, "new-folder") {
                        rows.push(
                            menu_item(p, "New folder", Some(Ico::New), None, false)
                                .on_press(|s: &mut App| begin_new_folder(s)),
                        );
                        h += MENU_ROW_H;
                    }
                    if menu_visible(s, "paste") {
                        rows.push(
                            menu_item(p, "Paste", Some(Ico::Paste), None, s.clipboard.is_none())
                                .on_press(|s: &mut App| begin_paste(s)),
                        );
                        h += MENU_ROW_H;
                    }
                    rows.push(menu_sep(p));
                    h += 9.0;
                    rows.push(menu_heading(p, "Sort by"));
                    h += 24.0;
                    for (k, label) in [(0u8, "Name"), (1, "Date modified"), (2, "Type"), (3, "Size")] {
                        let leading = (s.sort == k).then_some(Ico::Check);
                        rows.push(
                            menu_item(p, label, leading, None, false).on_press(move |s: &mut App| {
                                s.sort = k;
                                s.keep_order = false;
                                refresh_visible(s);
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    rows.push(menu_sep(p));
                    h += 9.0;
                    rows.push(menu_heading(p, "View"));
                    h += 24.0;
                    for (m, label) in [(Mode::Details, "Details"), (Mode::Grid, "Large icons")] {
                        let leading = (s.mode == m).then_some(Ico::Check);
                        rows.push(
                            menu_item(p, label, leading, None, false).on_press(move |s: &mut App| {
                                s.mode = m;
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    rows.push(menu_sep(p));
                    h += 9.0;
                    if menu_visible(s, "refresh") {
                        rows.push(
                            menu_item(p, "Refresh", Some(Ico::Refresh), Some("F5"), false).on_press(|s: &mut App| {
                                if s.view == View::Files {
                                    let r = s.root.clone();
                                    set_location(s, r);
                                } else {
                                    refresh_visible(s);
                                }
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    if menu_visible(s, "select-all") {
                        rows.push(
                            menu_item(p, "Select all", None, Some("Ctrl+A"), false).on_press(|s: &mut App| {
                                s.notice = "Select all arrives with multi-selection".into();
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
                    let root = s.root.to_string_lossy().into_owned();
                    let (shell_rows, shell_h) = shell_verb_rows(s, p, Some(&root), true, true);
                    rows.extend(shell_rows);
                    h += shell_h;
                    let ctx_root = root.clone();
                    rows.push(
                        menu_item(p, "Customize context menu…", Some(Ico::More), None, false).on_press(move |s: &mut App| {
                            s.customize_open = true;
                            s.customize_path = Some(ctx_root.clone());
                            s.customize_is_dir = true;
                            s.customize_background = true;
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                }
                CtxScope::Place => {
                    rows.push(
                        menu_item(p, "Open", Some(Ico::Open), None, false).on_press(move |s: &mut App| {
                            navigate(s, target);
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                    rows.push(menu_sep(p));
                    h += 9.0;
                    let path = place_path(target).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    let has_path = !path.is_empty();
                    let fav = is_favorite(s, &path);
                    let hideable = (3..=8).contains(&target);
                    let place_hidden = s.hidden_places.contains(&target);
                    if menu_visible(s, "favorite") {
                        let fp = path.clone();
                        rows.push(
                            menu_item(p, if fav { "Remove from favorites" } else { "Add to favorites" }, Some(Ico::Star), None, !has_path)
                                .on_press(move |s: &mut App| {
                                    toggle_favorite(s, &fp);
                                    s.menu = None;
                                }),
                        );
                        h += MENU_ROW_H;
                    }
                    if menu_visible(s, "pin") && hideable {
                        rows.push(
                            menu_item(p, if place_hidden { "Pin to sidebar" } else { "Remove from sidebar" }, Some(Ico::Pin), None, false)
                                .on_press(move |s: &mut App| {
                                    if let Some(i) = s.hidden_places.iter().position(|&x| x == target) {
                                        s.hidden_places.remove(i);
                                    } else {
                                        s.hidden_places.push(target);
                                    }
                                    s.menu = None;
                                }),
                        );
                        h += MENU_ROW_H;
                    }
                    let copy = path.clone();
                    rows.push(
                        menu_item(p, "Copy path", None, None, !has_path).on_press_tx(move |s: &mut App, tx: &mut Tx| {
                            if !copy.is_empty() {
                                tx.copy(copy.clone());
                                s.notice = "Path copied to clipboard".into();
                            }
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                }
                CtxScope::Favorite => {
                    let path = s.favorites.get(target).cloned().unwrap_or_default();
                    let is_dir = Path::new(&path).is_dir();
                    let open = path.clone();
                    rows.push(
                        menu_item(p, "Open", Some(Ico::Open), None, false).on_press(move |s: &mut App| {
                            if is_dir {
                                go(s, PathBuf::from(&open));
                            }
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                    let fp = path.clone();
                    rows.push(
                        menu_item(p, "Remove from favorites", Some(Ico::Star), None, false).on_press(move |s: &mut App| {
                            toggle_favorite(s, &fp);
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                    let copy = path.clone();
                    rows.push(
                        menu_item(p, "Copy path", None, None, false).on_press_tx(move |s: &mut App, tx: &mut Tx| {
                            tx.copy(copy.clone());
                            s.notice = "Path copied to clipboard".into();
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                }
                CtxScope::Pinned => {
                    let path = s.pinned.get(target).cloned().unwrap_or_default();
                    let open = path.clone();
                    rows.push(
                        menu_item(p, "Open", Some(Ico::Open), None, false).on_press(move |s: &mut App| {
                            go(s, PathBuf::from(&open));
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                    let pp = path.clone();
                    rows.push(
                        menu_item(p, "Remove from sidebar", Some(Ico::Pin), None, false).on_press(move |s: &mut App| {
                            toggle_pin(s, &pp);
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                    let copy = path.clone();
                    rows.push(
                        menu_item(p, "Copy path", None, None, false).on_press_tx(move |s: &mut App, tx: &mut Tx| {
                            tx.copy(copy.clone());
                            s.notice = "Path copied to clipboard".into();
                            s.menu = None;
                        }),
                    );
                    h += MENU_ROW_H;
                }
                CtxScope::Volume => {
                    if let Some(volume) = s.volumes.get(target).cloned() {
                        rows.push(
                            menu_item(p, "Open", Some(Ico::Open), None, false).on_press({
                                let path = volume.path.clone();
                                move |s: &mut App| {
                                    go(s, path.clone());
                                    s.menu = None;
                                }
                            }),
                        );
                        h += MENU_ROW_H;
                        let copy = volume.path.to_string_lossy().into_owned();
                        rows.push(
                            menu_item(p, "Copy path", None, None, false).on_press_tx(
                                move |s: &mut App, tx: &mut Tx| {
                                    tx.copy(copy.clone());
                                    s.notice = "Path copied to clipboard".into();
                                    s.menu = None;
                                },
                            ),
                        );
                        h += MENU_ROW_H;
                        if volume.is_ejectable() {
                            let title = volume.title();
                            let path = volume.path.clone();
                            rows.push(
                                menu_item(p, "Eject", Some(Ico::Eject), None, false).on_press(
                                    move |s: &mut App| match volumes::eject(&path) {
                                        Ok(()) => s.notice = format!("{title} ejected"),
                                        Err(e) => s.notice = e,
                                    },
                                ),
                            );
                            h += MENU_ROW_H;
                        }
                    }
                }
            }
        }
    }

    let mw = match &menu {
        Menu::Context { .. } | Menu::Drop { kind: DropMenu::More, .. } => MENU_W_WIDE,
        _ => MENU_W,
    };
    let x = ax.clamp(8.0, (ww - mw - 8.0).max(8.0));
    let y = ay.clamp(8.0, (wh - h - 8.0).max(8.0));
    // The panel swallows a press on its own padding so a near miss neither
    // picks an item nor dismisses. Outside it, the body scrim (see `content`)
    // closes the menu and keeps the press from the rows behind.
    col()
        .key("menu-panel")
        .absolute(x, y)
        .w(mw)
        .py(4.0)
        .px(4.0)
        .fill(p.menu)
        .border(p.hair, p.menu_line)
        .radius(8.0)
        .shadow(6.0, 18.0, Color::BLACK.alpha(0.45))
        .on_press(|_s: &mut App| {})
        .children(rows)
}

/// One checkbox row of the customize dialog; `default_on` decides which list a
/// toggle writes to (Lyre items start on, other apps' verbs start off).
fn customize_row(
    p: Paint,
    label: &str,
    source: Option<&str>,
    shown: bool,
    id: String,
    default_on: bool,
) -> El {
    let check = if shown {
        ico(p, Ico::Check, 12.0, Color::WHITE)
    } else {
        el().size(12.0, 12.0)
    };
    let mut text_row = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .items_center()
        .child(text(label).font(UI_TEXT).color(p.text).grow(1.0).min_w(0.0).truncate());
    if let Some(src) = source {
        text_row = text_row.child(text(src).font(CAPTION).color(p.tertiary));
    }
    row()
        .w_full()
        .h(32.0)
        .px(8.0)
        .gap(10.0)
        .radius(4.0)
        .fill(Color::TRANSPARENT)
        .hover(p.hover)
        .items_center()
        .child(
            el().size(18.0, 18.0)
                .radius(4.0)
                .border(p.hair, p.menu_line)
                .fill(if shown { p.accent_fill } else { Color::TRANSPARENT })
                .center()
                .child(check),
        )
        .child(text_row)
        .on_press(move |s: &mut App| toggle_menu_item(s, &id, default_on))
}

/// A modal dialog toggling which context-menu items appear; choices persist.
fn customize_dialog(s: &App, p: Paint) -> El {
    if !s.customize_open {
        return el().key("customize-none");
    }
    let mut list = col().key("customize-list").w_full().gap(2.0);
    for (id, label) in CUSTOM_ITEMS {
        list = list.child(customize_row(p, label, None, item_shown(s, id, true), id.to_string(), true));
    }
    let verbs = shell::verbs(s.customize_path.as_deref(), s.customize_is_dir, s.customize_background);
    if !verbs.is_empty() {
        list = list.child(menu_heading(p, "Other apps"));
        for v in verbs {
            list = list.child(customize_row(
                p,
                &v.label,
                Some(&v.source),
                item_shown(s, &v.id, false),
                v.id.clone(),
                false,
            ));
        }
    }
    el().key("customize-overlay")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK.alpha(0.45))
        .on_press(|s: &mut App| s.customize_open = false)
        .center()
        .child(
            col()
                .key("customize-panel")
                .w(380.0)
                .px(16.0)
                .py(16.0)
                .gap(12.0)
                .fill(p.menu)
                .border(p.hair, p.menu_line)
                .radius(8.0)
                .shadow(6.0, 24.0, Color::BLACK.alpha(0.5))
                .child(text("Customize context menu").font(TILE_TEXT).color(p.text))
                .child(text("Choose which items appear in right-click menus.").font(CAPTION).color(p.tertiary))
                .child(
                    col()
                        .key("customize-scroll")
                        .w_full()
                        .h(380.0)
                        .scroll(ScrollSpec { stick_bottom: false })
                        .child(list),
                )
                .child(
                    row().w_full().items_center()
                        .child(el().grow(1.0))
                        .child(
                            row()
                                .key("customize-done")
                                .h(30.0)
                                .px(18.0)
                                .radius(4.0)
                                .fill(p.accent_fill)
                                .hover(p.hover)
                                .center()
                                .child(text("Done").font(UI_TEXT).color(Color::WHITE))
                                .on_press(|s: &mut App| s.customize_open = false),
                        ),
                ),
        )
}

fn dialog_button(p: Paint, label: &str, primary: bool) -> El {
    row()
        .h(32.0)
        .px(16.0)
        .radius(4.0)
        .fill(if primary { p.accent_fill } else { p.field })
        .border(p.hair, if primary { p.accent_fill } else { p.field_line })
        .hover(p.hover)
        .center()
        .child(text(label).font(UI_TEXT).color(if primary { Color::WHITE } else { p.text }))
}

/// The modal dialog: delete confirmation, paste conflict, rename or new folder.
fn dialog_overlay(s: &App, p: Paint) -> El {
    let Some(dialog) = &s.dialog else {
        return el().key("dialog-none");
    };
    let base = || {
        col()
            .key("dialog-panel")
            .w(420.0)
            .px(20.0)
            .py(18.0)
            .gap(14.0)
            .fill(p.menu)
            .border(p.hair, p.menu_line)
            .radius(8.0)
            .shadow(6.0, 24.0, Color::BLACK.alpha(0.5))
    };
    let panel = match dialog {
        Dialog::Delete { permanent, paths } => {
            let n = paths.len();
            let plural = if n == 1 { "" } else { "s" };
            let (title, msg) = if *permanent {
                ("Permanently delete?", format!("Permanently delete {n} item{plural}? This cannot be undone."))
            } else {
                ("Move to trash?", format!("Move {n} item{plural} to the trash? You can restore it later."))
            };
            let paths = paths.clone();
            let mode = if *permanent { fileops::DeleteMode::Permanent } else { fileops::DeleteMode::Trash };
            let confirm_label = if *permanent { "Delete permanently" } else { "Delete" };
            base()
                .child(text(title).font(TILE_TEXT).color(p.text))
                .child(text(msg.as_str()).font(UI_TEXT).color(p.secondary))
                .child(
                    row().w_full().gap(8.0).child(el().grow(1.0))
                        .child(dialog_button(p, "Cancel", false).on_press(|s: &mut App| s.dialog = None))
                        .child(dialog_button(p, confirm_label, true).on_press(move |s: &mut App| {
                            s.dialog = None;
                            start_delete(s, mode, paths.clone());
                        })),
                )
        }
        Dialog::PasteConflict { mode, sources, target } => {
            let mode = *mode;
            let rename_sources = sources.clone();
            let rename_target = target.clone();
            let over_sources = sources.clone();
            let over_target = target.clone();
            base()
                .child(text("Replace or keep both?").font(TILE_TEXT).color(p.text))
                .child(text("An item with the same name already exists in this folder.").font(UI_TEXT).color(p.secondary))
                .child(
                    row().w_full().gap(8.0).child(el().grow(1.0))
                        .child(dialog_button(p, "Cancel", false).on_press(|s: &mut App| s.dialog = None))
                        .child(dialog_button(p, "Keep both", false).on_press(move |s: &mut App| {
                            s.dialog = None;
                            start_paste(s, mode, fileops::Conflict::Rename, rename_sources.clone(), rename_target.clone());
                        }))
                        .child(dialog_button(p, "Overwrite", true).on_press(move |s: &mut App| {
                            s.dialog = None;
                            start_paste(s, mode, fileops::Conflict::Overwrite, over_sources.clone(), over_target.clone());
                        })),
                )
        }
        Dialog::Rename { edit, .. } => {
            base()
                .child(text("Rename").font(TILE_TEXT).color(p.text))
                .child(
                    row()
                        .w_full()
                        .h(FIELD_H)
                        .radius(4.0)
                        .fill(p.field)
                        .border(p.hair, p.field_line)
                        .px(10.0)
                        .items_center()
                        .child(
                            text_input(edit, UI_TEXT)
                                .key("dialog-edit")
                                .grow(1.0)
                                .min_w(0.0)
                                .single_line()
                                .color(p.text)
                                .on_edit(|s: &mut App| match &mut s.dialog {
                                    Some(Dialog::Rename { edit, .. }) => edit,
                                    _ => unreachable!(),
                                })
                                .on_submit(|s: &mut App, _tx: &mut Tx| commit_rename(s)),
                        ),
                )
                .child(
                    row().w_full().gap(8.0).child(el().grow(1.0))
                        .child(dialog_button(p, "Cancel", false).on_press(|s: &mut App| s.dialog = None))
                        .child(dialog_button(p, "OK", true).on_press(|s: &mut App| commit_rename(s))),
                )
        }
        Dialog::NewFolder { edit, .. } => {
            base()
                .child(text("New folder").font(TILE_TEXT).color(p.text))
                .child(
                    row()
                        .w_full()
                        .h(FIELD_H)
                        .radius(4.0)
                        .fill(p.field)
                        .border(p.hair, p.field_line)
                        .px(10.0)
                        .items_center()
                        .child(
                            text_input(edit, UI_TEXT)
                                .key("dialog-edit")
                                .grow(1.0)
                                .min_w(0.0)
                                .single_line()
                                .color(p.text)
                                .on_edit(|s: &mut App| match &mut s.dialog {
                                    Some(Dialog::NewFolder { edit, .. }) => edit,
                                    _ => unreachable!(),
                                })
                                .on_submit(|s: &mut App, _tx: &mut Tx| commit_new_folder(s)),
                        ),
                )
                .child(
                    row().w_full().gap(8.0).child(el().grow(1.0))
                        .child(dialog_button(p, "Cancel", false).on_press(|s: &mut App| s.dialog = None))
                        .child(dialog_button(p, "Create", true).on_press(|s: &mut App| commit_new_folder(s))),
                )
        }
        Dialog::DefaultManager => {
            base()
                .child(text("Open folders with Lyre?").font(TILE_TEXT).color(p.text))
                .child(
                    text("Lyre can open folders instead of your current file manager. This changes a few per-user settings and can be undone at any time from the More menu.")
                        .font(UI_TEXT)
                        .color(p.secondary)
                        .w_full()
                        .wrap(),
                )
                .child(
                    row().w_full().gap(8.0).items_center()
                        .child(
                            text("Don't ask again")
                                .font(CAPTION)
                                .color(p.tertiary)
                                .on_press(|s: &mut App| {
                                    s.prompt_default_manager = false;
                                    s.dialog = None;
                                }),
                        )
                        .child(el().grow(1.0))
                        .child(dialog_button(p, "Cancel", false).on_press(|s: &mut App| s.dialog = None))
                        .child(dialog_button(p, "Make default", true).on_press(|s: &mut App| {
                            match defaultmanager::make_default() {
                                Ok(_) => s.notice = "Lyre now opens folders".into(),
                                Err(e) => s.notice = e,
                            }
                            s.dialog = None;
                        })),
                )
        }
        Dialog::Update => {
            let offer = s.update_offer.clone();
            let busy = s.update_busy;
            let installing = s.update_installing;
            let progress = s.update_progress;
            let mut panel = base().child(text("Update available").font(TILE_TEXT).color(p.text));
            if let Some(o) = &offer {
                panel = panel.child(text(format!("Version {}  →  {}", o.current, o.version)).font(UI_TEXT).color(p.secondary));
                if let Some(notes) = &o.notes {
                    if !notes.is_empty() {
                        panel = panel.child(text(notes.as_str()).font(CAPTION).color(p.tertiary).w_full().max_lines(6));
                    }
                }
            }
            if busy {
                let frac = match progress {
                    Some((done, Some(total))) if total > 0 => (done as f64 / total as f64).min(1.0) as f32,
                    _ => 0.0,
                };
                let label = if installing {
                    "Installing…".to_string()
                } else {
                    match progress {
                        Some((_, Some(total))) if total > 0 => format!("{}%", (frac * 100.0) as u32),
                        Some((done, _)) => format!("{:.1} MB", done as f64 / 1_048_576.0),
                        None => "Downloading…".to_string(),
                    }
                };
                panel = panel.child(
                    row().w_full().gap(8.0).items_center()
                        .child(el().w(200.0).h(6.0).radius(3.0).fill(p.line).child(el().w_frac(frac).h_full().radius(3.0).fill(p.accent_fill)))
                        .child(text(label.as_str()).font(CAPTION).color(p.secondary)),
                );
            }
            let mut footer = row().w_full().gap(8.0).items_center();
            if !busy {
                footer = footer.child(
                    text("Don't check at startup")
                        .font(CAPTION)
                        .color(p.tertiary)
                        .on_press(|s: &mut App| {
                            s.update_check_startup = false;
                            s.dialog = None;
                        }),
                );
            }
            panel.child(
                footer
                    .child(el().grow(1.0))
                    .child(dialog_button(p, "Cancel", false).on_press(move |s: &mut App| {
                        if !busy {
                            s.dialog = None;
                        }
                    }))
                    .child(dialog_button(p, if installing { "Installing…" } else { "Install" }, true).on_press(move |s: &mut App| {
                        if !busy {
                            start_update_install(s);
                        }
                    })),
            )
        }
    };
    el().key("dialog-overlay")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK.alpha(0.45))
        .on_press(|s: &mut App| s.dialog = None)
        .center()
        .child(panel)
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
    .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
        s.menu = Some(Menu::Context { scope: CtxScope::Place, target: index, x: at.x, y: at.y });
    })
}

/// A pinned folder in the rail: opens its path, right-click offers unpin.
fn pinned_item(s: &App, p: Paint, pi: usize, path: &str) -> El {
    let active = s.view == View::Files && s.root == Path::new(path);
    let ink = if active { p.text } else { p.secondary };
    let name = path_name(path);
    let open = path.to_string();
    let mut item = row()
        .key(format!("pinned-{pi}"))
        .w_full()
        .h(28.0)
        .radius(4.0)
        .fill(if active { p.rail_sel } else { Color::TRANSPARENT })
        .hover(p.hover)
        .px(8.0)
        .gap(8.0)
        .items_center()
        .motion(Spring::NONE)
        .instant_changes()
        .child(el().w(9.0))
        .child(folder(16.0, "#ffd66b", "#f0b232", "#d89427", None, p))
        .child(text(name.as_str()).font(ROW_TEXT).color(ink).grow(1.0).min_w(0.0).truncate())
        .child(ico(p, Ico::Pin, 10.0, p.tertiary));
    if active {
        item = item.border(p.hair, p.sel_line);
    }
    item.on_press(move |s: &mut App| {
        go(s, PathBuf::from(&open));
    })
    .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
        s.menu = Some(Menu::Context { scope: CtxScope::Pinned, target: pi, x: at.x, y: at.y });
    })
}

fn nav_rail(s: &App, p: Paint) -> El {
    let mut rail = col()
        .key("rail")
        .w(s.rail_w)
        .h_full()
        .fill(p.content)
        .px(4.0)
        .pt(6.0)
        .gap(1.0)
        .motion(Spring::NONE)
        .instant_changes()
        .child(rail_item(s, p, 0, "Home", false, false))
        .child(rail_item(s, p, 1, "Gallery", false, false))
        .child(el().w_full().h(p.hair).fill(p.soft_line).my(5.0).key("rail-sep-1"));
    for (index, label) in [
        (3, "Desktop"),
        (4, "Downloads"),
        (5, "Documents"),
        (6, "Pictures"),
        (7, "Music"),
        (8, "Videos"),
    ] {
        if !s.hidden_places.contains(&index) {
            rail = rail.child(rail_item(s, p, index, label, false, true));
        }
    }
    if !s.pinned.is_empty() {
        rail = rail.child(el().w_full().h(p.hair).fill(p.soft_line).my(5.0).key("rail-sep-pinned"));
        for (pi, path) in s.pinned.iter().enumerate() {
            rail = rail.child(pinned_item(s, p, pi, path));
        }
    }
    let rail = rail.child(el().w_full().h(p.hair).fill(p.soft_line).my(5.0).key("rail-sep-2"))
        .child(rail_item(s, p, 9, "This PC", true, false))
        .child(rail_item(s, p, 10, "Network", true, false))
        .child(el().grow(1.0));
    row().key("rail-wrap")
        .h_full()
        .child(rail)
        .child(
            el().w(4.0)
                .h_full()
                .hover(p.hover)
                .on_drag(|s: &mut App, d: &mut rune::Drag, _tx: &mut Tx| match d.phase {
                    Phase::Down => d.data = s.rail_w,
                    Phase::Move => s.rail_w = (d.data + d.dx).clamp(120.0, 400.0),
                    _ => {}
                }),
        )
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

/// The Details column header with drag-resizable dividers. Each divider widens
/// the column to its left; the widths persist in `col_w`. The Size column gets
/// its own right-edge divider, as in Explorer, so all four columns resize.
fn details_header(s: &App, p: Paint) -> El {
    let labels = ["Name", "Date modified", "Type", "Size"];
    let mut h = row().key("header").w_full().h(HEADER_H).items_center();
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            h = h.child(column_divider(p, i - 1));
        }
        let t = text(*label).font(CAPTION).color(p.secondary);
        h = h.child(col().w(s.col_w[i]).pl(12.0).pr(8.0).child(if i == 3 {
            t.w_full().align(Align::End)
        } else {
            t
        }));
    }
    h = h.child(column_divider(p, 3));
    h.child(el().grow(1.0))
}

/// A column divider: a hairline centred in a wider grab zone that resizes the
/// column to its left. `idx` is that column's index.
fn column_divider(p: Paint, idx: usize) -> El {
    el()
        .w(COL_SEP_W)
        .h_full()
        .hover(p.hover)
        .cursor(Cursor::ResizeColumn)
        .center()
        .child(el().w(p.hair).h(16.0).fill(p.line))
        .on_drag(move |s: &mut App, d: &mut rune::Drag, _tx: &mut Tx| match d.phase {
            Phase::Down => d.data = s.col_w[idx],
            Phase::Move => s.col_w[idx] = (d.data + d.dx).clamp(60.0, 600.0),
            _ => {}
        })
}

/// The Details row's width: the four columns and their separators, ending at
/// the Size column's right edge. Everything to its right is list background,
/// which is what a rubber-band drag needs to start on.
fn row_width(s: &App) -> f32 {
    s.col_w.iter().sum::<f32>() + COL_SEP_W * 4.0
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
/// Whether a row's entry is part of the current selection.
fn is_selected(s: &App, ei: usize) -> bool {
    s.selected.contains(&ei)
}

/// The status-bar detail Explorer shows beside the count: one item's size, or
/// the number selected and their total.
fn update_selection_detail(s: &mut App) {
    let n = s.selected.len();
    if n == 0 {
        s.selection_detail.clear();
        return;
    }
    if n == 1 {
        let size = entry_by_index(s, s.selected[0]).size_label();
        s.selection_detail = if size.is_empty() {
            "1 item selected".into()
        } else {
            format!("1 item selected  {size}")
        };
    } else {
        let total: u64 = s.selected.iter().map(|&i| entry_by_index(s, i).size).sum();
        s.selection_detail = if total == 0 {
            format!("{n} items selected")
        } else {
            format!("{n} items selected  {}", format_size(total))
        };
    }
}

/// Plain click: select just this entry and make it the range anchor.
fn select_entry(s: &mut App, ei: usize) {
    s.menu = None;
    s.selected = vec![ei];
    s.focused = Some(ei);
    s.anchor = Some(ei);
    s.notice.clear();
    update_selection_detail(s);
}

/// Ctrl/Cmd-click: toggle this entry in the selection.
fn toggle_select(s: &mut App, ei: usize) {
    s.menu = None;
    if let Some(pos) = s.selected.iter().position(|&x| x == ei) {
        s.selected.remove(pos);
    } else {
        s.selected.push(ei);
    }
    s.focused = Some(ei);
    s.anchor = Some(ei);
    s.notice.clear();
    update_selection_detail(s);
}

/// The display row an entry index occupies, if it is on screen.
fn row_of(s: &App, ei: usize) -> Option<usize> {
    if s.searching {
        (ei < s.search_hits.len()).then_some(ei)
    } else {
        s.visible.iter().position(|&x| x == ei)
    }
}

/// The entry index a display row renders.
fn ei_at_row(s: &App, row: usize) -> usize {
    if s.searching {
        row
    } else {
        s.visible.get(row).copied().unwrap_or(row)
    }
}

/// Shift-click / Shift-arrow: select the row range from the anchor to here.
fn range_select(s: &mut App, ei: usize) {
    s.menu = None;
    let anchor = s.anchor.or(s.focused).unwrap_or(ei);
    if let (Some(a), Some(b)) = (row_of(s, anchor), row_of(s, ei)) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        s.selected = (lo..=hi).map(|r| ei_at_row(s, r)).collect();
    } else {
        s.selected = vec![ei];
    }
    s.focused = Some(ei);
    s.notice.clear();
    update_selection_detail(s);
}

/// A click with the live modifier state: plain, toggle (Cmd/Ctrl) or range (Shift).
fn click_select(s: &mut App, ei: usize, mods: rune::Mods) {
    if mods.shift {
        range_select(s, ei);
    } else if mods.cmd || mods.ctrl {
        toggle_select(s, ei);
    } else {
        select_entry(s, ei);
    }
}

/// Ctrl+A: every row on screen.
fn select_all(s: &mut App) {
    s.selected = (0..display_len(s)).map(|r| ei_at_row(s, r)).collect();
    s.focused = s.selected.last().copied();
    s.anchor = s.selected.first().copied();
    update_selection_detail(s);
}

fn clear_selection(s: &mut App) {
    s.selected.clear();
    s.focused = None;
    s.anchor = None;
    s.selection_detail.clear();
}

/// The display row the focus sits on, defaulting to the first.
fn focus_row(s: &App) -> usize {
    s.focused.and_then(|ei| row_of(s, ei)).unwrap_or(0)
}

/// The element key a row is drawn under, per view, for scroll-into-view.
fn row_key(s: &App, row: usize) -> String {
    match s.mode {
        Mode::List => format!("list-{row}"),
        Mode::Grid | Mode::Tiles => format!("tile-{row}"),
        Mode::Details => format!("row-{row}"),
    }
}

/// Arrow-key navigation: move to a row, selecting plainly, extending with Shift,
/// or moving the focus alone with Ctrl/Cmd; then flash and reveal the row.
fn move_focus(s: &mut App, target: usize, mods: rune::Mods, tx: &mut Tx) {
    let len = display_len(s);
    if len == 0 {
        return;
    }
    let row = target.min(len - 1);
    let ei = ei_at_row(s, row);
    if mods.shift {
        range_select(s, ei);
    } else if mods.cmd || mods.ctrl {
        s.focused = Some(ei);
    } else {
        select_entry(s, ei);
    }
    s.reveal = Some(ei);
    s.reveal_at = Some(std::time::Instant::now());
    tx.scroll_into_view(row_key(s, row));
}

/// Record a marquee-able item's drawn rect, in window points, at its visible
/// row. `on_layout` reports `[x, y, w, h]`; the band stores the edges.
/// `tx.live_only()` keeps the report from re-running the view.
fn marquee_track(el: El, i: usize) -> El {
    el.on_layout_tx(move |s: &mut App, [x, y, w, h], tx| {
        let mut rects = s.marquee_rects.borrow_mut();
        if rects.len() <= i {
            rects.resize(i + 1, [0.0; 4]);
        }
        rects[i] = [x, y, x + w, y + h];
        tx.live_only();
    })
}

/// Select every visible row whose drawn rect the band crosses, at a given
/// scroll offset (the live one, or the offset a poll is about to scroll to).
fn apply_marquee_at(s: &mut App, off: f32) {
    let Some(m) = s.marquee.as_ref() else { return };
    let vp = s.viewport_rect.get();
    let [mx0, my0, mx1, my1] = [
        vp[0] + m.x0.min(m.x1),
        vp[1] - off + m.y0.min(m.y1),
        vp[0] + m.x0.max(m.x1),
        vp[1] - off + m.y0.max(m.y1),
    ];
    let base = m.base.clone();
    let len = display_len(s);
    let rects = s.marquee_rects.borrow();
    let mut sel = base;
    for (row, r) in rects.iter().take(len).enumerate() {
        if r[2] >= mx0 && r[0] <= mx1 && r[3] >= my0 && r[1] <= my1 {
            let ei = ei_at_row(s, row);
            if !sel.contains(&ei) {
                sel.push(ei);
            }
        }
    }
    drop(rects);
    s.selected = sel;
    update_selection_detail(s);
}

/// Select every visible row whose drawn rect the band crosses.
fn apply_marquee(s: &mut App) {
    let off = s.scroll_off.get();
    apply_marquee_at(s, off);
}

/// Which way the band should push the viewport: -1 up, 1 down, 0 none. The
/// pointer's window y is compared with the viewport's edges.
fn marquee_auto_scroll(s: &App, d: &rune::Drag) -> i8 {
    let vp = s.viewport_rect.get();
    if vp[3] <= 0.0 {
        return 0;
    }
    let off = s.scroll_off.get();
    let wy = vp[1] - off + d.y;
    let edge = 48.0;
    if wy < vp[1] + edge {
        -1
    } else if wy > vp[1] + vp[3] - edge {
        1
    } else {
        0
    }
}

/// Drive the rubber-band selection from a drag on the list's background. A
/// press that lands on an item is left to the item, so the drag only ever
/// starts on empty space; the band selects every item whose rect it crosses.
fn marquee_drag(s: &mut App, d: &mut rune::Drag, tx: &mut Tx) {
    match d.phase {
        Phase::Down => {
            let additive = tx.mods().cmd || tx.mods().ctrl;
            let base = if additive { s.selected.clone() } else { Vec::new() };
            if !additive {
                clear_selection(s);
            }
            s.marquee = Some(Marquee { x0: d.x, y0: d.y, x1: d.x, y1: d.y, base, auto_scroll: 0 });
            apply_marquee(s);
        }
        Phase::Move => {
            let auto = marquee_auto_scroll(s, d);
            if let Some(m) = s.marquee.as_mut() {
                m.x1 = d.x;
                m.y1 = d.y;
                m.auto_scroll = auto;
            }
            apply_marquee(s);
        }
        Phase::Up => {
            s.marquee = None;
        }
    }
}

/// The blue rubber-band rectangle. The drag is recorded in content coordinates;
/// subtract the scroll offset to place it in the list area's own coordinates.
fn marquee_overlay(s: &App, p: Paint) -> El {
    let Some(m) = &s.marquee else { return el().key("marquee-none") };
    let x = m.x0.min(m.x1);
    let y = m.y0.min(m.y1) - s.scroll_off.get();
    let w = (m.x0 - m.x1).abs().max(1.0);
    let h = (m.y0 - m.y1).abs().max(1.0);
    el().key("marquee")
        .absolute(x, y)
        .w(w)
        .h(h)
        .fill(p.accent.alpha(0.18))
        .border(1.0, p.accent.alpha(0.9))
}

// ---------------------------------------------------------- file operations ----

fn selected_paths(s: &App) -> Vec<PathBuf> {
    let len = if s.searching { s.search_hits.len() } else { s.entries.len() };
    s.selected
        .iter()
        .filter(|&&i| i < len)
        .map(|&i| PathBuf::from(&entry_by_index(s, i).path))
        .collect()
}

/// Whether an item is on the clipboard as a cut, so the list can ghost it. A
/// copy replaces the clipboard, which clears the cut mark automatically.
fn is_cut(s: &App, path: &str) -> bool {
    match &s.clipboard {
        Some((paths, fileops::Mode::Cut)) => paths.iter().any(|p| p.as_path() == Path::new(path)),
        _ => false,
    }
}

// --------------------------------------------------------------- search ----

/// How many rows the list shows: search results while searching, else the folder.
fn display_len(s: &App) -> usize {
    if s.searching {
        s.search_hits.len()
    } else {
        s.visible.len()
    }
}

/// The entry a row renders, and its selection index, in either display mode.
fn display_entry(s: &App, i: usize) -> (usize, &crate::Entry) {
    if s.searching {
        (i, &s.search_hits[i].entry)
    } else {
        let ei = s.visible.get(i).copied().unwrap_or(i);
        (ei, &s.entries[ei])
    }
}

fn entry_by_index(s: &App, ei: usize) -> &crate::Entry {
    if s.searching {
        &s.search_hits[ei].entry
    } else {
        &s.entries[ei]
    }
}

/// The name as shown: the extension is dropped for files when the toggle is off.
/// Folders and dotfiles (`.gitignore`) keep their whole name.
fn display_name(s: &App, e: &crate::Entry) -> String {
    if !s.show_extensions && !e.is_dir {
        if let Some(pos) = e.name.rfind('.') {
            if pos > 0 {
                return e.name[..pos].to_string();
            }
        }
    }
    e.name.clone()
}

/// The entry name as text, with the matched characters highlighted while searching.
fn name_text(s: &App, e: &crate::Entry, ei: usize, font: TextStyle, color: Color, p: Paint) -> El {
    let name = display_name(s, e);
    let t = text(name.as_str()).font(font).color(color);
    if s.searching {
        if let Some(hit) = s.search_hits.get(ei) {
            // Match offsets index the full name; the shown name is a prefix of it
            // when the extension is hidden, so keep only offsets still in view.
            let shown = name.chars().count();
            let indices: Vec<u32> = hit.indices.iter().copied().filter(|&i| (i as usize) < shown).collect();
            let spans = highlight_spans(&name, &indices, p.accent.alpha(0.35));
            if !spans.is_empty() {
                return t.spans(spans);
            }
        }
    }
    t
}

/// Turn match character offsets into byte-range background spans, grouping
/// consecutive characters into one run.
fn highlight_spans(name: &str, indices: &[u32], color: Color) -> Vec<Span> {
    let mut boundaries: Vec<usize> = name.char_indices().map(|(b, _)| b).collect();
    boundaries.push(name.len());
    let mut spans = Vec::new();
    let mut i = 0;
    while i < indices.len() {
        let start = indices[i] as usize;
        let mut end = start + 1;
        while i + 1 < indices.len() && indices[i + 1] as usize == end {
            end += 1;
            i += 1;
        }
        if end < boundaries.len() {
            spans.push(Span::new(boundaries[start], boundaries[end]).bg(color, 1.0));
        }
        i += 1;
    }
    spans
}

/// Launch the search worker for a query, cancelling any walk already running.
fn start_search(s: &mut App, query: String) {
    use std::sync::atomic::Ordering;
    if let Some(c) = &s.search_cancel {
        c.store(true, Ordering::Relaxed);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let req = search::Request {
        path: s.root.to_string_lossy().into_owned(),
        local: s.entries.clone(),
        query: query.clone(),
        recursive: true,
        show_hidden: s.show_hidden,
        show_protected: s.show_protected,
    };
    s.search_rx = Some(rx);
    s.search_cancel = Some(cancel);
    s.search_query = query;
    s.searching = true;
    s.search_busy = true;
    s.search_matched = 0;
    s.search_scanned = 0;
    s.search_elapsed = 0;
    s.search_truncated = false;
    std::thread::spawn(move || {
        search::search(&req, &worker_cancel, |ev| tx.send(ev).is_ok());
    });
}

/// Drain the search worker's events into the result list; true to redraw.
fn drain_search(s: &mut App) -> bool {
    let mut changed = false;
    let mut done = false;
    if let Some(rx) = &s.search_rx {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                search::Event::Local { items, matched, elapsed_ms } => {
                    s.search_hits = items;
                    s.search_matched = matched;
                    s.search_elapsed = elapsed_ms;
                    s.searching = true;
                    changed = true;
                }
                search::Event::Batch { items, scanned } => {
                    s.search_hits.extend(items);
                    s.search_scanned = scanned;
                    changed = true;
                }
                search::Event::Done { matched, scanned, elapsed_ms, truncated } => {
                    s.search_matched = matched;
                    s.search_scanned = scanned;
                    s.search_elapsed = elapsed_ms;
                    s.search_truncated = truncated;
                    s.search_busy = false;
                    done = true;
                    changed = true;
                }
            }
        }
    }
    if done {
        // Settle the streamed results into one ranked list: folders first, then
        // score, then the shorter name, then alphabetical.
        s.search_hits.sort_by(|a, b| {
            b.entry.is_dir.cmp(&a.entry.is_dir)
                .then_with(|| b.score.cmp(&a.score))
                .then_with(|| a.entry.name.len().cmp(&b.entry.name.len()))
                .then_with(|| a.entry.name.to_lowercase().cmp(&b.entry.name.to_lowercase()))
        });
        s.search_rx = None;
        s.search_cancel = None;
    }
    changed
}

/// Stop searching and return to the folder listing.
fn clear_search(s: &mut App) {
    if let Some(c) = &s.search_cancel {
        c.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    s.search_rx = None;
    s.search_cancel = None;
    s.searching = false;
    s.search_hits.clear();
    s.search_query.clear();
    s.search_busy = false;
    clear_selection(s);
}

// ----------------------------------------------------------- auto-update ----

/// Ask the release page whether there is a newer build, off the UI thread.
fn start_update_check(s: &mut App, silent: bool) {
    if s.update_busy {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    s.update_rx = Some(rx);
    s.update_busy = true;
    s.update_silent = silent;
    if !silent {
        s.notice = "Checking for updates…".into();
    }
    std::thread::spawn(move || {
        let _ = tx.send(UpdateMsg::Checked(updates::check()));
    });
}

/// Download and install the offered build, reporting progress off the UI thread.
fn start_update_install(s: &mut App) {
    let Some(offer) = s.update_offer.clone() else { return };
    if s.update_busy {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    s.update_rx = Some(rx);
    s.update_busy = true;
    s.update_progress = Some((0, None));
    std::thread::spawn(move || {
        let result = updates::download_and_install(&offer, |ev| match ev {
            updates::UpdateEvent::Progress { downloaded, total } => {
                let _ = tx.send(UpdateMsg::Progress(downloaded, total));
            }
            updates::UpdateEvent::Installing => {
                let _ = tx.send(UpdateMsg::Installing);
            }
        });
        let _ = tx.send(UpdateMsg::Done(result));
    });
}

/// Drain the update worker's messages; true to redraw.
fn drain_update(s: &mut App) -> bool {
    let mut changed = false;
    let mut finished = false;
    if let Some(rx) = &s.update_rx {
        while let Ok(msg) = rx.try_recv() {
            match msg {
                UpdateMsg::Checked(result) => {
                    s.update_busy = false;
                    finished = true;
                    match result {
                        Ok(Some(offer)) => {
                            s.update_offer = Some(offer);
                            // A quiet launch check must not interrupt another
                            // dialog (the default-manager offer); a manual check
                            // always shows what it found.
                            if !s.update_silent || s.dialog.is_none() {
                                s.dialog = Some(Dialog::Update);
                            }
                            s.notice.clear();
                        }
                        Ok(None) => {
                            if !s.update_silent {
                                s.notice = format!("You're up to date (version {})", env!("CARGO_PKG_VERSION"));
                            }
                        }
                        Err(e) => {
                            if !s.update_silent {
                                s.notice = e;
                            }
                        }
                    }
                    changed = true;
                }
                UpdateMsg::Progress(downloaded, total) => {
                    s.update_progress = Some((downloaded, total));
                    changed = true;
                }
                UpdateMsg::Installing => {
                    s.update_installing = true;
                    changed = true;
                }
                UpdateMsg::Done(result) => {
                    s.update_busy = false;
                    s.update_installing = false;
                    s.update_progress = None;
                    finished = true;
                    match result {
                        Ok(()) => {
                            s.notice = "Update installed; restart to finish".into();
                            s.dialog = None;
                        }
                        Err(e) => s.notice = e,
                    }
                    changed = true;
                }
            }
        }
    }
    if finished {
        s.update_rx = None;
    }
    changed
}

fn copy_selection(s: &mut App) {
    let paths = selected_paths(s);
    if !paths.is_empty() {
        s.clipboard = Some((paths, fileops::Mode::Copy));
        s.notice = "Copied to clipboard".into();
    }
}

fn cut_selection(s: &mut App) {
    let paths = selected_paths(s);
    if !paths.is_empty() {
        s.clipboard = Some((paths, fileops::Mode::Cut));
        s.notice = "Cut to clipboard".into();
    }
}

/// Spawn the worker for a paste, wiring progress into the status bar.
fn start_paste(s: &mut App, mode: fileops::Mode, conflict: fileops::Conflict, sources: Vec<PathBuf>, target: PathBuf) {
    use std::sync::atomic::Ordering;
    let total = sources.len();
    let label = if mode == fileops::Mode::Cut { "Moving" } else { "Copying" };
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    s.op_rx = Some(rx);
    s.op_cancel = Some(cancel);
    s.transfer = Some(Transfer { label, name: String::new(), done: 0, total, done_bytes: 0, total_bytes: 0, stopping: false });
    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let emit = |p: fileops::Progress| {
            let _ = progress_tx.send(OpMsg::Progress(p));
            !worker_cancel.load(Ordering::Relaxed)
        };
        let result = fileops::paste(mode, conflict, &sources, &target, &worker_cancel, emit);
        let _ = tx.send(OpMsg::Done(result));
    });
}

fn start_delete(s: &mut App, mode: fileops::DeleteMode, paths: Vec<PathBuf>) {
    use std::sync::atomic::Ordering;
    let total = paths.len();
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    s.op_rx = Some(rx);
    s.op_cancel = Some(cancel);
    s.transfer = Some(Transfer { label: "Deleting", name: String::new(), done: 0, total, done_bytes: 0, total_bytes: 0, stopping: false });
    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let emit = |p: fileops::Progress| {
            let _ = progress_tx.send(OpMsg::Progress(p));
            !worker_cancel.load(Ordering::Relaxed)
        };
        let result = fileops::delete(mode, &paths, &worker_cancel, emit);
        let _ = tx.send(OpMsg::Done(result));
    });
}

/// Paste the clipboard into the current folder, asking about conflicts first.
fn begin_paste(s: &mut App) {
    let Some((sources, mode)) = s.clipboard.clone() else {
        s.notice = "Nothing to paste".into();
        return;
    };
    if s.view != View::Files {
        s.notice = "Open a folder to paste into".into();
        return;
    }
    let target = s.root.clone();
    // Cutting into the folder an item already lives in is a no-op, not a conflict.
    let sources: Vec<PathBuf> = if mode == fileops::Mode::Cut {
        sources
            .into_iter()
            .filter(|src| src.parent() != Some(target.as_path()))
            .collect()
    } else {
        sources
    };
    if sources.is_empty() {
        s.menu = None;
        s.notice = "Those items are already in this folder".into();
        return;
    }
    let conflict_exists = sources.iter().any(|src| {
        src.file_name().map(|n| target.join(n).exists()).unwrap_or(false)
    });
    s.menu = None;
    if conflict_exists {
        s.dialog = Some(Dialog::PasteConflict { mode, sources, target });
    } else {
        start_paste(s, mode, fileops::Conflict::Rename, sources, target);
    }
}

/// Ask before deleting; Shift chooses permanent, otherwise the trash.
fn begin_delete(s: &mut App, permanent: bool) {
    let paths = selected_paths(s);
    if paths.is_empty() {
        return;
    }
    s.menu = None;
    s.dialog = Some(Dialog::Delete { permanent, paths });
}

fn begin_rename(s: &mut App) {
    let Some(e) = s.selected.first().and_then(|&i| s.entries.get(i)) else { return };
    let mut edit = TextEdit::default();
    edit.text = e.name.clone();
    s.menu = None;
    s.dialog = Some(Dialog::Rename { path: PathBuf::from(&e.path), edit });
}

fn begin_new_folder(s: &mut App) {
    if s.view != View::Files {
        s.notice = "Open a folder to create one in".into();
        return;
    }
    let mut edit = TextEdit::default();
    edit.text = "New folder".into();
    s.menu = None;
    s.dialog = Some(Dialog::NewFolder { parent: s.root.clone(), edit });
}

fn commit_rename(s: &mut App) {
    let Some(Dialog::Rename { path, edit }) = s.dialog.take() else { return };
    let name = edit.text.trim().to_string();
    if name.is_empty() {
        return;
    }
    match fileops::rename(&path, &name) {
        Ok(_) => {
            if s.view == View::Files {
                let root = s.root.clone();
                set_location(s, root);
            }
        }
        Err(e) => s.notice = e,
    }
}

fn commit_new_folder(s: &mut App) {
    let Some(Dialog::NewFolder { parent, edit }) = s.dialog.take() else { return };
    let name = edit.text.trim().to_string();
    if name.is_empty() {
        return;
    }
    match fileops::create_folder(&parent, &name) {
        Ok(_) => {
            if s.view == View::Files {
                let root = s.root.clone();
                set_location(s, root);
            }
        }
        Err(e) => s.notice = e,
    }
}

fn cancel_op(s: &mut App) {
    if let Some(cancel) = &s.op_cancel {
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(t) = &mut s.transfer {
        t.stopping = true;
    }
}

/// Drain the worker's messages into the status bar; returns true to redraw.
fn drain_op(s: &mut App) -> bool {
    let mut changed = false;
    let mut finished = false;
    if let Some(rx) = &s.op_rx {
        while let Ok(msg) = rx.try_recv() {
            match msg {
                OpMsg::Progress(p) => {
                    if let Some(t) = &mut s.transfer {
                        t.name = p.name;
                        t.done = p.done;
                        t.total = p.total;
                        t.done_bytes = p.done_bytes;
                        t.total_bytes = p.total_bytes;
                    }
                    changed = true;
                }
                OpMsg::Done(result) => {
                    finished = true;
                    let label = s.transfer.as_ref().map(|t| t.label).unwrap_or("Done");
                    let past = match label {
                        "Copying" => "Copied",
                        "Moving" => "Moved",
                        "Deleting" => "Deleted",
                        _ => "Finished",
                    };
                    match result {
                        Ok(n) => s.notice = format!("{past} {n} item{}", if n == 1 { "" } else { "s" }),
                        Err(e) => s.notice = e,
                    }
                }
            }
        }
    }
    if finished {
        s.op_rx = None;
        s.op_cancel = None;
        s.transfer = None;
        clear_selection(s);
        if s.view == View::Files {
            let root = s.root.clone();
            set_location(s, root);
        }
        changed = true;
    }
    changed
}

/// Open the selected item: a folder navigates, a file launches its default app.
fn open_selected(s: &mut App) {
    let Some(i) = s.focused.or_else(|| s.selected.first().copied()) else { return };
    let valid = if s.searching { i < s.search_hits.len() } else { i < s.entries.len() };
    if !valid {
        return;
    }
    let e = entry_by_index(s, i);
    let path = e.path.clone();
    let is_dir = e.is_dir;
    open_path(s, &path, is_dir);
}

/// Open one path: a folder navigates, a file opens with its default app. A
/// file is remembered in Home's Recent list.
fn open_path(s: &mut App, path: &str, is_dir: bool) {
    if is_dir {
        go(s, PathBuf::from(path));
    } else {
        record_recent(s, path);
        if let Err(err) = shell::open_default(path) {
            s.notice = err;
        }
    }
}

/// Open a Home Recent row, which is always a file.
fn open_recent(s: &mut App, i: usize) {
    let Some(e) = s.entries.get(i) else { return };
    let path = e.path.clone();
    let is_dir = e.is_dir;
    open_path(s, &path, is_dir);
    // The opened file moves to the front of the list.
    refresh_recent(s);
}

/// Global shortcuts. Returns true when the key was handled. Editing keys are
/// left to a focused field or an open dialog.
fn handle_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.key == Key::Escape {
        if s.dialog.is_some() {
            s.dialog = None;
            return true;
        }
        if s.menu.is_some() {
            s.menu = None;
            return true;
        }
        return false;
    }
    if s.dialog.is_some() || s.search_edit.focused {
        return false;
    }
    if ev.is_command('c') {
        copy_selection(s);
        return true;
    }
    if ev.is_command('x') {
        cut_selection(s);
        return true;
    }
    if ev.is_command('v') {
        begin_paste(s);
        return true;
    }
    if ev.is_command('a') {
        select_all(s);
        return true;
    }
    match ev.key {
        Key::Delete => {
            begin_delete(s, ev.mods.shift);
            true
        }
        Key::Function(2) => {
            begin_rename(s);
            true
        }
        Key::Enter => {
            open_selected(s);
            true
        }
        Key::Backspace => {
            go_up(s);
            true
        }
        Key::Up | Key::Left => {
            move_focus(s, focus_row(s).saturating_sub(1), ev.mods, tx);
            true
        }
        Key::Down | Key::Right => {
            move_focus(s, focus_row(s) + 1, ev.mods, tx);
            true
        }
        Key::Home => {
            move_focus(s, 0, ev.mods, tx);
            true
        }
        Key::End => {
            move_focus(s, display_len(s).saturating_sub(1), ev.mods, tx);
            true
        }
        _ => false,
    }
}

fn file_row(s: &App, i: usize, p: Paint) -> El {
    let (ei, e) = display_entry(s, i);
    let selected = is_selected(s, ei);
    let focused = s.focused == Some(ei);
    let mut line = row()
        .key(format!("row-{i}"))
        .w(row_width(s))
        .h(row_h(s))
        .items_center()
        .fill(if selected { p.row_sel } else { Color::TRANSPARENT })
        .hover(p.hover)
        .opacity(if is_cut(s, &e.path) { 0.4 } else { 1.0 });
    if s.reveal == Some(ei) {
        line = line.border(2.0, p.accent);
    } else if focused && selected {
        line = line.border(p.hair, p.focus_line);
    }
    let line = line
        .child(
            row()
                .w(s.col_w[0])
                .pl(12.0)
                .pr(8.0)
                .gap(8.0)
                .items_center()
                .child(entry_icon(s, e, p, 16.0))
                .child(
                    name_text(s, e, ei, ROW_TEXT, p.text, p)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                ),
        )
        .child(el().w(COL_SEP_W).h(16.0).center().child(el().w(p.hair).h(16.0).fill(p.soft_line)))
        .child(
            col()
                .w(s.col_w[1])
                .pl(12.0)
                .child(text(e.modified.as_str()).font(ROW_TEXT).color(p.secondary)),
        )
        .child(el().w(COL_SEP_W).h(16.0).center().child(el().w(p.hair).h(16.0).fill(p.soft_line)))
        .child(
            col().w(s.col_w[2]).pl(12.0).pr(8.0).child(
                text(e.type_label.as_str())
                    .font(ROW_TEXT)
                    .color(p.secondary)
                    .w_full()
                    .truncate(),
            ),
        )
        .child(el().w(COL_SEP_W).h(16.0).center().child(el().w(p.hair).h(16.0).fill(p.soft_line)))
        .child(
            col().w(s.col_w[3]).pl(8.0).pr(12.0).child(
                text(e.size_label())
                    .font(ROW_TEXT)
                    .color(p.secondary)
                    .w_full()
                    .align(Align::End),
            ),
        )
        .child(el().w(COL_SEP_W).h(16.0).center().child(el().w(p.hair).h(16.0).fill(p.soft_line)));
    marquee_track(
        line.on_press_tx(move |s: &mut App, tx: &mut Tx| {
            click_select(s, ei, tx.mods());
        })
        .on_double_press(move |s: &mut App| {
            select_entry(s, ei);
            open_selected(s);
        })
        .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
            if !is_selected(s, ei) {
                select_entry(s, ei);
            }
            s.menu = Some(Menu::Context { scope: CtxScope::Item, target: ei, x: at.x, y: at.y });
        }),
        i,
    )
}

/// Wrap list content in a scroll viewport plus a drawn scrollbar gutter. The
/// viewport reports its offset into Live bindings each frame; the window-level
/// thumb follows without a view re-run, and the gutter reserves its lane.
fn scrollable(s: &App, p: Paint, content: El) -> El {
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
        })
        .drawn_rect_into(&s.viewport_rect);
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
        .child(marquee_overlay(s, p))
}

fn details_rows(s: &App, p: Paint) -> El {
    let list = if !s.groups.is_empty() && !s.searching {
        let mut list = col().key("rows").w_full();
        for i in 0..s.visible.len() {
            if let Some((_, label)) = s.groups.iter().find(|(start, _)| *start == i) {
                list = list.child(group_header(p, label));
            }
            list = list.child(file_row(s, i, p));
        }
        list.on_drag(|s: &mut App, d: &mut rune::Drag, tx: &mut Tx| marquee_drag(s, d, tx))
    } else {
        col()
            .key("rows")
            .w_full()
            .growth(GrowthSpec::default())
            .virtual_rows(
                display_len(s),
                row_h(s),
                |s: &App, i| Cow::Borrowed(display_entry(s, i).1.path.as_str()),
                move |s: &App, i| file_row(s, i, p),
            )
            .on_drag(|s: &mut App, d: &mut rune::Drag, tx: &mut Tx| marquee_drag(s, d, tx))
    };
    scrollable(s, p, list)
}

fn grid_tile(s: &App, i: usize, p: Paint) -> El {
    let (ei, e) = display_entry(s, i);
    let selected = is_selected(s, ei);
    let preview = if e.fixture && e.kind == Kind::Image {
        let at = ["095523", "103957", "104522", "111606", "111619"]
            .iter()
            .position(|t| e.name.ends_with(t))
            .unwrap_or(0);
        screenshot_thumb(at, 90.0, 72.0)
    } else if previewable(e) {
        thumb_rect(s, e, p, 90.0, 72.0, 128)
    } else {
        el().w(90.0).h(72.0).center().child(entry_icon(s, e, p, 44.0))
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
        .opacity(if is_cut(s, &e.path) { 0.4 } else { 1.0 })
        .child(preview)
        .child(
            name_text(s, e, ei, CAPTION, p.text, p)
                .w(94.0)
                .align(Align::Center)
                .max_lines(3),
        );
    if selected {
        tile = tile.border(p.hair, p.focus_line);
    }
    marquee_track(
        tile.on_press_tx(move |s: &mut App, tx: &mut Tx| {
            click_select(s, ei, tx.mods());
        })
        .on_double_press(move |s: &mut App| {
            select_entry(s, ei);
            open_selected(s);
        })
        .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
            if !is_selected(s, ei) {
                select_entry(s, ei);
            }
            s.menu = Some(Menu::Context { scope: CtxScope::Item, target: ei, x: at.x, y: at.y });
        }),
        i,
    )
}

fn grid_rows(s: &App, p: Paint) -> El {
    let tiles = row()
        .key("tiles")
        .w_full()
        .flex_wrap()
        .pt(12.0)
        .px(16.0)
        .gap(8.0)
        .children((0..display_len(s)).map(|i| grid_tile(s, i, p)))
        .on_drag(|s: &mut App, d: &mut rune::Drag, tx: &mut Tx| marquee_drag(s, d, tx));
    scrollable(s, p, tiles)
}

/// List view: a compact small-icon + name flow, denser under Compact view.
fn list_item(s: &App, i: usize, p: Paint) -> El {
    let (ei, e) = display_entry(s, i);
    let selected = is_selected(s, ei);
    let mut item = row()
        .key(format!("list-{i}"))
        .w(230.0)
        .h(if s.compact { 22.0 } else { 26.0 })
        .radius(4.0)
        .fill(if selected { p.rail_sel } else { Color::TRANSPARENT })
        .hover(p.hover)
        .px(6.0)
        .gap(8.0)
        .items_center()
        .opacity(if is_cut(s, &e.path) { 0.4 } else { 1.0 })
        .child(entry_icon(s, e, p, 16.0))
        .child(
            name_text(s, e, ei, ROW_TEXT, p.text, p)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if selected {
        item = item.border(p.hair, p.focus_line);
    }
    marquee_track(
        item.on_press_tx(move |s: &mut App, tx: &mut Tx| {
            click_select(s, ei, tx.mods());
        })
        .on_double_press(move |s: &mut App| {
            select_entry(s, ei);
            open_selected(s);
        })
        .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
            if !is_selected(s, ei) {
                select_entry(s, ei);
            }
            s.menu = Some(Menu::Context { scope: CtxScope::Item, target: ei, x: at.x, y: at.y });
        }),
        i,
    )
}

fn list_rows(s: &App, p: Paint) -> El {
    let items = row()
        .key("list-items")
        .w_full()
        .flex_wrap()
        .pt(8.0)
        .px(12.0)
        .gap(2.0)
        .children((0..display_len(s)).map(|i| list_item(s, i, p)))
        .on_drag(|s: &mut App, d: &mut rune::Drag, tx: &mut Tx| marquee_drag(s, d, tx));
    scrollable(s, p, items)
}

/// Tiles view: a medium icon with the name and a type/size subtitle.
fn tile_item(s: &App, i: usize, p: Paint) -> El {
    let (ei, e) = display_entry(s, i);
    let selected = is_selected(s, ei);
    let subtitle = if e.is_dir {
        e.type_label.clone()
    } else {
        format!("{}  {}", e.type_label, e.size_label())
    };
    let mut tile = row()
        .key(format!("tileitem-{i}"))
        .w(268.0)
        .h(64.0)
        .radius(4.0)
        .fill(if selected { p.rail_sel } else { Color::TRANSPARENT })
        .hover(p.hover)
        .px(8.0)
        .gap(10.0)
        .items_center()
        .opacity(if is_cut(s, &e.path) { 0.4 } else { 1.0 })
        .child(if previewable(e) {
            thumb_rect(s, e, p, 40.0, 40.0, 64)
        } else {
            entry_icon(s, e, p, 32.0)
        })
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(name_text(s, e, ei, ROW_TEXT, p.text, p).w_full().truncate())
                .child(text(subtitle.as_str()).font(CAPTION).color(p.tertiary).w_full().truncate()),
        );
    if selected {
        tile = tile.border(p.hair, p.focus_line);
    }
    marquee_track(
        tile.on_press_tx(move |s: &mut App, tx: &mut Tx| {
            click_select(s, ei, tx.mods());
        })
        .on_double_press(move |s: &mut App| {
            select_entry(s, ei);
            open_selected(s);
        })
        .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
            if !is_selected(s, ei) {
                select_entry(s, ei);
            }
            s.menu = Some(Menu::Context { scope: CtxScope::Item, target: ei, x: at.x, y: at.y });
        }),
        i,
    )
}

fn tiles_rows(s: &App, p: Paint) -> El {
    let tiles = row()
        .key("tiles-view")
        .w_full()
        .flex_wrap()
        .pt(12.0)
        .px(16.0)
        .gap(8.0)
        .children((0..display_len(s)).map(|i| tile_item(s, i, p)))
        .on_drag(|s: &mut App, d: &mut rune::Drag, tx: &mut Tx| marquee_drag(s, d, tx));
    scrollable(s, p, tiles)
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
            &[("Name", 440.0), ("Date modified", 240.0), ("Activity", 160.0)],
            false,
            false,
        ));
        if s.entries.is_empty() {
            table = table.child(
                col()
                    .w_full()
                    .h(110.0)
                    .center()
                    .child(ico(p, Ico::Clock, 28.0, p.disabled))
                    .child(text("No recent files yet").font(CAPTION).color(p.tertiary)),
            );
        }
        for &ei in s.visible.iter() {
            let e = &s.entries[ei];
            let selected = is_selected(s, ei);
            let mut row = row()
                .w_full()
                .h(46.0)
                .items_center()
                .fill(if selected { p.row_sel } else { Color::TRANSPARENT })
                .hover(p.hover)
                .child(
                    row()
                        .w(440.0)
                        .pl(16.0)
                        .gap(10.0)
                        .items_center()
                        .child(entry_icon(s, e, p, 24.0))
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
                .child(col().w(160.0));
            row = row
                .on_press_tx(move |st: &mut App, tx: &mut Tx| click_select(st, ei, tx.mods()))
                .on_double_press(move |st: &mut App| {
                    select_entry(st, ei);
                    open_recent(st, ei);
                })
                .on_secondary_press(move |st: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
                    if !is_selected(st, ei) {
                        select_entry(st, ei);
                    }
                    st.menu = Some(Menu::Context { scope: CtxScope::Item, target: ei, x: at.x, y: at.y });
                });
            table = table.child(row);
        }
    } else if s.favorites.is_empty() {
        table = table.child(
            col()
                .w_full()
                .h(120.0)
                .center()
                .child(ico(p, Ico::Star, 28.0, p.disabled))
                .child(text("No favorites yet").font(CAPTION).color(p.tertiary)),
        );
    } else {
        table = table.child(column_header(
            p,
            &[("Name", 440.0), ("Location", 240.0), ("", 160.0)],
            false,
            false,
        ));
        for (fi, fav) in s.favorites.iter().enumerate() {
            let name = path_name(fav);
            let parent = Path::new(fav)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let is_dir = Path::new(fav).is_dir();
            let open = fav.clone();
            table = table.child(
                row()
                    .w_full()
                    .h(40.0)
                    .items_center()
                    .hover(p.hover)
                    .child(
                        row()
                            .w(440.0)
                            .pl(16.0)
                            .gap(10.0)
                            .items_center()
                            .child(ico(p, Ico::Star, 16.0, p.accent))
                            .child(text(name.as_str()).font(ROW_TEXT).color(p.text).truncate()),
                    )
                    .child(
                        col()
                            .w(240.0)
                            .pl(12.0)
                            .child(text(parent.as_str()).font(CAPTION).color(p.tertiary).truncate()),
                    )
                    .on_press(move |s: &mut App| {
                        if is_dir {
                            go(s, PathBuf::from(&open));
                        }
                    })
                    .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
                        s.menu = Some(Menu::Context { scope: CtxScope::Favorite, target: fi, x: at.x, y: at.y });
                    }),
            );
        }
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
    let mut list = col()
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
        );
    if s.volumes.is_empty() {
        return list.child(
            col()
                .w_full()
                .h(120.0)
                .center()
                .child(ico(p, Ico::Info, 28.0, p.disabled))
                .child(text("No devices found").font(CAPTION).color(p.tertiary)),
        );
    }
    for (vi, volume) in s.volumes.iter().enumerate() {
        let selected = s.volumes_selected == Some(vi);
        let title = volume.title();
        let space = volume.space_label();
        let used = volume.used_fraction();
        let ejectable = volume.is_ejectable();
        let mut card = row()
            .w(300.0)
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
                    .child(text(title.as_str()).font(ROW_TEXT).color(p.text))
                    .child(
                        el().w_full()
                            .h(10.0)
                            .radius(2.0)
                            .fill(c("#e6e6e6"))
                            .child(el().w_frac(used).h_full().radius(2.0).fill(c("#0070cb"))),
                    )
                    .child(text(space.as_str()).font(CAPTION).color(p.tertiary)),
            );
        if ejectable {
            card = card.child(
                el().key(format!("eject-{vi}"))
                    .size(28.0, 28.0)
                    .radius(4.0)
                    .hover(p.hover)
                    .center()
                    .help("Eject")
                    .child(ico(p, Ico::Eject, 14.0, p.secondary))
                    .on_press({
                        let path = volume.path.clone();
                        let title = title.clone();
                        move |s: &mut App| match volumes::eject(&path) {
                            Ok(()) => {
                                s.notice = format!("{title} ejected");
                                refresh_volumes(s);
                            }
                            Err(e) => s.notice = e,
                        }
                    }),
            );
        }
        let open = volume.path.clone();
        card = card
            .on_press(move |s: &mut App| s.volumes_selected = Some(vi))
            .on_double_press(move |s: &mut App| go(s, open.clone()))
            .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
                s.volumes_selected = Some(vi);
                s.menu = Some(Menu::Context { scope: CtxScope::Volume, target: vi, x: at.x, y: at.y });
            });
        list = list.child(card);
    }
    list
}

/// The month a photo belongs to, for the Gallery's group headers.
fn month_label(at: Option<std::time::SystemTime>) -> String {
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ];
    let Some(at) = at else { return "Unknown date".to_string() };
    let Ok(secs) = at.duration_since(UNIX_EPOCH) else { return "Unknown date".to_string() };
    let (y, m, _) = civil_from_days((secs.as_secs() / 86_400) as i64);
    format!("{} {}", MONTHS[(m - 1).clamp(0, 11) as usize], y)
}

/// Walk a folder for images, newest first, bounded so a large library cannot
/// stall the view. Dot-folders are skipped.
fn sweep_images(root: &Path, limit: usize, max_depth: usize) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth || out.len() >= limit {
            continue;
        }
        let Ok(reader) = fs::read_dir(&dir) else { continue };
        for item in reader.flatten() {
            if out.len() >= limit {
                break;
            }
            let name = item.file_name().to_string_lossy().into_owned();
            let Ok(meta) = item.metadata() else { continue };
            if meta.is_dir() {
                if !name.starts_with('.') {
                    stack.push((item.path(), depth + 1));
                }
            } else {
                let kind = classify(&name, false);
                if kind != Kind::Image {
                    continue;
                }
                let modified_at = meta.modified().ok();
                let modified = modified_at
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| format_time(d.as_secs()))
                    .unwrap_or_default();
                out.push(Entry {
                    name,
                    path: item.path().to_string_lossy().into_owned(),
                    is_dir: false,
                    size: meta.len(),
                    modified,
                    modified_at,
                    type_label: kind_label(kind).to_string(),
                    kind,
                    hidden: false,
                    system: false,
                    fixture: false,
                    fixture_size: String::new(),
                });
            }
        }
    }
    out.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    out
}

/// Re-read the Pictures library into the Gallery's entries.
fn refresh_gallery(s: &mut App) {
    let Some(home) = home_dir() else { return };
    s.entries = sweep_images(&home.join("Pictures"), 4000, 8);
    s.visible = (0..s.entries.len()).collect();
    s.count = s.entries.len();
    s.count_override = Some(s.count);
    s.groups.clear();
}

/// Open the Gallery: the Pictures library as a photo grid. A re-click is a
/// no-op, as with the other places.
fn go_gallery(s: &mut App) {
    s.menu = None;
    if s.view == View::Gallery {
        return;
    }
    let Some(home) = home_dir() else {
        s.notice = "Pictures is not available here".into();
        return;
    };
    let pictures = home.join("Pictures");
    if !pictures.is_dir() {
        s.notice = "Pictures is not available here".into();
        return;
    }
    s.history.push(s.root.clone());
    s.forward.clear();
    s.root = pictures;
    s.view = View::Gallery;
    s.tab = "Gallery".into();
    s.crumbs = vec!["Gallery".into()];
    s.search = "Search Gallery".into();
    s.nav = 1;
    s.preview = true;
    s.contextual = true;
    s.groups.clear();
    clear_selection(s);
    s.notice.clear();
    s.search_edit.text.clear();
    s.keep_order = false;
    s.count_override = None;
    refresh_gallery(s);
}

/// One photo in the Gallery grid.
fn gallery_cell(s: &App, i: usize, p: Paint) -> El {
    let (ei, e) = display_entry(s, i);
    let selected = is_selected(s, ei);
    marquee_track(
        col()
            .key(format!("gcell-{i}"))
            .w(148.0)
            .radius(4.0)
            .fill(if selected { p.row_sel } else { Color::TRANSPARENT })
            .hover(p.hover)
            .px(4.0)
            .py(4.0)
            .gap(4.0)
            .items_center()
            .child(thumb_rect(s, e, p, 140.0, 100.0, 160))
            .child(
                text(e.name.as_str())
                    .font(CAPTION)
                    .color(p.text)
                    .w_full()
                    .align(Align::Center)
                    .max_lines(2),
            )
            .on_press_tx(move |s: &mut App, tx: &mut Tx| click_select(s, ei, tx.mods()))
            .on_double_press(move |s: &mut App| {
                select_entry(s, ei);
                open_selected(s);
            })
            .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
                if !is_selected(s, ei) {
                    select_entry(s, ei);
                }
                s.menu = Some(Menu::Context { scope: CtxScope::Item, target: ei, x: at.x, y: at.y });
            }),
        i,
    )
}

/// The Gallery: the Pictures library as a date-grouped photo grid.
fn gallery_content(s: &App, p: Paint) -> El {
    let mut out = col()
        .key("gallery")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .fill(p.content)
        .pb(16.0);
    let len = display_len(s);
    if len == 0 {
        out = out.child(
            col()
                .w_full()
                .h(160.0)
                .center()
                .child(ico(p, Ico::Pictures, 30.0, p.disabled))
                .child(text("No pictures found").font(CAPTION).color(p.tertiary)),
        );
    }
    let mut month = String::new();
    let mut cells: Vec<El> = Vec::new();
    for i in 0..len {
        let this = month_label(display_entry(s, i).1.modified_at);
        if this != month {
            if !cells.is_empty() {
                out = out.child(
                    row().w_full().flex_wrap().gap(4.0).px(16.0).children(cells.drain(..)),
                );
            }
            out = out.child(
                text(this.as_str())
                    .font(TILE_TEXT)
                    .color(p.text)
                    .px(16.0)
                    .pt(14.0),
            );
            month = this;
        }
        cells.push(gallery_cell(s, i, p));
    }
    if !cells.is_empty() {
        out = out.child(row().w_full().flex_wrap().gap(4.0).px(16.0).pt(4.0).children(cells));
    }
    scrollable(s, p, out.on_drag(|s: &mut App, d: &mut rune::Drag, tx: &mut Tx| marquee_drag(s, d, tx)))
}

fn preview_pane(s: &App, p: Paint) -> El {
    let inner = if s.view == View::ThisPc {
        let title = s
            .volumes_selected
            .and_then(|i| s.volumes.get(i))
            .map(|v| v.title())
            .unwrap_or_else(|| "This PC".to_string());
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
                    .child(text(title.as_str()).font(TILE_TEXT).color(p.text))
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
                                text("Select a device to see its free space, or eject a removable one.")
                                    .font(CAPTION)
                                    .color(p.secondary)
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .max_lines(3),
                            ),
                    ),
            )
    } else {
        let e = s.selected.first().and_then(|&i| s.entries.get(i)).or_else(|| s.entries.first());
        let name = e.map(|x| x.name.as_str()).unwrap_or("");
        let thumb = match e {
            Some(entry) if previewable(entry) => {
                let img = entry_thumb(s, entry, 320);
                col()
                    .w_full()
                    .h(210.0)
                    .fill(p.thumb_bg)
                    .center()
                    .child(el().size(PREVIEW_W - 2.0, 210.0).pixels_fit(img.pixels(), SurfaceFit::Fit))
            }
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
                .child(entry_icon(s, entry, p, 64.0)),
            None => col().w_full().h(210.0).fill(p.thumb_bg),
        };
        // A Gallery photo belongs to its own folder, not to the Gallery.
        let location = if s.view == View::Gallery {
            e.and_then(|x| Path::new(&x.path).parent())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            s.crumbs.join(" / ")
        };
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

fn files_list(s: &App, p: Paint) -> El {
    col()
        .key("content")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .fill(p.content)
        .on_secondary_press(|s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
            s.menu = Some(Menu::Context { scope: CtxScope::Background, target: 0, x: at.x, y: at.y });
        })
        .child(if s.mode == Mode::Details {
            details_header(s, p)
        } else {
            el().w_full().h(p.hair)
        })
        .child(el().w_full().h(p.hair).fill(p.line))
        .child(match s.mode {
            Mode::Details => details_rows(s, p),
            Mode::List => list_rows(s, p),
            Mode::Tiles => tiles_rows(s, p),
            Mode::Grid => grid_rows(s, p),
        })
}

fn content(s: &App, p: Paint) -> El {
    // Search results render as a file list whichever place is active.
    let main = if s.searching {
        files_list(s, p)
    } else {
        match s.view {
            View::Home => home_content(s, p),
            View::ThisPc => this_pc_content(s, p),
            View::Gallery => gallery_content(s, p),
            View::Files => files_list(s, p),
        }
    };
    // While a flyout or context menu is open, the work area is inert: a row or
    // rail item behind the menu neither takes the press nor lights up. The top
    // bar stays live, so its buttons still switch or close the menu in one press.
    let inert = s.menu.is_some();
    let mut body = row().key("body").w_full().grow(1.0).min_h(0.0);
    if s.nav_open {
        let rail = nav_rail(s, p);
        body = body.child(if inert { rail.inert() } else { rail });
    }
    body = body.child(if inert { main.inert() } else { main });
    if s.preview {
        let pane = preview_pane(s, p);
        body = body.child(if inert { pane.inert() } else { pane });
    }
    if inert {
        body = body.child(
            el()
                .key("menu-scrim")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .fill(Color::TRANSPARENT)
                .on_press(|s: &mut App| s.menu = None),
        );
    }
    body
}

fn body(s: &App, p: Paint) -> El {
    content(s, p)
}

fn status_bar(s: &App, p: Paint) -> El {
    let bar = row()
        .key("status")
        .w_full()
        .h(STATUS_H)
        .fill(p.status)
        .px(12.0)
        .gap(8.0)
        .items_center();

    // A running transfer takes the bar: label, progress, percent and cancel.
    if let Some(t) = &s.transfer {
        let frac = if t.total_bytes > 0 {
            (t.done_bytes as f64 / t.total_bytes as f64).min(1.0) as f32
        } else if t.total > 0 {
            (t.done as f32 / t.total as f32).min(1.0)
        } else {
            0.0
        };
        let readout = if t.total_bytes > 0 {
            format!("{}%", (frac * 100.0) as u32)
        } else {
            format!("{} of {}", t.done.min(t.total), t.total)
        };
        let label = if t.stopping { "Stopping".to_string() } else { format!("{} {}", t.label, t.name) };
        let cancel_label = if t.stopping { "Stopping…" } else { "Cancel" };
        return bar
            .child(el().size(7.0, 7.0).radius(4.0).fill(p.accent))
            .child(text(label.as_str()).font(CAPTION).color(p.secondary).w(240.0).truncate())
            .child(
                el().w(160.0).h(6.0).radius(3.0).fill(p.line)
                    .child(el().w_frac(frac).h_full().radius(3.0).fill(p.accent_fill)),
            )
            .child(text(readout.as_str()).font(CAPTION).color(p.secondary))
            .child(
                row().h(18.0).px(8.0).radius(3.0).hover(p.hover).center()
                    .child(text(cancel_label).font(CAPTION).color(p.secondary))
                    .on_press(|s: &mut App| cancel_op(s)),
            )
            .child(el().grow(1.0));
    }

    // While searching, the count that matters is matches, and the walk reports
    // how much of the tree it has examined.
    if s.searching {
        let results = format!(
            "{} {}{}",
            s.search_matched,
            if s.search_matched == 1 { "result" } else { "results" },
            if s.search_truncated { " (showing the best)" } else { "" }
        );
        let elapsed = if s.search_elapsed < 1 { "<1 ms".to_string() } else { format!("{} ms", s.search_elapsed) };
        return bar
            .child(text(results.as_str()).font(CAPTION).color(p.secondary))
            .child(el().w(p.hair).h(12.0).fill(p.line))
            .child(if s.search_scanned > 0 {
                text(format!("{} examined", s.search_scanned)).font(CAPTION).color(p.tertiary)
            } else {
                text("").font(CAPTION)
            })
            .child(el().grow(1.0))
            .child(if s.search_busy {
                row().gap(6.0).items_center()
                    .child(el().size(7.0, 7.0).radius(4.0).fill(p.accent))
                    .child(text("Searching").font(CAPTION).color(p.secondary))
            } else {
                row()
            })
            .child(text(elapsed.as_str()).font(CAPTION).color(p.tertiary));
    }

    let middle = if s.notice.is_empty() {
        s.selection_detail.as_str()
    } else {
        s.notice.as_str()
    };
    let filtering = !s.search_edit.text.is_empty();
    let elapsed = if s.scan_ms < 1.0 { "<1 ms".to_string() } else { format!("{:.0} ms", s.scan_ms) };
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
    bar.child(
        text(if s.count == 1 { "1 item".to_string() } else { format!("{} items", s.count) })
            .font(CAPTION)
            .color(p.secondary),
    )
    .child(el().w(p.hair).h(12.0).fill(p.line))
    .child(text(middle).font(CAPTION).color(if s.notice.is_empty() { p.secondary } else { p.accent }))
    .child(el().grow(1.0))
    .child(if filtering {
        text(format!("{} shown", s.visible.len())).font(CAPTION).color(p.secondary)
    } else {
        text("").font(CAPTION)
    })
    .child(text(elapsed.as_str()).font(CAPTION).color(p.tertiary))
    .child(
        row()
            .gap(1.0)
            .px(2.0)
            .py(1.0)
            .radius(4.0)
            .border(p.hair, p.line)
            .child(toggle(Ico::Details, s.mode == Mode::Details, false))
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

fn main() {
    // A launch the shell meant for Explorer itself (a /select reveal, a shell
    // namespace) is handed back rather than opened as a folder.
    if defaultmanager::forward_unhostable_launch() {
        return;
    }
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
            ("thispc", |s| navigate(s, 9)),
            ("list", |s| {
                load(s, Fixture::Downloads);
                s.mode = Mode::List;
            }),
            ("tiles", |s| {
                load(s, Fixture::Downloads);
                s.mode = Mode::Tiles;
            }),
            ("menu", |s| {
                load(s, Fixture::Downloads);
                s.menu = Some(Menu::Drop { kind: DropMenu::Sort, x: 336.0, y: 132.0 });
            }),
            ("favorites", |s| {
                load(s, Fixture::Home);
                s.home_tab = 1;
                let root = s.root.to_string_lossy().into_owned();
                s.favorites = vec![root.clone(), format!("{}\\src", root)];
                s.pinned = vec![root];
            }),
            ("customize", |s| {
                load(s, Fixture::Downloads);
                s.customize_open = true;
                s.customize_path = Some(s.root.to_string_lossy().into_owned());
                s.customize_is_dir = true;
                s.customize_background = true;
            }),
            ("context", |s| {
                load(s, Fixture::Downloads);
                s.menu = Some(Menu::Context { scope: CtxScope::Item, target: 0, x: 420.0, y: 200.0 });
            }),
            ("real", |s| {
                s.view = View::Files;
                s.mode = Mode::Details;
            }),
            ("deletedlg", |s| {
                s.view = View::Files;
                s.mode = Mode::Details;
                s.selected = vec![0];
                let paths = selected_paths(s);
                s.dialog = Some(Dialog::Delete { permanent: false, paths });
            }),
            ("transfer", |s| {
                s.view = View::Files;
                s.mode = Mode::Details;
                s.transfer = Some(Transfer {
                    label: "Copying",
                    name: "ubuntu-24.04.iso".into(),
                    done: 3,
                    total: 10,
                    done_bytes: 450,
                    total_bytes: 1000,
                    stopping: false,
                });
            }),
            ("cut", |s| {
                s.view = View::Files;
                s.mode = Mode::Details;
                if let Some(e) = s.entries.first() {
                    s.clipboard = Some((vec![PathBuf::from(&e.path)], fileops::Mode::Cut));
                }
            }),
            ("renamedlg", |s| {
                s.view = View::Files;
                s.mode = Mode::Details;
                if let Some(e) = s.entries.first() {
                    let mut edit = TextEdit::default();
                    edit.text = e.name.clone();
                    s.dialog = Some(Dialog::Rename { path: PathBuf::from(&e.path), edit });
                }
            }),
            ("search", |s| {
                s.view = View::Files;
                s.mode = Mode::Details;
                s.searching = true;
                s.search_busy = false;
                s.search_matched = 3;
                s.search_scanned = 128;
                s.search_elapsed = 12;
                s.search_query = "do".into();
                s.search_edit.text = "do".into();
                s.search_hits = s.entries.iter().take(3).enumerate().map(|(i, e)| {
                    search::Hit { entry: e.clone(), score: 100 - i as u32, indices: vec![0, 1] }
                }).collect();
            }),
            ("moremenu", |s| {
                s.view = View::Files;
                s.mode = Mode::Details;
                s.menu = Some(Menu::Drop { kind: DropMenu::More, x: 820.0, y: 120.0 });
            }),
            ("defaultdlg", |s| {
                s.view = View::Files;
                s.dialog = Some(Dialog::DefaultManager);
            }),
            ("updatedlg", |s| {
                s.view = View::Files;
                s.update_offer = Some(updates::Offer {
                    version: "0.3.0".into(),
                    current: "0.2.0".into(),
                    notes: Some("Faster folder loads, real shell icons, and the new search engine.".into()),
                    url: "https://example.invalid/lyre.exe".into(),
                    signature: String::new(),
                });
                s.dialog = Some(Dialog::Update);
            }),
            ("marquee", |s| {
                load(s, Fixture::Downloads);
                s.mode = Mode::Details;
                s.groups.clear();
                s.selected = s.visible.iter().take(4).copied().collect();
                s.marquee = Some(Marquee { x0: 16.0, y0: 6.0, x1: 420.0, y1: 118.0, base: Vec::new(), auto_scroll: 0 });
            }),
        ],
        poll: Some(poll),
        on_key: Some(handle_key),
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
        // macOS rounds a transparent borderless window through Rune's own corner
        // radius; Windows rounds through DWM in frame::install; Linux leaves the
        // corners to the compositor.
        let window = WindowSpec::new("main", spec).configured_options(
            WindowOptions {
                borderless: true,
                resizable: true,
                min_size: Some((760.0, 480.0)),
                transparent: cfg!(target_os = "macos"),
                ..Default::default()
            }
            .macos(rune::MacWindowOptions::default().corner_radius(8.0)),
        );
        let window = window.on_created(|handle| frame::install(&handle));
        [window.boxed()]
    });
}
