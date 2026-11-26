//! Named colour palettes, loaded from and saved back to `mdlite.ini`.
//!
//! Includes the ten built-in palettes inspired by rmdv (One Dark/Light,
//! GitHub Dark/Light, Solarized Dark/Light, Gruvbox Dark, Nord, Dracula,
//! Tokyo Night) plus derived surface, border, selection and hover tones so
//! custom user palettes get cohesive chrome automatically.
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub background: u32,
    pub foreground: u32,
    pub muted: u32,
    pub accent: u32,
    pub code_background: u32,
    pub code_foreground: u32,
    pub rule: u32,
}

// Win32 COLORREF stores channels as 0x00BBGGRR.
pub const fn rgb(hex: u32) -> u32 {
    ((hex & 0xff) << 16) | (hex & 0xff00) | ((hex >> 16) & 0xff)
}

/// Linearly blend two COLORREFs: `num / 256` of `b` over `a`. Pure integer math.
pub const fn mix(a: u32, b: u32, num: u32) -> u32 {
    let inv = 256u32.saturating_sub(num);
    let r = (((a & 0xff) * inv + (b & 0xff) * num) + 128) >> 8;
    let g = ((((a >> 8) & 0xff) * inv + ((b >> 8) & 0xff) * num) + 128) >> 8;
    let bl = ((((a >> 16) & 0xff) * inv + ((b >> 16) & 0xff) * num) + 128) >> 8;
    (r & 0xff) | ((g & 0xff) << 8) | ((bl & 0xff) << 16)
}

/// Pipeline names as they appear in `mdlite.ini`, indexed by `render::Pipe`.
pub const RENDERERS: [&str; 3] = ["gdi", "gdiplus", "direct2d"];

impl Palette {
    /// The seven roles, as an INI key and the label the settings window shows,
    /// in the order they are laid out there.
    pub const ROLES: [(&'static str, &'static str); 7] = [
        ("background", "Background"),
        ("foreground", "Text"),
        ("muted", "Muted text"),
        ("accent", "Links"),
        ("code_background", "Code background"),
        ("code_foreground", "Code text"),
        ("rule", "Rules and bars"),
    ];

