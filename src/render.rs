//! Pipeline selection and the GDI+ text backend.
//!
//! Three pipelines draw text. GDI is the default because it is the cheapest and
//! the sharpest for plain Latin text. GDI+ takes over when a block needs the
//! glyph fallback and alpha blending that `TextOutW` does not provide. Direct2D
//! takes over when the block needs real shaping or bidirectional ordering.
//! Selection escalates in that order and never falls back down: a block that
//! needs Direct2D keeps it even when the user pins a simpler pipeline.
//!
//! Colour emoji are the exception to block-level choice. A colour glyph is one
//! indivisible cluster, so the shaping engine draws just that cluster in place
//! and the sentence around it keeps whichever pipeline its own text earned.
#![allow(non_snake_case)]
use crate::win32::*;
use std::{cell::Cell, ffi::c_void};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pipe {
    #[default]
    Gdi,
    GdiPlus,
    D2d,
}
impl Pipe {
    pub const ALL: [Pipe; 3] = [Pipe::Gdi, Pipe::GdiPlus, Pipe::D2d];
    pub fn name(self) -> &'static str {
        crate::theme::RENDERERS[self as usize]
    }
    /// The name shown in the status bar.
    pub fn short(self) -> &'static str {
        match self {
            Pipe::Gdi => "GDI",
            Pipe::GdiPlus => "GDI+",
            Pipe::D2d => "Direct2D",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Pipe::Gdi => "&GDI — fastest, plain Latin text",
            Pipe::GdiPlus => "GDI&+ — smoother glyph fallback",
            Pipe::D2d => "Direct&2D — full shaping and colour",
        }
    }
    /// What pinning this pipeline costs the document, so the settings window
    /// can say something true about the choice under the pointer. Direct2D
    /// gets no warning: nothing renders worse under it.
    pub fn note(pinned: Option<Pipe>) -> &'static str {
        match pinned {
            None => "Each block is drawn by the cheapest pipeline that can draw it correctly, and colour glyphs are drawn in place, so one emoji never moves a whole paragraph.",
            Some(Pipe::Gdi) => "Sharpest and cheapest for Latin text. Greek, Cyrillic and CJK give up the smoother fallback of GDI+, and blocks that need shaping still use Direct2D.",
            Some(Pipe::GdiPlus) => "Smoother glyph fallback everywhere, for a little more work per line. Blocks that need shaping or bidirectional ordering still use Direct2D.",
            Some(Pipe::D2d) => "Everything is shaped by DirectWrite. Nothing renders worse this way, but plain Latin text costs more than it needs to.",
        }
    }
    /// What in `text` drives the choice, as SHAPING/COLOR/NON_LATIN flags. The
    /// status bar reports these back to the reader.
    pub fn reasons(text: &str) -> u8 {
        // Every reason lies above U+00A9, so plain ASCII decides in one pass
        // over the bytes and never looks at a character.
        if text.is_ascii() {
            return 0;
        }
        let mut flags = 0;
        for c in text.chars() {
            if (c as u32) < 0xa9 {
                continue;
            }
            // A colour cluster is drawn on its own, so it never decides which
            // pipeline the words around it need.
            if colored(c) {
                flags |= COLOR;
            } else {
                if c as u32 >= 0x250 {
                    flags |= NON_LATIN;
                }
                if crate::text::shapes(c) {
                    flags |= SHAPING;
                }
            }
        }
        flags
    }
    /// The cheapest pipeline that can draw text with these reasons. Colour
    /// glyphs do not escalate the block: they are single clusters that the
    /// shaping engine draws in place, leaving the sentence around them on GDI.
    pub fn of(reasons: u8) -> Pipe {
        if reasons & SHAPING != 0 {
            Pipe::D2d
        } else if reasons & NON_LATIN != 0 {
            Pipe::GdiPlus
        } else {
            Pipe::Gdi
        }
    }
    /// The pipeline a block will use, given what the user pinned. Content only
    /// Direct2D can render keeps Direct2D whatever the setting says.
    pub fn choose(reasons: u8, pinned: Option<Pipe>) -> Pipe {
        let wanted = Pipe::of(reasons);
        match pinned {
            Some(pinned) if wanted != Pipe::D2d => pinned,
            _ => wanted,
        }
    }
    /// The cheapest pipeline that can render `text` correctly.
    #[cfg(test)]
    pub fn required(text: &str) -> Pipe {
        Pipe::of(Pipe::reasons(text))
    }
}

/// Why a block escalated past GDI. RTL is filled in by the shaping engine.
pub const SHAPING: u8 = 1;
pub const COLOR: u8 = 2;
pub const NON_LATIN: u8 = 4;
pub const RTL: u8 = 8;
/// One line per reason, in the order they are reported.
pub const CAUSES: [(u8, &str); 4] = [
    (RTL, "right-to-left text such as Arabic or Hebrew"),
    (
        SHAPING,
        "complex-script shaping (Arabic, Indic, Thai, combining marks)",
    ),
    (COLOR, "colour emoji or symbols, drawn glyph by glyph"),
    (
        NON_LATIN,
        "non-Latin characters that GDI+ renders more evenly",
    ),
];

