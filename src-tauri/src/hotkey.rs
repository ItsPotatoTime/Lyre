//! Win+E, redirected to this application while it is the folder handler.
//!
//! The shell does not route Win+E through a shell verb. Measured on Windows 11
//! 25H2, the key spawns `"C:\Windows\explorer.exe" ::{52205FD8-5DFB-447D-801A-
//! D0B52F2E83E1}` by full path, so no registry key can intercept it: the CLSID
//! command is honoured when that verb is called explicitly, but the hotkey never
//! calls it, and a full path bypasses `App Paths` too.
//!
//! That leaves the input itself. A low level keyboard hook sees the combination
//! before the shell's own hotkey handling and can swallow it.
//!
//! Four constraints shape the code:
//!
//! - The hook runs on a thread of its own, with a message pump. A low level hook
//!   is delivered through the queue of the thread that installed it, and the
//!   system waits only a short time for that thread to answer before it stops
//!   consulting it. Installing on the thread that also draws the window means a
//!   busy frame can lose the key.
//! - The callback does nothing slow. No file writes, no registry reads, no
//!   process creation: it reads atomics and posts to a channel that another
//!   thread drains. A callback that blocks gets the hook skipped, and then the
//!   shell opens Explorer while this code looks like it was never called.
//! - The Windows key and the `E` repeat are tracked from the key stream rather
//!   than read with `GetAsyncKeyState` at callback time, which answers about the
//!   moment the callback runs rather than the moment the key was pressed.
//! - The hook only exists while a process that installed it is alive, so Win+E
//!   falls back to Explorer when this application is closed.
//!
//! Every running instance installs its own hook, so one keystroke reaches all of
//! them. What stops that from opening a window per instance is a short lived
//! named mutex per press: the first to create it opens the window and the rest
//! stay quiet.
//!
//! Every press, and every press declined, leaves one line in
//! `%LOCALAPPDATA%\Lyre\hotkey.log` with the foreground window and the number of
//! key events the hook has seen. That is what distinguishes "the hook decided
//! not to take it" from "the hook never saw it".

use std::ffi::c_void;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;
use std::time::Duration;

use crate::defaultmanager;

/// Keeps `reg.exe` from flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Where a per-user logon entry lives.
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
/// The entry that keeps the agent running after every window is closed.
const RUN_NAME: &str = "LyreWinE";

const WH_KEYBOARD_LL: i32 = 13;
const WM_KEYDOWN: usize = 0x0100;
const WM_KEYUP: usize = 0x0101;
const WM_SYSKEYDOWN: usize = 0x0104;
const WM_SYSKEYUP: usize = 0x0105;
const VK_E: u32 = 0x45;
const VK_LWIN: i32 = 0x5B;
const VK_RWIN: i32 = 0x5C;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const ERROR_ALREADY_EXISTS: u32 = 183;
/// Keep the diagnostic file from growing without limit.
const LOG_LIMIT: u64 = 128 * 1024;
/// Posted to a thread by `SetTimer`, used here as a heartbeat.
const WM_TIMER: u32 = 0x0113;
/// How often the hook thread says it is still alive and counting.
const HEARTBEAT_MS: u32 = 300_000;

