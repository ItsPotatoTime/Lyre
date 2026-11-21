//! Which appearance Lyre uses, and following the desktop's own choice.
//!
//! The preference is one of three: follow the desktop, or force light or dark.
//! Following it means asking the platform, which each does in its own place:
//! Windows keeps it in the registry, macOS in a defaults domain, and the
//! freedesktop desktops in `gsettings`.

use rune::Color;
use rune::color::srgb_encode;

/// Default custom accent color (`#4cc2ff`).
pub const DEFAULT_ACCENT_HEX: &str = "#4cc2ff";
/// Allowed range for surface tint strength (`0.0..=0.25`).
pub const SURFACE_TINT_MIN: f32 = 0.0;
pub const SURFACE_TINT_MAX: f32 = 0.25;
/// Allowed range for floating panel opacity (`0.50..=1.0`).
pub const PANEL_OPACITY_MIN: f32 = 0.50;
pub const PANEL_OPACITY_MAX: f32 = 1.0;
/// Allowed range for backdrop blur radius (`0.0..=24.0`).
pub const BLUR_RADIUS_MIN: f32 = 0.0;
pub const BLUR_RADIUS_MAX: f32 = 24.0;

/// What the person chose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    /// Whatever the desktop is using.
    System,
    Light,
    Dark,
    /// An accent and a light/dark base the person picked themselves. The
    /// resolved base lives in `CustomTheme::dark`; `is_dark` cannot answer for
    /// this variant on its own.
    Custom,
}

impl Theme {
    pub fn to_u8(self) -> u8 {
        match self {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
            Theme::Custom => 3,
        }
    }

    pub fn from_u8(value: u8) -> Theme {
        match value {
            1 => Theme::Light,
            2 => Theme::Dark,
            3 => Theme::Custom,
            _ => Theme::System,
        }
    }

    /// The appearance to draw now, resolving `System` against the desktop.
    pub fn is_dark(self) -> bool {
        match self {
            Theme::Light => false,
            Theme::Dark => true,
            Theme::System => system_is_dark(),
            // Resolved by App from custom_dark; never call this for Custom.
            Theme::Custom => false,
        }
    }
}

/// Native OS window backdrop material.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BackdropStyle {
    #[default]
    Opaque,
    Mica,
    Acrylic,
}

impl BackdropStyle {
    pub const ALL: [BackdropStyle; 3] = [
        BackdropStyle::Opaque,
        BackdropStyle::Mica,
        BackdropStyle::Acrylic,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BackdropStyle::Opaque => "opaque",
            BackdropStyle::Mica => "mica",
            BackdropStyle::Acrylic => "acrylic",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "mica" => BackdropStyle::Mica,
            "acrylic" => BackdropStyle::Acrylic,
            _ => BackdropStyle::Opaque,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BackdropStyle::Opaque => "Opaque",
            BackdropStyle::Mica => "Mica",
            BackdropStyle::Acrylic => "Acrylic",
        }
    }
}

/// Corner rounding profile for panels, dialogs, cards, and inputs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CornerStyle {
    Sharp,
    #[default]
    Standard,
    Rounded,
}

impl CornerStyle {
    pub const ALL: [CornerStyle; 3] = [
        CornerStyle::Sharp,
        CornerStyle::Standard,
        CornerStyle::Rounded,
    ];

    pub fn radius(&self) -> f32 {
        match self {
            CornerStyle::Sharp => 4.0,
            CornerStyle::Standard => 8.0,
            CornerStyle::Rounded => 12.0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CornerStyle::Sharp => "sharp",
            CornerStyle::Standard => "standard",
            CornerStyle::Rounded => "rounded",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "sharp" => CornerStyle::Sharp,
            "rounded" => CornerStyle::Rounded,
            _ => CornerStyle::Standard,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CornerStyle::Sharp => "Sharp",
            CornerStyle::Standard => "Standard",
            CornerStyle::Rounded => "Rounded",
        }
    }
}

