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
    places::places()
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
            dir_stamp
        ])
        .run(tauri::generate_context!())
        .expect("error while running Lyre");
}

#[cfg(test)]
mod scan_tests;
