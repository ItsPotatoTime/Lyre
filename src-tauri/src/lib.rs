//! Lyre's Rust core.
//!
//! Everything that touches the filesystem lives here so the interface only
//! ever receives rows it is about to paint. Scans are cancellable, streamed,
//! and never block the thread the window is waiting on.

mod defaultmanager;
mod dropstage;
mod eject;
mod fileops;
mod fold;
mod graphics;
mod updates;
#[cfg(windows)]
mod hotkey;
mod openwith;
mod preview;
mod ops;
mod places;
mod scan;
mod search;
mod shellmenu;
mod watch;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, State};

#[derive(Default)]
struct AppState {
    jobs: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    next_id: AtomicU64,
    /// What the last scan of each folder read, so search never reads the disk
    /// for a folder that is already on screen.
    index: Arc<search::Index>,
}

impl AppState {
    fn begin(&self) -> (u64, Arc<AtomicBool>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.insert(id, Arc::clone(&cancel));
        }
        (id, cancel)
    }

    fn cancel(&self, id: u64) {
        if let Ok(mut jobs) = self.jobs.lock() {
            if let Some(flag) = jobs.remove(&id) {
                flag.store(true, Ordering::Relaxed);
            }
        }
    }

    fn finish(&self, id: u64) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(&id);
        }
    }
}

/// Read a directory and stream the rows back.
///
/// Returns immediately with a job id; the work happens on a detached thread so
/// that a slow volume cannot stall the command dispatcher. The id is only
/// needed to cancel.
#[tauri::command]
fn scan_dir(
    state: State<'_, AppState>,
    request: scan::ScanRequest,
    on_event: Channel<scan::ScanEvent>,
) -> Result<u64, String> {
    let (id, cancel) = state.begin();
    let index = Arc::clone(&state.index);
    let path = std::path::PathBuf::from(&request.path);
    let trace = std::env::var_os("LYRE_TRACE").is_some();
    if trace {
        eprintln!(
            "[lyre] scan {id} start {path:?} hidden={} protected={}",
            request.show_hidden, request.show_protected
        );
    }

    // The state is reached through the app handle inside the thread; keeping a
    // clone of the Arc flags is enough because `finish` only needs the id.
    std::thread::Builder::new()
        .name("lyre-scan".into())
        .spawn(move || {
            // Collected in the compact form the cache keeps, so a scan does
            // not clone every row's path and extension just to index it.
            let mut collected: Vec<search::CompactEntry> = Vec::new();
            let mut complete = false;
            scan::scan(
                &path,
                request.show_hidden,
                request.show_protected,
                &cancel,
                |event| {
                    if trace {
                        if let scan::ScanEvent::Done { total, elapsed_ms } = &event {
                            eprintln!("[lyre] scan {id} done {total} in {elapsed_ms} ms");
                        }
                    }
                    match &event {
                        scan::ScanEvent::Batch { entries, .. } => {
                            collected.extend(entries.iter().map(search::CompactEntry::from))
                        }
                        scan::ScanEvent::Done { .. } => complete = true,
                        scan::ScanEvent::Failed { .. } => {}
                    }
                    on_event.send(event).is_ok()
                },
            );
            // Only a complete read is worth indexing; a cancelled scan would
            // leave the search answering from a partial folder.
            if complete {
                index.store(&path, Arc::new(collected));
            }
        })
        .map_err(|err| format!("Could not start the directory scan: {err}"))?;

    Ok(id)
}

/// Release a scan's bookkeeping and ask it to stop.
#[tauri::command]
fn cancel_job(state: State<'_, AppState>, id: u64) {
    state.cancel(id);
}

/// Called by the interface once it has consumed a job, so the map does not grow
/// for the life of the window.
#[tauri::command]
fn release_job(state: State<'_, AppState>, id: u64) {
    state.finish(id);
}