    /// Atom / Zed One Dark (rmdv default dark), enhanced for clarity.
    pub fn dark() -> Palette {
        Palette::one_dark()
    }
    /// Atom / Zed One Light (rmdv default light), enhanced for clarity.
    #[cfg(test)]
    pub fn light() -> Palette {
        Palette::one_light()
    }
    pub fn one_dark() -> Palette {
        Palette {
            background: rgb(0x282c34),
            foreground: rgb(0xdcdfe4),
            muted: rgb(0x969ca7),
            accent: rgb(0xe5a06b),
            code_background: rgb(0x30333b),
            code_foreground: rgb(0x98c379),
            rule: rgb(0x3e4451),
        }
    }
    pub fn one_light() -> Palette {
        Palette {
            background: rgb(0xfafafa),
            foreground: rgb(0x383a42),
            muted: rgb(0x70747c),
            accent: rgb(0xd97757),
            code_background: rgb(0xf1f1f2),
            code_foreground: rgb(0x40783f),
            rule: rgb(0xdadce0),
        }
    }
    pub fn github_dark() -> Palette {
        Palette {
            background: rgb(0x0d1117),
            foreground: rgb(0xc9d1d9),
            muted: rgb(0x8b949e),
            accent: rgb(0xfd8c73),
            code_background: rgb(0x161b22),
            code_foreground: rgb(0xa5d6ff),
            rule: rgb(0x30363d),
        }
    }
    pub fn github_light() -> Palette {
        Palette {
            background: rgb(0xffffff),
            foreground: rgb(0x24292f),
            muted: rgb(0x656d76),
            accent: rgb(0xbc4c00),
            code_background: rgb(0xf6f8fa),
            code_foreground: rgb(0x0a3069),
            rule: rgb(0xd0d7de),
        }
    }
    pub fn solarized_dark() -> Palette {
        Palette {
            background: rgb(0x002b36),
            foreground: rgb(0x93a1a1),
            muted: rgb(0x657b83),
            accent: rgb(0xcb4b16),
            code_background: rgb(0x073642),
            code_foreground: rgb(0x2aa198),
            rule: rgb(0x0f4b5b),
        }
    }
    pub fn solarized_light() -> Palette {
        Palette {
            background: rgb(0xfdf6e3),
            foreground: rgb(0x586e75),
            muted: rgb(0x657b83),
            accent: rgb(0xcb4b16),
            code_background: rgb(0xf4eddb),
            code_foreground: rgb(0x2a8078),
            rule: rgb(0xd6cfbb),
        }
    }
    pub fn gruvbox_dark() -> Palette {
        Palette {
            background: rgb(0x282828),
            foreground: rgb(0xebdbb2),
            muted: rgb(0xa89984),
            accent: rgb(0xfe8019),
            code_background: rgb(0x32302f),
            code_foreground: rgb(0xb8bb26),
            rule: rgb(0x504945),
        }
    }
    pub fn nord() -> Palette {
        Palette {
            background: rgb(0x2e3440),
            foreground: rgb(0xd8dee9),
            muted: rgb(0x8892a8),
            accent: rgb(0x88c0d0),
            code_background: rgb(0x353b47),
            code_foreground: rgb(0xa3be8c),
            rule: rgb(0x4c566a),
        }
    }
    pub fn dracula() -> Palette {
        Palette {
            background: rgb(0x282a36),
            foreground: rgb(0xf8f8f2),
            muted: rgb(0x959ab8),
            accent: rgb(0xbd93f9),
            code_background: rgb(0x30313d),
            code_foreground: rgb(0x50fa7b),
            rule: rgb(0x44475a),
        }
    }
    pub fn tokyo_night() -> Palette {
        Palette {
            background: rgb(0x24283b),
            foreground: rgb(0xc0caf5),
            muted: rgb(0x9aa5ce),
            accent: rgb(0x7aa2f7),
            code_background: rgb(0x2c3042),
            code_foreground: rgb(0x9ece6a),
            rule: rgb(0x3b4261),
        }
    }

    pub fn role(&self, index: usize) -> u32 {
        [
            self.background,
            self.foreground,
            self.muted,
            self.accent,
            self.code_background,
            self.code_foreground,
            self.rule,
        ][index.min(6)]
    }
    pub fn set_role(&mut self, index: usize, color: u32) {
        *[
            &mut self.background,
            &mut self.foreground,
            &mut self.muted,
            &mut self.accent,
            &mut self.code_background,
            &mut self.code_foreground,
            &mut self.rule,
        ][index.min(6)] = color;
    }
    /// A palette is dark when its background is. Perceptual channel weights, so
    /// a saturated blue background still counts as dark.
    pub fn is_dark(&self) -> bool {
        let channel = |shift: u32| (self.background >> shift) & 0xff;
        channel(0) * 2 + channel(8) * 5 + channel(16) < 128 * 8
    }
    /// A colour that stays legible on this palette's chrome: the foreground,
    /// unless the caller is drawing on a highlight.
    pub fn contrasting(color: u32) -> u32 {
        let channel = |shift: u32| (color >> shift) & 0xff;
        if channel(0) * 2 + channel(8) * 5 + channel(16) < 128 * 8 {
            rgb(0xffffff)
        } else {
            rgb(0x101010)
        }
    }

    // Derived rmdv-inspired chrome & surface tones, computed from the 7 roles
    // with zero heap allocation so every built-in and custom palette looks cohesive.