/// User-customizable appearance settings for `Theme::Custom` and panel effects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CustomTheme {
    pub accent: Color,
    pub dark: bool,
    pub surface_tint: f32,
    pub panel_opacity: f32,
    pub blur_radius: f32,
    pub backdrop: BackdropStyle,
    pub corners: CornerStyle,
    pub fx_enabled: bool,
    pub fx_frosted: bool,
    pub fx_shadows: bool,
    pub fx_borders: bool,
}

impl Default for CustomTheme {
    fn default() -> Self {
        Self {
            accent: hex_or_default(DEFAULT_ACCENT_HEX),
            dark: true,
            surface_tint: 0.0,
            panel_opacity: 0.80,
            blur_radius: 12.0,
            backdrop: BackdropStyle::Opaque,
            corners: CornerStyle::Standard,
            fx_enabled: true,
            fx_frosted: true,
            fx_shadows: true,
            fx_borders: true,
        }
    }
}

impl CustomTheme {
    /// Return a copy with numeric sliders clamped to safe bounds and non-finite
    /// values replaced by defaults.
    pub fn clamped(self) -> Self {
        let clamp_f = |v: f32, min: f32, max: f32, fallback: f32| {
            if v.is_finite() { v.clamp(min, max) } else { fallback }
        };
        let clamp_c = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        Self {
            accent: Color {
                r: clamp_c(self.accent.r),
                g: clamp_c(self.accent.g),
                b: clamp_c(self.accent.b),
                a: 1.0,
            },
            surface_tint: clamp_f(self.surface_tint, SURFACE_TINT_MIN, SURFACE_TINT_MAX, 0.0),
            panel_opacity: clamp_f(self.panel_opacity, PANEL_OPACITY_MIN, PANEL_OPACITY_MAX, 0.80),
            blur_radius: clamp_f(self.blur_radius, BLUR_RADIUS_MIN, BLUR_RADIUS_MAX, 12.0),
            ..self
        }
    }

    /// Restore every custom theme field to its default value.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Format the accent color as `#rrggbb` in sRGB.
    pub fn accent_hex(self) -> String {
        color_to_hex(self.clamped().accent)
    }
}

/// A curated starter theme configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemePreset {
    pub id: &'static str,
    pub label: &'static str,
    pub accent_hex: &'static str,
    pub dark: bool,
    pub surface_tint: f32,
    pub panel_opacity: f32,
    pub blur_radius: f32,
    pub backdrop: BackdropStyle,
    pub corners: CornerStyle,
}

