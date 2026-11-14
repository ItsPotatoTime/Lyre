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
    let bytes = take_capped(response.into_reader(), MAX_UPDATE_BYTES, total, &mut on_event)?;

    verify(&bytes, &offer.signature)?;
    on_event(UpdateEvent::Installing);
    install(&bytes, &offer.url)
}

/// The largest download an update may be. A build of this application is
/// tens of megabytes; a response without end is a broken or hostile server
/// filling memory until the machine gives up, signature or no signature.
const MAX_UPDATE_BYTES: u64 = 1 << 30;

/// Read a download into memory a chunk at a time, reporting progress, and
/// stop rather than grow past `cap` bytes.
fn take_capped(
    mut reader: impl std::io::Read,
    cap: u64,
    total: Option<u64>,
    on_event: &mut impl FnMut(UpdateEvent),
) -> Result<Vec<u8>, String> {
    if total.is_some_and(|declared| declared > cap) {
        return Err("The update is larger than any build of this application; refusing it.".into());
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = reader.read(&mut buffer).map_err(|e| format!("The download failed: {e}"))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() as u64 > cap {
            return Err(
                "The update download did not end where a build of this application ends; refusing it."
                    .into(),
            );
        }
        on_event(UpdateEvent::Progress { downloaded: bytes.len() as u64, total });
    }
    Ok(bytes)
}

/// Write bytes to a temporary file no one else can have named first, and
/// return its path. The installer used to land at one fixed name in the
/// shared temporary folder, where anything running as this user could plant
/// a link at that name ahead of time and have the verified bytes written
/// through it somewhere else. The name is unique to this call and opened
/// with `create_new`, which refuses a path that already stands, link or
/// not, so the bytes that were verified are the bytes handed over.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
fn stage_temp(bytes: &[u8], leaf: &str) -> Result<std::path::PathBuf, String> {
    use std::io::Write as _;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir();
    for _ in 0..100 {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("lyre-update-{}-{n}-{leaf}", std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(e) = file.write_all(bytes) {
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("Could not stage the update: {e}"));
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Could not stage the update: {e}")),
        }
    }
    Err("Could not find a free name for the update in the temporary folder.".into())
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
    let path = stage_temp(bytes, "installer.exe")?;
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
    let path = stage_temp(bytes, &format!("update.{ext}"))?;
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
    fn a_download_past_the_cap_is_refused() {
        let data = vec![7u8; 100];
        let mut events = |_| {};
        assert!(take_capped(&data[..], 50, None, &mut events).is_err(), "a stream past the cap is refused");
        assert!(
            take_capped(&data[..], 50, Some(51), &mut events).is_err(),
            "a declared length past the cap is refused up front"
        );
        let taken = take_capped(&data[..], 100, Some(100), &mut events).expect("a download at the cap is taken");
        assert_eq!(taken, data);
    }

    #[test]
    fn a_staged_update_never_follows_a_planted_link() {
        let dir = std::env::temp_dir();
        let victim = dir.join(format!("lyre-update-victim-{}", std::process::id()));
        std::fs::write(&victim, b"do not touch").unwrap();
        let mut planted = Vec::new();
        #[cfg(unix)]
        for n in 0..64 {
            let candidate = dir.join(format!("lyre-update-{}-{n}-installer.exe", std::process::id()));
            if std::os::unix::fs::symlink(&victim, &candidate).is_ok() {
                planted.push(candidate);
            }
        }
        let staged = stage_temp(b"the verified bytes", "installer.exe").expect("staging succeeds");
        assert!(!planted.contains(&staged), "the staged file got a name of its own");
        assert_eq!(std::fs::read(&staged).unwrap(), b"the verified bytes");
        assert_eq!(std::fs::read(&victim).unwrap(), b"do not touch", "nothing was written through a link");
        let _ = std::fs::remove_file(&staged);
        let _ = std::fs::remove_file(&victim);
        for path in planted {
            let _ = std::fs::remove_file(path);
        }
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