/// Start a search and stream results back.
///
/// Returns a job id immediately: the first results come from memory and are
/// emitted on the worker thread, so nothing about this call waits.
#[tauri::command]
fn search_dir(
    state: State<'_, AppState>,
    request: search::SearchRequest,
    on_event: Channel<search::SearchEvent>,
) -> Result<u64, String> {
    let (id, cancel) = state.begin();
    let index = Arc::clone(&state.index);
    let trace = std::env::var_os("LYRE_TRACE").is_some();
    if trace {
        eprintln!(
            "[lyre] search {id} {:?} in {:?} recursive={}",
            request.query, request.path, request.recursive
        );
    }

    std::thread::Builder::new()
        .name("lyre-search".into())
        .spawn(move || {
            search::search(&index, &request, &cancel, |event| {
                if trace {
                    if let search::SearchEvent::Done {
                        matched,
                        scanned,
                        elapsed_ms,
                        ..
                    } = &event
                    {
                        eprintln!(
                            "[lyre] search {id} done {matched} matches, {scanned} scanned, {elapsed_ms} ms"
                        );
                    }
                }
                on_event.send(event).is_ok()
            });
        })
        .map_err(|err| format!("Could not start the search: {err}"))?;

    Ok(id)
}

/// Create a folder and return the path it actually used, so the interface can
/// select it and start renaming, which is what Explorer does.
#[tauri::command]
fn create_folder(
    state: State<'_, AppState>,
    parent: String,
    name: String,
) -> Result<String, String> {
    let created = ops::create_folder(&parent, &name)?;
    // A cached listing for the parent is now missing a row.
    state.index.forget(std::path::Path::new(&parent));
    Ok(created)
}

/// Rename an item and return its new path.
#[tauri::command]
fn rename_path(
    state: State<'_, AppState>,
    path: String,
    name: String,
) -> Result<String, String> {
    let renamed = ops::rename_path(&path, &name)?;
    if let Some(parent) = std::path::Path::new(&path).parent() {
        state.index.forget(parent);
    }
    Ok(renamed)
}

/// Paste copied or cut entries into a folder, streaming progress.
#[tauri::command]
fn paste_into(
    state: State<'_, AppState>,
    drop_state: State<'_, dropstage::DropState>,
    destination: String,
    sources: Vec<String>,
    mode: fileops::Mode,
    conflict: fileops::Conflict,
    on_event: Channel<fileops::PasteEvent>,
) -> Result<u64, String> {
    let (id, cancel) = state.begin();
    let index = Arc::clone(&state.index);
    let destination_for_cache = destination.clone();
    let sources_for_cache = sources.clone();
    // `State` is a borrow of the app; the paste thread needs an owned handle.
    let drop_state = (*drop_state).clone();

    std::thread::Builder::new()
        .name("lyre-paste".into())
        .spawn(move || {
            fileops::paste(&destination, &sources, mode, conflict, &cancel, |event| {
                on_event.send(event).is_ok()
            });
            // A staged extraction drop has now been read; nothing else needs
            // the hard links that kept it alive.
            dropstage::release_after_paste(&sources, &drop_state);
            // Both ends changed: the destination gained rows and, when moving,
            // the source folders lost them.
            index.forget(std::path::Path::new(&destination_for_cache));
            for source in sources_for_cache {
                if let Some(parent) = std::path::Path::new(&source).parent() {
                    index.forget(parent);
                }
            }
        })
        .map_err(|err| format!("Could not start the paste: {err}"))?;

    Ok(id)
}

/// Which of these items already exist in the destination, so the interface can
/// ask before anything is written.
#[tauri::command]
fn paste_conflicts(destination: String, sources: Vec<String>) -> Vec<String> {
    fileops::conflicts(&destination, &sources)
}

/// Move entries to the trash, streaming progress the way a paste does.
#[tauri::command]
fn delete_paths(
    state: State<'_, AppState>,
    paths: Vec<String>,
    on_event: Channel<fileops::DeleteEvent>,
) -> Result<u64, String> {
    let (id, cancel) = state.begin();
    let index = Arc::clone(&state.index);
    let paths_for_cache = paths.clone();

    std::thread::Builder::new()
        .name("lyre-delete".into())
        .spawn(move || {
            fileops::delete(&paths, &cancel, |event| on_event.send(event).is_ok());
            for path in paths_for_cache {
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    index.forget(parent);
                }
            }
        })
        .map_err(|err| format!("Could not start the delete: {err}"))?;

    Ok(id)
}