/// `KBDLLHOOKSTRUCT`: what the keyboard hook is handed for each key event.
#[repr(C)]
struct KbdLlHookStruct {
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

/// `MSG`, only ever used as somewhere for the queue to put its messages.
#[repr(C)]
struct Msg {
    window: usize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: Point,
    private: u32,
}

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

#[link(name = "user32")]
extern "system" {
    fn SetWindowsHookExW(
        id_hook: i32,
        proc: Option<HookProc>,
        module: usize,
        thread_id: u32,
    ) -> usize;
    fn CallNextHookEx(hook: usize, code: i32, wparam: usize, lparam: isize) -> isize;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetMessageW(msg: *mut Msg, window: usize, min: u32, max: u32) -> i32;
    fn GetForegroundWindow() -> usize;
    fn GetWindowThreadProcessId(window: usize, process: *mut u32) -> u32;
    fn SetTimer(
        window: usize,
        id: usize,
        elapse: u32,
        proc: Option<unsafe extern "system" fn(usize, u32, usize, u32)>,
    ) -> usize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn CreateMutexW(attributes: *mut c_void, owner: i32, name: *const u16) -> usize;
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> usize;
    fn CloseHandle(handle: usize) -> i32;
    fn GetLastError() -> u32;
    fn GetTickCount64() -> u64;
}

/// Enough access to open a mutex that is only being watched for existence.
const SYNCHRONIZE: u32 = 0x0010_0000;

/// What the callback decided, handed to the thread that does the slow work.
enum Work {
    /// The key belongs to this application and was swallowed.
    Taken { foreground: u32, events: u64 },
    /// The key was seen and left to the shell, with the reason.
    Passed {
        foreground: u32,
        events: u64,
        why: &'static str,
    },
}

/// The hook handle, or zero before it is installed.
static HOOK: AtomicUsize = AtomicUsize::new(0);
/// Whether a thread has already been started for the hook.
static STARTED: AtomicBool = AtomicBool::new(false);
/// The name a window process holds for as long as it is running.
static WINDOW: AtomicUsize = AtomicUsize::new(0);
/// The name the helper holds, so a second helper exits at once.
static HELPER: AtomicUsize = AtomicUsize::new(0);
/// Whether the Windows key is down, as seen in the key stream.
static WIN_DOWN: AtomicBool = AtomicBool::new(false);
/// Whether `E` is already down, so a held key opens one window and not one per
/// repeat.
static E_DOWN: AtomicBool = AtomicBool::new(false);
/// How many key events the hook has been handed, which says whether it is being
/// consulted at all.
static EVENTS: AtomicU64 = AtomicU64::new(0);
/// Whether this application is the folder handler, refreshed off the hook thread.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// When the Windows key was last seen in the stream, so a press that arrives
/// without a visible modifier can be recognised as that rather than as typing.
static WIN_SEEN_AT: AtomicU64 = AtomicU64::new(0);
/// Where the callback hands its work.
static WORK: OnceLock<Sender<Work>> = OnceLock::new();

fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Whether a key event is the combination this module takes: the Windows key
/// held, `E` pressed, and no other modifier, which leaves everything else
/// including Win+Shift+E to whoever wants it.
fn takes_the_key(vk_code: u32, win_down: bool, other_modifier_down: bool) -> bool {
    vk_code == VK_E && win_down && !other_modifier_down
}

fn key_down(key: i32) -> bool {
    // The high bit is set while the key is held. Used only as a fallback, because
    // it answers about the moment the callback runs.
    unsafe { GetAsyncKeyState(key) as u16 & 0x8000 != 0 }
}

fn log_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
    Some(base.join("Lyre").join("hotkey.log"))
}

/// One line per event worth explaining. Best effort: a hotkey must not fail
/// because a log could not be written. Never called from the hook thread.
fn log(line: &str) {
    let Some(path) = log_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path)
        .map(|meta| meta.len() > LOG_LIMIT)
        .unwrap_or(false)
    {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        // One write per line, so two threads writing at once cannot interleave
        // into a single unreadable line.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        let _ = file.write_all(format!("[{stamp}] {line}\n").as_bytes());
    }
}

/// Appends a line to the same diagnostic log from elsewhere in the application.
pub fn note(line: &str) {
    log(line);
}

/// Opens a window of this application, once per press even though every running
/// instance sees the same keystroke.
///
/// The first instance to create the press mutex opens the window and holds it
/// for a moment; the others find it already there and stay quiet. Holding it
/// also covers anything that slips past the repeat check. The spawn happens on
/// its own thread: starting a process takes long enough to be felt as input lag
/// otherwise.
fn open_window() {
    let name = wide(r"Local\Lyre.WinE.press");
    let press = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if press == 0 {
        log("could not create the press mutex");
        return;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(press) };
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        unsafe { CloseHandle(press) };
    });

    std::thread::spawn(|| {
        if let Ok(exe) = std::env::current_exe() {
            match std::process::Command::new(exe).spawn() {
                Ok(_) => log("spawned a window"),
                Err(err) => log(&format!("could not start a window: {err}")),
            }
        }
    });
}

