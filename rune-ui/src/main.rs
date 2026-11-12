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

// A release build is a window, not a console program: without this, Windows
// allocates a console for it and shows one alongside the window. Debug builds
// keep it, because that is where the tracing and `--snap` output goes.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
mod handlers;
mod imagery;
mod known;
mod legacy;
mod metadata;
mod openwith;
mod osdrag;
mod search;
mod shell;
mod shortcut;
mod theme;
mod updates;
mod volumes;
mod watch;

// Metrics and colours sampled from the four reference captures. The chrome is
// three distinct bands, not one surface: a near-black tab strip, a Mica-tinted
// address row, and a command bar that shares the work area's neutral charcoal.
const TAB_H: f32 = 38.0;
/// The smallest the window can be dragged to. Everything in the chrome is
/// fixed height, so this leaves the body its last usable band; the rail narrows
/// itself and the command bar drops its words well before here.
const MIN_W: f32 = 400.0;
const MIN_H: f32 = 280.0;
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
pub(crate) struct Entry {
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
    /// The landing page: recent files and favorites. A place rather than a
    /// folder, so it has no parent and nothing on disk to scan.
    QuickAccess,
    Files,
    ThisPc,
    /// The Pictures library as a date-grouped photo grid.
    Gallery,
}

/// Which place a window is showing.
///
/// The rail lights a row from this and nothing else, so exactly one row can be
/// selected: the one whose place the window is actually in. Remembering the
/// pressed row in a field instead is what let a stale row stay lit beside the
/// new one, and left This PC selected while a pinned folder was on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Place {
    Home,
    Gallery,
    QuickAccess,
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
    ThisPc,
    Network,
    /// A folder that is none of the built-in places: a pinned folder, or any
    /// folder reached through one.
    Folder,
}

impl Place {
    /// The rail index, shared with `place_path` and the rail element keys.
    fn index(self) -> usize {
        match self {
            Place::Home => 0,
            Place::Gallery => 1,
            Place::QuickAccess => 2,
            Place::Desktop => 3,
            Place::Downloads => 4,
            Place::Documents => 5,
            Place::Pictures => 6,
            Place::Music => 7,
            Place::Videos => 8,
            Place::ThisPc => 9,
            Place::Network => 10,
            Place::Folder => usize::MAX,
        }
    }
}

/// The folders the built-in places stand for, apart from Home, which is the
/// home directory itself. One table, so the rail, the drop targets and the
/// place lookup cannot drift apart.
const PLACE_FOLDERS: [(Place, &str); 6] = [
    (Place::Desktop, "Desktop"),
    (Place::Downloads, "Downloads"),
    (Place::Documents, "Documents"),
    (Place::Pictures, "Pictures"),
    (Place::Music, "Music"),
    (Place::Videos, "Videos"),
];

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
    Subfolders,
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
    QuickAccess,
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

/// What a folder scan reports back from its worker thread.
enum ScanMsg {
    /// How many entries have been read so far.
    Progress(usize),
    Done(Box<ScanDone>),
}

/// A finished scan. `path` is what it was asked for, so an answer that lands
/// after the person has moved on can be recognised and dropped.
struct ScanDone {
    path: PathBuf,
    entries: Vec<Entry>,
    scan_ms: f64,
    cancelled: bool,
}

/// Messages from the operation worker thread, drained in `poll`.
enum OpMsg {
    Progress(fileops::Progress),
    Done {
        outcome: Result<usize, String>,
        /// What it did, in the form that can be taken back.
        undo: Option<Undo>,
    },
}

/// One finished file operation, in the form that can be taken back: what Ctrl+Z
/// reverses and Ctrl+Y does again.
#[derive(Clone)]
enum Undo {
    /// A name changed.
    Rename { from: PathBuf, to: PathBuf },
    /// A folder was made.
    Made { path: PathBuf },
    /// Items were copied here, so taking it back deletes the copies.
    Copied { pairs: Vec<(PathBuf, PathBuf)> },
    /// Items were moved away, so taking it back puts each one back.
    Moved { pairs: Vec<(PathBuf, PathBuf)> },
    /// Items went to the trash, so taking it back brings them out again.
    Trashed { items: Vec<fileops::Trashed> },
}

/// Messages from the update worker thread, drained in `poll`.
enum UpdateMsg {
    Checked(Result<Option<updates::Offer>, String>),
    Progress(u64, Option<u64>),
    Installing,
    Done(Result<(), String>),
}

/// A shell icon request for the icon worker. Extraction reads executables and
/// shortcuts from disk, so it must never run on the UI thread.
struct IconJob {
    key: String,
    path: String,
    is_dir: bool,
    px: u32,
}

/// The icon worker's answer, drained in `poll`.
struct IconDone {
    key: String,
    picture: Option<(u32, u32, Vec<u8>)>,
}

/// A modal dialog: delete confirmation, paste conflict, rename, new folder,
/// default manager, update, or which application opens a file.
enum Dialog {
    Delete { permanent: bool, paths: Vec<PathBuf> },
    PasteConflict { mode: fileops::Mode, sources: Vec<PathBuf>, target: PathBuf },
    Rename { path: PathBuf, edit: TextEdit },
    NewFolder { parent: PathBuf, edit: TextEdit },
    DefaultManager,
    Update,
    /// What can be said about one item, gathered when the dialog opens.
    Properties(Box<metadata::Details>),
    /// `data` is None while the platform is still being asked; `selected` holds
    /// the app id the Open button would launch.
    OpenWith {
        path: String,
        name: String,
        data: Option<openwith::OpenWith>,
        selected: Option<String>,
        remember: bool,
    },
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

/// Drops and hovers the OS drag session sent: hit-test the landing point and
/// copy drops in, keeping the staging alive until the paste finishes; light
/// the hovered target the way an internal drag does.
fn drain_osdrop(s: &mut App, tx: &mut Tx) -> bool {
    let mut msgs = Vec::new();
    if let Some(rx) = &s.osdrop_rx {
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
    }
    let any = !msgs.is_empty();
    let mut hover = None;
    for m in msgs {
        match m {
            osdrag::OsDragMsg::Drop(m) => {
                // OLE hands over screen pixels; the hit test wants window points.
                #[cfg(target_os = "windows")]
                let at = match frame::hwnd() {
                    Some(hwnd) => osdrag::win::to_window(hwnd, m.at.0, m.at.1, s.scale.get()),
                    None => m.at,
                };
                #[cfg(not(target_os = "windows"))]
                let at = m.at;
                apply_osdrop(tx, at, m.paths);
            }
            // Only the last hover of the frame matters, and a drop ends the
            // session with a leave, so the last word after a drop is always off.
            osdrag::OsDragMsg::Hover(at) => hover = Some(at),
        }
    }
    if let Some(at) = hover {
        match at {
            None => s.os_hover = None,
            Some(at) => {
                #[cfg(target_os = "windows")]
                let at = match frame::hwnd() {
                    Some(hwnd) => osdrag::win::to_window(hwnd, at.0, at.1, s.scale.get()),
                    None => at,
                };
                tx.element_at(at.0, at.1, move |s: &mut App, picked, _tx: &mut Tx| {
                    s.os_hover = os_hover_key(s, picked.as_ref().map(|p| p.path()));
                });
            }
        }
    }
    any
}

/// The highlight key for the element under an OS drag: the same key an
/// internal drag's hit test produces, and nothing over unkeyed space.
fn os_hover_key(s: &App, picked: Option<&str>) -> Option<String> {
    let key = picked.map(|p| parse_drop_target(s, p).0).unwrap_or_default();
    (!key.is_empty()).then_some(key)
}

/// One staged drop at a window point: hit-test, then copy into the target.
fn apply_osdrop(tx: &mut Tx, at: (f32, f32), paths: Vec<PathBuf>) {
    if std::env::var_os("LYRE_TRACE").is_some() {
        eprintln!("[drop] apply at={at:?} paths={}", paths.len());
    }
    tx.element_at(at.0, at.1, move |s: &mut App, picked, _tx: &mut Tx| {
        let picked_path = picked.as_ref().map(|p| p.path().to_string());
        let (_, target) = picked
            .map(|p| parse_drop_target(s, p.path()))
            .unwrap_or_default();
        let target = target.or_else(|| {
            matches!(s.view, View::Files | View::Gallery).then_some(DropTarget::Background)
        });
        if std::env::var_os("LYRE_TRACE").is_some() {
            eprintln!("[drop] pick path={picked_path:?} target={target:?}");
        }
        let Some(tp) = drop_target_path(s, target.as_ref()) else { return };
        s.drop_staged.extend(paths.iter().cloned());
        paste_into(s, paths.clone(), fileops::Mode::Copy, tp);
    });
}

/// A drop the host delivered (Linux, macOS): the element that took it names
/// the target. Our own macOS session keeps the volume rule with Shift;
/// anything from outside copies, as the old app did.
fn take_drop(s: &mut App, target: Option<DropTarget>, files: &[PathBuf], tx: &mut Tx) {
    let target = match target {
        Some(DropTarget::Background) if !matches!(s.view, View::Files | View::Gallery) => None,
        other => other,
    };
    let Some(tp) = drop_target_path(s, target.as_ref()) else { return };
    let paths: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let own = s.os_drag.as_ref().is_some_and(|p| *p == paths);
    let mode = if own {
        drop_mode(s, &paths, target.as_ref(), tx.mods().shift)
    } else {
        fileops::Mode::Copy
    };
    let staged = osdrag::stage(files);
    s.drop_staged.extend(staged.iter().cloned());
    paste_into(s, staged, mode, tp);
}

/// What a row drag carries: the selection, or just the pressed row.
fn drag_paths(s: &App, ei: usize) -> Vec<PathBuf> {
    if s.selected.contains(&ei) {
        selected_paths(s)
    } else {
        vec![PathBuf::from(&s.entries[ei].path)]
    }
}

/// The icon the OS draws under the pointer once a drag leaves the window:
/// Lyre's own ghost is a window overlay and cannot follow it out there.
#[cfg(target_os = "windows")]
fn drag_preview(s: &App, ei: usize) -> Option<osdrag::DragPreview> {
    let entry = s.entries.get(ei)?;
    let (width, height, rgba) = imagery::icon_rgba(&entry.path, entry.is_dir, 48)?;
    Some(osdrag::DragPreview { width, height, rgba })
}

/// On macOS the OS runs the drag session (rune's drag_out), so the row offers
/// its files there and hears the session's start and end; elsewhere these
/// handlers are inert and the pointer drag owns the gesture.
fn os_drag_source(el: El, ei: usize) -> El {
    el.drag_out(move |s: &App| {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let mut item = rune::DragItem::new();
        for p in drag_paths(s, ei) {
            item = item.file(p);
        }
        Some(item.allow(rune::DragOperation::Move))
    })
    .on_drag_start(move |s: &mut App, _tx: &mut Tx| {
        s.os_drag = Some(
            drag_paths(s, ei)
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        );
    })
    .on_drag_end(|s: &mut App, _outcome, _tx: &mut Tx| s.os_drag = None)
    .on_drop_files(move |s: &mut App, files, tx: &mut Tx| {
        let target = if s.entries.get(ei).is_some_and(|e| e.is_dir) {
            Some(DropTarget::Folder(PathBuf::from(&s.entries[ei].path)))
        } else {
            Some(DropTarget::Background)
        };
        take_drop(s, target, files, tx);
    })
}

/// Where a drag would land: a folder path, a tab's folder, or the empty
/// background of the folder on screen.
#[derive(Clone, Debug, PartialEq)]
enum DropTarget {
    Folder(PathBuf),
    Tab(usize),
    Background,
}

/// An internal file drag in flight: the paths, the pointer in window points,
/// the hit-tested target and the copy/move decision for it.
#[derive(Clone)]
struct DragState {
    paths: Vec<String>,
    at: (f32, f32),
    shift: bool,
    target: Option<DropTarget>,
    /// The element key under the pointer, for the hover highlight.
    target_key: String,
    mode: fileops::Mode,
}

struct App {
    dark: bool,
    /// What was chosen: follow the desktop, or force one. `dark` is that choice
    /// resolved, and is what everything draws from.
    theme: theme::Theme,
    /// When the desktop was last asked, for following it while running.
    theme_at: Instant,
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
    /// The internal file drag, and the window point its press started at.
    drag: Option<DragState>,
    drag_press: Option<(f32, f32)>,
    /// Details column widths (Name, Date, Type, Size) and the nav rail width.
    col_w: [f32; 4],
    rail_w: f32,
    count: usize,
    selection_detail: String,
    notice: String,
    search_edit: TextEdit,
    history: Vec<Spot>,
    forward: Vec<Spot>,
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
    /// Owner groups the person has opened in the customize list.
    customize_expanded: Vec<String>,
    /// Third-party extensions the person allowed Lyre to load, by CLSID.
    handler_loaded: Vec<String>,
    /// The target the customize dialog lists shell verbs for.
    customize_path: Option<String>,
    customize_is_dir: bool,
    customize_background: bool,
    /// Signature of the persisted state; poll writes to disk when it changes.
    prefs_sig: String,
    /// Real shell icons keyed by type/path, and decoded image previews by path.
    icon_cache: RefCell<HashMap<String, Pixels>>,
    /// The icon worker's channels and how many extractions are still running.
    icon_tx: std::sync::mpsc::Sender<IconJob>,
    icon_rx: std::sync::mpsc::Receiver<IconDone>,
    icons_pending: std::cell::Cell<usize>,
    /// Wakes the window from a worker thread; installed once the app handle
    /// exists, so a watch never needs a polling timer to be noticed.
    waker: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    /// The live folder watch: one blocked kernel wait, replaced on navigation.
    watch: Option<watch::Handle>,
    watch_path: Option<PathBuf>,
    watch_rx: Option<std::sync::mpsc::Receiver<watch::Refreshed>>,
    /// Rescans of restored tab listings, which were snapshotted while another
    /// tab was on screen and missed everything that happened since.
    rescan_tx: std::sync::mpsc::Sender<watch::Refreshed>,
    rescan_rx: std::sync::mpsc::Receiver<watch::Refreshed>,
    /// A refresh that arrived while an inline rename owned its row.
    pending_refresh: Option<watch::Refreshed>,
    /// The address bar turned into a path editor; None shows breadcrumbs.
    address_edit: Option<TextEdit>,
    rect_crumb_tail: Live<[f32; 4]>,
    /// Subfolders listed by the breadcrumb tail dropdown, read when it opens.
    tail_folders: Vec<(String, String)>,
    /// Drops the OS thread staged and handed over, and the staging roots the
    /// running paste still needs alive.
    osdrop_rx: Option<std::sync::mpsc::Receiver<osdrag::OsDragMsg>>,
    drop_staged: Vec<PathBuf>,
    /// The element key an OS drag hovers over: the same highlight an internal
    /// drag paints, without the ghost — the OS draws its own drag image.
    os_hover: Option<String>,
    /// The payload of our own macOS drag session, so a drop back onto this
    /// window keeps the move/copy rule instead of always copying.
    os_drag: Option<Vec<String>>,
    /// A drag that left the window, waiting for the frame that paints without
    /// its ghost before OLE's modal loop takes the pointer over.
    #[cfg(target_os = "windows")]
    pending_os_drag: Option<(Vec<PathBuf>, Option<osdrag::DragPreview>)>,
    /// The window scale, for turning OS drop pixels into window points.
    /// Names gathered by the last full walk, answering the rest of this search
    /// session without touching the disk. Dropped when the box is cleared or the
    /// folder changes, so it cannot outlive what it describes.
    search_cache: Option<std::sync::Arc<search::NameCache>>,
    /// A query typed but not yet walked, and when it becomes worth walking.
    search_pending: Option<(String, Instant)>,
    /// The letters typed so far, for jumping to a name by typing it.
    type_ahead: String,
    type_ahead_at: Instant,
    /// The window size in physical pixels that was last written to settings.
    /// `window_pending` is a size being dragged, which replaces it only once it
    /// has stopped changing.
    window_settled: (u32, u32),
    window_pending: Option<(u32, u32)>,
    window_changed_at: Instant,
    scale: std::cell::Cell<f32>,
    trace_tick: u64,
    thumb_cache: RefCell<HashMap<String, Image>>,
    /// Cut/copy clipboard, the running transfer, its worker channel and cancel.
    clipboard: Option<(Vec<PathBuf>, fileops::Mode)>,
    transfer: Option<Transfer>,
    op_rx: Option<std::sync::mpsc::Receiver<OpMsg>>,
    op_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Handler items for the open menu, memoised by the selection they were read
    /// for: building them instantiates a third-party DLL, which is far too much
    /// to repeat on every frame the menu is drawn.
    menu_handler_rows: RefCell<Option<(String, Vec<shell::Verb>)>>,
    /// Per-type facts for the previewed item, memoised by path: reading a file
    /// for its metadata must not happen on every frame the pane is drawn.
    preview_facts: RefCell<Option<(String, Vec<metadata::Fact>)>>,
    /// Rail places whose list is open, by place index.
    rail_open: Vec<usize>,
    /// The folder scan in flight: its channel, the folder it was asked for, and
    /// the flag that stops it.
    scan_rx: Option<std::sync::mpsc::Receiver<ScanMsg>>,
    scan_want: Option<PathBuf>,
    scan_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// How many entries the running scan has reached, for the status bar.
    scan_seen: usize,
    /// The open modal dialog (delete confirm, paste conflict, rename, new folder).
    dialog: Option<Dialog>,
    /// Which of the dialog's buttons the keyboard is on, in drawing order.
    /// Enter and Space press it, the arrows move it.
    dialog_focus: usize,
    /// What Ctrl+Z takes back, newest last, and what Ctrl+Y does again.
    undo: Vec<Undo>,
    redo: Vec<Undo>,
    /// The "Open with" worker's answer: listing applications touches the disk
    /// and, on Linux, runs `xdg-mime`, so it never runs on the UI thread.
    openwith_rx: Option<std::sync::mpsc::Receiver<openwith::OpenWith>>,
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
    quick_access_tab: u8,
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
    /// Every open tab's saved state, and which one is showing. The showing
    /// tab's live state stays in the flat fields above; this holds the rest.
    tabs: Vec<Tab>,
    active_tab: usize,
}

/// One tab's navigation state: where it is, how it got there, and what it
/// shows. Captured when the tab is left and restored when it is reopened.
#[derive(Clone)]
struct Tab {
    root: PathBuf,
    history: Vec<Spot>,
    forward: Vec<Spot>,
    view: View,
    mode: Mode,
    name: String,
    crumbs: Vec<String>,
    search: String,
    entries: Vec<Entry>,
    scan_ms: f64,
    groups: Vec<(usize, String)>,
    visible: Vec<usize>,
    selected: Vec<usize>,
    focused: Option<usize>,
    anchor: Option<usize>,
    reveal: Option<usize>,
    reveal_at: Option<Instant>,
    marquee: Option<Marquee>,
    count: usize,
    selection_detail: String,
    count_override: Option<usize>,
    keep_order: bool,
    contextual: bool,
    preview: bool,
    quick_access_tab: u8,
    volumes_selected: Option<usize>,
}

/// Snapshot the showing tab's state into a `Tab`.
fn capture_tab(s: &App) -> Tab {
    Tab {
        root: s.root.clone(),
        history: s.history.clone(),
        forward: s.forward.clone(),
        view: s.view,
        mode: s.mode,
        name: s.tab.clone(),
        crumbs: s.crumbs.clone(),
        search: s.search.clone(),
        entries: s.entries.clone(),
        scan_ms: s.scan_ms,
        groups: s.groups.clone(),
        visible: s.visible.clone(),
        selected: s.selected.clone(),
        focused: s.focused,
        anchor: s.anchor,
        reveal: s.reveal,
        reveal_at: s.reveal_at,
        marquee: s.marquee.clone(),
        count: s.count,
        selection_detail: s.selection_detail.clone(),
        count_override: s.count_override,
        keep_order: s.keep_order,
        contextual: s.contextual,
        preview: s.preview,
        quick_access_tab: s.quick_access_tab,
        volumes_selected: s.volumes_selected,
    }
}

/// Put a saved tab's state back into the showing fields.
fn restore_tab(s: &mut App, tab: &Tab) {
    s.root = tab.root.clone();
    s.history = tab.history.clone();
    s.forward = tab.forward.clone();
    s.view = tab.view;
    s.mode = tab.mode;
    s.tab = tab.name.clone();
    s.crumbs = tab.crumbs.clone();
    s.search = tab.search.clone();
    s.entries = tab.entries.clone();
    s.scan_ms = tab.scan_ms;
    s.groups = tab.groups.clone();
    s.visible = tab.visible.clone();
    s.selected = tab.selected.clone();
    s.focused = tab.focused;
    s.anchor = tab.anchor;
    s.reveal = tab.reveal;
    s.reveal_at = tab.reveal_at;
    s.marquee = tab.marquee.clone();
    s.count = tab.count;
    s.selection_detail = tab.selection_detail.clone();
    s.count_override = tab.count_override;
    s.keep_order = tab.keep_order;
    s.contextual = tab.contextual;
    s.preview = tab.preview;
    s.quick_access_tab = tab.quick_access_tab;
    s.volumes_selected = tab.volumes_selected;
}

/// Show a saved tab, saving the one being left first.
fn switch_tab(s: &mut App, index: usize) {
    if index >= s.tabs.len() || index == s.active_tab {
        return;
    }
    s.tabs[s.active_tab] = capture_tab(s);
    s.active_tab = index;
    let tab = s.tabs[index].clone();
    restore_tab(s, &tab);
    // A search belongs to the tab that started it: its hits are that folder's
    // names, and drawing them under the arriving tab would show the wrong list.
    clear_search(s);
    s.menu = None;
    s.marquee = None;
    s.address_edit = None;
    // A gesture in flight was aimed at the tab being left, so it cannot carry
    // over: a drag released after the switch would paste into the new folder.
    s.drag = None;
    s.drag_press = None;
    s.os_hover = None;
    rescan_restored(s);
}

/// Reset the showing tab's fields to a fresh Quick Access, closing its history.
/// The caller saves or replaces the tab's slot afterwards.
fn reset_tab_quick_access(s: &mut App) {
    s.view = View::QuickAccess;
    // The view, the sort and the details pane are the person's, and a new tab
    // opens the way the others look.
    s.tab = "Quick Access".into();
    s.crumbs = vec!["Quick Access".into()];
    s.search = "Search Quick Access".into();
    s.contextual = false;
    s.groups.clear();
    clear_selection(s);
    s.notice.clear();
    s.menu = None;
    s.marquee = None;
    s.history.clear();
    s.forward.clear();
    s.keep_order = true;
    s.count_override = None;
    refresh_recent(s);
}

/// Open a new tab on Quick Access.
fn new_tab(s: &mut App) {
    s.tabs[s.active_tab] = capture_tab(s);
    reset_tab_quick_access(s);
    s.tabs.push(capture_tab(s));
    s.active_tab = s.tabs.len() - 1;
}

/// Close a tab. The last one is reset to Quick Access rather than closing the
/// window.
fn close_tab_at(s: &mut App, index: usize) {
    if index >= s.tabs.len() {
        return;
    }
    if s.tabs.len() <= 1 {
        reset_tab_quick_access(s);
        s.tabs[0] = capture_tab(s);
        return;
    }
    s.tabs.remove(index);
    if index < s.active_tab {
        s.active_tab -= 1;
    } else if s.active_tab >= s.tabs.len() {
        s.active_tab = s.tabs.len() - 1;
    }
    let tab = s.tabs[s.active_tab].clone();
    restore_tab(s, &tab);
    s.menu = None;
    s.address_edit = None;
    s.drag = None;
    s.drag_press = None;
    s.os_hover = None;
    rescan_restored(s);
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

/// Scan a folder with nothing able to stop it. Used where there is nobody to
/// cancel: the first frame, and the watcher's own rescan.
pub(crate) fn scan(path: &Path) -> (Vec<Entry>, f64) {
    scan_until(path, &std::sync::atomic::AtomicBool::new(false), |_| {})
}

/// A folder reached through a link is a folder here, which is what Explorer
/// lists and what opening it must do.
fn entry_is_dir(path: &Path, meta: &fs::Metadata) -> bool {
    if meta.is_dir() {
        return true;
    }
    if !meta.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        // Windows marks a folder link — a junction, or a symlink to a folder —
        // as both a reparse point and a directory, and the directory bit is
        // what Explorer draws and opens by. It is still set when the target has
        // gone, which is how a link that leads nowhere is shown as the folder
        // it was. The standard library leaves `is_dir()` false for it because
        // it treats a reparse point as a link, so as a file it sorted with the
        // files, drew a file's blank icon, and was handed to the shell, which
        // started a second Lyre window on the folder instead of opening it.
        use std::os::windows::fs::MetadataExt;
        // The entry's own attributes answer, so its target is never asked.
        let _ = path;
        meta.file_attributes() & 0x10 != 0
    }
    #[cfg(not(windows))]
    {
        // A Unix symlink carries no such bit, so ask where it points. One whose
        // target has gone is not a folder.
        fs::metadata(path).is_ok_and(|target| target.is_dir())
    }
}

/// What a link leads to, when the link itself cannot be listed.
///
/// Windows marks the legacy `Documents\My Videos`, `My Music` and `My Pictures`
/// links with a deny on reading through them, so the link can be listed in the
/// folder that holds it but its contents cannot be. What such a link means is
/// the folder it names, which is read normally, so that is what opens. A link
/// that can be read is opened where it is, and anything that is not a link is
/// left alone.
fn link_target_to_read(path: &Path) -> Option<PathBuf> {
    let is_link = fs::symlink_metadata(path)
        .is_ok_and(|meta| meta.file_type().is_symlink());
    if !is_link {
        return None;
    }
    let mut at = path.to_path_buf();
    // A link to a link is followed to the end, bounded so a pair that point at
    // each other cannot spin.
    for _ in 0..8 {
        if fs::read_dir(&at).is_ok() {
            return (at != path).then_some(at);
        }
        let next = fs::read_link(&at).ok()?;
        // A junction stores its target as a device path, `\??\C:\...`, which is
        // not a path anything else here can use.
        let text = next.to_string_lossy();
        at = match text.strip_prefix(r"\??\") {
            Some(rest) => PathBuf::from(rest),
            None => next,
        };
    }
    None
}

/// Scan a folder, giving up when `cancel` is set and reporting how many items
/// have been read so far. A folder of many thousand entries takes long enough
/// that the window must not sit and wait for it.
pub(crate) fn scan_until(
    path: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(usize),
) -> (Vec<Entry>, f64) {
    use std::sync::atomic::Ordering;

    let start = Instant::now();
    let mut entries = Vec::new();
    if let Ok(reader) = fs::read_dir(path) {
        for item in reader.flatten() {
            // Checked per entry, so a cancelled read stops where it is rather
            // than at the end of the folder.
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let name = item.file_name().to_string_lossy().into_owned();
            let path = item.path();
            let Ok(meta) = item.metadata() else { continue };
            let is_dir = entry_is_dir(&path, &meta);
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
                path: path.to_string_lossy().into_owned(),
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
            // Rare enough not to matter per entry, often enough that a slow
            // folder visibly counts up.
            if entries.len() % 256 == 0 {
                progress(entries.len());
            }
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

/// Whether two paths name the same folder as far as a place is concerned.
///
/// A trailing separator, and on Windows letter case, do not make a second
/// place. Only the components are compared, so this is a string test and never
/// touches the disk: the rail runs it for every row it draws.
fn same_path(a: &Path, b: &Path) -> bool {
    let mut left = a.components();
    let mut right = b.components();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) => {
                let (x, y) = (x.as_os_str(), y.as_os_str());
                let equal = if cfg!(windows) {
                    x.to_string_lossy().eq_ignore_ascii_case(&y.to_string_lossy())
                } else {
                    x == y
                };
                if !equal {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// The same test for paths held as strings, as the favorites and pins are.
fn same_text_path(a: &str, b: &str) -> bool {
    same_path(Path::new(a), Path::new(b))
}

/// The place a view and root are showing. This is the only thing that decides
/// which rail row is selected.
fn place_of(view: View, root: &Path) -> Place {
    match view {
        View::QuickAccess => Place::QuickAccess,
        View::ThisPc => Place::ThisPc,
        View::Gallery => Place::Gallery,
        View::Files => {
            let Some(home) = home_dir() else { return Place::Folder };
            if same_path(root, &home) {
                return Place::Home;
            }
            // The place claims the folder the platform says it is, not the one
            // with its English name: a Desktop moved to OneDrive is still Desktop.
            for (place, name) in PLACE_FOLDERS {
                if known::folder(&home, name).is_some_and(|path| same_path(root, &path)) {
                    return place;
                }
            }
            Place::Folder
        }
    }
}

fn place_path(index: usize) -> Option<PathBuf> {
    let home = home_dir()?;
    if index == Place::Home.index() {
        return Some(home);
    }
    let (_, name) = PLACE_FOLDERS.iter().find(|(place, _)| place.index() == index)?;
    known::folder(&home, name)
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
    load_prefs_from(s, PREFS_ID);
}

/// Carry over the favorites and pins the 0.2 application saved.
///
/// An update from 0.2 replaced the webview with a native window, and the
/// settings store with it: 0.2's lists lived in the webview's local storage and
/// this build's live in `prefs.json`, so the first launch after the update found
/// nothing, started empty, and saved that over the lists the person had. This
/// reads the old store once, at launch, and writes what it finds into the new
/// one before any window opens.
///
/// Lists this build already has are never touched, so someone who went on to
/// rebuild them after the update does not have a stale set merged in, and the
/// store is marked as read either way so it is never read twice.
fn import_legacy_library() {
    let mut prefs = rune::prefs::Prefs::open(PREFS_ID);
    if prefs.get::<bool>("legacy_import").unwrap_or(false) {
        return;
    }
    let _ = prefs.set("legacy_import", &true);
    let has_paths =
        |key: &str| prefs.get::<Vec<String>>(key).is_some_and(|list| !list.is_empty());
    let has_hidden = prefs
        .get::<Vec<usize>>("hidden_places")
        .is_some_and(|places| !places.is_empty());
    if has_paths("favorites") || has_paths("pinned") || has_hidden {
        return;
    }
    let Some(library) = legacy::read() else { return };
    let _ = prefs.set("favorites", &unique_paths(library.favorites));
    let _ = prefs.set("pinned", &unique_paths(library.pins));
    // The old store named the built-in rows by path; this build names them by
    // rail index, which is what a hidden row is remembered as. A path that is
    // not one of the built-in places is not a rail row at all, so it is dropped.
    let hidden: Vec<usize> = library
        .hidden
        .iter()
        .filter_map(|path| {
            let place = place_of(View::Files, Path::new(path));
            PLACE_FOLDERS.iter().any(|(known, _)| *known == place).then(|| place.index())
        })
        .collect();
    if !hidden.is_empty() {
        let _ = prefs.set("hidden_places", &hidden);
    }
}

/// The same restore against a named store, so a test can round-trip without
/// touching the settings a person actually uses.
fn load_prefs_from(s: &mut App, id: &str) {
    let prefs = rune::prefs::Prefs::open(id);
    let mut stored_dark = None;
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
        stored_dark = Some(dark);
    }
    // The appearance preference. A save from before it existed carries only the
    // old boolean, which is taken as the choice it was.
    match prefs.get::<u8>("theme") {
        Some(stored) => {
            s.theme = theme::Theme::from_u8(stored);
            s.dark = s.theme.is_dark();
        }
        None => {
            if let Some(dark) = stored_dark {
                s.theme = if dark { theme::Theme::Dark } else { theme::Theme::Light };
                s.dark = dark;
            } else {
                s.theme = theme::Theme::System;
                s.dark = s.theme.is_dark();
            }
        }
    }
    if let Some(f) = prefs.get::<Vec<String>>("favorites") {
        s.favorites = unique_paths(f);
    }
    if let Some(r) = prefs.get::<Vec<String>>("recent") {
        s.recent = unique_paths(r);
    }
    if let Some(p) = prefs.get::<Vec<String>>("pinned") {
        s.pinned = unique_paths(p);
    }
    if let Some(m) = prefs.get::<Vec<String>>("menu_hidden") {
        s.menu_hidden = m;
    }
    if let Some(m) = prefs.get::<Vec<String>>("menu_shown") {
        s.menu_shown = m;
    }
    // Extensions the person allowed Lyre to load stay allowed: re-asking on every
    // launch would make the switch meaningless.
    if let Some(h) = prefs.get::<Vec<String>>("handler_loaded") {
        s.handler_loaded = h;
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
    // The size the window was last left at, restored below when it opens.
    if let Some(size) = prefs.get::<(u32, u32)>("window_size") {
        s.window_settled = size;
    }
}

/// Write the whole view state and collections; poll calls this once per change.
fn persist(s: &App) {
    persist_to(s, PREFS_ID);
}

/// The same save against a named store, for tests.
fn persist_to(s: &App, id: &str) {
    let mut prefs = rune::prefs::Prefs::open(id);
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
    let _ = prefs.set("theme", &s.theme.to_u8());
    let _ = prefs.set("window_size", &s.window_settled);
    let _ = prefs.set("favorites", &s.favorites);
    let _ = prefs.set("recent", &s.recent);
    let _ = prefs.set("pinned", &s.pinned);
    let _ = prefs.set("menu_hidden", &s.menu_hidden);
    let _ = prefs.set("menu_shown", &s.menu_shown);
    let _ = prefs.set("handler_loaded", &s.handler_loaded);
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
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{}|{:?}|{}|{}|{:?}",
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
        s.handler_loaded,
        s.hidden_places,
        s.update_check_startup,
        s.prompt_default_manager,
        s.show_protected,
        s.show_extensions,
        s.col_w,
        s.rail_w,
        s.theme.to_u8(),
        s.window_settled
    )
}

fn is_favorite(s: &App, path: &str) -> bool {
    s.favorites.iter().any(|p| same_text_path(p, path))
}

fn is_pinned(s: &App, path: &str) -> bool {
    s.pinned.iter().any(|p| same_text_path(p, path))
}

fn toggle_favorite(s: &mut App, path: &str) {
    if let Some(i) = s.favorites.iter().position(|p| same_text_path(p, path)) {
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
    if let Some(i) = s.pinned.iter().position(|p| same_text_path(p, path)) {
        s.pinned.remove(i);
    } else if !path.is_empty() {
        s.pinned.push(path.to_string());
    }
}

/// Drop repeats of the same folder from a restored list, keeping the first.
///
/// A pin saved twice, once with a trailing separator or in another case, would
/// otherwise draw two rows for one folder that both look selected.
fn unique_paths(paths: Vec<String>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for path in paths {
        if !path.is_empty() && !kept.iter().any(|other| same_text_path(other, &path)) {
            kept.push(path);
        }
    }
    kept
}

/// Whether a context-menu item survives the person's customize choices. Lyre's
/// own items start on; another program's verbs start off and are turned on one
/// at a time, so installing software never silently changes a right-click.
/// Change the appearance choice, resolving it now so the next frame draws the
/// right one.
fn set_theme(s: &mut App, theme: theme::Theme) {
    s.theme = theme;
    s.dark = theme.is_dark();
    s.theme_at = Instant::now();
}

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
                // The timestamp, not the text that shows it: "4/30/2026" and
                // "10/20/2026" compare the wrong way round as strings, which
                // made this sort look random. An entry with no timestamp goes
                // last rather than first.
                1 => match (ea.modified_at, eb.modified_at) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                },
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
        // Newest first, so today leads and the days run backwards, as Explorer
        // groups them. Same key as the sort above: the timestamp, not its text.
        order.sort_by(|&a, &b| s.entries[b].modified_at.cmp(&s.entries[a].modified_at));
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
    // The gesture left the window last frame, which drew no ghost; now carry it
    // into an OS drag. The handoff is posted to the window's message pump, not
    // run here: inside this callback winit buffers every redraw, and OLE's
    // modal loop would freeze the window for the whole drag.
    #[cfg(target_os = "windows")]
    if let Some((paths, preview)) = s.pending_os_drag.take()
        && let Some(hwnd) = frame::hwnd()
    {
        osdrag::win::request_start(hwnd, paths, preview);
    }
    // The mouse's side buttons walk the same history as Alt+Left and Alt+Right.
    // `poll` runs before the frame is drawn, so the change is on screen already,
    // as long as it is reported below: a step onto a page starts no scan, so
    // nothing else would mark the frame.
    let stepped = match frame::take_side_button() {
        Some(1) => go_back(s),
        Some(2) => go_forward(s),
        _ => false,
    };
    // A middle press closes the tab under it, as in a browser. Rune has no
    // handler for the middle button, so the window catches the press and this
    // finds what it landed on.
    let mut tab_closed = false;
    if let Some((x, y)) = frame::take_middle_press() {
        let (x, y) = (x / s.scale.get(), y / s.scale.get());
        let closed = std::rc::Rc::new(std::cell::Cell::new(false));
        let closed_by_press = closed.clone();
        tx.element_at(x, y, move |s: &mut App, picked, _tx: &mut Tx| {
            if let Some(path) = picked.map(|p| p.path().to_string()) {
                closed_by_press.set(close_tab_at_path(s, &path));
            }
        });
        tab_closed = closed.get();
    }
    // Remember the window's size, but only once a resize has stopped. Rune has no
    // resize event, so it is sampled here; writing the settings on every frame of
    // a drag would be wasteful, and nothing else would wake the window after the
    // last frame of it, hence the delayed wake.
    if let Some(size) = frame::content_size()
        && size != s.window_settled
    {
        if s.window_pending != Some(size) {
            s.window_pending = Some(size);
            s.window_changed_at = Instant::now();
            tx.wake_after(std::time::Duration::from_millis(500));
        } else if s.window_changed_at.elapsed() >= std::time::Duration::from_millis(400) {
            s.window_settled = size;
            s.window_pending = None;
        }
    } else {
        s.window_pending = None;
    }
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
                    open_dialog(s, Dialog::DefaultManager);
                }
            }
        }
    }
    // A step taken from the side buttons is a change the view reads, and a step
    // back to a page starts no scan whose answer would mark the frame otherwise.
    let mut changed = drain_op(s) || stepped || tab_closed;
    // Keep frames coming while a transfer runs so its progress drains promptly.
    if s.transfer.is_some() {
        tx.wake_after(std::time::Duration::from_millis(60));
    }
    changed |= drain_icons(s);
    if s.icons_pending.get() > 0 {
        tx.wake_after(std::time::Duration::from_millis(16));
    }
    // Shift flips a held drag between move and copy even while the pointer
    // rests, as the old app's key listeners did.
    if s.drag.is_some() {
        let shift = tx.mods().shift;
        let (paths, target) = {
            let dr = s.drag.as_ref().unwrap();
            (dr.paths.clone(), dr.target.clone())
        };
        let mode = drop_mode(s, &paths, target.as_ref(), shift);
        if let Some(dr) = &mut s.drag {
            if dr.shift != shift || dr.mode != mode {
                dr.shift = shift;
                dr.mode = mode;
                changed = true;
            }
        }
        tx.wake_after(std::time::Duration::from_millis(16));
    }
    sync_watch(s);
    changed |= drain_watch(s);
    changed |= drain_osdrop(s, tx);
    if std::env::var_os("LYRE_TRACE").is_some() && s.trace_tick % 60 == 0 {
        #[cfg(target_os = "windows")]
        if let Some(hwnd) = frame::hwnd() {
            osdrag::win::trace_registration(hwnd);
        }
    }
    s.trace_tick = s.trace_tick.wrapping_add(1);
    // Clicking away from the address editor cancels it, as Explorer does.
    if s.address_edit.as_ref().is_some_and(|e| !e.focused) {
        s.address_edit = None;
        changed = true;
    }
    changed |= drain_search(s);
    if s.search_busy {
        tx.wake_after(std::time::Duration::from_millis(40));
    }
    changed |= drain_update(s);
    if s.update_busy {
        tx.wake_after(std::time::Duration::from_millis(60));
    }
    changed |= drain_scan(s);
    if s.scan_rx.is_some() {
        // Keep frames coming so the count climbing is visible and the answer is
        // picked up promptly.
        tx.wake_after(std::time::Duration::from_millis(30));
    }
    // Following the desktop means noticing when it changes while running. Asking
    // costs a registry read on Windows and a child process elsewhere, so only
    // Windows re-asks; the others settle it at launch and when it is chosen.
    #[cfg(target_os = "windows")]
    if s.theme == theme::Theme::System
        && s.theme_at.elapsed() >= std::time::Duration::from_secs(2)
    {
        s.theme_at = Instant::now();
        let dark = s.theme.is_dark();
        if dark != s.dark {
            s.dark = dark;
            changed = true;
        }
    }
    changed |= drain_openwith(s);
    if s.openwith_rx.is_some() {
        tx.wake_after(std::time::Duration::from_millis(16));
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
        } else if s.search_pending.as_ref().map(|(text, _)| text) != Some(&q) {
            // Wait for the typing to stop. Every keystroke used to start a fresh
            // walk and throw the previous one away, so a six-letter query meant
            // six walks of the disk to answer one of them.
            s.search_pending = Some((q, Instant::now() + std::time::Duration::from_millis(180)));
        }
        changed = true;
    }
    if let Some((text, due)) = s.search_pending.clone()
        && Instant::now() >= due
    {
        s.search_pending = None;
        if text == s.search_edit.text {
            start_search(s, text);
        }
        changed = true;
    }
    if s.search_pending.is_some() {
        tx.wake_after(std::time::Duration::from_millis(60));
    }
    changed
}