/// Which applications can open this file, and which one currently does.
#[tauri::command]
async fn apps_for_file(path: String) -> openwith::OpenWith {
    tauri::async_runtime::spawn_blocking(move || openwith::open_with(&path))
        .await
        .unwrap_or_else(|_| openwith::OpenWith {
            mime: String::new(),
            default_app: None,
            apps: Vec::new(),
            unsupported: Some("Could not read the application list.".into()),
            system_picker: false,
        })
}

/// A file path to the icon the platform shows while dragging out of the window.
///
/// The plugin wants a path, not bytes, so the icon is written once into the
/// temporary directory. `include_bytes!` means this works the same in a
/// development run and in a bundled application, where resource paths differ.
#[tauri::command]
fn drag_icon() -> Result<String, String> {
    let path = std::env::temp_dir().join("lyre-drag-icon.png");
    if !path.exists() {
        std::fs::write(&path, include_bytes!("../icons/32x32.png"))
            .map_err(|err| format!("Could not prepare the drag preview: {err}"))?;
    }
    Ok(path.to_string_lossy().into_owned())
}

/// What the preview pane should show for a file.
#[tauri::command]
async fn preview_info(path: String) -> preview::PreviewInfo {
    // Synchronous commands run on the webview's own thread, and this one reads
    // a file. Off the main loop it goes.
    tauri::async_runtime::spawn_blocking(move || preview::info(&path))
        .await
        .unwrap_or_else(|_| preview::unavailable())
}

/// The thumbnail for a file, as PNG bytes rather than a path, so the webview
/// never gets permission to read the file itself.
#[tauri::command]
async fn preview_thumbnail(path: String) -> Result<tauri::ipc::Response, String> {
    // This spawns ffmpeg or pdftoppm and waits for it. Run synchronously it
    // blocked the main thread for the length of every thumbnail, which froze
    // the window on any folder with pictures in it.
    let bytes = tauri::async_runtime::spawn_blocking(move || preview::thumbnail(&path))
        .await
        .map_err(|err| err.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Whether the reduced-memory graphics path is set, and whether it is active.
#[tauri::command]
fn graphics_settings() -> graphics::GraphicsSettings {
    graphics::state()
}

/// Turn the reduced-memory graphics path on or off. Needs a restart.
#[tauri::command]
fn set_graphics_settings(reduced_memory: bool) -> Result<graphics::GraphicsSettings, String> {
    graphics::set(reduced_memory)
}

/// A line of diagnostics from the interface, printed when LYRE_TRACE is set.
/// Used for the paths that cross the boundary to the platform, where a silent
/// failure is otherwise invisible.
#[tauri::command]
fn trace(message: String) {
    if std::env::var_os("LYRE_TRACE").is_some() {
        eprintln!("[lyre] {message}");
    }
}

/// Watch for volumes appearing and disappearing while the window is open.
///
/// A stick plugged in after startup was invisible until the next manual
/// refresh. This polls the volume list, compares a fingerprint that ignores
/// free space, and tells the window when it actually changed. Two seconds is
/// the compromise: fast enough that plugging something in feels noticed, slow
/// enough that the enumeration is invisible.
fn watch_places(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("lyre-places".into())
        .spawn(move || {
            let mut last = places::fingerprint();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let current = places::fingerprint();
                if current == last {
                    continue;
                }
                last = current;
                if std::env::var_os("LYRE_TRACE").is_some() {
                    eprintln!("[lyre] volumes changed, {} entries", last.len());
                }
                let _ = app.emit("places-changed", places::places());
            }
        })
        .ok();
}

/// Unmount a removable volume and power the device down.
#[tauri::command]
async fn eject_volume(path: String) -> eject::EjectOutcome {
    // Unmounting can take seconds, and the window must stay alive while it does.
    tauri::async_runtime::spawn_blocking(move || eject::eject(&path))
        .await
        .unwrap_or_else(|_| eject::EjectOutcome {
            ejected: false,
            steps: Vec::new(),
            error: Some("The eject task stopped unexpectedly.".into()),
        })
}

/// Whether this application is the desktop's folder handler, and if not, what
/// is. Called when the window opens to decide whether to offer the switch.
#[tauri::command]
async fn file_manager_state() -> defaultmanager::ManagerState {
    tauri::async_runtime::spawn_blocking(defaultmanager::state)
        .await
        .unwrap_or_else(|_| defaultmanager::unavailable())
}

/// Register as the handler for folders.
#[tauri::command]
async fn become_file_manager() -> Result<defaultmanager::ManagerState, String> {
    tauri::async_runtime::spawn_blocking(defaultmanager::make_default)
        .await
        .map_err(|err| err.to_string())?
}

/// Undo the folder handler change, putting the platform's own back.
#[tauri::command]
async fn restore_file_manager() -> Result<defaultmanager::ManagerState, String> {
    tauri::async_runtime::spawn_blocking(defaultmanager::restore)
        .await
        .map_err(|err| err.to_string())?
}

/// The folder this process was launched with, if it was launched with one.
#[tauri::command]
fn startup_folder() -> Option<String> {
    defaultmanager::startup_path()
}

/// Is there a newer build on the release page?
#[tauri::command]
async fn check_for_update(app: tauri::AppHandle) -> Result<Option<updates::Offer>, String> {
    updates::check(&app).await
}

/// Download and install it, reporting the download as it goes.
#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    on_event: Channel<updates::UpdateEvent>,
) -> Result<(), String> {
    updates::install(&app, on_event).await
}