    /// Recessed surface for the outline sidebar and tab bar background.
    pub fn sidebar(&self) -> u32 {
        if self.is_dark() {
            mix(self.background, rgb(0x000000), 42)
        } else {
            mix(self.background, rgb(0x000000), 10)
        }
    }
    /// Subtle elevated surface for table headers, status bar, and floating controls.
    pub fn surface(&self) -> u32 {
        if self.is_dark() {
            mix(self.background, rgb(0xffffff), 10)
        } else {
            mix(self.background, rgb(0x000000), 12)
        }
    }
    /// Hairline border around code blocks, tables, and chrome dividers.
    pub fn border(&self) -> u32 {
        if self.is_dark() {
            mix(self.background, rgb(0xffffff), 24)
        } else {
            mix(self.background, rgb(0x000000), 30)
        }
    }
    /// Subtle hover fill for tabs, outline items, and toolbar buttons.
    pub fn hover(&self, base: u32) -> u32 {
        if self.is_dark() {
            mix(base, rgb(0xebebeb), 24)
        } else {
            mix(base, rgb(0x1a1a1a), 16)
        }
    }
    /// Accent-tinted wash for the active outline item (like rmdv's tree_selected_bg).
    pub fn active_item(&self, base: u32) -> u32 {
        mix(base, self.accent, if self.is_dark() { 38 } else { 28 })
    }
    /// Accent-tinted background behind selected document text.
    pub fn selection(&self) -> u32 {
        mix(
            self.background,
            self.accent,
            if self.is_dark() { 86 } else { 62 },
        )
    }
    /// Subtle accent wash behind a hovered link.
    pub fn link_hover(&self) -> u32 {
        mix(
            self.background,
            self.accent,
            if self.is_dark() { 38 } else { 28 },
        )
    }
    /// Scrollbar thumb at rest and when hovered/dragged.
    pub fn scroller(&self, hot: bool) -> u32 {
        let ink = if self.is_dark() {
            rgb(0xffffff)
        } else {
            rgb(0x000000)
        };
        let strength = match (self.is_dark(), hot) {
            (true, false) => 38,
            (true, true) => 78,
            (false, false) => 46,
            (false, true) => 92,
        };
        mix(self.background, ink, strength)
    }
    /// Subtle secondary text tone for chevrons, close icons, and empty hints.
    pub fn subtle(&self) -> u32 {
        mix(self.background, self.muted, 175)
    }
}