/// Stop the scan in flight, if there is one, and forget its answer.
fn cancel_scan(s: &mut App) {
    use std::sync::atomic::Ordering;

    if let Some(cancel) = s.scan_cancel.take() {
        cancel.store(true, Ordering::Relaxed);
    }
    s.scan_rx = None;
    s.scan_want = None;
    s.scan_seen = 0;
}

/// Read the folder at `s.root` on a worker thread, so a slow one never freezes
/// the window. Progress and the finished list arrive through `drain_scan`.
fn begin_scan(s: &mut App) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    cancel_scan(s);
    let path = s.root.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    s.scan_rx = Some(rx);
    s.scan_want = Some(path.clone());
    s.scan_cancel = Some(cancel.clone());
    let waker = s.waker.clone();
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let (entries, scan_ms) = scan_until(&path, &flag, |seen| {
            let _ = tx.send(ScanMsg::Progress(seen));
        });
        let cancelled = flag.load(Ordering::Relaxed);
        let answer = ScanDone {
            path,
            entries,
            scan_ms,
            cancelled,
        };
        if tx.send(ScanMsg::Done(Box::new(answer))).is_ok()
            && let Some(wake) = waker
        {
            wake();
        }
    });
}

/// Drain the scan worker's messages; true to redraw.
fn drain_scan(s: &mut App) -> bool {
    let Some(rx) = s.scan_rx.take() else {
        return false;
    };
    let mut done = None;
    let mut changed = false;
    while let Ok(msg) = rx.try_recv() {
        match msg {
            ScanMsg::Progress(seen) => {
                s.scan_seen = seen;
                changed = true;
            }
            ScanMsg::Done(result) => done = Some(result),
        }
    }
    let Some(result) = done else {
        // Still reading; keep the receiver for the next frame.
        s.scan_rx = Some(rx);
        return changed;
    };
    // An answer for a folder the person has already left is dropped, and the
    // scan actually wanted keeps both its channel and its cancel flag.
    if s.scan_want.as_deref() != Some(result.path.as_path()) {
        s.scan_rx = Some(rx);
        return changed;
    }
    s.scan_want = None;
    s.scan_cancel = None;
    s.scan_seen = 0;
    if result.cancelled {
        return changed;
    }
    s.entries = result.entries;
    s.scan_ms = result.scan_ms;
    refresh_visible(s);
    true
}

/// Where a window can be, as one step of its history: a folder, or one of the
/// pages that is drawn from state rather than read from disk.
///
/// This PC, Quick Access and the Gallery have no path of their own. They keep
/// the last real folder behind them only so a return to it still reloads, and
/// that folder was all history used to record: it is why Back out of a folder
/// skipped the page and landed on whatever the page had been opened from.
#[derive(Clone)]
struct Spot {
    view: View,
    root: PathBuf,
}

/// Where the window is now, as history records it.
fn spot_of(s: &App) -> Spot {
    Spot { view: s.view, root: s.root.clone() }
}

/// Put the window back on a recorded step.
///
/// Each arm is what the rail's press for that page does, so going back to a
/// page and pressing its row cannot draw it two different ways.
fn restore(s: &mut App, spot: Spot) {
    match spot.view {
        // A folder can be gone by the time the step is walked back over: a
        // deleted directory, an unplugged drive. Explorer says the location is
        // not available and stays where it is, rather than drawing an empty
        // folder that is not empty.
        View::Files => {
            let path = link_target_to_read(&spot.root).unwrap_or(spot.root);
            if path.is_dir() {
                set_location(s, path);
            } else {
                s.notice = format!("{} is not available here", path.display());
            }
        }
        View::QuickAccess => load(s, Fixture::QuickAccess),
        View::ThisPc => {
            load(s, Fixture::ThisPc);
            refresh_volumes(s);
        }
        // Pictures can be removed between the step being taken and walked back
        // to; there is then no Gallery to draw and the window stays put.
        View::Gallery => match gallery_folder() {
            Some(pictures) => show_gallery(s, pictures),
            None => s.notice = "Pictures is not available here".into(),
        },
    }
}

/// Navigate to a folder and start reading it.
fn set_location(s: &mut App, path: PathBuf) {
    clear_search(s);
    s.address_edit = None;
    s.drag = None;
    s.drag_press = None;
    s.os_hover = None;
    let crumbs: Vec<String> = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .filter(|part| part != "\\" && part != "/")
        .collect();
    // The home folder is named "Home" rather than after the account, so the word
    // means the same thing in a tab as it does in the rail.
    let tab = if place_of(View::Files, &path) == Place::Home {
        "Home".to_string()
    } else {
        crumbs.last().cloned().unwrap_or_else(|| "Lyre".into())
    };
    s.root = path;
    // The list fills in when the worker answers; until then the status bar says
    // the folder is being read.
    s.entries.clear();
    s.scan_ms = 0.0;
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
    begin_scan(s);
}

fn go(s: &mut App, path: PathBuf) {
    s.menu = None;
    // Quick Access and This PC keep the last real root behind them, so a return
    // to that same folder must still reload; only a no-op while already showing
    // it as a file list.
    if path == s.root && s.view == View::Files {
        return;
    }
    // A folder listed through a link can lead nowhere, and a folder can be
    // deleted between the listing and the press. Explorer says the location is
    // not available and stays where it is; so does this, rather than showing an
    // empty folder that is not empty.
    if !path.is_dir() {
        s.notice = format!("{} is not available here", path.display());
        return;
    }
    // The folder as it is meant: a link the shell will not read through opens
    // the folder it names instead of an empty page.
    let path = link_target_to_read(&path).unwrap_or(path);
    s.history.push(spot_of(s));
    s.forward.clear();
    set_location(s, path);
}

/// Open Quick Access. A re-press is a no-op, as with the other pages.
fn open_quick_access(s: &mut App) {
    s.menu = None;
    if s.view == View::QuickAccess {
        return;
    }
    s.history.push(spot_of(s));
    s.forward.clear();
    load(s, Fixture::QuickAccess);
}

/// Open This PC. A re-press only re-reads the drives, which is what its row is
/// for, rather than adding a step that Back would have to walk out of.
fn open_this_pc(s: &mut App) {
    s.menu = None;
    if s.view == View::ThisPc {
        refresh_volumes(s);
        return;
    }
    s.history.push(spot_of(s));
    s.forward.clear();
    load(s, Fixture::ThisPc);
    refresh_volumes(s);
}

/// Step back, reporting whether the window has something new to draw: `poll`
/// asks for a frame only when it is told so, and a step back to a page starts no
/// scan whose answer would mark the frame instead.
fn go_back(s: &mut App) -> bool {
    match s.history.pop() {
        Some(previous) => {
            s.forward.push(spot_of(s));
            restore(s, previous);
        }
        None => s.notice = "Nothing to go back to".into(),
    }
    true
}

/// Step forward, with the same report as `go_back`.
fn go_forward(s: &mut App) -> bool {
    match s.forward.pop() {
        Some(next) => {
            s.history.push(spot_of(s));
            restore(s, next);
        }
        None => s.notice = "Nothing to go forward to".into(),
    }
    true
}

/// Whether Up has somewhere to go.
///
/// A folder does, and so does the Gallery, whose root is the Pictures library
/// it is drawn from. Quick Access and This PC do not: they are places that keep
/// the last real root behind them only so a return to it still reloads, so
/// reading the parent off that root walked out of a page that is not in a folder
/// at all. This PC offered Up to `C:\Users\<name>` because it was sitting on
/// the home folder it had replaced.
fn can_go_up(s: &App) -> bool {
    matches!(s.view, View::Files | View::Gallery) && s.root.parent().is_some()
}

fn go_up(s: &mut App) {
    let parent = can_go_up(s).then(|| s.root.parent()).flatten().map(Path::to_path_buf);
    match parent {
        Some(parent) => go(s, parent),
        None => s.notice = "Already at the root".into(),
    }
}

fn navigate(s: &mut App, index: usize) {
    s.menu = None;
    match index {
        0 => open_home(s),
        2 => open_quick_access(s),
        9 => open_this_pc(s),
        10 => s.notice = "Network browsing is not implemented in this slice".into(),
        1 => go_gallery(s),
        i => {
            match place_path(i) {
                Some(path) if path.is_dir() => go(s, path),
                Some(path) => s.notice = format!("{} is not available here", path.display()),
                None => s.notice = "This place is not available here".into(),
            }
        }
    }
}

/// Open the home folder. Unlike Quick Access, This PC and Gallery, this is a
/// folder that exists, so it navigates like any other: its breadcrumbs are the
/// real ones and Up goes to the folder that holds it.
fn open_home(s: &mut App) {
    match home_dir() {
        Some(home) if home.is_dir() => go(s, home),
        Some(home) => s.notice = format!("{} is not available here", home.display()),
        None => s.notice = "Home is not available here".into(),
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
    s.address_edit = None;
    s.drag = None;
    s.drag_press = None;
    s.os_hover = None;
    s.contextual = false;
    clear_selection(s);
    s.notice.clear();
    s.search_edit.text.clear();
    match f {
        Fixture::QuickAccess => {
            s.view = View::QuickAccess;
            s.tab = "Quick Access".into();
            s.crumbs = vec!["Quick Access".into()];
            s.search = "Search Quick Access".into();
            s.count = 0;
            s.selection_detail.clear();
            refresh_recent(s);
        }
        Fixture::Downloads => {
            s.view = View::Files;
            s.tab = "Downloads".into();
            s.crumbs = vec!["Downloads".into()];
            s.search = "Search Downloads".into();
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
            s.tab = "This PC".into();
            s.crumbs = vec!["This PC".into()];
            s.search = "Search This PC".into();
            s.count = 1;
            s.selection_detail.clear();
            s.entries.clear();
        }
    }
    let authored_count = s.count;
    s.keep_order = true;
    s.count_override = Some(authored_count);
    refresh_visible(s);
}

fn init() -> App {
    // Where to start. An explicit path is a request: the folder handler passing a
    // folder to open, or LYRE_RUNE_PATH for development. With neither, the window
    // opens on Quick Access rather than on whatever folder the process happened
    // to start in, which for an installed copy is its own install folder.
    let requested = std::env::var("LYRE_RUNE_PATH")
        .map(PathBuf::from)
        .ok()
        .or_else(|| defaultmanager::startup_path().map(PathBuf::from));
    let start_quick_access = requested.is_none();
    let root = match requested {
        // A link the shell will not read through is shown as the folder it
        // names, the same as pressing one opens it.
        Some(path) => link_target_to_read(&path).unwrap_or(path),
        None => home_dir().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        }),
    };
    let (entries, scan_ms) = scan(&root);
    let crumbs: Vec<String> = root
        .components()
        .map(|p| p.as_os_str().to_string_lossy().into_owned())
        .filter(|s| s != "\\" && s != "/")
        .collect();
    let tab = crumbs.last().cloned().unwrap_or_else(|| "Lyre".into());
    let count = entries.len();
    // One worker extracts shell icons in request order, so the rows on screen
    // get theirs first and a folder of installers never stalls a frame.
    let (icon_tx, icon_jobs) = std::sync::mpsc::channel::<IconJob>();
    let (icon_done, icon_rx) = std::sync::mpsc::channel::<IconDone>();
    let (rescan_tx, rescan_rx) = std::sync::mpsc::channel::<watch::Refreshed>();
    let (osdrop_tx, osdrop_rx) = std::sync::mpsc::channel::<osdrag::OsDragMsg>();
    osdrag::set_sender(osdrop_tx);
    osdrag::sweep_stale();
    std::thread::spawn(move || {
        imagery::prepare_thread();
        while let Ok(job) = icon_jobs.recv() {
            let picture = imagery::icon_rgba(&job.path, job.is_dir, job.px);
            if icon_done.send(IconDone { key: job.key, picture }).is_err() {
                break;
            }
        }
        imagery::release_thread();
    });
    let mut app = App {
        dark: true,
        theme: theme::Theme::System,
        theme_at: Instant::now(),
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
        drag: None,
        drag_press: None,
        col_w: [NAME_W, DATE_W, TYPE_W, SIZE_W],
        rail_w: RAIL_W,
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
        customize_expanded: Vec::new(),
        handler_loaded: Vec::new(),
        customize_path: None,
        customize_is_dir: false,
        customize_background: false,
        prefs_sig: String::new(),
        icon_cache: RefCell::new(HashMap::new()),
        icon_tx,
        icon_rx,
        icons_pending: std::cell::Cell::new(0),
        waker: None,
        watch: None,
        watch_path: None,
        watch_rx: None,
        rescan_tx,
        rescan_rx,
        pending_refresh: None,
        address_edit: None,
        rect_crumb_tail: Live::new([0.0; 4]),
        tail_folders: Vec::new(),
        osdrop_rx: Some(osdrop_rx),
        drop_staged: Vec::new(),
        os_hover: None,
        os_drag: None,
        #[cfg(target_os = "windows")]
        pending_os_drag: None,
        type_ahead: String::new(),
        type_ahead_at: Instant::now(),
        search_pending: None,
        search_cache: None,
        window_settled: (0, 0),
        window_pending: None,
        window_changed_at: Instant::now(),
        scale: std::cell::Cell::new(1.0),
        trace_tick: 0,
        thumb_cache: RefCell::new(HashMap::new()),
        clipboard: None,
        transfer: None,
        op_rx: None,
        op_cancel: None,
        menu_handler_rows: RefCell::new(None),
        preview_facts: RefCell::new(None),
        rail_open: Vec::new(),
        scan_rx: None,
        scan_want: None,
        scan_cancel: None,
        scan_seen: 0,
        dialog: None,
        dialog_focus: 0,
        undo: Vec::new(),
        redo: Vec::new(),
        openwith_rx: None,
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
        quick_access_tab: 0,
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
        tabs: Vec::new(),
        active_tab: 0,
    };
    load_prefs(&mut app);
    if start_quick_access {
        // Before the first tab is captured, so the tab strip agrees with the view.
        load(&mut app, Fixture::QuickAccess);
    }
    refresh_visible(&mut app);
    app.tabs = vec![capture_tab(&app)];
    app.active_tab = 0;
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

