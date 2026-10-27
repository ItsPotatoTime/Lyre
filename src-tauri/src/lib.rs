//! Lyre's Rust core.
//!
//! Everything that touches the filesystem lives here so the interface only
//! ever receives rows it is about to paint. Scans are cancellable, streamed,
//! and never block the thread the window is waiting on.

mod fileops;
mod openwith;
mod ops;
mod places;
mod scan;
mod search;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::State;

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
        eprintln!("[lyre] scan {id} start {path:?} hidden={}", request.show_hidden);
    }

    // The state is reached through the app handle inside the thread; keeping a
    // clone of the Arc flags is enough because `finish` only needs the id.
    std::thread::Builder::new()
        .name("lyre-scan".into())
        .spawn(move || {
            let mut collected: Vec<scan::Entry> = Vec::new();
            let mut complete = false;
            scan::scan(&path, request.show_hidden, &cancel, |event| {
                if trace {
                    if let scan::ScanEvent::Done { total, elapsed_ms } = &event {
                        eprintln!("[lyre] scan {id} done {total} in {elapsed_ms} ms");
                    }
                }
                match &event {
                    scan::ScanEvent::Batch { entries, .. } => collected.extend(entries.iter().cloned()),
                    scan::ScanEvent::Done { .. } => complete = true,
                    scan::ScanEvent::Failed { .. } => {}
                }
                on_event.send(event).is_ok()
            });
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
    destination: String,
    sources: Vec<String>,
    mode: fileops::Mode,
    on_event: Channel<fileops::PasteEvent>,
) -> Result<u64, String> {
    let (id, cancel) = state.begin();
    let index = Arc::clone(&state.index);
    let destination_for_cache = destination.clone();
    let sources_for_cache = sources.clone();

    std::thread::Builder::new()
        .name("lyre-paste".into())
        .spawn(move || {
            fileops::paste(&destination, &sources, mode, &cancel, |event| {
                on_event.send(event).is_ok()
            });
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

/// Move entries to the trash and report how many went.
#[tauri::command]
fn delete_to_trash(state: State<'_, AppState>, paths: Vec<String>) -> Result<usize, String> {
    let moved = fileops::delete_to_trash(&paths)?;
    for path in &paths {
        if let Some(parent) = std::path::Path::new(path).parent() {
            state.index.forget(parent);
        }
    }
    Ok(moved)
}

/// Which applications can open this file, and which one currently does.
#[tauri::command]
fn apps_for_file(path: String) -> openwith::OpenWith {
    openwith::open_with(&path)
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

/// Resolve the breadcrumb, parent and existence of a path in one round trip.
#[tauri::command]
fn path_info(path: String) -> places::PathInfo {
    places::path_info(&path)
}

/// The navigation rail's contents: home, known folders, and mounted volumes.
#[tauri::command]
fn places() -> Vec<places::Place> {
    let list = places::places();
    if std::env::var_os("LYRE_TRACE").is_some() {
        let volumes = list.iter().filter(|place| place.kind != "folder").count();
        eprintln!(
            "[lyre] places {} entries, {volumes} volumes",
            list.len()
        );
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

/// A cheap stamp used to decide whether a cached listing is still valid.
/// Comparing the directory's own mtime is one `stat`, which is far cheaper
/// than re-reading the directory to find out nothing changed.
#[tauri::command]
fn dir_stamp(path: String) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            scan_dir,
            search_dir,
            cancel_job,
            release_job,
            path_info,
            places,
            home_dir,
            dir_stamp,
            open_path,
            create_folder,
            rename_path,
            path_properties,
            paste_into,
            delete_to_trash,
            apps_for_file,
            choose_app_with_system,
            open_with_app,
            set_default_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running Lyre");
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
