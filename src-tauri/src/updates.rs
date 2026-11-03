//! Updating from the project's own releases.
//!
//! The release workflow signs every installer and publishes a `latest.json`
//! beside it. This module asks that file whether there is something newer, and
//! if there is, downloads it, checks the signature against the public key in
//! `tauri.conf.json`, and hands it to the installer. Nothing is installed that
//! the key does not vouch for, which is the whole reason the signing step is not
//! optional.
//!
//! Two things are worth knowing. A build whose configured key is empty can check
//! and download, and will refuse at the signature step: that is a development
//! build, not a broken one. And the check is made only when asked for, never at
//! startup, because a file manager that reaches the network before showing a
//! folder is a file manager that feels slower than it is.

use serde::Serialize;
use tauri::ipc::Channel;

/// What the release channel is offering.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub version: String,
    pub current: String,
    /// The release notes the workflow wrote into `latest.json`.
    pub notes: Option<String>,
}

/// What a download reports while it runs.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UpdateEvent {
    /// Bytes of the new build as they arrive, and the size if the server said.
    Progress { downloaded: u64, total: Option<u64> },
    /// The bytes are here and the installer is running.
    Installing,
}

/// Is there a newer build on the release page?
pub async fn check(app: &tauri::AppHandle) -> Result<Option<Offer>, String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|err| err.to_string())?;
    let found = updater.check().await.map_err(|err| explain(&err))?;
    Ok(found.map(|update| Offer {
        version: update.version.clone(),
        current: update.current_version.clone(),
        notes: update.body.clone(),
    }))
}

/// Download and install the newer build.
pub async fn install(app: &tauri::AppHandle, on_event: Channel<UpdateEvent>) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    // An AppImage replaces itself; a deb or rpm is owned by the package manager,
    // and writing over its files would leave the system's database describing a
    // program that is no longer there.
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("APPIMAGE").is_none() {
        return Err(
            "This copy was installed by a package manager, so it updates through that \
             rather than here."
                .into(),
        );
    }

    let updater = app.updater().map_err(|err| err.to_string())?;
    let Some(update) = updater.check().await.map_err(|err| explain(&err))? else {
        return Err("There is no newer version to install.".into());
    };

    let progress = on_event.clone();
    let mut downloaded = 0u64;
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress.send(UpdateEvent::Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|err| explain(&err))?;

    let _ = on_event.send(UpdateEvent::Installing);
    Ok(())
}

/// The plugin's messages are accurate and unfriendly. The one worth translating
/// is the signature failure, because it is what a build with no key configured
/// reports, and it looks like a corrupt download otherwise.
fn explain(error: &tauri_plugin_updater::Error) -> String {
    match error {
        tauri_plugin_updater::Error::Minisign(_) => MISSING_KEY.to_string(),
        other => other.to_string(),
    }
}

const MISSING_KEY: &str = "This build has no update signing key, so it cannot verify a \
                           download. See docs/updates.md.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_missing_key_message_says_what_to_do() {
        assert!(MISSING_KEY.contains("signing key"), "{MISSING_KEY}");
        assert!(MISSING_KEY.contains("docs/updates.md"), "{MISSING_KEY}");
    }
}