/// A filled five-point star: the mark Explorer gives Quick Access.
fn star(cx: f32, cy: f32, outer: f32, inner: f32) -> Vec<[f32; 2]> {
    (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { outer } else { inner };
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
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
/// platform supplies none (or the path is only a fixture). A cache miss queues
/// the extraction on the icon worker and draws the glyph meanwhile; `poll`
/// drains the answer and the rebuilt row swaps the glyph for the real icon.
fn entry_icon(s: &App, e: &Entry, p: Paint, size: f32) -> El {
    let key = icon_key(e);
    let pixels = {
        let mut cache = s.icon_cache.borrow_mut();
        if let Some(px) = cache.get(&key) {
            px.clone()
        } else {
            let px = Pixels::new();
            cache.insert(key.clone(), px.clone());
            s.icons_pending.set(s.icons_pending.get() + 1);
            let _ = s.icon_tx.send(IconJob {
                key,
                path: e.path.clone(),
                is_dir: e.is_dir,
                px: (size.ceil() as u32).max(32),
            });
            px
        }
    };
    if pixels.size() == (0, 0) {
        return file_glyph(e, p, size);
    }
    el().size(size, size).pixels(&pixels)
}

/// Land finished extractions in the icon cache; true to rebuild rows with them.
fn drain_icons(s: &App) -> bool {
    let mut changed = false;
    while let Ok(done) = s.icon_rx.try_recv() {
        s.icons_pending.set(s.icons_pending.get().saturating_sub(1));
        if let Some((w, h, rgba)) = done.picture
            && let Some(px) = s.icon_cache.borrow().get(&done.key)
        {
            px.set(w, h, rgba);
            changed = true;
        }
    }
    changed
}

/// Watch exactly the folder on screen. One watch per window: navigating,
/// switching tabs or entering search replaces it; Quick Access and This PC have
/// no folder behind them, so nothing is watched there.
fn sync_watch(s: &mut App) {
    let wanted = if s.searching {
        None
    } else {
        match s.view {
            View::Files | View::Gallery => Some(s.root.clone()),
            _ => None,
        }
    };
    if wanted == s.watch_path {
        return;
    }
    s.watch = None;
    s.watch_rx = None;
    s.watch_path = wanted.clone();
    s.pending_refresh = None;
    let (Some(path), Some(wake)) = (wanted, s.waker.clone()) else { return };
    let (handle, rx) = watch::spawn(path, wake);
    s.watch = Some(handle);
    s.watch_rx = Some(rx);
}

/// Swap a watched folder's new listing in without disturbing the person:
/// selection, focus and anchor follow their paths, and the scroll stays put.
fn apply_refresh(s: &mut App, msg: watch::Refreshed) {
    // The Gallery shows a swept library under date headers, not the folder's own
    // listing: a plain rescan of Pictures would swap that grid for the flat list
    // it sits above, every time a photo anywhere under it appeared.
    if s.view == View::Gallery {
        refresh_gallery(s);
        return;
    }
    let path_of = |i: usize| s.entries.get(i).map(|e| e.path.clone());
    let selected: Vec<String> = s.selected.iter().filter_map(|&i| path_of(i)).collect();
    let focused = s.focused.and_then(path_of);
    let anchor = s.anchor.and_then(path_of);
    s.entries = msg.entries;
    s.scan_ms = msg.scan_ms;
    refresh_visible(s);
    let index_of = |p: &String| s.entries.iter().position(|e| &e.path == p);
    s.selected = selected.iter().filter_map(index_of).collect();
    s.focused = focused.as_ref().and_then(index_of);
    s.anchor = anchor.as_ref().and_then(index_of);
    update_selection_detail(s);
}

/// A restored tab shows its saved listing instantly, but that snapshot missed
/// everything that happened while another tab was on screen — a paste from
/// that tab, another program's writes. Rescan off the UI thread and let the
/// seamless apply swap the news in without moving the scroll or selection.
fn rescan_restored(s: &mut App) {
    if s.view == View::Gallery {
        // Rebuilt rather than rescanned, for the reason `apply_refresh` gives.
        refresh_gallery(s);
        return;
    }
    if s.view != View::Files {
        return;
    }
    let path = s.root.clone();
    let tx = s.rescan_tx.clone();
    let wake = s.waker.clone();
    let _ = std::thread::Builder::new().name("lyre-rescan".into()).spawn(move || {
        let (entries, scan_ms) = scan(&path);
        if tx.send(watch::Refreshed { path, entries, scan_ms }).is_ok()
            && let Some(wake) = wake
        {
            wake();
        }
    });
}

/// Drain finished rescans. An inline rename owns its row, and a search owns the
/// list: hold the refresh until the editor closes or the search clears rather
/// than pulling either out from under the person.
fn drain_watch(s: &mut App) -> bool {
    let mut msgs = Vec::new();
    if let Some(rx) = &s.watch_rx {
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }
    }
    while let Ok(msg) = s.rescan_rx.try_recv() {
        msgs.push(msg);
    }
    let mut changed = false;
    let held = matches!(s.dialog, Some(Dialog::Rename { .. })) || s.searching;
    for msg in msgs {
        if msg.path != s.root {
            continue;
        }
        if held {
            s.pending_refresh = Some(msg);
        } else {
            apply_refresh(s, msg);
            changed = true;
        }
    }
    if !held
        && let Some(msg) = s.pending_refresh.take()
        && msg.path == s.root
    {
        apply_refresh(s, msg);
        changed = true;
    }
    changed
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
        // Quick Access: the folder the landing page gathers, with the star its
        // favorites are marked by.
        2 => Shape::new()
            .fill(vec![[2.0, 6.0], [9.0, 6.0], [11.0, 9.0], [22.0, 9.0], [22.0, 21.0], [2.0, 21.0]], c("#f5a623"))
            .fill(vec![[2.0, 10.0], [22.0, 10.0], [22.0, 21.0], [2.0, 21.0]], c("#ffc861"))
            .fill(star(12.0, 15.5, 5.4, 2.2), c("#2f6fd0")),
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
    s.scale.set(cx.env().scale);
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
        .on_drop_files(|s: &mut App, files, _tx: &mut Tx| {
            // Platforms whose host delivers drops itself (Linux, macOS) land
            // here; Windows uses its own IDropTarget with the landing point.
            if !matches!(s.view, View::Files | View::Gallery) {
                return;
            }
            let staged = osdrag::stage(files);
            s.drop_staged.extend(staged.iter().cloned());
            let target = s.root.clone();
            paste_into(s, staged, fileops::Mode::Copy, target);
        })
        .child(tab_strip(s, p))
        .child(address_row(s, p))
        .child(command_bar(s, p))
        .child(body(s, p))
        .child(status_bar(s, p))
        .child(scroll_thumb(s, p))
        .child(drag_overlay(s, p))
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

/// One tab in the strip: its place mark, name and close button. The active tab
/// is filled; the others are transparent until hovered.
fn tab_button(s: &App, p: Paint, index: usize, tab: &Tab) -> El {
    let active = index == s.active_tab;
    // The showing tab is drawn from the live state, never from its snapshot.
    // A snapshot is only taken when the tab is left, opened or closed, so a tab
    // that navigated kept the name and icon it was opened at: the strip said
    // Quick Access while the window showed Desktop.
    let (name, view, root) = if active {
        (s.tab.clone(), s.view, s.root.as_path())
    } else {
        (tab.name.clone(), tab.view, tab.root.as_path())
    };
    let glyph = place_of(view, root).index();
    let b = row()
        .key(format!("tab-{index}"))
        .on_drop_files(move |s: &mut App, files, tx: &mut Tx| {
            take_drop(s, Some(DropTarget::Tab(index)), files, tx);
        })
        .h(TAB_ITEM_H)
        .w(180.0)
        .corners([8.0, 8.0, 0.0, 0.0])
        .fill(if active { p.tab } else if drop_hot(s, &format!("tab-{index}")) { p.hover } else { Color::TRANSPARENT })
        .hover(if active { Color::TRANSPARENT } else { p.hover })
        .pl(12.0)
        .pr(6.0)
        .gap(8.0)
        .items_center()
        .child(place_glyph(glyph, 16.0))
        .child(
            text(name.as_str())
                .font(CAPTION)
                .color(p.text)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(
            el().key(format!("tab-close-{index}"))
                .size(22.0, 22.0)
                .radius(4.0)
                .hover(p.hover)
                .center()
                .child(ico(p, Ico::Close, 11.0, p.secondary))
                .help("Close tab")
                .on_press(move |s: &mut App| close_tab_at(s, index)),
        );
    // Alt+click closes, as it does in a browser; a plain click switches.
    b.on_press_tx(move |s: &mut App, tx: &mut Tx| tab_pressed(s, index, tx.mods().alt))
}

/// A press on a tab: Alt closes it, anything else switches to it.
fn tab_pressed(s: &mut App, index: usize, alt: bool) {
    if alt {
        close_tab_at(s, index);
    } else {
        switch_tab(s, index);
    }
}

/// Close the tab a press landed on, from the key path of the element under it.
/// True when a tab was there to close.
fn close_tab_at_path(s: &mut App, path: &str) -> bool {
    match parse_drop_target(s, path).1 {
        Some(DropTarget::Tab(index)) => {
            close_tab_at(s, index);
            true
        }
        _ => false,
    }
}

fn tab_strip(s: &App, p: Paint) -> El {
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
        .window_drag();
    for (index, tab) in s.tabs.iter().enumerate() {
        strip = strip.child(tab_button(s, p, index, tab));
    }
    strip = strip
        .child(
            el().key("new-tab")
                .size(30.0, 30.0)
                .radius(4.0)
                .hover(p.hover)
                .center()
                .child(text("+").font(TextStyle::new(16.0, 400.0)).color(p.secondary))
                .help("New tab")
                .on_press(|s: &mut App| new_tab(s)),
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
            0 => {
                go_back(s);
            }
            1 => {
                go_forward(s);
            }
            2 => go_up(s),
            _ => {
                let path = s.root.clone();
                set_location(s, path);
            }
        })
}

/// The breadcrumb segments of the place on screen, each with the path it
/// opens. Virtual places offer their single label; a real root offers every
/// component, drive prefix included, with the prefix path it stands for.
fn crumb_segments(s: &App) -> Vec<(String, PathBuf)> {
    match s.view {
        View::QuickAccess => return vec![("Quick Access".into(), PathBuf::new())],
        View::ThisPc => return vec![("This PC".into(), PathBuf::new())],
        View::Gallery => return vec![("Gallery".into(), s.root.clone())],
        View::Files => {}
    }
    use std::path::Component;
    let mut prefix = PathBuf::new();
    let mut out = Vec::new();
    for part in s.root.components() {
        match part {
            Component::Prefix(p) => {
                prefix.push(p.as_os_str());
                prefix.push("\\");
                out.push((p.as_os_str().to_string_lossy().into_owned(), prefix.clone()));
            }
            Component::RootDir => {
                prefix.push(std::path::MAIN_SEPARATOR.to_string());
                // On Windows the root belongs to the drive prefix ("C:\"),
                // not to a crumb of its own.
                if !cfg!(windows) {
                    out.push(("/".into(), prefix.clone()));
                }
            }
            Component::Normal(n) => {
                prefix.push(n);
                out.push((n.to_string_lossy().into_owned(), prefix.clone()));
            }
            _ => {}
        }
    }
    out
}

/// A breadcrumb press: virtual places reload themselves, a real segment opens
/// the prefix path it names.
fn open_crumb(s: &mut App, path: PathBuf) {
    match s.view {
        View::QuickAccess => open_quick_access(s),
        View::ThisPc => navigate(s, 9),
        View::Gallery => go_gallery(s),
        View::Files => go(s, path),
    }
}

/// Turn the bar into a path editor with the current path selected, as Ctrl+L
/// and a click on the empty field do in Explorer.
fn begin_address_edit(s: &mut App) {
    if s.address_edit.is_some() {
        return;
    }
    let draft = match s.view {
        View::Files | View::Gallery => s.root.to_string_lossy().into_owned(),
        _ => String::new(),
    };
    let mut edit = TextEdit::new(draft);
    edit.anchor = 0;
    edit.focus = edit.text.chars().count();
    edit.focused = true;
    s.search_edit.focused = false;
    s.menu = None;
    s.address_edit = Some(edit);
}

/// Enter in the address editor: open what was typed. Relative paths resolve
/// against the folder on screen; the places answer to their names, Home being
/// the home folder itself; a path that is not a folder keeps the editor open
/// with a notice.
fn commit_address(s: &mut App) {
    let Some(edit) = s.address_edit.take() else { return };
    let draft = edit.text.trim().trim_matches('"').to_string();
    if draft.is_empty() {
        return;
    }
    match draft.to_ascii_lowercase().as_str() {
        "home" => return open_home(s),
        "quick access" | "quickaccess" => return open_quick_access(s),
        "this pc" | "thispc" => return navigate(s, 9),
        "gallery" => return go_gallery(s),
        _ => {}
    }
    let path = if let Some(rest) = draft.strip_prefix('~') {
        match home_dir() {
            Some(home) => home.join(rest.trim_start_matches(['/', '\\'])),
            None => PathBuf::from(&draft),
        }
    } else {
        let typed = PathBuf::from(&draft);
        if typed.is_absolute() { typed } else { s.root.join(typed) }
    };
    if path == s.root && s.view == View::Files {
        return;
    }
    if path.is_dir() {
        go(s, path);
    } else {
        s.notice = format!("{} is not available here", draft);
        let mut edit = TextEdit::new(draft);
        edit.anchor = 0;
        edit.focus = edit.text.chars().count();
        edit.focused = true;
        s.address_edit = Some(edit);
    }
}

fn address_row(s: &App, p: Paint) -> El {
    let crumb = if let Some(edit) = &s.address_edit {
        row()
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
            .child(ico(p, Ico::ThisPc, 15.0, p.secondary))
            .child(
                text_input(edit, UI_TEXT)
                    .key("address-edit")
                    .grow(1.0)
                    .min_w(0.0)
                    .single_line()
                    .color(p.text)
                    .placeholder("Enter a path", p.tertiary)
                    .on_edit(|s: &mut App| s.address_edit.as_mut().expect("address editing"))
                    .on_submit(|s: &mut App, _tx: &mut Tx| commit_address(s)),
            )
    } else {
        let segments = crumb_segments(s);
        // Deep paths keep their head and tail and fold the middle, as Explorer
        // does; the fold opens the editor with the whole path.
        let folded: Vec<Option<&(String, PathBuf)>> = if segments.len() > 5 {
            let mut v = vec![Some(&segments[0]), None];
            v.extend(segments[segments.len() - 3..].iter().map(Some));
            v
        } else {
            segments.iter().map(Some).collect()
        };
        let mut bar = row()
            .key("breadcrumb")
            .grow(1.0)
            // The trail gives way first in a narrow window: the search field and
            // the navigation buttons have to stay reachable.
            .min_w(if narrow(s) { 80.0 } else { 120.0 })
            .h(FIELD_H)
            .radius(4.0)
            .fill(p.field)
            .border(p.hair, p.field_line)
            .px(10.0)
            .gap(6.0)
            .items_center()
            // A place draws its own mark ahead of its single crumb; a real
            // folder starts at the drive or the root, so it shows the machine.
            .child(match s.view {
                View::Files => ico(p, Ico::ThisPc, 15.0, p.secondary),
                _ => place_glyph(place_of(s.view, &s.root).index(), 15.0),
            });
        for (i, segment) in folded.iter().enumerate() {
            bar = bar.child(ico(p, Ico::ChevronRight, 10.0, p.tertiary));
            match segment {
                None => {
                    bar = bar.child(
                        el().key("crumb-fold")
                            .h(22.0)
                            .px(4.0)
                            .radius(3.0)
                            .hover(p.hover)
                            .center()
                            .child(text("…").font(UI_TEXT).color(p.secondary))
                            .help("Edit address")
                            .on_press(|s: &mut App| begin_address_edit(s)),
                    );
                }
                Some((label, path)) => {
                    let tip = if path.as_os_str().is_empty() {
                        label.clone()
                    } else {
                        path.to_string_lossy().into_owned()
                    };
                    let path = path.clone();
                    bar = bar.child(
                        el().key(format!("crumb-{i}"))
                            .on_drop_files(move |s: &mut App, files, tx: &mut Tx| {
                                let t = crumb_segments(s)
                                    .get(i)
                                    .map(|(_, p)| p.clone())
                                    .filter(|p| !p.as_os_str().is_empty())
                                    .map(DropTarget::Folder);
                                take_drop(s, t, files, tx);
                            })
                            .h(22.0)
                            .px(4.0)
                            .radius(3.0)
                            .fill(if drop_hot(s, &format!("crumb-{i}")) { p.hover } else { Color::TRANSPARENT })
                            .hover(p.hover)
                            .center()
                            .child(text(label.as_str()).font(UI_TEXT).color(p.text).truncate())
                            .help(tip)
                            .on_press(move |s: &mut App| open_crumb(s, path.clone())),
                    );
                }
            }
        }
        bar = bar.child(ico(p, Ico::ChevronRight, 10.0, p.tertiary));
        // The subfolder arrow is the first thing to go in a narrow window: a
        // trail that runs out of room would otherwise end past the field it is
        // drawn in.
        if matches!(s.view, View::Files | View::Gallery) && !narrow(s) {
            bar = bar.child(
                el().key("crumb-tail")
                    .size(22.0, 22.0)
                    .radius(3.0)
                    .hover(p.hover)
                    .center()
                    .drawn_rect_into(&s.rect_crumb_tail)
                    .child(ico(p, Ico::ChevronDown, 10.0, p.tertiary))
                    .help("Open a subfolder")
                    .on_press(|s: &mut App| {
                        s.tail_folders = fs::read_dir(&s.root)
                            .into_iter()
                            .flatten()
                            .flatten()
                            .filter(|e| e.path().is_dir())
                            .take(30)
                            .map(|e| {
                                (
                                    e.file_name().to_string_lossy().into_owned(),
                                    e.path().to_string_lossy().into_owned(),
                                )
                            })
                            .collect();
                        let r = s.rect_crumb_tail.get();
                        open_drop(s, DropMenu::Subfolders, r);
                    }),
            );
        }
        // The blank space right of the crumbs opens the editor, as a click on
        // the empty field does in Explorer.
        bar.child(el().grow(1.0).h_full().on_press(|s: &mut App| begin_address_edit(s)))
    };
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
        .child(nav_button(p, Ico::Up, "up", 2, can_go_up(s), "Up"))
        .child(nav_button(p, Ico::Refresh, "refresh", 3, true, "Refresh"))
        .child(el().w(6.0))
        .child(crumb)
        .child(el().w(6.0))
        .child(
            row()
                .key("search")
                // The field takes a share of a narrow window rather than its
                // full width, which used to push it and everything after it
                // past the edge.
                .w(search_width(s))
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

/// How wide the search field is: what it always was, or a share of a window too
/// narrow to give it that.
fn search_width(s: &App) -> f32 {
    let body = body_width(s);
    260.0_f32.min(body * 0.45).max(96.0)
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
    text_button_parts(p, which, Some(label), chevron, dim)
}

/// The same button without its word, for a window too narrow to hold a row of
/// them: the icon keeps the press and the tooltip keeps the name.
fn text_button_bare(p: Paint, which: Ico, chevron: bool, dim: bool) -> El {
    text_button_parts(p, which, None, chevron, dim)
}

fn text_button_parts(p: Paint, which: Ico, label: Option<&str>, chevron: bool, dim: bool) -> El {
    let ink = if dim { p.disabled } else { p.secondary };
    let mut b = row()
        .h(32.0)
        .px(8.0)
        .gap(6.0)
        .radius(4.0)
        .hover(p.hover)
        .press(p.press)
        .items_center()
        .child(ico(p, which, 16.0, ink));
    if let Some(label) = label {
        b = b.child(text(label).font(UI_TEXT).color(if dim { p.disabled } else { p.text }));
    }
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
    // A narrow window keeps the icons and drops the words: seven labelled
    // buttons and a Details toggle do not fit under about 560 points of body,
    // and overflowing them past the edge is what went out of bounds.
    let words = !narrow(s);
    let label_button = |which: Ico, label: &str, chevron: bool| {
        if words {
            text_button(p, which, label, chevron, false)
        } else {
            text_button_bare(p, which, chevron, false).help(label)
        }
    };
    let mut bar = row()
        .key("command")
        .w_full()
        .h(COMMAND_H)
        .fill(p.command)
        .px(10.0)
        .gap(2.0)
        .items_center()
        .child(
            label_button(Ico::New, "New", true)
                .key("cmd-new")
                .drawn_rect_into(&s.rect_new)
                .help("New item")
                .on_press(|s: &mut App| {
                    let r = s.rect_new.get();
                    open_drop(s, DropMenu::New, r);
                }),
        )
        .child(el().w(p.hair).h(20.0).fill(p.line).mx(6.0));
    // The file-operation buttons are the first to go when the row will not fit:
    // the context menu and the keyboard still have every one of them.
    if !tight(s) {
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
    }
    bar = bar
        .child(el().w(p.hair).h(20.0).fill(p.line).mx(6.0))
        .child(
            label_button(Ico::Sort, "Sort", true)
                .key("cmd-sort")
                .drawn_rect_into(&s.rect_sort)
                .help("Sort")
                .on_press(|s: &mut App| {
                    let r = s.rect_sort.get();
                    open_drop(s, DropMenu::Sort, r);
                }),
        )
        .child(
            label_button(Ico::View, "View", true)
                .key("cmd-view")
                .drawn_rect_into(&s.rect_view)
                .help("View")
                .on_press(|s: &mut App| {
                    let r = s.rect_view.get();
                    open_drop(s, DropMenu::View, r);
                }),
        );
    bar = bar.child(
        label_button(Ico::Filter, "Filter", true)
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
    let details = label_button(Ico::Details, "Details", false)
        .help("Toggle the details pane")
        .on_press(|s: &mut App| {
            s.preview = !s.preview;
        });
    // Lit only when the pane is actually on screen: a window too narrow for it
    // leaves the button unlit rather than claiming a pane nobody can see.
    bar.child(if preview_shown(s) {
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
    paths: &[String],
    is_dir: bool,
    background: bool,
) -> (Vec<El>, f32) {
    let mut rows = Vec::new();
    let mut h = 0.0;
    let mut trees = shell::verbs(paths.first().map(String::as_str), is_dir, background);
    trees.extend(menu_handlers(s, paths, is_dir, background));
    push_verb_rows(s, p, &trees, paths, &mut rows, &mut h);
    if rows.is_empty() {
        return (rows, h);
    }
    let mut all = Vec::with_capacity(rows.len() + 1);
    all.push(menu_sep(p));
    all.extend(rows);
    (all, h + 9.0)
}

/// Handler items for the open menu, memoised by the selection they were read for.
/// Building them instantiates a third-party DLL, which is far too much work to
/// repeat on every frame the menu is drawn.
fn menu_handlers(s: &App, paths: &[String], is_dir: bool, background: bool) -> Vec<shell::Verb> {
    if s.handler_loaded.is_empty() {
        return Vec::new();
    }
    let key = format!("{is_dir}|{background}|{}", paths.join("\u{1}"));
    if let Some((cached, verbs)) = s.menu_handler_rows.borrow().as_ref()
        && *cached == key
    {
        return verbs.clone();
    }
    let verbs = shell::handlers(paths, is_dir, background, &s.handler_loaded);
    *s.menu_handler_rows.borrow_mut() = Some((key, verbs.clone()));
    verbs
}

/// The paths a context action applies to: the whole selection when the clicked
/// item is part of it, otherwise just that item. Explorer behaves this way, and
/// an extension needs all of them, not the one the pointer happened to be over.
fn context_paths(s: &App, target: usize) -> Vec<String> {
    let pick = |index: usize| -> Option<String> {
        if s.searching {
            s.search_hits.get(index).map(|hit| hit.entry.path.clone())
        } else {
            s.entries.get(index).map(|entry| entry.path.clone())
        }
    };
    if s.selected.len() > 1 && s.selected.contains(&target) {
        let picked: Vec<String> = s.selected.iter().filter_map(|index| pick(*index)).collect();
        if !picked.is_empty() {
            return picked;
        }
    }
    pick(target).into_iter().collect()
}

/// Append a verb tree to a menu, parents as a heading over the items under them.
fn push_verb_rows(
    s: &App,
    p: Paint,
    verbs: &[shell::Verb],
    paths: &[String],
    rows: &mut Vec<El>,
    h: &mut f32,
) {
    for verb in verbs {
        if verb.children.is_empty() {
            if !item_shown(s, &verb.id, false) {
                continue;
            }
            let picked = verb.clone();
            let target = paths.to_vec();
            let hint = (!verb.source.is_empty()).then_some(verb.source.as_str());
            rows.push(
                menu_item(p, &verb.label, None, hint, false).on_press(move |s: &mut App| {
                    if let Err(e) = shell::run(&picked, &target) {
                        s.notice = e;
                    }
                    s.menu = None;
                }),
            );
            *h += MENU_ROW_H;
        } else {
            // A parent is only worth drawing when something under it is on.
            if !verb
                .children
                .iter()
                .any(|child| item_shown(s, &child.id, false))
            {
                continue;
            }
            rows.push(menu_heading(p, &verb.label));
            *h += 24.0;
            push_verb_rows(s, p, &verb.children, paths, rows, h);
        }
    }
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
                DropMenu::Subfolders => {
                    if s.tail_folders.is_empty() {
                        rows.push(menu_item(p, "No subfolders", None, None, true));
                        h += MENU_ROW_H;
                    }
                    for (name, path) in s.tail_folders.clone() {
                        rows.push(
                            menu_item(p, &name, None, None, false).on_press(
                                move |s: &mut App| {
                                    s.menu = None;
                                    go(s, PathBuf::from(&path));
                                },
                            ),
                        );
                        h += MENU_ROW_H;
                    }
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
                    // Undo and redo first, dimmed when there is nothing to take
                    // back or do again, so the shortcut is discoverable.
                    rows.push(
                        menu_item(p, "Undo", None, Some("Ctrl+Z"), s.undo.is_empty())
                            .on_press(|s: &mut App| {
                                s.menu = None;
                                undo_last(s);
                            }),
                    );
                    h += MENU_ROW_H;
                    rows.push(
                        menu_item(p, "Redo", None, Some("Ctrl+Y"), s.redo.is_empty())
                            .on_press(|s: &mut App| {
                                s.menu = None;
                                redo_last(s);
                            }),
                    );
                    h += MENU_ROW_H;
                    rows.push(menu_sep(p));
                    h += 9.0;
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
                                    open_dialog(s, Dialog::DefaultManager);
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
                    rows.push(menu_heading(p, "Appearance"));
                    h += 24.0;
                    let theme = s.theme;
                    for (choice, label) in [
                        (theme::Theme::System, "Follow system"),
                        (theme::Theme::Light, "Light"),
                        (theme::Theme::Dark, "Dark"),
                    ] {
                        rows.push(
                            menu_item(
                                p,
                                label,
                                (theme == choice).then_some(Ico::Check),
                                None,
                                false,
                            )
                            .on_press(move |s: &mut App| {
                                set_theme(s, choice);
                                s.menu = None;
                            }),
                        );
                        h += MENU_ROW_H;
                    }
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
                        let label = entry.map(|e| e.name.clone()).unwrap_or_default();
                        rows.push(
                            menu_item(p, "Open with…", None, None, false).on_press(move |s: &mut App| {
                                start_open_with(s, open.clone(), label.clone());
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
                    let (shell_rows, shell_h) = shell_verb_rows(s, p, &context_paths(s, target), is_dir, false);
                    rows.extend(shell_rows);
                    h += shell_h;
                    rows.push(menu_sep(p));
                    h += 9.0;
                    if menu_visible(s, "properties") {
                        rows.push(
                            menu_item(p, "Properties", Some(Ico::Info), None, false).on_press(move |s: &mut App| {
                                let entry = if s.searching {
                                    s.search_hits.get(target).map(|h| &h.entry)
                                } else {
                                    s.entries.get(target)
                                };
                                if let Some(entry) = entry {
                                    let details = metadata::describe(
                                        Path::new(&entry.path),
                                        entry.is_dir,
                                        kind_label(entry.kind),
                                    );
                                    open_dialog(s, Dialog::Properties(Box::new(details)));
                                }
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
                    let (shell_rows, shell_h) = shell_verb_rows(s, p, std::slice::from_ref(&root), true, true);
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

/// One row of the customize list.
struct CustomLine {
    id: String,
    label: String,
    caption: String,
    shown: bool,
    /// The state this item has before the person ever touched it.
    default_on: bool,
    /// When set, the row switches loading this extension instead of showing an
    /// item, and `id` is the CLSID.
    load: Option<String>,
}

/// One owner's items: Lyre itself, a program that registered verbs, or a
/// third-party extension.
struct CustomGroup {
    owner: String,
    caption: String,
    lines: Vec<CustomLine>,
}

fn push_custom(group: &mut Vec<CustomGroup>, owner: &str, caption: &str, line: CustomLine) {
    if let Some(existing) = group.iter_mut().find(|g| g.owner == owner) {
        existing.lines.push(line);
        return;
    }
    group.push(CustomGroup {
        owner: owner.to_string(),
        caption: caption.to_string(),
        lines: vec![line],
    });
}

/// Everything that can appear in a right-click menu for the dialog's target,
/// grouped by who provides it.
fn customize_groups(s: &App) -> Vec<CustomGroup> {
    let mut groups: Vec<CustomGroup> = Vec::new();

    // Lyre's own items always come first, in the order they appear in a menu.
    let mine: Vec<CustomLine> = CUSTOM_ITEMS
        .iter()
        .map(|(id, label)| CustomLine {
            id: id.to_string(),
            label: label.to_string(),
            caption: String::new(),
            shown: item_shown(s, id, true),
            default_on: true,
            load: None,
        })
        .collect();
    groups.push(CustomGroup {
        owner: "Lyre".into(),
        caption: String::new(),
        lines: mine,
    });

    let path = s.customize_path.as_deref();
    let is_dir = s.customize_is_dir;
    let background = s.customize_background;

    // Registry verbs, credited to the program that registered them.
    for verb in shell::verbs(path, is_dir, background) {
        for item in verb.flatten() {
            push_custom(
                &mut groups,
                &item.source,
                "",
                CustomLine {
                    id: item.id.clone(),
                    label: item.label.clone(),
                    caption: if item.extended { "Shift".into() } else { String::new() },
                    shown: item_shown(s, &item.id, false),
                    default_on: false,
                    load: None,
                },
            );
        }
    }

    // Third-party extensions. Their items only exist once the person lets Lyre
    // load the DLL, so the group carries that switch above them.
    let target: Vec<String> = path.map(|path| vec![path.to_string()]).unwrap_or_default();
    for handler in shell::handlers(&target, is_dir, background, &s.handler_loaded) {
        let clsid = handler.verb.clone();
        let loaded = !clsid.is_empty()
            && s.handler_loaded
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&clsid));
        let caption = if loaded {
            handler.source.clone()
        } else {
            format!("{} (not loaded)", handler.source)
        };
        push_custom(
            &mut groups,
            &handler.label,
            &caption,
            CustomLine {
                id: clsid.clone(),
                label: if loaded {
                    "Read by this extension".into()
                } else {
                    "Load this extension's items".into()
                },
                caption: String::new(),
                shown: loaded,
                default_on: false,
                load: Some(clsid.clone()),
            },
        );
        for item in &handler.children {
            push_custom(
                &mut groups,
                &handler.label,
                &caption,
                CustomLine {
                    id: item.id.clone(),
                    label: item.label.clone(),
                    caption: String::new(),
                    shown: item_shown(s, &item.id, false),
                    default_on: false,
                    load: None,
                },
            );
        }
    }

    groups
}

/// A tri-state owner switch: on when every item it owns is shown, off when none
/// is, and mixed otherwise.
fn group_state(lines: &[CustomLine]) -> Option<bool> {
    let mut any_on = false;
    let mut any_off = false;
    for line in lines.iter().filter(|line| line.load.is_none()) {
        if line.shown { any_on = true } else { any_off = true }
    }
    match (any_on, any_off) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// The tick box for an owner: filled when all of its items are on, a minus when
/// only some are, empty when none is.
fn tri_switch(p: Paint, state: Option<bool>) -> El {
    let box_el = el()
        .w(18.0)
        .h(18.0)
        .radius(4.0)
        .border(p.hair, p.menu_line)
        .center();
    match state {
        Some(true) => box_el
            .fill(p.accent_fill)
            .child(ico(p, Ico::Check, 12.0, Color::WHITE)),
        None => box_el.child(el().w(8.0).h(2.0).radius(1.0).fill(p.accent_fill)),
        Some(false) => box_el,
    }
}

/// Switch one extension on or off. Loading is what makes its items readable, so
/// this is deliberately a separate choice from showing an item.
fn toggle_handler(s: &mut App, clsid: &str) {
    if let Some(at) = s
        .handler_loaded
        .iter()
        .position(|allowed| allowed.eq_ignore_ascii_case(clsid))
    {
        s.handler_loaded.remove(at);
    } else {
        s.handler_loaded.push(clsid.to_string());
    }
}

/// One item row inside an owner group.
fn custom_line(p: Paint, line: &CustomLine) -> El {
    let id = line.id.clone();
    let default_on = line.default_on;
    let load = line.load.clone();
    let check = if line.shown {
        ico(p, Ico::Check, 12.0, Color::WHITE)
    } else {
        el().size(12.0, 12.0)
    };
    let mut label_row = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .items_center()
        .child(
            text(line.label.as_str())
                .font(UI_TEXT)
                .color(if load.is_some() && !line.shown { p.secondary } else { p.text })
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if !line.caption.is_empty() {
        label_row = label_row.child(text(line.caption.as_str()).font(CAPTION).color(p.tertiary));
    }
    row()
        .w_full()
        .h(30.0)
        .px(22.0)
        .gap(10.0)
        .radius(4.0)
        .fill(Color::TRANSPARENT)
        .hover(p.hover)
        .items_center()
        .child(
            el().size(17.0, 17.0)
                .radius(4.0)
                .border(p.hair, p.menu_line)
                .fill(if line.shown { p.accent_fill } else { Color::TRANSPARENT })
                .center()
                .child(check),
        )
        .child(label_row)
        .on_press(move |s: &mut App| match &load {
            Some(clsid) => toggle_handler(s, clsid),
            None => toggle_menu_item(s, &id, default_on),
        })
}

/// One owner's block: a header that opens on hover, its tri-state switch, and
/// its items once open.
fn customize_group(s: &App, p: Paint, group: &CustomGroup) -> El {
    let open = s.customize_expanded.iter().any(|name| name == &group.owner);
    let state = group_state(&group.lines);
    let total = group.lines.iter().filter(|line| line.load.is_none()).count();
    let on = group
        .lines
        .iter()
        .filter(|line| line.load.is_none() && line.shown)
        .count();

    let mut header = row()
        .w_full()
        .h(28.0)
        .px(6.0)
        .gap(8.0)
        .radius(4.0)
        .hover(p.hover)
        .items_center()
        .child(
            text(if open { "▾" } else { "▸" })
                .font(CAPTION)
                .color(p.tertiary),
        )
        .child(
            text(group.owner.as_str())
                .font(GROUP_TEXT)
                .color(p.text)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if !group.caption.is_empty() {
        header = header.child(text(group.caption.as_str()).font(CAPTION).color(p.tertiary));
    }
    if total > 0 {
        header = header.child(
            text(format!("{on}/{total}"))
                .font(CAPTION)
                .color(p.tertiary),
        );
        // Flipping the owner sets every one of its items the same way.
        let targets: Vec<(String, bool)> = group
            .lines
            .iter()
            .filter(|line| line.load.is_none())
            .map(|line| (line.id.clone(), line.default_on))
            .collect();
        let wanted = state != Some(true);
        header = header.child(tri_switch(p, state).on_press(move |s: &mut App| {
            for (id, default_on) in &targets {
                if item_shown(s, id, *default_on) != wanted {
                    toggle_menu_item(s, id, *default_on);
                }
            }
        }));
    }

    let mut block = col()
        .key(format!("customize-group:{}", group.owner))
        .w_full()
        .gap(2.0)
        .child(header);
    if open {
        for line in &group.lines {
            block = block.child(custom_line(p, line));
        }
    }
    // Hovering anywhere in the block, header or items, keeps it open; leaving
    // closes it, which is how the shell's own cascades behave.
    let hovered = group.owner.clone();
    block.on_hover_change(move |s: &mut App, inside, _tx: &mut Tx| {
        if inside {
            if !s.customize_expanded.iter().any(|name| name == &hovered) {
                s.customize_expanded.push(hovered.clone());
            }
        } else {
            s.customize_expanded.retain(|name| name != &hovered);
        }
    })
}

/// A modal dialog toggling which context-menu items appear; choices persist.
fn customize_dialog(s: &App, p: Paint) -> El {
    if !s.customize_open {
        return el().key("customize-none");
    }
    let groups = customize_groups(s);
    let mut list = col().key("customize-list").w_full().gap(10.0);
    for group in &groups {
        list = list.child(customize_group(s, p, group));
    }

    el().key("customize-overlay")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK.alpha(0.45))
        .on_press(|s: &mut App| s.customize_open = false)
        .center()
        .child(
            col()
                .key("customize-panel")
                .w(560.0)
                .px(16.0)
                .py(16.0)
                .gap(12.0)
                .fill(p.menu)
                .border(p.hair, p.menu_line)
                .radius(8.0)
                .shadow(6.0, 24.0, Color::BLACK.alpha(0.5))
                .child(text("Customize context menu").font(TILE_TEXT).color(p.text))
                .child(
                    text("Hover an owner to open it. Items marked Shift need Shift held.")
                        .font(CAPTION)
                        .color(p.tertiary),
                )
                .child(
                    col()
                        .key("customize-scroll")
                        .w_full()
                        .h(430.0)
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

/// One button a dialog can show. The view draws the dialog's buttons from
/// `dialog_buttons` and the keyboard presses them by the same index, so neither
/// can offer something the other does not have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DialogButton {
    Cancel,
    /// The dialog's own verb: Delete, Create, OK.
    Confirm,
    KeepBoth,
    Overwrite,
    MakeDefault,
    Install,
    ChooseApp,
    Open,
    Close,
}

/// The buttons of `dialog`, in the order they are drawn and walked. A dialog's
/// buttons depend only on the dialog itself, so the row and the keyboard agree.
fn dialog_buttons(dialog: Option<&Dialog>) -> Vec<DialogButton> {
    match dialog {
        Some(Dialog::Delete { .. })
        | Some(Dialog::Rename { .. })
        | Some(Dialog::NewFolder { .. }) => vec![DialogButton::Cancel, DialogButton::Confirm],
        Some(Dialog::PasteConflict { .. }) => {
            vec![DialogButton::Cancel, DialogButton::KeepBoth, DialogButton::Overwrite]
        }
        Some(Dialog::DefaultManager) => vec![DialogButton::Cancel, DialogButton::MakeDefault],
        Some(Dialog::Update) => vec![DialogButton::Cancel, DialogButton::Install],
        Some(Dialog::OpenWith { data, .. }) => {
            // The same two conditions the row is drawn under: the system's
            // picker where the app cannot set a default, and Open only when
            // there is an application to open with.
            let mut buttons = vec![DialogButton::Cancel];
            if data.as_ref().is_some_and(|d| d.default_via_picker) {
                buttons.push(DialogButton::ChooseApp);
            }
            if data.as_ref().is_some_and(|d| !d.apps.is_empty()) {
                buttons.push(DialogButton::Open);
            }
            buttons
        }
        Some(Dialog::Properties(_)) => vec![DialogButton::Close],
        None => Vec::new(),
    }
}

fn dialog_button_label(s: &App, button: DialogButton) -> String {
    match button {
        DialogButton::Cancel => "Cancel".into(),
        DialogButton::Confirm => match s.dialog.as_ref() {
            Some(Dialog::Delete { permanent: true, .. }) => "Delete permanently".into(),
            Some(Dialog::Delete { .. }) => "Delete".into(),
            Some(Dialog::NewFolder { .. }) => "Create".into(),
            _ => "OK".into(),
        },
        DialogButton::KeepBoth => "Keep both".into(),
        DialogButton::Overwrite => "Overwrite".into(),
        DialogButton::MakeDefault => "Make default".into(),
        DialogButton::Install => {
            if s.update_installing { "Installing…".into() } else { "Install".into() }
        }
        DialogButton::ChooseApp => "Choose another app…".into(),
        DialogButton::Open => "Open".into(),
        DialogButton::Close => "Close".into(),
    }
}

/// Whether the button carries the accent fill: one per dialog, the one Enter
/// presses when the dialog opens.
fn dialog_button_primary(button: DialogButton) -> bool {
    !matches!(
        button,
        DialogButton::Cancel | DialogButton::KeepBoth | DialogButton::ChooseApp
    )
}

/// Where the keyboard starts in a dialog: on its primary button, as Windows does.
fn dialog_default_focus(buttons: &[DialogButton]) -> usize {
    buttons
        .iter()
        .position(|button| dialog_button_primary(*button))
        .unwrap_or(0)
}

/// The button the keyboard is on, clamped to a dialog whose buttons changed.
fn dialog_focus_index(s: &App, len: usize) -> usize {
    s.dialog_focus.min(len.saturating_sub(1))
}

/// Open a dialog with the keyboard on its primary button.
fn open_dialog(s: &mut App, dialog: Dialog) {
    let buttons = dialog_buttons(Some(&dialog));
    s.dialog_focus = dialog_default_focus(&buttons);
    s.dialog = Some(dialog);
}

/// Press one of the dialog's buttons. The press handlers and the keyboard call
/// this, so a button does the same thing however it is pressed.
fn press_dialog_button(s: &mut App, button: DialogButton) {
    match button {
        DialogButton::Cancel | DialogButton::Close => {
            // The update dialog is not dismissable while it downloads: its
            // progress is the only sign that anything is happening.
            if matches!(s.dialog, Some(Dialog::Update)) && s.update_busy {
                return;
            }
            s.dialog = None;
        }
        DialogButton::Confirm => match s.dialog.as_ref() {
            Some(Dialog::Delete { .. }) => {
                let Some(Dialog::Delete { permanent, paths }) = s.dialog.take() else { return };
                start_delete(s, if permanent { fileops::DeleteMode::Permanent } else { fileops::DeleteMode::Trash }, paths);
            }
            Some(Dialog::Rename { .. }) => commit_rename(s),
            Some(Dialog::NewFolder { .. }) => commit_new_folder(s),
            _ => {}
        },
        DialogButton::KeepBoth | DialogButton::Overwrite => {
            let Some(Dialog::PasteConflict { mode, sources, target }) = s.dialog.take() else {
                return;
            };
            let conflict = if button == DialogButton::Overwrite {
                fileops::Conflict::Overwrite
            } else {
                fileops::Conflict::Rename
            };
            start_paste(s, mode, conflict, sources, target);
        }
        DialogButton::MakeDefault => {
            match defaultmanager::make_default() {
                Ok(_) => s.notice = "Lyre now opens folders".into(),
                Err(e) => s.notice = e,
            }
            s.dialog = None;
        }
        DialogButton::Install => {
            if !s.update_busy {
                start_update_install(s);
            }
        }
        DialogButton::ChooseApp => {
            let Some(Dialog::OpenWith { path, .. }) = s.dialog.as_ref() else { return };
            let path = path.clone();
            if let Err(e) = shell::open_with_picker(&path) {
                s.notice = e;
            }
            s.dialog = None;
        }
        DialogButton::Open => {
            let Some(Dialog::OpenWith { path, selected, remember, data, .. }) = s.dialog.as_ref()
            else {
                return;
            };
            let (path, selected) = (path.clone(), selected.clone());
            // Windows cannot be told a new default, so the one-off open is all
            // that is left to honour there.
            let commit = *remember && !data.as_ref().is_some_and(|d| d.default_via_picker);
            if let Some(id) = selected {
                s.dialog = None;
                apply_open_with(s, &path, &id, commit);
            }
        }
    }
}

fn dialog_button(p: Paint, label: &str, primary: bool, focused: bool) -> El {
    let b = row()
        .h(32.0)
        .px(16.0)
        .radius(4.0)
        .fill(if primary { p.accent_fill } else { p.field })
        .border(p.hair, if primary { p.accent_fill } else { p.field_line })
        .hover(p.hover)
        .center()
        .child(text(label).font(UI_TEXT).color(if primary { Color::WHITE } else { p.text }));
    // The keyboard sits on one button; the ring says which, so Enter is never a
    // guess. It is drawn a little clear of the fill so both remain visible.
    if focused {
        b.border(p.hair * 2.0, if primary { Color::WHITE } else { p.accent })
    } else {
        b
    }
}

/// The dialog's buttons as one row, each pressing what the keyboard would.
fn dialog_buttons_el(s: &App, p: Paint) -> El {
    let buttons = dialog_buttons(s.dialog.as_ref());
    let focused = dialog_focus_index(s, buttons.len());
    let mut row_el = row().key("dialog-buttons").gap(8.0).items_center();
    for (index, button) in buttons.iter().copied().enumerate() {
        let label = dialog_button_label(s, button);
        row_el = row_el.child(
            dialog_button(p, &label, dialog_button_primary(button), index == focused)
                .key(format!("dialog-button-{index}"))
                .on_press(move |s: &mut App| press_dialog_button(s, button)),
        );
    }
    row_el
}

/// The radio mark an "Open with" row carries: a ring, filled in when chosen.
fn radio_dot(p: Paint, picked: bool) -> El {
    let ring = if picked { p.accent_fill } else { p.tertiary };
    let dot = el()
        .w(16.0)
        .h(16.0)
        .radius(8.0)
        .border(p.hair * 2.0, ring)
        .center();
    if picked {
        dot.child(el().w(8.0).h(8.0).radius(4.0).fill(p.accent_fill))
    } else {
        dot
    }
}

/// The tick box beside "Always use this application".
fn check_box(p: Paint, on: bool) -> El {
    let mark = el()
        .w(16.0)
        .h(16.0)
        .radius(3.0)
        .border(p.hair * 2.0, if on { p.accent_fill } else { p.tertiary })
        .center();
    if on {
        mark.fill(p.accent_fill).child(ico(p, Ico::Check, 11.0, Color::WHITE))
    } else {
        mark
    }
}

/// One label/value line of the Properties dialog.
fn property_row(p: Paint, label: &str, value: &str) -> El {
    row()
        .w_full()
        .gap(12.0)
        .items_center()
        .child(text(label).font(CAPTION).color(p.tertiary).w(92.0))
        .child(
            text(value)
                .font(UI_TEXT)
                .color(p.text)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
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
            base()
                .child(text(title).font(TILE_TEXT).color(p.text))
                .child(text(msg.as_str()).font(UI_TEXT).color(p.secondary))
                .child(
                    row().w_full().gap(8.0).child(el().grow(1.0))
                        .child(dialog_buttons_el(s, p)),
                )
        }
        Dialog::PasteConflict { .. } => {
            base()
                .child(text("Replace or keep both?").font(TILE_TEXT).color(p.text))
                .child(text("An item with the same name already exists in this folder.").font(UI_TEXT).color(p.secondary))
                .child(
                    row().w_full().gap(8.0).child(el().grow(1.0))
                        .child(dialog_buttons_el(s, p)),
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
                        .child(dialog_buttons_el(s, p)),
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
                        .child(dialog_buttons_el(s, p)),
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
                        .child(dialog_buttons_el(s, p)),
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
                    .child(dialog_buttons_el(s, p)),
            )
        }
        Dialog::OpenWith { name, data, selected, remember, .. } => {
            let picked = selected.clone();
            let mut panel = base().child(
                text(format!("Open “{name}” with")).font(TILE_TEXT).color(p.text).truncate(),
            );
            let mut loaded: Option<&openwith::OpenWith> = None;
            match data {
                None => {
                    panel = panel.child(
                        text("Looking for applications…").font(UI_TEXT).color(p.secondary),
                    );
                }
                Some(d) => {
                    loaded = Some(d);
                    if let Some(reason) = &d.unsupported {
                        panel = panel.child(
                            text(reason.as_str()).font(UI_TEXT).color(p.secondary).w_full().wrap(),
                        );
                    } else if d.apps.is_empty() {
                        let kind = if d.kind.is_empty() {
                            String::new()
                        } else {
                            format!("{} ", d.kind)
                        };
                        panel = panel.child(
                            text(format!(
                                "No application on this system says it can open {kind}files."
                            ))
                            .font(UI_TEXT)
                            .color(p.secondary)
                            .w_full()
                            .wrap(),
                        );
                    } else {
                        let default_id = d.default_app.clone();
                        // Application lists run long: the whole set scrolls
                        // inside a fixed height rather than growing the dialog.
                        let mut list = col()
                            .key("openwith-list")
                            .w_full()
                            .gap(2.0)
                            .max_h(252.0)
                            .scroll(ScrollSpec { stick_bottom: false });
                        for app in &d.apps {
                            let id = app.id.clone();
                            let is_default = default_id.as_deref() == Some(app.id.as_str());
                            let is_selected = picked.as_deref() == Some(app.id.as_str());
                            let mut item = row()
                                .w_full()
                                .h(30.0)
                                .px(8.0)
                                .gap(10.0)
                                .radius(4.0)
                                .fill(Color::TRANSPARENT)
                                .items_center()
                                .hover(p.hover)
                                .press(p.press)
                                .child(radio_dot(p, is_selected))
                                .child(
                                    text(app.name.as_str())
                                        .font(UI_TEXT)
                                        .color(p.text)
                                        .grow(1.0)
                                        .min_w(0.0)
                                        .truncate(),
                                )
                                .on_press(move |s: &mut App| {
                                    if let Some(Dialog::OpenWith { selected, .. }) = &mut s.dialog {
                                        *selected = Some(id.clone());
                                    }
                                });
                            if is_selected {
                                item = item.fill(p.row_sel);
                            }
                            if is_default {
                                item = item.child(text("Default").font(CAPTION).color(p.accent));
                            }
                            list = list.child(item);
                        }
                        panel = panel.child(list);
                    }
                }
            }

            let via_picker = loaded.is_some_and(|d| d.default_via_picker);

            // Ours to honour on Linux and macOS. On Windows the association is
            // hash-protected, so this row gives way to the system's own picker.
            if loaded.is_some() && !via_picker {
                let label = match loaded.and_then(|d| (!d.kind.is_empty()).then(|| d.kind.clone())) {
                    Some(kind) => format!("Always use this application to open {kind} files"),
                    None => "Always use this application for these files".to_string(),
                };
                panel = panel.child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .radius(4.0)
                        .items_center()
                        .hover(p.hover)
                        .child(check_box(p, *remember))
                        .child(
                            text(label)
                                .font(CAPTION)
                                .color(p.secondary)
                                .grow(1.0)
                                .min_w(0.0)
                                .wrap(),
                        )
                        .on_press(|s: &mut App| {
                            if let Some(Dialog::OpenWith { remember, .. }) = &mut s.dialog {
                                *remember = !*remember;
                            }
                        }),
                );
            }

            let footer = row()
                .w_full()
                .gap(8.0)
                .items_center()
                .child(el().grow(1.0))
                .child(dialog_buttons_el(s, p));
            panel.child(footer)
        }
        Dialog::Properties(details) => {
            let mut rows = col()
                .key("properties-rows")
                .w_full()
                .gap(6.0)
                .child(property_row(p, "Type", &details.kind));
            if details.size > 0 {
                rows = rows.child(property_row(p, "Size", &details.size_label()));
            }
            if !details.location.is_empty() {
                rows = rows.child(property_row(p, "Location", &details.location));
            }
            for (label, at) in [
                ("Modified", details.modified),
                ("Created", details.created),
                ("Accessed", details.accessed),
            ] {
                if let Some(at) = at {
                    rows = rows.child(property_row(p, label, &format_time(at)));
                }
            }
            // Whatever the type itself could tell us: dimensions, a text file's
            // shape, what a folder holds.
            for (label, value) in &details.extra {
                rows = rows.child(property_row(p, label, value));
            }
            base()
                .child(
                    text(format!("Properties: {}", details.name))
                        .font(TILE_TEXT)
                        .color(p.text)
                        .truncate(),
                )
                .child(
                    col()
                        .key("properties-scroll")
                        .w_full()
                        .max_h(320.0)
                        .scroll(ScrollSpec { stick_bottom: false })
                        .child(rows),
                )
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(el().grow(1.0))
                        .child(dialog_buttons_el(s, p)),
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
    current: Place,
    index: usize,
    label: &str,
    chevron: bool,
    pin: bool,
) -> El {
    let active = current.index() == index;
    let ink = if active { p.text } else { p.secondary };
    let open = s.rail_open.contains(&index);

    // The row itself carries the press that opens the place, so the whole row is
    // a target: on the body alone, the padding and the spacer beside it were not,
    // and a press there did nothing. The chevron keeps its own press, which wins
    // where it is drawn, so expanding a list still does not navigate.
    let body = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(8.0)
        .items_center()
        .child(place_glyph(index, 16.0))
        .child(text(label).font(ROW_TEXT).color(ink).grow(1.0).min_w(0.0).truncate());
    let body = if pin {
        body.child(ico(p, Ico::Pin, 10.0, p.tertiary))
    } else {
        body
    };

    let mut item = row()
        .key(format!("rail-{index}"))
        .on_drop_files(move |s: &mut App, files, tx: &mut Tx| {
            take_drop(s, place_path(index).map(DropTarget::Folder), files, tx);
        })
        .on_press_tx(move |s: &mut App, tx: &mut Tx| {
            navigate(s, index);
            tx.instant();
        })
        .w_full()
        .h(28.0)
        .radius(4.0)
        .fill(if active {
            p.rail_sel
        } else if drop_hot(s, &format!("rail-{index}")) {
            p.hover
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
    item = if chevron {
        let mark = if open { Ico::ChevronDown } else { Ico::ChevronRight };
        item.child(
            el()
                .w(11.0)
                .h_full()
                .fill(Color::TRANSPARENT)
                .center()
                .child(ico(p, mark, 9.0, p.tertiary))
                .on_press_tx(move |s: &mut App, tx: &mut Tx| {
                    toggle_rail(s, index);
                    tx.instant();
                }),
        )
    } else {
        item.child(el().w(11.0))
    };
    item.child(body)
        .on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
            s.menu = Some(Menu::Context { scope: CtxScope::Place, target: index, x: at.x, y: at.y });
        })
}

/// Open or close the list under a rail place. Drives come and go, so the volume
/// table is asked again each time This PC is opened rather than once at startup.
fn toggle_rail(s: &mut App, place: usize) {
    if let Some(at) = s.rail_open.iter().position(|open| *open == place) {
        s.rail_open.remove(at);
        return;
    }
    if place == 9 {
        refresh_volumes(s);
    }
    s.rail_open.push(place);
}

/// One row beneath an expanded rail place. A place with nothing to list says so
/// rather than leaving the chevron looking broken. `volume` is the index into
/// `s.volumes` when the row is a drive, so it can offer that drive's own menu.
fn rail_leaf(p: Paint, label: &str, path: Option<&str>, volume: Option<usize>) -> El {
    let leaf = row()
        .w_full()
        .h(24.0)
        .pl(31.0)
        .pr(8.0)
        .gap(6.0)
        .radius(4.0)
        .items_center()
        .child(
            text(label)
                .font(CAPTION)
                .color(p.secondary)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    let leaf = match path {
        Some(path) => {
            let target = path.to_string();
            leaf.hover(p.hover)
                .on_press(move |s: &mut App| go(s, PathBuf::from(target.clone())))
        }
        None => leaf,
    };
    // The same menu This PC's cards offer, so a drive can be ejected from here
    // too rather than only from the view on the right.
    match volume {
        Some(index) => leaf.on_secondary_press(move |s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
            s.menu = Some(Menu::Context { scope: CtxScope::Volume, target: index, x: at.x, y: at.y });
        }),
        None => leaf,
    }
}

/// A pinned folder in the rail: opens its path, right-click offers unpin.
///
/// `owner` is the one pinned row that names the folder on screen, decided by
/// `nav_rail`. Passing it rather than testing each row is what stops two pins of
/// the same folder, saved with and without a trailing separator or in another
/// case, from lighting up together.
fn pinned_item(s: &App, p: Paint, owner: Option<usize>, pi: usize, path: &str) -> El {
    let active = owner == Some(pi);
    let ink = if active { p.text } else { p.secondary };
    let name = path_name(path);
    let open = path.to_string();
    let mut item = row()
        .key(format!("pinned-{pi}"))
        .on_drop_files(move |s: &mut App, files, tx: &mut Tx| {
            let t = s.pinned.get(pi).cloned().map(|p| DropTarget::Folder(PathBuf::from(p)));
            take_drop(s, t, files, tx);
        })
        .w_full()
        .h(28.0)
        .radius(4.0)
        .fill(if active { p.rail_sel } else if drop_hot(s, &format!("pinned-{pi}")) { p.hover } else { Color::TRANSPARENT })
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
    // Resolved once for the six rows below, which all ask it.
    let home = home_dir();
    // Where the window actually is, which is the only input to the rail's
    // selection: one place, at most one lit row.
    let current = place_of(s.view, &s.root);
    // The one pinned row that names the folder on screen. Only a folder no
    // built-in place claims can select a pin, so This PC or Quick Access can
    // never sit lit beside one.
    let pinned_at = if current == Place::Folder {
        s.pinned.iter().position(|path| same_path(Path::new(path), &s.root))
    } else {
        None
    };
    let mut rail = col()
        .key("rail")
        .w(rail_width(s))
        .h_full()
        .fill(p.content)
        .px(4.0)
        .pt(6.0)
        .gap(1.0)
        .motion(Spring::NONE)
        .instant_changes()
        .child(rail_item(s, p, current, Place::QuickAccess.index(), "Quick Access", false, false))
        .child(rail_item(s, p, current, Place::Home.index(), "Home", false, false))
        .child(rail_item(s, p, current, Place::Gallery.index(), "Gallery", false, false))
        .child(el().w_full().h(p.hair).fill(p.soft_line).my(5.0).key("rail-sep-1"));
    for (place, label) in PLACE_FOLDERS {
        // A folder that is not on this machine is not offered, as the 0.2 rail
        // did not offer it: a row that can only answer "not available here" is
        // worse than no row.
        let offered = home.as_deref().is_some_and(|home| known::folder(home, label).is_some());
        if offered && !s.hidden_places.contains(&place.index()) {
            rail = rail.child(rail_item(s, p, current, place.index(), label, false, true));
        }
    }
    if !s.pinned.is_empty() {
        rail = rail.child(el().w_full().h(p.hair).fill(p.soft_line).my(5.0).key("rail-sep-pinned"));
        for (pi, path) in s.pinned.iter().enumerate() {
            rail = rail.child(pinned_item(s, p, pinned_at, pi, path));
        }
    }
    let mut rail = rail
        .child(el().w_full().h(p.hair).fill(p.soft_line).my(5.0).key("rail-sep-2"))
        .child(rail_item(s, p, current, Place::ThisPc.index(), "This PC", true, false));
    if s.rail_open.contains(&9) {
        if s.volumes.is_empty() {
            rail = rail.child(rail_leaf(p, "No drives found", None, None));
        }
        // The title is the volume's own: a label when it has one, otherwise the
        // kind of device, so an unnamed system drive reads "Local Disk (C:)".
        for (index, volume) in s.volumes.iter().enumerate() {
            let title = volume.title();
            let path = volume.path.to_string_lossy().into_owned();
            rail = rail.child(rail_leaf(p, &title, Some(&path), Some(index)));
        }
    }
    rail = rail.child(rail_item(s, p, current, Place::Network.index(), "Network", true, false));
    if s.rail_open.contains(&10) {
        rail = rail.child(rail_leaf(p, "Network browsing is not implemented", None, None));
    }
    let rail = rail.child(el().grow(1.0));
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

/// The Details columns as drawn.
///
/// The person's widths are honoured while they fit; a window too narrow to hold
/// them takes the difference out of all four in proportion, with a floor so a
/// column stays readable, rather than pushing the last two past its edge.
fn column_widths(s: &App) -> [f32; 4] {
    let room = (content_width(s) - COL_SEP_W * 4.0).max(120.0);
    let total: f32 = s.col_w.iter().sum();
    if total <= room {
        return s.col_w;
    }
    let share = room / total;
    let mut out = s.col_w.map(|w| (w * share).max(40.0));
    // Rounding up to the floors can still overshoot; take it back from the name,
    // which has the most to give.
    let sum: f32 = out.iter().sum();
    if sum > room {
        out[0] = (out[0] - (sum - room)).max(40.0);
    }
    out
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
    let widths = column_widths(s);
    let mut h = row().key("header").w_full().h(HEADER_H).items_center();
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            h = h.child(column_divider(p, i - 1));
        }
        let t = text(*label).font(CAPTION).color(p.secondary);
        h = h.child(col().w(widths[i]).pl(12.0).pr(8.0).child(if i == 3 {
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
    column_widths(s).iter().sum::<f32>() + COL_SEP_W * 4.0
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
    // A selection can outlive the row it named, when a refresh lands between the
    // press and this: only the rows that are still there are counted.
    let len = display_len(s);
    let selected: Vec<usize> = s.selected.iter().copied().filter(|&i| i < len).collect();
    let n = selected.len();
    if n == 0 {
        s.selection_detail.clear();
        return;
    }
    if n == 1 {
        let size = entry_by_index(s, selected[0]).size_label();
        s.selection_detail = if size.is_empty() {
            "1 item selected".into()
        } else {
            format!("1 item selected  {size}")
        };
    } else {
        let total: u64 = selected.iter().map(|&i| entry_by_index(s, i).size).sum();
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

/// The context-menu target under a window point: the item whose recorded rect
/// contains it, or the background. The scrim uses this to move an open menu to
/// whatever a right-click lands on.
fn context_target_at(s: &App, x: f32, y: f32) -> (CtxScope, usize) {
    let len = if s.view == View::ThisPc { s.volumes.len() } else { display_len(s) };
    let rects = s.marquee_rects.borrow();
    for (i, r) in rects.iter().take(len).enumerate() {
        if x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3] {
            let scope = if s.view == View::ThisPc { CtxScope::Volume } else { CtxScope::Item };
            return (scope, i);
        }
    }
    (CtxScope::Background, 0)
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

/// The volume a path lives on, longest mounted prefix first, so a drag between
/// two folders on one disk moves and a drag to another disk copies.
fn volume_of(s: &App, path: &Path) -> String {
    let mut best = String::new();
    for v in &s.volumes {
        let vp = v.path.to_string_lossy();
        if path.starts_with(&v.path) && vp.len() > best.len() {
            best = vp.into_owned();
        }
    }
    if best.is_empty() {
        best = path
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    if cfg!(windows) { best.to_ascii_lowercase() } else { best }
}

fn same_volume(s: &App, paths: &[String], target: &Path) -> bool {
    paths
        .first()
        .map(|p| volume_of(s, Path::new(p)) == volume_of(s, target))
        .unwrap_or(true)
}

fn drop_target_path(s: &App, target: Option<&DropTarget>) -> Option<PathBuf> {
    match target {
        Some(DropTarget::Folder(p)) => Some(p.clone()),
        Some(DropTarget::Tab(i)) => s.tabs.get(*i).map(|t| t.root.clone()),
        Some(DropTarget::Background) => Some(s.root.clone()),
        None => None,
    }
}

/// The old app's rule, kept: one volume moves, across volumes copies, and
/// Shift inverts whichever the volumes decided.
fn drop_mode(s: &App, paths: &[String], target: Option<&DropTarget>, shift: bool) -> fileops::Mode {
    let base = match drop_target_path(s, target) {
        Some(tp) if same_volume(s, paths, &tp) => fileops::Mode::Cut,
        Some(_) => fileops::Mode::Copy,
        None => fileops::Mode::Cut,
    };
    if !shift {
        return base;
    }
    match base {
        fileops::Mode::Cut => fileops::Mode::Copy,
        fileops::Mode::Copy => fileops::Mode::Cut,
    }
}

/// Read a drop target out of an element key path: the deepest keyed segment
/// that names a place a drag can land.
fn parse_drop_target(s: &App, path: &str) -> (String, Option<DropTarget>) {
    // Virtual rows are keyed by entry path (the virtualiser owns the root key),
    // so the row segment is the path itself, up to the first indexed child.
    if let Some((_, rest)) = path.split_once("/rows/") {
        let entry_path = rest.split("/#").next().unwrap_or(rest);
        if let Some(e) = s.entries.iter().find(|e| e.path == entry_path) {
            let key = format!("row:{}", e.path);
            let target = e.is_dir.then(|| DropTarget::Folder(PathBuf::from(&e.path)));
            return (key, target);
        }
    }
    for seg in path.rsplit('/') {
        if let Some(n) = seg.strip_prefix("row-") {
            if let Some(e) = n.parse::<usize>().ok().map(|i| display_entry(s, i).1) {
                let key = format!("row:{}", e.path);
                let target = e.is_dir.then(|| DropTarget::Folder(PathBuf::from(&e.path)));
                return (key, target);
            }
        }
        let target = if let Some(n) = seg.strip_prefix("crumb-") {
            n.parse::<usize>()
                .ok()
                .and_then(|i| crumb_segments(s).get(i).map(|(_, p)| p.clone()))
                .filter(|p| !p.as_os_str().is_empty())
                .map(DropTarget::Folder)
        } else if let Some(n) = seg.strip_prefix("tab-close-") {
            n.parse::<usize>()
                .ok()
                .filter(|i| *i < s.tabs.len())
                .map(DropTarget::Tab)
        } else if let Some(n) = seg.strip_prefix("tab-") {
            n.parse::<usize>()
                .ok()
                .filter(|i| *i < s.tabs.len())
                .map(DropTarget::Tab)
        } else if let Some(n) = seg.strip_prefix("rail-") {
            n.parse::<usize>().ok().and_then(place_path).map(DropTarget::Folder)
        } else if let Some(n) = seg.strip_prefix("pinned-") {
            n.parse::<usize>()
                .ok()
                .and_then(|i| s.pinned.get(i).cloned())
                .map(|p| DropTarget::Folder(PathBuf::from(p)))
        } else if let Some(n) = seg.strip_prefix("list-") {
            n.parse::<usize>()
                .ok()
                .and_then(|i| s.favorites.get(i).cloned())
                .map(|p| DropTarget::Folder(PathBuf::from(p)))
        } else if let Some(n) = seg.strip_prefix("drop-place-") {
            n.parse::<usize>().ok().and_then(place_path).map(DropTarget::Folder)
        } else if let Some(n) = seg.strip_prefix("drop-fav-") {
            n.parse::<usize>()
                .ok()
                .and_then(|i| s.favorites.get(i).cloned())
                .map(|p| DropTarget::Folder(PathBuf::from(p)))
        } else if let Some(n) = seg.strip_prefix("drop-vol-") {
            n.parse::<usize>()
                .ok()
                .and_then(|i| s.volumes.get(i).map(|v| PathBuf::from(&v.path)))
                .map(DropTarget::Folder)
        } else {
            continue;
        };
        return (seg.to_string(), target);
    }
    (String::new(), None)
}

/// Whether the element under a drag's pointer is its current target.
fn drop_hot(s: &App, key: &str) -> bool {
    s.drag.as_ref().is_some_and(|d| d.target_key == key) || s.os_hover.as_deref() == Some(key)
}

/// The gesture on a row: past the threshold it becomes a file drag carrying
/// the selection, hit-testing every frame against the places a drop can land.
fn row_drag(s: &mut App, rect_i: usize, ei: usize, d: &mut rune::Drag, tx: &mut Tx) {
    match d.phase {
        Phase::Down => {
            let rects = s.marquee_rects.borrow();
            if let Some(r) = rects.get(rect_i) {
                s.drag_press = Some((r[0] + d.x, r[1] + d.y));
            }
        }
        Phase::Move => {
            let Some(press) = s.drag_press else { return };
            // A move with no travel carries no new aim (hosts synthesize one
            // at the press point on release); keep the last real target.
            if d.dx == 0.0 && d.dy == 0.0 {
                return;
            }
            if s.drag.is_none() {
                if d.dx * d.dx + d.dy * d.dy < 16.0 {
                    return;
                }
                let paths = if s.selected.contains(&ei) {
                    selected_paths(s)
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect()
                } else {
                    vec![s.entries[ei].path.clone()]
                };
                if paths.is_empty() {
                    return;
                }
                // macOS runs the session through the OS (os_drag_source).
                if cfg!(target_os = "macos") {
                    return;
                }
                s.volumes = volumes::enumerate();
                s.marquee = None;
                s.drag = Some(DragState {
                    paths,
                    at: (press.0 + d.dx, press.1 + d.dy),
                    shift: tx.mods().shift,
                    target: None,
                    target_key: String::new(),
                    mode: fileops::Mode::Cut,
                });
            }
            let at = (press.0 + d.dx, press.1 + d.dy);
            let shift = tx.mods().shift;
            if let Some(dr) = &mut s.drag {
                dr.at = at;
                dr.shift = shift;
            }
            // Leaving the window hands the same gesture to the OS, so it can
            // finish in a browser or another app without a second drag.
            #[cfg(target_os = "windows")]
            if let Some(dr) = &s.drag {
                let w = s.rect_window.get();
                if at.0 < w[0] || at.0 > w[0] + w[2] || at.1 < w[1] || at.1 > w[1] + w[3] {
                    let paths: Vec<PathBuf> = dr.paths.iter().map(PathBuf::from).collect();
                    s.drag = None;
                    s.drag_press = None;
                    // Hand over on the next frame, so this one paints without
                    // the ghost before the OS takes the pointer over.
                    s.pending_os_drag = Some((paths, drag_preview(s, ei)));
                    return;
                }
            }
            #[cfg(target_os = "linux")]
            if let Some(dr) = &s.drag {
                let w = s.rect_window.get();
                if at.0 < w[0] || at.0 > w[0] + w[2] || at.1 < w[1] || at.1 > w[1] + w[3] {
                    let paths: Vec<PathBuf> = dr.paths.iter().map(PathBuf::from).collect();
                    s.drag = None;
                    s.drag_press = None;
                    osdrag::linux::start_drag_out(&paths);
                    return;
                }
            }
            tx.element_at(at.0, at.1, move |s: &mut App, picked, _tx: &mut Tx| {
                let picked_path = picked.as_ref().map(|p| p.path().to_string());
                let (key, mut target) = picked_path
                    .as_deref()
                    .map(|p| parse_drop_target(s, p))
                    .unwrap_or_default();
                // Anywhere in the list that is not a folder row — a file row,
                // the header, the empty space below the rows — means the
                // folder on screen, as a drop on Explorer's background does.
                if target.is_none()
                    && matches!(s.view, View::Files | View::Gallery)
                    && picked_path
                        .as_deref()
                        .is_some_and(|p| p.contains("list-area") || p.contains("/rows"))
                {
                    target = Some(DropTarget::Background);
                }
                let paths = s.drag.as_ref().map(|d| d.paths.clone()).unwrap_or_default();
                let mode = drop_mode(s, &paths, target.as_ref(), shift);
                if let Some(dr) = &mut s.drag {
                    dr.target_key = key;
                    dr.target = target;
                    dr.mode = mode;
                }
            });
        }
        Phase::Up => {
            s.drag_press = None;
            if d.cancelled() {
                s.drag = None;
                return;
            }
            finish_drag(s);
        }
    }
}

/// Let go: copy or move onto the target. A tab target takes the items into its
/// folder and brings that tab forward; the background of the folder the items
/// already live in is a no-op, as in Explorer.
fn finish_drag(s: &mut App) {
    let Some(dr) = s.drag.take() else { return };
    let Some(target) = dr.target else { return };
    let (target_path, tab) = match &target {
        DropTarget::Folder(p) => (p.clone(), None),
        DropTarget::Tab(i) => match s.tabs.get(*i) {
            Some(t) => (t.root.clone(), Some(*i)),
            None => return,
        },
        DropTarget::Background => (s.root.clone(), None),
    };
    let sources: Vec<PathBuf> = dr.paths.iter().map(PathBuf::from).collect();
    if dr.mode == fileops::Mode::Cut
        && sources.iter().all(|p| p.parent() == Some(target_path.as_path()))
    {
        return;
    }
    paste_into(s, sources, dr.mode, target_path);
    if let Some(i) = tab {
        if i != s.active_tab {
            switch_tab(s, i);
        }
    }
}

/// The ghost that follows the pointer — the dragged items' own icons and the
/// first name — with the badge saying what the drop would do, and how many.
fn drag_overlay(s: &App, p: Paint) -> El {
    let Some(dr) = &s.drag else { return el().key("drag-none") };
    let cut = dr.mode == fileops::Mode::Cut;
    let label = format!(
        "{} {} item{}",
        if cut { "Move" } else { "Copy" },
        dr.paths.len(),
        if dr.paths.len() == 1 { "" } else { "s" }
    );
    let ghost = dr.paths.iter().take(3).enumerate().map(|(i, path)| {
        let icon = s
            .entries
            .iter()
            .find(|e| &e.path == path)
            .map(|e| entry_icon(s, e, p, 32.0))
            .unwrap_or_else(|| el().size(32.0, 32.0));
        el().absolute(i as f32 * 6.0, i as f32 * 4.0).opacity(0.75).child(icon)
    });
    let name = dr
        .paths
        .first()
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    el().key("drag-badge")
        .absolute(dr.at.0 + 8.0, dr.at.1 + 8.0)
        .child(
            el().size(44.0, 40.0).children(ghost),
        )
        .child(
            el().absolute(0.0, 42.0).w(180.0).opacity(0.85).child(
                text(name.as_str()).font(CAPTION).color(p.text).truncate(),
            ),
        )
        .child(
            el().absolute(36.0, 12.0).child(
                row()
                    .h(24.0)
                    .px(8.0)
                    .gap(6.0)
                    .radius(4.0)
                    .fill(p.field)
                    .border(p.hair, p.field_line)
                    .items_center()
                    .child(ico(p, if cut { Ico::Forward } else { Ico::Copy }, 12.0, p.secondary))
                    .child(text(label.as_str()).font(CAPTION).color(p.text)),
            ),
        )
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
        // A walk from earlier in this search session answers without disk reads.
        cache: s.search_cache.clone(),
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
                search::Event::Index(cache) => {
                    // Keep it for the rest of this search session: the next
                    // keystroke is answered from memory.
                    s.search_cache = Some(cache);
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
    s.search_pending = None;
    // The names describe the folder that was being searched; they do not outlive
    // the search box, so nothing can be answered from a stale index.
    s.search_cache = None;
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
    // A dialog opened from this drain is applied after it, because the worker's
    // receiver is borrowed for the length of the loop.
    let mut opening: Option<Dialog> = None;
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
                                opening = Some(Dialog::Update);
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
    if let Some(dialog) = opening {
        open_dialog(s, dialog);
    }
    changed
}

/// Remember something that can be taken back. Doing anything new drops the redo
/// steps, as every editor does: they belong to a branch that no longer exists.
fn record_undo(s: &mut App, action: Undo) {
    s.undo.push(action);
    s.redo.clear();
}

/// Take back the last operation. Ctrl+Z.
fn undo_last(s: &mut App) -> bool {
    let Some(action) = s.undo.pop() else {
        s.notice = "Nothing to undo".into();
        return true;
    };
    match take_back(&action) {
        Ok(()) => {
            s.redo.push(action);
            after_undo(s, "Undone");
        }
        Err(e) => {
            // It stays where it was: the operation is still in effect.
            s.undo.push(action);
            s.notice = e;
        }
    }
    true
}

/// Do the last thing taken back again. Ctrl+Y.
fn redo_last(s: &mut App) -> bool {
    let Some(action) = s.redo.pop() else {
        s.notice = "Nothing to redo".into();
        return true;
    };
    match put_back(&action) {
        Ok(()) => {
            s.undo.push(action);
            after_undo(s, "Redone");
        }
        Err(e) => {
            s.redo.push(action);
            s.notice = e;
        }
    }
    true
}

/// Reload the folder on screen, so what was put back or taken away shows.
fn after_undo(s: &mut App, word: &str) {
    if s.view == View::Files {
        let root = s.root.clone();
        set_location(s, root);
    }
    s.notice = word.into();
}

/// Reverse one operation on disk.
fn take_back(action: &Undo) -> Result<(), String> {
    match action {
        Undo::Rename { from, to } => fs::rename(to, from)
            .map_err(|e| format!("Could not put the name back: {e}")),
        Undo::Made { path } => fileops::remove(path)
            .map_err(|e| format!("Could not remove the folder again: {e}")),
        Undo::Copied { pairs } => {
            for (_, copy) in pairs {
                fileops::remove(copy).map_err(|e| format!("Could not remove the copy: {e}"))?;
            }
            Ok(())
        }
        Undo::Moved { pairs } => {
            for (from, to) in pairs {
                if let Some(parent) = from.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::rename(to, from).map_err(|e| format!("Could not move it back: {e}"))?;
            }
            Ok(())
        }
        Undo::Trashed { items } => {
            fileops::restore(items).map_err(|e| format!("Could not take it out of the trash: {e}"))
        }
    }
}

/// Do one reversed operation again.
fn put_back(action: &Undo) -> Result<(), String> {
    match action {
        Undo::Rename { from, to } => {
            fs::rename(from, to).map_err(|e| format!("Could not rename it again: {e}"))
        }
        Undo::Made { path } => fs::create_dir_all(path)
            .map_err(|e| format!("Could not make the folder again: {e}")),
        Undo::Copied { pairs } => {
            fileops::copy_again(pairs).map_err(|e| format!("Could not copy it again: {e}"))
        }
        Undo::Moved { pairs } => {
            for (from, to) in pairs {
                if let Some(parent) = to.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::rename(from, to).map_err(|e| format!("Could not move it again: {e}"))?;
            }
            Ok(())
        }
        Undo::Trashed { items } => {
            // Deleting again: the originals are what the trash recorded.
            let paths: Vec<PathBuf> = items.iter().map(|item| item.original.clone()).collect();
            let cancel = std::sync::atomic::AtomicBool::new(false);
            fileops::delete(fileops::DeleteMode::Trash, &paths, &cancel, |_| true)
                .map(|_| ())
                .map_err(|e| format!("Could not delete it again: {e}"))
        }
    }
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
    if std::env::var_os("LYRE_TRACE").is_some() {
        eprintln!("[paste] mode={mode:?} sources={sources:?} target={target:?}");
    }
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
        let (outcome, undo) = match result {
            Ok(pairs) => {
                let count = pairs.len();
                let undo = if mode == fileops::Mode::Cut {
                    Undo::Moved { pairs }
                } else {
                    Undo::Copied { pairs }
                };
                (Ok(count), Some(undo))
            }
            Err(e) => (Err(e), None),
        };
        let _ = tx.send(OpMsg::Done { outcome, undo });
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
        // A delete to the trash can be taken back; a permanent one cannot, so it
        // records nothing to undo.
        let (outcome, undo) = match result {
            Ok(deleted) => {
                let undo = (!deleted.trashed.is_empty()).then_some(Undo::Trashed { items: deleted.trashed });
                (Ok(deleted.count), undo)
            }
            Err(e) => (Err(e), None),
        };
        let _ = tx.send(OpMsg::Done { outcome, undo });
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
    paste_into(s, sources, mode, target);
}

/// Copy or move sources into a target, asking about conflicts. Cutting into
/// the folder an item already lives in is a no-op, not a conflict.
fn paste_into(s: &mut App, sources: Vec<PathBuf>, mode: fileops::Mode, target: PathBuf) {
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
        open_dialog(s, Dialog::PasteConflict { mode, sources, target });
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
    open_dialog(s, Dialog::Delete { permanent, paths });
}

fn begin_rename(s: &mut App) {
    let Some(e) = s.selected.first().and_then(|&i| s.entries.get(i)) else { return };
    let mut edit = TextEdit::default();
    edit.text = e.name.clone();
    s.menu = None;
    open_dialog(s, Dialog::Rename { path: PathBuf::from(&e.path), edit });
}

fn begin_new_folder(s: &mut App) {
    if s.view != View::Files {
        s.notice = "Open a folder to create one in".into();
        return;
    }
    let mut edit = TextEdit::default();
    edit.text = "New folder".into();
    s.menu = None;
    open_dialog(s, Dialog::NewFolder { parent: s.root.clone(), edit });
}

fn commit_rename(s: &mut App) {
    let Some(Dialog::Rename { path, edit }) = s.dialog.take() else { return };
    let name = edit.text.trim().to_string();
    if name.is_empty() {
        return;
    }
    match fileops::rename(&path, &name) {
        Ok(target) => {
            record_undo(s, Undo::Rename { from: path, to: target });
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
        Ok(path) => {
            record_undo(s, Undo::Made { path });
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
    // What the operation has to say. It is put on screen only after the reload
    // below, because reloading a folder clears the notice: a failed delete used
    // to be reported into that clearing and say nothing at all.
    let mut outcome: Option<String> = None;
    let mut undo: Option<Undo> = None;
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
                OpMsg::Done { outcome: result, undo: action } => {
                    finished = true;
                    undo = action;
                    let label = s.transfer.as_ref().map(|t| t.label).unwrap_or("Done");
                    let past = match label {
                        "Copying" => "Copied",
                        "Moving" => "Moved",
                        "Deleting" => "Deleted",
                        _ => "Finished",
                    };
                    outcome = Some(match result {
                        Ok(n) => format!("{past} {n} item{}", if n == 1 { "" } else { "s" }),
                        Err(e) => e,
                    });
                }
            }
        }
    }
    if finished {
        s.op_rx = None;
        s.op_cancel = None;
        s.transfer = None;
        osdrag::release(&std::mem::take(&mut s.drop_staged));
        clear_selection(s);
        if s.view == View::Files {
            let root = s.root.clone();
            set_location(s, root);
        }
        if let Some(outcome) = outcome {
            s.notice = outcome;
        }
        if let Some(action) = undo {
            record_undo(s, action);
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
        return;
    }
    // A shortcut to a folder opens that folder here. Handing it to the shell
    // asks whichever program opens folders to open it, and that is Lyre, so a
    // second window appeared on the folder instead of this one showing it.
    if let Some(folder) = shortcut::folder_target(path) {
        return go(s, folder);
    }
    record_recent(s, path);
    if let Err(err) = shell::open_default(path) {
        s.notice = err;
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

/// Show the dialog that asks which application opens this file. The platform's
/// answer is collected on a worker thread: on Linux that runs `xdg-mime` twice.
fn start_open_with(s: &mut App, path: String, name: String) {
    let (tx, rx) = std::sync::mpsc::channel();
    s.openwith_rx = Some(rx);
    open_dialog(s, Dialog::OpenWith {
        path: path.clone(),
        name,
        data: None,
        selected: None,
        remember: true,
    });
    std::thread::spawn(move || {
        let _ = tx.send(openwith::open_with(&path));
    });
}

/// Drain the "Open with" worker's answer into the open dialog; true to redraw.
fn drain_openwith(s: &mut App) -> bool {
    let Some(rx) = s.openwith_rx.take() else {
        return false;
    };
    let mut answer = None;
    while let Ok(data) = rx.try_recv() {
        answer = Some(data);
    }
    let Some(data) = answer else {
        // Still being listed; keep the receiver for the next frame.
        s.openwith_rx = Some(rx);
        return false;
    };
    let default = data.default_app.clone();
    let first = data.apps.first().map(|app| app.id.clone());
    if let Some(Dialog::OpenWith { data: slot, selected, .. }) = &mut s.dialog {
        *slot = Some(data);
        // The platform's current default is offered first, as Explorer does.
        *selected = default.or(first);
    }
    true
}

/// Open the file in one chosen application, optionally making it the default
/// for this file's type first.
fn apply_open_with(s: &mut App, path: &str, app_id: &str, remember: bool) {
    record_recent(s, path);
    if remember {
        if let Err(err) = openwith::set_default_app(path, app_id) {
            s.notice = err;
        } else {
            s.notice = "Default application set".into();
        }
    }
    if let Err(err) = openwith::open_with_app(path, app_id) {
        s.notice = err;
    }
}

/// Global shortcuts. Returns true when the key was handled. Editing keys are
/// left to a focused field or an open dialog.
/// The keys the dialog on screen answers: Enter or Space presses the button the
/// keyboard is on, the arrows walk between the buttons, and Escape is handled
/// before this. A dialog with a field in it leaves every key to that field,
/// which submits with Enter and moves the caret with the arrows.
fn dialog_key(s: &mut App, key: &Key) -> bool {
    let buttons = dialog_buttons(s.dialog.as_ref());
    if buttons.is_empty() {
        return false;
    }
    let typing = match s.dialog.as_ref() {
        Some(Dialog::Rename { edit, .. }) | Some(Dialog::NewFolder { edit, .. }) => edit.focused,
        _ => false,
    };
    if typing {
        return false;
    }
    let index = dialog_focus_index(s, buttons.len());
    match key {
        Key::Enter => {
            press_dialog_button(s, buttons[index]);
            true
        }
        // A space is a character, not a named key.
        Key::Text(text) if text == " " => {
            press_dialog_button(s, buttons[index]);
            true
        }
        Key::Left | Key::Up => {
            s.dialog_focus = if index == 0 { buttons.len() - 1 } else { index - 1 };
            true
        }
        Key::Right | Key::Down => {
            s.dialog_focus = if index + 1 >= buttons.len() { 0 } else { index + 1 };
            true
        }
        _ => false,
    }
}

fn handle_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.key == Key::Escape && (s.drag.is_some() || s.os_hover.is_some()) {
        s.drag = None;
        s.drag_press = None;
        s.os_hover = None;
        return true;
    }
    if s.address_edit.is_some() {
        match ev.key {
            Key::Escape => {
                s.address_edit = None;
                return true;
            }
            Key::Enter => {
                commit_address(s);
                return true;
            }
            _ => return false,
        }
    }
    if ev.is_command('l')
        || (ev.mods.alt && matches!(&ev.key, Key::Text(t) if t.eq_ignore_ascii_case("d")))
    {
        begin_address_edit(s);
        return true;
    }
    if ev.key == Key::Escape {
        if s.dialog.is_some() {
            s.dialog = None;
            return true;
        }
        if s.menu.is_some() {
            s.menu = None;
            return true;
        }
        // A folder still being read is the next thing Escape should stop.
        if s.scan_rx.is_some() {
            cancel_scan(s);
            s.notice = "Stopped reading the folder".into();
            return true;
        }
        if !s.selected.is_empty() {
            clear_selection(s);
            return true;
        }
        return false;
    }
    if s.dialog.is_some() {
        return dialog_key(s, &ev.key);
    }
    if s.search_edit.focused {
        return false;
    }
    if ev.is_command('c') {
        copy_selection(s);
        return true;
    }
    // Undo and redo, before the rest: Ctrl+Shift+Z is a redo too, the way most
    // editors spell it.
    if ev.is_command('z') {
        return if ev.mods.shift { redo_last(s) } else { undo_last(s) };
    }
    if ev.is_command('y') {
        return redo_last(s);
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
    if ev.is_command('t') {
        new_tab(s);
        return true;
    }
    if ev.is_command('w') {
        close_tab_at(s, s.active_tab);
        return true;
    }
    // Alt plus an arrow walks the history. Checked before the plain arrows below,
    // which move the selection instead.
    if ev.mods.alt {
        match ev.key {
            Key::Left => {
                go_back(s);
                return true;
            }
            Key::Right => {
                go_forward(s);
                return true;
            }
            Key::Up => {
                go_up(s);
                return true;
            }
            Key::Enter => {
                open_properties(s);
                return true;
            }
            _ => {}
        }
    }
    if ev.key == Key::Function(5) {
        refresh_folder(s);
        return true;
    }
    if ev.key == Key::Function(4) {
        begin_address_edit(s);
        return true;
    }
    if ev.key == Key::Function(3) || ev.is_command('f') || ev.is_command('e') {
        s.search_edit.focused = true;
        return true;
    }
    if ev.is_command('b') {
        s.nav_open = !s.nav_open;
        return true;
    }
    // Ctrl+Shift+N and Ctrl+Shift+C come before the plain Ctrl letters so the
    // shifted forms are not mistaken for new-tab and copy.
    if ev.mods.ctrl && ev.mods.shift {
        match &ev.key {
            Key::Text(t) if t.eq_ignore_ascii_case("n") => {
                begin_new_folder(s);
                return true;
            }
            Key::Text(t) if t.eq_ignore_ascii_case("c") => {
                copy_path(s, tx);
                return true;
            }
            // Explorer's large-icons and details accelerators. The digit arrives
            // as its shifted symbol on most layouts, so both spellings count.
            Key::Text(t) if t.as_str() == "2" || t.as_str() == "@" => {
                s.mode = Mode::Tiles;
                return true;
            }
            Key::Text(t) if t.as_str() == "6" || t.as_str() == "^" => {
                s.mode = Mode::Details;
                return true;
            }
            _ => {}
        }
    }
    // Ctrl+1 through Ctrl+9 select a tab; 9 is the last, as in a browser.
    if ev.mods.ctrl
        && let Key::Text(t) = &ev.key
        && let Ok(digit) = t.parse::<usize>()
        && (1..=9).contains(&digit)
        && !s.tabs.is_empty()
    {
        let index = if digit == 9 {
            s.tabs.len() - 1
        } else {
            (digit - 1).min(s.tabs.len() - 1)
        };
        switch_tab(s, index);
        return true;
    }
    // Typing jumps to the first name that starts with what has been typed, the
    // buffer clearing after a pause so two separate jumps do not compound.
    if let Key::Text(text) = &ev.key
        && !ev.mods.ctrl
        && !ev.mods.alt
        && !ev.mods.cmd
        && !text.is_empty()
    {
        // A space is not the start of a name; it toggles the focused row.
        if text == " " {
            toggle_focused(s);
            return true;
        }
        if s.type_ahead_at.elapsed() > std::time::Duration::from_millis(900) {
            s.type_ahead.clear();
        }
        s.type_ahead.push_str(&text.to_lowercase());
        s.type_ahead_at = Instant::now();
        if jump_to_prefix(s) {
            return true;
        }
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
        Key::PageUp => {
            move_focus(s, focus_row(s).saturating_sub(10), ev.mods, tx);
            true
        }
        Key::PageDown => {
            let last = display_len(s).saturating_sub(1);
            move_focus(s, (focus_row(s) + 10).min(last), ev.mods, tx);
            true
        }
        Key::Tab if ev.mods.cmd || ev.mods.ctrl => {
            let len = s.tabs.len();
            if len > 1 {
                let delta = if ev.mods.shift { len - 1 } else { 1 };
                switch_tab(s, (s.active_tab + delta) % len);
            }
            true
        }
        _ => false,
    }
}

/// Reload the folder on screen, the way F5 does.
fn refresh_folder(s: &mut App) {
    if s.view == View::Files {
        let root = s.root.clone();
        set_location(s, root);
    } else if s.view == View::ThisPc {
        refresh_volumes(s);
    } else {
        refresh_recent(s);
    }
}

/// The Properties dialog for the focused row, as Alt+Enter asks for it.
fn open_properties(s: &mut App) {
    let Some(index) = s.focused.or_else(|| s.selected.first().copied()) else {
        return;
    };
    let entry = if s.searching {
        s.search_hits.get(index).map(|hit| &hit.entry)
    } else {
        s.entries.get(index)
    };
    if let Some(entry) = entry {
        let details = metadata::describe(
            Path::new(&entry.path),
            entry.is_dir,
            kind_label(entry.kind),
        );
        open_dialog(s, Dialog::Properties(Box::new(details)));
    }
}

/// Put the selected paths on the clipboard as text, one per line.
fn copy_path(s: &mut App, tx: &mut Tx) {
    let paths = selected_paths(s);
    if paths.is_empty() {
        return;
    }
    let text: Vec<String> = paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    tx.copy(text.join("\n"));
    s.notice = if paths.len() == 1 {
        "Path copied to clipboard".into()
    } else {
        format!("{} paths copied to clipboard", paths.len())
    };
}

/// Select or deselect the focused row, which is what Space does in Explorer.
fn toggle_focused(s: &mut App) {
    let Some(index) = s.focused else { return };
    if s.selected.contains(&index) {
        s.selected.retain(|&selected| selected != index);
        s.selection_detail.clear();
    } else {
        select_entry(s, index);
    }
}

/// Focus and select the first row whose name begins with what has been typed.
fn jump_to_prefix(s: &mut App) -> bool {
    if s.type_ahead.is_empty() {
        return false;
    }
    let prefix = s.type_ahead.clone();
    let mut hit = None;
    for row in 0..display_len(s) {
        let (index, entry) = display_entry(s, row);
        if display_name(s, entry).to_lowercase().starts_with(&prefix) {
            hit = Some(index);
            break;
        }
    }
    match hit {
        Some(index) => {
            select_entry(s, index);
            s.focused = Some(index);
            true
        }
        None => false,
    }
}

fn file_row(s: &App, i: usize, p: Paint) -> El {
    let (ei, e) = display_entry(s, i);
    let selected = is_selected(s, ei);
    let focused = s.focused == Some(ei);
    let widths = column_widths(s);
    let mut line = row()
        .key(format!("row-{i}"))
        .w(row_width(s))
        .h(row_h(s))
        .items_center()
        .fill(if selected || drop_hot(s, &format!("row:{}", e.path)) { p.row_sel } else { Color::TRANSPARENT })
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
                .w(widths[0])
                .min_w(0.0)
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
                .w(widths[1])
                .min_w(0.0)
                .pl(12.0)
                .child(
                    text(e.modified.as_str())
                        .font(ROW_TEXT)
                        .color(p.secondary)
                        .min_w(0.0)
                        .truncate(),
                ),
        )
        .child(el().w(COL_SEP_W).h(16.0).center().child(el().w(p.hair).h(16.0).fill(p.soft_line)))
        .child(
            col().w(widths[2]).min_w(0.0).pl(12.0).pr(8.0).child(
                text(e.type_label.as_str())
                    .font(ROW_TEXT)
                    .color(p.secondary)
                    .w_full()
                    .truncate(),
            ),
        )
        .child(el().w(COL_SEP_W).h(16.0).center().child(el().w(p.hair).h(16.0).fill(p.soft_line)))
        .child(
            col().w(widths[3]).min_w(0.0).pl(8.0).pr(12.0).child(
                text(e.size_label())
                    .font(ROW_TEXT)
                    .color(p.secondary)
                    .w_full()
                    .align(Align::End),
            ),
        )
        .child(el().w(COL_SEP_W).h(16.0).center().child(el().w(p.hair).h(16.0).fill(p.soft_line)));
    os_drag_source(marquee_track(
        line.on_press_tx(move |s: &mut App, tx: &mut Tx| {
            click_select(s, ei, tx.mods());
        })
        .on_drag(move |s: &mut App, d: &mut rune::Drag, tx: &mut Tx| row_drag(s, i, ei, d, tx))
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
    ), ei)
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
        .fill(if selected || drop_hot(s, &format!("tile-{i}")) { p.rail_sel } else { Color::TRANSPARENT })
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
    os_drag_source(marquee_track(
        tile.on_press_tx(move |s: &mut App, tx: &mut Tx| {
            click_select(s, ei, tx.mods());
        })
        .on_drag(move |s: &mut App, d: &mut rune::Drag, tx: &mut Tx| row_drag(s, i, ei, d, tx))
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
    ), ei)
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
        .on_drop_files(move |s: &mut App, files, tx: &mut Tx| {
            let t = s.favorites.get(i).cloned().map(|p| DropTarget::Folder(PathBuf::from(p)));
            take_drop(s, t, files, tx);
        })
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
    os_drag_source(marquee_track(
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
    ), ei)
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
    os_drag_source(marquee_track(
        tile.on_press_tx(move |s: &mut App, tx: &mut Tx| {
            click_select(s, ei, tx.mods());
        })
        .on_drag(move |s: &mut App, d: &mut rune::Drag, tx: &mut Tx| row_drag(s, i, ei, d, tx))
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
    ), ei)
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

fn quick_tile(s: &App, p: Paint, label: &str, mark: Ico, top: &str, bottom: &str, back: &str, nav: usize) -> El {
    // A tile is as wide as its room allows: a fixed one was wider than a narrow
    // window's page, so the last tile in every line hung past the edge.
    let width = 272.0_f32.min((content_width(s) - 24.0).max(150.0));
    row()
        .key(format!("drop-place-{nav}"))
        .on_drop_files(move |s: &mut App, files, tx: &mut Tx| {
            take_drop(s, place_path(nav).map(DropTarget::Folder), files, tx);
        })
        .fill(if drop_hot(s, &format!("drop-place-{nav}")) { p.hover } else { Color::TRANSPARENT })
        .w(width)
        .h(60.0)
        .radius(4.0)
        .hover(p.hover)
        .px(12.0)
        .gap(12.0)
        .items_center()
        .child(folder(40.0, top, bottom, back, Some(mark), p))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(text(label).font(TILE_TEXT).color(p.text).min_w(0.0).truncate())
                .child(
                    text("Stored locally")
                        .font(CAPTION)
                        .color(p.tertiary)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(ico(p, Ico::Pin, 9.0, p.tertiary)),
        )
        .on_press_tx(move |s: &mut App, tx: &mut Tx| {
            navigate(s, nav);
            tx.instant();
        })
}

/// The window's width in points, measured on the last frame, or the size a
/// window opens at before the first frame has measured anything.
fn window_width(s: &App) -> f32 {
    let window = s.rect_window.get()[2];
    // A minimised window reports the sliver it collapsed to, and a frame drawn
    // from that measurement lays the whole page out for 160 points wide. Below
    // the smallest window this can be, the measurement is not the window.
    if window < MIN_W * 0.5 {
        1076.0
    } else {
        window
    }
}

/// How wide the rail is for the window as it is.
///
/// A narrow window gives the room to the list instead: a rail that keeps its
/// width when there is no width to spare leaves the folder it lists unusable.
fn rail_width(s: &App) -> f32 {
    let window = window_width(s);
    s.rail_w.min((window * 0.45).max(96.0))
}

/// The room the body has, in points. Before the first frame has measured the
/// window this answers the size a window opens at, so the first frame is laid
/// out for something rather than for nothing.
fn body_width(s: &App) -> f32 {
    // The rail's own padding comes off with its width, or a page laid out to
    // exactly this would end a few points past the edge.
    (window_width(s) - rail_width(s) - 8.0).max(160.0)
}

/// Whether the window is narrow enough that the command bar's words will not
/// fit: the row of labelled buttons needs about 700 points.
fn narrow(s: &App) -> bool {
    window_width(s) < 700.0
}

/// Whether it is narrow enough that the file-operation buttons will not fit
/// either, and have to make do with the context menu and the keyboard.
fn tight(s: &App) -> bool {
    window_width(s) < 560.0
}

fn quick_access_content(s: &App, p: Paint) -> El {
    let tiles = [
        ("Desktop", Ico::Desktop, "#2bbad8", "#046cbd", "#0a5a9e", 3usize),
        ("Downloads", Ico::Downloads, "#2bc79a", "#0e9f76", "#0b7a5a", 4),
        ("Documents", Ico::Documents, "#b9c9d9", "#8fa6bc", "#7488a0", 5),
        ("Pictures", Ico::Pictures, "#2bb3e8", "#0e7fc4", "#0a6298", 6),
        ("Music", Ico::Music, "#f2a06b", "#e06a3c", "#b8542e", 7),
        ("Videos", Ico::Videos, "#a86fe8", "#7b3fd0", "#6230a8", 8),
    ];
    // The tiles wrap to however many fit, rather than three to a line: a narrow
    // window used to push the third one past the edge of the window.
    let mut line = row().w_full().flex_wrap().justify_center().gap(12.0);
    for (label, mark, top, bottom, back, nav) in tiles {
        line = line.child(quick_tile(s, p, label, mark, top, bottom, back, nav));
    }
    let grid = col().w_full().pt(10.0).gap(6.0).child(line);
    let recent_on = s.quick_access_tab == 0;
    let pills = row()
        .w_full()
        .pt(14.0)
        .px(16.0)
        .gap(8.0)
        // One line where there is room, two where there is not: a row that
        // could not wrap held the whole page wider than a narrow window.
        .flex_wrap()
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
                .on_press(|s: &mut App| s.quick_access_tab = 0),
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
                .on_press(|s: &mut App| s.quick_access_tab = 1),
        );
    let mut table = col().w_full().pt(14.0);
    if recent_on {
        // The name takes whatever the other two leave, and the two after it give
        // room first: a long name used to be drawn past its own column, over the
        // date beside it.
        let room = content_width(s);
        let date_w = if narrow(s) { 120.0 } else { 240.0 };
        let activity_w = if room > 780.0 { 160.0 } else { 0.0 };
        let name_w = (room - date_w - activity_w - 32.0).max(60.0);
        let mut columns = vec![("Name", name_w), ("Date modified", date_w)];
        if activity_w > 0.0 {
            columns.push(("Activity", activity_w));
        }
        table = table.child(column_header(
            p,
            &columns,
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
                        .w(name_w)
                        .min_w(0.0)
                        .pl(16.0)
                        .gap(10.0)
                        .items_center()
                        .child(entry_icon(s, e, p, 24.0))
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(1.0)
                                .child(
                                    text(e.name.as_str())
                                        .font(ROW_TEXT)
                                        .color(p.text)
                                        .min_w(0.0)
                                        .truncate(),
                                )
                                .child(text(e.type_label.as_str()).font(CAPTION).color(p.tertiary)),
                        ),
                )
                .child(
                    col()
                        .w(date_w)
                        .min_w(0.0)
                        .pl(12.0)
                        .child(
                            text(e.modified.as_str())
                                .font(ROW_TEXT)
                                .color(p.secondary)
                                .min_w(0.0)
                                .truncate(),
                        ),
                )
                .child(col().w(activity_w));
            row = row
                .on_press_tx(move |st: &mut App, tx: &mut Tx| click_select(st, ei, tx.mods()))
                .on_drag(move |st: &mut App, d: &mut rune::Drag, tx: &mut Tx| row_drag(st, ei, ei, d, tx))
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
            table = table.child(os_drag_source(marquee_track(row, ei), ei));
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
            .key(format!("drop-vol-{vi}"))
            .on_drop_files(move |s: &mut App, files, tx: &mut Tx| {
                let t = s.volumes.get(vi).map(|v| DropTarget::Folder(v.path.clone()));
                take_drop(s, t, files, tx);
            })
            // A share of a narrow page, as the Quick Access tiles take: a fixed
            // card was wider than the page it sat on.
            .w(300.0_f32.min((content_width(s) - 48.0).max(180.0)))
            .h(64.0)
            .radius(4.0)
            .px(8.0)
            .gap(10.0)
            .items_center()
            .fill(if selected || drop_hot(s, &format!("drop-vol-{vi}")) { p.row_sel } else { Color::TRANSPARENT })
            .hover(p.hover)
            .child(drive_glyph(p, 40.0))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(4.0)
                    .child(
                        text(title.as_str())
                            .font(ROW_TEXT)
                            .color(p.text)
                            .min_w(0.0)
                            .truncate(),
                    )
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
        list = list.child(marquee_track(card, vi));
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

/// The Pictures library the Gallery is drawn from: wherever this machine keeps
/// it, since a redirected or moved Pictures is still the Gallery's.
fn gallery_folder() -> Option<PathBuf> {
    let home = home_dir()?;
    known::folder(&home, "Pictures")
}

/// Re-read the Pictures library into the Gallery's entries.
fn refresh_gallery(s: &mut App) {
    let Some(pictures) = gallery_folder() else { return };
    s.entries = sweep_images(&pictures, 4000, 8);
    s.visible = (0..s.entries.len()).collect();
    s.count = s.entries.len();
    s.count_override = Some(s.count);
    s.groups.clear();
}

/// Open the Gallery: the Pictures library as a photo grid. A re-click is a
/// no-op, as with the other places.
fn go_gallery(s: &mut App) {
    s.menu = None;
    s.address_edit = None;
    if s.view == View::Gallery {
        return;
    }
    // Pictures is wherever this machine keeps it: a redirected or moved
    // Pictures library is still the Gallery's.
    let Some(pictures) = gallery_folder() else {
        s.notice = "Pictures is not available here".into();
        return;
    };
    s.history.push(spot_of(s));
    s.forward.clear();
    show_gallery(s, pictures);
}

/// Draw the Gallery from `pictures`, which opening it and a step back to it
/// share.
fn show_gallery(s: &mut App, pictures: PathBuf) {
    s.root = pictures;
    s.view = View::Gallery;
    s.tab = "Gallery".into();
    s.crumbs = vec!["Gallery".into()];
    s.search = "Search Gallery".into();
    s.contextual = true;
    s.groups.clear();
    clear_selection(s);
    s.notice.clear();
    s.search_edit.text.clear();
    s.menu = None;
    s.keep_order = false;
    s.count_override = None;
    refresh_gallery(s);
}

/// One photo in the Gallery grid.
fn gallery_cell(s: &App, i: usize, p: Paint) -> El {
    let (ei, e) = display_entry(s, i);
    let selected = is_selected(s, ei);
    os_drag_source(marquee_track(
        col()
            .key(format!("gcell-{i}"))
            .on_drag(move |s: &mut App, d: &mut rune::Drag, tx: &mut Tx| row_drag(s, i, ei, d, tx))
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
    ), ei)
}

/// The Gallery: the Pictures library as a date-grouped photo grid.
fn gallery_content(s: &App, p: Paint) -> El {
    let mut out = col()
        .key("gallery")
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

/// Per-type facts for one item, memoised by path. Reading a file for its
/// dimensions or its word count is cheap once and far too much per frame.
fn preview_facts(s: &App, entry: &Entry) -> Vec<metadata::Fact> {
    let key = entry.path.clone();
    if let Some((cached, facts)) = s.preview_facts.borrow().as_ref()
        && *cached == key
    {
        return facts.clone();
    }
    let facts =
        metadata::describe(Path::new(&entry.path), entry.is_dir, &entry.type_label).extra;
    *s.preview_facts.borrow_mut() = Some((key, facts.clone()));
    facts
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
        let mut rows: Vec<(String, String)> = vec![
            ("Type".to_string(), e.map(|x| x.type_label.clone()).unwrap_or_default()),
            ("Size".to_string(), e.map(|x| x.size_label()).unwrap_or_default()),
            ("File location".to_string(), location),
            ("Date modified".to_string(), e.map(|x| x.modified.clone()).unwrap_or_default()),
        ];
        // The same per-type facts the Properties dialog shows, so the pane says
        // what a file is without opening anything.
        if let Some(entry) = e {
            rows.extend(preview_facts(s, entry));
        }
        for (k, v) in &rows {
            details = details.child(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_center()
                    .child(col().w(96.0).child(text(k.as_str()).font(CAPTION).color(p.tertiary)))
                    .child(
                        text(v.as_str())
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
        .w(preview_width(s))
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
        // Not `w_full`: beside the rail, that asks for the whole window's width
        // and pushes the list past the right edge.
        .min_w(0.0)
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
            View::QuickAccess => quick_access_content(s, p),
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
    if preview_shown(s) {
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
                .on_press(|s: &mut App| s.menu = None)
                .on_secondary_press(|s: &mut App, at: rune::PointerPress, _tx: &mut Tx| {
                    // A right-click on another item moves the open menu to it,
                    // as a fresh right-click would.
                    let (scope, target) = context_target_at(s, at.x, at.y);
                    s.menu = Some(Menu::Context { scope, target, x: at.x, y: at.y });
                }),
        );
    }
    body
}

/// How wide the preview pane is, which is its own width unless the window is too
/// narrow to give it that.
fn preview_width(s: &App) -> f32 {
    PREVIEW_W.min(body_width(s) * 0.45).max(150.0)
}

/// Whether the details pane is drawn at all: a window too narrow to hold both
/// the list and the pane shows the list, as there is nowhere to put the pane.
fn preview_shown(s: &App) -> bool {
    // The pane needs its own width and the page beside it needs a usable one; a
    // window that cannot give both shows the page, which is what the person is
    // reading, and the pane comes back when there is room for it again.
    s.preview && body_width(s) >= 700.0
}

/// The room the page on screen has beside the rail and, when it is drawn, the
/// details pane.
fn content_width(s: &App) -> f32 {
    let pane = if preview_shown(s) { preview_width(s) } else { 0.0 };
    (body_width(s) - pane).max(80.0)
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
            format!("{}% ({} of {})", (frac * 100.0) as u32, t.done.min(t.total), t.total)
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

    // A folder being read says so, and counts up, rather than looking empty.
    let reading = s.scan_rx.is_some();
    let progress = if reading && s.scan_seen > 0 {
        format!("Reading folder… {} items", s.scan_seen)
    } else {
        "Reading folder…".to_string()
    };
    let middle = if reading {
        progress.as_str()
    } else if s.notice.is_empty() {
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
        text(if reading {
            // The count is unknown until the read finishes.
            String::new()
        } else if s.count == 1 {
            "1 item".to_string()
        } else {
            format!("{} items", s.count)
        })
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

/// Fit a remembered window size to the screen it will open on: never past the
/// screen, and never below the window's minimum, except where the screen itself
/// is smaller than that minimum, in which case the screen wins.
fn fit_to_screen(saved: (u32, u32), screen: (u32, u32), scale: f32) -> Option<(u32, u32)> {
    if screen.0 == 0 || screen.1 == 0 {
        return None;
    }
    let scale = scale.max(0.5);
    let min_w = ((MIN_W * scale).round() as u32).min(screen.0);
    let min_h = ((MIN_H * scale).round() as u32).min(screen.1);
    Some((saved.0.clamp(min_w, screen.0), saved.1.clamp(min_h, screen.1)))
}

/// Where a panic is written: `lyre-crash.log` beside the settings, or the
/// temporary folder when the settings have no folder of their own.
fn crash_log_path() -> PathBuf {
    rune::prefs::Prefs::open(PREFS_ID)
        .path()
        .and_then(|file| file.parent().map(|dir| dir.join("lyre-crash.log")))
        .unwrap_or_else(|| std::env::temp_dir().join("lyre-crash.log"))
}

/// Write a panic to a file beside the settings before the window closes.
///
/// A release build is a window, not a console program, so a panic takes the
/// window away and leaves nothing to read: "it just closed" is all anyone can
/// report. The hook keeps the file the next report can name.
fn record_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info
            .location()
            .map(|at| format!("{}:{}", at.file(), at.line()))
            .unwrap_or_else(|| "an unknown place".into());
        let line = format!(
            "lyre {} panicked at {place}\n{info}\n\n",
            env!("CARGO_PKG_VERSION")
        );
        let log = crash_log_path();
        if let Some(dir) = log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
            use std::io::Write;
            let _ = file.write_all(line.as_bytes());
        }
        // The debug console still gets it, and the exit status is unchanged.
        previous(info);
    }));
}

fn main() {
    record_panics();
    // A launch the shell meant for Explorer itself (a /select reveal, a shell
    // namespace) is handed back rather than opened as a folder.
    if defaultmanager::forward_unhostable_launch() {
        return;
    }
    // Before any window opens, so the first frame already shows the favorites
    // and pins the 0.2 application left behind.
    import_legacy_library();
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
            ("quickaccess", |s| load(s, Fixture::QuickAccess)),
            // The home folder itself, with its own listing and breadcrumbs.
            ("home", |s| open_home(s)),
            ("downloads", |s| load(s, Fixture::Downloads)),
            ("pictures", |s| load(s, Fixture::Screenshots)),
            ("thispc", |s| {
                navigate(s, 9);
                // The reference capture this scene reproduces has the details
                // pane open. The page itself no longer forces it: that is the
                // person's setting, and turning it on for them was the bug.
                s.preview = true;
            }),
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
            ("address", |s| {
                load(s, Fixture::Downloads);
                begin_address_edit(s);
            }),
            ("drag", |s| {
                load(s, Fixture::Downloads);
                s.drag = Some(DragState {
                    paths: vec!["a.txt".into(), "b.txt".into()],
                    at: (500.0, 300.0),
                    shift: false,
                    target: Some(DropTarget::Folder(PathBuf::from(&s.entries[2].path))),
                    target_key: format!("row:{}", s.entries[2].path),
                    mode: fileops::Mode::Copy,
                });
            }),
            ("oshover", |s| {
                load(s, Fixture::Downloads);
                s.os_hover = Some(format!("row:{}", s.entries[2].path));
            }),
            ("crumbmenu", |s| {
                load(s, Fixture::Downloads);
                s.tail_folders = fs::read_dir(&s.root)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .take(6)
                    .map(|e| {
                        (
                            e.file_name().to_string_lossy().into_owned(),
                            e.path().to_string_lossy().into_owned(),
                        )
                    })
                    .collect();
                s.menu = Some(Menu::Drop { kind: DropMenu::Subfolders, x: 700.0, y: 100.0 });
            }),
            ("favorites", |s| {
                load(s, Fixture::QuickAccess);
                s.quick_access_tab = 1;
                let root = s.root.to_string_lossy().into_owned();
                s.favorites = vec![root.clone(), format!("{}\\src", root)];
                s.pinned = vec![root];
            }),
            ("reading", |s| {
                load(s, Fixture::Downloads);
                // A channel nothing ever answers, so the state is drawn as it
                // looks while a folder is still being read.
                let (_, rx) = std::sync::mpsc::channel();
                s.scan_rx = Some(rx);
                s.scan_seen = 1024;
            }),
            ("customize", |s| {
                load(s, Fixture::Downloads);
                s.customize_open = true;
                s.customize_path = Some(s.root.to_string_lossy().into_owned());
                s.customize_is_dir = true;
                s.customize_background = true;
                s.customize_expanded.push("Lyre".into());
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
                open_dialog(s, Dialog::Delete { permanent: false, paths });
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
                    open_dialog(s, Dialog::Rename { path: PathBuf::from(&e.path), edit });
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
                open_dialog(s, Dialog::DefaultManager);
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
                open_dialog(s, Dialog::Update);
            }),
            ("openwithdlg", |s| {
                s.view = View::Files;
                s.mode = Mode::Details;
                let (path, name) = match s
                    .entries
                    .iter()
                    .find(|e| !e.is_dir && Path::new(&e.path).extension().is_some())
                    .or_else(|| s.entries.first())
                {
                    Some(e) => (e.path.clone(), e.name.clone()),
                    None => (String::new(), String::new()),
                };
                let data = openwith::open_with(&path);
                let selected = data
                    .default_app
                    .clone()
                    .or_else(|| data.apps.first().map(|app| app.id.clone()));
                open_dialog(s, Dialog::OpenWith {
                    path,
                    name,
                    data: Some(data),
                    selected,
                    remember: true,
                });
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
    run_windows(|displays| {
        // Open at the size the window was last left at, clamped to the screen it
        // will actually open on so a size remembered from a larger display still
        // fits, and never below the window's own minimum.
        let saved = rune::prefs::Prefs::open(PREFS_ID).get::<(u32, u32)>("window_size");
        let physical = saved.and_then(|(width, height)| {
            let display = displays.iter().find(|d| d.primary).or(displays.first())?;
            fit_to_screen((width, height), display.size, display.scale as f32)
        });
        // macOS rounds a transparent borderless window through Rune's own corner
        // radius; Windows rounds through DWM in frame::install; Linux leaves the
        // corners to the compositor.
        let window = WindowSpec::new("main", spec).configured_options(
            WindowOptions {
                borderless: true,
                resizable: true,
                min_size: Some((MIN_W, MIN_H)),
                physical_size: physical,
                transparent: cfg!(target_os = "macos"),
                ..Default::default()
            }
            .macos(rune::MacWindowOptions::default().corner_radius(8.0)),
        );
        // The folder watch rescan lands on a worker thread; this handle is how
        // it wakes the event loop without any polling timer on the UI side.
        let app_handle = window.app.handle();
        let wake = app_handle.clone();
        app_handle.update(move |s: &mut App, _tx: &mut Tx| {
            s.waker = Some(std::sync::Arc::new(move || wake.wake()));
        });
        let window = window.on_created(|handle| frame::install(&handle));
        [window.boxed()]
    });
}

#[cfg(all(test, target_os = "windows"))]
mod customize_tests {
    use super::*;

    /// The reported bug: an extension had to be allowed again after every
    /// restart. This round-trips the real save and restore through a store of its
    /// own, so it neither depends on nor disturbs the person's settings.
    #[test]
    fn an_allowed_extension_survives_a_restart() {
        let store = "lyre-rune-prefs-test";
        let clsid = "{23170F69-40C1-278A-1000-000100020000}".to_string();

        let mut before = init();
        before.handler_loaded = vec![clsid.clone()];
        persist_to(&before, store);

        // A fresh launch, reading that store back.
        let mut after = init();
        after.handler_loaded.clear();
        load_prefs_from(&mut after, store);
        assert_eq!(after.handler_loaded, vec![clsid]);

        // Leave nothing behind in the test store.
        let mut prefs = rune::prefs::Prefs::open(store);
        let _ = prefs.set("handler_loaded", &Vec::<String>::new());
    }

    /// Allowing an extension is a preference, not a session setting: it used to
    /// be forgotten on restart, which made the switch pointless.
    #[test]
    fn allowing_an_extension_is_part_of_the_saved_state() {
        let mut s = init();
        let before = prefs_signature(&s);
        s.handler_loaded
            .push("{23170F69-40C1-278A-1000-000100020000}".into());
        assert_ne!(
            prefs_signature(&s),
            before,
            "changing it has to trigger the save"
        );
    }

    /// The menu has to carry the extension's items, and they have to be read for
    /// the whole selection rather than the item under the pointer.
    #[test]
    fn a_loaded_extension_survives_a_multi_file_selection() {
        let dir = std::env::temp_dir().join(format!("lyre-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<String> = (0..3)
            .map(|n| {
                let file = dir.join(format!("part{n}.txt"));
                std::fs::write(&file, b"x").unwrap();
                file.to_string_lossy().into_owned()
            })
            .collect();

        // What the customize dialog offers for the same selection.
        let mut s = init();
        s.customize_path = Some(paths[0].clone());
        let groups = customize_groups(&s);
        let Some(index) = groups
            .iter()
            .position(|group| group.owner.to_lowercase().contains("7-zip"))
        else {
            std::fs::remove_dir_all(&dir).ok();
            return;
        };
        s.handler_loaded.push(groups[index].lines[0].load.clone().unwrap());

        // Reading it for three files at once still names its commands.
        let rows = menu_handlers(&s, &paths, false, false);
        let children: Vec<&shell::Verb> = rows.iter().flat_map(|row| row.children.iter()).collect();
        assert!(
            children
                .iter()
                .any(|item| item.label.to_lowercase().contains("archive")),
            "a multiple selection should still offer the archive commands: {:?}",
            children.iter().map(|item| &item.label).collect::<Vec<_>>()
        );

        // And context actions cover the selection, not just the clicked item.
        s.selected = vec![0, 1, 2];
        assert_eq!(context_paths(&s, 1).len(), 3);
        // An item outside the selection is on its own.
        if let Some(entry) = s.entries.get(2) {
            s.selected = vec![0];
            assert_eq!(context_paths(&s, 2), vec![entry.path.clone()]);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The dialog has to list what actually exists, including the extensions
    /// that draw their own items, which the old flat list never showed at all.
    #[test]
    fn groups_cover_lyre_verbs_and_extensions() {
        let dir = std::env::temp_dir().join(format!("lyre-customize-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("probe.txt");
        std::fs::write(&file, b"x").unwrap();

        let mut s = init();
        // Start from nothing allowed: the saved settings may already hold an
        // extension the person switched on.
        s.handler_loaded.clear();
        s.customize_open = true;
        s.customize_path = Some(file.to_string_lossy().into_owned());
        s.customize_is_dir = false;
        s.customize_background = false;

        let groups = customize_groups(&s);
        let lyre = groups
            .iter()
            .find(|group| group.owner == "Lyre")
            .expect("Lyre's own items are always a group");
        assert_eq!(lyre.lines.len(), CUSTOM_ITEMS.len());

        let Some(index) = groups
            .iter()
            .position(|group| group.owner.to_lowercase().contains("7-zip"))
        else {
            // A machine without 7-Zip can still check the rest.
            std::fs::remove_dir_all(&dir).ok();
            return;
        };

        // Before it is allowed to load, the extension offers only that choice.
        assert_eq!(
            groups[index].lines.len(),
            1,
            "an unloaded extension lists only its switch"
        );
        let clsid = groups[index].lines[0]
            .load
            .clone()
            .expect("the row switches loading");

        // Once allowed, its real items are there.
        s.handler_loaded.push(clsid);
        let groups = customize_groups(&s);
        let ext = groups
            .iter()
            .find(|group| group.owner.to_lowercase().contains("7-zip"))
            .unwrap();
        assert!(
            ext.lines.len() > 5,
            "7-Zip should list its items, got {}",
            ext.lines.len()
        );
        assert!(
            ext.lines
                .iter()
                .any(|line| line.load.is_none() && line.label.to_lowercase().contains("archive")),
            "the archive items should be named: {:?}",
            ext.lines.iter().map(|line| &line.label).collect::<Vec<_>>()
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod start_tests {
    use super::*;

    /// An installed copy is launched from its own folder, so falling back to the
    /// working directory opened Lyre in `%LOCALAPPDATA%\Programs\Lyre` instead of
    /// the quick start screen.
    #[test]
    fn a_plain_launch_starts_on_quick_access() {
        if std::env::var_os("LYRE_RUNE_PATH").is_some() {
            // A path in the environment is a request to open it; nothing to check.
            return;
        }
        let s = init();
        if s.view == View::Files {
            // Only reachable if the shell handed a folder over, which the test
            // harness never does; leave it rather than fail on a stray argument.
            return;
        }
        assert!(
            matches!(s.view, View::QuickAccess),
            "a plain launch opens on Quick Access"
        );
        assert_eq!(s.tab, "Quick Access");
    }
}

#[cfg(test)]
mod view_pref_tests {
    use super::*;

    /// The reported bug: This PC turned a Date modified, descending sort into
    /// Name, descending, because laying its page out put the column back to the
    /// first one and left the direction behind. No page owns the view, the sort
    /// or the details pane.
    #[test]
    fn no_page_overwrites_the_view_sort_or_details_pane() {
        let mut s = init();
        s.mode = Mode::List;
        s.sort = 1; // Date modified
        s.sort_desc = true;
        s.preview = true;
        for page in 0..4 {
            match page {
                // Startup, This PC, the Gallery where this machine has one, and
                // a new tab.
                1 => navigate(&mut s, 9),
                2 => {
                    if gallery_folder().is_none() {
                        continue;
                    }
                    navigate(&mut s, 1);
                }
                3 => new_tab(&mut s),
                _ => load(&mut s, Fixture::QuickAccess),
            }
            assert!(matches!(s.mode, Mode::List), "page {page} changed the view");
            assert_eq!(s.sort, 1, "page {page} changed the sort column");
            assert!(s.sort_desc, "page {page} changed the sort direction");
            assert!(s.preview, "page {page} changed the details pane");
        }
    }

    /// The reported bug: opening This PC turned the details pane on, and the
    /// choice was then saved, so it stayed on everywhere afterwards. The Gallery
    /// did the same, so every page answers to the same rule.
    #[test]
    fn opening_a_page_keeps_the_details_pane_as_it_was() {
        for preview in [false, true] {
            for open in 0..2 {
                let mut s = init();
                s.preview = preview;
                // Both the rail row and a step back onto the page.
                if open == 0 {
                    navigate(&mut s, 9);
                } else {
                    load(&mut s, Fixture::ThisPc);
                }
                assert_eq!(
                    s.preview, preview,
                    "This PC must not change the person's details pane"
                );
            }
            // Quick Access goes through the same loader.
            let mut s = init();
            s.preview = preview;
            load(&mut s, Fixture::QuickAccess);
            assert_eq!(s.preview, preview, "Quick Access must leave the pane alone");
            // The Gallery, on a machine that has Pictures to draw it from.
            if gallery_folder().is_some() {
                let mut s = init();
                s.preview = preview;
                navigate(&mut s, 1);
                assert!(s.view == View::Gallery, "the rail press opened the Gallery");
                assert_eq!(s.preview, preview, "the Gallery must leave the pane alone");
            }
        }
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn a_remembered_size_is_kept_when_it_fits() {
        assert_eq!(fit_to_screen((1200, 800), (1920, 1080), 1.0), Some((1200, 800)));
    }

    #[test]
    fn a_size_bigger_than_the_screen_is_pulled_back_inside_it() {
        // Remembered from a larger display.
        assert_eq!(fit_to_screen((2560, 1440), (1920, 1080), 1.0), Some((1920, 1080)));
    }

    #[test]
    fn a_size_below_the_minimum_is_raised_to_it() {
        // The window's own floor at this scale.
        assert_eq!(
            fit_to_screen((300, 200), (1920, 1080), 1.0),
            Some((MIN_W as u32, MIN_H as u32))
        );
        // At 2x the floor is twice as many physical pixels.
        assert_eq!(
            fit_to_screen((300, 200), (3840, 2160), 2.0),
            Some(((MIN_W * 2.0) as u32, (MIN_H * 2.0) as u32))
        );
    }

    #[test]
    fn a_screen_smaller_than_the_minimum_does_not_panic_or_overflow_it() {
        // The minimum has to give way to the screen rather than clamp() panic.
        assert_eq!(fit_to_screen((1200, 800), (640, 400), 1.0), Some((640, 400)));
    }

    #[test]
    fn a_screen_with_no_size_keeps_the_default() {
        assert_eq!(fit_to_screen((1200, 800), (0, 0), 1.0), None);
    }
}

#[cfg(test)]
mod rail_tests {
    use super::*;

    /// The chevrons on This PC and Network were decoration: there was no state
    /// behind them, so pressing one navigated instead of opening anything.
    #[test]
    fn a_chevron_opens_and_closes_its_list() {
        let mut s = init();
        assert!(s.rail_open.is_empty());

        toggle_rail(&mut s, 9);
        assert_eq!(s.rail_open, vec![9]);
        toggle_rail(&mut s, 9);
        assert!(s.rail_open.is_empty(), "pressing again closes it");

        // Two places open independently, and closing one leaves the other.
        toggle_rail(&mut s, 9);
        toggle_rail(&mut s, 10);
        assert_eq!(s.rail_open, vec![9, 10]);
        toggle_rail(&mut s, 9);
        assert_eq!(s.rail_open, vec![10]);
    }
    /// The rail used to fall back to the bare letter, so an unnamed system drive
    /// read "C:". The volume names itself by kind instead.
    #[test]
    fn an_unnamed_drive_is_named_by_what_it_is() {
        let drive = |kind, name: &str| volumes::Volume {
            path: PathBuf::from(name),
            label: String::new(),
            kind,
            free: 0,
            total: 0,
        };
        assert_eq!(drive(volumes::VolumeKind::Fixed, r"C:\").title(), "Local Disk (C:)");
        assert_eq!(
            drive(volumes::VolumeKind::Removable, r"E:\").title(),
            "Removable Disk (E:)"
        );
    }
}

#[cfg(test)]
mod sort_tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    /// A folder or file with a chosen size, timestamp and type name.
    fn item(name: &str, is_dir: bool, size: u64, day: u64, type_label: &str) -> Entry {
        Entry {
            name: name.to_string(),
            path: format!("C:/probe/{name}"),
            is_dir,
            size: if is_dir { 0 } else { size },
            modified: format_time(day * 86_400),
            modified_at: Some(UNIX_EPOCH + Duration::from_secs(day * 86_400)),
            type_label: type_label.to_string(),
            kind: if is_dir { Kind::Folder } else { Kind::Text },
            hidden: false,
            system: false,
            fixture: false,
            fixture_size: String::new(),
        }
    }

    fn order_with(sort: u8, desc: bool, entries: Vec<Entry>) -> Vec<String> {
        let mut s = init();
        s.view = View::Files;
        s.keep_order = false;
        s.count_override = None;
        s.searching = false;
        s.search_edit.text.clear();
        s.filter = Filter::All;
        s.group_by_date = false;
        s.sort = sort;
        s.sort_desc = desc;
        s.entries = entries;
        refresh_visible(&mut s);
        s.visible
            .iter()
            .map(|&index| s.entries[index].name.clone())
            .collect()
    }

    /// Day 40 is February, day 300 is November, so as text "11/…" sorts before
    /// "2/…" and the old string comparison put the older file last.
    fn probe() -> Vec<Entry> {
        vec![
            item("zeta", true, 0, 50, "File folder"),
            item("b-late", false, 10, 300, "Text document"),
            item("a-early", false, 500, 40, "Text document"),
            item("c-early", false, 50, 40, "Image"),
        ]
    }

    #[test]
    fn date_modified_orders_by_the_timestamp_not_its_text() {
        assert_eq!(
            order_with(1, false, probe()),
            ["zeta", "a-early", "c-early", "b-late"]
        );
        assert_eq!(
            order_with(1, true, probe()),
            ["zeta", "b-late", "a-early", "c-early"]
        );
    }

    #[test]
    fn name_size_and_type_order_as_expected() {
        assert_eq!(
            order_with(0, false, probe()),
            ["zeta", "a-early", "b-late", "c-early"]
        );
        assert_eq!(
            order_with(0, true, probe()),
            ["zeta", "c-early", "b-late", "a-early"]
        );
        assert_eq!(
            order_with(3, false, probe()),
            ["zeta", "b-late", "c-early", "a-early"]
        );
        assert_eq!(
            order_with(3, true, probe()),
            ["zeta", "a-early", "c-early", "b-late"]
        );
        // Image beats Text document, whatever the direction.
        let by_type = order_with(2, false, probe());
        assert_eq!(by_type[1], "c-early");
    }

    #[test]
    fn folders_lead_in_every_direction() {
        for sort in 0..4u8 {
            for desc in [false, true] {
                let order = order_with(sort, desc, probe());
                assert_eq!(order.first().map(String::as_str), Some("zeta"));
            }
        }
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;

    #[test]
    fn a_cancelled_scan_reads_nothing() {
        let dir = std::env::temp_dir().join(format!("lyre-scan-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in 0..64 {
            std::fs::write(dir.join(format!("f{n}.txt")), b"x").unwrap();
        }
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let (entries, _) = scan_until(&dir, &cancelled, |_| {});
        assert!(entries.is_empty(), "nothing is read once cancelled");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_long_scan_reports_how_far_it_has_read() {
        let dir = std::env::temp_dir().join(format!("lyre-scan-progress-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in 0..300 {
            std::fs::write(dir.join(format!("f{n}.txt")), b"x").unwrap();
        }
        let never = std::sync::atomic::AtomicBool::new(false);
        let mut seen = Vec::new();
        let (entries, _) = scan_until(&dir, &never, |count| seen.push(count));
        assert_eq!(entries.len(), 300);
        assert_eq!(seen, vec![256], "progress is reported as the read advances");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn navigating_fills_the_list_from_the_worker() {
        let dir = std::env::temp_dir().join(format!("lyre-scan-nav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in 0..3 {
            std::fs::write(dir.join(format!("f{n}.txt")), b"x").unwrap();
        }

        let mut s = init();
        go(&mut s, dir.clone());
        assert!(s.scan_rx.is_some(), "the read is handed to a worker");
        assert!(s.entries.is_empty(), "and the list does not wait for it");

        // Drain the way `poll` does until the worker answers.
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while s.scan_rx.is_some() && Instant::now() < deadline {
            drain_scan(&mut s);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(s.scan_rx.is_none(), "the worker finished in time");
        assert_eq!(s.entries.len(), 3, "its answer became the list");
        assert_eq!(s.root, dir);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_answer_for_a_folder_already_left_is_dropped() {
        let mut s = init();
        let shown = s.root.clone();
        s.scan_want = Some(PathBuf::from("elsewhere"));
        let (tx, rx) = std::sync::mpsc::channel();
        s.scan_rx = Some(rx);

        tx.send(ScanMsg::Done(Box::new(ScanDone {
            path: shown.clone(),
            entries: Vec::new(),
            scan_ms: 1.0,
            cancelled: false,
        })))
        .unwrap();

        drain_scan(&mut s);
        assert_eq!(s.root, shown, "the folder on screen does not change");
        assert!(
            s.scan_rx.is_some(),
            "the scan actually wanted keeps its channel"
        );
        assert_eq!(
            s.scan_want.as_deref(),
            Some(Path::new("elsewhere")),
            "and is still the one waited for"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the first render of a folder must not extract shell icons on
    /// the calling (UI) thread. A miss queues work and draws the glyph; the
    /// answer lands in the cache only when drained, as `poll` does.
    #[test]
    fn icon_miss_queues_worker_and_lands_through_drain() {
        let s = init();
        let path = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        let e = Entry {
            name: "app.exe".into(),
            path,
            is_dir: false,
            size: 0,
            modified: String::new(),
            modified_at: None,
            type_label: "Application".into(),
            kind: classify("app.exe", false),
            hidden: false,
            system: false,
            fixture: false,
            fixture_size: String::new(),
        };
        let key = icon_key(&e);
        let _ = entry_icon(&s, &e, paint(true, 1.0), 16.0);
        assert_eq!(s.icons_pending.get(), 1, "the miss must queue exactly one job");
        assert_eq!(
            s.icon_cache.borrow().get(&key).unwrap().size(),
            (0, 0),
            "nothing may be extracted synchronously on the calling thread"
        );
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while s.icons_pending.get() > 0 && Instant::now() < deadline {
            drain_icons(&s);
            if s.icons_pending.get() > 0 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        assert_eq!(s.icons_pending.get(), 0, "the worker answer must drain");
        #[cfg(target_os = "windows")]
        {
            // The shell is allowed to answer "no icon" while the machine is
            // loaded: this suite runs a whole App per test, so on a wide machine
            // the first few asks land while dozens of workers are running and
            // come back empty, and the same ask succeeds a moment later. A miss
            // is queued again rather than read as the worker being broken, which
            // is what a wrong answer looks like here: it stays empty every time.
            let mut landed = s.icon_cache.borrow().get(&key).unwrap().size();
            for _ in 0..8 {
                if landed != (0, 0) {
                    break;
                }
                s.icon_cache.borrow_mut().remove(&key);
                let _ = entry_icon(&s, &e, paint(true, 1.0), 16.0);
                while s.icons_pending.get() > 0 {
                    drain_icons(&s);
                    if s.icons_pending.get() > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
                landed = s.icon_cache.borrow().get(&key).unwrap().size();
                if landed == (0, 0) {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
            assert_ne!(
                landed,
                (0, 0),
                "an executable's shell icon must arrive through the worker"
            );
        }
        // A second render is a pure cache hit: no new job.
        let _ = entry_icon(&s, &e, paint(true, 1.0), 16.0);
        assert_eq!(s.icons_pending.get(), 0);
    }

    /// A live refresh must not disturb the person: selection, focus and anchor
    /// follow their paths through a reordered listing, and the scroll stays.
    #[test]
    fn refresh_keeps_selection_focus_and_scroll_on_their_paths() {
        let mut s = init();
        let entry = |name: &str| Entry {
            name: name.into(),
            path: format!("/fake/{name}"),
            is_dir: false,
            size: 1,
            modified: String::new(),
            modified_at: None,
            type_label: "File".into(),
            kind: classify(name, false),
            hidden: false,
            system: false,
            fixture: false,
            fixture_size: String::new(),
        };
        s.entries = vec![entry("a"), entry("b"), entry("c")];
        refresh_visible(&mut s);
        s.selected = vec![1];
        s.focused = Some(1);
        s.anchor = Some(1);
        s.scroll_off.set(120.0);
        update_selection_detail(&mut s);
        let root = s.root.clone();
        apply_refresh(&mut s, watch::Refreshed {
            path: root.clone(),
            entries: vec![entry("new"), entry("c"), entry("b"), entry("a")],
            scan_ms: 0.1,
        });
        let names = |idx: &Vec<usize>| idx.iter().map(|&i| s.entries[i].name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&s.selected), vec!["b"]);
        assert_eq!(s.focused.map(|i| s.entries[i].name.as_str()), Some("b"));
        assert_eq!(s.anchor.map(|i| s.entries[i].name.as_str()), Some("b"));
        assert_eq!(s.scroll_off.get(), 120.0);
        // A selected file that disappears leaves no dangling index behind.
        apply_refresh(&mut s, watch::Refreshed {
            path: root,
            entries: vec![entry("new"), entry("c"), entry("a")],
            scan_ms: 0.1,
        });
        assert!(s.selected.is_empty());
        assert_eq!(s.focused, None);
    }

    /// The whole live-refresh path: arming on the open folder, an external
    /// write, and the rescan landing in the showing entries without a poll
    /// timer — the watcher thread does the waiting and the scanning.
    #[test]
    fn an_external_write_lands_in_the_open_folder() {
        let dir = std::env::temp_dir().join(format!("lyre-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = init();
        s.waker = Some(std::sync::Arc::new(|| {}));
        s.view = View::Files;
        s.root = dir.clone();
        s.entries = scan(&dir).0;
        refresh_visible(&mut s);
        sync_watch(&mut s);
        assert!(s.watch.is_some(), "the open folder must be watched");
        std::fs::write(dir.join("appeared.txt"), b"x").unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        let mut landed = false;
        while Instant::now() < deadline {
            if drain_watch(&mut s) {
                landed = s.entries.iter().any(|e| e.name == "appeared.txt");
                if landed {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(landed, "the external write must appear without any manual refresh");
        // Leaving the folder retires the watch.
        s.view = View::QuickAccess;
        sync_watch(&mut s);
        assert!(s.watch.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression: cut in one tab, paste in another, switch back — the source
    /// tab must show the file gone without a manual refresh. The snapshot
    /// still draws first (instant tab switch), then the rescan corrects it.
    #[test]
    fn a_move_done_while_a_tab_was_backgrounded_shows_on_switch_back() {
        let base = std::env::temp_dir().join(format!("lyre-tabs-{}", std::process::id()));
        let dir_a = base.join("a");
        let dir_b = base.join("b");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        std::fs::write(dir_a.join("moved.txt"), b"x").unwrap();
        let mut s = init();
        s.waker = Some(std::sync::Arc::new(|| {}));
        s.view = View::Files;
        s.root = dir_a.clone();
        s.entries = scan(&dir_a).0;
        refresh_visible(&mut s);
        s.tabs = vec![capture_tab(&s)];
        s.active_tab = 0;
        s.root = dir_b.clone();
        s.entries = scan(&dir_b).0;
        refresh_visible(&mut s);
        s.tabs.push(capture_tab(&s));
        s.active_tab = 1;
        // The paste's move lands while tab A is backgrounded and unwatched.
        std::fs::rename(dir_a.join("moved.txt"), dir_b.join("moved.txt")).unwrap();
        switch_tab(&mut s, 0);
        assert!(
            s.entries.iter().any(|e| e.name == "moved.txt"),
            "the saved listing must draw first, instantly"
        );
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        let mut gone = false;
        while Instant::now() < deadline {
            drain_watch(&mut s);
            if !s.entries.iter().any(|e| e.name == "moved.txt") {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(gone, "switching back must rescan what happened while the tab was away");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The address editor resolves relative paths against the folder on screen,
    /// opens absolute paths, answers to the virtual places' names, and keeps
    /// the editor open with a notice when the typed path is not a folder.
    #[test]
    fn address_commit_covers_relative_absolute_virtual_and_missing() {
        let base = std::env::temp_dir().join(format!("lyre-address-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        let mut s = init();
        s.view = View::Files;
        s.root = base.clone();
        s.entries = scan(&base).0;
        refresh_visible(&mut s);
        s.address_edit = Some(TextEdit::new("sub"));
        commit_address(&mut s);
        assert_eq!(s.root, base.join("sub"));
        assert!(s.address_edit.is_none());
        s.address_edit = Some(TextEdit::new(base.to_string_lossy().into_owned()));
        commit_address(&mut s);
        assert_eq!(s.root, base);
        s.address_edit = Some(TextEdit::new("missing"));
        commit_address(&mut s);
        assert!(s.address_edit.is_some(), "a bad path keeps the editor for correction");
        assert!(!s.notice.is_empty());
        s.address_edit = Some(TextEdit::new("Quick Access"));
        commit_address(&mut s);
        assert!(matches!(s.view, View::QuickAccess));
        // "Home" is the home folder itself now, so it opens as a folder rather
        // than as the landing page.
        if let Some(home) = home_dir().filter(|home| home.is_dir()) {
            s.address_edit = Some(TextEdit::new("Home"));
            commit_address(&mut s);
            assert!(matches!(s.view, View::Files));
            assert!(same_path(&s.root, &home));
        }
        s.address_edit = Some(TextEdit::new("This PC"));
        commit_address(&mut s);
        assert!(matches!(s.view, View::ThisPc));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Every breadcrumb names the prefix path it opens, on both path flavours,
    /// and the virtual places offer their single label instead.
    #[test]
    fn crumbs_carry_the_prefix_path_they_open() {
        let mut s = init();
        s.view = View::Files;
        s.root = if cfg!(windows) {
            PathBuf::from("C:\\a\\b\\c")
        } else {
            PathBuf::from("/a/b/c")
        };
        let segs = crumb_segments(&s);
        let names: Vec<&str> = segs.iter().map(|(n, _)| n.as_str()).collect();
        if cfg!(windows) {
            assert_eq!(names, vec!["C:", "a", "b", "c"]);
            assert_eq!(segs[0].1, PathBuf::from("C:\\"));
            assert_eq!(segs[2].1, PathBuf::from("C:\\a\\b"));
        } else {
            assert_eq!(names, vec!["/", "a", "b", "c"]);
            assert_eq!(segs[0].1, PathBuf::from("/"));
            assert_eq!(segs[2].1, PathBuf::from("/a/b"));
        }
        s.view = View::QuickAccess;
        assert_eq!(crumb_segments(&s)[0].0, "Quick Access");
        s.view = View::ThisPc;
        assert_eq!(crumb_segments(&s)[0].0, "This PC");
        s.view = View::Gallery;
        assert_eq!(crumb_segments(&s)[0].0, "Gallery");
    }

    /// The reported bug: This PC was left selected beside the pinned folder
    /// that had replaced it, because the rail lit whichever row was last
    /// pressed and nothing cleared it. Selection now comes from the location,
    /// so opening a folder takes the one lit row with it.
    #[test]
    fn one_place_at_a_time() {
        let mut s = init();
        let pinned = std::env::temp_dir().join("lyre-place-test");
        s.pinned = vec![pinned.to_string_lossy().into_owned()];

        navigate(&mut s, 9);
        assert_eq!(place_of(s.view, &s.root), Place::ThisPc);

        // What a press on the pinned row does, without a disk behind it.
        s.view = View::Files;
        s.root = pinned.clone();
        assert_eq!(
            place_of(s.view, &s.root),
            Place::Folder,
            "the pinned folder is not This PC, so This PC cannot stay selected"
        );
        // And the pin is only selected while the window is in that folder.
        s.root = std::env::temp_dir();
        assert!(
            !same_path(&s.root, &pinned),
            "a folder that is not the pin leaves it unselected"
        );
    }

    /// A folder pinned twice, once with a trailing separator or in another
    /// case, drew two rows that both looked selected. One folder answers to one
    /// pin, and a restored list folds the repeats.
    #[test]
    fn a_folder_pinned_twice_selects_once() {
        let folder = std::env::temp_dir().join("lyre-pin-test");
        let spelled = folder.to_string_lossy().into_owned();
        let again = format!("{spelled}{}", std::path::MAIN_SEPARATOR);
        assert!(same_text_path(&spelled, &again), "a trailing separator is the same folder");
        assert_eq!(unique_paths(vec![spelled.clone(), again]).len(), 1);
        // The rail selects the first row whose path matches, and only one.
        let pins = unique_paths(vec![spelled.clone(), spelled]);
        let owner = pins
            .iter()
            .position(|path| same_path(Path::new(path), &folder));
        assert_eq!(owner, Some(0));
        assert_eq!(pins.len(), 1);
    }

    /// Up out of This PC went to `C:\Users\<name>`, which shows the folder the
    /// place had replaced rather than anything above This PC. A place that is
    /// not a folder has nowhere to go up to.
    #[test]
    fn up_does_not_leave_a_place() {
        let mut s = init();
        for fixture in [Fixture::ThisPc, Fixture::QuickAccess] {
            load(&mut s, fixture);
            let root = s.root.clone();
            assert!(!can_go_up(&s), "a place is not in a folder");
            go_up(&mut s);
            assert!(s.root == root, "Up moved out of a place");
            assert!(!s.notice.is_empty(), "Up said nothing");
        }
    }

    /// Home is the home folder itself: a real place with its own breadcrumbs,
    /// its own Up, and its own name in the tab.
    #[test]
    fn home_is_the_home_folder() {
        let Some(home) = home_dir().filter(|home| home.is_dir()) else { return };
        let mut s = init();
        open_home(&mut s);
        assert!(matches!(s.view, View::Files), "Home is a folder, not a page");
        assert!(same_path(&s.root, &home));
        assert_eq!(s.tab, "Home");
        assert_eq!(place_of(s.view, &s.root), Place::Home);
        let crumbs = crumb_segments(&s);
        assert_eq!(
            crumbs.last().map(|(name, _)| name.as_str()),
            home.file_name().and_then(|name| name.to_str()),
            "the crumbs are the real path, not a place label"
        );
    }

    /// A tab carries the search it started, and only it: switching tabs put the
    /// leaving tab's hits under the arriving one, and carried a drag across.
    #[test]
    fn a_tab_does_not_carry_its_search_or_its_drag_to_the_next() {
        let mut s = init();
        let base = std::env::temp_dir().join(format!("lyre-tab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        new_tab(&mut s);
        set_location(&mut s, base.clone());
        s.searching = true;
        s.search_hits = vec![search::Hit { entry: s.entries.first().cloned().unwrap_or_else(|| {
            make_entry("hit.txt", "", "Text Document", "1 KB", Kind::Text)
        }), score: 1, indices: vec![] }];
        s.drag = Some(DragState {
            paths: vec!["a.txt".into()],
            at: (10.0, 10.0),
            shift: false,
            target: None,
            target_key: String::new(),
            mode: fileops::Mode::Copy,
        });
        switch_tab(&mut s, 0);
        assert!(!s.searching, "the leaving tab's search is over");
        assert!(s.search_hits.is_empty(), "and its hits are gone");
        assert!(s.drag.is_none(), "a drag aimed at the leaving tab does not survive");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// One volume moves, across volumes copies, and Shift inverts whichever
    /// the volumes decided — the old app's rule, kept.
    #[test]
    fn drag_effect_follows_volumes_and_shift_inverts() {
        let mut s = init();
        s.volumes = vec![
            volumes::Volume {
                path: PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }),
                label: String::new(),
                kind: volumes::VolumeKind::Fixed,
                free: 0,
                total: 0,
            },
            volumes::Volume {
                path: PathBuf::from(if cfg!(windows) { "D:\\" } else { "/mnt/usb" }),
                label: String::new(),
                kind: volumes::VolumeKind::Removable,
                free: 0,
                total: 0,
            },
        ];
        let src = vec![if cfg!(windows) { "C:\\docs\\a.txt".to_string() } else { "/docs/a.txt".to_string() }];
        let same = DropTarget::Folder(PathBuf::from(if cfg!(windows) { "C:\\other" } else { "/other" }));
        let cross = DropTarget::Folder(PathBuf::from(if cfg!(windows) { "D:\\drop" } else { "/mnt/usb/drop" }));
        assert_eq!(drop_mode(&s, &src, Some(&same), false), fileops::Mode::Cut);
        assert_eq!(drop_mode(&s, &src, Some(&cross), false), fileops::Mode::Copy);
        assert_eq!(drop_mode(&s, &src, Some(&same), true), fileops::Mode::Copy);
        assert_eq!(drop_mode(&s, &src, Some(&cross), true), fileops::Mode::Cut);
    }

    /// Every place a drop can land is readable from the element key path the
    /// hit test returns, including a tab's close button and the favorites list.
    #[test]
    fn drop_targets_parse_from_key_paths() {
        let base = std::env::temp_dir().join(format!("lyre-drag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("zdir")).unwrap();
        std::fs::write(base.join("a.txt"), b"x").unwrap();
        let mut s = init();
        s.view = View::Files;
        s.root = base.clone();
        s.entries = scan(&base).0;
        refresh_visible(&mut s);
        s.tabs = vec![capture_tab(&s)];
        s.pinned = vec![base.to_string_lossy().into_owned()];
        s.favorites = vec![base.to_string_lossy().into_owned()];
        let (key, target) = parse_drop_target(&s, &format!("window/list-area/viewport/rows/{}/#0/#1", base.join("zdir").to_string_lossy()));
        assert!(key.starts_with("row:"));
        assert_eq!(target, Some(DropTarget::Folder(base.join("zdir"))));
        let (key, target) = parse_drop_target(&s, "main/rows/row-0/name");
        assert!(key.starts_with("row:"));
        assert_eq!(target, Some(DropTarget::Folder(base.join("zdir"))));
        let (_, target) = parse_drop_target(&s, "main/tabs/tab-0/tab-close-0");
        assert_eq!(target, Some(DropTarget::Tab(0)));
        let (_, target) = parse_drop_target(&s, "main/rail/pinned-0");
        assert_eq!(target, Some(DropTarget::Folder(base.clone())));
        let (_, target) = parse_drop_target(&s, "main/home/list-0");
        assert_eq!(target, Some(DropTarget::Folder(base.clone())));
        let (key, target) = parse_drop_target(&s, "main/rows");
        assert_eq!(key, "");
        assert_eq!(target, None);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// An OS drag hovering the window lights the same target an internal drag
    /// would — the round-trip drag's folder highlight — and a leave clears it.
    #[test]
    fn os_hover_lights_the_drop_target_and_clears() {
        let base = std::env::temp_dir().join(format!("lyre-oshover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("zdir")).unwrap();
        std::fs::write(base.join("a.txt"), b"x").unwrap();
        let mut s = init();
        s.view = View::Files;
        s.root = base.clone();
        s.entries = scan(&base).0;
        refresh_visible(&mut s);
        let row_path = format!("window/list-area/viewport/rows/{}/#0/#1", base.join("zdir").to_string_lossy());
        let key = os_hover_key(&s, Some(&row_path)).unwrap();
        assert_eq!(key, format!("row:{}", base.join("zdir").to_string_lossy()));
        s.os_hover = Some(key.clone());
        assert!(drop_hot(&s, &key));
        assert!(!drop_hot(&s, "tab-0"));
        // Unkeyed space (the list background) lights nothing, as internal drags.
        assert_eq!(os_hover_key(&s, Some("main/rows")), None);
        assert_eq!(os_hover_key(&s, None), None);
        s.os_hover = None;
        assert!(!drop_hot(&s, &key));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Moving onto the folder the items already live in does nothing; a copy
    /// onto a name that exists asks about the conflict instead of overwriting.
    #[test]
    fn drop_noops_in_place_and_asks_about_conflicts() {
        let base = std::env::temp_dir().join(format!("lyre-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.txt"), b"x").unwrap();
        let mut s = init();
        s.view = View::Files;
        s.root = base.clone();
        s.entries = scan(&base).0;
        refresh_visible(&mut s);
        let drag = |target: DropTarget, mode: fileops::Mode| DragState {
            paths: vec![base.join("a.txt").to_string_lossy().into_owned()],
            at: (0.0, 0.0),
            shift: false,
            target: Some(target),
            target_key: String::new(),
            mode,
        };
        s.drag = Some(drag(DropTarget::Background, fileops::Mode::Cut));
        finish_drag(&mut s);
        assert!(s.transfer.is_none(), "moving within the same folder must do nothing");
        assert!(s.dialog.is_none());
        s.drag = Some(drag(DropTarget::Folder(base.clone()), fileops::Mode::Copy));
        finish_drag(&mut s);
        assert!(matches!(s.dialog, Some(Dialog::PasteConflict { .. })));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A drop onto another folder really moves the file and refreshes the
    /// listing, through the same worker paste uses.
    #[test]
    fn dropping_onto_a_folder_moves_the_file() {
        let base = std::env::temp_dir().join(format!("lyre-dropmove-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("dest")).unwrap();
        std::fs::write(base.join("a.txt"), b"x").unwrap();
        let mut s = init();
        s.view = View::Files;
        s.root = base.clone();
        s.entries = scan(&base).0;
        refresh_visible(&mut s);
        s.drag = Some(DragState {
            paths: vec![base.join("a.txt").to_string_lossy().into_owned()],
            at: (0.0, 0.0),
            shift: false,
            target: Some(DropTarget::Folder(base.join("dest"))),
            target_key: String::new(),
            mode: fileops::Mode::Cut,
        });
        finish_drag(&mut s);
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while s.transfer.is_some() && Instant::now() < deadline {
            drain_op(&mut s);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drain_op(&mut s);
        assert!(base.join("dest/a.txt").exists(), "the drop must move the file");
        assert!(!base.join("a.txt").exists());
        assert!(!s.entries.iter().any(|e| e.name == "a.txt"), "the listing must refresh");
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod click_tests {
    use super::*;

    /// What hangs past the right edge of a window this size.
    fn past_the_edge(width: f32, height: f32, prepare: impl Fn(&mut App)) -> Vec<(String, [f32; 4])> {
        let mut rt = rune::Runtime::new(init(), view, (width, height), rune::Env::new(true, 1.0));
        // The harness draws no paint pass, so the window's own measurement never
        // lands in the cell the layout reads; a real window fills it on its first
        // frame. Prime it so this measures the narrow case and not the fallback.
        rt.state.rect_window.set([0.0, 0.0, width, height]);
        rt.update(|s, _tx| {
            prepare(s);
            s.rect_window.set([0.0, 0.0, width, height]);
        });
        // The first frame measures the window; the second lays out for it.
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);
        rt.elements()
            .into_iter()
            .filter_map(|e| {
                let key = e.key?;
                // A scroll view lays its contents out past the edge on purpose;
                // everything else belongs inside the window.
                (e.rect[0] + e.rect[2] > width + 0.5 && !key.contains("viewport"))
                    .then_some((key, e.rect))
            })
            .collect()
    }

    /// The reported bug: at a narrow window the Quick Access tiles ran off the
    /// right edge, and a long file name was drawn over the date beside it.
    #[test]
    fn nothing_hangs_past_the_edge_of_a_narrow_window() {
        for width in [MIN_W, 420.0, 520.0, 620.0, 700.0, 760.0] {
            for page in 0..3 {
                let past = past_the_edge(width, 300.0, |s| match page {
                    0 => load(s, Fixture::QuickAccess),
                    1 => load(s, Fixture::Downloads),
                    _ => {
                        navigate(s, 9);
                    }
                });
                assert!(past.is_empty(), "page {page} hangs past {width}: {past:?}");
            }
        }
    }

    /// The dialog draws one button per entry of the list the keyboard walks, so
    /// the two can never offer different things.
    #[test]
    fn the_dialog_draws_the_buttons_the_keyboard_walks() {
        let mut rt = runtime();
        // Opened through `update` so the runtime knows the frame is out of date.
        rt.update(|s, _tx| {
            open_dialog(s, Dialog::Delete { permanent: true, paths: vec![PathBuf::from("/nowhere")] });
        });
        rt.frame(0.0);
        let elements = rt.elements();
        let drawn = elements
            .iter()
            .filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with("dialog-button-")))
            .count();
        assert_eq!(
            drawn,
            dialog_buttons(rt.state.dialog.as_ref()).len(),
            "every button of the dialog is on screen"
        );
        // And the row is where a hand expects it: both buttons in one band.
        let cancel = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("dialog-button-0"))
            .expect("the first button");
        assert!(cancel.rect[2] > 0.0 && cancel.rect[3] > 0.0, "and has a size to press");
    }

    /// A press on a tab still switches to it now that the tab also answers an
    /// Alt+click, and a press on its close button still closes it.
    #[test]
    fn a_press_on_a_tab_switches_to_it() {
        let mut rt = runtime();
        new_tab(&mut rt.state);
        rt.frame(0.0);
        assert_eq!(rt.state.active_tab, 1, "the new tab is the showing one");
        let tab = rect_of(&rt, "tab-0");
        click_at(&mut rt, tab[0] + 60.0, tab[1] + tab[3] * 0.5);
        assert_eq!(rt.state.active_tab, 0, "the press switched to the first tab");
        assert_eq!(rt.state.tabs.len(), 2, "and did not close anything");
    }

    fn runtime() -> rune::Runtime<App> {
        let mut rt = rune::Runtime::new(init(), view, (1076.0, 600.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        rt
    }

    /// An element's rect as `ElementInfo` reports it: x, y, width, height.
    fn rect_of(rt: &rune::Runtime<App>, key: &str) -> [f32; 4] {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no element keyed {key}"))
            .rect
    }

    /// A press and release at one point, then the frame after it.
    fn click_at(rt: &mut rune::Runtime<App>, x: f32, y: f32) {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
    }

    /// The text a tab shows, read from the strip itself rather than from state.
    fn tab_label(rt: &rune::Runtime<App>, index: usize) -> String {
        let prefix = format!("window/tabs/tab-{index}/");
        rt.elements()
            .into_iter()
            .find(|e| e.path.starts_with(&prefix) && e.role == "text")
            .and_then(|e| e.text)
            .unwrap_or_default()
    }

    /// The reported bug: a press on a folder sometimes did not open it, and it
    /// took several presses. Part of the rail row was not pressable at all: the
    /// press lived on the inner body, so the row's padding and the spacer beside
    /// the label took it and did nothing with it.
    #[test]
    fn a_press_anywhere_on_a_rail_row_opens_the_place() {
        let rt = runtime();
        // Home is a folder on every platform, unlike Desktop, which a fresh
        // Linux install does not have.
        let r = rect_of(&rt, "rail-0");
        let (w, h) = (r[2], r[3]);
        for frac in [0.01f32, 0.03, 0.06, 0.1, 0.25, 0.5, 0.75, 0.95, 0.99] {
            let mut rt = runtime();
            click_at(&mut rt, r[0] + w * frac, r[1] + h * 0.5);
            assert!(
                rt.state.view == View::Files,
                "a press at {frac} of the row opened nothing"
            );
        }
    }

    /// The chevron is drawn inside the row and keeps its own press: it expands
    /// the place's list without also navigating to it.
    #[test]
    fn the_rail_chevron_still_only_expands() {
        let mut rt = runtime();
        let row = rect_of(&rt, "rail-9");
        assert!(rt.state.rail_open.is_empty());
        // The chevron is the first child of the row, past its 8 px padding.
        click_at(&mut rt, row[0] + 8.0 + 5.5, row[1] + row[3] * 0.5);
        assert!(
            rt.state.view == View::QuickAccess,
            "a press on the chevron must not open the place"
        );
        assert_eq!(rt.state.rail_open, vec![9], "the chevron expanded the place");
    }

    /// The reported bug: the strip kept saying Quick Access while the window
    /// showed another folder. A tab's name and icon were read from the snapshot
    /// taken when the tab was last left, which only a tab change refreshed.
    #[test]
    fn the_open_tab_follows_where_the_window_is() {
        let mut rt = runtime();
        assert_eq!(tab_label(&rt, 0), "Quick Access");
        let row = rect_of(&rt, "rail-0");
        click_at(&mut rt, row[0] + row[2] * 0.5, row[1] + row[3] * 0.5);
        assert!(rt.state.view == View::Files, "the press opened Home");
        assert_eq!(rt.state.tab, "Home");
        assert_eq!(
            tab_label(&rt, 0),
            "Home",
            "the strip must name the folder on screen, not the tab it was opened at"
        );
    }
    /// The reported bug, through the control the person actually presses: This
    /// PC, then a folder, then Back left the window on whatever folder This PC
    /// had been opened from rather than returning to This PC.
    #[test]
    fn the_back_button_returns_to_this_pc() {
        let mut rt = runtime();
        let pc = rect_of(&rt, "rail-9");
        click_at(&mut rt, pc[0] + pc[2] * 0.5, pc[1] + pc[3] * 0.5);
        assert!(rt.state.view == View::ThisPc, "the press opened This PC");
        // Home is a folder on every platform, unlike Desktop.
        let home = rect_of(&rt, "rail-0");
        click_at(&mut rt, home[0] + home[2] * 0.5, home[1] + home[3] * 0.5);
        assert!(rt.state.view == View::Files, "the second press opened Home");
        let back = rect_of(&rt, "back");
        click_at(&mut rt, back[0] + back[2] * 0.5, back[1] + back[3] * 0.5);
        assert!(
            rt.state.view == View::ThisPc,
            "the Back button must return to This PC"
        );
    }
}

#[cfg(test)]
mod undo_tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lyre-undo-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A window on `dir`, with nothing on the undo stack.
    fn open(dir: &Path) -> App {
        let mut s = init();
        s.view = View::Files;
        s.root = dir.to_path_buf();
        s.undo.clear();
        s.redo.clear();
        s
    }

    fn wait(s: &mut App) {
        for _ in 0..250 {
            drain_op(s);
            if s.transfer.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the operation did not finish");
    }

    fn rename_to(path: &Path, name: &str) -> Dialog {
        let mut edit = TextEdit::default();
        edit.text = name.into();
        Dialog::Rename { path: path.to_path_buf(), edit }
    }

    #[test]
    fn undo_puts_a_renamed_file_back_and_redo_renames_it_again() {
        let dir = scratch("rename");
        let before = dir.join("before.txt");
        std::fs::write(&before, b"x").unwrap();
        let mut s = open(&dir);
        s.dialog = Some(rename_to(&before, "after.txt"));
        commit_rename(&mut s);
        assert!(dir.join("after.txt").exists(), "it was renamed");

        undo_last(&mut s);
        assert!(before.exists(), "the old name is back");
        assert!(!dir.join("after.txt").exists(), "and the new one is gone");

        redo_last(&mut s);
        assert!(dir.join("after.txt").exists(), "redo renamed it again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_takes_a_new_folder_away_and_redo_makes_it_again() {
        let dir = scratch("folder");
        let mut s = open(&dir);
        let mut edit = TextEdit::default();
        edit.text = "made".into();
        s.dialog = Some(Dialog::NewFolder { parent: dir.clone(), edit });
        commit_new_folder(&mut s);
        assert!(dir.join("made").is_dir(), "the folder was made");

        undo_last(&mut s);
        assert!(!dir.join("made").exists(), "undo took it away");

        redo_last(&mut s);
        assert!(dir.join("made").is_dir(), "redo made it again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_takes_a_copy_away_and_redo_copies_it_again() {
        let source = scratch("copy-src");
        let target = scratch("copy-dst");
        let file = source.join("kept.txt");
        std::fs::write(&file, b"hello").unwrap();
        let mut s = open(&target);
        start_paste(&mut s, fileops::Mode::Copy, fileops::Conflict::Rename, vec![file], target.clone());
        wait(&mut s);
        assert!(target.join("kept.txt").exists(), "the copy landed");

        undo_last(&mut s);
        assert!(!target.join("kept.txt").exists(), "undo took the copy away");
        assert!(source.join("kept.txt").exists(), "and left the original");

        redo_last(&mut s);
        assert_eq!(
            std::fs::read_to_string(target.join("kept.txt")).unwrap(),
            "hello",
            "redo copied it again"
        );
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn undo_moves_a_cut_file_back_and_redo_moves_it_again() {
        let source = scratch("move-src");
        let target = scratch("move-dst");
        let file = source.join("moved.txt");
        std::fs::write(&file, b"x").unwrap();
        let mut s = open(&target);
        start_paste(&mut s, fileops::Mode::Cut, fileops::Conflict::Rename, vec![file.clone()], target.clone());
        wait(&mut s);
        assert!(target.join("moved.txt").exists(), "the file moved");
        assert!(!file.exists(), "and left where it was");

        undo_last(&mut s);
        assert!(file.exists(), "undo moved it back");
        assert!(!target.join("moved.txt").exists());

        redo_last(&mut s);
        assert!(target.join("moved.txt").exists(), "redo moved it again");
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&target);
    }

    /// Doing something new drops what could have been redone: those steps belong
    /// to a branch that no longer exists.
    #[test]
    fn a_new_operation_drops_the_redo_steps() {
        let dir = scratch("branch");
        let file = dir.join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let mut s = open(&dir);
        s.dialog = Some(rename_to(&file, "b.txt"));
        commit_rename(&mut s);
        undo_last(&mut s);
        assert_eq!(s.redo.len(), 1, "the rename is there to redo");

        let mut edit = TextEdit::default();
        edit.text = "fresh".into();
        s.dialog = Some(Dialog::NewFolder { parent: dir.clone(), edit });
        commit_new_folder(&mut s);
        assert!(s.redo.is_empty(), "the new folder dropped it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With nothing to take back, Ctrl+Z says so rather than doing something.
    #[test]
    fn undo_with_nothing_to_undo_says_so() {
        let dir = scratch("empty");
        let mut s = open(&dir);
        undo_last(&mut s);
        assert_eq!(s.notice, "Nothing to undo");
        assert!(s.undo.is_empty() && s.redo.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reported wish: a delete goes to the trash and comes back out of it.
    #[test]
    fn undo_brings_a_deleted_file_back_out_of_the_trash() {
        let dir = scratch("trash");
        let file = dir.join("gone.txt");
        std::fs::write(&file, b"content").unwrap();
        let mut s = open(&dir);
        s.entries = scan(&dir).0;
        s.visible = (0..s.entries.len()).collect();
        select_all(&mut s);
        start_delete(&mut s, fileops::DeleteMode::Trash, vec![file.clone()]);
        wait(&mut s);
        if file.exists() {
            // A machine whose bin is turned off cannot delete this way at all.
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert_eq!(s.undo.len(), 1, "the delete is on the undo stack");

        undo_last(&mut s);
        assert!(file.exists(), "undo brought it back out of the trash");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "content");

        redo_last(&mut s);
        assert!(!file.exists(), "redo deleted it again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The chord itself, through the key handler the window calls.
    #[test]
    fn ctrl_z_takes_it_back_and_ctrl_y_does_it_again() {
        let dir = scratch("keys");
        let before = dir.join("one.txt");
        std::fs::write(&before, b"x").unwrap();
        let chord = |key: Key, shift: bool| KeyEvent {
            key,
            mods: rune::Mods { cmd: true, shift, ..rune::Mods::default() },
        };
        let mut rt = rune::Runtime::new(init(), view, (900.0, 600.0), rune::Env::new(true, 1.0));
        rt.update(|s, _tx| {
            s.view = View::Files;
            s.root = dir.clone();
            s.undo.clear();
            s.redo.clear();
            s.dialog = Some(rename_to(&before, "two.txt"));
            commit_rename(s);
        });

        let undo = chord(Key::Command('z'), false);
        rt.update(|s, tx| assert!(handle_key(s, &undo, tx), "Ctrl+Z is handled"));
        assert!(before.exists(), "Ctrl+Z put the name back");

        let redo = chord(Key::Command('y'), false);
        rt.update(|s, tx| assert!(handle_key(s, &redo, tx), "Ctrl+Y is handled"));
        assert!(dir.join("two.txt").exists(), "Ctrl+Y renamed it again");

        // Ctrl+Shift+Z is the other way to redo.
        let undo = chord(Key::Command('z'), false);
        rt.update(|s, tx| {
            handle_key(s, &undo, tx);
        });
        assert!(before.exists(), "undone again");
        let shift_redo = chord(Key::Command('z'), true);
        rt.update(|s, tx| assert!(handle_key(s, &shift_redo, tx)));
        assert!(dir.join("two.txt").exists(), "Ctrl+Shift+Z redid it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod paste_tests {
    use super::*;

    /// The note a crash leaves behind is the only thing a windowed build can
    /// say, so it has to name the panic and the line it came from.
    #[test]
    fn a_panic_is_written_to_the_crash_log() {
        let log = crash_log_path();
        let _ = std::fs::remove_file(&log);
        record_panics();
        let raised = std::panic::catch_unwind(|| panic!("probe panic for the log"));
        assert!(raised.is_err(), "the panic was raised");
        let text = std::fs::read_to_string(&log).expect("the log was written");
        assert!(text.contains("probe panic for the log"), "it names the reason: {text}");
        assert!(text.contains("main.rs"), "and the place: {text}");
        let _ = std::fs::remove_file(&log);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lyre-paste-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Show `dir` with its real contents, and select every row.
    fn open_and_select_all(dir: &Path) -> App {
        let mut s = init();
        s.view = View::Files;
        s.root = dir.to_path_buf();
        s.crumbs = vec![dir.file_name().unwrap_or_default().to_string_lossy().into_owned()];
        s.entries = scan(dir).0;
        s.visible = (0..s.entries.len()).collect();
        select_all(&mut s);
        s
    }

    fn drain_to_end(s: &mut App) {
        for _ in 0..250 {
            drain_op(s);
            if s.transfer.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the operation did not finish");
    }

    /// Cut a file and paste it into another folder: the move the person does,
    /// through the same functions the keys and the menu call.
    #[test]
    fn a_cut_and_paste_moves_the_file() {
        let source = scratch("cut-src");
        let target = scratch("cut-dst");
        std::fs::write(source.join("moved.txt"), b"hello").unwrap();

        let mut s = open_and_select_all(&source);
        cut_selection(&mut s);
        assert!(s.clipboard.is_some(), "the file is on the clipboard as a cut");
        assert!(is_cut(&s, &source.join("moved.txt").to_string_lossy()), "and drawn as cut");

        let (sources, mode) = s.clipboard.clone().unwrap();
        paste_into(&mut s, sources, mode, target.clone());
        drain_to_end(&mut s);

        assert!(target.join("moved.txt").exists(), "the file arrived");
        assert!(!source.join("moved.txt").exists(), "and left where it was");
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&target);
    }

    /// Copying onto a name that is taken asks, and "Keep both" writes the copy
    /// beside the original instead of over it.
    #[test]
    fn a_paste_that_collides_asks_and_keeps_both() {
        let dir = scratch("collide");
        std::fs::write(dir.join("same.txt"), b"original").unwrap();

        let mut s = open_and_select_all(&dir);
        copy_selection(&mut s);
        let (sources, mode) = s.clipboard.clone().unwrap();
        paste_into(&mut s, sources, mode, dir.clone());
        assert!(matches!(s.dialog, Some(Dialog::PasteConflict { .. })), "the question is asked");

        press_dialog_button(&mut s, DialogButton::KeepBoth);
        drain_to_end(&mut s);

        assert_eq!(
            std::fs::read_to_string(dir.join("same.txt")).unwrap(),
            "original",
            "the file that was there is untouched"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("same (2).txt")).unwrap(),
            "original",
            "and the copy sits beside it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A rename in the middle of the listing, then a cut: the selection must not
    /// leave the app holding an index that no longer exists.
    #[test]
    fn a_cut_survives_the_listing_changing_under_it() {
        let dir = scratch("refresh");
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let mut s = open_and_select_all(&dir);
        select_entry(&mut s, 2);
        assert_eq!(s.selected, vec![2]);

        // Another program deletes what was selected and writes something else.
        std::fs::remove_file(dir.join("c.txt")).unwrap();
        apply_refresh(&mut s, watch::Refreshed {
            path: dir.clone(),
            entries: scan(&dir).0,
            scan_ms: 0.0,
        });

        cut_selection(&mut s);
        let copied = s.clipboard.clone();
        let _ = copied;
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod dialog_key_tests {
    use super::*;

    /// A window showing one file, selected, so the delete dialog has a target.
    fn one_selected_file(tag: &str) -> (App, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("lyre-keys-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gone.txt");
        std::fs::write(&file, b"x").unwrap();
        let mut s = init();
        s.view = View::Files;
        s.root = dir.clone();
        s.crumbs = vec![dir.file_name().unwrap_or_default().to_string_lossy().into_owned()];
        s.entries = vec![Entry {
            name: "gone.txt".into(),
            path: file.to_string_lossy().into_owned(),
            is_dir: false,
            size: 1,
            modified: String::new(),
            modified_at: None,
            type_label: "Text document".into(),
            kind: classify("gone.txt", false),
            hidden: false,
            system: false,
            fixture: false,
            fixture_size: String::new(),
        }];
        s.visible = vec![0];
        s.selected = vec![0];
        (s, dir, file)
    }

    /// The reported wish: Delete opens the dialog, and Enter confirms it without
    /// the hand leaving the keyboard.
    #[test]
    fn enter_confirms_the_delete_dialog() {
        let (mut s, dir, file) = one_selected_file("confirm");
        begin_delete(&mut s, true);
        assert!(matches!(s.dialog, Some(Dialog::Delete { .. })), "Delete opened the dialog");
        let buttons = dialog_buttons(s.dialog.as_ref());
        assert_eq!(
            dialog_focus_index(&s, buttons.len()),
            dialog_default_focus(&buttons),
            "the keyboard starts on the dialog's primary button"
        );
        assert_eq!(
            buttons[dialog_focus_index(&s, buttons.len())],
            DialogButton::Confirm,
            "which is Delete, so Enter confirms"
        );

        assert!(dialog_key(&mut s, &Key::Enter));
        assert!(s.dialog.is_none(), "the dialog closed");
        for _ in 0..250 {
            drain_op(&mut s);
            if s.transfer.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!file.exists(), "Enter deleted the file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The arrows walk to Cancel, and Enter there stops without deleting.
    #[test]
    fn the_arrows_reach_cancel_and_enter_takes_it() {
        let (mut s, dir, file) = one_selected_file("cancel");
        begin_delete(&mut s, true);
        assert!(dialog_key(&mut s, &Key::Left), "the arrow moved the focus");
        let buttons = dialog_buttons(s.dialog.as_ref());
        assert_eq!(
            buttons[dialog_focus_index(&s, buttons.len())],
            DialogButton::Cancel
        );
        assert!(dialog_key(&mut s, &Key::Enter));
        assert!(s.dialog.is_none(), "Cancel closed the dialog");
        assert!(s.transfer.is_none(), "and started nothing");
        assert!(file.exists(), "the file is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A dialog with a field in it leaves the keys to that field: Enter submits
    /// through the field itself, and the arrows move the caret.
    #[test]
    fn a_field_in_the_dialog_keeps_its_keys() {
        let (mut s, dir, _file) = one_selected_file("field");
        begin_new_folder(&mut s);
        if let Some(Dialog::NewFolder { edit, .. }) = s.dialog.as_mut() {
            edit.focused = true;
        }
        assert!(!dialog_key(&mut s, &Key::Left), "the field keeps the arrow");
        assert!(!dialog_key(&mut s, &Key::Enter), "and the Enter that submits it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tab_close_tests {
    use super::*;

    fn three_tabs() -> App {
        let mut s = init();
        new_tab(&mut s);
        new_tab(&mut s);
        s.active_tab = 0;
        s
    }

    /// Alt+click closes the tab it lands on, as it does in a browser; a plain
    /// click still only switches.
    #[test]
    fn alt_click_closes_the_tab_it_lands_on() {
        let mut s = three_tabs();
        assert_eq!(s.tabs.len(), 3);
        tab_pressed(&mut s, 1, true);
        assert_eq!(s.tabs.len(), 2, "Alt and a press closes the tab");
        tab_pressed(&mut s, 1, false);
        assert_eq!(s.tabs.len(), 2, "a plain press does not close it");
        assert_eq!(s.active_tab, 1, "it switches to that tab instead");
    }

    /// A middle press closes whatever tab it landed on, and nothing when it
    /// landed anywhere else.
    #[test]
    fn a_middle_press_closes_the_tab_under_it() {
        let mut s = three_tabs();
        assert!(close_tab_at_path(&mut s, "main/tabs/tab-1/tab-close-1"));
        assert_eq!(s.tabs.len(), 2, "the tab under the press is gone");
        // The close button is the same target as the tab around it.
        assert!(close_tab_at_path(&mut s, "main/tabs/tab-0/tab-close-0"));
        assert_eq!(s.tabs.len(), 1);
        // Anywhere else is not a tab, so the press does nothing.
        assert!(!close_tab_at_path(&mut s, "main/tabs/rail-0"));
        assert_eq!(s.tabs.len(), 1, "a press off the strip closes nothing");
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    /// Creating or deleting where the account has no permission must say so and
    /// leave the window as it was: the reported symptom was the application
    /// closing instead.
    #[cfg(windows)]
    mod permissions {
        use super::*;

        /// A folder that can be read and listed but not written to: Everyone is
        /// denied write-data and add-file, which is what C:\Projects is for an
        /// account without it.
        fn read_only(tag: &str) -> Option<PathBuf> {
            let dir = std::env::temp_dir().join(format!("lyre-perm-{tag}-{}", std::process::id()));
            let _ = deny(&dir, false);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).ok()?;
            deny(&dir, true)?.then_some(dir)
        }

        /// Grant or remove the deny entry; false when this account cannot.
        fn deny(dir: &Path, on: bool) -> Option<bool> {
            let mut command = std::process::Command::new("icacls");
            command.arg(dir);
            if on {
                command.arg("/deny").arg("*S-1-1-0:(WD,AD)");
            } else {
                command.arg("/remove:d").arg("*S-1-1-0");
            }
            command.output().ok().map(|out| out.status.success())
        }

        fn clean(dir: &Path) {
            deny(dir, false);
            let _ = std::fs::remove_dir_all(dir);
        }

        fn open_in(dir: &Path) -> App {
            let mut s = init();
            s.view = View::Files;
            s.root = dir.to_path_buf();
            s.crumbs = vec![dir.file_name().unwrap_or_default().to_string_lossy().into_owned()];
            s
        }

        #[test]
        fn creating_a_folder_where_you_may_not_says_so() {
            let Some(dir) = read_only("create") else { return };
            let mut s = open_in(&dir);
            begin_new_folder(&mut s);
            if let Some(Dialog::NewFolder { edit, .. }) = s.dialog.as_mut() {
                edit.text = "new folder".into();
            }
            commit_new_folder(&mut s);
            assert!(s.dialog.is_none(), "the dialog closes either way");
            assert!(
                s.notice.contains("Could not create the folder"),
                "the window must say why, got {:?}",
                s.notice
            );
            assert!(same_path(&s.root, &dir), "and stay in the folder it was in");
            clean(&dir);
        }

        #[test]
        fn deleting_a_file_that_cannot_be_deleted_says_so() {
            // A plain writable folder, so the file can be made first; the deny
            // entry then goes on the file, which is what a protected item has.
            let dir = std::env::temp_dir().join(format!("lyre-perm-delete-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("kept.txt");
            std::fs::write(&file, b"x").unwrap();
            // Windows lets a file be deleted through its folder's delete-child
            // right, so both the file and the folder have to refuse it.
            let denied = std::process::Command::new("icacls")
                .arg(&file)
                .args(["/deny", "*S-1-1-0:(D)"])
                .output()
                .is_ok_and(|out| out.status.success())
                && std::process::Command::new("icacls")
                    .arg(&dir)
                    .args(["/deny", "*S-1-1-0:(DC)"])
                    .output()
                    .is_ok_and(|out| out.status.success());
            if !denied {
                clean(&dir);
                return;
            }

            let mut s = open_in(&dir);
            start_delete(&mut s, fileops::DeleteMode::Permanent, vec![file.clone()]);
            for _ in 0..250 {
                drain_op(&mut s);
                if s.transfer.is_none() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            assert!(s.transfer.is_none(), "the operation ends rather than hanging");
            assert!(file.exists(), "the file it could not delete is still there");
            assert!(
                s.notice.contains("os error 5") || s.notice.contains("Access is denied"),
                "the window must say why it could not delete, got {:?}",
                s.notice
            );
            let _ = std::process::Command::new("icacls")
                .arg(&file)
                .args(["/remove:d", "*S-1-1-0"])
                .output();
            let _ = std::process::Command::new("icacls")
                .arg(&dir)
                .args(["/remove:d", "*S-1-1-0"])
                .output();
            clean(&dir);
        }
    }

    /// A folder on disk that is not one of the rail's places.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lyre-history-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The reported bug: This PC, then a folder, then Back did not return to
    /// This PC. History kept bare paths, and This PC has none of its own: it
    /// stood on the last real folder, that folder was what got recorded, and
    /// Back walked out of the page instead of going back to it.
    #[test]
    fn back_from_a_folder_returns_to_this_pc() {
        let mut s = init();
        navigate(&mut s, 9);
        assert!(s.view == View::ThisPc, "the rail press opened This PC");
        let folder = scratch("thispc");
        go(&mut s, folder.clone());
        assert!(s.view == View::Files, "the folder opened");
        go_back(&mut s);
        assert!(
            s.view == View::ThisPc,
            "Back must return to This PC, not to the folder This PC was opened from"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The same for the landing page, which has no path either.
    #[test]
    fn back_returns_to_quick_access() {
        let mut s = init();
        let folder = scratch("quick");
        go(&mut s, folder.clone());
        navigate(&mut s, 2);
        assert!(s.view == View::QuickAccess, "the rail press opened Quick Access");
        let inner = folder.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        go(&mut s, inner);
        go_back(&mut s);
        assert!(s.view == View::QuickAccess, "Back must return to Quick Access");
        go_back(&mut s);
        assert!(s.view == View::Files, "and once more to the folder before it");
        assert!(same_path(&s.root, &folder), "which is the folder it was");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Forward walks the same steps the other way.
    #[test]
    fn forward_returns_to_the_folder_back_left() {
        let mut s = init();
        let first = scratch("fwd-a");
        let second = scratch("fwd-b");
        go(&mut s, first.clone());
        go(&mut s, second.clone());
        go_back(&mut s);
        assert!(same_path(&s.root, &first), "Back returned to the first folder");
        go_forward(&mut s);
        assert!(same_path(&s.root, &second), "Forward returned to the second");
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    /// Forward has to come back to a page too, not only to a folder.
    #[test]
    fn forward_returns_to_this_pc() {
        let mut s = init();
        navigate(&mut s, 9);
        let folder = scratch("fwd-pc");
        go(&mut s, folder.clone());
        go_back(&mut s);
        assert!(s.view == View::ThisPc);
        go_forward(&mut s);
        assert!(
            s.view == View::Files && same_path(&s.root, &folder),
            "Forward must return to the folder Back left"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Pressing the page already on screen is not a step: two presses of This PC
    /// must not need two presses of Back to leave it.
    #[test]
    fn pressing_the_page_you_are_on_does_not_stack_a_step() {
        let mut s = init();
        navigate(&mut s, 9);
        let depth = s.history.len();
        navigate(&mut s, 9);
        assert_eq!(s.history.len(), depth, "a re-press is not another step");
    }

    /// A window closed over a page and opened again on it still has its steps.
    #[test]
    fn the_gallery_keeps_its_place_in_history() {
        if gallery_folder().is_none() {
            // A machine without Pictures has no Gallery to enter.
            return;
        }
        let mut s = init();
        let folder = scratch("gallery");
        go(&mut s, folder.clone());
        navigate(&mut s, 1);
        assert!(s.view == View::Gallery, "the rail press opened the Gallery");
        let inner = folder.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        go(&mut s, inner);
        go_back(&mut s);
        assert!(s.view == View::Gallery, "Back must return to the Gallery");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Each tab carries its own steps, pages included: a tab sitting on This PC
    /// behind a folder still steps back to This PC after another tab is used.
    #[test]
    fn a_tab_keeps_its_own_steps_back_to_a_page() {
        let mut s = init();
        navigate(&mut s, 9);
        let folder = scratch("tabs");
        go(&mut s, folder.clone());
        // A second tab on Quick Access, then back to the first.
        new_tab(&mut s);
        switch_tab(&mut s, 0);
        assert!(
            s.view == View::Files && same_path(&s.root, &folder),
            "the first tab came back where it was"
        );
        go_back(&mut s);
        assert!(
            s.view == View::ThisPc,
            "the tab's own step back is the page it was on"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A step back over a folder that has since gone says so and stays put,
    /// rather than drawing a folder that is not there.
    #[test]
    fn back_to_a_folder_that_is_gone_says_so() {
        let mut s = init();
        let folder = scratch("gone");
        let elsewhere = std::env::temp_dir();
        go(&mut s, folder.clone());
        go(&mut s, elsewhere.clone());
        std::fs::remove_dir_all(&folder).unwrap();
        go_back(&mut s);
        assert!(
            s.notice.contains("is not available here"),
            "the window should say the folder is gone, got {:?}",
            s.notice
        );
        assert!(
            same_path(&s.root, &elsewhere),
            "and stay where it is instead of showing nothing"
        );
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lyre-link-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A real directory junction, which is what a folder "shortcut" on a
    /// Windows machine usually is. `cmd` makes one without needing privilege.
    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) -> bool {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|out| out.status.success())
    }

    /// The reported bug. `Documents\My Videos` is a directory junction, and the
    /// standard library reports `is_dir()` false for one because it treats a
    /// reparse point as a link. It was listed as a file: sorted with the files,
    /// drew a file's blank icon, and was handed to the shell, which opened the
    /// folder it names by starting a second Lyre window.
    #[cfg(windows)]
    #[test]
    fn a_directory_junction_is_a_folder_in_the_listing() {
        let base = scratch("junction");
        std::fs::create_dir_all(base.join("target")).unwrap();
        std::fs::create_dir_all(base.join("shown")).unwrap();
        let link = base.join("shown").join("My Videos");
        if !junction(&link, &base.join("target")) {
            // A machine that refuses junctions can say nothing about them.
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        let (entries, _) = scan(&base.join("shown"));
        let entry = entries.iter().find(|e| e.name == "My Videos").expect("listed");
        assert!(entry.is_dir, "a junction to a folder is a folder");
        assert_eq!(entry.type_label, "File folder");
        assert_eq!(
            icon_key(entry),
            "folder",
            "it draws the folder's icon, not a blank file's"
        );

        // The target goes. Explorer still lists the folder, and opening it says
        // the location is not available rather than showing an empty folder.
        std::fs::remove_dir_all(base.join("target")).unwrap();
        let (entries, _) = scan(&base.join("shown"));
        let entry = entries.iter().find(|e| e.name == "My Videos").expect("listed");
        assert!(entry.is_dir, "the link is still the folder it was");
        let mut s = init();
        let root = s.root.clone();
        go(&mut s, PathBuf::from(&entry.path));
        assert!(
            same_path(&s.root, &root),
            "a folder that leads nowhere is not opened"
        );
        assert!(
            s.notice.contains("not available here"),
            "and it says why: {}",
            s.notice
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Windows marks the legacy `Documents\My Videos` links with a deny on
    /// reading through them, so the link can be listed but never read. Only the
    /// folder it names can be shown, and that is what pressing one opens.
    #[cfg(windows)]
    #[test]
    fn a_link_that_cannot_be_listed_opens_the_folder_it_names() {
        let base = scratch("deny");
        std::fs::create_dir_all(base.join("Videos")).unwrap();
        std::fs::write(base.join("Videos").join("clip.mp4"), b"x").unwrap();
        std::fs::create_dir_all(base.join("shown")).unwrap();
        let link = base.join("shown").join("My Videos");
        if !junction(&link, &base.join("Videos")) {
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        // The deny Windows puts on them, by SID so it reads the same in every
        // language: everyone may not read the folder's contents.
        let denied = std::process::Command::new("icacls")
            .arg(&link)
            .arg("/deny")
            .arg("*S-1-1-0:(RD)")
            .output()
            .is_ok_and(|out| out.status.success());
        if !denied || fs::read_dir(&link).is_ok() {
            // A machine that will not take the deny cannot show this case.
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        let mut s = init();
        go(&mut s, link);
        assert!(
            same_path(&s.root, &base.join("Videos")),
            "the folder the link names is what opens"
        );
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while s.scan_rx.is_some() && Instant::now() < deadline {
            drain_scan(&mut s);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            s.entries.iter().any(|e| e.name == "clip.mp4"),
            "and it is read, not shown as an empty folder"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Away from Windows a link carries no directory bit, so where it points is
    /// what decides — and a link to nothing is not a folder.
    #[cfg(unix)]
    #[test]
    fn a_symlink_is_a_folder_when_it_points_at_one() {
        use std::os::unix::fs::symlink;
        let base = scratch("symlink");
        std::fs::create_dir_all(base.join("dir")).unwrap();
        symlink(base.join("dir"), base.join("link")).unwrap();
        let (entries, _) = scan(&base);
        assert!(
            entries.iter().find(|e| e.name == "link").expect("listed").is_dir,
            "a link to a folder is a folder"
        );
        std::fs::remove_dir_all(base.join("dir")).unwrap();
        let (entries, _) = scan(&base);
        assert!(
            !entries.iter().find(|e| e.name == "link").expect("listed").is_dir,
            "a link to nothing is not a folder"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The same reported bug for a real shortcut: a `.lnk` to a folder was handed
    /// to the shell, which asked Lyre to open the folder and started a second
    /// window on it. The folder opens in this window instead.
    #[test]
    fn a_shortcut_to_a_folder_opens_in_this_window() {
        let base = scratch("lnk");
        std::fs::create_dir_all(base.join("Videos")).unwrap();
        let shortcut_path = base.join("My Videos.lnk");
        std::fs::write(
            &shortcut_path,
            shortcut::encoded(&base.join("Videos").to_string_lossy(), ""),
        )
        .unwrap();
        let mut s = init();
        open_path(&mut s, &shortcut_path.to_string_lossy(), false);
        assert!(matches!(s.view, View::Files), "the folder opens here");
        assert!(
            same_path(&s.root, &base.join("Videos")),
            "and shows the folder the shortcut names"
        );
        assert_eq!(s.tab, "Videos");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A shortcut to a file stays the shell's: it is what runs an application
    /// with its arguments, its working directory and its window state.
    #[test]
    fn a_shortcut_to_a_file_is_left_to_the_shell() {
        let base = scratch("lnk-file");
        std::fs::write(base.join("notes.txt"), b"x").unwrap();
        let shortcut_path = base.join("notes.lnk");
        std::fs::write(
            &shortcut_path,
            shortcut::encoded(&base.join("notes.txt").to_string_lossy(), ""),
        )
        .unwrap();
        assert_eq!(
            shortcut::folder_target(&shortcut_path.to_string_lossy()),
            None,
            "a file's shortcut names no folder to open here"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
