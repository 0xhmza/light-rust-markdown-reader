//! Named colour palettes, loaded from and saved back to `mdlite.ini`.
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
    pub fn dark() -> Palette {
        Palette {
            background: rgb(0x181a1f),
            foreground: rgb(0xe3e6eb),
            muted: rgb(0xaeb6c2),
            accent: rgb(0x8ab4f8),
            code_background: rgb(0x232730),
            code_foreground: rgb(0xd5e5c0),
            rule: rgb(0x444c59),
        }
    }
    pub fn light() -> Palette {
        Palette {
            background: rgb(0xfaf9f6),
            foreground: rgb(0x242830),
            muted: rgb(0x5b6472),
            accent: rgb(0x1555a2),
            code_background: rgb(0xeeede8),
            code_foreground: rgb(0x365314),
            rule: rgb(0xc9cdd3),
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
}
impl Default for Themes {
    fn default() -> Self {
        Self {
            palettes: vec![
                ("Dark".to_owned(), Palette::dark()),
                ("Light".to_owned(), Palette::light()),
            ],
            start: 0,
            renderer: None,
            smooth: true,
            zoom: 100,
            floating: true,
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
        if let Some(index) = start.and_then(|name| result.find(&name)) {
            result.start = index;
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
        let t = Themes::parse(
            "[solarized]\nbackground=#002b36\n[viewer]\ntheme=Solarized\nrenderer=direct2d",
        );
        assert_eq!(t.palettes.len(), 3, "the two built-ins plus one more");
        assert_eq!(t.palettes[2].0, "solarized");
        assert_eq!(t.start, 2, "a palette may be named before it is defined");
        assert_eq!(t.renderer, Some(2));
        // An unlisted role keeps a readable default rather than turning black.
        assert_eq!(t.palettes[2].1.foreground, Palette::dark().foreground);
        assert!(t.palettes[2].1.is_dark());
    }
    #[test]
    fn smooth_scrolling_is_on_unless_the_file_says_otherwise() {
        assert!(Themes::default().smooth);
        assert!(
            !Themes::parse(
                "[viewer]
smooth=off"
            )
            .smooth
        );
        assert!(
            !Themes::parse(
                "[viewer]
Smooth=NO"
            )
            .smooth
        );
        assert!(
            Themes::parse(
                "[viewer]
smooth=on"
            )
            .smooth
        );
        assert!(
            Themes::parse(
                "[dark]
smooth=off"
            )
            .smooth,
            "only [viewer] holds it"
        );
    }
    #[test]
    fn palettes_can_be_added_and_named_safely() {
        let mut t = Themes::default();
        let index = t.add("Nord", Palette::light());
        assert_eq!(index, 2);
        assert_eq!(t.palettes[2].0, "Nord");
        assert_eq!(t.palettes[2].1, Palette::light(), "a new palette is a copy");
        // An existing name selects it rather than making a second one.
        assert_eq!(t.add("nord", Palette::dark()), 2);
        assert_eq!(t.palettes.len(), 3);
        assert_eq!(t.palettes[2].1, Palette::light(), "and does not overwrite");
        // INI syntax cannot be smuggled into a section name.
        let index = t.add(
            "  [Dark]=x;
evil  ",
            Palette::dark(),
        );
        assert_eq!(t.palettes[index].0, "Darkxevil");
        // An empty name still gives something usable, and never a duplicate.
        let first = t.add("", Palette::dark());
        assert_eq!(t.palettes[first].0, "Custom");
        let second = t.add("   ", Palette::dark());
        assert_eq!(t.palettes[second].0, "Custom 2");
        // What was added survives a round trip through the file.
        let mut text = String::from(
            "[viewer]
theme=Custom 2
",
        );
        for (name, palette) in &t.palettes {
            text.push_str(&format!(
                "[{name}]
"
            ));
            for (index, (key, _)) in Palette::ROLES.iter().enumerate() {
                let color = palette.role(index);
                text.push_str(&format!(
                    "{key}=#{:02X}{:02X}{:02X}
",
                    color & 0xff,
                    (color >> 8) & 0xff,
                    (color >> 16) & 0xff
                ));
            }
        }
        let read = Themes::parse(&text);
        assert_eq!(read.palettes.len(), t.palettes.len());
        assert_eq!(read.palettes[2].1, Palette::light());
        assert_eq!(read.start, read.find("Custom 2").unwrap());
    }
    #[test]
    fn the_floating_control_is_on_unless_the_file_says_otherwise() {
        assert!(Themes::default().floating);
        assert!(
            !Themes::parse(
                "[viewer]
floating=off"
            )
            .floating
        );
        assert!(
            Themes::parse(
                "[viewer]
Floating=Yes"
            )
            .floating
        );
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
}