/// Characters Windows may render from a colour font. GDI has no path for
/// these, so the shaping engine draws each one as its own cluster.
pub fn colored(c: char) -> bool {
    matches!(c as u32, 0xa9 | 0xae | 0x203c | 0x2049 | 0x2122 | 0x2139 |
        0x2194..=0x2199 | 0x21a9..=0x21aa | 0x231a..=0x231b | 0x2328 | 0x23cf |
        0x23e9..=0x23f3 | 0x23f8..=0x23fa | 0x24c2 | 0x25aa..=0x25ab | 0x25b6 |
        0x25c0 | 0x25fb..=0x27bf | 0x2934..=0x2935 | 0x2b05..=0x2b07 |
        0x2b1b..=0x2b1c | 0x2b50 | 0x2b55 | 0x3030 | 0x303d | 0x3297 | 0x3299 |
        0x1f000..=0x1faff)
}

#[repr(C)]
struct Startup {
    version: u32,
    callback: *const c_void,
    suppress_background: i32,
    suppress_codecs: i32,
}
#[repr(C)]
struct RectF {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}
// gdiplus.dll is loaded from System32 on first use. The mingw toolchain has no
// import library for it, and a reader that never draws GDI+ text never pays.
macro_rules! flat {
    ($($name:ident($($arg:ty),*)),* $(,)?) => {
        #[allow(non_snake_case)]
        struct Api { $($name: unsafe extern "system" fn($($arg),*) -> u32,)* }
        unsafe fn api() -> Option<&'static Api> {
            static API: std::sync::OnceLock<Option<Api>> = std::sync::OnceLock::new();
            API.get_or_init(|| {
                Some(Api { $($name: std::mem::transmute(system_function(
                    "gdiplus.dll",
                    std::ffi::CStr::from_bytes_with_nul(concat!(stringify!($name), "\0").as_bytes()).ok()?,
                )?),)* })
            })
            .as_ref()
        }
    };
}
flat! {
    GdiplusStartup(*mut usize, *const Startup, *mut c_void),
    GdipCreateFromHDC(Hdc, *mut Handle),
    GdipDeleteGraphics(Handle),
    GdipSetTextRenderingHint(Handle, i32),
    GdipCreateFontFromDC(Hdc, *mut Handle),
    GdipDeleteFont(Handle),
    GdipCreateSolidFill(u32, *mut Handle),
    GdipSetSolidFillColor(Handle, u32),
    GdipDeleteBrush(Handle),
    GdipStringFormatGetGenericTypographic(*mut Handle),
    GdipCloneStringFormat(Handle, *mut Handle),
    GdipSetStringFormatFlags(Handle, i32),
    GdipDeleteStringFormat(Handle),
    GdipDrawString(Handle, *const u16, i32, Handle, *const RectF, Handle, Handle),
}

/// GDI+ needs one process-wide initialization. It is never shut down: the
/// library outlives every canvas and Windows reclaims it at process exit.
unsafe fn started() -> Option<&'static Api> {
    static TOKEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let api = api()?;
    let ok = *TOKEN.get_or_init(|| {
        let mut token = 0;
        let input = Startup {
            version: 1,
            callback: std::ptr::null(),
            suppress_background: 0,
            suppress_codecs: 0,
        };
        (api.GdiplusStartup)(&mut token, &input, std::ptr::null_mut()) == 0
    });
    ok.then_some(api)
}

/// Release a font produced by [`Canvas::font`].
pub unsafe fn delete_font(font: Handle) {
    if let Some(api) = api() {
        (api.GdipDeleteFont)(font);
    }
}