/// Restart into the build that was just installed.
#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

/// Whether Win+E is taken even when no Lyre window is open, which is what the
/// logon agent provides.
#[tauri::command]
fn win_e_agent() -> bool {
    #[cfg(windows)]
    {
        hotkey::agent_enabled()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Turn the Win+E agent on or off, and answer with what is now true.
#[tauri::command]
async fn set_win_e_agent(enabled: bool) -> Result<bool, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || hotkey::set_agent(enabled))
            .await
            .map_err(|err| err.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err("Win+E belongs to Explorer on this platform.".into())
    }
}

/// Show the platform's own application picker, for platforms that protect the
/// list and the default.
#[tauri::command]
fn choose_app_with_system(path: String) -> Result<(), String> {
    openwith::choose_with_system(&path)
}

/// Open this file with a specific application.
#[tauri::command]
fn open_with_app(path: String, app_id: String) -> Result<(), String> {
    openwith::open_with_app(&path, &app_id)
}

/// Make an application the default for this file's type.
#[tauri::command]
fn set_default_app(path: String, app_id: String) -> Result<String, String> {
    openwith::set_default_app(&path, &app_id)
}

/// Everything the properties dialog shows for one item.
#[tauri::command]
fn path_properties(path: String) -> Result<ops::Properties, String> {
    ops::properties(&path)
}

/// What the system's own menu would offer for this target.
///
/// A handler is asked what it would draw, which means loading it, so this runs
/// off the thread that draws the window. It is also called before the menu
/// opens, so the answer is usually already there when it is needed.
#[tauri::command(async)]
fn shell_menu(path: Option<String>, background: bool) -> shellmenu::Catalog {
    shellmenu::catalog(path.as_deref(), background)
}

/// Run one of those items, through the program that registered it.
#[tauri::command(async)]
fn shell_menu_run(id: String, paths: Vec<String>) -> Result<(), String> {
    shellmenu::run(&id, &paths)
}

/// Resolve the breadcrumb, parent and existence of a path in one round trip.
#[tauri::command]
fn path_info(path: String) -> places::PathInfo {
    places::path_info(&path)
}

/// The navigation rail's contents: home, known folders, and mounted volumes.
#[tauri::command]
async fn places() -> Vec<places::Place> {
    // Enumerating volumes touches every mount and reads the udev links, so it
    // belongs off the thread that draws the window.
    let list = tauri::async_runtime::spawn_blocking(places::places)
        .await
        .unwrap_or_default();
    if std::env::var_os("LYRE_TRACE").is_some() {
        let volumes: Vec<&places::Place> =
            list.iter().filter(|place| place.kind != "folder").collect();
        eprintln!("[lyre] places {} entries, {} volumes", list.len(), volumes.len());
        for volume in volumes {
            eprintln!(
                "[lyre]   {:?} at {:?} detail={:?} {}/{} bytes",
                volume.name, volume.path, volume.detail, volume.free, volume.total
            );
        }
    }
    list
}

