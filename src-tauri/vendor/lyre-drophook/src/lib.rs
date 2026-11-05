//! A process-global, *synchronous* bridge from wry's Windows drop target to
//! Lyre's Rust core.
//!
//! wry reads `CF_HDROP` inside `IDropTarget::Drop`, then only posts a message to
//! the Tauri event loop. An archiver such as 7-Zip extracts its temporary files
//! during that `GetData` call and deletes them as soon as `DoDragDrop` returns,
//! so anything waiting for the queued event is already too late. The patched
//! wry calls `invoke` while `IDropTarget::Drop` is still on the stack, which is
//! the only moment the files are guaranteed to exist.
//!
//! This crate has no dependencies of its own: both wry and Lyre depend on this
//! one copy, so the `OnceLock` they share is the same allocation. Keeping the
//! callback a plain `fn` (rather than a boxed closure) means no allocation and
//! no lifetime to reason about on a COM callback thread.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Where in an OLE drag the hook was called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
  /// `IDropTarget::DragEnter`. Some sources have not created their files yet.
  Enter,
  /// `IDropTarget::Drop`. The files exist and the source still holds them.
  Drop,
  /// `IDropTarget::DragLeave`, or a cancelled drag.
  Leave,
}

/// A plain function pointer: wry calls this without allocating.
pub type Hook = fn(Phase, &[PathBuf]);

static HOOK: OnceLock<Hook> = OnceLock::new();

/// Install the hook. Returns `false` if something already installed one, which
/// is not an error: the first registration wins for the life of the process.
pub fn set(hook: Hook) -> bool {
  HOOK.set(hook).is_ok()
}

/// Called by wry on the drop target's own thread. Must stay cheap and must not
/// panic: it runs inside a COM `extern "system"` boundary.
pub fn invoke(phase: Phase, paths: &[PathBuf]) {
  if let Some(hook) = HOOK.get() {
    hook(phase, paths);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn record(_phase: Phase, _paths: &[PathBuf]) {}

  #[test]
  fn a_hook_is_installed_once() {
    // The real process installs once at startup. Anything after that must be
    // ignored rather than replacing the live callback.
    assert!(set(record));
    assert!(!set(record));
    invoke(Phase::Leave, &[]);
  }
}
