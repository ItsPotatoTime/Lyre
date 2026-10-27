//! The one graphics knob worth exposing.
//!
//! WebKit on Linux shares GPU buffers with the compositor through dmabuf. That
//! is the fast path for drawing, and it also costs memory: measured on this
//! project, turning it off takes about 46 MB off an idle window, 15 percent.
//! The trade is an extra copy per frame, which is cheap at 60 Hz and not free
//! at 300 Hz, so it is a choice rather than a default.
//!
//! It has to be read before the webview is created, because WebKit reads the
//! variable once during initialisation. That is why it lives in a small file
//! read at startup instead of a preference the window could change live.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsSettings {
    /// True when the GPU buffer path is disabled to save memory.
    pub reduced_memory: bool,
    /// Whether the current process actually started with it disabled.
    pub active: bool,
    /// Whether changing it needs a restart to take effect.
    pub needs_restart: bool,
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("lyre").join("graphics.conf"))
}

fn stored() -> bool {
    let Some(path) = config_path() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.lines()
        .find_map(|line| line.strip_prefix("reduced-memory="))
        .map(|value| value.trim() == "on")
        .unwrap_or(false)
}

/// Applied before the app starts. An explicit environment variable wins, so
/// anyone who set it themselves keeps control.
pub fn apply_at_startup() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some() {
        return;
    }
    if stored() {
        // SAFETY: single-threaded startup, before any webview exists.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
}

pub fn state() -> GraphicsSettings {
    let active = std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some();
    let reduced = stored();
    GraphicsSettings {
        reduced_memory: reduced,
        active,
        needs_restart: reduced != active,
    }
}

pub fn set(reduced: bool) -> Result<GraphicsSettings, String> {
    let path = config_path().ok_or_else(|| "Could not find a config directory.".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Could not create {}: {err}", parent.display()))?;
    }
    std::fs::write(&path, format!("reduced-memory={}\n", if reduced { "on" } else { "off" }))
        .map_err(|err| format!("Could not write {}: {err}", path.display()))?;
    Ok(state())
}
