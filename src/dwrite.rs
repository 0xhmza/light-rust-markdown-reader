//! The DirectWrite pipeline: shaping, bidi ordering and colour glyphs.
//!
//! A paragraph is shaped once by DirectWrite and drawn by a custom renderer so
//! inline code backgrounds, link colours and underlines follow the palette
//! without reshaping. The renderer paints through Direct2D when it is available
//! and falls back to DirectWrite's GDI bitmap interop when it is not.
use crate::{
    markdown::{Span, Style},
    theme::Palette,
    win32::*,
};
use std::{cell::Cell, collections::VecDeque, ffi::c_void, ptr, rc::Rc};

// Split emoji sequences as indivisible layout units. Keep VS15 text presentation
// on GDI; preserve modifiers, ZWJ, regional pairs, keycaps and tag sequences.
pub fn split(text: &str) -> Vec<(&str, bool)> {
    let mut result = Vec::new();
    let mut iter = text.char_indices().peekable();
    let mut plain = 0;
    while let Some((start, ch)) = iter.next() {
        let keycap = matches!(ch, '0'..='9' | '#' | '*');
        if !crate::render::colored(ch) && !keycap {
            continue;
        }
        let mut scan = iter.clone();
        let mut end = start + ch.len_utf8();
        let mut text_presentation = false;
        let mut has_keycap = false;
        let mut regional = (0x1f1e6..=0x1f1ff).contains(&(ch as u32));
        while let Some(&(at, next)) = scan.peek() {
            let n = next as u32;
            if matches!(n, 0xfe0e | 0xfe0f | 0x20e3 | 0x1f3fb..=0x1f3ff | 0xe0020..=0xe007f) {
                text_presentation |= n == 0xfe0e;
                has_keycap |= n == 0x20e3;
                scan.next();
                end = at + next.len_utf8();
            } else if regional && (0x1f1e6..=0x1f1ff).contains(&n) {
                scan.next();
                end = at + next.len_utf8();
                regional = false;
            } else if n == 0x200d {
                let mut joined = scan.clone();
                joined.next();
                if let Some((at, base)) = joined.next().filter(|(_, c)| crate::render::colored(*c))
                {
                    end = at + base.len_utf8();
                    scan = joined;
                    regional = false;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        if (keycap && !has_keycap) || text_presentation {
            continue;
        }
        if plain < start {
            result.push((&text[plain..start], false));
        }
        result.push((&text[start..end], true));
        plain = end;
        iter = scan;
    }
    if plain < text.len() {
        result.push((&text[plain..], false));
    }
    if result.is_empty() {
        result.push((text, false));
    }
    result
}

#[repr(C)]
#[derive(PartialEq, Eq)]
struct Guid {
    a: u32,
    b: u16,
    c: u16,
    d: [u8; 8],
}
const FACTORY2: Guid = Guid {
    a: 0x0439fc60,
    b: 0xca44,
    c: 0x4994,
    d: [0x8d, 0xee, 0x3a, 0x9a, 0xf7, 0xb7, 0x32, 0xec],
};
const RENDERER: Guid = Guid {
    a: 0xef8a8135,
    b: 0x5cc6,
    c: 0x45fe,
    d: [0x88, 0x25, 0xc5, 0xa0, 0x72, 0x4e, 0xb8, 0x19],
};
const SNAPPING: Guid = Guid {
    a: 0xeaf3a2da,
    b: 0xecf4,
    c: 0x4d24,
    d: [0xb6, 0x44, 0xb3, 0x4f, 0x68, 0x42, 0x02, 0x4b],
};
const UNKNOWN: Guid = Guid {
    a: 0,
    b: 0,
    c: 0,
    d: [0xc0, 0, 0, 0, 0, 0, 0, 0x46],
};

// Slot indices and signatures follow the SDK's dwrite.h and dwrite_2.h.
// COM pointers are owned, never sent to another thread, and released on Drop.
struct Com(Handle);
impl Clone for Com {
    fn clone(&self) -> Self {
        unsafe {
            call!(self.0, 1, () -> u32);
        }
        Self(self.0)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            call!(self.0, 2, () -> u32);
        }
    }
}
macro_rules! call {
    ($object:expr, $slot:expr, ($($arg:expr => $ty:ty),*) -> $ret:ty) => {{
        let object: Handle = $object;
        let table = *(object as *const *const *const c_void);
        let method: unsafe extern "system" fn(Handle, $($ty),*) -> $ret = std::mem::transmute(*table.add($slot));
        method(object, $($arg),*)
    }};
}
use call;
/// A COM status as a `Result`, so a failure rides up on `?`.
fn ok(hr: i32) -> Result<(), i32> {
    if hr < 0 {
        Err(hr)
    } else {
        Ok(())
    }
}
fn checked(hr: i32, object: Handle) -> Result<Com, i32> {
    if hr < 0 {
        Err(hr)
    } else if object.is_null() {
        Err(0x80004005u32 as i32)
    } else {
        Ok(Com(object))
    }
}

#[repr(C)]
struct GlyphRun {
    face: Handle,
    size: f32,
    count: u32,
    indices: *const u16,
    advances: *const f32,
    offsets: *const c_void,
    sideways: i32,
    bidi: u32,
}
#[repr(C)]
struct ColorRun {
    glyphs: GlyphRun,
    description: *const c_void,
    x: f32,
    y: f32,
    color: [f32; 4],
    palette: u16,
}
#[repr(C)]
#[derive(Default)]
struct Metrics {
    left: f32,
    top: f32,
    width: f32,
    trailing_width: f32,
    height: f32,
    layout_width: f32,
    layout_height: f32,
    bidi: u32,
    lines: u32,
}
#[repr(C)]
#[derive(Default)]
struct LineMetrics {
    length: u32,
    trailing: u32,
    newline: u32,
    height: f32,
    baseline: f32,
    trimmed: i32,
}

pub struct Layout {
    object: Com,
    pub width: i32,
    pub height: i32,
    /// Width the text wants when nothing forces it to wrap. Table columns need
    /// this before they can be sized.
    pub natural: i32,
    /// Distance from the top of the layout to its first baseline. Only a
    /// colour cluster sitting inside a line of GDI text needs it.
    pub ascent: i32,
}

// Direct2D. Slot indices follow d2d1.h: ID2D1Factory::CreateDCRenderTarget is
// the last of its methods, and ID2D1DCRenderTarget::BindDC follows the whole
// ID2D1RenderTarget table.
const FACTORY_D2D: Guid = Guid {
    a: 0x06152247,
    b: 0x6f50,
    c: 0x465a,
    d: [0x92, 0x45, 0x11, 0x8b, 0xfd, 0x3b, 0x60, 0x07],
};
#[repr(C)]
struct ColorF {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}
impl ColorF {
    // COLORREF is 0x00BBGGRR.
    fn new(color: u32) -> Self {
        let channel = |shift: u32| ((color >> shift) & 0xff) as f32 / 255.0;
        Self {
            r: channel(0),
            g: channel(8),
            b: channel(16),
            a: 1.0,
        }
    }
}
#[repr(C)]
struct RectF {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}
#[repr(C)]
struct PointF {
    x: f32,
    y: f32,
}
#[repr(C)]
struct TargetProperties {
    kind: u32,
    format: u32,
    alpha: u32,
    dpi_x: f32,
    dpi_y: f32,
    usage: u32,
    level: u32,
}
struct Direct2D {
    target: Com,
    brush: Com,
    _factory: Com,
}
unsafe fn direct2d() -> Option<Direct2D> {
    type Create = unsafe extern "system" fn(u32, *const Guid, *const u32, *mut Handle) -> i32;
    static CREATE: std::sync::OnceLock<Option<Create>> = std::sync::OnceLock::new();
    let create = (*CREATE.get_or_init(|| {
        system_function("d2d1.dll", c"D2D1CreateFactory").map(|function| {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Create>(function)
        })
    }))?;
    let mut raw = NULL;
    let factory = checked(create(0, &FACTORY_D2D, &0, &mut raw), raw).ok()?;
    // A device-context target renders straight into the window DC, so scrolling,
    // clipping and the GDI-drawn decorations around the text all still apply.
    let properties = TargetProperties {
        kind: 0,
        format: 87, // DXGI_FORMAT_B8G8R8A8_UNORM
        alpha: 3,   // D2D1_ALPHA_MODE_IGNORE
        dpi_x: 96.0,
        dpi_y: 96.0, // One DIP per pixel: the viewer has already applied DPI.
        usage: 2,    // GDI compatible.
        level: 0,
    };
    let hr = call!(factory.0, 16, (&properties => *const TargetProperties, &mut raw => *mut Handle) -> i32);
    let target = checked(hr, raw).ok()?;
    let hr = call!(target.0, 8, (&ColorF::new(0) => *const ColorF, ptr::null() => *const c_void, &mut raw => *mut Handle) -> i32);
    let brush = checked(hr, raw).ok()?;
    Some(Direct2D {
        target,
        brush,
        _factory: factory,
    })
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TextRange {
    start: u32,
    length: u32,
}

#[derive(PartialEq, Eq, Hash)]
pub struct RichText {
    pub text: Box<[u16]>,
    ranges: Vec<(TextRange, u8)>,
    rtl: bool,
}
impl RichText {
    /// True when the paragraph's first strong character orders right to left.
    pub fn rtl(&self) -> bool {
        self.rtl
    }
    pub fn new(spans: &[Span]) -> Self {
        let mut text = Vec::new();
        let mut ranges = Vec::with_capacity(spans.len());
        for span in spans {
            let start = text.len() as u32;
            text.extend(span.text.encode_utf16());
            ranges.push((
                TextRange {
                    start,
                    length: text.len() as u32 - start,
                },
                span.style.0,
            ));
        }
        let rtl = crate::text::right_to_left(&text);
        Self {
            text: text.into_boxed_slice(),
            ranges,
            rtl,
        }
    }
}
struct CachedLayout {
    text: Rc<RichText>,
    pixels: i32,
    width: i32,
    bold: bool,
    layout: Rc<Layout>,
}
pub struct Engine {
    factory: Com,
    target: Com,
    renderer: Com,
    capacity: (u32, u32),
    rich: VecDeque<CachedLayout>,
    effects: [Com; 2],
    formats: Vec<(i32, Com)>,
    d2d: Option<Direct2D>,
}
impl Engine {
    pub unsafe fn new() -> Result<Self, i32> {
        type Create = unsafe extern "system" fn(u32, *const Guid, *mut Handle) -> i32;
        static CREATE: std::sync::OnceLock<Option<Create>> = std::sync::OnceLock::new();
        let create = CREATE
            .get_or_init(|| {
                system_function("dwrite.dll", c"DWriteCreateFactory").map(|function| {
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, Create>(function)
                })
            })
            .ok_or(0x80004005u32 as i32)?;
        let mut raw = NULL;
        let hr = create(0, &FACTORY2, &mut raw);
        let factory = checked(hr, raw)?;
        let hr = call!(factory.0, 17, (&mut raw => *mut Handle) -> i32);
        let interop = checked(hr, raw)?;
        let hr = call!(interop.0, 7, (NULL => Hdc, 64 => u32, 64 => u32, &mut raw => *mut Handle) -> i32);
        let target = checked(hr, raw)?;
        call!(target.0, 6, (1.0 => f32) -> i32);
        let hr = call!(factory.0, 10, (&mut raw => *mut Handle) -> i32);
        let params = checked(hr, raw)?;
        let renderer = Com(Box::into_raw(Box::new(Renderer {
            vtable: &VTABLE,
            refs: Cell::new(1),
            factory: factory.clone(),
            target: target.clone(),
            params,
            foreground: Cell::new(0),
            palette: Cell::new(Palette::dark()),
            color_layers: Cell::new(0),
            d2d: Cell::new(NULL),
            brush: Cell::new(NULL),
        })) as Handle);
        Ok(Self {
            factory,
            target,
            renderer,
            capacity: (64, 64),
            rich: VecDeque::new(),
            effects: [effect(Style::CODE), effect(Style::LINK)],
            formats: vec![],
            d2d: direct2d(),
        })
    }
    pub fn clear_paragraphs(&mut self) {
        self.rich.clear();
    }

    pub unsafe fn paragraph(
        &mut self,
        text: &Rc<RichText>,
        pixels: i32,
        width: i32,
        bold: bool,
    ) -> Result<Rc<Layout>, i32> {
        if let Some(at) = self.rich.iter().position(|entry| {
            Rc::ptr_eq(&entry.text, text)
                && entry.pixels == pixels
                && entry.width == width
                && entry.bold == bold
        }) {
            let entry = self.rich.remove(at).unwrap();
            let layout = entry.layout.clone();
            self.rich.push_back(entry);
            return Ok(layout);
        }
        let mut raw = NULL;
        let hr = call!(self.factory.0, 15, (wide("Segoe UI").as_ptr() => *const u16, NULL => Handle,
            if bold {700} else {400} => u32, 0 => u32, 5 => u32, pixels as f32 => f32,
            wide("en-us").as_ptr() => *const u16, &mut raw => *mut Handle) -> i32);
        let format = checked(hr, raw)?;
        ok(call!(format.0, 6, (text.rtl as u32 => u32) -> i32))?;
        let hr = call!(self.factory.0, 18, (text.text.as_ptr() => *const u16, text.text.len() as u32 => u32,
            format.0 => Handle, width.max(1) as f32 => f32, 1.0e9 => f32, &mut raw => *mut Handle) -> i32);
        let object = checked(hr, raw)?;
        for &(range, style) in &text.ranges {
            if range.length == 0 {
                continue;
            }
            let weight = if bold || style & Style::BOLD != 0 {
                700
            } else {
                400
            };
            for (slot, value) in [
                (32, weight),
                (33, if style & Style::ITALIC != 0 { 2 } else { 0 }),
                (36, (style & Style::LINK != 0) as u32),
                (37, (style & Style::STRIKE != 0) as u32),
            ] {
                ok(call!(object.0, slot, (value => u32, range => TextRange) -> i32))?;
            }
            if style & Style::CODE != 0 {
                ok(
                    call!(object.0, 31, (wide("Consolas").as_ptr() => *const u16, range => TextRange) -> i32),
                )?;
            }
            let effect = if style & Style::CODE != 0 {
                self.effects[0].0
            } else if style & Style::LINK != 0 {
                self.effects[1].0
            } else {
                NULL
            };
            if !effect.is_null() {
                ok(call!(object.0, 38, (effect => Handle, range => TextRange) -> i32))?;
            }
        }
        let mut metrics = Metrics::default();
        ok(call!(object.0, 60, (&mut metrics => *mut Metrics) -> i32))?;
        let layout = Rc::new(Layout {
            object,
            width,
            height: metrics.height.ceil() as i32,
            natural: metrics.width.ceil() as i32,
            ascent: 0,
        });
        if self.rich.len() == 64 {
            self.rich.pop_front();
        }
        self.rich.push_back(CachedLayout {
            text: text.clone(),
            pixels,
            width,
            bold,
            layout: layout.clone(),
        });
        Ok(layout)
    }

    /// Draw the visible slice of a paragraph at `x`, `y` in `dc` coordinates.
    /// Direct2D renders straight into the device context; without it a tiled
    /// DirectWrite bitmap target keeps the same output on legacy systems.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw_paragraph(
        &mut self,
        layout: &Layout,
        dc: Hdc,
        x: i32,
        y: i32,
        clip: Rect,
        palette: Palette,
        foreground: u32,
        background: u32,
    ) -> bool {
        let area = Rect {
            left: (x - 2).max(clip.left),
            top: (y - 2).max(clip.top),
            right: (x + layout.width + 2).min(clip.right),
            bottom: y
                .saturating_add(layout.height)
                .saturating_add(2)
                .min(clip.bottom),
        };
        if area.right <= area.left || area.bottom <= area.top {
            return true;
        }
        let renderer = &*(self.renderer.0 as *const Renderer);
        renderer.foreground.set(foreground);
        renderer.palette.set(palette);
        if self.d2d.is_some() {
            if self.draw_direct2d(layout, dc, x, y, area, background) {
                return true;
            }
            // A lost device is not fatal: drop it and keep drawing through GDI.
            self.d2d = None;
        }
        self.draw_bitmap(layout, dc, x, y, area)
    }

    unsafe fn draw_direct2d(
        &mut self,
        layout: &Layout,
        dc: Hdc,
        x: i32,
        y: i32,
        area: Rect,
        background: u32,
    ) -> bool {
        let Some(d2d) = &self.d2d else { return false };
        let renderer = &*(self.renderer.0 as *const Renderer);
        if call!(d2d.target.0, 57, (dc => Hdc, &area => *const Rect) -> i32) < 0 {
            return false;
        }
        call!(d2d.target.0, 48, () -> ());
        // BindDC does not import the device context's pixels, so the row's own
        // background is restored before the glyphs go down.
        call!(d2d.target.0, 47, (&ColorF::new(background) => *const ColorF) -> ());
        renderer.d2d.set(d2d.target.0);
        renderer.brush.set(d2d.brush.0);
        let hr = call!(layout.object.0, 58, (ptr::null_mut() => *mut c_void, self.renderer.0 => Handle,
            (x - area.left) as f32 => f32, (y - area.top) as f32 => f32) -> i32);
        renderer.d2d.set(NULL);
        renderer.brush.set(NULL);
        let end = call!(d2d.target.0, 49, (ptr::null_mut() => *mut u64, ptr::null_mut() => *mut u64) -> i32);
        hr >= 0 && end >= 0
    }

    // Rasterize only the visible portion. A long paragraph must never allocate a
    // document-height bitmap. Tiles also cap temporary bitmap height at 512px.
    unsafe fn draw_bitmap(&mut self, layout: &Layout, dc: Hdc, x: i32, y: i32, area: Rect) -> bool {
        let (left, right) = (area.left, area.right);
        let mut top = area.top;
        while top < area.bottom {
            let height = (area.bottom - top).min(512);
            let width = right - left;
            let capacity = (
                (width as u32).max(self.capacity.0),
                (height as u32).max(self.capacity.1),
            );
            if capacity != self.capacity {
                if call!(self.target.0, 10, (capacity.0 => u32, capacity.1 => u32) -> i32) < 0 {
                    return false;
                }
                self.capacity = capacity;
            }
            let memory = call!(self.target.0, 4, () -> Hdc);
            if BitBlt(memory, 0, 0, width, height, dc, left, top, 0x00cc0020) == 0 {
                return false;
            }
            let hr = call!(layout.object.0, 58, (ptr::null_mut() => *mut c_void, self.renderer.0 => Handle,
                (x - left) as f32 => f32, (y - top) as f32 => f32) -> i32);
            if hr < 0 || BitBlt(dc, left, top, width, height, memory, 0, 0, 0x00cc0020) == 0 {
                return false;
            }
            top += height;
        }
        true
    }
    /// Lay out one colour cluster on its own, so a sentence of Latin text keeps
    /// its cheap GDI path and only the glyph that needs colour is shaped.
    pub unsafe fn cluster(&mut self, text: &[u16], pixels: i32) -> Result<Layout, i32> {
        let index = if let Some(index) = self.formats.iter().position(|(size, _)| *size == pixels) {
            index
        } else {
            let mut raw = NULL;
            let hr = call!(self.factory.0, 15, (wide("Segoe UI Emoji").as_ptr() => *const u16, NULL => Handle,
                400 => u32, 0 => u32, 5 => u32, pixels as f32 => f32, wide("en-us").as_ptr() => *const u16, &mut raw => *mut Handle) -> i32);
            let format = checked(hr, raw)?;
            call!(format.0, 5, (1 => u32) -> i32); // No wrap: a cluster is indivisible.
            self.formats.push((pixels, format));
            self.formats.len() - 1
        };
        let mut raw = NULL;
        let hr = call!(self.factory.0, 18, (text.as_ptr() => *const u16, text.len() as u32 => u32,
            self.formats[index].1.0 => Handle, 65536.0 => f32, 4096.0 => f32, &mut raw => *mut Handle) -> i32);
        let object = checked(hr, raw)?;
        let mut metrics = Metrics::default();
        ok(call!(object.0, 60, (&mut metrics => *mut Metrics) -> i32))?;
        let mut line = LineMetrics::default();
        let mut count = 0;
        ok(
            call!(object.0, 59, (&mut line => *mut LineMetrics, 1 => u32, &mut count => *mut u32) -> i32),
        )?;
        Ok(Layout {
            object,
            width: metrics.trailing_width.ceil() as i32,
            natural: metrics.width.ceil() as i32,
            height: metrics.height.ceil() as i32,
            ascent: line.baseline.ceil() as i32,
        })
    }
    /// Draw one colour cluster at a baseline inside a line of GDI text. The
    /// device context's own pixels are copied first, so antialiasing blends
    /// with whatever background the row already has.
    pub unsafe fn draw_cluster(
        &mut self,
        layout: &Layout,
        dc: Hdc,
        x: i32,
        baseline: i32,
        foreground: u32,
    ) -> bool {
        let width = (layout.width + 4).max(1) as u32;
        let height = (layout.height + 4).max(1) as u32;
        if width > self.capacity.0 || height > self.capacity.1 {
            let capacity = (width.max(self.capacity.0), height.max(self.capacity.1));
            if call!(self.target.0, 10, (capacity.0 => u32, capacity.1 => u32) -> i32) < 0 {
                return false;
            }
            self.capacity = capacity;
        }
        let memory = call!(self.target.0, 4, () -> Hdc);
        let top = baseline - layout.ascent;
        BitBlt(
            memory,
            0,
            0,
            width as i32,
            height as i32,
            dc,
            x - 2,
            top - 2,
            0x00cc0020,
        );
        let renderer = &*(self.renderer.0 as *const Renderer);
        renderer.foreground.set(foreground);
        let hr = call!(layout.object.0, 58, (ptr::null_mut() => *mut c_void, self.renderer.0 => Handle, 2.0 => f32, 2.0 => f32) -> i32);
        if hr < 0 {
            return false;
        }
        BitBlt(
            dc,
            x - 2,
            top - 2,
            width as i32,
            height as i32,
            memory,
            0,
            0,
            0x00cc0020,
        ) != 0
    }
}

#[repr(C)]
struct Renderer {
    vtable: *const RendererVtable,
    refs: Cell<u32>,
    factory: Com,
    target: Com,
    params: Com,
    foreground: Cell<u32>,
    palette: Cell<Palette>,
    color_layers: Cell<u32>,
    // Non-null while a Direct2D target is bound; otherwise the GDI bitmap
    // interop target below is used.
    d2d: Cell<Handle>,
    brush: Cell<Handle>,
}
#[repr(C)]
struct RendererVtable {
    query: unsafe extern "system" fn(*mut Renderer, *const Guid, *mut Handle) -> i32,
    add: unsafe extern "system" fn(*mut Renderer) -> u32,
    release: unsafe extern "system" fn(*mut Renderer) -> u32,
    snapping: unsafe extern "system" fn(*mut Renderer, Handle, *mut i32) -> i32,
    transform: unsafe extern "system" fn(*mut Renderer, Handle, *mut [f32; 6]) -> i32,
    pixels: unsafe extern "system" fn(*mut Renderer, Handle, *mut f32) -> i32,
    glyphs: unsafe extern "system" fn(
        *mut Renderer,
        Handle,
        f32,
        f32,
        u32,
        *const GlyphRun,
        *const c_void,
        Handle,
    ) -> i32,
    underline:
        unsafe extern "system" fn(*mut Renderer, Handle, f32, f32, *const c_void, Handle) -> i32,
    strike:
        unsafe extern "system" fn(*mut Renderer, Handle, f32, f32, *const c_void, Handle) -> i32,
    inline:
        unsafe extern "system" fn(*mut Renderer, Handle, f32, f32, Handle, i32, i32, Handle) -> i32,
}
static VTABLE: RendererVtable = RendererVtable {
    query,
    add,
    release,
    snapping,
    transform,
    pixels,
    glyphs,
    underline,
    strike,
    inline: inline_object,
};
unsafe extern "system" fn query(this: *mut Renderer, iid: *const Guid, out: *mut Handle) -> i32 {
    if out.is_null() || iid.is_null() {
        return 0x80004003u32 as i32;
    }
    *out = NULL;
    if *iid == UNKNOWN || *iid == RENDERER || *iid == SNAPPING {
        *out = this.cast();
        add(this);
        0
    } else {
        0x80004002u32 as i32
    }
}
unsafe extern "system" fn add(this: *mut Renderer) -> u32 {
    let n = (*this).refs.get() + 1;
    (*this).refs.set(n);
    n
}
unsafe extern "system" fn release(this: *mut Renderer) -> u32 {
    let n = (*this).refs.get() - 1;
    (*this).refs.set(n);
    if n == 0 {
        drop(Box::from_raw(this));
    }
    n
}
unsafe extern "system" fn snapping(_: *mut Renderer, _: Handle, out: *mut i32) -> i32 {
    *out = 0;
    0
}
unsafe extern "system" fn transform(_: *mut Renderer, _: Handle, out: *mut [f32; 6]) -> i32 {
    *out = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    0
}
unsafe extern "system" fn pixels(_: *mut Renderer, _: Handle, out: *mut f32) -> i32 {
    *out = 1.0;
    0
}
// Effects carry semantic color roles so theme changes need no text relayout.
#[repr(C)]
struct Effect {
    vtable: *const EffectVtable,
    refs: Cell<u32>,
    style: u8,
}
#[repr(C)]
struct EffectVtable {
    query: unsafe extern "system" fn(*mut Effect, *const Guid, *mut Handle) -> i32,
    add: unsafe extern "system" fn(*mut Effect) -> u32,
    release: unsafe extern "system" fn(*mut Effect) -> u32,
}
static EFFECT_VTABLE: EffectVtable = EffectVtable {
    query: effect_query,
    add: effect_add,
    release: effect_release,
};
fn effect(style: u8) -> Com {
    Com(Box::into_raw(Box::new(Effect {
        vtable: &EFFECT_VTABLE,
        refs: Cell::new(1),
        style,
    })) as Handle)
}
unsafe extern "system" fn effect_query(
    this: *mut Effect,
    iid: *const Guid,
    out: *mut Handle,
) -> i32 {
    if iid.is_null() || out.is_null() {
        return 0x80004003u32 as i32;
    }
    *out = NULL;
    if *iid != UNKNOWN {
        return 0x80004002u32 as i32;
    }
    *out = this.cast();
    effect_add(this);
    0
}
unsafe extern "system" fn effect_add(this: *mut Effect) -> u32 {
    let refs = (*this).refs.get() + 1;
    (*this).refs.set(refs);
    refs
}
unsafe extern "system" fn effect_release(this: *mut Effect) -> u32 {
    let refs = (*this).refs.get() - 1;
    (*this).refs.set(refs);
    if refs == 0 {
        drop(Box::from_raw(this));
    }
    refs
}
unsafe fn foreground(renderer: &Renderer, effect: Handle) -> u32 {
    match if effect.is_null() {
        0
    } else {
        (*(effect as *const Effect)).style
    } {
        Style::CODE => renderer.palette.get().code_foreground,
        Style::LINK => renderer.palette.get().accent,
        _ => renderer.foreground.get(),
    }
}
unsafe fn rectangle(dc: Hdc, rect: Rect, color: u32) {
    SetDCBrushColor(dc, color);
    FillRect(dc, &rect, GetStockObject(18));
}
/// Fill through whichever target is currently bound.
unsafe fn fill(r: &Renderer, rect: Rect, color: u32) {
    let target = r.d2d.get();
    if target.is_null() {
        return rectangle(call!(r.target.0, 4, () -> Hdc), rect, color);
    }
    call!(r.brush.get(), 8, (&ColorF::new(color) => *const ColorF) -> ());
    let rect = RectF {
        left: rect.left as f32,
        top: rect.top as f32,
        right: rect.right as f32,
        bottom: rect.bottom as f32,
    };
    call!(target, 17, (&rect => *const RectF, r.brush.get() => Handle) -> ());
}
unsafe fn decoration(
    this: *mut Renderer,
    x: f32,
    y: f32,
    data: *const c_void,
    effect: Handle,
    underline: bool,
) -> i32 {
    // Both SDK records begin with width, thickness and offset. Underline has
    // one extra float (runHeight) before readingDirection.
    let floats = data as *const f32;
    let width = *floats;
    let thickness = *floats.add(1);
    let offset = *floats.add(2);
    let direction = *((data as *const u32).add(if underline { 4 } else { 3 }));
    let left = if direction == 1 { x - width } else { x };
    let r = &*this;
    fill(
        r,
        Rect {
            left: left.floor() as i32,
            right: (left + width).ceil() as i32,
            top: (y + offset).floor() as i32,
            bottom: (y + offset).floor() as i32 + (thickness.ceil() as i32).max(1),
        },
        foreground(r, effect),
    );
    0
}
unsafe extern "system" fn underline(
    this: *mut Renderer,
    _: Handle,
    x: f32,
    y: f32,
    data: *const c_void,
    effect: Handle,
) -> i32 {
    decoration(this, x, y, data, effect, true)
}
unsafe extern "system" fn strike(
    this: *mut Renderer,
    _: Handle,
    x: f32,
    y: f32,
    data: *const c_void,
    effect: Handle,
) -> i32 {
    decoration(this, x, y, data, effect, false)
}
unsafe extern "system" fn inline_object(
    _: *mut Renderer,
    _: Handle,
    _: f32,
    _: f32,
    _: Handle,
    _: i32,
    _: i32,
    _: Handle,
) -> i32 {
    0
}
unsafe extern "system" fn glyphs(
    this: *mut Renderer,
    _: Handle,
    x: f32,
    y: f32,
    mode: u32,
    run: *const GlyphRun,
    description: *const c_void,
    effect: Handle,
) -> i32 {
    let renderer = &*this;
    let foreground = foreground(renderer, effect);
    if !effect.is_null() && (*(effect as *const Effect)).style == Style::CODE {
        let run = &*run;
        let width: f32 = if run.count == 0 {
            0.0
        } else {
            std::slice::from_raw_parts(run.advances, run.count as usize)
                .iter()
                .sum()
        };
        // DWRITE_FONT_METRICS consists of ten 16-bit fields.
        let mut metrics = [0u16; 10];
        call!(run.face, 8, (metrics.as_mut_ptr() => *mut u16) -> ());
        let scale = run.size / metrics[0].max(1) as f32;
        let left = if run.bidi & 1 != 0 { x - width } else { x };
        fill(
            renderer,
            Rect {
                left: left.floor() as i32,
                right: (left + width).ceil() as i32,
                top: (y - metrics[1] as f32 * scale).floor() as i32,
                bottom: (y + metrics[2] as f32 * scale).ceil() as i32,
            },
            renderer.palette.get().code_background,
        );
    }
    let mut raw = NULL;
    let hr = call!(renderer.factory.0, 28, (x => f32, y => f32, run => *const GlyphRun,
        description => *const c_void, mode => u32, ptr::null() => *const c_void, 0 => u32, &mut raw => *mut Handle) -> i32);
    if hr == 0x8898500cu32 as i32 {
        // DWRITE_E_NOCOLOR: preserve monochrome fallback.
        return draw_run(renderer, x, y, mode, run, foreground);
    }
    let enumerator = match checked(hr, raw) {
        Ok(e) => e,
        Err(hr) => return hr,
    };
    loop {
        let mut more = 0;
        let hr = call!(enumerator.0, 3, (&mut more => *mut i32) -> i32);
        if hr < 0 || more == 0 {
            return hr;
        }
        let mut color: *const ColorRun = ptr::null();
        let hr = call!(enumerator.0, 4, (&mut color => *mut *const ColorRun) -> i32);
        if hr < 0 {
            return hr;
        }
        let color = &*color;
        let rgb = if color.palette == 0xffff {
            foreground
        } else {
            renderer
                .color_layers
                .set(renderer.color_layers.get().saturating_add(1));
            let channel = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
            channel(color.color[0])
                | (channel(color.color[1]) << 8)
                | (channel(color.color[2]) << 16)
        };
        let hr = draw_run(renderer, color.x, color.y, mode, &color.glyphs, rgb);
        if hr < 0 {
            return hr;
        }
    }
}
unsafe fn draw_run(
    r: &Renderer,
    x: f32,
    y: f32,
    mode: u32,
    run: *const GlyphRun,
    color: u32,
) -> i32 {
    let target = r.d2d.get();
    if target.is_null() {
        return call!(r.target.0, 3, (x => f32, y => f32, mode => u32, run => *const GlyphRun,
            r.params.0 => Handle, color => u32, ptr::null_mut() => *mut Rect) -> i32);
    }
    call!(r.brush.get(), 8, (&ColorF::new(color) => *const ColorF) -> ());
    call!(target, 29, (PointF { x, y } => PointF, run => *const GlyphRun,
        r.brush.get() => Handle, mode => u32) -> ());
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipped_paragraph_pixels_match_full_paint_and_bitmap_stays_small() {
        unsafe {
            let surface = Buffer::new(NULL, 600, 256).expect("an off-screen surface");
            let dc = surface.dc;
            let mut engine = Engine::new().unwrap();
            let direct2d = engine.d2d.as_ref().map(|_| ());
            let text = Rc::new(RichText::new(&crate::markdown::inline(
                &"العَرَبِيَّةُ **نص** हिन्दी café 😀 `code` [رابط](url) ~~gone~~ ".repeat(200),
            )));
            let layout = engine.paragraph(&text, 20, 550, false).unwrap();
            assert!(layout.height > 1000);
            for palette in [Palette::dark(), Palette::light()] {
                let full = Rect {
                    left: 0,
                    top: 0,
                    right: 600,
                    bottom: 256,
                };
                rectangle(dc, full, palette.background);
                assert!(engine.draw_paragraph(
                    &layout,
                    dc,
                    20,
                    -180,
                    full,
                    palette,
                    palette.foreground,
                    palette.background,
                ));
                let snapshot = surface.pixels().to_vec();
                let background = crate::theme::rgb(palette.background);
                assert!(
                    snapshot
                        .iter()
                        .filter(|&&p| p & 0xffffff != background)
                        .count()
                        > 1000
                );
                let accent = crate::theme::rgb(palette.accent);
                assert!(
                    snapshot.iter().any(|&p| p & 0xffffff == accent),
                    "Link underline must use the active palette"
                );
                assert!(
                    snapshot
                        .iter()
                        .filter(|&&p| ((p >> 16) & 255) > 150
                            && ((p >> 8) & 255) > 90
                            && (p & 255) < 100)
                        .count()
                        > 20,
                    "Mixed paragraphs must retain emoji color"
                );
                rectangle(dc, full, palette.background);
                let clip = Rect {
                    left: 40,
                    top: 50,
                    right: 540,
                    bottom: 220,
                };
                assert!(engine.draw_paragraph(
                    &layout,
                    dc,
                    20,
                    -180,
                    clip,
                    palette,
                    palette.foreground,
                    palette.background,
                ));
                let partial = surface.pixels();
                for y in 0..256 {
                    for x in 0..600 {
                        let expected = if (40..540).contains(&x) && (50..220).contains(&y) {
                            snapshot[y * 600 + x] & 0xffffff
                        } else {
                            background
                        };
                        assert_eq!(
                            partial[y * 600 + x] & 0xffffff,
                            expected,
                            "Paint clip mismatch at {x},{y}"
                        );
                    }
                }
                assert!(engine.capacity.0 <= 600 && engine.capacity.1 <= 512);
                assert!(
                    direct2d.is_none() || engine.d2d.is_some(),
                    "Direct2D must keep rendering once it is available"
                );
            }
        }
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Cluster {
        width: f32,
        length: u16,
        flags: u16,
    }
    #[test]
    fn multilingual_wrapping_preserves_native_clusters_and_bidi() {
        unsafe {
            let mut engine = Engine::new().unwrap();
            for text in [
                "العَرَبِيَّةُ **لغة جميلة** English 123 😀",
                "שָׁלוֹם **עולם** English 123",
                "हिन्दी क्षि त्रि ज्ञ श्र कि की",
                "বাংলা ক্ষি কি কী",
                "தமிழ் கி கை கொ",
                "ภาษาไทยมีสระและวรรณยุกต์",
                "ខ្មែរ សួស្តី",
                "မြန်မာ မင်္ဂလာပါ",
                "cafe\u{301} cafe\u{301}",
                "한글 か\u{3099}",
            ] {
                let rich = Rc::new(RichText::new(&crate::markdown::inline(text)));
                let mut last_height = i32::MAX;
                for width in [1, 60, 180, 600] {
                    let layout = engine.paragraph(&rich, 20, width, false).unwrap();
                    assert!(
                        layout.height > 0 && layout.height <= last_height,
                        "{text}, width {width}"
                    );
                    last_height = layout.height;
                    let mut clusters = vec![Cluster::default(); rich.text.len()];
                    let mut count = 0;
                    assert!(
                        call!(layout.object.0, 62, (clusters.as_mut_ptr() => *mut Cluster, clusters.len() as u32 => u32, &mut count => *mut u32) -> i32)
                            >= 0
                    );
                    let mut position = 0;
                    let boundaries: Vec<_> = clusters[..count as usize]
                        .iter()
                        .map(|cluster| {
                            position += cluster.length as u32;
                            position
                        })
                        .collect();
                    assert_eq!(position as usize, rich.text.len());
                    let mut lines: Vec<LineMetrics> = (0..rich.text.len())
                        .map(|_| LineMetrics::default())
                        .collect();
                    assert!(
                        call!(layout.object.0, 59, (lines.as_mut_ptr() => *mut LineMetrics, lines.len() as u32 => u32, &mut count => *mut u32) -> i32)
                            >= 0
                    );
                    position = 0;
                    for line in &lines[..count as usize] {
                        position += line.length;
                        assert!(
                            boundaries.contains(&position),
                            "Line split a cluster: {text}"
                        );
                    }
                    assert_eq!(position as usize, rich.text.len());
                    let mut metrics = Metrics::default();
                    assert!(call!(layout.object.0, 60, (&mut metrics => *mut Metrics) -> i32) >= 0);
                    if rich.rtl && width == 600 {
                        assert!(
                            metrics.left > 0.0,
                            "RTL paragraph must start at the right edge"
                        );
                        assert!(metrics.bidi >= 2, "Mixed bidi levels must be retained");
                    }
                }
            }
        }
    }
    #[test]
    fn direct2d_is_the_pipeline_that_actually_draws() {
        unsafe {
            let engine = Engine::new().unwrap();
            assert!(
                engine.d2d.is_some(),
                "Direct2D device-context target must be available on this system"
            );
        }
    }
    #[test]
    fn paragraph_cache_is_bounded_and_font_sensitive() {
        unsafe {
            let mut engine = Engine::new().unwrap();
            let text = Rc::new(RichText::new(&crate::markdown::inline("हिन्दी **पाठ**")));
            let first = engine.paragraph(&text, 17, 400, false).unwrap();
            assert!(Rc::ptr_eq(
                &first,
                &engine.paragraph(&text, 17, 400, false).unwrap()
            ));
            assert!(!Rc::ptr_eq(
                &first,
                &engine.paragraph(&text, 25, 400, false).unwrap()
            ));
            assert!(!Rc::ptr_eq(
                &first,
                &engine.paragraph(&text, 17, 400, true).unwrap()
            ));
            for width in 1..100 {
                engine.paragraph(&text, 17, width, false).unwrap();
            }
            assert_eq!(engine.rich.len(), 64);
            engine.clear_paragraphs();
            assert!(engine.rich.is_empty());
        }
    }
}