impl ThemePreset {
    pub const ALL: &'static [ThemePreset] = &[
        ThemePreset {
            id: "fluent-dark",
            label: "Fluent Dark",
            accent_hex: "#4cc2ff",
            dark: true,
            surface_tint: 0.0,
            panel_opacity: 0.80,
            blur_radius: 12.0,
            backdrop: BackdropStyle::Opaque,
            corners: CornerStyle::Standard,
        },
        ThemePreset {
            id: "fluent-light",
            label: "Fluent Light",
            accent_hex: "#0067c0",
            dark: false,
            surface_tint: 0.0,
            panel_opacity: 0.84,
            blur_radius: 12.0,
            backdrop: BackdropStyle::Opaque,
            corners: CornerStyle::Standard,
        },
        ThemePreset {
            id: "frosted-glass",
            label: "Frosted Glass",
            accent_hex: "#60cdff",
            dark: true,
            surface_tint: 0.08,
            panel_opacity: 0.68,
            blur_radius: 20.0,
            backdrop: BackdropStyle::Acrylic,
            corners: CornerStyle::Rounded,
        },
        ThemePreset {
            id: "midnight",
            label: "Midnight",
            accent_hex: "#7b68ee",
            dark: true,
            surface_tint: 0.14,
            panel_opacity: 0.78,
            blur_radius: 16.0,
            backdrop: BackdropStyle::Mica,
            corners: CornerStyle::Standard,
        },
        ThemePreset {
            id: "nord",
            label: "Nord",
            accent_hex: "#88c0d0",
            dark: true,
            surface_tint: 0.12,
            panel_opacity: 0.86,
            blur_radius: 10.0,
            backdrop: BackdropStyle::Opaque,
            corners: CornerStyle::Standard,
        },
        ThemePreset {
            id: "rose",
            label: "Rose",
            accent_hex: "#e85d8a",
            dark: true,
            surface_tint: 0.10,
            panel_opacity: 0.80,
            blur_radius: 14.0,
            backdrop: BackdropStyle::Mica,
            corners: CornerStyle::Rounded,
        },
        ThemePreset {
            id: "forest",
            label: "Forest",
            accent_hex: "#3fb950",
            dark: true,
            surface_tint: 0.10,
            panel_opacity: 0.82,
            blur_radius: 12.0,
            backdrop: BackdropStyle::Opaque,
            corners: CornerStyle::Standard,
        },
        ThemePreset {
            id: "high-contrast",
            label: "High Contrast",
            accent_hex: "#ffd700",
            dark: true,
            surface_tint: 0.0,
            panel_opacity: 1.0,
            blur_radius: 0.0,
            backdrop: BackdropStyle::Opaque,
            corners: CornerStyle::Sharp,
        },
    ];

    /// Materialize this preset into a clamped `CustomTheme`.
    pub fn to_custom_theme(self) -> CustomTheme {
        CustomTheme {
            accent: hex_or_default(self.accent_hex),
            dark: self.dark,
            surface_tint: self.surface_tint,
            panel_opacity: self.panel_opacity,
            blur_radius: self.blur_radius,
            backdrop: self.backdrop,
            corners: self.corners,
            fx_enabled: true,
            fx_frosted: self.blur_radius > 0.0,
            fx_shadows: true,
            fx_borders: true,
        }
        .clamped()
    }
}

/// Derived accent and tinted surface tokens computed from a [`CustomTheme`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CustomPalette {
    pub accent: Color,
    pub accent_fill: Color,
    pub accent_ink: Color,
    pub title: Color,
    pub toolbar: Color,
    pub tab_active: Color,
    pub menu: Color,
    pub content: Color,
    pub rail_sel: Color,
    pub row_sel: Color,
    pub sel_line: Color,
}

fn hex_or_default(hex: &str) -> Color {
    rune::color::hex(hex).unwrap_or(Color {
        r: 0.072,
        g: 0.539,
        b: 1.0,
        a: 1.0,
    })
}

