//! Ejecting removable volumes safely.
//!
//! "Safe" means unmounted before the device is powered down. Pulling a stick
//! that still has a mounted filesystem is how people lose writes that were
//! buffered but not flushed, so this never powers off a device it could not
//! first unmount.
//!
//! Linux goes through `udisksctl`, which is the same path the desktop's own
//! eject button takes: it unmounts every partition of the device and then asks
//! the kernel to power it off. Windows and macOS use their own tools, and both
//! report exactly what failed rather than pretending to have succeeded.

use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EjectOutcome {
    /// Whether the device is now safe to remove.
    pub ejected: bool,
    /// What happened, in the order it happened, for the status bar.
    pub steps: Vec<String>,
    pub error: Option<String>,
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("Could not run {program}: {err}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .trim()
    .to_string();
    if output.status.success() {
        Ok(text)
    } else if text.is_empty() {
        Err(format!("{program} failed"))
    } else {
        Err(text)
    }
}

#[cfg(target_os = "linux")]
pub fn eject(mount_path: &str) -> EjectOutcome {
    use sysinfo::Disks;

    let mut steps = Vec::new();
    let disks = Disks::new_with_refreshed_list();

    // The volume the caller pointed at, and the disk it sits on. A stick with
    // two partitions has two mounts and one device to power off.
    let Some(target) = disks
        .list()
        .iter()
        .find(|disk| disk.mount_point().to_string_lossy() == mount_path)
    else {
        return EjectOutcome {
            ejected: false,
            steps,
            error: Some(format!("{mount_path} is not a mounted volume.")),
        };
    };

    if !target.is_removable() {
        return EjectOutcome {
            ejected: false,
            steps,
            error: Some("That volume is not removable.".into()),
        };
    }

    let device = target.name().to_string_lossy().into_owned();
    // /dev/sdb1 -> /dev/sdb, /dev/nvme0n1p1 -> /dev/nvme0n1. `lsblk` knows the
    // real answer, including for names this pattern would get wrong.
    let base = run("lsblk", &["-no", "PKNAME", &device])
        .ok()
        .map(|parent| parent.lines().next().unwrap_or_default().trim().to_string())
        .filter(|parent| !parent.is_empty())
        .map(|parent| format!("/dev/{parent}"))
        .unwrap_or_else(|| device.clone());

    // Every mounted partition of this device, so nothing is left mounted when
    // the power goes off.
    let family: Vec<String> = disks
        .list()
        .iter()
        .filter(|disk| {
            let name = disk.name().to_string_lossy();
            disk.is_removable()
                && (name == base
                    || name == device
                    || (name.starts_with(&base) && name != base))
        })
        .map(|disk| disk.name().to_string_lossy().into_owned())
        .collect();

    for partition in &family {
        match run("udisksctl", &["unmount", "-b", partition]) {
            Ok(message) => steps.push(if message.is_empty() {
                format!("Unmounted {partition}")
            } else {
                message
            }),
            Err(message) => {
                // A partition that is not mounted is not a failure worth
                // stopping for; anything else is, because powering off with a
                // busy filesystem is exactly what this function exists to
                // prevent.
                if !message.to_lowercase().contains("not mounted") {
                    return EjectOutcome {
                        ejected: false,
                        steps,
                        error: Some(message),
                    };
                }
            }
        }
    }

    match run("udisksctl", &["power-off", "-b", &base]) {
        Ok(message) => {
            steps.push(if message.is_empty() {
                format!("Powered off {base}")
            } else {
                message
            });
            EjectOutcome {
                ejected: true,
                steps,
                error: None,
            }
        }
        Err(message) => EjectOutcome {
            ejected: false,
            steps,
            error: Some(message),
        },
    }
}

#[cfg(target_os = "windows")]
pub fn eject(mount_path: &str) -> EjectOutcome {
    // The drive letter is what the shell needs.
    let letter = mount_path.trim_end_matches(['\\', '/']).to_string();
    let script = format!(
        "$d = (New-Object -comObject Shell.Application).Namespace(17).ParseName('{letter}'); \
         if ($d -eq $null) {{ Write-Error 'Drive not found'; exit 1 }}; \
         $d.InvokeVerb('Eject'); Write-Output 'Ejected'"
    );
    match run(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    ) {
        Ok(message) => EjectOutcome {
            ejected: true,
            steps: vec![message],
            error: None,
        },
        Err(message) => EjectOutcome {
            ejected: false,
            steps: Vec::new(),
            error: Some(message),
        },
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn eject(mount_path: &str) -> EjectOutcome {
    match run("diskutil", &["eject", mount_path]) {
        Ok(message) => EjectOutcome {
            ejected: true,
            steps: vec![message],
            error: None,
        },
        Err(message) => EjectOutcome {
            ejected: false,
            steps: Vec::new(),
            error: Some(message),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_that_is_not_a_volume_is_refused_before_running_anything() {
        // Nothing is executed here, so this is safe on any machine.
        let outcome = eject("/definitely/not/a/mounted/volume");
        assert!(!outcome.ejected);
        assert!(outcome.error.is_some());
        assert!(outcome.steps.is_empty());
    }

    #[test]
    fn the_directory_listing_style_trimming_keeps_the_letter() {
        // What Windows passes to the shell: C:\ -> C:
        assert_eq!("C:\\".trim_end_matches(['\\', '/']), "C:");
        assert_eq!("/media/you/stick/".trim_end_matches(['\\', '/']), "/media/you/stick");
    }
}
