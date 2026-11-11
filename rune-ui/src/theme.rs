//! Which appearance Lyre uses, and following the desktop's own choice.
//!
//! The preference is one of three: follow the desktop, or force light or dark.
//! Following it means asking the platform, which each does in its own place:
//! Windows keeps it in the registry, macOS in a defaults domain, and the
//! freedesktop desktops in `gsettings`.

/// What the person chose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    /// Whatever the desktop is using.
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn to_u8(self) -> u8 {
        match self {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        }
    }

    pub fn from_u8(value: u8) -> Theme {
        match value {
            1 => Theme::Light,
            2 => Theme::Dark,
            _ => Theme::System,
        }
    }

    /// The appearance to draw now, resolving `System` against the desktop.
    pub fn is_dark(self) -> bool {
        match self {
            Theme::Light => false,
            Theme::Dark => true,
            Theme::System => system_is_dark(),
        }
    }
}

/// Whether the desktop is in its dark appearance right now.
///
/// Two of the three read a process or a key, so this is asked when the window
/// starts and when the appearance menu is opened rather than every frame.
pub fn system_is_dark() -> bool {
    platform::is_dark()
}

#[cfg(target_os = "windows")]
mod platform {
    /// `AppsUseLightTheme` is 0 in the dark appearance. A machine that has never
    /// recorded a choice reads as light, which is also the system default.
    pub fn is_dark() -> bool {
        matches!(
            crate::shell::reg_dword(
                r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                Some("AppsUseLightTheme"),
            ),
            Some(0)
        )
    }
}

#[cfg(target_os = "macos")]
mod platform {
    pub fn is_dark() -> bool {
        // The key exists only in the dark appearance, which is why `defaults`
        // exits non-zero in light.
        let read = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output();
        match read {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .trim()
                .eq_ignore_ascii_case("Dark"),
            _ => false,
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    pub fn is_dark() -> bool {
        // GNOME and the desktops that follow it record the preference as a
        // colour scheme. KDE has its own setting and simply reports light here.
        let read = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output();
        if let Ok(out) = read
            && out.status.success()
        {
            return scheme_is_dark(&String::from_utf8_lossy(&out.stdout));
        }
        // A GTK theme that names itself dark does too.
        std::env::var("GTK_THEME")
            .map(|theme| theme.to_lowercase().contains("dark"))
            .unwrap_or(false)
    }

    /// What `gsettings` prints for the colour scheme: `'prefer-dark'`,
    /// `'default'`, or a custom scheme's name.
    fn scheme_is_dark(scheme: &str) -> bool {
        scheme.to_lowercase().contains("dark")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_colour_scheme_says_which_appearance() {
            assert!(scheme_is_dark("'prefer-dark'\n"));
            assert!(!scheme_is_dark("'default'\n"));
            assert!(!scheme_is_dark("'prefer-light'\n"));
            assert!(scheme_is_dark("'org.example.Dark'\n"));
        }
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
mod platform {
    pub fn is_dark() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preference_round_trips() {
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::from_u8(theme.to_u8()), theme);
        }
        // An older save that stored nothing at all still reads as following.
        assert_eq!(Theme::from_u8(0), Theme::System);
        assert_eq!(Theme::from_u8(200), Theme::System);
    }

    #[test]
    fn a_forced_choice_does_not_ask_the_desktop() {
        // The answer cannot depend on the machine for these two.
        assert!(!Theme::Light.is_dark());
        assert!(Theme::Dark.is_dark());
    }

    /// Reads the real desktop preference. Ignored because the answer belongs to
    /// the machine; the forced choices above are the portable part.
    #[test]
    #[cfg(target_os = "windows")]
    #[ignore]
    fn the_desktop_preference_is_readable() {
        let light = crate::shell::reg_dword(
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            Some("AppsUseLightTheme"),
        );
        println!(
            "AppsUseLightTheme = {light:?}, so dark = {}",
            system_is_dark()
        );
        assert!(light.is_some(), "the appearance key should be readable");
        assert_eq!(system_is_dark(), light == Some(0));
    }
}
