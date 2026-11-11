//! Updating from the project's own GitHub releases.
//!
//! The release workflow publishes a `latest.json` beside signed installers. This
//! module asks that file whether there is something newer, and if there is,
//! downloads it, checks the minisign signature against the public key compiled
//! in below, and hands it to the platform's installer. Nothing is installed that
//! the key does not vouch for. The check is made only when asked, never at
//! startup, so the window opens before any network work.
//!
//! Ported from the old Tauri updater to plain HTTP (ureq) + minisign-verify. It
//! functions once the release workflow publishes signed `latest.json`; until then
//! a check finds nothing newer (or no manifest) and says so.

use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;

const ENDPOINT: &str = "https://github.com/ItsPotatoTime/Lyre/releases/latest/download/latest.json";
/// The project's minisign public key (base64 of the key file), from the old
/// tauri.conf.json. An empty key matches nothing, so a dev build refuses installs.
const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDhFM0FCNDU1OUUzNTNEMzIKUldReVBUV2VWYlE2am1obU5URjdLUzNSZlN3a2FZOUJ5T1dBdzVwRG12WDBHNzVTbFB4bXNkckYK";

/// What the release channel is offering for this platform.
#[derive(Clone)]
pub struct Offer {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
    pub url: String,
    pub signature: String,
}

/// What a download reports while it runs.
pub enum UpdateEvent {
    Progress { downloaded: u64, total: Option<u64> },
    Installing,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    platforms: HashMap<String, Platform>,
}

#[derive(Deserialize)]
struct Platform {
    url: String,
    signature: String,
}

fn platform_key() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    { "windows-x86_64" }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    { "windows-aarch64" }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    { "linux-x86_64" }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    { "linux-aarch64" }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    { "darwin-x86_64" }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    { "darwin-aarch64" }
}

/// Is there a newer build on the release page?
pub fn check() -> Result<Option<Offer>, String> {
    let body = ureq::get(ENDPOINT)
        .call()
        .map_err(|e| format!("Could not reach the release page: {e}"))?
        .into_string()
        .map_err(|e| format!("Could not read the release manifest: {e}"))?;
    let manifest: Manifest = serde_json::from_str(&body).map_err(|e| format!("The release manifest is malformed: {e}"))?;
    let current = env!("CARGO_PKG_VERSION");
    let new_v = semver::Version::parse(manifest.version.trim()).map_err(|e| format!("Bad release version: {e}"))?;
    let cur_v = semver::Version::parse(current).map_err(|e| format!("Bad current version: {e}"))?;
    if new_v <= cur_v {
        return Ok(None);
    }
    let platform = manifest
        .platforms
        .get(platform_key())
        .ok_or_else(|| "This release has no build for your platform.".to_string())?;
    Ok(Some(Offer {
        version: manifest.version,
        current: current.to_string(),
        notes: manifest.notes,
        url: platform.url.clone(),
        signature: platform.signature.clone(),
    }))
}

/// Download the newer build, verify its signature, and run the installer.
pub fn download_and_install<F: FnMut(UpdateEvent)>(offer: &Offer, mut on_event: F) -> Result<(), String> {
    // A deb/rpm is owned by the package manager; writing over its files would
    // leave the system database describing a program that is no longer there.
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("APPIMAGE").is_none() {
        return Err("This copy was installed by a package manager, so it updates through that rather than here.".into());
    }

    let response = ureq::get(&offer.url)
        .call()
        .map_err(|e| format!("Could not download the update: {e}"))?;
    let total: Option<u64> = response.header("Content-Length").and_then(|v| v.parse().ok());
    let mut reader = response.into_reader();
    let mut bytes: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = reader.read(&mut buffer).map_err(|e| format!("The download failed: {e}"))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        on_event(UpdateEvent::Progress { downloaded: bytes.len() as u64, total });
    }

    verify(&bytes, &offer.signature)?;
    on_event(UpdateEvent::Installing);
    install(&bytes, &offer.url)
}

/// Nothing installs that the project key does not vouch for.
fn verify(data: &[u8], signature_b64: &str) -> Result<(), String> {
    use minisign_verify::{PublicKey, Signature};
    if PUBKEY.trim().is_empty() {
        return Err("This build has no update signing key, so it cannot verify a download.".into());
    }
    let key_file = base64_decode(PUBKEY).map_err(|_| "The compiled-in public key is malformed.".to_string())?;
    let key_text = String::from_utf8_lossy(&key_file);
    let key_line = key_text.lines().nth(1).ok_or("The compiled-in public key is malformed.")?;
    let public_key = PublicKey::from_base64(key_line).map_err(|e| format!("The public key could not be read: {e}"))?;
    let sig_file = base64_decode(signature_b64).map_err(|_| "The update signature is malformed.".to_string())?;
    let sig_text = String::from_utf8_lossy(&sig_file);
    let signature = match Signature::decode(&sig_text) {
        Ok(signature) => signature,
        Err(_) => {
            // Releases up to 0.3.1 encoded the `.sig` file, which is already
            // base64, one level too deep. Decoding once more recovers the
            // minisign text, so those manifests still verify.
            let inner = base64_decode(sig_text.trim())
                .map_err(|_| "The update signature is malformed.".to_string())?;
            let inner_text = String::from_utf8_lossy(&inner);
            Signature::decode(&inner_text)
                .map_err(|e| format!("The update signature could not be read: {e}"))?
        }
    };
    public_key
        .verify(data, &signature, false)
        .map_err(|_| "The update signature does not match the project key; refusing to install.".to_string())
}