/// The program each platform uses to hand a file to its registered handler.
#[cfg(target_os = "windows")]
fn opener() -> std::process::Command {
    let mut command = std::process::Command::new("cmd");
    // `start` treats its first argument as the window title, so an empty one
    // keeps a path containing spaces from being read as the title.
    command.args(["/C", "start", ""]);
    command
}

#[cfg(target_os = "macos")]
fn opener() -> std::process::Command {
    std::process::Command::new("open")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn opener() -> std::process::Command {
    std::process::Command::new("xdg-open")
}

/// Open a file with whatever the desktop associates with its type.
///
/// Spawned and dropped rather than awaited: the handler outlives us, and a
/// slow application launch must not block the interface.
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.exists() {
        return Err("That item no longer exists.".into());
    }
    opener()
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Could not open that item: {err}"))
}

/// Start directory for a fresh window.
#[tauri::command]
fn home_dir() -> String {
    places::home().to_string_lossy().into_owned()
}

/// Watch the folder the window is showing. `null` stops watching, which is
/// what Home, This PC and search pages do. The main process owns the thread;
/// the window only ever says which folder matters right now.
#[tauri::command]
fn watch_folder(
    path: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, watch::WatchState>,
) {
    watch::set_folder(path, app, &state);
}

/// Exchange a drop's original paths for the snapshot taken on the event loop.
/// An archiver deletes its extraction folder before the webview can paste, so
/// this is the only safe set of paths to hand to `paste_into`.
#[tauri::command]
fn staged_drop_paths(paths: Vec<String>, state: State<'_, dropstage::DropState>) -> Vec<String> {
    dropstage::take(paths, &state)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before the webview exists: WebKit reads this once, at initialisation.
    graphics::apply_at_startup();
    // A helper run: no window, only the Win+E hook. It exists because a low
    // level keyboard hook is not reliably called while a window of the process
    // that installed it has the foreground, and this process never has one.
    let hook_helper = std::env::args().find_map(|arg| match arg.as_str() {
        "--win-e-helper" => Some(false),
        "--win-e-agent" => Some(true),
        _ => None,
    });
    if let Some(persist) = hook_helper {
        #[cfg(windows)]
        {
            hotkey::run_helper(persist);
            return;
        }
        #[cfg(not(windows))]
        {
            let _ = persist;
            return;
        }
    }
    // A launch that asked for something only Explorer can serve (a selection, a
    // shell namespace, one of its own switches) is handed straight to it, rather
    // than opening a window that ignores the request.
    if defaultmanager::forward_unhostable_launch() {
        return;
    }
    // Win+E is launched by full path, so no shell verb can catch it; this takes
    // the key itself, for as long as this application is the folder handler.
    #[cfg(windows)]
    hotkey::install();
    // Under LYRE_TRACE, a close reports which step it reached. A freeze can be
    // the request never arriving, the window not being disposed, or the event
    // loop not ending, and one line says which without attaching a debugger to
    // a window that no longer responds.
    let tracing = std::env::var_os("LYRE_TRACE").is_some();
    let app = tauri::Builder::default()
        // Updates come from the project's own releases, signed and checked
        // against the key in `tauri.conf.json`.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Lets a drag that reaches the window edge become a real operating
        // system drag, so files can be dropped into other applications.
        .plugin(tauri_plugin_drag::init())
        .setup(|app| {
            // A window created by a background process can be handed the
            // launcher's show state and come up minimized. Nothing here asks for
            // that, so if it happens, undo it. Only while it is actually
            // minimized, so a window the person put away is left alone.
            if let Some(window) = app.get_webview_window("main") {
                if window.is_minimized().unwrap_or(false) {
                    #[cfg(windows)]
                    hotkey::note("the window came up minimized; restoring it");
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                // The shell can apply its show state after creation returns, so
                // watch for a few seconds rather than only during setup. The
                // window is only corrected while it is actually minimized.
                let window = window.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        if window.is_minimized().unwrap_or(false) {
                            #[cfg(windows)]
                            hotkey::note("the window was minimized after starting; restoring it");
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                            break;
                        }
                    }
                });
            }
            watch_places(app.handle().clone());
            Ok(())
        })
        .manage(AppState::default())
        .manage(watch::WatchState::default())
        .manage(dropstage::DropState::default())
        .on_window_event(move |window, event| {
            // wry's `IDropTarget::Drop` only posts a message to this loop; by
            // the time it is read, an archiver may already have deleted its
            // extraction folder. `DragEnter` carries the same paths while the
            // drag is still live, so snapshot there first and keep `Drop` as a
            // second chance for a platform that sends no `Enter` paths.
            if let tauri::WindowEvent::DragDrop(drag) = event {
                match drag {
                    tauri::DragDropEvent::Enter { paths, .. }
                    | tauri::DragDropEvent::Drop { paths, .. } => {
                        let state = window.state::<dropstage::DropState>();
                        dropstage::stage_early(paths.clone(), &state);
                    }
                    tauri::DragDropEvent::Leave => {
                        let state = window.state::<dropstage::DropState>();
                        dropstage::cancel(&state);
                    }
                    _ => {}
                }
                return;
            }
            let tauri::WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            // Closing ends the process instead of disposing the window, and the
            // reason is a bug below us rather than a preference.
            //
            // Disposing the window runs `on_window_close` in tauri-runtime-wry,
            // which drops the Tao window while holding a mutable borrow of its
            // window store. The drag plugin leaves GTK signal handlers on the
            // window whenever a drag-out neither completes nor fails, and those
            // handlers are destroyed as part of that drop; their teardown
            // borrows the same store, so it panics with "RefCell already
            // mutably borrowed" inside an extern "C" callback, which cannot
            // unwind and therefore aborts. The desktop showed that as a window
            // that had stopped responding.
            //
            // Nothing is lost by skipping it: preferences are written when they
            // change, no edit is buffered, and the window is the only one.
            api.prevent_close();
            if tracing {
                eprintln!("[lyre] exit on close");
            }
            std::process::exit(0);
        })
        .invoke_handler(tauri::generate_handler![
            scan_dir,
            search_dir,
            cancel_job,
            release_job,
            path_info,
            places,
            home_dir,
            watch_folder,
            staged_drop_paths,
            open_path,
            create_folder,
            rename_path,
            path_properties,
            shell_menu,
            shell_menu_run,
            paste_into,
            paste_conflicts,
            delete_paths,
            apps_for_file,
            choose_app_with_system,
            file_manager_state,
            become_file_manager,
            restore_file_manager,
            startup_folder,
            win_e_agent,
            set_win_e_agent,
            check_for_update,
            install_update,
            restart_app,
            eject_volume,
            drag_icon,
            trace,
            graphics_settings,
            set_graphics_settings,
            preview_info,
            preview_thumbnail,
            open_with_app,
            set_default_app
        ])
        .build(tauri::generate_context!())
        .expect("error while building Lyre");
    // Closing calls process::exit, so a staging folder dropped at a cancelled
    // conflict dialog has no destructor to clean it up. Sweep the old ones.
    dropstage::sweep();
    app.run(move |_app, event| {
        if tracing && matches!(event, tauri::RunEvent::Exit) {
            eprintln!("[lyre] exit");
        }
    });
}

#[cfg(test)]
mod scan_tests;

#[cfg(test)]
mod search_tests;

#[cfg(test)]
mod opener_tests {
    use super::*;

    #[test]
    fn a_missing_item_is_reported_instead_of_launched() {
        let error = open_path("/definitely/not/here/lyre".into())
            .expect_err("a missing path must not be handed to the launcher");
        assert!(
            error.contains("no longer exists"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn each_platform_names_its_own_opener() {
        let program = opener().get_program().to_string_lossy().into_owned();
        #[cfg(target_os = "windows")]
        assert_eq!(program, "cmd");
        #[cfg(target_os = "macos")]
        assert_eq!(program, "open");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(program, "xdg-open");
    }
}
