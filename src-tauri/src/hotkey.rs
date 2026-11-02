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
//! Three constraints shape the code:
//!
//! - The hook runs on a thread of its own, with a message pump. A low level hook
//!   is delivered through the queue of the thread that installed it, and the
//!   system waits only a short time for that thread to answer before it stops
//!   consulting it. Installing on the thread that also draws the window means a
//!   busy frame can lose the key.
//! - The Windows key is tracked from the key stream rather than read with
//!   `GetAsyncKeyState`, which answers about the moment the callback runs rather
//!   than the moment the key was pressed. A fast press can be answered late.
//! - The hook only exists while a process that installed it is alive, so Win+E
//!   falls back to Explorer when this application is closed.
//!
//! Every running instance installs its own hook, so one keystroke reaches all of
//! them. What stops that from opening a window per instance is a short lived
//! named mutex per press: the first to create it opens the window and the rest
//! stay quiet. An earlier version let only one instance hook, which meant that
//! closing that instance silently switched Win+E back to Explorer for every
//! window that was still open.
//!
//! Whether the key is ours to take is read at the moment it is pressed rather
//! than cached, so restoring Explorer from any window stops the redirection
//! immediately.
//!
//! Every press leaves one line in `%LOCALAPPDATA%\Lyre\hotkey.log`, which is what
//! makes "Win+E still opens Explorer" answerable: it says whether the key
//! arrived, what the modifier state was, and whether the key was taken.

use std::ffi::c_void;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::defaultmanager;

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
const LOG_LIMIT: u64 = 64 * 1024;

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
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn CreateMutexW(attributes: *mut c_void, owner: i32, name: *const u16) -> usize;
    fn CloseHandle(handle: usize) -> i32;
    fn GetLastError() -> u32;
}

/// The hook handle, or zero before it is installed.
static HOOK: AtomicUsize = AtomicUsize::new(0);
/// Whether a thread has already been started for the hook.
static STARTED: AtomicBool = AtomicBool::new(false);
/// Whether the Windows key is down, as seen in the key stream.
static WIN_DOWN: AtomicBool = AtomicBool::new(false);
/// Whether `E` is already down, so a held key opens one window and not one per
/// repeat.
static E_DOWN: AtomicBool = AtomicBool::new(false);

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
/// because a log could not be written.
fn log(line: &str) {
    let Some(path) = log_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).map(|meta| meta.len() > LOG_LIMIT).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{line}");
    }
}

/// Opens a window of this application, once per press even though every running
/// instance sees the same keystroke.
///
/// The first instance to create the press mutex opens the window and holds it
/// for a moment; the others find it already there and stay quiet. Holding it
/// also covers the key repeating while it is held down. The spawn happens off
/// the hook thread: starting a process takes long enough to be felt as input lag
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

unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code >= 0 {
        let event = unsafe { &*(lparam as *const KbdLlHookStruct) };
        let pressed = wparam == WM_KEYDOWN || wparam == WM_SYSKEYDOWN;
        let released = wparam == WM_KEYUP || wparam == WM_SYSKEYUP;

        // The modifier state is taken from the stream, in order, so it describes
        // the press being reported rather than the moment this code runs.
        if event.vk_code == VK_LWIN as u32 || event.vk_code == VK_RWIN as u32 {
            if pressed {
                WIN_DOWN.store(true, Ordering::Relaxed);
            } else if released {
                WIN_DOWN.store(false, Ordering::Relaxed);
            }
        }

        if event.vk_code == VK_E as u32 {
            if released {
                E_DOWN.store(false, Ordering::Relaxed);
            }
            if pressed {
                let streamed = WIN_DOWN.load(Ordering::Relaxed);
                let observed = key_down(VK_LWIN) || key_down(VK_RWIN);
                // Only a press of the Windows key is interesting. Plain typing has
                // to stay silent and untouched, or every `e` in the system would be
                // written to the log and every held `e` would be swallowed.
                if streamed || observed {
                    let repeat = E_DOWN.swap(true, Ordering::Relaxed);
                    let other = key_down(VK_SHIFT) || key_down(VK_CONTROL) || key_down(VK_MENU);
                    if takes_the_key(event.vk_code, true, other) {
                        if defaultmanager::is_current_handler() {
                            if !repeat {
                                log(&format!(
                                    "win+e taken (win streamed={streamed} observed={observed})"
                                ));
                                open_window();
                            }
                            // Swallow it, and every repeat while it is held, so the
                            // shell never sees the combination and a held key opens
                            // one window.
                            return 1;
                        }
                        log("win+e seen but Lyre is not the folder handler; passing it on");
                    } else {
                        log(&format!("win+e seen but not taken (other modifier={other})"));
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(0, code, wparam, lparam) }
}

/// Runs the hook for the rest of the process's life.
///
/// Runs on a thread of its own with a message pump, so the window being busy
/// cannot delay the callback past the point where the system stops asking it.
pub fn install() {
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }

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
                    log("keyboard hook installed");
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
            loop {
                let result = unsafe { GetMessageW(&mut message, 0, 0, 0) };
                if result <= 0 {
                    break;
                }
            }
        });

    if started.is_err() {
        STARTED.store(false, Ordering::Relaxed);
        log("could not start the hook thread");
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