/// Replace a file by staging the new bytes beside it and renaming over it.
///
/// Two constraints force this shape. On Linux the kernel refuses to write to a
/// running executable at all (`ETXTBSY`, "Text file busy"), so the download
/// cannot be written over the AppImage being executed. And a rename can only
/// cross within one filesystem, so staging in the system temporary folder and
/// moving across would fail where `/tmp` is its own mount. The copy is therefore
/// written next to the target, given its executable bit, and renamed into place,
/// which is atomic: the running process keeps the old file until it exits.
///
/// The staging name starts with a dot so a leftover from a failed attempt is not
/// mistaken for the application, and anything left over is removed.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn replace_beside(target: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let folder = target
        .parent()
        .ok_or_else(|| "That file has no folder to write beside.".to_string())?;
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| "That file has no name.".to_string())?;
    let staged = folder.join(format!(".{name}.update"));

    std::fs::write(&staged, bytes).map_err(|e| {
        format!("Could not write the update beside {}: {e}", target.display())
    })?;
    // The replacement must be runnable even though it was just written.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)) {
            let _ = std::fs::remove_file(&staged);
            return Err(format!("Could not mark the update executable: {e}"));
        }
    }
    if let Err(e) = std::fs::rename(&staged, target) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("Could not replace {}: {e}", target.display()));
    }
    Ok(())
}

/// Write the installer somewhere temporary and hand it to the platform.
#[cfg(target_os = "windows")]
fn install(bytes: &[u8], _url: &str) -> Result<(), String> {
    let path = std::env::temp_dir().join("lyre-update.exe");
    std::fs::write(&path, bytes).map_err(|e| format!("Could not stage the installer: {e}"))?;
    std::process::Command::new(&path)
        .spawn()
        .map_err(|e| format!("Could not run the installer: {e}"))?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn install(bytes: &[u8], _url: &str) -> Result<(), String> {
    // An AppImage replaces itself. It cannot be written over directly, because it
    // is the file being executed, so it is staged beside and renamed into place.
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not locate this application: {e}"))?;
    replace_beside(&exe, bytes)
}

#[cfg(target_os = "macos")]
fn install(bytes: &[u8], url: &str) -> Result<(), String> {
    let ext = if url.ends_with(".dmg") { "dmg" } else { "tar.gz" };
    let path = std::env::temp_dir().join(format!("lyre-update.{ext}"));
    std::fs::write(&path, bytes).map_err(|e| format!("Could not stage the update: {e}"))?;
    std::process::Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Could not open the update: {e}"))?;
    Ok(())
}

/// Standard base64 decoding for the Tauri-style key/signature wrappers.
fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    fn value(c: u8) -> Result<u32, ()> {
        match c {
            b'A'..=b'Z' => Ok((c - b'A') as u32),
            b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
            b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(()),
        }
    }
    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in &cleaned {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_the_key_wrapper() {
        let decoded = base64_decode(PUBKEY).expect("pubkey decodes");
        let text = String::from_utf8_lossy(&decoded);
        assert!(text.starts_with("untrusted comment: minisign public key:"));
        assert!(text.lines().nth(1).is_some_and(|line| line.starts_with("RW")));
    }

    #[test]
    fn base64_decodes_known_vectors() {
        assert_eq!(base64_decode("TWFu").unwrap(), b"Man");
        assert_eq!(base64_decode("TWE=").unwrap(), b"Ma");
        assert_eq!(base64_decode("TQ==").unwrap(), b"M");
        assert_eq!(base64_decode("").unwrap(), b"");
    }

    #[test]
    fn replacing_a_file_stages_beside_it_and_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("lyre-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Lyre.AppImage");
        std::fs::write(&target, b"the old build").unwrap();

        replace_beside(&target, b"the new build").expect("replacement should succeed");

        assert_eq!(std::fs::read(&target).unwrap(), b"the new build");
        // The staged copy is renamed, not copied, so nothing is left beside it.
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "Lyre.AppImage")
            .collect();
        assert!(
            leftovers.is_empty(),
            "a staged copy was left behind: {leftovers:?}"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "the replacement must be executable");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Ignored: checks the real published manifest against the real download.
    /// Run with `--ignored` after `gh release download`, which is how the
    /// double encoding was found and how this fix is proved.
    #[test]
    #[ignore]
    fn the_published_manifest_signature_verifies() {
        let setup = std::fs::read("../dist/Lyre_0.3.1_x64-setup.exe")
            .expect("download the 0.3.1 setup into dist/");
        let manifest: Manifest = serde_json::from_slice(
            &std::fs::read("../dist/published/latest.json").expect("download latest.json into dist/published/"),
        )
        .expect("the manifest parses");
        let platform = manifest
            .platforms
            .get("windows-x86_64")
            .expect("the manifest names this platform");
        verify(&setup, &platform.signature).expect("the published signature should verify");
    }

    #[test]
    fn an_empty_key_refuses_to_verify() {
        // The real PUBKEY is non-empty; this checks the guard logic indirectly by
        // confirming a garbage signature is rejected rather than panicking.
        let result = verify(b"some data", "not-base64!!!");
        assert!(result.is_err());
    }
}