/// Format a [`Color`] as `#rrggbb` in sRGB.
pub fn color_to_hex(color: Color) -> String {
    let enc = |c: f32| (srgb_encode(c.clamp(0.0, 1.0)) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", enc(color.r), enc(color.g), enc(color.b))
}

/// Pick a high-contrast ink color (`#111111` or white) for text and icons on `bg`.
///
/// Uses relative luminance (`Color::luminance()`) so bright fills such as
/// `#4cc2ff`, `#ffd700`, and `#ffffff` receive dark ink while darker accents
/// receive white ink.
pub fn contrast_ink(bg: Color) -> Color {
    // Luminance 0.19 sits right above the equal-contrast point (~0.179) so
    // classic Fluent Blue (#0078d4, L ~ 0.182) keeps crisp white text at >4.5:1
    // while bright accents (#4cc2ff, #ffd700, #ffffff) switch to dark ink.
    if bg.luminance() > 0.19 {
        rune::color::hex("#111111").unwrap_or(Color::BLACK)
    } else {
        Color::WHITE
    }
}

fn lift_luminance(color: Color, min_luminance: f32) -> Color {
    let l = color.luminance();
    if l >= min_luminance || l >= 1.0 {
        return color;
    }
    let t = ((min_luminance - l) / (1.0 - l)).clamp(0.0, 1.0);
    color.mix(Color::WHITE, t).alpha(color.a)
}

fn cap_luminance(color: Color, max_luminance: f32) -> Color {
    let l = color.luminance();
    if l <= max_luminance || l <= 0.0 {
        return color;
    }
    let t = (1.0 - max_luminance / l).clamp(0.0, 1.0);
    color.mix(Color::BLACK, t).alpha(color.a)
}

/// Derive contrast-safe accent tokens and tinted surfaces from `ct`.
pub fn derive_custom_colors(base_dark: bool, ct: &CustomTheme) -> CustomPalette {
    let ct = ct.clamped();
    let raw_accent = ct.accent.alpha(1.0);

    // Ensure accent text/icons remain legible on the base content background,
    // even for extreme user inputs like #000000 on dark or #ffffff on light.
    let accent = if base_dark {
        lift_luminance(raw_accent, 0.14)
    } else {
        cap_luminance(raw_accent, 0.30)
    };

    // Keep filled buttons distinct from the base surfaces and readable with contrast_ink.
    let raw_fill = if base_dark {
        raw_accent.mix(Color::BLACK, 0.18)
    } else {
        raw_accent.mix(Color::WHITE, 0.03)
    };
    let accent_fill = if base_dark {
        lift_luminance(raw_fill, 0.09)
    } else {
        cap_luminance(raw_fill, 0.55)
    };
    let accent_ink = contrast_ink(accent_fill);

    let (base_title, base_toolbar, base_tab, base_menu, base_content) = if base_dark {
        (
            hex_or_default("#000d17"),
            hex_or_default("#1a2127"),
            hex_or_default("#202429"),
            hex_or_default("#2b2b2b"),
            hex_or_default("#191919"),
        )
    } else {
        (
            hex_or_default("#e9edf2"),
            hex_or_default("#f4f6f8"),
            hex_or_default("#ffffff"),
            hex_or_default("#ffffff"),
            hex_or_default("#ffffff"),
        )
    };

    let tint = ct.surface_tint;
    let title = base_title.mix(raw_accent, tint);
    let toolbar = base_toolbar.mix(raw_accent, tint * 0.70);
    let tab_active = base_tab.mix(raw_accent, tint * 0.60);
    let menu = base_menu.mix(raw_accent, tint * 0.50);
    let content = base_content.mix(raw_accent, tint * 0.35);

    let rail_sel = accent.mix(content, if base_dark { 0.88 } else { 0.92 });
    let row_sel = accent.mix(content, if base_dark { 0.86 } else { 0.90 });
    let sel_line = accent.mix(content, if base_dark { 0.62 } else { 0.68 });

    CustomPalette {
        accent,
        accent_fill,
        accent_ink,
        title,
        toolbar,
        tab_active,
        menu,
        content,
        rail_sel,
        row_sel,
        sel_line,
    }
}

/// Compute floating panel fill color from `base_menu` and `ct`, decoupling
/// panel opacity from frosted blur.
pub fn panel_fill(base_menu: Color, ct: &CustomTheme) -> Color {
    let ct = ct.clamped();
    if ct.fx_enabled {
        base_menu.alpha(ct.panel_opacity)
    } else {
        base_menu.alpha(1.0)
    }
}

/// Compute backdrop blur radius for floating panels, independent of `panel_opacity`.
pub fn panel_blur(ct: &CustomTheme) -> Option<f32> {
    let ct = ct.clamped();
    if ct.fx_enabled && ct.fx_frosted && ct.blur_radius > 0.0 {
        Some(ct.blur_radius)
    } else {
        None
    }
}

/// Compute floating panel border color: transparent when `fx_borders` is off,
/// subtle hairline when translucent, or `base_line` when opaque.
pub fn panel_border(base_line: Color, dark: bool, ct: &CustomTheme) -> Color {
    let ct = ct.clamped();
    if !ct.fx_borders {
        return Color::TRANSPARENT;
    }
    if ct.fx_enabled && ct.panel_opacity < 0.999 {
        if dark {
            Color::WHITE.alpha(0.12)
        } else {
            Color::BLACK.alpha(0.10)
        }
    } else {
        base_line
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

    fn contrast_ratio(a: Color, b: Color) -> f32 {
        let la = a.luminance();
        let lb = b.luminance();
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn the_preference_round_trips() {
        for theme in [Theme::System, Theme::Light, Theme::Dark, Theme::Custom] {
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

    #[test]
    fn backdrop_style_round_trips() {
        for style in BackdropStyle::ALL {
            assert_eq!(BackdropStyle::from_str(style.as_str()), style);
            assert_eq!(BackdropStyle::from_str(&style.as_str().to_uppercase()), style);
            assert!(!style.label().is_empty());
        }
        assert_eq!(BackdropStyle::from_str("unknown"), BackdropStyle::Opaque);
    }

    #[test]
    fn corner_style_round_trips_and_radii() {
        assert_eq!(CornerStyle::Sharp.radius(), 4.0);
        assert_eq!(CornerStyle::Standard.radius(), 8.0);
        assert_eq!(CornerStyle::Rounded.radius(), 12.0);
        for style in CornerStyle::ALL {
            assert_eq!(CornerStyle::from_str(style.as_str()), style);
            assert_eq!(CornerStyle::from_str(&style.as_str().to_uppercase()), style);
            assert!(!style.label().is_empty());
        }
        assert_eq!(CornerStyle::from_str("unknown"), CornerStyle::Standard);
    }

    #[test]
    fn custom_theme_clamping_and_reset() {
        let mut ct = CustomTheme {
            accent: Color {
                r: 2.0,
                g: -1.0,
                b: f32::NAN,
                a: 0.2,
            },
            dark: false,
            surface_tint: 0.9,
            panel_opacity: 0.1,
            blur_radius: 99.0,
            backdrop: BackdropStyle::Acrylic,
            corners: CornerStyle::Rounded,
            fx_enabled: false,
            fx_frosted: false,
            fx_shadows: false,
            fx_borders: false,
        };
        let clamped = ct.clamped();
        assert_eq!(clamped.surface_tint, SURFACE_TINT_MAX);
        assert_eq!(clamped.panel_opacity, PANEL_OPACITY_MIN);
        assert_eq!(clamped.blur_radius, BLUR_RADIUS_MAX);
        assert_eq!(clamped.accent.r, 1.0);
        assert_eq!(clamped.accent.g, 0.0);
        assert_eq!(clamped.accent.b, 0.0);
        assert_eq!(clamped.accent.a, 1.0);

        let nan_clamped = CustomTheme {
            surface_tint: f32::NAN,
            panel_opacity: f32::INFINITY,
            blur_radius: f32::NEG_INFINITY,
            ..ct
        }
        .clamped();
        assert_eq!(nan_clamped.surface_tint, 0.0);
        assert_eq!(nan_clamped.panel_opacity, 0.80);
        assert_eq!(nan_clamped.blur_radius, 12.0);

        ct.reset();
        assert_eq!(ct, CustomTheme::default());
        assert_eq!(ct.accent_hex(), DEFAULT_ACCENT_HEX);
    }

    #[test]
    fn contrast_ink_picks_readable_text() {
        for bright in ["#4cc2ff", "#ffd700", "#ffffff", "#88c0d0"] {
            let bg = rune::color::hex(bright).unwrap();
            let ink = contrast_ink(bg);
            assert!(
                ink.luminance() < 0.05,
                "{bright} should get dark ink, got {:?}",
                ink
            );
            assert!(contrast_ratio(bg, ink) >= 4.5);
        }
        for dark in ["#000000", "#0078d4", "#005fb8", "#107c10", "#191919"] {
            let bg = rune::color::hex(dark).unwrap();
            let ink = contrast_ink(bg);
            assert_eq!(ink, Color::WHITE, "{dark} should get white ink");
            assert!(contrast_ratio(bg, ink) >= 4.5);
        }
    }

    #[test]
    fn extreme_accents_stay_visible_and_contrasted() {
        let black_ct = CustomTheme {
            accent: Color::BLACK,
            dark: true,
            ..CustomTheme::default()
        };
        let dark_pal = derive_custom_colors(true, &black_ct);
        assert!(contrast_ratio(dark_pal.accent, dark_pal.content) >= 2.5);
        assert!(contrast_ratio(dark_pal.accent_fill, dark_pal.content) >= 2.0);
        assert!(contrast_ratio(dark_pal.accent_fill, dark_pal.accent_ink) >= 4.5);

        let white_ct = CustomTheme {
            accent: Color::WHITE,
            dark: false,
            ..CustomTheme::default()
        };
        let light_pal = derive_custom_colors(false, &white_ct);
        assert!(contrast_ratio(light_pal.accent, light_pal.content) >= 2.5);
        assert!(contrast_ratio(light_pal.accent_fill, light_pal.content) >= 1.7);
        assert!(contrast_ratio(light_pal.accent_fill, light_pal.accent_ink) >= 4.5);
    }

    #[test]
    fn surface_tint_modulates_surfaces() {
        let untinted = CustomTheme {
            surface_tint: 0.0,
            ..CustomTheme::default()
        };
        let tinted = CustomTheme {
            surface_tint: 0.25,
            ..CustomTheme::default()
        };
        let p0 = derive_custom_colors(true, &untinted);
        let p1 = derive_custom_colors(true, &tinted);
        assert_ne!(p0.title, p1.title);
        assert_ne!(p0.toolbar, p1.toolbar);
        assert_ne!(p0.tab_active, p1.tab_active);
        assert_ne!(p0.menu, p1.menu);
        assert_ne!(p0.content, p1.content);
    }

    #[test]
    fn panel_fill_and_blur_are_decoupled() {
        let base_menu = rune::color::hex("#2b2b2b").unwrap();
        let base_line = rune::color::hex("#454545").unwrap();

        // Frosted off, opacity 0.72 -> still translucent, blur disabled.
        let translucent_only = CustomTheme {
            fx_enabled: true,
            fx_frosted: false,
            panel_opacity: 0.72,
            blur_radius: 16.0,
            ..CustomTheme::default()
        };
        assert!((panel_fill(base_menu, &translucent_only).a - 0.72).abs() < 1e-6);
        assert_eq!(panel_blur(&translucent_only), None);
        assert_eq!(panel_border(base_line, true, &translucent_only), Color::WHITE.alpha(0.12));

        // Opacity 1.0, frosted on -> opaque fill, blur still reports radius if enabled.
        let opaque_frosted = CustomTheme {
            fx_enabled: true,
            fx_frosted: true,
            panel_opacity: 1.0,
            blur_radius: 14.0,
            ..CustomTheme::default()
        };
        assert!((panel_fill(base_menu, &opaque_frosted).a - 1.0).abs() < 1e-6);
        assert_eq!(panel_blur(&opaque_frosted), Some(14.0));
        assert_eq!(panel_border(base_line, true, &opaque_frosted), base_line);

        // Master switch off -> opaque and no blur regardless of sliders.
        let disabled = CustomTheme {
            fx_enabled: false,
            fx_frosted: true,
            fx_borders: false,
            panel_opacity: 0.65,
            blur_radius: 20.0,
            ..CustomTheme::default()
        };
        assert!((panel_fill(base_menu, &disabled).a - 1.0).abs() < 1e-6);
        assert_eq!(panel_blur(&disabled), None);
        assert_eq!(panel_border(base_line, true, &disabled), Color::TRANSPARENT);
    }

    #[test]
    fn presets_are_valid_and_contrasted() {
        assert!(!ThemePreset::ALL.is_empty());
        for preset in ThemePreset::ALL {
            assert!(
                rune::color::hex(preset.accent_hex).is_some(),
                "invalid hex in preset {}",
                preset.id
            );
            let ct = preset.to_custom_theme();
            assert_eq!(ct, ct.clamped());
            let pal = derive_custom_colors(ct.dark, &ct);
            assert!(
                contrast_ratio(pal.accent_fill, pal.accent_ink) >= 4.5,
                "preset {} failed button contrast",
                preset.id
            );
        }
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