/// Everything the hook is not allowed to do, done after it has returned.
fn drain(work: Receiver<Work>) {
    for item in work {
        match item {
            Work::Taken { foreground, events } => {
                log(&format!(
                    "win+e taken (pid {}, foreground {foreground}, {events} key events seen)",
                    std::process::id()
                ));
                open_window();
            }
            Work::Passed {
                foreground,
                events,
                why,
            } => {
                log(&format!(
                    "win+e passed on: {why} (pid {}, foreground {foreground}, {events} key events seen)",
                    std::process::id()
                ));
            }
        }
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    let seen = EVENTS.fetch_add(1, Ordering::Relaxed) + 1;
    // A negative code means the event is not ours to look at. It is recorded a
    // few times because "the hook is installed but only ever handed negative
    // codes" is a real failure mode, and from the outside it looks exactly like
    // no hook at all: Win+E opens Explorer and nothing is logged.
    if code < 0 {
        if seen <= 8 {
            if let Some(work) = WORK.get() {
                let _ = work.send(Work::Passed {
                    foreground: 0,
                    events: seen,
                    why: "the system handed the hook a negative code",
                });
            }
        }
    } else {
        let event = unsafe { &*(lparam as *const KbdLlHookStruct) };
        let events = seen;
        let pressed = wparam == WM_KEYDOWN || wparam == WM_SYSKEYDOWN;
        let released = wparam == WM_KEYUP || wparam == WM_SYSKEYUP;

        // The modifier state is taken from the stream, in order, so it describes
        // the press being reported rather than the moment this code runs.
        if event.vk_code == VK_LWIN as u32 || event.vk_code == VK_RWIN as u32 {
            WIN_SEEN_AT.store(now_ms(), Ordering::Relaxed);
            if pressed {
                WIN_DOWN.store(true, Ordering::Relaxed);
            } else if released {
                WIN_DOWN.store(false, Ordering::Relaxed);
            }
        }

        if event.vk_code == VK_E {
            if released {
                E_DOWN.store(false, Ordering::Relaxed);
            }
            if pressed {
                let streamed = WIN_DOWN.load(Ordering::Relaxed);
                let observed = key_down(VK_LWIN) || key_down(VK_RWIN);
                // Plain typing has to stay silent and untouched, or every held
                // `e` in the system would be swallowed and every one logged.
                if streamed || observed {
                    let repeat = E_DOWN.swap(true, Ordering::Relaxed);
                    let other = key_down(VK_SHIFT) || key_down(VK_CONTROL) || key_down(VK_MENU);
                    let foreground = unsafe { GetForegroundWindow() } as u32;
                    let mut process = 0u32;
                    unsafe { GetWindowThreadProcessId(foreground as usize, &mut process) };
                    if !takes_the_key(event.vk_code, true, other) {
                        if let Some(work) = WORK.get() {
                            let _ = work.send(Work::Passed {
                                foreground: process,
                                events,
                                why: "another modifier is down",
                            });
                        }
                    } else if !ACTIVE.load(Ordering::Relaxed) {
                        if let Some(work) = WORK.get() {
                            let _ = work.send(Work::Passed {
                                foreground: process,
                                events,
                                why: "Lyre is not the folder handler",
                            });
                        }
                    } else {
                        if !repeat {
                            if let Some(work) = WORK.get() {
                                let _ = work.send(Work::Taken {
                                    foreground: process,
                                    events,
                                });
                            }
                        }
                        // Swallow it, and every repeat while it is held, so the
                        // shell never sees the combination and a held key still
                        // opens one window.
                        return 1;
                    }
                } else if now_ms().saturating_sub(WIN_SEEN_AT.load(Ordering::Relaxed)) < 1500 {
                    // The key was physically pressed moments ago but neither the
                    // stream nor the async state can see it. That is worth saying,
                    // because the key is then left to the shell and Explorer opens.
                    let foreground = unsafe { GetForegroundWindow() } as u32;
                    let mut process = 0u32;
                    unsafe { GetWindowThreadProcessId(foreground as usize, &mut process) };
                    if let Some(work) = WORK.get() {
                        let _ = work.send(Work::Passed {
                            foreground: process,
                            events,
                            why: "the windows key was pressed but its state was not visible",
                        });
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(0, code, wparam, lparam) }
}

/// Keeps the handler flag current, so the callback never touches the registry.
fn watch_handler() {
    loop {
        let active = defaultmanager::is_current_handler();
        let previous = ACTIVE.swap(active, Ordering::Relaxed);
        if previous != active {
            log(if active {
                "Lyre is the folder handler; win+e is taken"
            } else {
                "Lyre is not the folder handler; win+e is left alone"
            });
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Starts the helper that owns the key, once per window process.
///
/// The hook itself does not live here. A low level keyboard hook is not called
/// while a window of the process that installed it has the foreground; measured
/// on Windows 11 25H2, the press that should have opened a window left the
/// window's own hook with zero events while a helper process saw it. So the
/// window keeps a name that says it exists, and the helper does the listening.
pub fn install() {
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }

    // Held for the life of this window process, so the helper can tell when the
    // last window is gone.
    let name = wide(r"Local\Lyre.WinE.window");
    let window = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if window != 0 {
        WINDOW.store(window, Ordering::Relaxed);
    }

    start_helper();

    // Keep one alive for as long as this window is. The agent can be switched off
    // or killed while a window is open, and the key would otherwise go back to
    // Explorer with the window still there.
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(10));
        if !helper_exists() {
            start_helper();
        }
    });
}

/// Whether any process is already listening for the key, agent or helper.
fn helper_exists() -> bool {
    let name = wide(r"Local\Lyre.WinE.helper");
    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
    if handle == 0 {
        return false;
    }
    unsafe { CloseHandle(handle) };
    true
}

/// Starts the listener, agent if it is wanted and a plain helper otherwise.
///
/// A process that is already listening exits at once on its own name, so calling
/// this repeatedly is safe.
fn start_helper() {
    let Ok(exe) = std::env::current_exe() else {
        log("could not locate this application to start the win+e helper");
        return;
    };
    let mode = if agent_enabled() {
        "--win-e-agent"
    } else {
        "--win-e-helper"
    };
    if let Err(err) = std::process::Command::new(exe).arg(mode).spawn() {
        log(&format!("could not start the win+e helper: {err}"));
    }
}

/// Whether a Lyre window process is still running.
fn window_exists() -> bool {
    let name = wide(r"Local\Lyre.WinE.window");
    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
    if handle == 0 {
        return false;
    }
    unsafe { CloseHandle(handle) };
    true
}

/// Whether the agent is registered to start with the user's session.
pub fn agent_enabled() -> bool {
    let output = std::process::Command::new("reg")
        .args(["query", RUN_KEY, "/v", RUN_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    matches!(output, Ok(out) if out.status.success())
}

/// Writes or removes the logon entry. Removing one that is not there is not an
/// error, because the answer wanted is "it is gone".
fn register_agent(enabled: bool) -> Result<(), String> {
    let mut command = std::process::Command::new("reg");
    command.creation_flags(CREATE_NO_WINDOW);
    if enabled {
        let exe = std::env::current_exe()
            .map_err(|err| format!("Could not locate this application: {err}"))?;
        let value = format!("\"{}\" --win-e-agent", exe.display());
        command.args([
            "add", RUN_KEY, "/v", RUN_NAME, "/t", "REG_SZ", "/d", &value, "/f",
        ]);
        let output = command
            .output()
            .map_err(|err| format!("Could not run reg: {err}"))?;
        if !output.status.success() {
            let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if reason.is_empty() {
                "Windows would not write the logon entry.".to_string()
            } else {
                reason
            });
        }
        return Ok(());
    }
    let _ = command
        .args(["delete", RUN_KEY, "/v", RUN_NAME, "/f"])
        .output();
    Ok(())
}

/// Starts the agent without waiting for the next session.
fn start_agent() {
    if let Ok(exe) = std::env::current_exe() {
        if let Err(err) = std::process::Command::new(exe)
            .arg("--win-e-agent")
            .spawn()
        {
            log(&format!("could not start the win+e agent: {err}"));
        }
    }
}

/// Turns the "Win+E even when Lyre is closed" behaviour on or off.
///
/// Turning it on writes the logon entry and starts the agent at once. Turning it
/// off removes the entry; a running agent notices within a few seconds and
/// stops, which is simpler than a second channel between the two.
pub fn set_agent(enabled: bool) -> Result<bool, String> {
    register_agent(enabled)?;
    if enabled {
        start_agent();
    }
    Ok(agent_enabled())
}

/// The helper run: owns the key, never owns a window.
///
/// `persist` is the agent started with the session, which has to outlive every
/// window. Otherwise the helper stops once the last window closes, so it does not
/// sit there holding the executable open after the user is done.
///
/// Returns immediately if another helper already holds the name, so windows can
/// start one whenever they like.
pub fn run_helper(persist: bool) {
    let name = wide(r"Local\Lyre.WinE.helper");
    let owner = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if owner == 0 {
        return;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(owner) };
        return;
    }
    HELPER.store(owner, Ordering::Relaxed);

    if persist {
        // The agent lives until it is turned off, and the logon entry is the
        // switch. Checking it is cheaper than a second way to be told.
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(5));
            if !agent_enabled() {
                log("the win+e agent is no longer wanted; stopping");
                std::process::exit(0);
            }
        });
    } else {
        // Outlive the windows, but not the application: once the last window is
        // gone there is nothing to open, and a process that stays would hold the
        // executable open against the next update.
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(5));
            if !window_exists() {
                log("no Lyre window left; the win+e helper is stopping");
                std::process::exit(0);
            }
        });
    }

    let (sender, receiver) = mpsc::channel();
    let _ = WORK.set(sender);
    let _ = std::thread::Builder::new()
        .name("win-e-worker".into())
        .spawn(move || drain(receiver));
    let _ = std::thread::Builder::new()
        .name("win-e-handler".into())
        .spawn(watch_handler);

    let started = std::thread::Builder::new()
        .name("win-e".into())
        .spawn(|| {
            // A failure here is not permanent: another application can be holding
            // the hook slot or the desktop can be mid-transition, so keep trying
            // rather than leaving the key with Explorer for the rest of the run.
            loop {
                let hook = unsafe {
                    SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(keyboard_proc),
                        GetModuleHandleW(std::ptr::null()),
                        0,
                    )
                };
                if hook != 0 {
                    HOOK.store(hook, Ordering::Relaxed);
                    log(&format!("keyboard hook installed (pid {})", std::process::id()));
                    log("hook thread pumping");
                    break;
                }
                log(&format!(
                    "could not install the keyboard hook (error {})",
                    unsafe { GetLastError() }
                ));
                std::thread::sleep(Duration::from_secs(5));
            }

            // The hook is called while this thread waits for messages, so the
            // loop has to keep running even though nothing is ever handled.
            let mut message = Msg {
                window: 0,
                message: 0,
                wparam: 0,
                lparam: 0,
                time: 0,
                point: Point { x: 0, y: 0 },
                private: 0,
            };
            // A timer wakes this thread now and then so the log can say the hook
            // is still installed and still counting. Without it, a hook that has
            // been dropped and a hook that is merely never called look the same.
            unsafe { SetTimer(0, 0, HEARTBEAT_MS, None) };
            loop {
                let result = unsafe { GetMessageW(&mut message, 0, 0, 0) };
                if result <= 0 {
                    log(&format!("hook thread stopped pumping (result {result})"));
                    break;
                }
                if message.message == WM_TIMER {
                    log(&format!(
                        "hook thread alive, {} key events seen",
                        EVENTS.load(Ordering::Relaxed)
                    ));
                }
            }
        });

    if started.is_err() {
        log("could not start the hook thread");
        return;
    }

    // Nothing else belongs to this process. The hook thread, the worker and the
    // watchdog all outlive this call.
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_key_and_e_is_the_combination() {
        assert!(takes_the_key(VK_E, true, false));
    }

    #[test]
    fn e_on_its_own_and_other_combinations_are_left_alone() {
        // Typing the letter must keep working.
        assert!(!takes_the_key(VK_E, false, false));
        // Win+Shift+E, Win+Ctrl+E and Win+Alt+E belong to somebody else.
        assert!(!takes_the_key(VK_E, true, true));
        // Every other key with the Windows key held stays with the shell.
        assert!(!takes_the_key(0x44, true, false));
    }
}