/// A GDI+ drawing surface bound to one device context for the length of a paint.
pub struct Canvas {
    graphics: Handle,
    brush: Handle,
    format: Handle,
    color: Cell<u32>,
}
impl Canvas {
    pub unsafe fn new(dc: Hdc) -> Option<Canvas> {
        let Some(api) = started() else {
            return None;
        };
        // The generic format is a shared library object and is never deleted;
        // the clone belongs to this canvas.
        let (mut graphics, mut brush, mut generic, mut format) = (NULL, NULL, NULL, NULL);
        (api.GdipCreateFromHDC)(dc, &mut graphics);
        (api.GdipCreateSolidFill)(0, &mut brush);
        (api.GdipStringFormatGetGenericTypographic)(&mut generic);
        (api.GdipCloneStringFormat)(generic, &mut format);
        let canvas = Canvas {
            graphics,
            brush,
            format,
            color: Cell::new(0),
        };
        if graphics.is_null() || brush.is_null() || format.is_null() {
            return None;
        }
        // Measure trailing spaces, never wrap, never clip: the caller has
        // already decided where every fragment begins and ends.
        (api.GdipSetStringFormatFlags)(format, 0x800 | 0x1000 | 0x4000);
        (api.GdipSetTextRenderingHint)(graphics, 5); // ClearTypeGridFit, as GDI draws.
        Some(canvas)
    }
    /// A GDI+ font matching the HFONT currently selected into `dc`, so GDI
    /// measurements stay valid for text this canvas draws.
    pub unsafe fn font(dc: Hdc) -> Handle {
        let Some(api) = api() else { return NULL };
        let mut font = NULL;
        if (api.GdipCreateFontFromDC)(dc, &mut font) != 0 {
            NULL
        } else {
            font
        }
    }
    pub unsafe fn text(&self, font: Handle, x: i32, y: i32, text: &[u16], color: u32) -> bool {
        let Some(api) = api() else { return false };
        if font.is_null() {
            return false;
        }
        // COLORREF is 0x00BBGGRR; GDI+ wants opaque 0xAARRGGBB.
        let argb = 0xff00_0000 | ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff);
        if self.color.get() != argb {
            self.color.set(argb);
            (api.GdipSetSolidFillColor)(self.brush, argb);
        }
        let rect = RectF {
            x: x as f32,
            y: y as f32,
            width: 0.0,
            height: 0.0,
        };
        (api.GdipDrawString)(
            self.graphics,
            text.as_ptr(),
            text.len() as i32,
            font,
            &rect,
            self.format,
            self.brush,
        ) == 0
    }
}
impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            let Some(api) = api() else { return };
            for (handle, delete) in [
                (self.format, api.GdipDeleteStringFormat),
                (self.brush, api.GdipDeleteBrush),
                (self.graphics, api.GdipDeleteGraphics),
            ] {
                if !handle.is_null() {
                    delete(handle);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gdi_plus_really_rasterizes_text_in_the_requested_colour() {
        unsafe {
            let surface = Buffer::new(NULL, 200, 40).expect("an off-screen surface");
            let dc = surface.dc;
            let face = wide("Segoe UI");
            let font = CreateFontW(-20, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
            let previous = SelectObject(dc, font);

            let canvas = Canvas::new(dc).expect("GDI+ must initialize");
            let plus = Canvas::font(dc);
            assert!(!plus.is_null(), "GDI+ must mirror the selected HFONT");
            let text: Vec<u16> = "Hamburgefonstiv".encode_utf16().collect();
            // 0x0000ff as a COLORREF is pure red; GDI+ takes 0xffff0000.
            assert!(canvas.text(plus, 4, 4, &text, 0x0000ff));
            let pixels = surface.pixels();
            let red = pixels.iter().filter(|&&p| p & 0xffffff == 0xff0000).count();
            assert!(
                red > 40,
                "GDI+ drew {red} red pixels; the canvas is not rendering"
            );
            assert!(
                pixels.iter().filter(|&&p| p & 0xffffff != 0).count() > red,
                "ClearType antialiasing must produce blended edge pixels too"
            );
            delete_font(plus);
            drop(canvas);
            SelectObject(dc, previous);
            DeleteObject(font);
        }
    }
    #[test]
    fn pipelines_escalate_with_content() {
        for text in ["plain ascii", "# Heading, 123 -- ok", "naive"] {
            assert_eq!(Pipe::required(text), Pipe::Gdi, "{text}");
        }
        for text in ["Ελληνικά", "Українська", "日本語", "😀 日本語"] {
            assert_eq!(Pipe::required(text), Pipe::GdiPlus, "{text}");
        }
        for text in ["العربية", "עברית", "हिन्दी", "ไทย", "café\u{301}"]
        {
            assert_eq!(Pipe::required(text), Pipe::D2d, "{text}");
        }
        // A colour glyph rides inside its sentence rather than escalating it.
        assert_eq!(Pipe::required("plain 😀 text"), Pipe::Gdi);
        assert_eq!(Pipe::reasons("plain 😀 text"), COLOR);
        let choose = |text: &str, pinned| Pipe::choose(Pipe::reasons(text), pinned);
        assert_eq!(choose("plain", Some(Pipe::D2d)), Pipe::D2d);
        assert_eq!(choose("日本語", Some(Pipe::Gdi)), Pipe::Gdi);
        assert_eq!(choose("العربية", Some(Pipe::Gdi)), Pipe::D2d);
        assert_eq!(choose("日本語", None), Pipe::GdiPlus);
        assert_eq!(choose("😀", Some(Pipe::Gdi)), Pipe::Gdi);
        assert_eq!(Pipe::reasons("😀 日本語"), COLOR | NON_LATIN);
        assert_eq!(Pipe::reasons("العربية"), SHAPING | NON_LATIN);
        assert_eq!(Pipe::reasons("plain"), 0);
        assert_eq!(Pipe::D2d.name(), "direct2d");
        assert!(Pipe::Gdi < Pipe::GdiPlus && Pipe::GdiPlus < Pipe::D2d);
    }
}
