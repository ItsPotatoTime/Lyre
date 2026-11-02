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
//! Two constraints shape the code:
//!
//! - The hook only exists while a process that installed it is alive, so Win+E
//!   falls back to Explorer when this application is closed.
//! - Every running instance would otherwise install its own hook and open one
//!   window each, so the first instance takes a named mutex and the rest stay
//!   out. Without that, each press would multiply the number of windows.
//!
//! Whether the key is ours to take is read at the moment it is pressed rather
//! than cached, so restoring Explorer from any window stops the redirection
//! immediately.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use crate::defaultmanager;

const WH_KEYBOARD_LL: i32 = 13;
const WM_KEYDOWN: usize = 0x0100;
const WM_SYSKEYDOWN: usize = 0x0104;
const VK_E: u32 = 0x45;
const VK_LWIN: i32 = 0x5B;
const VK_RWIN: i32 = 0x5C;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const ERROR_ALREADY_EXISTS: u32 = 183;

/// `KBDLLHOOKSTRUCT`: what the keyboard hook is handed for each key event.
#[repr(C)]
struct KbdLlHookStruct {
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
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
/// Held for the life of the process that owns the key.
static OWNER: AtomicUsize = AtomicUsize::new(0);
/// When the last window was opened, in milliseconds since start.
static LAST_OPEN: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();

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
    // The high bit is set while the key is held, which is the documented idiom
    // and the only part of the result that is meaningful here.
    unsafe { GetAsyncKeyState(key) as u16 & 0x8000 != 0 }
}

/// Opens a window of this application, at most once per press.
///
/// The key repeats while it is held, so a repeat has to be swallowed without
/// opening anything. The spawn itself happens off the hook thread: starting a
/// process takes long enough to be felt as input lag otherwise.
fn open_window() {
    let now = START.get_or_init(Instant::now).elapsed().as_millis() as u64;
    let last = LAST_OPEN.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < 800 {
        return;
    }
    LAST_OPEN.store(now, Ordering::Relaxed);

    std::thread::spawn(|| {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
    });
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code >= 0 && (wparam == WM_KEYDOWN || wparam == WM_SYSKEYDOWN) {
        let event = unsafe { &*(lparam as *const KbdLlHookStruct) };
        let win_down = key_down(VK_LWIN) || key_down(VK_RWIN);
        let other_modifier = key_down(VK_SHIFT) || key_down(VK_CONTROL) || key_down(VK_MENU);
        if takes_the_key(event.vk_code, win_down, other_modifier)
            && defaultmanager::is_current_handler()
        {
            open_window();
            // Swallow it, so the shell never sees the combination at all.
            return 1;
        }
    }
    unsafe { CallNextHookEx(0, code, wparam, lparam) }
}

/// Claims the key for this process, once.
///
/// Must be called on the thread that runs the window message loop, because a
/// low level hook is delivered through that thread's queue.
pub fn install() {
    if HOOK.load(Ordering::Relaxed) != 0 {
        return;
    }

    // The first process owns the key. A later one sees the name taken and leaves
    // the hook alone, so pressing the combination opens one window and not as
    // many as there are running copies.
    let name = wide(r"Local\Lyre.WinE");
    let owner = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if owner == 0 {
        return;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(owner) };
        return;
    }
    OWNER.store(owner, Ordering::Relaxed);

    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_proc),
            GetModuleHandleW(std::ptr::null()),
            0,
        )
    };
    if hook == 0 {
        // Nothing to intercept with, so let another run try rather than holding
        // the name with no hook behind it.
        unsafe { CloseHandle(owner) };
        OWNER.store(0, Ordering::Relaxed);
        return;
    }
    HOOK.store(hook, Ordering::Relaxed);
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
