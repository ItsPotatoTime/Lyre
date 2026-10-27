//! Lyre's Rust core.
//!
//! Everything that touches the filesystem lives here so the interface only
//! ever receives rows it is about to paint. Scans are cancellable, streamed,
//! and never block the thread the window is waiting on.

mod places;
mod scan;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::State;

#[derive(Default)]
struct ScanState {
    jobs: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    next_id: AtomicU64,
}

impl ScanState {
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
    state: State<'_, ScanState>,
    request: scan::ScanRequest,
    on_event: Channel<scan::ScanEvent>,
) -> Result<u64, String> {
    let (id, cancel) = state.begin();
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
            scan::scan(&path, request.show_hidden, &cancel, |event| {
                if trace {
                    if let scan::ScanEvent::Done { total, elapsed_ms } = &event {
                        eprintln!("[lyre] scan {id} done {total} in {elapsed_ms} ms");
                    }
                }
                on_event.send(event).is_ok()
            });
        })
        .map_err(|err| format!("Could not start the directory scan: {err}"))?;

    Ok(id)
}

/// Release a scan's bookkeeping and ask it to stop.
#[tauri::command]
fn cancel_scan(state: State<'_, ScanState>, id: u64) {
    state.cancel(id);
}

/// Called by the interface once it has consumed a job, so the map does not grow
/// for the life of the window.
#[tauri::command]
fn release_scan(state: State<'_, ScanState>, id: u64) {
    state.finish(id);
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
        .manage(ScanState::default())
        .invoke_handler(tauri::generate_handler![
            scan_dir,
            cancel_scan,
            release_scan,
            path_info,
            places,
            home_dir,
            dir_stamp,
            open_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running Lyre");
}

#[cfg(test)]
mod scan_tests;

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