#[derive(Clone, Debug)]
pub struct Themes {
    /// Every palette in the file, in the order it defines them. Never empty.
    pub palettes: Vec<(String, Palette)>,
    pub start: usize,
    /// Pipeline the user pinned, or `None` to choose one per block.
    pub renderer: Option<u8>,
    /// Animate scrolling instead of jumping.
    pub smooth: bool,
    /// Text size as a percentage, on top of the monitor's DPI.
    pub zoom: i32,
    /// Show the zoom control floating over the document.
    pub floating: bool,
    /// Show the heading outline sidebar on startup.
    pub outline: bool,
}
impl Default for Themes {
    fn default() -> Self {
        Self {
            palettes: vec![
                ("One Dark".to_owned(), Palette::one_dark()),
                ("One Light".to_owned(), Palette::one_light()),
                ("GitHub Dark".to_owned(), Palette::github_dark()),
                ("GitHub Light".to_owned(), Palette::github_light()),
                ("Solarized Dark".to_owned(), Palette::solarized_dark()),
                ("Solarized Light".to_owned(), Palette::solarized_light()),
                ("Gruvbox Dark".to_owned(), Palette::gruvbox_dark()),
                ("Nord".to_owned(), Palette::nord()),
                ("Dracula".to_owned(), Palette::dracula()),
                ("Tokyo Night".to_owned(), Palette::tokyo_night()),
            ],
            start: 0,
            renderer: None,
            smooth: true,
            zoom: 100,
            floating: true,
            outline: true,
        }
    }
}
impl Themes {
    /// How far the text may be scaled, and the step each key or notch takes.
    pub const ZOOM: (i32, i32, i32) = (50, 400, 10);
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }
    pub fn find(&self, name: &str) -> Option<usize> {
        self.palettes
            .iter()
            .position(|(known, _)| known.eq_ignore_ascii_case(name))
    }
    /// As many palettes as the Options menu has command ids for. Beyond this
    /// a palette could not be selected, so it is not created.
    pub const MOST_PALETTES: usize = 64;
    /// Add a palette, copying `from` as a starting point, and return its
    /// index. An existing name selects that palette instead of duplicating it.
    pub fn add(&mut self, name: &str, from: Palette) -> usize {
        // The name becomes an INI section, so it may not carry the syntax.
        let name: String = name
            .chars()
            .filter(|c| !c.is_control() && !matches!(c, '[' | ']' | '=' | ';'))
            .take(32)
            .collect();
        let name = name.trim();
        if name.eq_ignore_ascii_case("viewer") {
            return self.add("", from);
        }
        if name.is_empty() {
            let mut n = 2;
            while self.find(&format!("Custom {n}")).is_some() {
                n += 1;
            }
            let name = if self.find("Custom").is_none() {
                "Custom".to_owned()
            } else {
                format!("Custom {n}")
            };
            return self.add(&name, from);
        }
        self.find(name).unwrap_or_else(|| {
            if self.palettes.len() >= Themes::MOST_PALETTES {
                return self.palettes.len() - 1;
            }
            self.palettes.push((name.to_owned(), from));
            self.palettes.len() - 1
        })
    }
    /// Every section but `[viewer]` names a palette. Unknown names start from
    /// the built-in dark colours, so a section may override only what it wants.
    pub fn parse(source: &str) -> Self {
        let mut result = Self::default();
        let mut start = None;
        // usize::MAX while inside [viewer]; otherwise an index into palettes.
        let mut section = usize::MAX;
        for line in source.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
                .map(str::trim)
            {
                section = if name.eq_ignore_ascii_case("viewer") {
                    usize::MAX
                } else if name.eq_ignore_ascii_case("dark") {
                    0
                } else if name.eq_ignore_ascii_case("light") {
                    1
                } else {
                    result.add(name, Palette::dark())
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.split(';').next().unwrap_or("").trim();
            if section == usize::MAX {
                match key.as_str() {
                    // The named palette may be defined further down the file.
                    "theme" => start = Some(value.to_owned()),
                    "zoom" => {
                        result.zoom = value
                            .parse()
                            .unwrap_or(100)
                            .clamp(Themes::ZOOM.0, Themes::ZOOM.1)
                    }
                    "floating" => {
                        result.floating = !["off", "false", "no", "0"]
                            .contains(&value.to_ascii_lowercase().as_str())
                    }
                    "outline" => {
                        result.outline = !["off", "false", "no", "0"]
                            .contains(&value.to_ascii_lowercase().as_str())
                    }
                    "smooth" => {
                        result.smooth = !["off", "false", "no", "0"]
                            .contains(&value.to_ascii_lowercase().as_str())
                    }
                    "renderer" => {
                        result.renderer = RENDERERS
                            .iter()
                            .position(|name| value.eq_ignore_ascii_case(name))
                            .map(|index| index as u8)
                    }
                    _ => {}
                }
                continue;
            }
            let value = value.strip_prefix('#').unwrap_or(value);
            if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            if let Some(role) = Palette::ROLES.iter().position(|(name, _)| *name == key) {
                let color = rgb(u32::from_str_radix(value, 16).unwrap());
                result.palettes[section].1.set_role(role, color);
            }
        }
        if let Some(name) = start {
            if let Some(index) = result.find(&name) {
                result.start = index;
            } else if name.eq_ignore_ascii_case("dark") {
                result.start = 0;
            } else if name.eq_ignore_ascii_case("light") {
                result.start = 1;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overrides_are_partial_and_case_insensitive() {
        let t = Themes::parse("\u{feff}[Viewer]\nTheme=Light\n[Dark]\nforeground=#123456 ; comment\nbackground=oops\n[Light]\naccent=abcdef");
        assert_eq!(t.start, 1);
        assert_eq!(t.palettes[0].1.foreground, 0x563412);
        assert_eq!(t.palettes[0].1.background, Palette::dark().background);
        assert_eq!(t.palettes[1].1.accent, 0xefcdab);
    }
    #[test]
    fn extra_sections_become_extra_palettes() {
        let base = Themes::default().palettes.len();
        let t = Themes::parse(
            "[mytheme]\nbackground=#002b36\n[viewer]\ntheme=MyTheme\nrenderer=direct2d",
        );
        assert_eq!(t.palettes.len(), base + 1, "the built-ins plus one more");
        assert_eq!(t.palettes[base].0, "mytheme");
        assert_eq!(t.start, base, "a palette may be named before it is defined");
        assert_eq!(t.renderer, Some(2));
        // An unlisted role keeps a readable default rather than turning black.
        assert_eq!(t.palettes[base].1.foreground, Palette::dark().foreground);
        assert!(t.palettes[base].1.is_dark());
    }
    #[test]
    fn smooth_scrolling_is_on_unless_the_file_says_otherwise() {
        assert!(Themes::default().smooth);
        assert!(!Themes::parse("[viewer]\nsmooth=off").smooth);
        assert!(!Themes::parse("[viewer]\nSmooth=NO").smooth);
        assert!(Themes::parse("[viewer]\nsmooth=on").smooth);
        assert!(
            Themes::parse("[dark]\nsmooth=off").smooth,
            "only [viewer] holds it"
        );
    }
    #[test]
    fn palettes_can_be_added_and_named_safely() {
        let mut t = Themes::default();
        let base = t.palettes.len();
        let index = t.add("Sepia", Palette::light());
        assert_eq!(index, base);
        assert_eq!(t.palettes[base].0, "Sepia");
        assert_eq!(
            t.palettes[base].1,
            Palette::light(),
            "a new palette is a copy"
        );
        // An existing name selects it rather than making a second one.
        assert_eq!(t.add("sepia", Palette::dark()), base);
        assert_eq!(t.palettes.len(), base + 1);
        assert_eq!(
            t.palettes[base].1,
            Palette::light(),
            "and does not overwrite"
        );
        // INI syntax cannot be smuggled into a section name.
        let index = t.add("  [Dark]=x;\nevil  ", Palette::dark());
        assert_eq!(t.palettes[index].0, "Darkxevil");
        // An empty name still gives something usable, and never a duplicate.
        let first = t.add("", Palette::dark());
        assert_eq!(t.palettes[first].0, "Custom");
        let second = t.add("   ", Palette::dark());
        assert_eq!(t.palettes[second].0, "Custom 2");
        // What was added survives a round trip through the file.
        let mut text = String::from("[viewer]\ntheme=Custom 2\n");
        for (name, palette) in &t.palettes {
            text.push_str(&format!("[{name}]\n"));
            for (index, (key, _)) in Palette::ROLES.iter().enumerate() {
                let color = palette.role(index);
                text.push_str(&format!(
                    "{key}=#{:02X}{:02X}{:02X}\n",
                    color & 0xff,
                    (color >> 8) & 0xff,
                    (color >> 16) & 0xff
                ));
            }
        }
        let read = Themes::parse(&text);
        assert_eq!(read.palettes.len(), t.palettes.len());
        assert_eq!(read.palettes[base].1, Palette::light());
        assert_eq!(read.start, read.find("Custom 2").unwrap());
    }
    #[test]
    fn the_floating_and_outline_controls_are_on_unless_disabled() {
        assert!(Themes::default().floating);
        assert!(Themes::default().outline);
        assert!(!Themes::parse("[viewer]\nfloating=off\noutline=off").floating);
        assert!(!Themes::parse("[viewer]\nfloating=off\noutline=off").outline);
        assert!(Themes::parse("[viewer]\nFloating=Yes\nOutline=1").outline);
    }
    #[test]
    fn zoom_is_a_bounded_percentage() {
        assert_eq!(Themes::default().zoom, 100);
        assert_eq!(Themes::parse("[viewer]\nzoom=150").zoom, 150);
        assert_eq!(Themes::parse("[viewer]\nzoom=5").zoom, Themes::ZOOM.0);
        assert_eq!(Themes::parse("[viewer]\nzoom=99999").zoom, Themes::ZOOM.1);
        assert_eq!(Themes::parse("[viewer]\nzoom=big").zoom, 100);
    }
    #[test]
    fn custom_palette_cannot_replace_the_viewer_section() {
        let mut themes = Themes::default();
        let index = themes.add("Viewer", Palette::light());
        assert_eq!(themes.palettes[index].0, "Custom");
    }

    #[test]
    fn invalid_values_keep_defaults() {
        let t = Themes::parse(
            "[viewer]\ntheme=nope\nrenderer=nope\n[dark]\naccent=1234567\nrule=💛\nbogus=#ffffff",
        );
        assert_eq!(t.palettes[0].1, Palette::dark());
        assert_eq!(t.start, 0);
        assert_eq!(t.renderer, None);
    }
    #[test]
    fn brightness_decides_dark_from_the_background_alone() {
        assert!(Palette::dark().is_dark() && !Palette::light().is_dark());
        for (hex, dark) in [(0x000000, true), (0xffffff, false), (0x002b36, true)] {
            let mut palette = Palette::light();
            palette.background = rgb(hex);
            assert_eq!(palette.is_dark(), dark, "{hex:06x}");
            assert_eq!(
                Palette::contrasting(palette.background) == rgb(0xffffff),
                dark
            );
        }
    }
    #[test]
    fn every_role_round_trips_by_index() {
        let mut palette = Palette::dark();
        for index in 0..Palette::ROLES.len() {
            palette.set_role(index, rgb(0x010203 + index as u32));
            assert_eq!(palette.role(index), rgb(0x010203 + index as u32));
        }
        assert_eq!(palette.background, rgb(0x010203));
        assert_eq!(palette.rule, rgb(0x010209));
    }
    #[test]
    fn built_in_rmdv_presets_classify_dark_and_light_correctly() {
        let t = Themes::default();
        assert_eq!(t.palettes.len(), 10);
        assert!(t.palettes[0].1.is_dark()); // One Dark
        assert!(!t.palettes[1].1.is_dark()); // One Light
        assert!(t.palettes[2].1.is_dark()); // GitHub Dark
        assert!(!t.palettes[3].1.is_dark()); // GitHub Light
        assert!(t.palettes[4].1.is_dark()); // Solarized Dark
        assert!(!t.palettes[5].1.is_dark()); // Solarized Light
        assert!(t.palettes[6].1.is_dark()); // Gruvbox Dark
        assert!(t.palettes[7].1.is_dark()); // Nord
        assert!(t.palettes[8].1.is_dark()); // Dracula
        assert!(t.palettes[9].1.is_dark()); // Tokyo Night
    }
    #[test]
    fn derived_surface_and_ui_colors_are_valid_for_every_preset() {
        let themes = Themes::parse(include_str!("../mdlite.ini"));
        assert!(themes.palettes.len() >= 11);
        for (name, p) in &themes.palettes {
            let sb = p.sidebar();
            for derived in [
                p.surface(),
                sb,
                p.border(),
                p.hover(sb),
                p.active_item(sb),
                p.selection(),
                p.link_hover(),
                p.scroller(false),
                p.scroller(true),
                p.subtle(),
                Palette::contrasting(p.accent),
            ] {
                assert_eq!(
                    derived >> 24,
                    0,
                    "{name}: derived color has non-zero alpha byte"
                );
            }
            assert_ne!(
                p.surface(),
                p.foreground,
                "{name}: surface must contrast with text"
            );
            assert_ne!(
                p.selection(),
                p.background,
                "{name}: selection must be visible"
            );
            assert_ne!(p.border(), p.background, "{name}: border must be visible");
        }
    }
}
