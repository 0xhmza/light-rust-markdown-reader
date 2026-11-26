//! The window: rmdv-inspired chrome, multi-tab reading, heading outline,
//! three-pipeline layout and painting, text selection/copy, clickable links,
//! and settings.
use crate::{
    clipboard,
    dwrite::{self, RichText},
    links::{self, LinkAction},
    markdown::{self, Kind, Span, Style},
    render::{self, Canvas, Pipe},
    theme::{Palette, Themes},
    win32::*,
};
use std::time::Instant;
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::OsString,
    ops::Range,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    ptr,
    rc::Rc,
};

const OPEN: usize = 100;
const RELOAD: usize = 101;
const EXIT: usize = 102;
const SETTINGS: usize = 103;
const RELOAD_THEME: usize = 104;
/// The zoom commands, in the order the floating control lays them out: the
/// command id, what it does, the glyph on the control, the menu label. One
/// table, so a cell can never post the command belonging to its neighbour.
const ZOOM: [(usize, Hit, &str, &str); 3] = [
    (105, Hit::ZoomOut, "\u{2212}", "Zoom &out\tCtrl+-"),
    (106, Hit::ZoomReset, "%", "&Reset zoom\tCtrl+0"),
    (107, Hit::ZoomIn, "+", "Zoom &in\tCtrl++"),
];
const OPEN_NEW_TAB: usize = 108;
const CLOSE_TAB: usize = 109;
const TOGGLE_OUTLINE: usize = 110;
// Context menu commands.
const CTX_COPY_PLAIN: usize = 120;
const CTX_COPY_MD: usize = 121;
const CTX_SELECT_ALL: usize = 122;
const CTX_OPEN_LINK: usize = 123;
const CTX_COPY_LINK: usize = 124;

/// One command per palette, so the Options menu lists whatever the INI defines.
const PALETTE: usize = 200;
const PALETTES: usize = 64;
// Settings window controls. PIPE marks "automatic"; PIPE + 1 + n pins a pipeline.
// OK and Cancel keep the standard dialog ids, so Enter and Escape work.
const OK: usize = 1;
const CANCEL: usize = 2;
const PIPE: usize = 310;
const NOTE: usize = 318;
const SPREAD: usize = 319;
const CHOICE: usize = 320;
const SMOOTH: usize = 321;
const NAME: usize = 322;
const ADD: usize = 323;
const FLOAT: usize = 324;
const DEFAULT_READER: usize = 325;
const TEXT_SIZE: usize = 326;
const DEFAULT_STATUS: usize = 327;
const OUTLINE_OPT: usize = 328;

/// How quickly a smooth scroll closes the distance, as the time constant of an
/// exponential ease-out. The view covers 63% of what is left in this long and is
/// within a pixel of a 300 px scroll after about a third of a second.
const EASE: f64 = 0.045;
/// The fastest a scroll may travel, in screens per second.
const SPEED: f64 = 5.0;
/// A scroll further than this many screens is not worth watching.
const FAR: i32 = 4;
/// One owner-drawn swatch per palette role.
const SWATCH: usize = 330;

/// rmdv's centered reading column cap at 100% zoom, 96 DPI.
const READING_MAX_PX: i32 = 832;
/// Default and bounds for the outline sidebar width in logical pixels.
const OUTLINE_DEFAULT_W: i32 = 230;
const OUTLINE_MIN_W: i32 = 150;
const OUTLINE_MAX_W: i32 = 420;
/// Sentinel value for `Run::link` meaning "not a link".
const NO_LINK: u32 = u32::MAX;

/// Carry fractional pixels across fine wheel events, including reversals.
fn wheel_pixels(delta: i32, amount: i32, remainder: &mut i32) -> i32 {
    let movement = delta.saturating_mul(amount).saturating_add(*remainder);
    *remainder = movement % 120;
    movement / 120
}

/// Single-pass line and word count matching `(s.lines().count(), s.split_whitespace().count())`.
fn count_lines_and_words(source: &str) -> (usize, usize) {
    if source.is_empty() {
        return (0, 0);
    }
    let bytes = source.as_bytes();
    let mut lines = 0usize;
    let mut words = 0usize;
    let mut in_word = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            if b == b'\n' {
                lines += 1;
                in_word = false;
            } else if matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c) {
                in_word = false;
            } else if !in_word {
                words += 1;
                in_word = true;
            }
            i += 1;
        } else {
            let ch = source[i..].chars().next().unwrap();
            if ch.is_whitespace() {
                in_word = false;
            } else if !in_word {
                words += 1;
                in_word = true;
            }
            i += ch.len_utf8();
        }
    }
    if bytes[bytes.len() - 1] != b'\n' {
        lines += 1;
    }
    (lines, words)
}

/// Append UTF-8 `text` as UTF-16 code units into `pool`, returning `(start, len)`.
/// Uses an exact-capacity ASCII fast path with zero per-run heap allocations.
fn push_utf16(pool: &mut Vec<u16>, text: &str) -> (u32, u16) {
    let start = pool.len();
    if text.is_ascii() {
        pool.reserve(text.len());
        for &b in text.as_bytes() {
            pool.push(b as u16);
        }
    } else {
        pool.extend(text.encode_utf16());
    }
    let len = (pool.len() - start).min(u16::MAX as usize);
    (start as u32, len as u16)
}

/// Release excess vector capacity back to the OS so potato machines never keep
/// 2× Vec doubling overhead or stale high-water-mark buffers after big files.
fn shrink_oversized<T>(v: &mut Vec<T>) {
    if v.capacity() > v.len() + v.len() / 4 + 64 {
        v.shrink_to_fit();
    }
}

struct Font {
    handle: Handle,
    key: (u8, u8),
    height: i32,
    ascent: i32,
    size: i32,
    /// Exact per-character advance widths for ASCII `0..128` via `GetCharWidth32W`.
    ascii: Option<[i32; 128]>,
    /// Lazy direct-mapped cache for non-ASCII BMP glyph widths (`0x80..0xD800`).
    bmp: RefCell<Option<Box<[(u16, i32); 512]>>>,
    /// The GDI+ twin of `handle`, built on first GDI+ use. `Some(NULL)` records
    /// a font GDI+ cannot represent, so the attempt is made only once.
    plus: Cell<Option<Handle>>,
}
impl Font {
    /// Measure how many UTF-16 units of `text` fit inside `max_width` and return
    /// `(fit_count, total_width_of_text)` using cached per-glyph widths when
    /// possible, falling back to `GetTextExtentExPointW` for surrogates/control chars.
    unsafe fn measure_fit(
        &self,
        dc: Hdc,
        selected_font: &mut usize,
        font_idx: usize,
        text: &[u16],
        max_width: i32,
    ) -> (usize, i32) {
        if let Some(ascii) = &self.ascii {
            // Fast path 1: pure printable ASCII (`0x20..=0x7E`), 0 GDI calls, pixel-exact.
            let mut total = 0i32;
            let mut fit = text.len();
            let mut fast = true;
            for (i, &u) in text.iter().enumerate() {
                if (0x20..=0x7e).contains(&u) {
                    let w = ascii[u as usize];
                    total += w;
                    if total > max_width && fit == text.len() {
                        fit = i;
                    }
                } else {
                    fast = false;
                    break;
                }
            }
            if fast {
                return (fit, total);
            }
            // Fast path 2: non-surrogate printable BMP (`>= 0x20 && < 0xD800`) via
            // direct-mapped per-codepoint cache.
            if text.iter().all(|&u| u >= 0x20 && u < 0xd800) {
                let mut total = 0i32;
                let mut fit = text.len();
                let mut cache_ref = self.bmp.borrow_mut();
                let cache = cache_ref.get_or_insert_with(|| Box::new([(0u16, 0i32); 512]));
                for (i, &u) in text.iter().enumerate() {
                    let w = if u < 128 {
                        ascii[u as usize]
                    } else {
                        let slot = (u as usize) & 511;
                        if cache[slot].0 == u {
                            cache[slot].1
                        } else {
                            if *selected_font != font_idx {
                                SelectObject(dc, self.handle);
                                *selected_font = font_idx;
                            }
                            let mut sz = Size::default();
                            GetTextExtentPoint32W(dc, &u, 1, &mut sz);
                            cache[slot] = (u, sz.cx);
                            sz.cx
                        }
                    };
                    total += w;
                    if total > max_width && fit == text.len() {
                        fit = i;
                    }
                }
                return (fit, total);
            }
        }
        if *selected_font != font_idx {
            SelectObject(dc, self.handle);
            *selected_font = font_idx;
        }
        let mut fit = 0i32;
        let mut size = Size::default();
        GetTextExtentExPointW(
            dc,
            text.as_ptr(),
            text.len() as i32,
            max_width,
            &mut fit,
            ptr::null_mut(),
            &mut size,
        );
        ((fit.max(0) as usize).min(text.len()), size.cx)
    }

    /// Measure the exact total width of `text`.
    unsafe fn measure_width(
        &self,
        dc: Hdc,
        selected_font: &mut usize,
        font_idx: usize,
        text: &[u16],
    ) -> i32 {
        if let Some(ascii) = &self.ascii {
            let mut total = 0i32;
            let mut fast = true;
            for &u in text {
                if (0x20..=0x7e).contains(&u) {
                    total += ascii[u as usize];
                } else {
                    fast = false;
                    break;
                }
            }
            if fast {
                return total;
            }
            if text.iter().all(|&u| u >= 0x20 && u < 0xd800) {
                let mut total = 0i32;
                let mut cache_ref = self.bmp.borrow_mut();
                let cache = cache_ref.get_or_insert_with(|| Box::new([(0u16, 0i32); 512]));
                for &u in text {
                    let w = if u < 128 {
                        ascii[u as usize]
                    } else {
                        let slot = (u as usize) & 511;
                        if cache[slot].0 == u {
                            cache[slot].1
                        } else {
                            if *selected_font != font_idx {
                                SelectObject(dc, self.handle);
                                *selected_font = font_idx;
                            }
                            let mut sz = Size::default();
                            GetTextExtentPoint32W(dc, &u, 1, &mut sz);
                            cache[slot] = (u, sz.cx);
                            sz.cx
                        }
                    };
                    total += w;
                }
                return total;
            }
        }
        if *selected_font != font_idx {
            SelectObject(dc, self.handle);
            *selected_font = font_idx;
        }
        let mut size = Size::default();
        GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size);
        size.cx
    }
}
impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.handle);
            if let Some(font) = self.plus.get().filter(|font| !font.is_null()) {
                render::delete_font(font);
            }
        }
    }
}
const NO_SHAPED: u32 = u32::MAX;
const NO_CELL: u16 = u16::MAX;
const UNMEASURED: i32 = -1;
const RUN_EMOJI: u8 = 1;

struct Run {
    /// Start offset into `Tab::text_pool`.
    text_start: u32,
    /// Index into `Tab::links`, or `NO_LINK` when not a link.
    link: u32,
    /// Cached GDI width in pixels, or `UNMEASURED` (`-1`).
    width: Cell<i32>,
    /// Length in UTF-16 code units (capped by `LONGEST_BLOCK` <= 8192).
    text_len: u16,
    style: Style,
    /// Bit 0 = `RUN_EMOJI` (colour cluster shaped by DirectWrite on demand).
    flags: u8,
}
impl Run {
    #[inline]
    fn is_emoji(&self) -> bool {
        self.flags & RUN_EMOJI != 0
    }
    #[inline]
    fn cached_width(&self) -> Option<i32> {
        let w = self.width.get();
        (w >= 0).then_some(w)
    }
    #[inline]
    fn set_width(&self, w: i32) {
        self.width.set(w.max(0));
    }
}
/// One flowable unit of text: a whole block, or a single table cell.
#[derive(Clone, Copy)]
struct Piece {
    runs_start: u32,
    runs_len: u16,
    pipe: Pipe,
    align: u8,
    /// Index into `Tab::shaped_pieces`, or `NO_SHAPED`.
    shaped: u32,
}
impl Piece {
    #[inline]
    fn run_range(&self) -> Range<usize> {
        let s = self.runs_start as usize;
        s..s + self.runs_len as usize
    }
}
struct Block {
    raw_start: u32,
    raw_end: u32,
    body: Piece,
    cells_start: u32,
    cells_len: u16,
    kind: Kind,
}
impl Block {
    #[inline]
    fn raw_range(&self) -> Range<usize> {
        self.raw_start as usize..self.raw_end as usize
    }
}
/// One heading entry shown in the Outline sidebar and targeted by `#anchor` links.
#[derive(Clone, Debug)]
struct Heading {
    level: u8,
    title: String,
    slug: String,
    block: usize,
    y: i32,
}
/// Position inside a laid-out document for text selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SelPos {
    row: usize,
    col: usize,
}
/// The parts of the footer that answer a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    ZoomOut,
    ZoomReset,
    ZoomIn,
    Engine,
}
/// Interactive hit targets in the top tab bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabHit {
    ToggleOutline,
    Select(usize),
    Close(usize),
    NewTab,
    CycleTheme,
    OpenSettings,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Deco {
    None,
    Code,
    Quote,
    Rule,
    Grid,
}
struct Fragment {
    run: u32,
    x: i32,
    width: i32,
    range_start: u16,
    range_len: u16,
    font: u16,
}
impl Fragment {
    #[inline]
    fn range(&self) -> Range<usize> {
        let s = self.range_start as usize;
        s..s + self.range_len as usize
    }
    #[inline]
    fn len(&self) -> usize {
        self.range_len as usize
    }
}
struct ShapedRow {
    text: Rc<RichText>,
    x: i32,
    width: i32,
    pixels: i32,
    bold: bool,
}
struct Row {
    y: i32,
    height: i32,
    ascent: i32,
    /// Horizontal extent of this row's decoration.
    span: (i32, i32),
    /// Owning block index for selection & copy.
    block: u32,
    /// Start offset into `Tab::parts_pool`.
    parts_start: u32,
    /// Index into `Tab::shaped_rows`, or `NO_SHAPED`.
    shaped: u32,
    /// Number of fragments in `Tab::parts_pool`.
    parts_len: u16,
    /// Table column index, or `NO_CELL`.
    cell: u16,
    deco: Deco,
    pipe: Pipe,
    /// Bit 0 = top edge of a code/table block; bit 1 = bottom edge; bit 2 = table header fill.
    flags: u8,
}
impl Row {
    const EDGE_TOP: u8 = 1;
    const EDGE_BOTTOM: u8 = 2;
    const TABLE_HEADER: u8 = 4;
    const HEADING_RULE: u8 = 8;
}
// Compile-time memory guards: keep hot per-line structs compact for low-RAM machines.
const _: () = assert!(std::mem::size_of::<Run>() == 16);
const _: () = assert!(std::mem::size_of::<Piece>() == 12);
const _: () = assert!(std::mem::size_of::<Block>() == 28);
const _: () = assert!(std::mem::size_of::<Fragment>() == 20);
const _: () = assert!(std::mem::size_of::<Row>() == 40);
/// Where a piece may flow and how it is decorated.
#[derive(Clone, Copy)]
struct Frame {
    left: i32,
    right: i32,
    span: (i32, i32),
    heading: u8,
    deco: Deco,
    align: u8,
    block: usize,
    cell: Option<usize>,
    flags: u8,
}
/// What the status bar reports about the open document.
#[derive(Default)]
struct Stats {
    zoom: i32,
    bytes: usize,
    lines: usize,
    words: usize,
    micros: u128,
    blocks: [usize; 8],
    reasons: u8,
    rtl: usize,
    truncated: bool,
}
impl Stats {
    fn breakdown(&self, pinned: Option<Pipe>) -> String {
        let spread = self.spread(pinned);
        let counts: Vec<String> = Pipe::ALL
            .into_iter()
            .filter(|pipe| spread[*pipe as usize] > 0)
            .map(|pipe| format!("{} {}", spread[pipe as usize], pipe.short()))
            .collect();
        if counts.is_empty() {
            return "No document open yet.".to_owned();
        }
        format!("This document: {} blocks.", counts.join(", "))
    }
    fn summary(&self) -> String {
        let size = match self.bytes {
            n if n < 1024 => format!("{n} B"),
            n if n < 1024 * 1024 => format!("{:.1} KiB", n as f64 / 1024.0),
            n => format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0)),
        };
        format!(
            "{size}  ·  {} lines{}  ·  {} words  ·  ready in {:.1} ms",
            self.lines,
            if self.truncated {
                " (only the first million shown)"
            } else {
                ""
            },
            self.words,
            self.micros as f64 / 1000.0
        )
    }
    fn rtl(&self) -> bool {
        self.rtl * 2 > self.blocks.iter().sum::<usize>()
    }
    fn spread(&self, pinned: Option<Pipe>) -> [usize; 3] {
        let mut counts = [0; 3];
        for (reasons, blocks) in self.blocks.iter().enumerate() {
            counts[Pipe::choose(reasons as u8, pinned) as usize] += blocks;
        }
        counts
    }
    fn escalated(&self, pinned: Option<Pipe>) -> usize {
        match pinned {
            Some(pinned) if pinned != Pipe::D2d => self.spread(Some(pinned))[Pipe::D2d as usize],
            _ => 0,
        }
    }
    fn label(&self, pinned: Option<Pipe>) -> String {
        match pinned {
            Some(pinned) => format!("{} (pinned)  \u{25be}", pinned.short()),
            None => {
                let spread = self.spread(None);
                let used: Vec<Pipe> = Pipe::ALL
                    .into_iter()
                    .filter(|pipe| spread[*pipe as usize] > 0)
                    .collect();
                format!(
                    "{}{}  \u{25be}",
                    used.last().copied().unwrap_or(Pipe::Gdi).short(),
                    if used.len() > 1 { " + mixed" } else { "" }
                )
            }
        }
    }
    fn warning(&self, pinned: Option<Pipe>) -> Option<String> {
        let escalated = self.escalated(pinned);
        (escalated > 0).then(|| {
            format!(
                "{escalated} block{} still need{} Direct2D",
                if escalated == 1 { "" } else { "s" },
                if escalated == 1 { "s" } else { "" }
            )
        })
    }
    fn explain(&self, pinned: Option<Pipe>) -> String {
        let spread = self.spread(pinned);
        let mut text = match pinned {
            None => "Chosen automatically: every block gets the cheapest pipeline\nthat can draw it correctly.\n\n"
                .to_owned(),
            Some(pinned) => format!("Pinned to {} in Settings.\n\n", pinned.short()),
        };
        for pipe in Pipe::ALL {
            if spread[pipe as usize] > 0 {
                text.push_str(&format!(
                    "  {:>9}   {} blocks\n",
                    pipe.short(),
                    spread[pipe as usize]
                ));
            }
        }
        if let Some(warning) = self.warning(pinned) {
            text.push_str(&format!(
                "\n{warning}: GDI and GDI+ cannot shape those scripts,\nso they would be unreadable any other way.\n"
            ));
        }
        let causes: Vec<&str> = render::CAUSES
            .iter()
            .filter(|(flag, _)| self.reasons & flag != 0)
            .map(|(_, cause)| *cause)
            .collect();
        if causes.is_empty() {
            text.push_str("\nOnly plain Latin text was found, so GDI can draw everything.");
        } else {
            text.push_str("\nDetected in this document:\n");
            for cause in causes {
                text.push_str(&format!("  \u{2022} {cause}\n"));
            }
        }
        text
    }
}

thread_local! {
    /// Set while the viewer is putting the system scrollbar away.
    static ADJUSTING: Cell<bool> = const { Cell::new(false) };
    /// Menu painting reads its font, palette and DPI from here.
    static MENU: Cell<(Handle, Palette, u32)> = Cell::new((NULL, Palette::dark(), 96));
}

/// Paint one owner-drawn menu item, so the menu matches the document.
unsafe fn draw_menu_item(item: &DrawItem) {
    let (font, palette, dpi) = MENU.get();
    let px = |n: i32| ((n as i64 * dpi as i64) / 96) as i32;
    let selected = item.state & 1 != 0; // ODS_SELECTED
    let disabled = item.state & (4 | 2) != 0; // ODS_DISABLED | ODS_GRAYED
    let checked = item.state & 8 != 0; // ODS_CHECKED
    let is_bar = item.item > 0xffff; // Submenu HMENU handles live on the top menu bar.
    let base_bg = if is_bar {
        palette.sidebar()
    } else {
        palette.background
    };
    fill(item.dc, item.rect, base_bg);
    let text = &*(item.data as *const Box<[u16]>);
    if text.is_empty() {
        let middle = (item.rect.top + item.rect.bottom) / 2;
        return fill(
            item.dc,
            Rect {
                left: item.rect.left + px(10),
                right: item.rect.right - px(10),
                top: middle,
                bottom: middle + px(1).max(1),
            },
            palette.rule,
        );
    }
    if selected && !disabled {
        let pill = if is_bar {
            Rect {
                left: item.rect.left + px(2),
                top: item.rect.top + px(2),
                right: item.rect.right - px(2),
                bottom: item.rect.bottom - px(2),
            }
        } else {
            Rect {
                left: item.rect.left + px(3),
                top: item.rect.top + px(1),
                right: item.rect.right - px(3),
                bottom: item.rect.bottom - px(1),
            }
        };
        let pill_color = if is_bar {
            palette.hover(base_bg)
        } else {
            palette.accent
        };
        fill_rounded(item.dc, pill, px(5).max(2), pill_color, pill_color);
    }
    let old = SelectObject(item.dc, font);
    SetBkMode(item.dc, 1);
    let fg = if disabled {
        palette.subtle()
    } else if selected && !is_bar {
        Palette::contrasting(palette.accent)
    } else {
        palette.foreground
    };
    SetTextColor(item.dc, fg);
    if checked && !is_bar {
        let dot = px(6).max(4);
        let cx = item.rect.left + px(10);
        let cy = (item.rect.top + item.rect.bottom - dot) / 2;
        let dot_c = if selected {
            Palette::contrasting(palette.accent)
        } else {
            palette.accent
        };
        fill_rounded(
            item.dc,
            Rect {
                left: cx,
                top: cy,
                right: cx + dot,
                bottom: cy + dot,
            },
            dot,
            dot_c,
            dot_c,
        );
    }
    let left_pad = if is_bar { px(10) } else { px(22) };
    let mut rect = Rect {
        left: item.rect.left + left_pad,
        right: item.rect.right - px(12),
        ..item.rect
    };
    let tab = text.iter().position(|&c| c == 9).unwrap_or(text.len());
    DrawTextW(item.dc, text.as_ptr(), tab as i32, &mut rect, 0x20 | 0x4);
    if tab + 1 < text.len() {
        SetTextColor(
            item.dc,
            if selected && !disabled && !is_bar {
                Palette::contrasting(palette.accent)
            } else {
                palette.muted
            },
        );
        DrawTextW(
            item.dc,
            text[tab + 1..].as_ptr(),
            (text.len() - tab - 1) as i32,
            &mut rect,
            0x20 | 0x4 | 0x2,
        );
    }
    SelectObject(item.dc, old);
}
/// Size an owner-drawn menu item using the font that will draw it.
unsafe fn measure_menu_item(item: &mut MeasureItem) {
    let (font, _, dpi) = MENU.get();
    let px = |n: i32| ((n as i64 * dpi as i64) / 96) as i32;
    let text = &*(item.data as *const Box<[u16]>);
    let dc = GetDC(NULL);
    let old = SelectObject(dc, font);
    let mut size = Size::default();
    GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size);
    SelectObject(dc, old);
    ReleaseDC(NULL, dc);
    if text.is_empty() {
        (item.width, item.height) = (0, px(9) as u32);
    } else {
        let is_bar = item.item > 0xffff;
        let pad_x = if is_bar { px(20) } else { px(36) };
        let gap = if text.contains(&9) { px(34) } else { 0 };
        (item.width, item.height) = ((size.cx + pad_x + gap) as u32, (size.cy + px(12)) as u32);
    }
}

/// Fast non-cryptographic hasher (FxHash) for bounded document-local interning.
#[derive(Default)]
struct FxHasher(u64);
impl std::hash::Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        const K: u64 = 0x517c_c1b7_2722_0a95;
        let mut h = self.0;
        while bytes.len() >= 8 {
            let word = u64::from_ne_bytes(bytes[..8].try_into().unwrap());
            h = (h.rotate_left(5) ^ word).wrapping_mul(K);
            bytes = &bytes[8..];
        }
        if bytes.len() >= 4 {
            let word = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as u64;
            h = (h.rotate_left(5) ^ word).wrapping_mul(K);
            bytes = &bytes[4..];
        }
        for &b in bytes {
            h = (h.rotate_left(5) ^ b as u64).wrapping_mul(K);
        }
        self.0 = h;
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}
type FxHashMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FxHasher>>;

/// Bounded interning shared by every piece of one document.
#[derive(Default)]
struct Intern<'a> {
    runs: FxHashMap<u16, FxHashMap<Cow<'a, str>, u32>>,
    link_runs: FxHashMap<(u16, u32, Cow<'a, str>), u32>,
    links: FxHashMap<Cow<'a, str>, u32>,
    entries: usize,
    rich: FxHashMap<Rc<RichText>, Rc<RichText>>,
}

/// All state owned by a single open document tab.
struct Tab {
    file: Option<PathBuf>,
    title: String,
    source: String,
    text_pool: Vec<u16>,
    runs: Vec<Run>,
    run_order: Vec<u32>,
    emojis: FxHashMap<u32, dwrite::Layout>,
    links: Vec<String>,
    shaped_pieces: Vec<Rc<RichText>>,
    cells_pool: Vec<Piece>,
    blocks: Vec<Block>,
    parts_pool: Vec<Fragment>,
    shaped_rows: Vec<ShapedRow>,
    rows: Vec<Row>,
    headings: Vec<Heading>,
    stats: Stats,
    content_height: i32,
    tallest: i32,
    scroll: i32,
    pos: f64,
    target: i32,
    animating: bool,
    selection: Option<(SelPos, SelPos)>,
    outline_scroll: i32,
    /// Layout inputs when `rows` was last built: `(width, sidebar_w, dpi, zoom, pinned)`.
    layout_sig: (i32, i32, u32, i32, Option<Pipe>),
}
impl Tab {
    fn new(title: &str) -> Self {
        Self {
            file: None,
            title: title.to_owned(),
            source: String::new(),
            text_pool: vec![],
            runs: vec![],
            run_order: vec![],
            emojis: FxHashMap::default(),
            links: vec![],
            shaped_pieces: vec![],
            cells_pool: vec![],
            blocks: vec![],
            parts_pool: vec![],
            shaped_rows: vec![],
            rows: vec![],
            headings: vec![],
            stats: Stats::default(),
            content_height: 0,
            tallest: 0,
            scroll: 0,
            pos: 0.0,
            target: 0,
            animating: false,
            selection: None,
            outline_scroll: 0,
            layout_sig: (0, 0, 0, 0, None),
        }
    }
    #[inline]
    fn run_text(&self, run: usize) -> &[u16] {
        let r = &self.runs[run];
        let s = r.text_start as usize;
        &self.text_pool[s..s + r.text_len as usize]
    }
    #[inline]
    fn block_cells(&self, block: usize) -> &[Piece] {
        let b = &self.blocks[block];
        let s = b.cells_start as usize;
        &self.cells_pool[s..s + b.cells_len as usize]
    }
    #[inline]
    fn piece_shaped(&self, piece: Piece) -> Option<&Rc<RichText>> {
        (piece.shaped != NO_SHAPED).then(|| &self.shaped_pieces[piece.shaped as usize])
    }
    #[inline]
    fn row_parts(&self, row: &Row) -> &[Fragment] {
        let s = row.parts_start as usize;
        &self.parts_pool[s..s + row.parts_len as usize]
    }
    #[inline]
    fn row_shaped(&self, row: &Row) -> Option<&ShapedRow> {
        (row.shaped != NO_SHAPED).then(|| &self.shaped_rows[row.shaped as usize])
    }
    #[inline]
    fn row_text_len(&self, row: &Row) -> usize {
        if let Some(s) = self.row_shaped(row) {
            s.text.text.len()
        } else {
            self.row_parts(row).iter().map(|p| p.len()).sum()
        }
    }
    fn set_document(&mut self, source: &str, zoom: i32, pinned: Option<Pipe>) {
        self.rows.clear();
        self.parts_pool.clear();
        self.shaped_rows.clear();
        self.runs.clear();
        self.emojis.clear();
        self.text_pool.clear();
        self.run_order.clear();
        self.links.clear();
        self.shaped_pieces.clear();
        self.cells_pool.clear();
        self.blocks.clear();
        self.headings.clear();
        self.selection = None;
        self.outline_scroll = 0;
        self.layout_sig = (0, 0, 0, 0, None);
        let started = Instant::now();
        let (lines, words) = count_lines_and_words(source);
        self.stats = Stats {
            zoom,
            bytes: source.len(),
            lines,
            words,
            ..Stats::default()
        };
        let est_blocks = (source.len() / 48).min(65_536);
        self.blocks.reserve(est_blocks);
        self.run_order.reserve(est_blocks);
        self.text_pool.reserve(source.len().min(4 * 1024 * 1024));
        let mut intern = Intern::default();
        self.stats.truncated = markdown::parse_each(source, |kind, spans, cells, raw| {
            let block_idx = self.blocks.len();
            let heading = match kind {
                Kind::Heading(level) => {
                    let title = markdown::plain_text(spans);
                    let slug = markdown::heading_slug(&title);
                    self.headings.push(Heading {
                        level,
                        title,
                        slug,
                        block: block_idx,
                        y: 0,
                    });
                    level
                }
                _ => 0,
            };
            let header = kind == Kind::Row(true);
            let body = self.piece(spans, heading, false, 0, pinned, &mut intern);
            let cells_start = self.cells_pool.len() as u32;
            let mut cells_len = 0u16;
            if !cells.is_empty() {
                for cell in cells.iter_mut() {
                    let p = self.piece(&mut cell.spans, 0, header, cell.align, pinned, &mut intern);
                    self.cells_pool.push(p);
                    cells_len = cells_len.saturating_add(1);
                }
            }
            self.blocks.push(Block {
                raw_start: raw.start as u32,
                raw_end: raw.end as u32,
                body,
                cells_start,
                cells_len,
                kind,
            });
        });
        drop(intern);
        shrink_oversized(&mut self.blocks);
        shrink_oversized(&mut self.cells_pool);
        shrink_oversized(&mut self.runs);
        shrink_oversized(&mut self.text_pool);
        shrink_oversized(&mut self.run_order);
        shrink_oversized(&mut self.links);
        shrink_oversized(&mut self.headings);
        shrink_oversized(&mut self.shaped_pieces);
        self.stats.micros = started.elapsed().as_micros();
        self.scroll = 0;
        self.pos = 0.0;
        self.target = 0;
        self.animating = false;
    }
    fn piece<'a>(
        &mut self,
        spans: &mut Vec<Span<'a>>,
        heading: u8,
        bold: bool,
        align: u8,
        pinned: Option<Pipe>,
        intern: &mut Intern<'a>,
    ) -> Piece {
        if bold {
            for span in spans.iter_mut() {
                span.style.0 |= Style::BOLD;
            }
        }
        let reasons = spans
            .iter()
            .fold(0, |flags, span| flags | Pipe::reasons(&span.text));
        let pipe = Pipe::choose(reasons, pinned);
        if !spans.is_empty() {
            self.stats.reasons |= reasons;
            self.stats.blocks[(reasons & 7) as usize] += 1;
        }
        let start = self.run_order.len() as u32;
        if pipe == Pipe::D2d {
            let text = Rc::new(RichText::with_links(spans, &mut self.links));
            spans.clear();
            if text.rtl() {
                self.stats.reasons |= render::RTL;
                self.stats.rtl += 1;
            }
            // Only share paragraphs without links so per-link URLs never alias.
            let text = if text.links.is_empty() {
                if let Some(shared) = intern.rich.get(&text) {
                    shared.clone()
                } else {
                    if text.text.len() <= 1024 && intern.rich.len() < 4096 {
                        intern.rich.insert(text.clone(), text.clone());
                    }
                    text
                }
            } else {
                text
            };
            let shaped_idx = self.shaped_pieces.len() as u32;
            self.shaped_pieces.push(text);
            return Piece {
                runs_start: start,
                runs_len: 0,
                pipe,
                align,
                shaped: shaped_idx,
            };
        }
        for span in spans.drain(..) {
            let link_id = match span.href {
                Some(href) => {
                    if let Some(&id) = intern.links.get(href.as_ref()) {
                        id
                    } else {
                        let id = self.links.len() as u32;
                        self.links.push(href.as_ref().to_owned());
                        if intern.links.len() < 8192 {
                            intern.links.insert(href, id);
                        }
                        id
                    }
                }
                None => NO_LINK,
            };
            let style = span.style;
            let mut push_run = |text: &str, borrowed: Option<&'a str>, is_emoji: bool| {
                let flags = if is_emoji { RUN_EMOJI } else { 0 };
                let key = style.0 as u16 | ((heading as u16) << 8) | ((is_emoji as u16) << 12);
                let index = if text.len() > 256 {
                    let idx = self.runs.len() as u32;
                    let (text_start, text_len) = push_utf16(&mut self.text_pool, text);
                    self.runs.push(Run {
                        text_start,
                        link: link_id,
                        width: Cell::new(UNMEASURED),
                        text_len,
                        style,
                        flags,
                    });
                    idx
                } else if link_id != NO_LINK {
                    let lookup = match borrowed {
                        Some(b) => Cow::Borrowed(b),
                        None => Cow::Owned(text.to_owned()),
                    };
                    if let Some(&idx) = intern.link_runs.get(&(key, link_id, lookup)) {
                        idx
                    } else {
                        let idx = self.runs.len() as u32;
                        let (text_start, text_len) = push_utf16(&mut self.text_pool, text);
                        self.runs.push(Run {
                            text_start,
                            link: link_id,
                            width: Cell::new(UNMEASURED),
                            text_len,
                            style,
                            flags,
                        });
                        if intern.link_runs.len() < 4096 {
                            let owned_key = match borrowed {
                                Some(b) => Cow::Borrowed(b),
                                None => Cow::Owned(text.to_owned()),
                            };
                            intern.link_runs.insert((key, link_id, owned_key), idx);
                        }
                        idx
                    }
                } else {
                    let group = intern.runs.entry(key).or_default();
                    if let Some(&index) = group.get(text) {
                        index
                    } else {
                        let index = self.runs.len() as u32;
                        let (text_start, text_len) = push_utf16(&mut self.text_pool, text);
                        self.runs.push(Run {
                            text_start,
                            link: NO_LINK,
                            width: Cell::new(UNMEASURED),
                            text_len,
                            style,
                            flags,
                        });
                        if intern.entries < 8192 {
                            let owned_key = match borrowed {
                                Some(b) => Cow::Borrowed(b),
                                None => Cow::Owned(text.to_owned()),
                            };
                            group.insert(owned_key, index);
                            intern.entries += 1;
                        }
                        index
                    }
                };
                self.run_order.push(index);
            };
            match span.text {
                Cow::Borrowed(s) => dwrite::for_each_cluster(s, |text, is_emoji| {
                    push_run(text, Some(text), is_emoji);
                }),
                Cow::Owned(s) => dwrite::for_each_cluster(&s, |text, is_emoji| {
                    push_run(text, None, is_emoji);
                }),
            }
        }
        let runs_len = (self.run_order.len() - start as usize).min(u16::MAX as usize) as u16;
        Piece {
            runs_start: start,
            runs_len,
            pipe,
            align,
            shaped: NO_SHAPED,
        }
    }
    /// Normalized selection range `(start, end)` if non-empty.
    fn normalized_selection(&self) -> Option<(SelPos, SelPos)> {
        let (a, b) = self.selection?;
        if a == b {
            return None;
        }
        Some(if a <= b { (a, b) } else { (b, a) })
    }
    /// Active heading index for the current scroll position (scroll-spy).
    fn active_heading(&self, lead_px: i32) -> Option<usize> {
        if self.headings.is_empty() {
            return None;
        }
        let cutoff = self.scroll.saturating_add(lead_px);
        let pos = self.headings.partition_point(|h| h.y <= cutoff);
        Some(pos.saturating_sub(1))
    }
    /// Extract UTF-16 slice `[col0..col1)` from `row` as a UTF-8 `String`.
    fn row_plain_slice(&self, row_idx: usize, col0: usize, col1: usize) -> String {
        let row = &self.rows[row_idx];
        if col1 <= col0 {
            return String::new();
        }
        if let Some(shaped) = self.row_shaped(row) {
            let end = col1.min(shaped.text.text.len());
            let start = col0.min(end);
            return String::from_utf16_lossy(&shaped.text.text[start..end]);
        }
        let mut units = Vec::with_capacity(col1 - col0);
        let mut cursor = 0usize;
        for part in self.row_parts(row) {
            let len = part.len();
            let part_end = cursor + len;
            if part_end > col0 && cursor < col1 {
                let lo = col0.saturating_sub(cursor);
                let hi = (col1 - cursor).min(len);
                let rtext = self.run_text(part.run as usize);
                let pstart = part.range_start as usize;
                units.extend_from_slice(&rtext[pstart + lo..pstart + hi]);
            }
            cursor = part_end;
            if cursor >= col1 {
                break;
            }
        }
        String::from_utf16_lossy(&units)
    }
    /// Extract selected text as **plain text** (no Markdown markup).
    fn selected_plain(&self) -> String {
        let Some((start, end)) = self.normalized_selection() else {
            return String::new();
        };
        if start.row >= self.rows.len() || end.row >= self.rows.len() {
            return String::new();
        }
        let mut out = String::new();
        let mut prev_block: Option<u32> = None;
        let mut prev_cell = NO_CELL;
        let mut prev_deco = Deco::None;
        for r in start.row..=end.row {
            let row = &self.rows[r];
            let len = self.row_text_len(row);
            if len == 0 {
                continue;
            }
            let c0 = if r == start.row {
                start.col.min(len)
            } else {
                0
            };
            let c1 = if r == end.row { end.col.min(len) } else { len };
            if c1 <= c0 {
                continue;
            }
            let piece = self.row_plain_slice(r, c0, c1);
            if !out.is_empty() {
                if prev_block == Some(row.block) {
                    if prev_cell != row.cell {
                        out.push('\t');
                    } else if prev_deco == Deco::Code {
                        out.push_str("\r\n");
                    } else if !out.ends_with(' ') && !piece.starts_with(' ') {
                        out.push(' ');
                    }
                } else {
                    out.push_str("\r\n");
                }
            }
            out.push_str(&piece);
            prev_block = Some(row.block);
            prev_cell = row.cell;
            prev_deco = row.deco;
        }
        out
    }
    /// Extract Markdown for a partial UTF-16 slice `[col0..col1)` of a single row.
    fn row_markdown_slice(&self, row_idx: usize, col0: usize, col1: usize) -> String {
        let row = &self.rows[row_idx];
        if col1 <= col0 {
            return String::new();
        }
        if row.deco == Deco::Code {
            return self.row_plain_slice(row_idx, col0, col1);
        }
        let mut out = String::new();
        if let Some(shaped) = self.row_shaped(row) {
            let rich = &shaped.text;
            let end = col1.min(rich.text.len()) as u32;
            let start = (col0 as u32).min(end);
            for &(range, style_bits) in &rich.ranges {
                let r_start = range.start;
                let r_end = range.start + range.length;
                if r_end <= start || r_start >= end {
                    continue;
                }
                let lo = start.max(r_start) as usize;
                let hi = end.min(r_end) as usize;
                let text = String::from_utf16_lossy(&rich.text[lo..hi]);
                let href = rich
                    .link_at(lo as u32)
                    .and_then(|id| self.links.get(id as usize))
                    .map(String::as_str);
                out.push_str(&clipboard::format_markdown_span(
                    &text,
                    Style(style_bits),
                    href,
                ));
            }
            return out;
        }
        // Merge contiguous fragments sharing the same style and link so wrapped
        // bold/link phrases aren't split into `**word1** **word2**`.
        let mut cur_style = Style::default();
        let mut cur_link = NO_LINK;
        let mut buf: Vec<u16> = Vec::new();
        let flush =
            |out: &mut String, buf: &mut Vec<u16>, style: Style, link: u32, links: &[String]| {
                if buf.is_empty() {
                    return;
                }
                let text = String::from_utf16_lossy(buf);
                let href = (link != NO_LINK)
                    .then(|| links.get(link as usize).map(String::as_str))
                    .flatten();
                out.push_str(&clipboard::format_markdown_span(&text, style, href));
                buf.clear();
            };
        let mut cursor = 0usize;
        for part in self.row_parts(row) {
            let len = part.len();
            let part_end = cursor + len;
            if part_end > col0 && cursor < col1 {
                let lo = col0.saturating_sub(cursor);
                let hi = (col1 - cursor).min(len);
                let run = &self.runs[part.run as usize];
                if !buf.is_empty() && (run.style != cur_style || run.link != cur_link) {
                    flush(&mut out, &mut buf, cur_style, cur_link, &self.links);
                }
                cur_style = run.style;
                cur_link = run.link;
                let rtext = self.run_text(part.run as usize);
                let pstart = part.range_start as usize;
                buf.extend_from_slice(&rtext[pstart + lo..pstart + hi]);
            }
            cursor = part_end;
            if cursor >= col1 {
                break;
            }
        }
        flush(&mut out, &mut buf, cur_style, cur_link, &self.links);
        out
    }
    /// Extract selected text as **Markdown**, preserving original source syntax
    /// for whole blocks and reconstructing inline formatting for partial slices.
    fn selected_markdown(&self) -> String {
        let Some((start, end)) = self.normalized_selection() else {
            return String::new();
        };
        if start.row >= self.rows.len() || end.row >= self.rows.len() {
            return String::new();
        }
        // Group the selected rows by owning block.
        let mut selected_blocks: Vec<(usize, usize, usize)> = Vec::new(); // (block, first_row, last_row)
        for r in start.row..=end.row {
            let row = &self.rows[r];
            if self.row_text_len(row) == 0 {
                continue;
            }
            let block = row.block as usize;
            if let Some(last) = selected_blocks.last_mut() {
                if last.0 == block {
                    last.2 = r;
                    continue;
                }
            }
            selected_blocks.push((block, r, r));
        }
        if selected_blocks.is_empty() {
            return String::new();
        }
        // Helper: check whether all text of `block_idx` is covered by the selection.
        let block_is_full = |block_idx: usize| -> bool {
            let target = block_idx as u32;
            let first_row = self
                .rows
                .iter()
                .position(|r| r.block == target && self.row_text_len(r) > 0);
            let last_row = self
                .rows
                .iter()
                .rposition(|r| r.block == target && self.row_text_len(r) > 0);
            let (Some(fr), Some(lr)) = (first_row, last_row) else {
                return false;
            };
            let starts_before_or_at_0 = start.row < fr || (start.row == fr && start.col == 0);
            let ends_after_or_at_end =
                end.row > lr || (end.row == lr && end.col >= self.row_text_len(&self.rows[lr]));
            starts_before_or_at_0 && ends_after_or_at_end
        };

        // Fast path: if every selected block is fully covered, slice the original
        // Markdown source directly so headings, fences, tables, and blank lines
        // come out 100% verbatim!
        if selected_blocks.iter().all(|&(b, ..)| block_is_full(b)) {
            let first_b = selected_blocks.first().unwrap().0;
            let last_b = selected_blocks.last().unwrap().0;
            let s0 = (self.blocks[first_b].raw_start as usize).min(self.source.len());
            let s1 = (self.blocks[last_b].raw_end as usize).min(self.source.len());
            if s0 <= s1 && self.source.is_char_boundary(s0) && self.source.is_char_boundary(s1) {
                let mut raw = self.source[s0..s1].to_owned();
                if self.blocks[first_b].kind == Kind::Code && self.blocks[last_b].kind == Kind::Code
                {
                    raw = format!("```\r\n{raw}\r\n```");
                }
                return raw.replace("\r\n", "\n").replace('\n', "\r\n");
            }
        }

        let mut parts: Vec<String> = Vec::new();
        for &(block_idx, r0, r1) in &selected_blocks {
            if block_is_full(block_idx) {
                let raw = self.blocks[block_idx].raw_range();
                if raw.start <= raw.end
                    && raw.end <= self.source.len()
                    && self.source.is_char_boundary(raw.start)
                    && self.source.is_char_boundary(raw.end)
                {
                    parts.push(self.source[raw.start..raw.end].to_owned());
                    continue;
                }
            }
            let mut block_md = String::new();
            let starts_at_block_start = {
                let target = block_idx as u32;
                let first = self
                    .rows
                    .iter()
                    .position(|r| r.block == target && self.row_text_len(r) > 0);
                first.is_some_and(|fr| start.row < fr || (start.row == fr && start.col == 0))
            };
            if starts_at_block_start {
                match self.blocks[block_idx].kind {
                    Kind::Heading(level) => {
                        for _ in 0..level {
                            block_md.push('#');
                        }
                        block_md.push(' ');
                    }
                    Kind::Quote => block_md.push_str("> "),
                    Kind::List(depth) => {
                        for _ in 0..depth {
                            block_md.push_str("  ");
                        }
                        block_md.push_str("- ");
                    }
                    _ => {}
                }
            }
            let mut body = String::new();
            let mut prev_cell = NO_CELL;
            for r in r0..=r1 {
                let len = self.row_text_len(&self.rows[r]);
                if len == 0 {
                    continue;
                }
                let c0 = if r == start.row {
                    start.col.min(len)
                } else {
                    0
                };
                let c1 = if r == end.row { end.col.min(len) } else { len };
                if c1 <= c0 {
                    continue;
                }
                let mut slice = self.row_markdown_slice(r, c0, c1);
                // Avoid duplicating `- • ` when prefixing a list item.
                if starts_at_block_start
                    && body.is_empty()
                    && matches!(self.blocks[block_idx].kind, Kind::List(_))
                {
                    if let Some(rest) = slice.strip_prefix("• ") {
                        slice = rest.to_owned();
                    }
                }
                if !body.is_empty() {
                    if prev_cell != self.rows[r].cell {
                        body.push_str(" | ");
                    } else if !body.ends_with(' ') && !slice.starts_with(' ') {
                        body.push(' ');
                    }
                }
                body.push_str(&slice);
                prev_cell = self.rows[r].cell;
            }
            block_md.push_str(&body);
            parts.push(block_md);
        }
        parts
            .join("\r\n")
            .replace("\r\n", "\n")
            .replace('\n', "\r\n")
    }
}

struct App {
    tabs: Vec<Tab>,
    active: usize,
    fonts: Vec<Font>,
    /// DPI-scaled UI fonts (never affected by document text zoom).
    ui_regular: Handle,
    ui_bold: Handle,
    ui_small: Handle,
    themes: Themes,
    /// Index into `themes.palettes`.
    theme: usize,
    menu_brush: Handle,
    bar_brush: Handle,
    menu_font: Handle,
    menu_labels: Vec<*mut Box<[u16]>>,
    restore: Option<(Themes, usize)>,
    pinned: Option<Pipe>,
    restyle: bool,
    /// Clickable areas of the footer, measured while it is painted.
    hot: RefCell<Vec<(Rect, Hit)>>,
    /// Clickable areas of the top tab bar, measured while it is painted.
    tab_hot: RefCell<Vec<(Rect, TabHit)>>,
    /// Clickable heading rows in the outline sidebar, measured while painted.
    outline_hot: RefCell<Vec<(Rect, usize)>>,
    floater: Hwnd,
    floating: bool,
    outline_open: bool,
    outline_width: i32,
    outline_drag: Option<(i32, i32)>,
    divider_hover: bool,
    outline_hover: Option<usize>,
    tab_hover: Option<TabHit>,
    /// URL/path of the link currently under the pointer (shown in status bar).
    hover_link: Option<String>,
    /// Selection drag state.
    selecting: bool,
    press_origin: Option<(i32, i32)>,
    press_link: Option<String>,
    context_link: Option<String>,
    theme_path: PathBuf,
    dpi: u32,
    zoom: i32,
    width: i32,
    /// Scrollable document viewport height (between tab bar and status bar).
    height: i32,
    wheel_remainder: i32,
    pixel_remainder: i32,
    last_frame: Instant,
    /// Grab offset inside the scrollbar thumb while dragging it.
    grab: Option<i32>,
    hover: bool,
    buffer: Option<Buffer>,
    smooth: bool,
    menu: Handle,
    layout_pending: bool,
    engine: RefCell<Option<dwrite::Engine>>,
    engine_failed: bool,
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            for font in [
                &mut self.ui_regular,
                &mut self.ui_bold,
                &mut self.ui_small,
                &mut self.menu_font,
                &mut self.menu_brush,
                &mut self.bar_brush,
            ] {
                let h = std::mem::replace(font, NULL);
                if !h.is_null() {
                    DeleteObject(h);
                }
            }
            for label in self.menu_labels.drain(..) {
                drop(Box::from_raw(label));
            }
        }
    }
}

impl App {
    fn new() -> Self {
        let theme_path = crate::config::path();
        let themes = Themes::load(&theme_path);
        Self {
            tabs: vec![Tab::new("Welcome")],
            active: 0,
            fonts: vec![],
            ui_regular: NULL,
            ui_bold: NULL,
            ui_small: NULL,
            theme: themes.start,
            pinned: pinned(&themes),
            smooth: themes.smooth,
            zoom: themes.zoom,
            floating: themes.floating,
            outline_open: themes.outline,
            outline_width: OUTLINE_DEFAULT_W,
            outline_drag: None,
            divider_hover: false,
            outline_hover: None,
            tab_hover: None,
            hover_link: None,
            selecting: false,
            press_origin: None,
            press_link: None,
            context_link: None,
            themes,
            menu_brush: NULL,
            bar_brush: NULL,
            menu_font: NULL,
            menu_labels: vec![],
            restore: None,
            theme_path,
            restyle: false,
            hot: RefCell::new(vec![]),
            tab_hot: RefCell::new(vec![]),
            outline_hot: RefCell::new(vec![]),
            floater: NULL,
            dpi: 96,
            width: 0,
            height: 0,
            wheel_remainder: 0,
            pixel_remainder: 0,
            last_frame: Instant::now(),
            grab: None,
            hover: false,
            buffer: None,
            menu: NULL,
            layout_pending: false,
            engine: RefCell::new(None),
            engine_failed: false,
        }
    }
    #[inline]
    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    #[inline]
    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
    /// Document-scaled pixels (DPI × zoom).
    fn px(&self, n: i32) -> i32 {
        ((n as i64 * self.dpi as i64 * self.zoom as i64) / (96 * 100)) as i32
    }
    /// Chrome-scaled pixels (DPI only, crisp at every document zoom).
    fn ui_px(&self, n: i32) -> i32 {
        ((n as i64 * self.dpi as i64) / 96) as i32
    }
    /// Height of the top tab bar.
    fn tab_bar_h(&self) -> i32 {
        self.ui_px(40).max(1)
    }
    /// Height of the status footer.
    fn status(&self) -> i32 {
        self.ui_px(28).max(1)
    }
    /// Total client height (tab bar + document viewport + status footer).
    fn client_h(&self) -> i32 {
        self.tab_bar_h() + self.height + self.status()
    }
    /// Width of the left outline sidebar (0 when hidden or window is narrow).
    fn sidebar_w(&self) -> i32 {
        if self.outline_open && self.width >= self.ui_px(480) {
            self.ui_px(self.outline_width).min(self.width / 2)
        } else {
            0
        }
    }
    /// Document viewport top y in client coordinates.
    fn doc_top(&self) -> i32 {
        self.tab_bar_h()
    }
    /// Document viewport bottom y in client coordinates.
    fn doc_bottom(&self) -> i32 {
        self.tab_bar_h() + self.height
    }
    /// Document viewport left x in client coordinates.
    fn doc_left(&self) -> i32 {
        self.sidebar_w()
    }
    /// Scrollbar width and left x.
    fn bar(&self) -> (i32, i32) {
        let width = self.ui_px(10).max(4);
        (width, self.width - width)
    }
    /// Centered reading column `(left, right)` inside the document viewport.
    fn column(&self) -> (i32, i32) {
        let left_bound = self.doc_left();
        let right_bound = self.bar().1;
        let avail = (right_bound - left_bound).max(1);
        let pad = self.px(28).min(avail / 8);
        let max_col = self.px(READING_MAX_PX);
        let col_w = (avail - 2 * pad).min(max_col).max(1);
        let left = left_bound + (avail - col_w) / 2;
        (left, (left + col_w).min(right_bound).max(left + 1))
    }
    /// Top (relative to `doc_top`) and height of the scrollbar thumb.
    fn thumb(&self) -> Option<(i32, i32)> {
        let tab = self.tab();
        let span = tab.content_height - self.height;
        if span <= 0 || self.height <= 0 {
            return None;
        }
        let height = (self.height as i64 * self.height as i64 / tab.content_height.max(1) as i64)
            .max(self.ui_px(28) as i64) as i32;
        let travel = (self.height - height).max(0);
        Some((
            (travel as i64 * tab.scroll as i64 / span as i64) as i32,
            height,
        ))
    }
    fn bar_update(&self, previous: Option<(i32, i32)>) -> Rect {
        let (width, x) = self.bar();
        let top0 = self.doc_top();
        let mut rect = Rect {
            left: x,
            top: top0,
            right: x + width,
            bottom: top0 + self.height,
        };
        if let (Some((top, height)), Some((was, was_height))) = (self.thumb(), previous) {
            rect.top = top0 + top.min(was);
            rect.bottom = top0 + (top + height).max(was + was_height);
        }
        rect
    }
    unsafe fn invalidate_bar(&self, hwnd: Hwnd, previous: Option<(i32, i32)>) {
        InvalidateRect(hwnd, &self.bar_update(previous), 0);
    }
    /// Route a click or drag on the scrollbar (`y` in client coordinates).
    fn bar_target(&self, client_y: i32) -> i32 {
        let y = client_y - self.doc_top();
        let tab = self.tab();
        let Some((top, height)) = self.thumb() else {
            return tab.scroll;
        };
        let span = tab.content_height - self.height;
        match self.grab {
            Some(grab) => {
                let travel = (self.height - height).max(1);
                ((y - grab).clamp(0, travel) as i64 * span as i64 / travel as i64) as i32
            }
            None if y < top => tab.scroll - (self.height - self.px(24)).max(1),
            None => tab.scroll + (self.height - self.px(24)).max(1),
        }
    }
    fn palette(&self) -> Palette {
        self.themes.palettes[self.theme.min(self.themes.palettes.len() - 1)].1
    }
    unsafe fn ensure_ui_fonts(&mut self) {
        if self.ui_regular.is_null() {
            self.ui_regular = ui_font_weight(self.ui_px(13), 400);
        }
        if self.ui_bold.is_null() {
            self.ui_bold = ui_font_weight(self.ui_px(13), 600);
        }
        if self.ui_small.is_null() {
            self.ui_small = ui_font_weight(self.ui_px(11), 600);
        }
    }
    fn set_document(&mut self, source: &str) {
        if let Some(engine) = self.engine.get_mut() {
            engine.clear_paragraphs();
        }
        let zoom = self.zoom;
        let pinned = self.pinned;
        self.tab_mut().set_document(source, zoom, pinned);
        self.wheel_remainder = 0;
        self.pixel_remainder = 0;
        self.hover_link = None;
    }
    fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi.clamp(96, 960);
        self.rescale_ui();
        self.rescale();
    }
    fn rescale_ui(&mut self) {
        unsafe {
            for font in [
                &mut self.menu_font,
                &mut self.ui_regular,
                &mut self.ui_bold,
                &mut self.ui_small,
            ] {
                let h = std::mem::replace(font, NULL);
                if !h.is_null() {
                    DeleteObject(h);
                }
            }
        }
    }
    /// Throw away everything that was measured at the old text size.
    fn rescale(&mut self) {
        self.fonts.clear();
        for tab in &mut self.tabs {
            for run in &mut tab.runs {
                run.width.set(UNMEASURED);
            }
            tab.emojis.clear();
            tab.layout_sig = (0, 0, 0, 0, None);
        }
        *self.engine.get_mut() = None;
        self.engine_failed = false;
    }
    fn zoomed(&self, spot: Hit) -> i32 {
        match spot {
            Hit::ZoomOut => self.zoom - Themes::ZOOM.2,
            Hit::ZoomIn => self.zoom + Themes::ZOOM.2,
            _ => 100,
        }
    }
    unsafe fn set_zoom(&mut self, hwnd: Hwnd, zoom: i32) {
        let zoom = zoom.clamp(Themes::ZOOM.0, Themes::ZOOM.1);
        if zoom == self.zoom {
            return;
        }
        let anchor = self.tab().scroll as f64 / self.tab().content_height.max(1) as f64;
        self.zoom = zoom;
        for tab in &mut self.tabs {
            tab.stats.zoom = zoom;
        }
        self.rescale();
        self.layout(hwnd);
        let target = (anchor * self.tab().content_height as f64) as i32;
        self.scroll_to(hwnd, target);
    }
    unsafe fn rebuild(&mut self, hwnd: Hwnd) {
        if let Some(engine) = self.engine.get_mut() {
            engine.clear_paragraphs();
        }
        let zoom = self.zoom;
        let pinned = self.pinned;
        for tab in &mut self.tabs {
            let source = std::mem::take(&mut tab.source);
            let scroll = tab.scroll;
            tab.set_document(&source, zoom, pinned);
            tab.source = source;
            tab.scroll = scroll;
        }
        self.layout(hwnd);
    }
    fn engine(&mut self) -> Option<&mut dwrite::Engine> {
        let engine = self.engine.get_mut();
        if engine.is_none() && !self.engine_failed {
            *engine = unsafe { dwrite::Engine::new().ok() };
            self.engine_failed = engine.is_none();
        }
        engine.as_mut()
    }
    unsafe fn font(&mut self, dc: Hdc, style: Style, heading: u8) -> usize {
        let key = (style.0, heading);
        if let Some(index) = self.fonts.iter().position(|font| font.key == key) {
            return index;
        }
        let code = style.0 & Style::CODE != 0;
        // rmdv / Notion reading scale: 16px body, 30/24/20/18/16/15 headings, 14px code.
        let size = match heading {
            1 => 30,
            2 => 24,
            3 => 20,
            4 => 18,
            5 => 16,
            6 => 15,
            _ => 16,
        };
        let weight = if heading == 1 || heading == 2 || style.0 & Style::BOLD != 0 {
            700
        } else if heading > 0 {
            600
        } else {
            400
        };
        let face_name = if code {
            modern_fonts().code
        } else if heading == 1 || heading == 2 {
            modern_fonts().display
        } else {
            modern_fonts().ui
        };
        let face = wide(face_name);
        let handle = CreateFontW(
            -self.px(if code { 14 } else { size }),
            0,
            0,
            0,
            weight,
            (style.0 & Style::ITALIC != 0) as u32,
            (style.0 & Style::LINK != 0) as u32,
            (style.0 & Style::STRIKE != 0) as u32,
            1,
            0,
            0,
            5,
            0,
            face.as_ptr(),
        );
        let handle = if handle.is_null() {
            GetStockObject(17)
        } else {
            handle
        };
        let old = SelectObject(dc, handle);
        let mut metric = TextMetric::default();
        GetTextMetricsW(dc, &mut metric);
        let mut ascii_buf = [0i32; 128];
        let ascii = (GetCharWidth32W(dc, 0, 127, ascii_buf.as_mut_ptr()) != 0).then_some(ascii_buf);
        SelectObject(dc, old);
        let index = self.fonts.len();
        self.fonts.push(Font {
            handle,
            key,
            height: metric.height.max(self.px(16)),
            ascent: metric.ascent.max(1),
            size: self.px(if code { 14 } else { size }),
            ascii,
            bmp: RefCell::new(None),
            plus: Cell::new(None),
        });
        index
    }
    fn blank(&self, y: i32, base: usize, frame: &Frame, pipe: Pipe) -> Row {
        let start = self.tab().parts_pool.len() as u32;
        Row {
            y,
            height: self.fonts[base].height + self.px(7).max(1),
            ascent: self.fonts[base].ascent,
            span: frame.span,
            block: frame.block as u32,
            parts_start: start,
            shaped: NO_SHAPED,
            parts_len: 0,
            cell: frame.cell.map_or(NO_CELL, |c| c as u16),
            deco: frame.deco,
            pipe,
            flags: frame.flags,
        }
    }
    unsafe fn layout(&mut self, hwnd: Hwnd) {
        self.layout_pending = false;
        self.ensure_ui_fonts();
        let mut client = Rect::default();
        GetClientRect(hwnd, &mut client);
        self.width = client.right;
        self.height = (client.bottom - self.tab_bar_h() - self.status()).max(0);
        if self.width <= 0 || self.height <= 0 {
            return;
        }
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return;
        }
        let normal = self.font(dc, Style::default(), 0);
        let old = SelectObject(dc, self.fonts[normal].handle);
        let mut selected_font = normal;
        let block_count = self.tab().blocks.len();
        {
            let tab = self.tab_mut();
            tab.rows.clear();
            tab.rows.reserve(block_count);
            tab.parts_pool.clear();
            tab.parts_pool.reserve(block_count);
            tab.shaped_rows.clear();
        }
        let (col_left, col_right) = self.column();
        let top_pad = self.px(24);
        let mut y = top_pad;
        let mut heading_cursor = 0usize;
        let mut has_tables = false;
        let mut bi = 0;
        while bi < block_count {
            if matches!(self.tab().blocks[bi].kind, Kind::Row(_)) {
                has_tables = true;
                (bi, y) = self.table(dc, &mut selected_font, bi, col_left, col_right, y);
                continue;
            }
            let kind = self.tab().blocks[bi].kind.clone();
            if kind == Kind::Gap {
                y = y.saturating_add(self.px(12));
                bi += 1;
                continue;
            }
            if kind == Kind::Rule {
                let frame = Frame {
                    left: col_left,
                    right: col_right,
                    span: (col_left, col_right),
                    heading: 0,
                    deco: Deco::Rule,
                    align: 0,
                    block: bi,
                    cell: None,
                    flags: 0,
                };
                let mut row = self.blank(y, normal, &frame, Pipe::Gdi);
                row.height = self.px(20);
                row.ascent = 0;
                self.tab_mut().rows.push(row);
                y = y.saturating_add(self.px(24));
                bi += 1;
                continue;
            }
            let heading = match kind {
                Kind::Heading(level) => level,
                _ => 0,
            };
            let deco = match kind {
                Kind::Code => Deco::Code,
                Kind::Quote => Deco::Quote,
                _ => Deco::None,
            };
            let indent = match kind {
                Kind::List(depth) => self.px(14 + (depth as i32 * 18)),
                Kind::Code => self.px(14),
                Kind::Quote => self.px(16),
                _ => 0,
            };
            let left = col_left + indent.min(((col_right - col_left) / 3).max(0));
            let right =
                (col_right - if deco == Deco::Code { self.px(14) } else { 0 }).max(left + 1);
            if heading > 0 && y > top_pad {
                let extra = match heading {
                    1 => self.px(22),
                    2 => self.px(18),
                    _ => self.px(12),
                };
                y = y.saturating_add(extra);
            }
            if heading > 0 {
                if let Some(h) = self.tab_mut().headings.get_mut(heading_cursor) {
                    if h.block == bi {
                        h.y = y;
                        heading_cursor += 1;
                    }
                }
            }
            // Mark first/last lines of consecutive code blocks so we can paint a
            // framed rmdv card border cleanly with zero extra blocks.
            let mut flags = 0u8;
            if deco == Deco::Code {
                let prev_code = bi > 0 && self.tab().blocks[bi - 1].kind == Kind::Code;
                let next_code =
                    bi + 1 < block_count && self.tab().blocks[bi + 1].kind == Kind::Code;
                if !prev_code {
                    flags |= Row::EDGE_TOP;
                }
                if !next_code {
                    flags |= Row::EDGE_BOTTOM;
                }
            }
            let piece = self.tab().blocks[bi].body;
            let frame = Frame {
                left,
                right,
                span: (col_left, col_right),
                heading,
                deco,
                align: 0,
                block: bi,
                cell: None,
                flags,
            };
            let first_row = self.tab().rows.len();
            y = self.flow(dc, &mut selected_font, piece, frame, y);
            let last_row = self.tab().rows.len();
            if deco == Deco::Code && last_row > first_row {
                // Only the very first laid-out row gets EDGE_TOP and very last gets EDGE_BOTTOM.
                for r in first_row..last_row {
                    let mut f = 0;
                    if r == first_row && flags & Row::EDGE_TOP != 0 {
                        f |= Row::EDGE_TOP;
                    }
                    if r + 1 == last_row && flags & Row::EDGE_BOTTOM != 0 {
                        f |= Row::EDGE_BOTTOM;
                    }
                    self.tab_mut().rows[r].flags = f;
                }
            }
            if heading > 0 {
                if (heading == 1 || heading == 2) && last_row > first_row {
                    self.tab_mut().rows[last_row - 1].flags |= Row::HEADING_RULE;
                    y = y.saturating_add(self.px(10));
                } else {
                    y = y.saturating_add(self.px(6));
                }
            }
            bi += 1;
        }
        let sidebar_w = self.sidebar_w();
        let sig = (self.width, sidebar_w, self.dpi, self.zoom, self.pinned);
        let vp_height = self.height;
        {
            let tab = self.tab_mut();
            tab.content_height = y.saturating_add(top_pad);
            if has_tables {
                tab.rows.sort_by_key(|row| row.y);
            }
            tab.tallest = tab.rows.iter().map(|row| row.height).max().unwrap_or(0);
            shrink_oversized(&mut tab.rows);
            shrink_oversized(&mut tab.parts_pool);
            shrink_oversized(&mut tab.shaped_rows);
            tab.scroll = tab.scroll.clamp(0, (tab.content_height - vp_height).max(0));
            tab.pos = tab.scroll as f64;
            tab.target = tab.scroll;
            tab.animating = false;
            tab.layout_sig = sig;
        }
        SelectObject(dc, old);
        ReleaseDC(hwnd, dc);
        self.update_scroll(hwnd);
        self.place_floater();
        self.buffer = None;
        InvalidateRect(hwnd, ptr::null(), 0);
    }

    /// Lay out the consecutive table rows starting at `first`.
    unsafe fn table(
        &mut self,
        dc: Hdc,
        selected_font: &mut usize,
        first: usize,
        left: i32,
        right: i32,
        mut y: i32,
    ) -> (usize, i32) {
        let last = (first..self.tab().blocks.len())
            .find(|&i| !matches!(self.tab().blocks[i].kind, Kind::Row(_)))
            .unwrap_or(self.tab().blocks.len());
        let columns = self.tab().blocks[first..last]
            .iter()
            .map(|block| block.cells_len as usize)
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return (last, y);
        }
        let (pad, gap) = (self.px(10), self.px(5));
        let mut widths = vec![0; columns];
        for bi in first..last {
            let cell_count = self.tab().blocks[bi].cells_len as usize;
            for ci in 0..cell_count {
                let cell = self.tab().block_cells(bi)[ci];
                widths[ci] = widths[ci].max(self.measure(dc, selected_font, cell));
            }
        }
        let room = (right - left - columns as i32 * 2 * pad).max(columns as i32 * self.px(16));
        let natural: i32 = widths.iter().sum();
        if natural > room {
            for width in &mut widths {
                *width = (*width as i64 * room as i64 / natural.max(1) as i64) as i32;
            }
        }
        let table_right = left + widths.iter().sum::<i32>() + columns as i32 * 2 * pad;
        // Top border of table frame.
        let top_rule_frame = Frame {
            left,
            right: table_right,
            span: (left, table_right),
            heading: 0,
            deco: Deco::Grid,
            align: 0,
            block: first,
            cell: None,
            flags: Row::EDGE_TOP,
        };
        let mut top_rule = self.blank(y, 0, &top_rule_frame, Pipe::Gdi);
        top_rule.height = self.px(1).max(1);
        top_rule.ascent = 0;
        self.tab_mut().rows.push(top_rule);
        y = y.saturating_add(self.px(1).max(1));

        for bi in first..last {
            let header = self.tab().blocks[bi].kind == Kind::Row(true);
            let row_start_y = y;
            y = y.saturating_add(gap);
            let (mut x, mut bottom) = (left, y);
            let row_flags = if header { Row::TABLE_HEADER } else { 0 };
            for ci in 0..columns {
                let end = x + widths[ci] + 2 * pad;
                if let Some(&cell) = self.tab().block_cells(bi).get(ci) {
                    let frame = Frame {
                        left: x + pad,
                        right: (end - pad).max(x + pad + 1),
                        span: (x, end),
                        heading: 0,
                        deco: Deco::None,
                        align: cell.align,
                        block: bi,
                        cell: Some(ci),
                        flags: row_flags,
                    };
                    bottom = bottom.max(self.flow(dc, selected_font, cell, frame, y));
                }
                x = end;
            }
            let row_bottom = bottom.saturating_add(gap);
            // If header row, insert a subtle surface background row behind header cells
            // (placed right at `row_start_y`, sorted before the cell rows at `y`).
            if header {
                let bg_frame = Frame {
                    left,
                    right: x,
                    span: (left, x),
                    heading: 0,
                    deco: Deco::None,
                    align: 0,
                    block: bi,
                    cell: None,
                    flags: Row::TABLE_HEADER,
                };
                let mut bg = self.blank(row_start_y, 0, &bg_frame, Pipe::Gdi);
                bg.height = (row_bottom - row_start_y).max(1);
                bg.ascent = 0;
                self.tab_mut().rows.push(bg);
            }
            y = row_bottom;
            let thickness = if header { self.px(2) } else { self.px(1) }.max(1);
            let frame = Frame {
                left,
                right: x,
                span: (left, x),
                heading: 0,
                deco: Deco::Grid,
                align: 0,
                block: bi,
                cell: None,
                flags: 0,
            };
            let mut rule = self.blank(y, 0, &frame, Pipe::Gdi);
            rule.height = thickness;
            rule.ascent = 0;
            self.tab_mut().rows.push(rule);
            y = y.saturating_add(thickness);
        }
        y = y.saturating_add(gap);
        (last, y)
    }

    /// The width a piece wants when nothing forces it to wrap.
    unsafe fn measure(&mut self, dc: Hdc, selected_font: &mut usize, piece: Piece) -> i32 {
        let base = self.font(dc, Style::default(), 0);
        if let Some(text) = self.tab().piece_shaped(piece).cloned() {
            let pixels = self.fonts[base].size;
            return self
                .engine()
                .and_then(|engine| engine.paragraph(&text, pixels, 1 << 20, false).ok())
                .map_or(0, |layout| layout.natural);
        }
        let mut total = 0;
        for order in piece.run_range() {
            let ri = self.tab().run_order[order] as usize;
            if let Some(width) = self.tab().runs[ri].cached_width() {
                total += width;
                continue;
            }
            let style = self.tab().runs[ri].style;
            let font = self.font(dc, style, 0);
            let tab = &self.tabs[self.active];
            let rtext = tab.run_text(ri);
            let count = rtext.len().min(4096);
            let w = self.fonts[font].measure_width(dc, selected_font, font, &rtext[..count]);
            if count == rtext.len() {
                tab.runs[ri].set_width(w);
            }
            total += w;
        }
        total
    }

    /// Break one piece into rows inside `frame`, returning the y below it.
    unsafe fn flow(
        &mut self,
        dc: Hdc,
        selected_font: &mut usize,
        piece: Piece,
        frame: Frame,
        mut y: i32,
    ) -> i32 {
        let leading = self.px(7).max(1);
        let code = frame.deco == Deco::Code;
        let base = self.font(dc, Style(if code { Style::CODE } else { 0 }), frame.heading);
        let width = (frame.right - frame.left).max(1);
        let start_row = self.tab().rows.len();
        if let Some(text) = self.tab().piece_shaped(piece).cloned() {
            let bold = frame.heading > 0;
            let pixels = self.fonts[base].size;
            let layout = self
                .engine()
                .and_then(|engine| engine.paragraph(&text, pixels, width, bold).ok());
            let height = layout
                .as_ref()
                .map_or(self.fonts[base].height, |layout| layout.height)
                + leading;
            let natural = layout.as_ref().map_or(width, |layout| layout.natural);
            let mut row = self.blank(y, base, &frame, Pipe::D2d);
            row.height = height;
            let tab = self.tab_mut();
            let shaped_idx = tab.shaped_rows.len() as u32;
            tab.shaped_rows.push(ShapedRow {
                text,
                x: frame.left + offset(frame.align, width, natural.min(width)),
                width,
                pixels,
                bold,
            });
            row.shaped = shaped_idx;
            tab.rows.push(row);
            return y.saturating_add(height);
        }
        let mut row = self.blank(y, base, &frame, piece.pipe);
        let mut x = frame.left;
        for order in piece.run_range() {
            let ri_u32 = self.tab().run_order[order];
            let ri = ri_u32 as usize;
            let style = self.tab().runs[ri].style;
            let font = self.font(dc, style, frame.heading);
            if self.tab().runs[ri].is_emoji()
                && !self.tab().emojis.contains_key(&ri_u32)
                && !self.engine_failed
            {
                let pixels = self.fonts[font].size;
                let engine = self.engine.get_mut();
                if engine.is_none() {
                    *engine = dwrite::Engine::new().ok();
                }
                match engine {
                    Some(engine) => {
                        if let Ok(cluster) =
                            engine.cluster(self.tabs[self.active].run_text(ri), pixels)
                        {
                            self.tabs[self.active].emojis.insert(ri_u32, cluster);
                        }
                    }
                    None => self.engine_failed = true,
                }
            }
            if let Some((ew, eh, ea, tlen)) = self
                .tab()
                .emojis
                .get(&ri_u32)
                .map(|e| (e.width, e.height, e.ascent, self.tab().runs[ri].text_len))
            {
                if x + ew > frame.right && row.parts_len > 0 {
                    y = y.saturating_add(row.height);
                    self.tab_mut().rows.push(row);
                    row = self.blank(y, base, &frame, piece.pipe);
                    x = frame.left;
                }
                let descent = (row.height - leading - row.ascent).max(eh - ea);
                row.ascent = row.ascent.max(ea);
                row.height = row.ascent + descent + leading;
                self.tab_mut().parts_pool.push(Fragment {
                    run: ri_u32,
                    x,
                    width: ew,
                    range_start: 0,
                    range_len: tlen,
                    font: font as u16,
                });
                row.parts_len = row.parts_len.saturating_add(1);
                x += ew;
                continue;
            }
            let font_ascent = self.fonts[font].ascent;
            let font_height = self.fonts[font].height;
            let mut pos = 0;
            let text_len = self.tab().runs[ri].text_len as usize;
            while pos < text_len {
                let mut count = (text_len - pos).min(4096);
                {
                    let text = self.tab().run_text(ri);
                    if count < text_len - pos && is_high_surrogate(text[pos + count - 1]) {
                        count -= 1;
                    }
                }
                let cached = if pos == 0 && count == text_len {
                    self.tab().runs[ri].cached_width()
                } else {
                    None
                };
                let (mut take, mut chunk_w) = if let Some(width) =
                    cached.filter(|&width| width <= (frame.right - x).max(0))
                {
                    (count, width)
                } else {
                    let tab = &self.tabs[self.active];
                    let slice = &tab.run_text(ri)[pos..pos + count];
                    let (fit, total_w) = self.fonts[font].measure_fit(
                        dc,
                        selected_font,
                        font,
                        slice,
                        (frame.right - x).max(0),
                    );
                    if pos == 0 && count == text_len {
                        tab.runs[ri].set_width(total_w);
                    }
                    (fit, total_w)
                };
                {
                    let text = self.tab().run_text(ri);
                    if take > 0 && take < text_len - pos && is_high_surrogate(text[pos + take - 1])
                    {
                        take -= 1;
                    }
                    if take > 0
                        && take < count
                        && !code
                        && row.parts_len > 0
                        && !matches!(text[pos + take], 32 | 9)
                    {
                        let word_start = text[pos..pos + take]
                            .iter()
                            .position(|&c| !matches!(c, 32 | 9));
                        if word_start.is_some_and(|start| {
                            !text[pos + start..pos + take]
                                .iter()
                                .any(|&c| matches!(c, 32 | 9))
                        }) {
                            take = 0;
                        }
                    }
                }
                if take == 0 && row.parts_len > 0 {
                    y = y.saturating_add(row.height);
                    self.tab_mut().rows.push(row);
                    row = self.blank(y, base, &frame, piece.pipe);
                    x = frame.left;
                    if !code {
                        let text = self.tab().run_text(ri);
                        while pos < text_len && (text[pos] == 32 || text[pos] == 9) {
                            pos += 1;
                        }
                    }
                    continue;
                }
                let wraps;
                {
                    let tab = &self.tabs[self.active];
                    let text = tab.run_text(ri);
                    if take == 0 {
                        take = if is_high_surrogate(text[pos]) && pos + 1 < text_len {
                            2
                        } else {
                            1
                        };
                    }
                    wraps = take < count;
                    if wraps && !code {
                        if let Some(space) = text[pos..pos + take]
                            .iter()
                            .rposition(|&c| c == 32 || c == 9)
                        {
                            if space > 0 {
                                take = space + 1;
                            }
                        }
                    }
                    if take != count {
                        chunk_w = self.fonts[font].measure_width(
                            dc,
                            selected_font,
                            font,
                            &text[pos..pos + take],
                        );
                    }
                }
                row.ascent = row.ascent.max(font_ascent);
                row.height = row.height.max(font_height + leading);
                self.tab_mut().parts_pool.push(Fragment {
                    run: ri_u32,
                    x,
                    width: chunk_w,
                    range_start: pos as u16,
                    range_len: take as u16,
                    font: font as u16,
                });
                row.parts_len = row.parts_len.saturating_add(1);
                x += chunk_w;
                pos += take;
                if wraps || x >= frame.right {
                    y = y.saturating_add(row.height);
                    self.tab_mut().rows.push(row);
                    row = self.blank(y, base, &frame, piece.pipe);
                    x = frame.left;
                    if !code {
                        let text = self.tab().run_text(ri);
                        while pos < text_len && (text[pos] == 32 || text[pos] == 9) {
                            pos += 1;
                        }
                    }
                }
            }
        }
        let all_empty = piece
            .run_range()
            .all(|i| self.tab().runs[self.tab().run_order[i] as usize].text_len == 0);
        if row.parts_len > 0 || all_empty {
            y = y.saturating_add(row.height);
            self.tab_mut().rows.push(row);
        }
        if frame.align != 0 {
            let tab = self.tab_mut();
            for row in &mut tab.rows[start_row..] {
                let s = row.parts_start as usize;
                let parts = &mut tab.parts_pool[s..s + row.parts_len as usize];
                let used = parts.last().map_or(0, |part| part.x + part.width) - frame.left;
                let shift = offset(frame.align, width, used);
                for part in parts {
                    part.x += shift;
                }
            }
        }
        y
    }
    unsafe fn update_scroll(&self, hwnd: Hwnd) {
        let tab = self.tab();
        let mut info = ScrollInfo::new(1 | 2 | 4);
        info.max = (tab.content_height - 1).max(0);
        info.page = self.height.max(0) as u32;
        info.pos = tab.scroll;
        SetScrollInfo(hwnd, 1, &info, 0);
        ADJUSTING.set(true);
        ShowScrollBar(hwnd, 1, 0);
        ADJUSTING.set(false);
    }
    /// Move the view now. Callers that animate go through `glide_to`.
    unsafe fn scroll_to(&mut self, hwnd: Hwnd, target: i32) {
        self.shift(hwnd, target);
        let tab = self.tab_mut();
        tab.target = tab.scroll;
        tab.animating = false;
    }
    /// Ease the view towards `target`.
    unsafe fn glide_to(&mut self, hwnd: Hwnd, target: i32) {
        let max_scroll = (self.tab().content_height - self.height).max(0);
        let target = target.clamp(0, max_scroll);
        let scroll = self.tab().scroll;
        let far = (target - scroll).abs() > self.height.saturating_mul(FAR);
        if !self.smooth || target == scroll || far {
            return self.scroll_to(hwnd, target);
        }
        if !self.tab().animating {
            self.tab_mut().animating = true;
            self.tab_mut().pos = scroll as f64;
            self.last_frame = Instant::now() - std::time::Duration::from_millis(8);
        }
        self.tab_mut().target = target;
    }
    /// One animation frame, driven by the message loop once per display refresh.
    unsafe fn glide(&mut self, hwnd: Hwnd) {
        let farthest = (self.tab().content_height - self.height).max(0);
        let vp_height = self.height;
        let now = Instant::now();
        let elapsed = now
            .duration_since(self.last_frame)
            .as_secs_f64()
            .clamp(0.001, 0.1);
        self.last_frame = now;
        let next_scroll = {
            let tab = self.tab_mut();
            tab.target = tab.target.clamp(0, farthest);
            if tab.pos.round() as i32 != tab.scroll {
                tab.pos = tab.scroll as f64;
            }
            let remaining = tab.target as f64 - tab.pos;
            tab.pos = if remaining.abs() < 0.5 {
                tab.target as f64
            } else {
                let reach = SPEED * vp_height.max(1) as f64 * elapsed;
                let step = remaining * (1.0 - (-elapsed / EASE).exp());
                tab.pos + step.clamp(-reach, reach)
            };
            tab.pos.round() as i32
        };
        self.shift(hwnd, next_scroll);
        let tab = self.tab_mut();
        if tab.scroll == tab.target {
            tab.animating = false;
        }
    }
    unsafe fn shift(&mut self, hwnd: Hwnd, target: i32) {
        let lead = self.px(48);
        let old_heading = self.tab().active_heading(lead);
        let max_scroll = (self.tab().content_height - self.height).max(0);
        let target = target.clamp(0, max_scroll);
        let old_scroll = self.tab().scroll;
        if target == old_scroll {
            return;
        }
        let delta = old_scroll - target;
        self.tab_mut().scroll = target;
        let new_heading = self.tab().active_heading(lead);
        let mut info = ScrollInfo::new(4);
        info.pos = target;
        SetScrollInfo(hwnd, 1, &info, 0);
        self.present(hwnd, delta, old_heading != new_heading);
    }
    /// Show the view at its new place without ever showing a half-painted one.
    unsafe fn present(&self, hwnd: Hwnd, delta: i32, outline_changed: bool) {
        let Some(buffer) = self
            .buffer
            .as_ref()
            .filter(|buffer| (buffer.width, buffer.height) == (self.width, self.client_h()))
        else {
            InvalidateRect(hwnd, ptr::null(), 0);
            return;
        };
        let doc_left = self.doc_left();
        let doc_top = self.doc_top();
        let doc_bottom = self.doc_bottom();
        let bar_x = self.bar().1;
        let view = Rect {
            left: doc_left,
            top: doc_top,
            right: bar_x,
            bottom: doc_bottom,
        };
        let exposed = if delta.abs() >= self.height {
            view
        } else {
            BitBlt(
                buffer.dc,
                doc_left,
                doc_top + delta.max(0),
                bar_x - doc_left,
                self.height - delta.abs(),
                buffer.dc,
                doc_left,
                doc_top + (-delta).max(0),
                0x00cc0020,
            );
            if delta > 0 {
                Rect {
                    bottom: doc_top + delta,
                    ..view
                }
            } else {
                Rect {
                    top: doc_bottom + delta,
                    ..view
                }
            }
        };
        self.draw(buffer.dc, exposed);
        self.draw_bar(buffer.dc);
        let blit_left = if outline_changed && doc_left > 0 {
            let sidebar_rect = Rect {
                left: 0,
                top: doc_top,
                right: doc_left,
                bottom: doc_bottom,
            };
            self.draw(buffer.dc, sidebar_rect);
            0
        } else {
            doc_left
        };
        let dc = GetDC(hwnd);
        if !dc.is_null() {
            BitBlt(
                dc,
                blit_left,
                doc_top,
                self.width - blit_left,
                self.height,
                buffer.dc,
                blit_left,
                doc_top,
                0x00cc0020,
            );
            ReleaseDC(hwnd, dc);
        }
    }
    unsafe fn paint(&mut self, hwnd: Hwnd) {
        self.ensure_ui_fonts();
        let mut paint: PaintStruct = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut paint);
        let (width, height) = (self.width, self.client_h());
        let fresh = !self
            .buffer
            .as_ref()
            .is_some_and(|buffer| (buffer.width, buffer.height) == (width, height));
        if fresh {
            self.buffer = Buffer::new(dc, width.max(1), height.max(1));
        }
        match &self.buffer {
            Some(buffer) => {
                let clip = paint.rect;
                let whole = Rect {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                };
                self.draw(buffer.dc, if fresh { whole } else { clip });
                BitBlt(
                    dc,
                    clip.left,
                    clip.top,
                    clip.right - clip.left,
                    clip.bottom - clip.top,
                    buffer.dc,
                    clip.left,
                    clip.top,
                    0x00cc0020,
                );
            }
            None => self.draw(dc, paint.rect),
        }
        EndPaint(hwnd, &paint);
    }
    unsafe fn draw(&self, dc: Hdc, clip: Rect) {
        let saved = SaveDC(dc);
        IntersectClipRect(dc, clip.left, clip.top, clip.right, clip.bottom);
        self.draw_clipped(dc, clip);
        RestoreDC(dc, saved);
    }
    unsafe fn draw_clipped(&self, dc: Hdc, clip: Rect) {
        let palette = self.palette();
        let doc_top = self.doc_top();
        let doc_bottom = self.doc_bottom();
        let doc_left = self.doc_left();
        let bar_x = self.bar().1;

        // 1. Document viewport (only if clip intersects it).
        let doc_clip = Rect {
            left: clip.left.max(doc_left),
            top: clip.top.max(doc_top),
            right: clip.right.min(bar_x),
            bottom: clip.bottom.min(doc_bottom),
        };
        if doc_clip.right > doc_clip.left && doc_clip.bottom > doc_clip.top {
            let saved = SaveDC(dc);
            IntersectClipRect(
                dc,
                doc_clip.left,
                doc_clip.top,
                doc_clip.right,
                doc_clip.bottom,
            );
            self.draw_document(dc, doc_clip, palette);
            RestoreDC(dc, saved);
        }

        // 2. Left outline sidebar.
        if doc_left > 0 && clip.left < doc_left && clip.bottom > doc_top && clip.top < doc_bottom {
            let saved = SaveDC(dc);
            IntersectClipRect(dc, 0, doc_top, doc_left, doc_bottom);
            self.draw_outline(dc, palette);
            RestoreDC(dc, saved);
        }

        // 3. Top tab bar.
        if clip.top < doc_top {
            let saved = SaveDC(dc);
            IntersectClipRect(dc, 0, 0, self.width, doc_top);
            self.draw_tabs(dc, palette);
            RestoreDC(dc, saved);
        }

        // 4. Bottom status footer.
        if clip.bottom > doc_bottom {
            self.draw_status(dc);
        }

        // 5. Slim scrollbar.
        if clip.right > bar_x && clip.bottom > doc_top && clip.top < doc_bottom {
            self.draw_bar(dc);
        }
    }

    unsafe fn draw_document(&self, dc: Hdc, clip: Rect, palette: Palette) {
        let tab = self.tab();
        let doc_top = self.doc_top();
        fill(dc, clip, palette.background);
        SetBkMode(dc, 1);
        let mut previous = NULL;
        let mut selected = usize::MAX;
        let mut canvas = None;
        let mut last_color = u32::MAX;
        let top = tab.scroll.saturating_add(clip.top - doc_top);
        let bottom = tab.scroll.saturating_add(clip.bottom - doc_top);
        let sel = tab.normalized_selection();
        let border_color = palette.border();
        let surface_color = palette.surface();
        let sel_bg = palette.selection();
        let px1 = self.px(1).max(1);

        let first = tab
            .rows
            .partition_point(|row| row.y.saturating_add(tab.tallest) <= top);
        for (offset_idx, row) in tab.rows[first..]
            .iter()
            .enumerate()
            .take_while(|(_, row)| row.y < bottom)
        {
            let row_idx = first + offset_idx;
            if row.y.saturating_add(row.height) <= top {
                continue;
            }
            let y = doc_top + row.y - tab.scroll;
            let (left, right) = row.span;

            // Table header row tinted fill (rmdv style).
            if row.flags & Row::TABLE_HEADER != 0 && row.parts_len == 0 && row.shaped == NO_SHAPED {
                fill(
                    dc,
                    Rect {
                        left,
                        top: y,
                        right,
                        bottom: y + row.height,
                    },
                    surface_color,
                );
                continue;
            }

            // Block decorations.
            match row.deco {
                Deco::Code => {
                    fill(
                        dc,
                        Rect {
                            left,
                            top: y,
                            right,
                            bottom: y + row.height,
                        },
                        palette.code_background,
                    );
                    // Crisp rmdv card border around code blocks.
                    fill(
                        dc,
                        Rect {
                            left,
                            top: y,
                            right: left + px1,
                            bottom: y + row.height,
                        },
                        border_color,
                    );
                    fill(
                        dc,
                        Rect {
                            left: right - px1,
                            top: y,
                            right,
                            bottom: y + row.height,
                        },
                        border_color,
                    );
                    if row.flags & Row::EDGE_TOP != 0 {
                        fill(
                            dc,
                            Rect {
                                left,
                                top: y,
                                right,
                                bottom: y + px1,
                            },
                            border_color,
                        );
                    }
                    if row.flags & Row::EDGE_BOTTOM != 0 {
                        fill(
                            dc,
                            Rect {
                                left,
                                top: y + row.height - px1,
                                right,
                                bottom: y + row.height,
                            },
                            border_color,
                        );
                    }
                }
                Deco::Grid => {
                    fill(
                        dc,
                        Rect {
                            left,
                            top: y,
                            right,
                            bottom: y + row.height,
                        },
                        border_color,
                    );
                }
                Deco::Quote => {
                    // Subtle modern surface wash with crisp rounded accent bar.
                    fill(
                        dc,
                        Rect {
                            left: left + self.px(3).max(3),
                            top: y,
                            right,
                            bottom: y + row.height,
                        },
                        surface_color,
                    );
                    fill(
                        dc,
                        Rect {
                            left,
                            top: y,
                            right: left + self.px(3).max(3),
                            bottom: y + row.height,
                        },
                        palette.accent,
                    );
                }
                Deco::Rule => {
                    fill(
                        dc,
                        Rect {
                            left,
                            top: y + row.height / 2,
                            right,
                            bottom: y + row.height / 2 + px1,
                        },
                        border_color,
                    );
                }
                Deco::None => {
                    if row.flags & Row::HEADING_RULE != 0 {
                        fill(
                            dc,
                            Rect {
                                left,
                                top: y + row.height - px1,
                                right,
                                bottom: y + row.height,
                            },
                            border_color,
                        );
                    }
                }
            }

            let foreground = match row.deco {
                Deco::Code => palette.code_foreground,
                Deco::Quote => palette.muted,
                _ => palette.foreground,
            };

            // Selection range on this row (if any).
            let row_sel = sel.and_then(|(s, e)| {
                if row_idx < s.row || row_idx > e.row {
                    return None;
                }
                let len = tab.row_text_len(row);
                let c0 = if row_idx == s.row { s.col.min(len) } else { 0 };
                let c1 = if row_idx == e.row {
                    e.col.min(len)
                } else {
                    len
                };
                (c1 > c0).then_some((c0, c1))
            });

            if let Some(shaped) = tab.row_shaped(row) {
                last_color = u32::MAX;
                let background = if row.deco == Deco::Code {
                    palette.code_background
                } else if row.flags & Row::TABLE_HEADER != 0 {
                    surface_color
                } else {
                    palette.background
                };
                let mut engine = self.engine.borrow_mut();
                if let Some(engine) = engine.as_mut() {
                    if let Ok(layout) =
                        engine.paragraph(&shaped.text, shaped.pixels, shaped.width, shaped.bold)
                    {
                        if engine.draw_paragraph_selected(
                            &layout,
                            dc,
                            shaped.x,
                            y,
                            clip,
                            palette,
                            foreground,
                            background,
                            row_sel.map(|(a, b)| (a as u32, b as u32)),
                        ) {
                            continue;
                        }
                    }
                }
                let Some(fallback) = self.fonts.first() else {
                    continue;
                };
                let old = SelectObject(dc, fallback.handle);
                SetTextColor(dc, foreground);
                TextOutW(
                    dc,
                    shaped.x,
                    y,
                    shaped.text.text.as_ptr(),
                    shaped.text.text.len().min(4096) as i32,
                );
                SelectObject(dc, old);
                continue;
            }

            // Paint GDI/GDI+ selection highlight behind text on this row.
            if let Some((c0, c1)) = row_sel {
                if let Some((sx0, sx1)) = self.row_selection_pixels(dc, row, c0, c1) {
                    fill(
                        dc,
                        Rect {
                            left: sx0,
                            top: y,
                            right: sx1,
                            bottom: y + row.height,
                        },
                        sel_bg,
                    );
                    selected = usize::MAX;
                }
            }

            for part in tab.row_parts(row) {
                let run_idx = part.run as usize;
                let font_idx = part.font as usize;
                let run = &tab.runs[run_idx];
                let font = &self.fonts[font_idx];
                if selected != font_idx {
                    let old = SelectObject(dc, font.handle);
                    if selected == usize::MAX {
                        previous = old;
                    }
                    selected = font_idx;
                }
                let inline_code = run.style.0 & Style::CODE != 0;
                if inline_code && row.deco != Deco::Code && row_sel.is_none() {
                    fill_rounded(
                        dc,
                        Rect {
                            left: part.x - self.px(2),
                            top: y + self.px(2),
                            right: part.x + part.width + self.px(2),
                            bottom: y + row.height - self.px(3),
                        },
                        self.px(4).max(2),
                        palette.code_background,
                        border_color,
                    );
                }
                if run.link != NO_LINK
                    && row_sel.is_none()
                    && self.hover_link.as_deref().is_some_and(|h| {
                        tab.links.get(run.link as usize).map(String::as_str) == Some(h)
                    })
                {
                    fill_rounded(
                        dc,
                        Rect {
                            left: part.x - self.px(1),
                            top: y + self.px(1),
                            right: part.x + part.width + self.px(1),
                            bottom: y + row.height - self.px(2),
                        },
                        self.px(3).max(2),
                        palette.link_hover(),
                        palette.link_hover(),
                    );
                }
                let color = if inline_code {
                    palette.code_foreground
                } else if run.style.0 & Style::LINK != 0 {
                    palette.accent
                } else {
                    foreground
                };
                let baseline = y + row.ascent - font.ascent;
                let text = &tab.run_text(run_idx)[part.range()];
                if run.is_emoji() {
                    if let Some(emoji) = tab.emojis.get(&part.run) {
                        if let Some(engine) = self.engine.borrow_mut().as_mut() {
                            if engine.draw_cluster(emoji, dc, part.x, y + row.ascent, color) {
                                continue;
                            }
                        }
                    }
                }
                if row.pipe == Pipe::GdiPlus {
                    let canvas = canvas.get_or_insert_with(|| Canvas::new(dc));
                    if let Some(canvas) = canvas {
                        let plus = font.plus.get().unwrap_or_else(|| {
                            let plus = Canvas::font(dc);
                            font.plus.set(Some(plus));
                            plus
                        });
                        if canvas.text(plus, part.x, baseline, text, color) {
                            continue;
                        }
                    }
                }
                if last_color != color {
                    SetTextColor(dc, color);
                    last_color = color;
                }
                TextOutW(dc, part.x, baseline, text.as_ptr(), text.len() as i32);
            }
        }
        if selected != usize::MAX {
            SelectObject(dc, previous);
        }
    }

    /// Compute pixel `[x0, x1]` for UTF-16 selection `[c0..c1)` on a GDI/GDI+ row.
    unsafe fn row_selection_pixels(
        &self,
        dc: Hdc,
        row: &Row,
        c0: usize,
        c1: usize,
    ) -> Option<(i32, i32)> {
        if row.parts_len == 0 || c1 <= c0 {
            return None;
        }
        let tab = self.tab();
        let parts = tab.row_parts(row);
        let total = tab.row_text_len(row);
        if c0 == 0 && c1 >= total {
            let first = parts.first()?;
            let last = parts.last()?;
            return Some((first.x, last.x + last.width));
        }
        let mut x0 = None;
        let mut x1 = None;
        let mut cursor = 0usize;
        for part in parts {
            let len = part.len();
            let part_end = cursor + len;
            let run = &tab.runs[part.run as usize];
            let rtext = tab.run_text(part.run as usize);
            let pstart = part.range_start as usize;
            let measure_prefix = |prefix_len: usize| -> i32 {
                if prefix_len == 0 {
                    return 0;
                }
                if prefix_len >= len {
                    return part.width;
                }
                if run.is_emoji() {
                    return part.width;
                }
                let old = SelectObject(dc, self.fonts[part.font as usize].handle);
                let mut sz = Size::default();
                GetTextExtentPoint32W(dc, rtext[pstart..].as_ptr(), prefix_len as i32, &mut sz);
                SelectObject(dc, old);
                sz.cx
            };
            if x0.is_none() && c0 >= cursor && c0 <= part_end {
                x0 = Some(part.x + measure_prefix(c0 - cursor));
            }
            if c1 >= cursor && c1 <= part_end {
                x1 = Some(part.x + measure_prefix(c1 - cursor));
            }
            cursor = part_end;
        }
        let left = x0.unwrap_or(parts[0].x);
        let right = x1.unwrap_or_else(|| {
            let last = parts.last().unwrap();
            last.x + last.width
        });
        (right > left).then_some((left, right))
    }

    /// Top tab bar with Outline toggle, floating rounded tab pills, `+`,
    /// and right-aligned theme switcher & Settings controls.
    unsafe fn draw_tabs(&self, dc: Hdc, palette: Palette) {
        let h = self.tab_bar_h();
        let bg = palette.sidebar();
        let border = palette.border();
        let px1 = self.ui_px(1).max(1);
        let bar = Rect {
            left: 0,
            top: 0,
            right: self.width,
            bottom: h,
        };
        fill(dc, bar, bg);
        fill(
            dc,
            Rect {
                left: 0,
                top: h - px1,
                right: self.width,
                bottom: h,
            },
            border,
        );
        let mut hot = self.tab_hot.borrow_mut();
        hot.clear();
        SetBkMode(dc, 1);

        // 1. Outline toggle button on the left (modern mini-sidebar icon).
        let btn_pad = self.ui_px(6);
        let toggle_rect = Rect {
            left: self.ui_px(8),
            top: btn_pad,
            right: self.ui_px(38),
            bottom: h - btn_pad,
        };
        let toggle_hot = self.tab_hover == Some(TabHit::ToggleOutline);
        if self.outline_open || toggle_hot {
            let fill_c = if toggle_hot {
                palette.hover(bg)
            } else {
                palette.active_item(bg)
            };
            fill_rounded(dc, toggle_rect, self.ui_px(6).max(3), fill_c, fill_c);
        }
        let icon_c = if self.outline_open {
            palette.accent
        } else {
            palette.muted
        };
        let ix = toggle_rect.left + self.ui_px(7);
        let ir = toggle_rect.right - self.ui_px(7);
        let it = toggle_rect.top + self.ui_px(6);
        let ib = toggle_rect.bottom - self.ui_px(6);
        if ir > ix + 4 && ib > it + 4 {
            // Crisp rounded mini-window frame with left sidebar pane.
            fill_rounded(
                dc,
                Rect {
                    left: ix,
                    top: it,
                    right: ir,
                    bottom: ib,
                },
                self.ui_px(2).max(1),
                bg,
                icon_c,
            );
            let pane_r = ix + ((ir - ix) * 2 / 5).max(3);
            fill(
                dc,
                Rect {
                    left: ix + px1,
                    top: it + px1,
                    right: pane_r,
                    bottom: ib - px1,
                },
                if self.outline_open {
                    palette.accent
                } else {
                    palette.hover(bg)
                },
            );
            fill(
                dc,
                Rect {
                    left: pane_r,
                    top: it,
                    right: pane_r + px1,
                    bottom: ib,
                },
                icon_c,
            );
        }
        hot.push((toggle_rect, TabHit::ToggleOutline));

        // 2. Right-aligned quick-action controls: Settings icon + Theme pill.
        let mut right_edge = self.width - self.ui_px(8);
        let settings_w = self.ui_px(30);
        if right_edge - settings_w > toggle_rect.right + self.ui_px(120) {
            let set_rect = Rect {
                left: right_edge - settings_w,
                top: btn_pad,
                right: right_edge,
                bottom: h - btn_pad,
            };
            let set_hot = self.tab_hover == Some(TabHit::OpenSettings);
            if set_hot {
                let hc = palette.hover(bg);
                fill_rounded(dc, set_rect, self.ui_px(6).max(3), hc, hc);
            }
            // Minimal geometric sliders icon (3 horizontal rails + knobs).
            let sc = if set_hot {
                palette.foreground
            } else {
                palette.muted
            };
            let sx = set_rect.left + self.ui_px(8);
            let sr = set_rect.right - self.ui_px(8);
            let smy = (set_rect.top + set_rect.bottom) / 2;
            let sgap = self.ui_px(4).max(3);
            let knob = self.ui_px(3).max(2);
            for (dy, frac) in [(-sgap, 2), (0, 5), (sgap, 3)] {
                let ry = smy + dy;
                fill(
                    dc,
                    Rect {
                        left: sx,
                        top: ry,
                        right: sr,
                        bottom: ry + px1,
                    },
                    sc,
                );
                let kx = sx + ((sr - sx - knob) * frac / 6).max(0);
                fill_rounded(
                    dc,
                    Rect {
                        left: kx,
                        top: ry - knob / 2,
                        right: kx + knob,
                        bottom: ry - knob / 2 + knob + px1,
                    },
                    knob,
                    if set_hot { palette.accent } else { sc },
                    if set_hot { palette.accent } else { sc },
                );
            }
            hot.push((set_rect, TabHit::OpenSettings));
            right_edge = set_rect.left - self.ui_px(6);
        }

        // Theme pill showing accent dot + current palette name (click cycles palettes).
        if right_edge > toggle_rect.right + self.ui_px(240) {
            let theme_name =
                &self.themes.palettes[self.theme.min(self.themes.palettes.len() - 1)].0;
            let name_w = wide(theme_name);
            let old = SelectObject(dc, self.ui_small);
            let mut sz = Size::default();
            GetTextExtentPoint32W(dc, name_w.as_ptr(), name_w.len() as i32 - 1, &mut sz);
            let dot = self.ui_px(7).max(5);
            let pill_w = (sz.cx + dot + self.ui_px(22)).min(self.ui_px(136));
            let pill_rect = Rect {
                left: right_edge - pill_w,
                top: btn_pad,
                right: right_edge,
                bottom: h - btn_pad,
            };
            let theme_hot = self.tab_hover == Some(TabHit::CycleTheme);
            let pill_bg = if theme_hot {
                palette.hover(palette.surface())
            } else {
                palette.surface()
            };
            fill_rounded(dc, pill_rect, self.ui_px(6).max(3), pill_bg, border);
            let dy = (pill_rect.top + pill_rect.bottom - dot) / 2;
            let dx = pill_rect.left + self.ui_px(8);
            fill_rounded(
                dc,
                Rect {
                    left: dx,
                    top: dy,
                    right: dx + dot,
                    bottom: dy + dot,
                },
                dot,
                palette.accent,
                palette.accent,
            );
            SetTextColor(
                dc,
                if theme_hot {
                    palette.foreground
                } else {
                    palette.muted
                },
            );
            let mut tr = Rect {
                left: dx + dot + self.ui_px(6),
                top: pill_rect.top,
                right: pill_rect.right - self.ui_px(8),
                bottom: pill_rect.bottom,
            };
            DrawTextW(
                dc,
                name_w.as_ptr(),
                name_w.len() as i32 - 1,
                &mut tr,
                0x20 | 0x4 | 0x8000 | 0x800,
            );
            SelectObject(dc, old);
            hot.push((pill_rect, TabHit::CycleTheme));
            right_edge = pill_rect.left - self.ui_px(8);
        }

        // 3. Document tabs (floating modern rounded pills).
        let start_x = toggle_rect.right + self.ui_px(8);
        let new_w = self.ui_px(28);
        let avail = (right_edge - start_x - new_w - self.ui_px(6)).max(1);
        let count = self.tabs.len().max(1) as i32;
        let tab_w = (avail / count).clamp(self.ui_px(84), self.ui_px(216));
        let gap = self.ui_px(4);
        let mut x = start_x;
        let close_w = self.ui_px(18);
        let tab_r = self.ui_px(6).max(3);

        for (i, tab) in self.tabs.iter().enumerate() {
            if x + self.ui_px(44) > right_edge - new_w - self.ui_px(4) {
                break;
            }
            let w = tab_w.min(right_edge - new_w - self.ui_px(6) - x);
            let is_active = i == self.active;
            let is_hovered = self.tab_hover == Some(TabHit::Select(i))
                || self.tab_hover == Some(TabHit::Close(i));
            let rect = Rect {
                left: x,
                top: self.ui_px(5),
                right: x + w,
                bottom: h - self.ui_px(5),
            };
            if is_active {
                fill_rounded(dc, rect, tab_r, palette.background, border);
                // Sleek inset rounded accent indicator at top of active pill.
                let bar_inset = self.ui_px(10).min((w / 4).max(2));
                fill_rounded(
                    dc,
                    Rect {
                        left: rect.left + bar_inset,
                        top: rect.top + px1,
                        right: rect.right - bar_inset,
                        bottom: rect.top + self.ui_px(3).max(2),
                    },
                    self.ui_px(2).max(1),
                    palette.accent,
                    palette.accent,
                );
            } else if is_hovered {
                let hc = palette.hover(bg);
                fill_rounded(dc, rect, tab_r, hc, hc);
            } else if i > 0 && i - 1 != self.active && self.tab_hover != Some(TabHit::Select(i - 1))
            {
                // Subtle hairline divider between unhovered inactive tabs.
                fill(
                    dc,
                    Rect {
                        left: x - gap / 2,
                        top: rect.top + self.ui_px(6),
                        right: x - gap / 2 + px1,
                        bottom: rect.bottom - self.ui_px(6),
                    },
                    border,
                );
            }

            let close_rect = Rect {
                left: rect.right - close_w - self.ui_px(5),
                top: rect.top + (rect.bottom - rect.top - close_w) / 2,
                right: rect.right - self.ui_px(5),
                bottom: rect.top + (rect.bottom - rect.top + close_w) / 2,
            };
            let show_close = self.tabs.len() > 1 || tab.file.is_some();
            let text_right = if show_close {
                close_rect.left - self.ui_px(2)
            } else {
                rect.right - self.ui_px(10)
            };
            let font = if is_active {
                self.ui_bold
            } else {
                self.ui_regular
            };
            let old = SelectObject(dc, font);
            SetTextColor(
                dc,
                if is_active || is_hovered {
                    palette.foreground
                } else {
                    palette.muted
                },
            );
            let title = wide(&tab.title);
            let mut label_rect = Rect {
                left: rect.left + self.ui_px(11),
                top: rect.top + self.ui_px(1),
                right: text_right.max(rect.left + self.ui_px(11)),
                bottom: rect.bottom,
            };
            DrawTextW(
                dc,
                title.as_ptr(),
                title.len() as i32 - 1,
                &mut label_rect,
                0x20 | 0x4 | 0x8000 | 0x800,
            );
            SelectObject(dc, old);

            if show_close && (is_active || is_hovered) {
                let close_hot = self.tab_hover == Some(TabHit::Close(i));
                if close_hot {
                    let hc = palette.hover(if is_active { palette.background } else { bg });
                    fill_rounded(dc, close_rect, self.ui_px(4).max(2), hc, hc);
                }
                let old_c = SelectObject(dc, self.ui_regular);
                SetTextColor(
                    dc,
                    if close_hot {
                        palette.foreground
                    } else {
                        palette.muted
                    },
                );
                let cross = wide("\u{00d7}");
                let mut cr = close_rect;
                DrawTextW(dc, cross.as_ptr(), 1, &mut cr, 0x20 | 0x4 | 0x1);
                SelectObject(dc, old_c);
                hot.push((close_rect, TabHit::Close(i)));
            }
            hot.push((rect, TabHit::Select(i)));
            x += w + gap;
        }

        // 4. New tab (`+`) button.
        let new_rect = Rect {
            left: x + self.ui_px(2),
            top: btn_pad,
            right: (x + self.ui_px(2) + new_w).min(right_edge),
            bottom: h - btn_pad,
        };
        if new_rect.right > new_rect.left + self.ui_px(12) {
            let new_hot = self.tab_hover == Some(TabHit::NewTab);
            if new_hot {
                let hc = palette.hover(bg);
                fill_rounded(dc, new_rect, self.ui_px(6).max(3), hc, hc);
            }
            let pc = if new_hot {
                palette.foreground
            } else {
                palette.muted
            };
            let cx = (new_rect.left + new_rect.right) / 2;
            let cy = (new_rect.top + new_rect.bottom) / 2;
            let arm = self.ui_px(5).max(3);
            let thick = self.ui_px(2).max(1);
            fill(
                dc,
                Rect {
                    left: cx - arm,
                    top: cy - thick / 2,
                    right: cx + arm + (thick & 1),
                    bottom: cy - thick / 2 + thick,
                },
                pc,
            );
            fill(
                dc,
                Rect {
                    left: cx - thick / 2,
                    top: cy - arm,
                    right: cx - thick / 2 + thick,
                    bottom: cy + arm + (thick & 1),
                },
                pc,
            );
            hot.push((new_rect, TabHit::NewTab));
        }
    }

    /// Left Outline sidebar (rmdv aesthetic: recessed surface, uppercase header,
    /// indented headings, subtle indent guides, scroll-spy accent bar).
    unsafe fn draw_outline(&self, dc: Hdc, palette: Palette) {
        let w = self.sidebar_w();
        if w <= 0 {
            return;
        }
        let doc_top = self.doc_top();
        let doc_bottom = self.doc_bottom();
        let bg = palette.sidebar();
        let border = palette.border();
        let px1 = self.ui_px(1).max(1);
        fill(
            dc,
            Rect {
                left: 0,
                top: doc_top,
                right: w,
                bottom: doc_bottom,
            },
            bg,
        );
        // Right hairline divider (accent when hovered/dragged).
        fill(
            dc,
            Rect {
                left: w - px1,
                top: doc_top,
                right: w,
                bottom: doc_bottom,
            },
            if self.outline_drag.is_some() || self.divider_hover {
                palette.accent
            } else {
                border
            },
        );
        SetBkMode(dc, 1);

        // Header: "OUTLINE"
        let header_h = self.ui_px(30);
        let old = SelectObject(dc, self.ui_small);
        SetTextColor(dc, palette.subtle());
        let title = wide("OUTLINE");
        let mut hrect = Rect {
            left: self.ui_px(14),
            top: doc_top,
            right: w - self.ui_px(10),
            bottom: doc_top + header_h,
        };
        DrawTextW(
            dc,
            title.as_ptr(),
            title.len() as i32 - 1,
            &mut hrect,
            0x20 | 0x4,
        );
        SelectObject(dc, old);

        let list_top = doc_top + header_h;
        let mut hot = self.outline_hot.borrow_mut();
        hot.clear();

        let tab = self.tab();
        if tab.headings.is_empty() {
            let old = SelectObject(dc, self.ui_regular);
            SetTextColor(dc, palette.subtle());
            let empty = wide("No headings");
            let mut r = Rect {
                left: self.ui_px(14),
                top: list_top + self.ui_px(6),
                right: w - self.ui_px(10),
                bottom: list_top + self.ui_px(30),
            };
            DrawTextW(
                dc,
                empty.as_ptr(),
                empty.len() as i32 - 1,
                &mut r,
                0x20 | 0x4,
            );
            SelectObject(dc, old);
            return;
        }

        let row_h = self.ui_px(26).max(18);
        let active_idx = tab.active_heading(self.px(48));
        let saved = SaveDC(dc);
        IntersectClipRect(dc, 0, list_top, w - px1, doc_bottom);
        for (i, h) in tab.headings.iter().enumerate() {
            let y = list_top + self.ui_px(2) + (i as i32) * row_h - tab.outline_scroll;
            if y + row_h <= list_top {
                continue;
            }
            if y >= doc_bottom {
                break;
            }
            let item_rect = Rect {
                left: self.ui_px(6),
                top: y,
                right: w - self.ui_px(7),
                bottom: y + row_h - self.ui_px(1),
            };
            let is_active = active_idx == Some(i);
            let is_hovered = self.outline_hover == Some(i);
            if is_active {
                let fill_c = palette.active_item(bg);
                fill_rounded(dc, item_rect, self.ui_px(5).max(2), fill_c, fill_c);
                // Crisp 2px accent indicator on the active heading.
                fill(
                    dc,
                    Rect {
                        left: item_rect.left,
                        top: item_rect.top + self.ui_px(4),
                        right: item_rect.left + self.ui_px(2).max(2),
                        bottom: item_rect.bottom - self.ui_px(4),
                    },
                    palette.accent,
                );
            } else if is_hovered {
                let fill_c = palette.hover(bg);
                fill_rounded(dc, item_rect, self.ui_px(5).max(2), fill_c, fill_c);
            }
            let depth = h.level.saturating_sub(1).min(5) as i32;
            let step = self.ui_px(12);
            // Subtle vertical indent guides for nested headings (rmdv style).
            for d in 0..depth {
                let gx = item_rect.left + self.ui_px(10) + d * step;
                fill(
                    dc,
                    Rect {
                        left: gx,
                        top: item_rect.top + self.ui_px(2),
                        right: gx + px1,
                        bottom: item_rect.bottom - self.ui_px(2),
                    },
                    border,
                );
            }
            let font = if h.level <= 1 || is_active {
                self.ui_bold
            } else {
                self.ui_regular
            };
            let old = SelectObject(dc, font);
            SetTextColor(
                dc,
                if is_active || h.level <= 1 || is_hovered {
                    palette.foreground
                } else {
                    palette.muted
                },
            );
            let label = wide(&h.title);
            let mut text_rect = Rect {
                left: item_rect.left + self.ui_px(10) + depth * step,
                top: item_rect.top,
                right: item_rect.right - self.ui_px(8),
                bottom: item_rect.bottom,
            };
            if text_rect.right > text_rect.left {
                DrawTextW(
                    dc,
                    label.as_ptr(),
                    label.len() as i32 - 1,
                    &mut text_rect,
                    0x20 | 0x4 | 0x8000 | 0x800,
                );
            }
            SelectObject(dc, old);
            hot.push((item_rect, i));
        }
        RestoreDC(dc, saved);
    }

    /// Sleek rmdv-style slim scrollbar with rounded thumb.
    unsafe fn draw_bar(&self, dc: Hdc) {
        let palette = self.palette();
        let (width, x) = self.bar();
        let doc_top = self.doc_top();
        let doc_bottom = self.doc_bottom();
        fill(
            dc,
            Rect {
                left: x,
                top: doc_top,
                right: x + width,
                bottom: doc_bottom,
            },
            palette.background,
        );
        let Some((top, height)) = self.thumb() else {
            return;
        };
        let inset = self.ui_px(2).max(1);
        let thumb_color = palette.scroller(self.hover || self.grab.is_some());
        fill_rounded(
            dc,
            Rect {
                left: x + inset,
                top: doc_top + top + inset,
                right: x + width - inset,
                bottom: doc_top + top + height - inset,
            },
            (width - 2 * inset).max(2),
            thumb_color,
            thumb_color,
        );
    }

    /// The footer: zoom controls, document stats (or hovered link target), and pipeline.
    unsafe fn draw_status(&self, dc: Hdc) {
        let font = if !self.ui_regular.is_null() {
            self.ui_regular
        } else if let Some(f) = self.fonts.first() {
            f.handle
        } else {
            return;
        };
        let palette = self.palette();
        let doc_bottom = self.doc_bottom();
        let bar = Rect {
            left: 0,
            top: doc_bottom,
            right: self.width,
            bottom: doc_bottom + self.status(),
        };
        fill(dc, bar, palette.sidebar());
        fill(
            dc,
            Rect {
                bottom: bar.top + self.ui_px(1).max(1),
                ..bar
            },
            palette.border(),
        );
        let old = SelectObject(dc, font);
        let mut metric = TextMetric::default();
        GetTextMetricsW(dc, &mut metric);
        let y = bar.top + ((self.status() - metric.height) / 2).max(0);
        let padding = self.ui_px(16);
        let mut hot = self.hot.borrow_mut();
        hot.clear();
        SetBkMode(dc, 1);

        // Zoom cluster on the left.
        let mut x = padding;
        for (_, spot, glyph, _) in ZOOM {
            let text = wide(&glyph.replace('%', &format!("{}%", self.zoom)));
            let count = text.len() as i32 - 1;
            let mut size = Size::default();
            GetTextExtentPoint32W(dc, text.as_ptr(), count, &mut size);
            let width = size.cx + self.ui_px(12);
            SetTextColor(
                dc,
                if spot == Hit::ZoomReset {
                    palette.foreground
                } else {
                    palette.accent
                },
            );
            TextOutW(dc, x + (width - size.cx) / 2, y, text.as_ptr(), count);
            hot.push((
                Rect {
                    left: x,
                    top: bar.top,
                    right: x + width,
                    bottom: bar.bottom,
                },
                spot,
            ));
            x += width;
        }

        // Pipeline name on the right.
        let tab = self.tab();
        let label = wide(&tab.stats.label(self.pinned));
        let mut size = Size::default();
        GetTextExtentPoint32W(dc, label.as_ptr(), label.len() as i32 - 1, &mut size);
        let mut left = self.width - padding - size.cx;
        SetTextColor(dc, palette.accent);
        TextOutW(dc, left, y, label.as_ptr(), label.len() as i32 - 1);
        hot.push((
            Rect {
                left,
                top: bar.top,
                right: left + size.cx,
                bottom: bar.bottom,
            },
            Hit::Engine,
        ));
        if let Some(warning) = tab.stats.warning(self.pinned) {
            let warning = wide(&format!("{warning}  \u{b7}  "));
            let count = warning.len() as i32 - 1;
            GetTextExtentPoint32W(dc, warning.as_ptr(), count, &mut size);
            left -= size.cx;
            SetTextColor(dc, palette.foreground);
            TextOutW(dc, left, y, warning.as_ptr(), count);
        }

        // Middle: hovered link URL preview (browser-style) or document summary.
        let (mid_text, mid_color) = if let Some(url) = &self.hover_link {
            (format!("\u{2197}  {url}"), palette.accent)
        } else {
            (format!("\u{b7}  {}", tab.stats.summary()), palette.muted)
        };
        let summary = wide(&mid_text);
        let mut room = Rect {
            left: x + self.ui_px(8),
            top: bar.top,
            right: left - self.ui_px(8),
            bottom: bar.bottom,
        };
        if room.right > room.left {
            SetTextColor(dc, mid_color);
            DrawTextW(
                dc,
                summary.as_ptr(),
                summary.len() as i32 - 1,
                &mut room,
                0x20 | 0x4 | 0x8000 | 0x800,
            );
        }
        SelectObject(dc, old);
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        self.hot
            .borrow()
            .iter()
            .find(|(rect, _)| {
                (rect.left..rect.right).contains(&x) && (rect.top..rect.bottom).contains(&y)
            })
            .map(|(_, spot)| *spot)
    }
    fn hit_tab(&self, x: i32, y: i32) -> Option<TabHit> {
        self.tab_hot
            .borrow()
            .iter()
            .find(|(rect, _)| {
                (rect.left..rect.right).contains(&x) && (rect.top..rect.bottom).contains(&y)
            })
            .map(|(_, spot)| *spot)
    }
    fn hit_outline(&self, x: i32, y: i32) -> Option<usize> {
        self.outline_hot
            .borrow()
            .iter()
            .find(|(rect, _)| {
                (rect.left..rect.right).contains(&x) && (rect.top..rect.bottom).contains(&y)
            })
            .map(|(_, idx)| *idx)
    }
    fn on_divider(&self, x: i32, y: i32) -> bool {
        let sw = self.sidebar_w();
        sw > 0
            && y >= self.doc_top()
            && y < self.doc_bottom()
            && (x - sw).abs() <= self.ui_px(4).max(3)
    }
    fn in_doc_viewport(&self, x: i32, y: i32) -> bool {
        x >= self.doc_left() && x < self.bar().1 && y >= self.doc_top() && y < self.doc_bottom()
    }

    /// Map a client point `(x, y)` inside the document viewport to a `SelPos`
    /// and (if the point landed directly on a link glyph) its URL string.
    unsafe fn hit_doc(&mut self, hwnd: Hwnd, x: i32, y: i32) -> (Option<SelPos>, Option<String>) {
        let tab = &self.tabs[self.active];
        if tab.rows.is_empty() {
            return (None, None);
        }
        let doc_y = (y - self.doc_top()).saturating_add(tab.scroll);
        // Find candidate rows spanning doc_y (tables can have multiple rows with the same y).
        let first = tab
            .rows
            .partition_point(|r| r.y.saturating_add(tab.tallest) <= doc_y);
        let mut best_row: Option<usize> = None;
        let mut best_dist = i32::MAX;
        for (i, row) in tab.rows[first..].iter().enumerate() {
            let idx = first + i;
            if row.y > doc_y && best_row.is_some() {
                break;
            }
            if tab.row_text_len(row) == 0 {
                continue;
            }
            let in_y = doc_y >= row.y && doc_y < row.y + row.height;
            if in_y {
                let x_dist = if x < row.span.0 {
                    row.span.0 - x
                } else if x > row.span.1 {
                    x - row.span.1
                } else {
                    0
                };
                if x_dist < best_dist {
                    best_dist = x_dist;
                    best_row = Some(idx);
                }
            }
        }
        // If clicked in a vertical gap or past end, snap to nearest selectable row.
        let row_idx = match best_row {
            Some(i) => i,
            None => {
                if doc_y <= tab.rows[0].y {
                    match tab.rows.iter().position(|r| tab.row_text_len(r) > 0) {
                        Some(i) => i,
                        None => return (None, None),
                    }
                } else {
                    let pos = tab.rows.partition_point(|r| r.y <= doc_y);
                    let before = tab.rows[..pos]
                        .iter()
                        .rposition(|r| tab.row_text_len(r) > 0);
                    let after = tab.rows[pos..]
                        .iter()
                        .position(|r| tab.row_text_len(r) > 0)
                        .map(|o| pos + o);
                    match (before, after) {
                        (Some(b), Some(a)) => {
                            if (doc_y - (tab.rows[b].y + tab.rows[b].height)).abs()
                                <= (tab.rows[a].y - doc_y).abs()
                            {
                                b
                            } else {
                                a
                            }
                        }
                        (Some(b), None) => b,
                        (None, Some(a)) => a,
                        (None, None) => return (None, None),
                    }
                }
            }
        };

        let on_row_y = {
            let row = &self.tabs[self.active].rows[row_idx];
            doc_y >= row.y && doc_y < row.y + row.height
        };

        // Direct2D shaped paragraph hit-test.
        let shaped_info = {
            let tab = &self.tabs[self.active];
            let row = &tab.rows[row_idx];
            tab.row_shaped(row)
                .map(|s| (s.text.clone(), s.x, s.width, s.pixels, s.bold, row.y))
        };
        if let Some((rich, rx, rw, rp, rb, ry)) = shaped_info {
            let mut pos = 0usize;
            let mut link = None;
            if let Some(engine) = self.engine.borrow_mut().as_mut() {
                if let Ok(layout) = engine.paragraph(&rich, rp, rw, rb) {
                    let (hit_pos, inside) =
                        layout.hit_test_point((x - rx) as f32, (doc_y - ry) as f32);
                    pos = (hit_pos as usize).min(rich.text.len());
                    if inside && on_row_y {
                        if let Some(id) =
                            rich.link_at(hit_pos.min(rich.text.len().saturating_sub(1) as u32))
                        {
                            link = self.tabs[self.active].links.get(id as usize).cloned();
                        }
                    }
                }
            }
            return (
                Some(SelPos {
                    row: row_idx,
                    col: pos,
                }),
                link,
            );
        }

        // GDI / GDI+ fragments hit-test.
        let dc = GetDC(hwnd);
        let tab = &self.tabs[self.active];
        let row = &tab.rows[row_idx];
        let mut cursor = 0usize;
        let mut found_col = None;
        let mut found_link = None;
        for part in tab.row_parts(row) {
            let len = part.len();
            if x < part.x {
                found_col = Some(cursor);
                break;
            }
            if x <= part.x + part.width {
                let run = &tab.runs[part.run as usize];
                if on_row_y && run.link != NO_LINK {
                    found_link = tab.links.get(run.link as usize).cloned();
                }
                let local = if run.is_emoji() {
                    if x - part.x >= part.width / 2 {
                        len
                    } else {
                        0
                    }
                } else if !dc.is_null() {
                    let old = SelectObject(dc, self.fonts[part.font as usize].handle);
                    let slice = &tab.run_text(part.run as usize)[part.range()];
                    let mut fit = 0i32;
                    let mut sz = Size::default();
                    GetTextExtentExPointW(
                        dc,
                        slice.as_ptr(),
                        slice.len() as i32,
                        (x - part.x).max(0),
                        &mut fit,
                        ptr::null_mut(),
                        &mut sz,
                    );
                    let mut col = (fit.max(0) as usize).min(slice.len());
                    // Snap to the closer side of the character straddling `x`.
                    if col < slice.len() {
                        let step = if is_high_surrogate(slice[col]) && col + 1 < slice.len() {
                            2
                        } else {
                            1
                        };
                        let mut s0 = Size::default();
                        let mut s1 = Size::default();
                        if col > 0 {
                            GetTextExtentPoint32W(dc, slice.as_ptr(), col as i32, &mut s0);
                        }
                        GetTextExtentPoint32W(dc, slice.as_ptr(), (col + step) as i32, &mut s1);
                        if x - part.x > (s0.cx + s1.cx) / 2 {
                            col += step;
                        }
                    }
                    SelectObject(dc, old);
                    col
                } else {
                    0
                };
                found_col = Some(cursor + local);
                break;
            }
            cursor += len;
        }
        if !dc.is_null() {
            ReleaseDC(hwnd, dc);
        }
        let col = found_col.unwrap_or(cursor);
        (Some(SelPos { row: row_idx, col }), found_link)
    }

    /// Double-click word selection at `pos`.
    fn select_word_at(&mut self, pos: SelPos) {
        let tab = self.tab_mut();
        if pos.row >= tab.rows.len() {
            return;
        }
        let len = tab.row_text_len(&tab.rows[pos.row]);
        if len == 0 {
            return;
        }
        let plain = tab.row_plain_slice(pos.row, 0, len);
        let units: Vec<u16> = plain.encode_utf16().collect();
        let at = pos.col.min(units.len().saturating_sub(1));
        let is_word = |u: u16| {
            char::from_u32(u as u32).is_some_and(|c| c.is_alphanumeric() || c == '_')
                || (0xd800..=0xdfff).contains(&u)
        };
        let target_word = is_word(units[at]);
        let mut s = at;
        while s > 0 && is_word(units[s - 1]) == target_word {
            s -= 1;
        }
        let mut e = at + 1;
        while e < units.len() && is_word(units[e]) == target_word {
            e += 1;
        }
        tab.selection = Some((
            SelPos {
                row: pos.row,
                col: s,
            },
            SelPos {
                row: pos.row,
                col: e,
            },
        ));
    }

    /// Select the entire document in the active tab (`Ctrl+A`).
    fn select_all(&mut self) {
        let tab = self.tab_mut();
        let first = tab.rows.iter().position(|r| tab.row_text_len(r) > 0);
        let last = tab.rows.iter().rposition(|r| tab.row_text_len(r) > 0);
        if let (Some(f), Some(l)) = (first, last) {
            let end_col = tab.row_text_len(&tab.rows[l]);
            tab.selection = Some((
                SelPos { row: f, col: 0 },
                SelPos {
                    row: l,
                    col: end_col,
                },
            ));
        }
    }

    /// Scroll to a heading slug (`#anchor`), if found in the active tab.
    unsafe fn scroll_to_anchor(&mut self, hwnd: Hwnd, slug: &str) -> bool {
        let target = self
            .tab()
            .headings
            .iter()
            .find(|h| h.slug.eq_ignore_ascii_case(slug))
            .map(|h| (h.y - self.px(16)).max(0));
        if let Some(y) = target {
            self.glide_to(hwnd, y);
            true
        } else {
            false
        }
    }

    /// Switch to an existing tab index, re-laying out ONLY if window/DPI/zoom/outline changed.
    unsafe fn switch_tab(&mut self, hwnd: Hwnd, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.active = index;
        self.hover_link = None;
        self.outline_hover = None;
        self.selecting = false;
        self.update_window_title(hwnd);
        let sig = (
            self.width,
            self.sidebar_w(),
            self.dpi,
            self.zoom,
            self.pinned,
        );
        if self.tab().layout_sig != sig {
            self.layout(hwnd);
        } else {
            self.update_scroll(hwnd);
            self.place_floater();
            self.buffer = None;
            InvalidateRect(hwnd, ptr::null(), 0);
        }
    }

    /// Close tab at `index`. Closing the last file tab resets to a clean Welcome tab.
    unsafe fn close_tab(&mut self, hwnd: Hwnd, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            if self.tabs[0].file.is_none() {
                return;
            }
            let mut welcome = Tab::new("Welcome");
            welcome.source = WELCOME_DOC.to_owned();
            welcome.set_document(WELCOME_DOC, self.zoom, self.pinned);
            self.tabs[0] = welcome;
            self.active = 0;
            self.update_window_title(hwnd);
            self.layout(hwnd);
            return;
        }
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        let next = self.active;
        self.switch_tab(hwnd, next);
    }

    unsafe fn toggle_outline(&mut self, hwnd: Hwnd) {
        self.outline_open = !self.outline_open;
        self.outline_hover = None;
        self.layout(hwnd);
    }

    unsafe fn update_window_title(&self, hwnd: Hwnd) {
        let title = match &self.tab().file {
            Some(path) => format!(
                "{} — MDLite",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            None => "MDLite".to_owned(),
        };
        SetWindowTextW(hwnd, wide(&title).as_ptr());
    }

    /// Show a palette-themed right-click context menu in the document viewport.
    unsafe fn show_context_menu(&mut self, hwnd: Hwnd, client_x: i32, client_y: i32) {
        let has_sel = self.tab().normalized_selection().is_some();
        let (_, link) = self.hit_doc(hwnd, client_x, client_y);
        self.context_link = link.clone();

        let popup = CreatePopupMenu();
        if popup.is_null() {
            return;
        }
        let mut labels: Vec<*mut Box<[u16]>> = Vec::new();
        let mut add = |flags: u32, id: usize, label: &str| {
            let text = Box::into_raw(Box::new(label.encode_utf16().collect::<Box<[u16]>>()));
            labels.push(text);
            AppendMenuW(popup, 0x100 | flags, id, text as *const u16);
        };
        let disabled = if has_sel { 0 } else { 1 | 2 }; // MF_GRAYED | MF_DISABLED
        add(disabled, CTX_COPY_PLAIN, "Copy\tCtrl+C");
        add(disabled, CTX_COPY_MD, "Copy as Markdown\tCtrl+Shift+C");
        if link.is_some() {
            add(1, 0, "");
            add(0, CTX_OPEN_LINK, "Open Link");
            add(0, CTX_COPY_LINK, "Copy Link Address");
        }
        add(1, 0, "");
        add(0, CTX_SELECT_ALL, "Select All\tCtrl+A");

        let palette = self.palette();
        MENU.set((self.menu_font, palette, self.dpi));
        let brush = CreateSolidBrush(palette.background);
        let info = MenuInfo {
            size: std::mem::size_of::<MenuInfo>() as u32,
            mask: 0x02,
            back: brush,
            ..Default::default()
        };
        SetMenuInfo(popup, &info);

        let mut pt = Point {
            x: client_x,
            y: client_y,
        };
        ClientToScreen(hwnd, &mut pt);
        // TPM_RIGHTBUTTON | TPM_RETURNCMD so we execute immediately without re-borrowing state.
        let cmd = TrackPopupMenu(popup, 0x0002 | 0x0100, pt.x, pt.y, 0, hwnd, ptr::null()) as usize;
        DestroyMenu(popup);
        DeleteObject(brush);
        for label in labels {
            drop(Box::from_raw(label));
        }

        match cmd {
            CTX_COPY_PLAIN => {
                let text = self.tab().selected_plain();
                clipboard::copy_text(hwnd, &text);
            }
            CTX_COPY_MD => {
                let text = self.tab().selected_markdown();
                clipboard::copy_text(hwnd, &text);
            }
            CTX_SELECT_ALL => {
                self.select_all();
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            CTX_COPY_LINK => {
                if let Some(url) = self.context_link.as_deref() {
                    clipboard::copy_text(hwnd, url);
                }
            }
            CTX_OPEN_LINK => {
                // Defer link opening via WM_COMMAND so `state` borrow is clean if a new file loads.
                PostMessageW(hwnd, WM_COMMAND, CTX_OPEN_LINK, 0);
            }
            _ => {}
        }
    }

    /// Build the menu bar from the palettes the INI defines.
    unsafe fn build_menu(&mut self, hwnd: Hwnd) {
        if self.menu_font.is_null() {
            self.menu_font = ui_font(self.ui_px(12));
        }
        let previous = GetMenu(hwnd);
        for label in self.menu_labels.drain(..) {
            drop(Box::from_raw(label));
        }
        let mut item = |menu: Handle, flags: u32, id: usize, label: &str| {
            let text = Box::into_raw(Box::new(label.encode_utf16().collect::<Box<[u16]>>()));
            self.menu_labels.push(text);
            AppendMenuW(menu, 0x100 | flags, id, text as *const u16);
        };
        let (bar, file, options) = (CreateMenu(), CreatePopupMenu(), CreatePopupMenu());
        for (id, label) in [
            (OPEN, "&Open…\tCtrl+O"),
            (OPEN_NEW_TAB, "Open in &New Tab…\tCtrl+T"),
            (CLOSE_TAB, "&Close Tab\tCtrl+W"),
            (RELOAD, "&Reload\tF5"),
            (EXIT, "E&xit\tAlt+F4"),
        ] {
            item(file, 0, id, label);
        }
        let names: Vec<String> = self
            .themes
            .palettes
            .iter()
            .take(PALETTES)
            .map(|(name, _)| name.clone())
            .collect();
        for (index, name) in names.iter().enumerate() {
            item(options, 0, PALETTE + index, name);
        }
        item(options, 1, 0, "");
        item(options, 0, TOGGLE_OUTLINE, "Toggle &Outline\tCtrl+B");
        item(options, 1, 0, "");
        for (id, _, _, label) in ZOOM {
            item(options, 0, id, label);
        }
        item(options, 1, 0, "");
        item(options, 0, SETTINGS, "&Settings…\tCtrl+,");
        item(options, 0, RELOAD_THEME, "&Reload theme file\tCtrl+F5");
        item(bar, 0x10, file as usize, "&File");
        item(bar, 0x10, options as usize, "&Options");
        MENU.set((self.menu_font, self.palette(), self.dpi));
        SetMenu(hwnd, bar);
        if !previous.is_null() {
            DestroyMenu(previous);
        }
        self.menu = options;
    }

    /// Put the floating zoom control in the far corner of the document viewport.
    unsafe fn place_floater(&mut self) {
        if self.floater.is_null() {
            return;
        }
        let doc_left = self.doc_left();
        let doc_top = self.doc_top();
        let doc_w = self.bar().1 - doc_left;
        let showing = self.floating && doc_w > self.ui_px(200) && self.height > self.ui_px(60);
        let (width, height) = (self.ui_px(114), self.ui_px(30));
        let inset = self.ui_px(14);
        let x = if self.tab().stats.rtl() {
            doc_left + inset
        } else {
            self.bar().1 - inset - width
        };
        SetWindowPos(self.floater, NULL, x, doc_top + inset, width, height, 0x4);
        let radius = self.ui_px(14).max(6);
        let rgn = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius);
        if !rgn.is_null() {
            SetWindowRgn(self.floater, rgn, 1);
        }
        ShowWindow(self.floater, showing.into());
        SetWindowLongPtrW(self.floater, size_of::<isize>() as i32, self.zoom as isize);
        InvalidateRect(self.floater, ptr::null(), 0);
    }

    unsafe fn apply_theme(&mut self, hwnd: Hwnd) {
        self.apply_theme_inner(hwnd, true);
    }

    unsafe fn apply_theme_inner(&mut self, hwnd: Hwnd, repaint_frame: bool) {
        let last = self.themes.palettes.len() - 1;
        CheckMenuRadioItem(
            self.menu,
            PALETTE as u32,
            (PALETTE + last) as u32,
            (PALETTE + self.theme.min(last)) as u32,
            0,
        );
        let palette = self.palette();
        MENU.set((self.menu_font, palette, self.dpi));
        self.place_floater();
        apply_window_frame_theme(hwnd, palette);
        let popup_brush = CreateSolidBrush(palette.background);
        let bar_brush = CreateSolidBrush(palette.sidebar());
        let popup_info = MenuInfo {
            size: std::mem::size_of::<MenuInfo>() as u32,
            mask: 0x02 | 0x80000000,
            back: popup_brush,
            ..Default::default()
        };
        let bar_info = MenuInfo {
            size: std::mem::size_of::<MenuInfo>() as u32,
            mask: 0x02,
            back: bar_brush,
            ..Default::default()
        };
        let top_menu = GetMenu(hwnd);
        if !top_menu.is_null() {
            // Apply popup brush recursively first, then override the top menu bar
            // itself with `palette.sidebar()` so Title Bar -> Menu Bar -> Tab Bar
            // form one seamless unified header surface.
            SetMenuInfo(top_menu, &popup_info);
            SetMenuInfo(top_menu, &bar_info);
        }
        if !self.menu.is_null() {
            SetMenuInfo(self.menu, &popup_info);
        }
        if !self.menu_brush.is_null() {
            DeleteObject(self.menu_brush);
        }
        if !self.bar_brush.is_null() {
            DeleteObject(self.bar_brush);
        }
        self.menu_brush = popup_brush;
        self.bar_brush = bar_brush;
        if repaint_frame {
            DrawMenuBar(hwnd);
            SetWindowPos(hwnd, NULL, 0, 0, 0, 0, 0x27);
            paint_menu_seam(hwnd, palette);
            InvalidateRect(hwnd, ptr::null(), 0);
        }
    }

    /// Persist every edited palette and viewer preference.
    unsafe fn save(&self) -> bool {
        let mut saved = true;
        let path = wide(&self.theme_path.to_string_lossy());
        let section = wide("viewer");
        let (name, _) = &self.themes.palettes[self.theme.min(self.themes.palettes.len() - 1)];
        for (key, value) in [
            (
                "renderer",
                self.pinned.map_or("auto", Pipe::name).to_owned(),
            ),
            ("theme", name.clone()),
            ("smooth", if self.smooth { "on" } else { "off" }.to_owned()),
            ("zoom", self.zoom.to_string()),
            (
                "floating",
                if self.floating { "on" } else { "off" }.to_owned(),
            ),
            (
                "outline",
                if self.outline_open { "on" } else { "off" }.to_owned(),
            ),
        ] {
            saved &= WritePrivateProfileStringW(
                section.as_ptr(),
                wide(key).as_ptr(),
                wide(&value).as_ptr(),
                path.as_ptr(),
            ) != 0;
        }
        for (name, palette) in &self.themes.palettes {
            let section = wide(name);
            for (index, (key, _)) in Palette::ROLES.iter().enumerate() {
                let color = palette.role(index);
                let hex = format!(
                    "#{:02X}{:02X}{:02X}",
                    color & 0xff,
                    (color >> 8) & 0xff,
                    (color >> 16) & 0xff
                );
                saved &= WritePrivateProfileStringW(
                    section.as_ptr(),
                    wide(key).as_ptr(),
                    wide(&hex).as_ptr(),
                    path.as_ptr(),
                ) != 0;
            }
        }
        saved
    }
}

const WELCOME_DOC: &str = "# MDLite\n\n\
Open a Markdown document with **Ctrl+O**, press **Ctrl+T** for a new tab, or drop a `.md` file here.\n\n\
- **Outline** — Toggle the heading sidebar with **Ctrl+B** or the top-left button.\n\
- **Select & Copy** — Drag to select text; **Ctrl+C** copies plain text, right-click for **Copy as Markdown**.\n\
- **Links** — Click web links to open in your browser, `#headings` to jump, or `.md` links to open in a new tab.\n\
- **Themes** — Press **Ctrl+D** to cycle through 10 built-in palettes, or **Ctrl+,** for Settings.";

/// The zoom control that floats over the document.
unsafe extern "system" fn floater_proc(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize {
    let hovered = GetWindowLongPtrW(hwnd, 0);
    match msg {
        WM_PAINT => {
            let mut paint: PaintStruct = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            let (font, palette, dpi) = MENU.get();
            let px = |n: i32| ((n as i64 * dpi as i64) / 96) as i32;
            let mut client = Rect::default();
            GetClientRect(hwnd, &mut client);
            let r = px(7).max(4);
            fill(dc, client, palette.background);
            fill_rounded(dc, client, r, palette.surface(), palette.border());
            let old = SelectObject(dc, font);
            SetBkMode(dc, 1);
            let zoom = GetWindowLongPtrW(hwnd, size_of::<isize>() as i32) as i32;
            let width = (client.right / 3).max(1);
            for (index, (_, _, label, _)) in ZOOM.iter().enumerate() {
                let text = wide(&label.replace('%', &format!("{zoom}%")));
                let count = text.len() as i32 - 1;
                let mut cell = Rect {
                    left: width * index as i32,
                    top: client.top,
                    right: if index == 2 {
                        client.right
                    } else {
                        width * (index as i32 + 1)
                    },
                    bottom: client.bottom,
                };
                if hovered == index as isize {
                    let pill = Rect {
                        left: cell.left + px(2),
                        top: cell.top + px(2),
                        right: cell.right - px(2),
                        bottom: cell.bottom - px(2),
                    };
                    fill_rounded(dc, pill, px(5).max(3), palette.accent, palette.accent);
                }
                SetTextColor(
                    dc,
                    if hovered == index as isize {
                        Palette::contrasting(palette.accent)
                    } else if index == 1 {
                        palette.foreground
                    } else {
                        palette.accent
                    },
                );
                cell.top += px(1);
                DrawTextW(dc, text.as_ptr(), count, &mut cell, 0x20 | 0x4 | 0x1);
            }
            SelectObject(dc, old);
            EndPaint(hwnd, &paint);
            0
        }
        WM_MOUSEMOVE => {
            let mut client = Rect::default();
            GetClientRect(hwnd, &mut client);
            let cell = (((lp as i16) as i32) / (client.right / 3).max(1)).clamp(0, 2) as isize;
            if cell != hovered {
                SetWindowLongPtrW(hwnd, 0, cell);
                InvalidateRect(hwnd, ptr::null(), 0);
                let mut track = TrackMouse {
                    size: size_of::<TrackMouse>() as u32,
                    flags: 2,
                    window: hwnd,
                    time: 0,
                };
                TrackMouseEvent(&mut track);
            }
            0
        }
        WM_MOUSELEAVE => {
            SetWindowLongPtrW(hwnd, 0, -1);
            InvalidateRect(hwnd, ptr::null(), 0);
            0
        }
        WM_LBUTTONDOWN => {
            let mut client = Rect::default();
            GetClientRect(hwnd, &mut client);
            let cell = (((lp as i16) as i32) / (client.right / 3).max(1)).clamp(0, 2) as usize;
            PostMessageW(GetParent(hwnd), WM_COMMAND, ZOOM[cell].0, 0);
            0
        }
        WM_ERASEBKGND => 1,
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Resolved modern Windows system font families (probed once, zero overhead).
struct FontFaces {
    ui: &'static str,
    display: &'static str,
    code: &'static str,
}
unsafe fn font_installed(name: &str) -> bool {
    let dc = GetDC(NULL);
    if dc.is_null() {
        return false;
    }
    let h = CreateFontW(
        -16,
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        1,
        0,
        0,
        5,
        0,
        wide(name).as_ptr(),
    );
    let mut matched = false;
    if !h.is_null() {
        let old = SelectObject(dc, h);
        let mut actual = [0u16; 64];
        let len = GetTextFaceW(dc, actual.len() as i32, actual.as_mut_ptr()).max(0) as usize;
        let face = String::from_utf16_lossy(&actual[..len]);
        matched = face.eq_ignore_ascii_case(name);
        SelectObject(dc, old);
        DeleteObject(h);
    }
    ReleaseDC(NULL, dc);
    matched
}
fn modern_fonts() -> &'static FontFaces {
    static FACES: std::sync::OnceLock<FontFaces> = std::sync::OnceLock::new();
    FACES.get_or_init(|| unsafe {
        FontFaces {
            ui: if font_installed("Segoe UI Variable Text") {
                "Segoe UI Variable Text"
            } else {
                "Segoe UI"
            },
            display: if font_installed("Segoe UI Variable Display") {
                "Segoe UI Variable Display"
            } else {
                "Segoe UI"
            },
            code: if font_installed("Cascadia Mono") {
                "Cascadia Mono"
            } else {
                "Consolas"
            },
        }
    })
}

/// Apply unified Windows 11 DWM frame attributes (dark mode, rounded corners,
/// themed outer border, and caption matching `palette.sidebar()`).
unsafe fn apply_window_frame_theme(hwnd: Hwnd, palette: Palette) {
    let dark: i32 = palette.is_dark().into();
    let round: i32 = 2; // DWMWCP_ROUND
    let border_c = palette.border();
    let caption_c = palette.sidebar();
    let text_c = palette.foreground;
    DwmSetWindowAttribute(hwnd, 20, &dark as *const _ as _, 4);
    DwmSetWindowAttribute(hwnd, 33, &round as *const _ as _, 4);
    DwmSetWindowAttribute(hwnd, 34, &border_c as *const _ as _, 4);
    DwmSetWindowAttribute(hwnd, 35, &caption_c as *const _ as _, 4);
    DwmSetWindowAttribute(hwnd, 36, &text_c as *const _ as _, 4);
}

/// Paint over the infamous bright 1px white separator line that Windows draws
/// underneath a classic menu bar, unifying Menu Bar and Tab Bar seamlessly.
unsafe fn paint_menu_seam(hwnd: Hwnd, palette: Palette) {
    if GetMenu(hwnd).is_null() {
        return;
    }
    let mut win_rect = Rect::default();
    if GetWindowRect(hwnd, &mut win_rect) == 0 {
        return;
    }
    let mut origin = Point { x: 0, y: 0 };
    if ClientToScreen(hwnd, &mut origin) == 0 {
        return;
    }
    let mut client = Rect::default();
    GetClientRect(hwnd, &mut client);
    let left = origin.x - win_rect.left;
    let bottom = origin.y - win_rect.top;
    if bottom <= 2 || client.right <= 0 {
        return;
    }
    let dc = GetWindowDC(hwnd);
    if !dc.is_null() {
        fill(
            dc,
            Rect {
                left,
                top: (bottom - 2).max(0),
                right: left + client.right,
                bottom,
            },
            palette.sidebar(),
        );
        ReleaseDC(hwnd, dc);
    }
}

unsafe fn ui_font(height: i32) -> Handle {
    ui_font_weight(height, 400)
}
unsafe fn ui_font_weight(height: i32, weight: i32) -> Handle {
    CreateFontW(
        -height.max(1),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        1,
        0,
        0,
        5,
        0,
        wide(modern_fonts().ui).as_ptr(),
    )
}
unsafe fn register(name: &[u16], proc: Wndproc, background: Handle, extra: i32) -> bool {
    let instance = GetModuleHandleW(ptr::null());
    let icon = match LoadIconW(instance, 1usize as *const u16) {
        icon if icon.is_null() => LoadIconW(NULL, 32512usize as *const u16),
        icon => icon,
    };
    RegisterClassW(&WndClass {
        style: 0x0008, // CS_DBLCLKS: enables word selection and divider double-click reset.
        proc: Some(proc),
        class_extra: 0,
        window_extra: extra,
        instance,
        icon,
        cursor: LoadCursorW(NULL, 32512usize as *const u16),
        background,
        menu_name: ptr::null(),
        class_name: name.as_ptr(),
    }) != 0
}

fn pinned(themes: &Themes) -> Option<Pipe> {
    themes
        .renderer
        .and_then(|index| Pipe::ALL.get(index as usize).copied())
}
fn offset(align: u8, room: i32, used: i32) -> i32 {
    match align {
        1 => ((room - used) / 2).max(0),
        2 => (room - used).max(0),
        _ => 0,
    }
}
fn is_high_surrogate(unit: u16) -> bool {
    (0xd800..=0xdbff).contains(&unit)
}
unsafe fn fill(dc: Hdc, rect: Rect, color: u32) {
    SetDCBrushColor(dc, color);
    FillRect(dc, &rect, GetStockObject(18));
}
/// Draw a rounded rectangle using stock DC brush and pen (zero GDI allocations).
unsafe fn fill_rounded(dc: Hdc, rect: Rect, radius: i32, fill_color: u32, border_color: u32) {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }
    let old_brush = SelectObject(dc, GetStockObject(18)); // DC_BRUSH
    let old_pen = if border_color == fill_color {
        SelectObject(dc, GetStockObject(8)) // NULL_PEN
    } else {
        SetDCPenColor(dc, border_color);
        SelectObject(dc, GetStockObject(19)) // DC_PEN
    };
    SetDCBrushColor(dc, fill_color);
    let d = (radius * 2)
        .min(rect.right - rect.left)
        .min(rect.bottom - rect.top);
    RoundRect(dc, rect.left, rect.top, rect.right, rect.bottom, d, d);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
}

/// Decode UTF-8 (with/without BOM), and BOM-marked UTF-16. Reject binary/invalid input.
fn decode(bytes: Vec<u8>) -> Result<String, String> {
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if bytes.len() % 2 != 0 {
            return Err("The UTF-16 file has an incomplete character.".into());
        }
        let le = bytes[0] == 0xff;
        let words: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|b| {
                if le {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        String::from_utf16(&words).map_err(|_| "The file contains invalid UTF-16.".to_owned())?
    } else {
        String::from_utf8(bytes)
            .map_err(|_| "Save this document as UTF-8 or UTF-16 with a BOM.".to_owned())?
    };
    if text.contains('\0') {
        return Err("This appears to be a binary file, not a text document.".into());
    }
    Ok(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_owned(),
        None => text,
    })
}

unsafe fn error(hwnd: Hwnd, text: &str) {
    MessageBoxW(hwnd, wide(text).as_ptr(), wide("MDLite").as_ptr(), 0x10);
}

fn read_document(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Choose a regular text file.".to_owned());
    }
    const LIMIT: u64 = 64 * 1024 * 1024;
    if meta.len() > LIMIT {
        return Err("The document exceeds the 64 MiB size limit.".to_owned());
    }
    let mut bytes = Vec::with_capacity(meta.len().min(LIMIT) as usize);
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("The document exceeds the 64 MiB size limit.".to_owned());
    }
    decode(bytes)
}

fn same_file(a: &Path, b: &Path) -> bool {
    let na = links::normalize_path(a);
    let nb = links::normalize_path(b);
    na.to_string_lossy()
        .eq_ignore_ascii_case(&nb.to_string_lossy())
}

/// Load `path` either into the active tab (`new_tab == false`) or into a new tab
/// (`new_tab == true`, reusing an existing tab if `path` is already open, or
/// replacing an empty Welcome tab).
unsafe fn load_file_tab(
    hwnd: Hwnd,
    state: &RefCell<App>,
    path: PathBuf,
    new_tab: bool,
    anchor: Option<&str>,
) -> bool {
    let started = Instant::now();
    // If opening in a new tab and this file is already open, jump straight to it!
    if new_tab {
        let existing = state
            .borrow()
            .tabs
            .iter()
            .position(|t| t.file.as_deref().is_some_and(|p| same_file(p, &path)));
        if let Some(idx) = existing {
            let mut app = state.borrow_mut();
            app.switch_tab(hwnd, idx);
            if let Some(slug) = anchor {
                app.scroll_to_anchor(hwnd, slug);
            }
            return true;
        }
    }
    match read_document(&path) {
        Ok(text) => {
            let mut app = state.borrow_mut();
            let title = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let saved_scroll = if !new_tab
                && app
                    .tab()
                    .file
                    .as_deref()
                    .is_some_and(|p| same_file(p, &path))
            {
                app.tab().scroll
            } else {
                0
            };
            if new_tab && app.tab().file.is_some() {
                app.tabs.push(Tab::new(&title));
                app.active = app.tabs.len() - 1;
            }
            app.set_document(&text);
            {
                let tab = app.tab_mut();
                tab.source = text;
                tab.scroll = saved_scroll;
                tab.file = Some(path.clone());
                tab.title = title;
            }
            app.update_window_title(hwnd);
            app.layout(hwnd);
            app.tab_mut().stats.micros = started.elapsed().as_micros();
            if let Some(slug) = anchor {
                app.scroll_to_anchor(hwnd, slug);
            }
            true
        }
        Err(message) => {
            error(
                hwnd,
                &format!("Could not open {}\n\n{}", path.display(), message),
            );
            false
        }
    }
}

/// Activate a clicked link: `#anchor` jumps in-document, `.md` files open in a
/// new tab, and external `http(s)`/`mailto` URLs open in the default browser.
unsafe fn activate_link(hwnd: Hwnd, state: &RefCell<App>, href: &str) {
    let current = state.borrow().tab().file.clone();
    match links::resolve(href, current.as_deref()) {
        Some(LinkAction::Anchor(slug)) => {
            state.borrow_mut().scroll_to_anchor(hwnd, &slug);
        }
        Some(LinkAction::MarkdownFile { path, anchor }) => {
            load_file_tab(hwnd, state, path, true, anchor.as_deref());
        }
        Some(LinkAction::External(url)) => {
            ShellExecuteW(
                hwnd,
                wide("open").as_ptr(),
                wide(&url).as_ptr(),
                ptr::null(),
                ptr::null(),
                5, // SW_SHOW
            );
        }
        None => {}
    }
}

unsafe fn choose_file(hwnd: Hwnd) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let filter = wide("Markdown and text\0*.md;*.markdown;*.mdown;*.mkd;*.txt\0All files\0*.*\0");
    let mut info: OpenFileName = std::mem::zeroed();
    info.size = std::mem::size_of::<OpenFileName>() as u32;
    info.owner = hwnd;
    info.filter = filter.as_ptr();
    info.filter_index = 1;
    info.file = buffer.as_mut_ptr();
    info.max_file = buffer.len() as u32;
    info.flags = 0x80000 | 0x1000 | 0x800 | 8;
    if GetOpenFileNameW(&mut info) == 0 {
        return None;
    }
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(PathBuf::from(OsString::from_wide(&buffer[..length])))
}

/// Show the settings window and pump it until the user closes it.
unsafe fn settings(owner: Hwnd, state: &RefCell<App>) {
    let hwnd = settings_window(owner, state);
    if hwnd.is_null() {
        return;
    }
    ShowWindow(hwnd, 5);
    EnableWindow(owner, 0);
    let mut message = Msg::default();
    while IsWindow(hwnd) != 0 {
        let result = GetMessageW(&mut message, NULL, 0, 0);
        if result < 0 {
            break;
        }
        if result == 0 {
            PostQuitMessage(0);
            break;
        }
        if IsDialogMessageW(hwnd, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    EnableWindow(owner, 1);
    SetForegroundWindow(owner);
    let mut app = state.borrow_mut();
    if std::mem::take(&mut app.restyle) {
        app.rebuild(owner);
    } else {
        app.layout(owner);
    }
    app.build_menu(owner);
    app.apply_theme(owner);
    app.place_floater();
}

const DEFAULT_HINT: &str = "Registers MDLite, then opens Windows Settings so you can choose it.";
const DEFAULT_DONE: &str = "Registered. In Windows Settings, choose MDLite for .md and .markdown.";

struct SettingsVisuals {
    font: Handle,
    bold_font: Handle,
    bg_brush: Handle,
    surface_brush: Handle,
    input_brush: Handle,
    cards: [Rect; 4],
    dpi: i32,
}
impl Drop for SettingsVisuals {
    fn drop(&mut self) {
        unsafe {
            for h in [
                &mut self.font,
                &mut self.bold_font,
                &mut self.bg_brush,
                &mut self.surface_brush,
                &mut self.input_brush,
            ] {
                let obj = std::mem::replace(h, NULL);
                if !obj.is_null() {
                    DeleteObject(obj);
                }
            }
        }
    }
}

unsafe fn apply_settings_theme(hwnd: Hwnd, palette: Palette) {
    apply_window_frame_theme(hwnd, palette);
    let ptr = GetWindowLongPtrW(hwnd, 0) as *mut SettingsVisuals;
    if !ptr.is_null() {
        let v = &mut *ptr;
        let bg = CreateSolidBrush(palette.background);
        let surf = CreateSolidBrush(palette.surface());
        let inp = CreateSolidBrush(palette.code_background);
        if !v.bg_brush.is_null() {
            DeleteObject(v.bg_brush);
        }
        if !v.surface_brush.is_null() {
            DeleteObject(v.surface_brush);
        }
        if !v.input_brush.is_null() {
            DeleteObject(v.input_brush);
        }
        v.bg_brush = bg;
        v.surface_brush = surf;
        v.input_brush = inp;
    }
    let dark = palette.is_dark();
    let mut child = GetWindow(hwnd, 5); // GW_CHILD
    while !child.is_null() {
        let mut cls = [0u16; 32];
        let len = GetClassNameW(child, cls.as_mut_ptr(), 32).max(0) as usize;
        let class = String::from_utf16_lossy(&cls[..len]);
        let style = GetWindowLongPtrW(child, -16) as u32;
        if class.eq_ignore_ascii_case("ComboBox") {
            set_window_theme(child, if dark { "DarkMode_CFD" } else { "Explorer" }, None);
        } else if class.eq_ignore_ascii_case("Edit") {
            set_window_theme(
                child,
                if dark {
                    "DarkMode_Explorer"
                } else {
                    "Explorer"
                },
                None,
            );
        } else if class.eq_ignore_ascii_case("Button") && matches!(style & 0xf, 2..=7 | 9) {
            // Strip classic Win32 gray visual style on checkboxes/radios/groupboxes so
            // WM_CTLCOLORSTATIC/BTN renders theme foreground on surface cleanly.
            set_window_theme(child, "", Some(""));
        }
        InvalidateRect(child, ptr::null(), 1);
        child = GetWindow(child, 2); // GW_HWNDNEXT
    }
    InvalidateRect(hwnd, ptr::null(), 1);
}

unsafe fn settings_window(owner: Hwnd, state: &RefCell<App>) -> Hwnd {
    settings_window_at(owner, state, GetDpiForWindow(owner).max(96) as i32)
}

unsafe fn settings_window_at(owner: Hwnd, state: &RefCell<App>, dpi: i32) -> Hwnd {
    const STYLE: u32 = 0x8000_0000 | 0x00c0_0000 | 0x0008_0000;
    const CHILD: u32 = 0x5000_0000;
    const RADIO: u32 = 9;
    const CHECK: u32 = 3;
    const OWNER_BTN: u32 = 0x0b;
    const DROPLIST: u32 = 3 | 0x0020_0000;
    const WRAP: u32 = 0x2000;
    const GROUP: u32 = 0x0002_0000;
    const TAB: u32 = 0x0001_0000;
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    let class = wide("MDLite.Settings");
    let instance = GetModuleHandleW(ptr::null());
    REGISTERED.call_once(|| {
        register(&class, settings_proc, NULL, size_of::<Handle>() as i32);
    });
    let s = |n: i32| (n as i64 * dpi as i64 / 96) as i32;
    let hwnd = CreateWindowExW(
        1,
        class.as_ptr(),
        wide("Settings").as_ptr(),
        STYLE,
        0,
        0,
        s(100),
        s(100),
        owner,
        NULL,
        instance,
        ptr::null_mut(),
    );
    if hwnd.is_null() {
        return NULL;
    }
    SetWindowLongPtrW(hwnd, -21, state as *const RefCell<App> as isize);
    let font = ui_font(s(13));
    let bold_font = ui_font_weight(s(13), 600);
    let screen = GetDC(NULL);
    let previous = SelectObject(screen, font);
    let mut metric = TextMetric::default();
    GetTextMetricsW(screen, &mut metric);
    let line = metric.height;
    let words = |text: &str, width: i32| {
        let text = wide(text);
        let mut rect = Rect {
            left: 0,
            top: 0,
            right: width.max(1),
            bottom: 0,
        };
        DrawTextW(screen, text.as_ptr(), -1, &mut rect, 0x400 | 0x10);
        rect.bottom
    };
    let across = |text: &str| {
        let text = wide(&text.replace('&', ""));
        let mut size = Size::default();
        GetTextExtentPoint32W(screen, text.as_ptr(), text.len() as i32 - 1, &mut size);
        size.cx
    };
    let add =
        |class: &str, text: &str, style: u32, (x, y, w, h): (i32, i32, i32, i32), id: usize| {
            let handle = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                CHILD | style,
                x,
                y,
                w,
                h,
                hwnd,
                id as Handle,
                instance,
                ptr::null_mut(),
            );
            SendMessageW(handle, 0x30, font as usize, 1);
            handle
        };
    let (margin, gap, pad, column) = (s(16), s(10), s(14), s(340));
    let field = line + s(10);
    let button = line + s(14);
    let heading = line + s(8);
    let indicator = s(22);
    let inner = column - 2 * pad;
    let option = |text: &str, style: u32, x: i32, y: i32, id: usize| {
        let height = field.max(words(text, inner - indicator) + s(8));
        add("BUTTON", text, style | WRAP, (x, y, inner, height), id);
        height
    };
    let label_width = ["&Palette", "&New", "&Text size"]
        .iter()
        .map(|text| across(text))
        .max()
        .unwrap_or(0)
        + s(10);
    let caption = |text: &str, x: i32, y: i32| {
        add(
            "STATIC",
            text,
            0,
            (x, y + (field - line) / 2, label_width, line),
            0,
        );
    };
    let (chosen, smooth, floating, outline_open, theme, palette, names, breakdown, zoom) = {
        let mut app = state.borrow_mut();
        app.restore = Some((app.themes.clone(), app.theme));
        (
            app.pinned,
            app.smooth,
            app.floating,
            app.outline_open,
            app.theme,
            app.palette(),
            app.themes
                .palettes
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
            app.tab().stats.breakdown(app.pinned),
            app.zoom,
        )
    };

    // Left column: Appearance.
    let (x, mut y) = (margin, margin);
    let appearance = add("BUTTON", "Appearance", 7, (x, y, column, 0), 0);
    SendMessageW(appearance, 0x30, bold_font as usize, 1);
    let top = y;
    let (ix, mut y2) = (x + pad, y + heading);
    caption("&Palette", ix, y2);
    let list = add(
        "COMBOBOX",
        "",
        DROPLIST | GROUP | TAB,
        (ix + label_width, y2, inner - label_width, field + 8 * line),
        CHOICE,
    );
    y2 += field + gap;
    caption("&New", ix, y2);
    let add_width = (across("Add") + s(28)).max(s(70));
    add(
        "EDIT",
        "",
        0x0080_0000 | 0x80 | TAB,
        (
            ix + label_width,
            y2,
            inner - label_width - add_width - gap,
            field,
        ),
        NAME,
    );
    SendMessageW(GetDlgItem(hwnd, NAME as i32), 0xc5, 32, 0);
    add(
        "BUTTON",
        "&Add",
        OWNER_BTN | TAB,
        (ix + inner - add_width, y2, add_width, field),
        ADD,
    );
    y2 += field + gap;
    for name in &names {
        SendMessageW(list, 0x143, 0, wide(name).as_ptr() as isize);
    }
    SendMessageW(list, 0x14e, theme, 0);
    let cell = inner / 2;
    let swatch = (s(28), line + s(6));
    let name_width = cell - swatch.0 - s(8);
    let row = Palette::ROLES
        .iter()
        .map(|(_, label)| words(label, name_width))
        .max()
        .unwrap_or(0)
        .max(swatch.1)
        + s(6);
    for (index, (_, label)) in Palette::ROLES.iter().enumerate() {
        let (column, line_no) = (index as i32 / 4, index as i32 % 4);
        let (sx, sy) = (ix + column * cell, y2 + line_no * row);
        add(
            "BUTTON",
            "",
            OWNER_BTN | TAB,
            (sx, sy, swatch.0, swatch.1),
            SWATCH + index,
        );
        let text = words(label, name_width);
        add(
            "STATIC",
            label,
            0,
            (
                sx + swatch.0 + s(8),
                sy + (swatch.1 - text) / 2,
                name_width,
                text,
            ),
            0,
        );
    }
    y2 += 4 * row + s(2);
    let hint = "Colour changes preview on the page.";
    let hint_height = words(hint, inner);
    add("STATIC", hint, 0, (ix, y2, inner, hint_height), 0);
    y2 += hint_height + pad;
    let card0 = Rect {
        left: x,
        top,
        right: x + column,
        bottom: y2,
    };
    SetWindowPos(appearance, NULL, x, top, column, y2 - top, 0x14);
    y = y2 + gap;

    // Markdown files box.
    let files = add("BUTTON", "Markdown files", 7, (x, y, column, 0), 0);
    SendMessageW(files, 0x30, bold_font as usize, 1);
    let top = y;
    y2 = y + heading;
    add(
        "BUTTON",
        "Set as &default .md reader…",
        OWNER_BTN | TAB | GROUP,
        (ix, y2, inner, button),
        DEFAULT_READER,
    );
    y2 += button + s(8);
    let status = words(DEFAULT_HINT, inner).max(words(DEFAULT_DONE, inner));
    add(
        "STATIC",
        DEFAULT_HINT,
        0,
        (ix, y2, inner, status),
        DEFAULT_STATUS,
    );
    y2 += status + pad;
    let card1 = Rect {
        left: x,
        top,
        right: x + column,
        bottom: y2,
    };
    SetWindowPos(files, NULL, x, top, column, y2 - top, 0x14);
    let left_bottom = y2;

    // Right column: Reading comfort.
    let (x, mut y) = (margin + column + gap, margin);
    let comfort = add("BUTTON", "Reading comfort", 7, (x, y, column, 0), 0);
    SendMessageW(comfort, 0x30, bold_font as usize, 1);
    let top = y;
    let (ix, mut y2) = (x + pad, y + heading);
    let scrolling = option("&Smooth scrolling", CHECK | GROUP | TAB, ix, y2, SMOOTH);
    SendMessageW(GetDlgItem(hwnd, SMOOTH as i32), 0xf1, smooth as usize, 0);
    y2 += scrolling + s(4);
    let zoom_control = option("Show floating &zoom control", CHECK | TAB, ix, y2, FLOAT);
    SendMessageW(GetDlgItem(hwnd, FLOAT as i32), 0xf1, floating as usize, 0);
    y2 += zoom_control + s(4);
    let outline_ctl = option(
        "Show heading &outline sidebar",
        CHECK | TAB,
        ix,
        y2,
        OUTLINE_OPT,
    );
    SendMessageW(
        GetDlgItem(hwnd, OUTLINE_OPT as i32),
        0xf1,
        outline_open as usize,
        0,
    );
    y2 += outline_ctl + s(8);
    caption("Text &size", ix, y2);
    let sizes = add(
        "COMBOBOX",
        "",
        DROPLIST | TAB,
        (ix + label_width, y2, inner - label_width, field + 8 * line),
        TEXT_SIZE,
    );
    for (index, size) in (Themes::ZOOM.0..=Themes::ZOOM.1).enumerate() {
        SendMessageW(sizes, 0x143, 0, wide(&format!("{size}%")).as_ptr() as isize);
        if size == zoom {
            SendMessageW(sizes, 0x14e, index, 0);
        }
    }
    y2 += field + pad;
    let card2 = Rect {
        left: x,
        top,
        right: x + column,
        bottom: y2,
    };
    SetWindowPos(comfort, NULL, x, top, column, y2 - top, 0x14);
    y = y2 + gap;

    // Advanced rendering box.
    let rendering = add("BUTTON", "Advanced rendering", 7, (x, y, column, 0), 0);
    SendMessageW(rendering, 0x30, bold_font as usize, 1);
    let top = y;
    y2 = y + heading;
    let pipes = [(PIPE, "&Automatic (recommended)")]
        .into_iter()
        .chain(Pipe::ALL.map(|pipe| (PIPE + 1 + pipe as usize, pipe.label())));
    for (index, (id, label)) in pipes.enumerate() {
        let style = RADIO | TAB | if index == 0 { GROUP } else { 0 };
        let height = option(label, style, ix, y2, id);
        let on = chosen.map_or(index == 0, |pipe| index == 1 + pipe as usize);
        SendMessageW(GetDlgItem(hwnd, id as i32), 0xf1, on as usize, 0);
        y2 += height + s(2);
    }
    y2 += s(6);
    let note = [None, Some(Pipe::Gdi), Some(Pipe::GdiPlus), Some(Pipe::D2d)]
        .into_iter()
        .map(|pipe| words(Pipe::note(pipe), inner))
        .max()
        .unwrap_or(0);
    add("STATIC", Pipe::note(chosen), 0, (ix, y2, inner, note), NOTE);
    y2 += note + s(6);
    let sample = "This document: 88888 GDI, 88888 GDI+, 88888 Direct2D blocks.";
    let spread = words(sample, inner).max(words(&breakdown, inner));
    add("STATIC", &breakdown, 0, (ix, y2, inner, spread), SPREAD);
    y2 += spread + pad;
    let card3 = Rect {
        left: x,
        top,
        right: x + column,
        bottom: y2,
    };
    SetWindowPos(rendering, NULL, x, top, column, y2 - top, 0x14);
    let right_bottom = y2;

    // Closing buttons row.
    let y = left_bottom.max(right_bottom) + gap;
    let width = |text: &str| (across(text) + s(32)).max(s(88));
    let (save, cancel) = (width("Save"), width("Cancel"));
    let right = margin + 2 * column + gap;
    let save_btn = add(
        "BUTTON",
        "Save",
        OWNER_BTN | GROUP | TAB,
        (right - cancel - gap - save, y, save, button),
        OK,
    );
    SendMessageW(save_btn, 0x30, bold_font as usize, 1);
    add(
        "BUTTON",
        "Cancel",
        OWNER_BTN | TAB,
        (right - cancel, y, cancel, button),
        CANCEL,
    );
    let note = "Cancel discards appearance changes.";
    let room = right - cancel - gap - save - gap - margin;
    let height = words(note, room);
    add(
        "STATIC",
        note,
        0,
        (margin, y + (button - height) / 2, room, height),
        0,
    );

    SelectObject(screen, previous);
    ReleaseDC(NULL, screen);
    let mut rect = Rect {
        left: 0,
        top: 0,
        right: right + margin,
        bottom: y + button + margin,
    };
    AdjustWindowRectExForDpi(&mut rect, STYLE, 0, 1, dpi as u32);
    let mut frame = Rect::default();
    GetWindowRect(owner, &mut frame);
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    SetWindowPos(
        hwnd,
        NULL,
        frame.left + (frame.right - frame.left - width) / 2,
        frame.top + (frame.bottom - frame.top - height) / 3,
        width,
        height,
        0x14,
    );
    let visuals = Box::into_raw(Box::new(SettingsVisuals {
        font,
        bold_font,
        bg_brush: NULL,
        surface_brush: NULL,
        input_brush: NULL,
        cards: [card0, card1, card2, card3],
        dpi,
    }));
    SetWindowLongPtrW(hwnd, 0, visuals as isize);
    apply_settings_theme(hwnd, palette);
    hwnd
}

unsafe fn preview(hwnd: Hwnd, state: &RefCell<App>) {
    let owner = GetWindowLongPtrW(hwnd, -8) as Hwnd;
    let palette = state.borrow().palette();
    if !owner.is_null() {
        state.borrow_mut().apply_theme(owner);
    }
    apply_settings_theme(hwnd, palette);
}

unsafe extern "system" fn settings_proc(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize {
    let state = GetWindowLongPtrW(hwnd, -21) as *const RefCell<App>;
    let visuals = GetWindowLongPtrW(hwnd, 0) as *const SettingsVisuals;
    if msg == WM_CLOSE {
        DestroyWindow(hwnd);
        return 0;
    }
    if msg == WM_ERASEBKGND && !state.is_null() && !visuals.is_null() {
        let Ok(app) = (*state).try_borrow() else {
            return 1;
        };
        let palette = app.palette();
        let v = &*visuals;
        let dc = wp as Hdc;
        let mut client = Rect::default();
        GetClientRect(hwnd, &mut client);
        fill(dc, client, palette.background);
        let radius = ((8i64 * v.dpi as i64) / 96).max(4) as i32;
        let pad_top = ((6i64 * v.dpi as i64) / 96).max(3) as i32;
        for card in v.cards {
            let r = Rect {
                left: card.left,
                top: card.top + pad_top,
                right: card.right,
                bottom: card.bottom,
            };
            fill_rounded(dc, r, radius, palette.surface(), palette.border());
        }
        return 1;
    }
    if matches!(
        msg,
        WM_CTLCOLORDLG | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX
    ) && !state.is_null()
        && !visuals.is_null()
    {
        let Ok(app) = (*state).try_borrow() else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        let palette = app.palette();
        let v = &*visuals;
        let dc = wp as Hdc;
        if matches!(msg, WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX) {
            SetTextColor(dc, palette.foreground);
            SetBkColor(dc, palette.code_background);
            return v.input_brush as isize;
        }
        if msg == WM_CTLCOLORDLG {
            return v.bg_brush as isize;
        }
        let ctrl = lp as Hwnd;
        let ctrl_id = GetWindowLongPtrW(ctrl, -12) as usize;
        let mut wr = Rect::default();
        let mut in_card = false;
        if GetWindowRect(ctrl, &mut wr) != 0 {
            let mut pt = Point {
                x: (wr.left + wr.right) / 2,
                y: (wr.top + wr.bottom) / 2,
            };
            ScreenToClient(hwnd, &mut pt);
            in_card = v
                .cards
                .iter()
                .any(|c| pt.x >= c.left && pt.x <= c.right && pt.y >= c.top && pt.y <= c.bottom);
        }
        let style = GetWindowLongPtrW(ctrl, -16) as u32;
        let mut cls = [0u16; 16];
        let clen = GetClassNameW(ctrl, cls.as_mut_ptr(), 16).max(0) as usize;
        let is_group = String::from_utf16_lossy(&cls[..clen]).eq_ignore_ascii_case("Button")
            && (style & 0xf == 7);
        let is_muted = !in_card || matches!(ctrl_id, NOTE | SPREAD | DEFAULT_STATUS);
        let bg = if in_card {
            palette.surface()
        } else {
            palette.background
        };
        let fg = if is_group {
            palette.accent
        } else if is_muted {
            palette.muted
        } else {
            palette.foreground
        };
        SetBkMode(dc, 1);
        SetBkColor(dc, bg);
        SetTextColor(dc, fg);
        return if in_card {
            v.surface_brush as isize
        } else {
            v.bg_brush as isize
        };
    }
    if msg == WM_DRAWITEM && !state.is_null() {
        let item = &*(lp as *const DrawItem);
        let Ok(app) = (*state).try_borrow() else {
            return 1;
        };
        let palette = app.palette();
        let dpi = if !visuals.is_null() {
            (*visuals).dpi
        } else {
            96
        };
        let s = |n: i32| ((n as i64 * dpi as i64) / 96).max(1) as i32;
        let id = item.id as usize;
        if let Some(role) = id.checked_sub(SWATCH).filter(|r| *r < 7) {
            fill(item.dc, item.rect, palette.surface());
            let color = palette.role(role);
            let focused = item.state & 0x10 != 0;
            let ring = if focused {
                palette.accent
            } else {
                palette.border()
            };
            fill_rounded(item.dc, item.rect, s(5), color, ring);
            return 1;
        }
        if matches!(id, OK | CANCEL | ADD | DEFAULT_READER) {
            let pressed = item.state & 1 != 0;
            let focused = item.state & 0x10 != 0;
            let outer_bg = if matches!(id, OK | CANCEL) {
                palette.background
            } else {
                palette.surface()
            };
            fill(item.dc, item.rect, outer_bg);
            let (bg, border_c, fg) = if id == OK {
                let fill_c = if pressed {
                    palette.hover(palette.accent)
                } else {
                    palette.accent
                };
                (fill_c, fill_c, Palette::contrasting(palette.accent))
            } else {
                let base = if matches!(id, CANCEL) {
                    palette.surface()
                } else {
                    palette.background
                };
                let fill_c = if pressed { palette.hover(base) } else { base };
                let bc = if focused {
                    palette.accent
                } else {
                    palette.border()
                };
                (fill_c, bc, palette.foreground)
            };
            fill_rounded(item.dc, item.rect, s(6), bg, border_c);
            let font = if !visuals.is_null() {
                if id == OK {
                    (*visuals).bold_font
                } else {
                    (*visuals).font
                }
            } else {
                NULL
            };
            let old = if !font.is_null() {
                SelectObject(item.dc, font)
            } else {
                NULL
            };
            SetBkMode(item.dc, 1);
            SetTextColor(item.dc, fg);
            let mut label = [0u16; 128];
            let len = GetWindowTextW(item.window, label.as_mut_ptr(), 128).max(0) as i32;
            let mut tr = item.rect;
            DrawTextW(item.dc, label.as_ptr(), len, &mut tr, 0x20 | 0x4 | 0x1);
            if !old.is_null() {
                SelectObject(item.dc, old);
            }
            return 1;
        }
    }
    if msg == WM_COMMAND && !state.is_null() {
        let id = wp & 0xffff;
        if id == DEFAULT_READER {
            match crate::association::choose_default(hwnd) {
                Ok(()) => {
                    SetWindowTextW(
                        GetDlgItem(hwnd, DEFAULT_STATUS as i32),
                        wide(DEFAULT_DONE).as_ptr(),
                    );
                }
                Err(error) => {
                    MessageBoxW(
                        hwnd,
                        wide(&error.to_string()).as_ptr(),
                        wide("Default Markdown reader").as_ptr(),
                        0x10,
                    );
                }
            }
            return 0;
        }
        if id == CHOICE && (wp >> 16) & 0xffff == 1 {
            let chosen = SendMessageW(GetDlgItem(hwnd, CHOICE as i32), 0x147, 0, 0);
            if chosen >= 0 {
                (*state).borrow_mut().theme = chosen as usize;
                preview(hwnd, &*state);
            }
            return 0;
        }
        if id == ADD {
            let list = GetDlgItem(hwnd, CHOICE as i32);
            let mut typed = [0u16; 64];
            let length = GetWindowTextW(
                GetDlgItem(hwnd, NAME as i32),
                typed.as_mut_ptr(),
                typed.len() as i32,
            );
            let typed = String::from_utf16_lossy(&typed[..length.max(0) as usize]);
            let mut app = (*state).borrow_mut();
            let from = app.palette();
            let previous_count = app.themes.palettes.len();
            let index = app.themes.add(&typed, from);
            if app.themes.palettes.len() > previous_count {
                let name = wide(&app.themes.palettes[index].0);
                SendMessageW(list, 0x143, 0, name.as_ptr() as isize);
            }
            app.theme = index;
            drop(app);
            SendMessageW(list, 0x14e, index, 0);
            SetWindowTextW(GetDlgItem(hwnd, NAME as i32), wide("").as_ptr());
            preview(hwnd, &*state);
            return 0;
        }
        if let Some(role) = id.checked_sub(SWATCH).filter(|r| *r < 7) {
            let mut custom = [0xffffffu32; 16];
            let mut choose: ChooseColor = std::mem::zeroed();
            choose.size = std::mem::size_of::<ChooseColor>() as u32;
            choose.owner = hwnd;
            choose.custom = custom.as_mut_ptr();
            choose.result = (*state).borrow().palette().role(role);
            choose.flags = 0x1 | 0x2;
            if ChooseColorW(&mut choose) != 0 {
                {
                    let mut app = (*state).borrow_mut();
                    let theme = app.theme;
                    app.themes.palettes[theme].1.set_role(role, choose.result);
                }
                preview(hwnd, &*state);
            }
            return 0;
        }
    }
    if msg == WM_DESTROY {
        let ptr = SetWindowLongPtrW(hwnd, 0, 0) as *mut SettingsVisuals;
        if !ptr.is_null() {
            drop(Box::from_raw(ptr));
        }
        if !state.is_null() {
            let restore = (*state).borrow_mut().restore.take();
            if let Some((themes, theme)) = restore {
                let mut app = (*state).borrow_mut();
                app.themes = themes;
                app.theme = theme;
                let owner = GetWindowLongPtrW(hwnd, -8) as Hwnd;
                if !owner.is_null() {
                    app.apply_theme(owner);
                }
            }
        }
        return 0;
    }
    if msg == WM_COMMAND && (PIPE..PIPE + 4).contains(&(wp & 0xffff)) {
        let chosen = (wp & 0xffff)
            .checked_sub(PIPE + 1)
            .and_then(|index| Pipe::ALL.get(index).copied());
        SetWindowTextW(
            GetDlgItem(hwnd, NOTE as i32),
            wide(Pipe::note(chosen)).as_ptr(),
        );
        SetWindowTextW(
            GetDlgItem(hwnd, SPREAD as i32),
            wide(&(*state).borrow().tab().stats.breakdown(chosen)).as_ptr(),
        );
        return 0;
    }
    if msg == WM_COMMAND && matches!(wp & 0xffff, OK | CANCEL) {
        if wp & 0xffff == OK && !state.is_null() {
            let mut app = (*state).borrow_mut();
            app.restore = None;
            let checked = |id: usize| SendMessageW(GetDlgItem(hwnd, id as i32), 0xf0, 0, 0) == 1;
            let chosen = Pipe::ALL
                .into_iter()
                .find(|pipe| checked(PIPE + 1 + *pipe as usize));
            app.restyle = app.pinned != chosen;
            app.pinned = chosen;
            app.smooth = checked(SMOOTH);
            app.floating = checked(FLOAT);
            app.outline_open = checked(OUTLINE_OPT);
            let selected = SendMessageW(GetDlgItem(hwnd, TEXT_SIZE as i32), 0x147, 0, 0);
            if selected >= 0 {
                let zoom = Themes::ZOOM.0 + selected as i32;
                app.set_zoom(GetParent(hwnd), zoom);
            }
            if !app.smooth {
                let target = app.tab().target;
                app.scroll_to(GetParent(hwnd), target);
            }
            if !app.save() {
                drop(app);
                MessageBoxW(
                    hwnd,
                    wide("Your changes apply to this session, but mdlite.ini could not be saved. Move MDLite to a writable folder to keep your preferences.").as_ptr(),
                    wide("Settings could not be saved").as_ptr(),
                    0x30,
                );
            }
        }
        DestroyWindow(hwnd);
        return 0;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

unsafe extern "system" fn window_proc(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }
    if msg == WM_ERASEBKGND {
        return 1;
    }
    if msg == WM_SIZE && ADJUSTING.get() {
        return 0;
    }
    let state = GetWindowLongPtrW(hwnd, -21) as *const RefCell<App>;
    if state.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let state = &*state;
    if msg == WM_COMMAND {
        match wp & 0xffff {
            OPEN => {
                if let Some(path) = choose_file(hwnd) {
                    load_file_tab(hwnd, state, path, false, None);
                }
                return 0;
            }
            OPEN_NEW_TAB => {
                if let Some(path) = choose_file(hwnd) {
                    load_file_tab(hwnd, state, path, true, None);
                }
                return 0;
            }
            CLOSE_TAB => {
                let active = state.borrow().active;
                state.borrow_mut().close_tab(hwnd, active);
                return 0;
            }
            TOGGLE_OUTLINE => {
                state.borrow_mut().toggle_outline(hwnd);
                return 0;
            }
            CTX_OPEN_LINK => {
                let href = state.borrow_mut().context_link.take();
                if let Some(href) = href {
                    activate_link(hwnd, state, &href);
                }
                return 0;
            }
            RELOAD => {
                let path = state.borrow().tab().file.clone();
                if let Some(path) = path {
                    load_file_tab(hwnd, state, path, false, None);
                }
                return 0;
            }
            SETTINGS => {
                settings(hwnd, state);
                return 0;
            }
            EXIT => {
                DestroyWindow(hwnd);
                return 0;
            }
            _ => {}
        }
    }
    if msg == WM_NCPAINT || msg == WM_NCACTIVATE {
        let res = DefWindowProcW(hwnd, msg, wp, lp);
        if let Ok(app) = state.try_borrow() {
            paint_menu_seam(hwnd, app.palette());
        }
        return res;
    }
    if msg == WM_MEASUREITEM {
        let item = &mut *(lp as *mut MeasureItem);
        if item.kind == 1 {
            measure_menu_item(item);
        }
        return 1;
    }
    if msg == WM_DRAWITEM {
        let item = &*(lp as *const DrawItem);
        if item.kind == 1 {
            draw_menu_item(item);
        }
        return 1;
    }
    if msg == WM_DROPFILES {
        let drop = wp as Handle;
        let count = DragQueryFileW(drop, u32::MAX, ptr::null_mut(), 0);
        let mut paths = Vec::new();
        for i in 0..count {
            let length = DragQueryFileW(drop, i, ptr::null_mut(), 0);
            if length > 0 {
                let mut buffer = vec![0u16; length as usize + 1];
                DragQueryFileW(drop, i, buffer.as_mut_ptr(), buffer.len() as u32);
                paths.push(PathBuf::from(OsString::from_wide(
                    &buffer[..length as usize],
                )));
            }
        }
        DragFinish(drop);
        for path in paths {
            load_file_tab(hwnd, state, path, true, None);
        }
        return 0;
    }
    if !matches!(
        msg,
        WM_PAINT
            | WM_SIZE
            | WM_APP_LAYOUT
            | WM_DPICHANGED
            | WM_COMMAND
            | WM_MOUSEWHEEL
            | WM_KEYDOWN
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_LBUTTONDBLCLK
            | WM_RBUTTONUP
            | WM_MBUTTONUP
            | WM_MOUSEMOVE
            | WM_MOUSELEAVE
            | WM_SETCURSOR
            | 0x318
    ) {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    // Handle link click release without holding a RefCell borrow during file I/O.
    if msg == WM_LBUTTONUP {
        let (x, y) = ((lp as i16) as i32, ((lp >> 16) as i16) as i32);
        let clicked_link = {
            let Ok(mut app) = state.try_borrow_mut() else {
                return 0;
            };
            if app.outline_drag.take().is_some() {
                ReleaseCapture();
                app.layout(hwnd);
                return 0;
            }
            if app.grab.take().is_some() {
                ReleaseCapture();
                app.invalidate_bar(hwnd, app.thumb());
                return 0;
            }
            let was_selecting = std::mem::take(&mut app.selecting);
            if was_selecting {
                ReleaseCapture();
            }
            let origin = app.press_origin.take();
            let link = app.press_link.take();
            match (origin, link) {
                (Some((ox, oy)), Some(href)) if (x - ox).abs() <= 4 && (y - oy).abs() <= 4 => {
                    // Single click on a link: clear any zero-width anchor and follow it.
                    app.tab_mut().selection = None;
                    InvalidateRect(hwnd, ptr::null(), 0);
                    Some(href)
                }
                _ => None,
            }
        };
        if let Some(href) = clicked_link {
            activate_link(hwnd, state, &href);
        }
        return 0;
    }

    let Ok(mut app) = state.try_borrow_mut() else {
        if msg == WM_SIZE {
            PostMessageW(hwnd, WM_APP_LAYOUT, 1, 0);
            return 0;
        }
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    match msg {
        WM_PAINT => {
            app.paint(hwnd);
            0
        }
        WM_LBUTTONDBLCLK => {
            let (x, y) = ((lp as i16) as i32, ((lp >> 16) as i16) as i32);
            if app.on_divider(x, y) {
                app.outline_width = OUTLINE_DEFAULT_W;
                app.layout(hwnd);
                return 0;
            }
            if app.in_doc_viewport(x, y) {
                if let (Some(pos), _) = app.hit_doc(hwnd, x, y) {
                    app.select_word_at(pos);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = ((lp as i16) as i32, ((lp >> 16) as i16) as i32);
            // 1. Top tab bar clicks.
            if y < app.doc_top() {
                match app.hit_tab(x, y) {
                    Some(TabHit::ToggleOutline) => app.toggle_outline(hwnd),
                    Some(TabHit::Select(idx)) => app.switch_tab(hwnd, idx),
                    Some(TabHit::Close(idx)) => app.close_tab(hwnd, idx),
                    Some(TabHit::NewTab) => {
                        PostMessageW(hwnd, WM_COMMAND, OPEN_NEW_TAB, 0);
                    }
                    Some(TabHit::CycleTheme) => {
                        let next = (app.theme + 1) % app.themes.palettes.len();
                        PostMessageW(hwnd, WM_COMMAND, PALETTE + next, 0);
                    }
                    Some(TabHit::OpenSettings) => {
                        PostMessageW(hwnd, WM_COMMAND, SETTINGS, 0);
                    }
                    None => {}
                }
                return 0;
            }
            // 2. Outline sidebar resize divider.
            if app.on_divider(x, y) {
                app.outline_drag = Some((x, app.outline_width));
                SetCapture(hwnd);
                return 0;
            }
            // 3. Outline heading click -> jump smoothly to heading.
            if x < app.doc_left() && y < app.doc_bottom() {
                if let Some(idx) = app.hit_outline(x, y) {
                    if let Some(h) = app.tab().headings.get(idx).cloned() {
                        let target = (h.y - app.px(16)).max(0);
                        app.glide_to(hwnd, target);
                    }
                }
                return 0;
            }
            // 4. Scrollbar track / thumb.
            if x >= app.bar().1 && y >= app.doc_top() && y < app.doc_bottom() {
                if let Some((top, height)) = app.thumb() {
                    let local_y = y - app.doc_top();
                    app.grab = (top..top + height)
                        .contains(&local_y)
                        .then(|| local_y - top);
                    let target = app.bar_target(y);
                    SetCapture(hwnd);
                    if app.grab.is_some() {
                        app.scroll_to(hwnd, target);
                    } else {
                        app.glide_to(hwnd, target);
                    }
                    app.invalidate_bar(hwnd, app.thumb());
                }
                return 0;
            }
            // 5. Status bar clicks.
            if y >= app.doc_bottom() {
                match app.hit(x, y) {
                    Some(Hit::Engine) => {
                        let text = app.tab().stats.explain(app.pinned);
                        drop(app);
                        MessageBoxW(
                            hwnd,
                            wide(&text).as_ptr(),
                            wide("Rendering pipeline").as_ptr(),
                            0x40,
                        );
                    }
                    Some(spot) => {
                        let zoom = app.zoomed(spot);
                        app.set_zoom(hwnd, zoom);
                    }
                    None => {}
                }
                return 0;
            }
            // 6. Document viewport: start text selection or potential link click.
            if app.in_doc_viewport(x, y) {
                let (pos, link) = app.hit_doc(hwnd, x, y);
                app.press_origin = Some((x, y));
                app.press_link = link;
                let shift = GetKeyState(0x10) < 0;
                if let Some(pos) = pos {
                    if shift {
                        let anchor = app.tab().selection.map_or(pos, |(a, _)| a);
                        app.tab_mut().selection = Some((anchor, pos));
                    } else {
                        app.tab_mut().selection = Some((pos, pos));
                    }
                    app.selecting = true;
                    SetCapture(hwnd);
                    InvalidateRect(hwnd, ptr::null(), 0);
                } else if app.tab_mut().selection.take().is_some() {
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
            0
        }
        WM_MBUTTONUP => {
            let (x, y) = ((lp as i16) as i32, ((lp >> 16) as i16) as i32);
            if y < app.doc_top() {
                if let Some(TabHit::Select(idx) | TabHit::Close(idx)) = app.hit_tab(x, y) {
                    app.close_tab(hwnd, idx);
                }
            }
            0
        }
        WM_RBUTTONUP => {
            let (x, y) = ((lp as i16) as i32, ((lp >> 16) as i16) as i32);
            if app.in_doc_viewport(x, y) {
                app.show_context_menu(hwnd, x, y);
            }
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = ((lp as i16) as i32, ((lp >> 16) as i16) as i32);
            // Active outline divider drag.
            if let Some((start_x, start_w)) = app.outline_drag {
                let delta_logical = ((x - start_x) as i64 * 96 / app.dpi as i64) as i32;
                let next_w = (start_w + delta_logical).clamp(OUTLINE_MIN_W, OUTLINE_MAX_W);
                if next_w != app.outline_width {
                    app.outline_width = next_w;
                    app.layout(hwnd);
                }
                return 0;
            }
            // Active scrollbar drag.
            if app.grab.is_some() {
                let target = app.bar_target(y);
                app.scroll_to(hwnd, target);
                return 0;
            }
            // Active text selection drag.
            if app.selecting {
                // Auto-scroll slightly if dragging above or below viewport.
                if y < app.doc_top() {
                    let next = app.tab().scroll.saturating_sub(app.px(20));
                    app.scroll_to(hwnd, next);
                } else if y >= app.doc_bottom() {
                    let next = app.tab().scroll.saturating_add(app.px(20));
                    app.scroll_to(hwnd, next);
                }
                let clamped_y = y.clamp(app.doc_top(), app.doc_bottom() - 1);
                if let (Some(pos), _) = app.hit_doc(hwnd, x, clamped_y) {
                    if let Some((anchor, focus)) = app.tab().selection {
                        if focus != pos {
                            app.tab_mut().selection = Some((anchor, pos));
                            let doc_rect = Rect {
                                left: app.doc_left(),
                                top: app.doc_top(),
                                right: app.bar().1,
                                bottom: app.doc_bottom(),
                            };
                            InvalidateRect(hwnd, &doc_rect, 0);
                        }
                    }
                }
                return 0;
            }

            // Track mouse leave so hover states clear cleanly when cursor exits.
            let mut track = TrackMouse {
                size: size_of::<TrackMouse>() as u32,
                flags: 2,
                window: hwnd,
                time: 0,
            };
            TrackMouseEvent(&mut track);

            // Hover updates: scrollbar, tab bar, outline, divider, links.
            let bar_hover = x >= app.bar().1
                && y >= app.doc_top()
                && y < app.doc_bottom()
                && app.thumb().is_some();
            if bar_hover != app.hover {
                app.hover = bar_hover;
                app.invalidate_bar(hwnd, app.thumb());
            }
            let next_tab_hover = if y < app.doc_top() {
                app.hit_tab(x, y)
            } else {
                None
            };
            if next_tab_hover != app.tab_hover {
                app.tab_hover = next_tab_hover;
                let r = Rect {
                    left: 0,
                    top: 0,
                    right: app.width,
                    bottom: app.doc_top(),
                };
                InvalidateRect(hwnd, &r, 0);
            }
            let next_div = app.on_divider(x, y);
            let next_outline_hover =
                if x < app.doc_left() && y >= app.doc_top() && y < app.doc_bottom() && !next_div {
                    app.hit_outline(x, y)
                } else {
                    None
                };
            if next_div != app.divider_hover || next_outline_hover != app.outline_hover {
                app.divider_hover = next_div;
                app.outline_hover = next_outline_hover;
                let r = Rect {
                    left: 0,
                    top: app.doc_top(),
                    right: app.doc_left() + app.ui_px(2),
                    bottom: app.doc_bottom(),
                };
                InvalidateRect(hwnd, &r, 0);
            }
            let next_link = if app.in_doc_viewport(x, y) && !app.tab().links.is_empty() {
                app.hit_doc(hwnd, x, y).1
            } else {
                None
            };
            if next_link != app.hover_link {
                app.hover_link = next_link;
                let status_r = Rect {
                    left: 0,
                    top: app.doc_bottom(),
                    right: app.width,
                    bottom: app.client_h(),
                };
                InvalidateRect(hwnd, &status_r, 0);
                let doc_r = Rect {
                    left: app.doc_left(),
                    top: app.doc_top(),
                    right: app.bar().1,
                    bottom: app.doc_bottom(),
                };
                InvalidateRect(hwnd, &doc_r, 0);
            }
            0
        }
        WM_MOUSELEAVE => {
            let mut changed = false;
            if app.tab_hover.take().is_some() {
                changed = true;
            }
            if app.outline_hover.take().is_some() || std::mem::take(&mut app.divider_hover) {
                changed = true;
            }
            if app.hover_link.take().is_some() {
                changed = true;
            }
            if std::mem::take(&mut app.hover) {
                app.invalidate_bar(hwnd, app.thumb());
            }
            if changed {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_SETCURSOR => {
            let mut point = Point::default();
            GetCursorPos(&mut point);
            ScreenToClient(hwnd, &mut point);
            if wp == hwnd as usize && (lp as u32 & 0xffff) == 1 {
                let cursor_id = if app.outline_drag.is_some() || app.on_divider(point.x, point.y) {
                    Some(32644usize) // IDC_SIZEWE
                } else if app.hover_link.is_some()
                    || app.hit(point.x, point.y).is_some()
                    || (point.y < app.doc_top() && app.hit_tab(point.x, point.y).is_some())
                    || (point.x < app.doc_left()
                        && point.y >= app.doc_top()
                        && point.y < app.doc_bottom()
                        && app.hit_outline(point.x, point.y).is_some())
                {
                    Some(32649usize) // IDC_HAND
                } else if app.in_doc_viewport(point.x, point.y) {
                    Some(32513usize) // IDC_IBEAM
                } else {
                    None
                };
                if let Some(id) = cursor_id {
                    SetCursor(LoadCursorW(NULL, id as *const u16));
                    return 1;
                }
            }
            drop(app);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        0x318 => {
            let mut rect = Rect::default();
            GetClientRect(hwnd, &mut rect);
            app.draw(wp as Hdc, rect);
            0
        }
        WM_SIZE => {
            let width = (lp as u32 & 0xffff) as i32;
            let height = ((lp as u32 >> 16) & 0xffff) as i32;
            if wp == 1 {
                return 0;
            }
            if width != app.width {
                if !app.layout_pending {
                    app.layout_pending = true;
                    PostMessageW(hwnd, WM_APP_LAYOUT, 0, 0);
                }
            } else {
                app.height = (height - app.tab_bar_h() - app.status()).max(0);
                let scroll = app.tab().scroll;
                app.scroll_to(hwnd, scroll);
                app.update_scroll(hwnd);
                app.buffer = None;
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_APP_LAYOUT => {
            if app.layout_pending || wp != 0 {
                app.layout(hwnd);
            }
            0
        }
        WM_DPICHANGED => {
            app.set_dpi(wp as u32 & 0xffff);
            let rect = &*(lp as *const Rect);
            SetWindowPos(
                hwnd,
                NULL,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                0x14,
            );
            app.layout(hwnd);
            app.build_menu(hwnd);
            app.apply_theme(hwnd);
            0
        }
        WM_COMMAND => {
            match wp & 0xffff {
                id if (PALETTE..PALETTE + PALETTES).contains(&id) => {
                    app.theme = (id - PALETTE).min(app.themes.palettes.len() - 1);
                }
                id if ZOOM.iter().any(|(command, ..)| *command == id) => {
                    if let Some((.., spot, _, _)) = ZOOM.iter().find(|(c, ..)| *c == id) {
                        let zoom = app.zoomed(*spot);
                        app.set_zoom(hwnd, zoom);
                    }
                    return 0;
                }
                RELOAD_THEME => {
                    let was_outline = app.outline_open;
                    app.themes = Themes::load(&app.theme_path);
                    app.theme = app.theme.min(app.themes.palettes.len() - 1);
                    app.smooth = app.themes.smooth;
                    app.floating = app.themes.floating;
                    app.outline_open = app.themes.outline;
                    app.build_menu(hwnd);
                    let chosen = pinned(&app.themes);
                    if chosen != app.pinned {
                        app.pinned = chosen;
                        app.rebuild(hwnd);
                    } else if app.outline_open != was_outline {
                        app.layout(hwnd);
                    }
                }
                _ => return DefWindowProcW(hwnd, msg, wp, lp),
            }
            app.apply_theme(hwnd);
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) as u16 as i16) as i32;
            if wp & 8 != 0 {
                app.pixel_remainder = 0;
                app.wheel_remainder += delta;
                let steps = app.wheel_remainder / 120;
                app.wheel_remainder %= 120;
                if steps != 0 {
                    let zoom = app.zoom + steps * Themes::ZOOM.2;
                    app.set_zoom(hwnd, zoom);
                }
                return 0;
            }
            // Scroll the Outline sidebar independently when pointer is over it.
            let mut pt = Point {
                x: (lp as i16) as i32,
                y: ((lp >> 16) as i16) as i32,
            };
            ScreenToClient(hwnd, &mut pt);
            if pt.x < app.doc_left() && pt.y >= app.doc_top() && pt.y < app.doc_bottom() {
                let row_h = app.ui_px(26).max(18);
                let total_h = (app.tab().headings.len() as i32) * row_h + app.ui_px(8);
                let list_h = (app.height - app.ui_px(30)).max(1);
                let max_o = (total_h - list_h).max(0);
                let step = (delta * row_h * 2) / 120;
                let next = (app.tab().outline_scroll - step).clamp(0, max_o);
                if next != app.tab().outline_scroll {
                    app.tab_mut().outline_scroll = next;
                    let r = Rect {
                        left: 0,
                        top: app.doc_top(),
                        right: app.doc_left(),
                        bottom: app.doc_bottom(),
                    };
                    InvalidateRect(hwnd, &r, 0);
                }
                return 0;
            }
            let mut lines: u32 = 3;
            SystemParametersInfoW(0x68, 0, &mut lines as *mut _ as _, 0);
            let amount = if lines == u32::MAX {
                (app.height - app.px(24)).max(1)
            } else {
                app.px(24).saturating_mul(lines.min(100) as i32)
            };
            app.wheel_remainder = 0;
            let movement = wheel_pixels(delta, amount, &mut app.pixel_remainder);
            let (scroll, target_cur) = (app.tab().scroll, app.tab().target);
            let from =
                if (movement > 0 && target_cur > scroll) || (movement < 0 && target_cur < scroll) {
                    scroll
                } else {
                    target_cur
                };
            let target = from.saturating_sub(movement);
            app.glide_to(hwnd, target);
            0
        }
        WM_KEYDOWN => {
            let ctrl = GetKeyState(0x11) < 0;
            let shift = GetKeyState(0x10) < 0;
            // Selection & clipboard shortcuts.
            if ctrl && wp == 0x43 {
                // Ctrl+C copies PLAIN TEXT; Ctrl+Shift+C copies Markdown.
                let text = if shift {
                    app.tab().selected_markdown()
                } else {
                    app.tab().selected_plain()
                };
                clipboard::copy_text(hwnd, &text);
                return 0;
            }
            if ctrl && wp == 0x41 {
                app.select_all();
                InvalidateRect(hwnd, ptr::null(), 0);
                return 0;
            }
            if wp == 0x1b && app.tab().selection.is_some() {
                app.tab_mut().selection = None;
                InvalidateRect(hwnd, ptr::null(), 0);
                return 0;
            }
            // Tab switching shortcuts: Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+1..9.
            if ctrl && wp == 0x09 && app.tabs.len() > 1 {
                let n = app.tabs.len();
                let next = if shift {
                    (app.active + n - 1) % n
                } else {
                    (app.active + 1) % n
                };
                app.switch_tab(hwnd, next);
                return 0;
            }
            if ctrl && (0x31..=0x39).contains(&wp) {
                let idx = wp - 0x31;
                if idx < app.tabs.len() {
                    app.switch_tab(hwnd, idx);
                }
                return 0;
            }
            let command = match (ctrl, wp) {
                (true, 0x4f) => Some(OPEN),
                (true, 0x54) => Some(OPEN_NEW_TAB),
                (true, 0x57) => Some(CLOSE_TAB),
                (true, 0x42) => Some(TOGGLE_OUTLINE),
                (true, 0xbc) => Some(SETTINGS),
                (_, 0x74) => Some(if ctrl { RELOAD_THEME } else { RELOAD }),
                (true, 0x44) => Some(PALETTE + (app.theme + 1) % app.themes.palettes.len()),
                _ => None,
            };
            let zoom = match (ctrl, wp) {
                (true, 0xbb | 0x6b) => Some(app.zoom + Themes::ZOOM.2),
                (true, 0xbd | 0x6d) => Some(app.zoom - Themes::ZOOM.2),
                (true, 0x30 | 0x60) => Some(100),
                _ => None,
            };
            if let Some(zoom) = zoom {
                app.set_zoom(hwnd, zoom);
                return 0;
            }
            if let Some(command) = command {
                PostMessageW(hwnd, WM_COMMAND, command, 0);
                return 0;
            }
            let page = (app.height - app.px(24)).max(1);
            let from = app.tab().target;
            let content_h = app.tab().content_height;
            let target = match wp {
                0x26 => from.saturating_sub(app.px(24)),
                0x28 => from.saturating_add(app.px(24)),
                0x21 => from.saturating_sub(page),
                0x22 => from.saturating_add(page),
                0x20 => {
                    if shift {
                        from.saturating_sub(page)
                    } else {
                        from.saturating_add(page)
                    }
                }
                0x24 => 0,
                0x23 => content_h,
                _ => return DefWindowProcW(hwnd, msg, wp, lp),
            };
            app.glide_to(hwnd, target);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

pub fn run() {
    unsafe {
        SetProcessDpiAwarenessContext(-4isize as Handle);
        let instance = GetModuleHandleW(ptr::null());
        let name = wide("MDLite.NativeReader");
        if !register(&name, window_proc, NULL, 0) {
            error(NULL, "Could not register the native window.");
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            name.as_ptr(),
            wide("MDLite").as_ptr(),
            0x00cf0000 | 0x0200_0000,
            i32::MIN,
            i32::MIN,
            1060,
            740,
            NULL,
            NULL,
            instance,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            error(NULL, "Could not create the native window.");
            return;
        }
        for (which, metric) in [(0, 49), (1, 11)] {
            let side = GetSystemMetrics(metric);
            let icon = LoadImageW(instance, 1usize as *const u16, 1, side, side, 0x8000);
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, which, icon as isize);
            }
        }
        let floating = wide("MDLite.Zoom");
        let floater = register(&floating, floater_proc, NULL, 2 * size_of::<isize>() as i32)
            .then(|| {
                let child = CreateWindowExW(
                    0,
                    floating.as_ptr(),
                    ptr::null(),
                    0x4000_0000,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    NULL,
                    instance,
                    ptr::null_mut(),
                );
                SetWindowLongPtrW(child, 0, -1);
                child
            })
            .unwrap_or(NULL);
        let state = Box::new(RefCell::new(App::new()));
        SetWindowLongPtrW(hwnd, -21, &*state as *const RefCell<App> as isize);
        {
            let mut app = state.borrow_mut();
            app.floater = floater;
            app.dpi = GetDpiForWindow(hwnd).max(96);
            app.build_menu(hwnd);
        }
        DragAcceptFiles(hwnd, 1);
        // Fast startup: if a CLI file argument is supplied, load it directly
        // without wasting a parse+layout cycle on the Welcome placeholder.
        let mut loaded_any = false;
        for arg in std::env::args_os().skip(1) {
            if load_file_tab(hwnd, &state, Path::new(&arg).to_owned(), loaded_any, None) {
                loaded_any = true;
            }
        }
        if !loaded_any {
            let mut app = state.borrow_mut();
            app.tab_mut().source = WELCOME_DOC.to_owned();
            app.set_document(WELCOME_DOC);
            app.layout(hwnd);
        }
        state.borrow_mut().apply_theme_inner(hwnd, false);
        ShowWindow(hwnd, 5);
        UpdateWindow(hwnd);
        let mut message = Msg::default();
        'frames: loop {
            let animating = state.borrow().tab().animating;
            if animating {
                while PeekMessageW(&mut message, NULL, 0, 0, 1) != 0 {
                    if message.message == WM_QUIT {
                        break 'frames;
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                if let Ok(mut app) = state.try_borrow_mut() {
                    if app.tab().animating {
                        app.glide(hwnd);
                    }
                }
                if !wait_for_refresh() {
                    Sleep(8);
                }
            } else {
                if GetMessageW(&mut message, NULL, 0, 0) <= 0 {
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        state.borrow().save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    unsafe fn window(width: i32, height: i32) -> Hwnd {
        let hwnd = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("MDLite layout test").as_ptr(),
            0x00cf0000,
            0,
            0,
            width,
            height,
            NULL,
            NULL,
            GetModuleHandleW(ptr::null()),
            ptr::null_mut(),
        );
        assert!(!hwnd.is_null());
        hwnd
    }
    fn document(app: &mut App, source: &str) {
        app.set_document(source);
        app.tab_mut().source = source.to_owned();
    }
    unsafe fn save(surface: &Buffer, name: &str) {
        let mut file = vec![0u8; 54];
        file[0..2].copy_from_slice(b"BM");
        let pixels = surface.pixels();
        file[2..6].copy_from_slice(&((54 + pixels.len() * 4) as u32).to_le_bytes());
        file[10..14].copy_from_slice(&54u32.to_le_bytes());
        file[14..18].copy_from_slice(&40u32.to_le_bytes());
        file[18..22].copy_from_slice(&surface.width.to_le_bytes());
        file[22..26].copy_from_slice(&(-surface.height).to_le_bytes());
        file[26..28].copy_from_slice(&1u16.to_le_bytes());
        file[28..30].copy_from_slice(&32u16.to_le_bytes());
        for pixel in pixels {
            file.extend((pixel | 0xff00_0000).to_le_bytes());
        }
        let path = std::env::temp_dir().join(format!("mdlite-{name}.bmp"));
        std::fs::write(&path, file).unwrap();
        println!("wrote {}", path.display());
    }
    #[test]
    #[ignore]
    fn preview_documents() {
        for (name, dark, source) in [
            ("tables", true, include_str!("../examples/tables.md")),
            ("tables-light", false, include_str!("../examples/tables.md")),
            (
                "scripts",
                true,
                include_str!("../examples/complex-scripts.md"),
            ),
            ("showcase", true, include_str!("../examples/showcase.md")),
            (
                "showcase-light",
                false,
                include_str!("../examples/showcase.md"),
            ),
            ("emoji", true, include_str!("../examples/emoji.md")),
        ] {
            unsafe {
                let hwnd = window(1120, 760);
                let mut app = App::new();
                app.theme = usize::from(!dark);
                document(&mut app, source);
                app.layout(hwnd);
                let (width, height) = (app.width, app.client_h());
                let surface = Buffer::new(NULL, width, height).unwrap();
                app.draw(
                    surface.dc,
                    Rect {
                        left: 0,
                        top: 0,
                        right: width,
                        bottom: height,
                    },
                );
                save(&surface, name);
                DestroyWindow(hwnd);
            }
        }
    }
    struct Control {
        handle: Hwnd,
        id: usize,
        class: String,
        style: u32,
        rect: Rect,
        text: String,
    }
    impl Control {
        fn is_group(&self) -> bool {
            self.class == "Button" && self.style & 0xf == 7
        }
    }
    unsafe fn controls(hwnd: Hwnd) -> Vec<Control> {
        let mut found = vec![];
        let mut child = GetWindow(hwnd, 5);
        while !child.is_null() {
            let mut class = [0u16; 32];
            let length = GetClassNameW(child, class.as_mut_ptr(), 32).max(0) as usize;
            let mut text = vec![0u16; 512];
            let count = GetWindowTextW(child, text.as_mut_ptr(), 512).max(0) as usize;
            let mut rect = Rect::default();
            GetWindowRect(child, &mut rect);
            let mut corner = Point {
                x: rect.left,
                y: rect.top,
            };
            ScreenToClient(hwnd, &mut corner);
            found.push(Control {
                handle: child,
                id: GetWindowLongPtrW(child, -12) as usize,
                class: String::from_utf16_lossy(&class[..length]),
                style: GetWindowLongPtrW(child, -16) as u32,
                rect: Rect {
                    left: corner.x,
                    top: corner.y,
                    right: corner.x + rect.right - rect.left,
                    bottom: corner.y + rect.bottom - rect.top,
                },
                text: String::from_utf16_lossy(&text[..count]),
            });
            child = GetWindow(child, 2);
        }
        found
    }
    #[test]
    fn the_settings_window_never_overlaps_or_clips_at_any_scale() {
        unsafe {
            let disjoint = |a: &Rect, b: &Rect| {
                a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top
            };
            for dpi in [96, 120, 144, 168, 192, 288] {
                let owner = window(900, 700);
                let state = RefCell::new(App::new());
                let hwnd = settings_window_at(owner, &state, dpi);
                assert!(!hwnd.is_null());
                let mut client = Rect::default();
                GetClientRect(hwnd, &mut client);
                let all = controls(hwnd);
                assert!(all.len() >= 39, "{} controls at {dpi} dpi", all.len());
                let screen = GetDC(NULL);
                for (index, control) in all.iter().enumerate() {
                    let rect = &control.rect;
                    let name = format!("control {} {:?} at {dpi} dpi", control.id, control.text);
                    assert!(
                        rect.left >= 0
                            && rect.top >= 0
                            && rect.right <= client.right
                            && rect.bottom <= client.bottom,
                        "{name} leaves the window"
                    );
                    for other in &all[index + 1..] {
                        assert!(
                            control.is_group() != other.is_group() || disjoint(rect, &other.rect),
                            "{name} overlaps control {} {:?}",
                            other.id,
                            other.text
                        );
                    }
                    if control.text.is_empty() || control.is_group() {
                        continue;
                    }
                    let font = SendMessageW(control.handle, 0x31, 0, 0) as Handle;
                    let old = SelectObject(screen, font);
                    let indicator =
                        if control.class == "Button" && matches!(control.style & 0xf, 2..=6 | 9) {
                            22 * dpi / 96
                        } else {
                            0
                        };
                    let text = wide(&control.text.replace('&', ""));
                    let mut need = Rect {
                        left: 0,
                        top: 0,
                        right: (rect.right - rect.left - indicator).max(1),
                        bottom: 0,
                    };
                    DrawTextW(screen, text.as_ptr(), -1, &mut need, 0x400 | 0x10);
                    SelectObject(screen, old);
                    assert!(
                        need.bottom <= rect.bottom - rect.top,
                        "{name} needs {} px of height and has {}",
                        need.bottom,
                        rect.bottom - rect.top
                    );
                }
                ReleaseDC(NULL, screen);
                let groups: Vec<&Rect> = all
                    .iter()
                    .filter(|c| c.is_group())
                    .map(|c| &c.rect)
                    .collect();
                assert_eq!(groups.len(), 4);
                for control in all.iter().filter(|c| {
                    !c.is_group()
                        && !matches!(c.id, OK | CANCEL)
                        && !c.text.starts_with("Cancel discards")
                }) {
                    let r = &control.rect;
                    assert!(
                        groups.iter().any(|g| r.left >= g.left
                            && r.right <= g.right
                            && r.top >= g.top
                            && r.bottom <= g.bottom),
                        "control {} {:?} sits outside every group box at {dpi} dpi",
                        control.id,
                        control.text
                    );
                }
                DestroyWindow(hwnd);
                DestroyWindow(owner);
            }
        }
    }
    #[test]
    fn adding_an_existing_palette_does_not_duplicate_it() {
        unsafe {
            let owner = window(900, 700);
            let state = RefCell::new(App::new());
            let hwnd = settings_window(owner, &state);
            let list = GetDlgItem(hwnd, CHOICE as i32);
            let count = SendMessageW(list, 0x146, 0, 0);
            let last = state.borrow().themes.palettes.last().unwrap().0.clone();
            SetWindowTextW(GetDlgItem(hwnd, NAME as i32), wide(&last).as_ptr());
            SendMessageW(hwnd, WM_COMMAND, ADD, 0);
            assert_eq!(SendMessageW(list, 0x146, 0, 0), count);
            DestroyWindow(hwnd);
            DestroyWindow(owner);
        }
    }
    #[test]
    fn settings_save_reading_preferences_and_all_edited_palettes() {
        unsafe {
            let owner = window(900, 700);
            let state = RefCell::new(App::new());
            let path = std::env::temp_dir().join("mdlite-settings-controls-test.ini");
            state.borrow_mut().theme_path = path.clone();
            let hwnd = settings_window(owner, &state);
            SendMessageW(GetDlgItem(hwnd, SMOOTH as i32), 0xf1, 0, 0);
            SendMessageW(GetDlgItem(hwnd, FLOAT as i32), 0xf1, 0, 0);
            SendMessageW(GetDlgItem(hwnd, OUTLINE_OPT as i32), 0xf1, 0, 0);
            SendMessageW(
                GetDlgItem(hwnd, TEXT_SIZE as i32),
                0x14e,
                125 - Themes::ZOOM.0 as usize,
                0,
            );
            state.borrow_mut().themes.palettes[0].1.accent = 0x123456;
            state.borrow_mut().themes.palettes[1].1.accent = 0x654321;
            SendMessageW(hwnd, WM_COMMAND, OK, 0);
            let saved = Themes::load(&path);
            assert!(!saved.smooth && !saved.floating && !saved.outline);
            assert_eq!(saved.zoom, 125);
            assert_eq!(saved.palettes[0].1.accent, 0x123456);
            assert_eq!(saved.palettes[1].1.accent, 0x654321);
            std::fs::remove_file(path).unwrap();
            DestroyWindow(owner);
        }
    }
    #[test]
    #[ignore]
    fn bench() {
        unsafe {
            let hwnd = window(1100, 700);
            let mut unique = String::new();
            for i in 0..10_000 {
                unique.push_str(&format!(
                    "Row {i} has unique text Grüße 世界 😀 and **item {i}** with `value_{i}`.\n"
                ));
            }
            let table = format!(
                "| a | b | c |\n|---|---|---|\n{}",
                (0..2000)
                    .map(|i| format!("| row {i} | text {i} | {i} |\n"))
                    .collect::<String>()
            );
            for (name, source) in [("unique lines", &unique), ("table rows", &table)] {
                let mut app = App::new();
                let started = Instant::now();
                document(&mut app, source);
                let parsed = started.elapsed();
                let started = Instant::now();
                app.layout(hwnd);
                let laid = started.elapsed();
                let started = Instant::now();
                SetWindowPos(hwnd, NULL, 0, 0, 900, 700, 0x14);
                app.layout(hwnd);
                let relaid = started.elapsed();
                println!(
                    "{name}: parse {:?}, layout {:?}, relayout {:?}, {} rows",
                    parsed,
                    laid,
                    relaid,
                    app.tab().rows.len()
                );
            }
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn text_encodings_and_binary_rejection() {
        assert_eq!(decode(b"hello".to_vec()).unwrap(), "hello");
        assert_eq!(decode(vec![0xff, 0xfe, 0x3d, 0xd8, 0, 0xde]).unwrap(), "😀");
        assert_eq!(decode(vec![0xfe, 0xff, 0, 65]).unwrap(), "A");
        assert!(decode(vec![0xff, 0xfe, 65]).is_err());
        assert!(decode(vec![0xff]).is_err());
        assert!(decode(vec![65, 0, 66]).is_err());
    }
    #[test]
    fn language_fixtures_round_trip_all_supported_encodings() {
        for text in [
            include_str!("../examples/languages.md"),
            include_str!("../examples/complex-scripts.md"),
        ] {
            assert_eq!(decode(text.as_bytes().to_vec()).unwrap(), text);
            let mut utf8 = vec![0xef, 0xbb, 0xbf];
            utf8.extend(text.bytes());
            assert_eq!(decode(utf8).unwrap(), text);
            for le in [true, false] {
                let mut bytes = if le {
                    vec![0xff, 0xfe]
                } else {
                    vec![0xfe, 0xff]
                };
                for word in text.encode_utf16() {
                    bytes.extend(if le {
                        word.to_le_bytes()
                    } else {
                        word.to_be_bytes()
                    });
                }
                assert_eq!(decode(bytes).unwrap(), text);
            }
        }
        for bytes in [
            vec![0xff, 0xfe, 0, 0xd8],
            vec![0xfe, 0xff, 0xdc, 0],
            vec![0xf0, 0x80, 0x80, 0x80],
        ] {
            assert!(decode(bytes).is_err());
        }
    }
    #[test]
    fn a_document_too_large_to_hold_is_cut_and_says_so() {
        unsafe {
            let hwnd = window(700, 500);
            let mut app = App::new();
            let source = "line\n".repeat(2_000_000);
            let started = Instant::now();
            document(&mut app, &source);
            app.layout(hwnd);
            assert_eq!(app.tab().blocks.len(), markdown::LONGEST_DOCUMENT);
            assert!(app.tab().stats.truncated);
            assert!(app.tab().stats.summary().contains("only the first million"));
            assert!(started.elapsed().as_secs() < 30);
            app.scroll_to(hwnd, i32::MAX);
            assert!(
                app.tab().scroll > 0 && app.tab().scroll == app.tab().content_height - app.height
            );
            let buffer = Buffer::new(NULL, app.width, app.client_h()).unwrap();
            app.draw(
                buffer.dc,
                Rect {
                    left: 0,
                    top: 0,
                    right: app.width,
                    bottom: app.client_h(),
                },
            );
            document(&mut app, "a line\n\nanother\n");
            app.layout(hwnd);
            assert!(!app.tab().stats.truncated);
            assert!(!app.tab().stats.summary().contains("only the first"));
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn the_palette_list_cannot_outgrow_the_menu() {
        let mut themes = Themes::default();
        for n in 0..Themes::MOST_PALETTES * 2 {
            themes.add(&format!("Palette {n}"), Palette::dark());
        }
        assert_eq!(themes.palettes.len(), Themes::MOST_PALETTES);
        assert!(Themes::MOST_PALETTES <= PALETTES);
        let sections: String = (0..500).map(|n| format!("[Section {n}]\n")).collect();
        assert_eq!(
            Themes::parse(&sections).palettes.len(),
            Themes::MOST_PALETTES
        );
    }
    #[test]
    fn a_stale_palette_index_cannot_reach_past_the_list() {
        unsafe {
            let mut app = App::new();
            app.theme_path = std::env::temp_dir().join("mdlite-stale-test.ini");
            let _ = std::fs::remove_file(&app.theme_path);
            app.themes = Themes::default();
            app.theme = 999;
            assert_eq!(app.palette(), app.themes.palettes.last().unwrap().1);
            app.save();
            assert!(app.theme_path.exists());
            std::fs::remove_file(&app.theme_path).unwrap();
        }
    }
    #[test]
    fn absurd_window_sizes_and_zooms_are_survivable() {
        unsafe {
            let hwnd = window(600, 400);
            let mut app = App::new();
            document(&mut app, include_str!("../examples/tables.md"));
            for (width, height) in [(1, 1), (600, 1), (1, 400), (0, 0), (32, 32), (600, 400)] {
                SetWindowPos(hwnd, NULL, 0, 0, width, height, 0x14);
                for zoom in [Themes::ZOOM.0, 100, Themes::ZOOM.1] {
                    app.zoom = zoom;
                    app.rescale();
                    app.layout(hwnd);
                    app.scroll_to(hwnd, i32::MAX);
                    app.scroll_to(hwnd, i32::MIN);
                    assert!(app.tab().content_height >= 0 && app.tab().scroll >= 0);
                    if let Some(buffer) = Buffer::new(NULL, app.width.max(1), app.client_h().max(1))
                    {
                        app.draw(
                            buffer.dc,
                            Rect {
                                left: 0,
                                top: 0,
                                right: buffer.width,
                                bottom: buffer.height,
                            },
                        );
                    }
                    assert!(app.hit(-1, -1).is_none());
                    assert!(app.thumb().is_none_or(|(top, tall)| top >= 0 && tall > 0));
                }
            }
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn hostile_documents_still_lay_out() {
        unsafe {
            let hwnd = window(700, 500);
            let mut app = App::new();
            let wide_row = format!(
                "|{}\n|{}\n|{}\n",
                "a|".repeat(20_000),
                "-|".repeat(20_000),
                "b|".repeat(20_000)
            );
            let documents: Vec<(&str, String)> = vec![
                ("a table 20,000 columns wide", wide_row),
                ("one line of a million characters", "x".repeat(1_000_000)),
                ("half a million empty lines", "\n".repeat(500_000)),
                (
                    "lists nested a thousand deep",
                    (0..1000)
                        .map(|n| format!("{}- item\n", " ".repeat(n)))
                        .collect(),
                ),
                (
                    "emphasis nested a thousand deep",
                    format!("{}text{}", "*".repeat(1000), "*".repeat(1000)),
                ),
                (
                    "a table whose rows disagree about width",
                    format!(
                        "| a | b | c |\n|---|---|---|\n{}",
                        (0..500)
                            .map(|n| format!("{}\n", "| x ".repeat(n % 40)))
                            .collect::<String>()
                    ),
                ),
                (
                    "every character that decides a pipeline, interleaved",
                    "a\u{5d0}\u{628}\u{939}\u{e01}\u{1f600}\u{301}\u{200d}\u{fe0f}\u{202e}"
                        .repeat(20_000),
                ),
                (
                    "code fences that never close",
                    format!("```\n{}", "fn main() {}\n".repeat(50_000)),
                ),
                ("nothing at all", String::new()),
                ("a single byte order mark", "\u{feff}".to_owned()),
            ];
            for (name, source) in &documents {
                let started = Instant::now();
                document(&mut app, source);
                app.layout(hwnd);
                let buffer = Buffer::new(NULL, app.width, app.client_h()).unwrap();
                app.draw(
                    buffer.dc,
                    Rect {
                        left: 0,
                        top: 0,
                        right: app.width,
                        bottom: app.client_h(),
                    },
                );
                app.scroll_to(hwnd, i32::MAX);
                app.draw(
                    buffer.dc,
                    Rect {
                        left: 0,
                        top: 0,
                        right: app.width,
                        bottom: app.client_h(),
                    },
                );
                assert!(
                    app.tab().content_height >= 0 && app.tab().scroll >= 0,
                    "{name}"
                );
                assert!(started.elapsed().as_secs() < 20, "{name}");
            }
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn every_zoom_control_asks_for_what_its_label_says() {
        let mut app = App::new();
        app.zoom = 100;
        assert_eq!(
            ZOOM.map(|(_, spot, ..)| spot),
            [Hit::ZoomOut, Hit::ZoomReset, Hit::ZoomIn]
        );
        for (index, (_, spot, glyph, label)) in ZOOM.iter().enumerate() {
            let asked = app.zoomed(*spot);
            let expected = match index {
                0 => 100 - Themes::ZOOM.2,
                1 => 100,
                _ => 100 + Themes::ZOOM.2,
            };
            assert_eq!(asked, expected, "cell {index} ({glyph})");
            let wording = label.to_ascii_lowercase();
            assert_eq!(wording.contains("out"), *spot == Hit::ZoomOut);
            assert_eq!(wording.contains("in\t"), *spot == Hit::ZoomIn);
            assert_eq!(wording.contains("reset"), *spot == Hit::ZoomReset);
        }
        assert_eq!(
            ZOOM.iter().map(|(id, ..)| *id).collect::<Vec<_>>(),
            vec![105, 106, 107]
        );
    }
    #[test]
    fn the_floating_zoom_control_keeps_out_of_the_text() {
        unsafe {
            let hwnd = window(900, 600);
            let mut app = App::new();
            app.floater = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                ptr::null(),
                0x4000_0000,
                0,
                0,
                0,
                0,
                hwnd,
                NULL,
                GetModuleHandleW(ptr::null()),
                ptr::null_mut(),
            );
            assert!(!app.floater.is_null());
            let shown = |app: &App| GetWindowLongPtrW(app.floater, -16) as u32 & 0x1000_0000 != 0;
            let corner = |app: &App| {
                let mut rect = Rect::default();
                GetWindowRect(app.floater, &mut rect);
                let mut point = Point {
                    x: rect.left,
                    y: rect.top,
                };
                ScreenToClient(hwnd, &mut point);
                point
            };

            document(&mut app, &"An English paragraph.\n\n".repeat(20));
            app.layout(hwnd);
            assert!(!app.tab().stats.rtl());
            let point = corner(&app);
            let mid = (app.doc_left() + app.bar().1) / 2;
            assert!(point.x > mid, "expected right of {mid}, got {}", point.x);
            assert!(
                point.y >= app.doc_top() && point.y < app.doc_top() + app.ui_px(40) && shown(&app)
            );

            document(
                &mut app,
                &"هذا نص عربي يقرأ من اليمين إلى اليسار.\n\n".repeat(20),
            );
            app.layout(hwnd);
            assert!(app.tab().stats.rtl());
            assert!(corner(&app).x < mid, "expected left of {mid}");

            document(
                &mut app,
                &format!("{}\n\nكلمة\n", "An English paragraph.\n\n".repeat(20)),
            );
            app.layout(hwnd);
            assert!(!app.tab().stats.rtl());
            assert!(corner(&app).x > mid);

            app.floating = false;
            app.place_floater();
            assert!(!shown(&app));
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn zoom_scales_every_measurement_and_keeps_your_place() {
        unsafe {
            let hwnd = window(900, 500);
            let mut app = App::new();
            document(&mut app, &"A line of text.\n\n".repeat(300));
            app.layout(hwnd);
            let (padding, height, content) = (
                app.px(24),
                app.tab().rows[0].height,
                app.tab().content_height,
            );
            app.scroll_to(hwnd, content / 2);

            app.set_zoom(hwnd, 150);
            assert_eq!(app.zoom, 150);
            assert_eq!(app.px(24), padding * 3 / 2);
            assert!(app.tab().rows[0].height > height);
            assert!(app.tab().content_height > content);
            let before = content as f64 / 2.0 / content as f64;
            let after = app.tab().scroll as f64 / app.tab().content_height as f64;
            assert!((before - after).abs() < 0.02, "{before} became {after}");
            assert_eq!(app.tab().stats.zoom, 150);

            app.set_zoom(hwnd, 10_000);
            assert_eq!(app.zoom, Themes::ZOOM.1);
            app.set_zoom(hwnd, 0);
            assert_eq!(app.zoom, Themes::ZOOM.0);
            app.set_zoom(hwnd, 100);
            app.layout(hwnd);
            assert_eq!((app.px(24), app.tab().content_height), (padding, content));
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn a_new_palette_starts_from_the_one_on_screen_and_is_saved() {
        unsafe {
            let mut app = App::new();
            app.theme_path = std::env::temp_dir().join("mdlite-add-test.ini");
            let _ = std::fs::remove_file(&app.theme_path);
            app.themes = Themes::default();
            app.theme = 1;
            let from = app.palette();
            let index = app.themes.add("Sepia", from);
            app.theme = index;
            assert_eq!(app.palette(), from);
            app.themes.palettes[index]
                .1
                .set_role(0, crate::theme::rgb(0x704214));
            app.save();
            let read = Themes::load(&app.theme_path);
            let sepia = read.find("Sepia").expect("the new section is written");
            assert_eq!(read.start, sepia);
            assert_eq!(
                read.palettes[sepia].1.background,
                crate::theme::rgb(0x704214)
            );
            assert_eq!(
                read.palettes[sepia].1.foreground,
                Palette::light().foreground
            );
            std::fs::remove_file(&app.theme_path).unwrap();
        }
    }
    #[test]
    fn saved_settings_round_trip_without_disturbing_the_rest_of_the_ini() {
        unsafe {
            let mut app = App::new();
            app.theme_path = std::env::temp_dir().join("mdlite-save-test.ini");
            std::fs::write(
                &app.theme_path,
                "; a comment the viewer must not eat\n[dark]\naccent=#123456\n",
            )
            .unwrap();
            app.themes = Themes::load(&app.theme_path);
            app.pinned = Some(Pipe::GdiPlus);
            app.theme = 1;
            app.themes.palettes[1].1.background = crate::theme::rgb(0x102030);
            app.save();
            let text = std::fs::read_to_string(&app.theme_path).unwrap();
            assert!(text.contains("; a comment the viewer must not eat"));
            let themes = Themes::load(&app.theme_path);
            assert_eq!(themes.renderer, Some(Pipe::GdiPlus as u8));
            assert_eq!(pinned(&themes), Some(Pipe::GdiPlus));
            assert_eq!(themes.start, 1);
            assert_eq!(themes.palettes[1].1.background, crate::theme::rgb(0x102030));
            assert_eq!(themes.palettes[0].1.accent, crate::theme::rgb(0x123456));

            app.pinned = None;
            app.theme = 0;
            app.save();
            let themes = Themes::load(&app.theme_path);
            assert_eq!(pinned(&themes), None);
            assert_eq!(themes.start, 0);
            assert_eq!(themes.palettes[1].1.background, crate::theme::rgb(0x102030));
            std::fs::remove_file(&app.theme_path).unwrap();
        }
    }
    #[test]
    fn each_block_takes_the_cheapest_pipeline_that_can_draw_it() {
        let mut app = App::new();
        document(
            &mut app,
            "plain latin\n\n日本語 text\n\nالعربية\n\n😀 emoji\n",
        );
        let pipes = |app: &App| {
            app.tab()
                .blocks
                .iter()
                .filter(|block| block.kind == Kind::Paragraph)
                .map(|block| block.body.pipe)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            pipes(&app),
            [Pipe::Gdi, Pipe::GdiPlus, Pipe::D2d, Pipe::Gdi]
        );
        assert!(
            app.tab().blocks[4].body.shaped != NO_SHAPED
                && app.tab().blocks[0].body.shaped == NO_SHAPED
        );
        let emoji: Vec<_> = app.tab().blocks[6]
            .body
            .run_range()
            .map(|i| &app.tab().runs[app.tab().run_order[i] as usize])
            .collect();
        assert_eq!(emoji.len(), 2);
        assert!(emoji[0].is_emoji() && !emoji[1].is_emoji());

        for (pinned, expected) in [
            (
                Some(Pipe::Gdi),
                [Pipe::Gdi, Pipe::Gdi, Pipe::D2d, Pipe::Gdi],
            ),
            (
                Some(Pipe::GdiPlus),
                [Pipe::GdiPlus, Pipe::GdiPlus, Pipe::D2d, Pipe::GdiPlus],
            ),
            (Some(Pipe::D2d), [Pipe::D2d; 4]),
        ] {
            app.pinned = pinned;
            let source = std::mem::take(&mut app.tab_mut().source);
            document(&mut app, &source);
            assert_eq!(pipes(&app), expected, "pinned {pinned:?}");
        }
    }
    #[test]
    fn menu_items_and_the_scrollbar_paint_from_the_palette() {
        unsafe {
            let (width, height) = (240, 40);
            let surface = Buffer::new(NULL, width, height).expect("an off-screen surface");
            let dc = surface.dc;
            let font = ui_font(12);
            let palette = Palette::dark();
            MENU.set((font, palette, 96));
            let rect = Rect {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            let pixels = || surface.pixels();
            let count = |color: u32| {
                let target = crate::theme::rgb(0) | color.swap_bytes() >> 8;
                pixels().iter().filter(|&&p| p & 0xffffff == target).count()
            };
            for (label, state, background) in [
                ("&Open…\tCtrl+O", 0u32, palette.background),
                ("&Open…\tCtrl+O", 1, palette.accent),
            ] {
                let text: Box<[u16]> = label.encode_utf16().collect();
                let data = &text as *const Box<[u16]> as usize;
                let mut measure = MeasureItem {
                    kind: 1,
                    id: 0,
                    item: 0,
                    width: 0,
                    height: 0,
                    data,
                };
                measure_menu_item(&mut measure);
                assert!(measure.height > 12 && measure.width > 100);
                draw_menu_item(&DrawItem {
                    kind: 1,
                    id: 0,
                    item: 0,
                    action: 0,
                    state,
                    window: NULL,
                    dc,
                    rect,
                    data,
                });
                assert!(count(background) > (width * height / 2) as usize);
                assert!(
                    count(palette.foreground) + count(palette.muted) > 20
                        || state == 1 && count(Palette::contrasting(palette.accent)) > 20
                );
            }
            let empty: Box<[u16]> = Box::new([]);
            let data = &empty as *const Box<[u16]> as usize;
            let mut measure = MeasureItem {
                kind: 1,
                id: 0,
                item: 0,
                width: 0,
                height: 0,
                data,
            };
            measure_menu_item(&mut measure);
            assert_eq!(measure.width, 0);
            fill(dc, rect, palette.background);
            draw_menu_item(&DrawItem {
                kind: 1,
                id: 0,
                item: 0,
                action: 0,
                state: 0,
                window: NULL,
                dc,
                rect,
                data,
            });
            assert!(count(palette.rule) > 100 && count(palette.rule) < 2000);
            DeleteObject(font);
        }
    }
    #[test]
    fn painting_composes_into_a_back_buffer_and_a_scroll_repaints_a_sliver() {
        unsafe {
            let hwnd = window(900, 560);
            let mut app = App::new();
            document(
                &mut app,
                &format!(
                    "{}{}",
                    "العربية **نص** and English 😀\n\n".repeat(20),
                    "A plain line.\n\n".repeat(200)
                ),
            );
            app.layout(hwnd);
            InvalidateRect(hwnd, ptr::null(), 0);
            app.paint(hwnd);
            let (dc, width, height) = {
                let buffer = app.buffer.as_ref().expect("a back buffer must be built");
                (buffer.dc, buffer.width, buffer.height)
            };
            assert_eq!((width, height), (app.width, app.client_h()));
            app.draw(
                dc,
                Rect {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                },
            );
            let background = app.palette().background;
            assert_eq!(
                GetPixel(dc, app.doc_left() + 4, app.doc_top() + 4),
                background
            );
            let mut painted = 0;
            for y in (app.doc_top()..app.doc_bottom()).step_by(3) {
                for x in (app.doc_left()..app.bar().1).step_by(3) {
                    painted += u32::from(GetPixel(dc, x, y) != background);
                }
            }
            assert!(painted > 500, "only {painted} painted pixels in the buffer");

            let thumb = app.thumb();
            app.scroll_to(hwnd, 60);
            let update = app.bar_update(thumb);
            let (moved, _) = app.thumb().unwrap();
            assert_eq!(update.right - update.left, app.bar().0);
            assert!(
                update.top <= app.doc_top() + moved && update.bottom - update.top < app.height / 2
            );
            let all = app.bar_update(None);
            assert_eq!((all.top, all.bottom), (app.doc_top(), app.doc_bottom()));
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn scroll_state_survives_without_a_system_scrollbar() {
        unsafe {
            let hwnd = window(400, 300);
            let mut info = ScrollInfo::new(1 | 2 | 4);
            info.max = 5000;
            info.page = 300;
            info.pos = 1234;
            SetScrollInfo(hwnd, 1, &info, 0);
            let mut read = ScrollInfo::new(1 | 2 | 4);
            GetScrollInfo(hwnd, 1, &mut read);
            assert_eq!((read.max, read.page, read.pos), (5000, 300, 1234));
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn fine_wheel_input_preserves_distance_and_reverses_without_drift() {
        let mut remainder = 0;
        assert_eq!(wheel_pixels(30, 72, &mut remainder), 18);
        let forward: i32 = (0..120).map(|_| wheel_pixels(1, 73, &mut remainder)).sum();
        assert_eq!(forward, 73);
        assert_eq!(remainder, 0);
        let backward: i32 = (0..120).map(|_| wheel_pixels(-1, 73, &mut remainder)).sum();
        assert_eq!(forward + backward, 0);
        assert_eq!(remainder, 0);
        assert_eq!(wheel_pixels(120, 0, &mut remainder), 0);
    }
    #[test]
    fn smooth_scrolling_eases_to_its_target_and_then_stops() {
        unsafe {
            let hwnd = window(700, 480);
            let mut app = App::new();
            document(&mut app, &"A line of text.\n\n".repeat(400));
            app.layout(hwnd);
            let frame = |app: &mut App| {
                app.last_frame = Instant::now() - std::time::Duration::from_millis(8);
                app.glide(hwnd);
            };
            let settle = |app: &mut App| {
                let mut frames = 0;
                while app.tab().animating && frames < 500 {
                    frame(app);
                    frames += 1;
                }
                frames
            };
            app.smooth = false;
            app.glide_to(hwnd, 500);
            assert_eq!((app.tab().scroll, app.tab().target), (500, 500));
            assert!(!app.tab().animating);

            app.smooth = true;
            app.scroll_to(hwnd, 0);
            app.glide_to(hwnd, 500);
            assert_eq!(app.tab().scroll, 0);
            assert_eq!(app.tab().target, 500);
            assert!(app.tab().animating);
            let mut previous = 0;
            let mut frames = 0;
            while app.tab().animating && frames < 500 {
                frame(&mut app);
                assert!(app.tab().scroll >= previous && app.tab().scroll <= 500);
                let most = (SPEED * app.height as f64 * 0.0085).ceil() as i32 + 1;
                assert!(app.tab().scroll - previous <= most);
                previous = app.tab().scroll;
                frames += 1;
            }
            assert_eq!(app.tab().scroll, 500);
            assert!((4..90).contains(&frames));

            app.glide_to(hwnd, 1000);
            frame(&mut app);
            app.glide_to(hwnd, 100);
            assert_eq!(app.tab().target, 100);
            settle(&mut app);
            assert_eq!(app.tab().scroll, 100);

            app.scroll_to(hwnd, 0);
            app.glide_to(hwnd, 300);
            app.last_frame = Instant::now() - std::time::Duration::from_millis(100);
            app.glide(hwnd);
            let late = app.tab().scroll;
            app.scroll_to(hwnd, 0);
            app.glide_to(hwnd, 300);
            frame(&mut app);
            assert!(late > app.tab().scroll * 3);
            settle(&mut app);

            app.scroll_to(hwnd, 0);
            app.glide_to(hwnd, 400);
            frame(&mut app);
            app.tab_mut().scroll = 50;
            frame(&mut app);
            assert!(app.tab().scroll >= 50 && app.tab().scroll < 400);
            settle(&mut app);
            assert_eq!(app.tab().scroll, 400);

            app.glide_to(hwnd, i32::MAX);
            assert!(!app.tab().animating);
            assert_eq!(app.tab().scroll, app.tab().content_height - app.height);
            app.glide_to(hwnd, -50);
            assert_eq!(app.tab().scroll, 0);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn a_scrolled_frame_is_pixel_for_pixel_what_a_full_repaint_draws() {
        unsafe {
            for outline in [true, false] {
                let hwnd = window(820, 480);
                let mut app = App::new();
                app.outline_open = outline;
                let mut source = String::new();
                for n in 0..60 {
                    source.push_str(&format!(
                        "## Heading {n}\n\nA paragraph with **bold**, *italic*, `code`, a [link](x) and \
                         enough words to wrap onto a second line of this narrow window. 😀\n\n\
                         ```\nfn code_{n}() {{}}\n```\n\n> A quote\n\n| a | b |\n| - | - |\n| {n} | 世界 |\n\n"
                    ));
                }
                source.push_str("العربية **نص** and English\n\n");
                document(&mut app, &source);
                app.layout(hwnd);
                InvalidateRect(hwnd, ptr::null(), 0);
                app.paint(hwnd);
                let (width, full_h) = (app.width, app.client_h());
                let whole = Rect {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: full_h,
                };
                let farthest = app.tab().content_height - app.height;
                assert!(farthest > 3 * app.height);
                let mut at = 0;
                for delta in [
                    1,
                    2,
                    3,
                    7,
                    22,
                    57,
                    -1,
                    -5,
                    -31,
                    130,
                    -200,
                    311,
                    -90,
                    app.height - 1,
                    1 - app.height,
                    2 * app.height,
                ] {
                    at = (at + delta).clamp(0, farthest);
                    app.scroll_to(hwnd, at);
                    assert_eq!(app.tab().scroll, at);
                    let reference = Buffer::new(NULL, whole.right, whole.bottom).unwrap();
                    app.draw(reference.dc, whole);
                    let (shown, expected) =
                        (app.buffer.as_ref().unwrap().pixels(), reference.pixels());
                    let first_difference = (0..(width * full_h) as usize)
                        .find(|&i| shown[i] & 0xffffff != expected[i] & 0xffffff);
                    assert_eq!(
                        first_difference.map(|i| (i as i32 % width, i as i32 / width)),
                        None,
                        "outline={outline}: scrolling by {delta} to {at} drew a different frame"
                    );
                }
                app.set_zoom(hwnd, 150);
                assert!(app.buffer.is_none());
                InvalidateRect(hwnd, ptr::null(), 0);
                app.paint(hwnd);
                let whole = Rect {
                    bottom: app.client_h(),
                    ..whole
                };
                let reference = Buffer::new(NULL, whole.right, whole.bottom).unwrap();
                app.draw(reference.dc, whole);
                let (shown, expected) = (app.buffer.as_ref().unwrap().pixels(), reference.pixels());
                assert!((0..(width * app.client_h()) as usize)
                    .all(|i| shown[i] & 0xffffff == expected[i] & 0xffffff));
                DestroyWindow(hwnd);
            }
        }
    }
    #[test]
    fn the_scrollbar_tracks_the_document_and_the_thumb_stays_grabbable() {
        unsafe {
            let hwnd = window(700, 480);
            let mut app = App::new();
            document(&mut app, &"A line of text.\n\n".repeat(400));
            app.layout(hwnd);
            let (bar_width, bar_x) = app.bar();
            assert!(bar_width > 0 && bar_x + bar_width == app.width);
            let (top, height) = app.thumb().expect("a long document needs a thumb");
            assert_eq!(top, 0);
            assert!(height >= app.ui_px(28) && height < app.height);
            app.scroll_to(hwnd, i32::MAX);
            let (bottom_top, _) = app.thumb().unwrap();
            assert_eq!(bottom_top, app.height - height);
            app.grab = Some(4);
            let target = app.bar_target(app.doc_top() + 4);
            assert_eq!(target, 0);
            assert_eq!(
                app.bar_target(app.doc_top() + app.height),
                app.tab().content_height - app.height
            );
            app.grab = None;
            app.scroll_to(hwnd, app.tab().content_height / 2);
            assert!(app.bar_target(app.doc_top()) < app.tab().scroll);
            assert!(app.bar_target(app.doc_bottom() - 1) > app.tab().scroll);
            document(&mut app, "one line");
            app.layout(hwnd);
            assert!(app.thumb().is_none());
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn closing_the_settings_window_puts_a_previewed_palette_back() {
        unsafe {
            let owner = window(700, 500);
            let state = RefCell::new(App::new());
            {
                let mut app = state.borrow_mut();
                app.themes = Themes::parse(include_str!("../mdlite.ini"));
                app.build_menu(owner);
            }
            let original = state.borrow().palette();
            let hwnd = settings_window(owner, &state);
            assert!(state.borrow().restore.is_some());
            {
                let mut app = state.borrow_mut();
                app.theme = 2;
                app.themes.palettes[2]
                    .1
                    .set_role(0, crate::theme::rgb(0x123456));
            }
            preview(hwnd, &state);
            assert_ne!(state.borrow().palette(), original);
            assert_eq!(MENU.get().1, state.borrow().palette());
            DestroyWindow(hwnd);
            assert!(state.borrow().restore.is_none());
            assert_eq!(state.borrow().palette(), original);
            assert_eq!(
                state.borrow().themes.palettes[2].1.background,
                Themes::parse(include_str!("../mdlite.ini")).palettes[2]
                    .1
                    .background
            );
            DestroyWindow(owner);
        }
    }
    #[test]
    fn every_palette_can_be_applied_and_the_menu_follows_it() {
        unsafe {
            let hwnd = window(600, 400);
            let mut app = App::new();
            app.themes = Themes::parse(include_str!("../mdlite.ini"));
            assert!(app.themes.palettes.len() >= 10);
            app.build_menu(hwnd);
            for index in 0..app.themes.palettes.len() {
                app.theme = index;
                app.apply_theme(hwnd);
                let (font, palette, dpi) = MENU.get();
                assert!(!font.is_null());
                assert_eq!(palette, app.palette());
                assert_eq!(dpi, app.dpi);
                assert!(!app.menu_brush.is_null());
            }
            let labels = app.menu_labels.len();
            app.build_menu(hwnd);
            assert_eq!(app.menu_labels.len(), labels);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn the_status_bar_reports_the_document_and_answers_a_click() {
        unsafe {
            let hwnd = window(800, 520);
            let mut app = App::new();
            document(&mut app, "# Title\n\nplain latin\n\nالعربية and 😀\n");
            app.layout(hwnd);
            assert_eq!(app.tab().stats.lines, 5);
            assert_eq!(app.tab().stats.words, 7);
            let spread = app.tab().stats.spread(None);
            assert!(spread[Pipe::Gdi as usize] > 0 && spread[Pipe::D2d as usize] > 0);
            assert!(app.tab().stats.label(None).starts_with("Direct2D + mixed"));
            assert_eq!(app.tab().stats.warning(None), None);
            assert!(app
                .tab()
                .stats
                .label(Some(Pipe::Gdi))
                .starts_with("GDI (pinned)"));
            assert_eq!(
                app.tab().stats.warning(Some(Pipe::Gdi)),
                Some("1 block still needs Direct2D".to_owned())
            );
            assert_eq!(app.tab().stats.warning(Some(Pipe::D2d)), None);

            let dc = GetDC(hwnd);
            app.draw(
                dc,
                Rect {
                    left: 0,
                    top: 0,
                    right: app.width,
                    bottom: app.client_h(),
                },
            );
            ReleaseDC(hwnd, dc);
            let (edge, footer) = (app.width - app.ui_px(16), app.doc_bottom() + 4);
            assert_eq!(app.hit(edge - 1, footer), Some(Hit::Engine));
            assert_eq!(app.hit(edge - 1, app.doc_bottom() - 2), None);
            let zoom: Vec<Hit> = app
                .hot
                .borrow()
                .iter()
                .map(|(_, spot)| *spot)
                .take(3)
                .collect();
            assert_eq!(zoom, [Hit::ZoomOut, Hit::ZoomReset, Hit::ZoomIn]);
            let cells = app.hot.borrow();
            for (rect, spot) in cells.iter().take(3) {
                let middle = (rect.left + rect.right) / 2;
                assert_eq!(app.hit(middle, footer), Some(*spot));
            }
            assert!(cells[2].0.right < cells[3].0.left);
            drop(cells);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn tables_size_columns_align_cells_and_stay_inside_the_page() {
        unsafe {
            let hwnd = window(900, 540);
            let mut app = App::new();
            document(
                &mut app,
                "intro\n\n| Fruit | Qty | Price |\n|:------|:---:|------:|\n| Apple | 3 | 1.50 |\n| Kiwi | 12 | 0.25 |\n\nafter\n",
            );
            let headers: Vec<_> = app
                .tab()
                .blocks
                .iter()
                .filter_map(|block| match block.kind {
                    Kind::Row(header) => Some(header),
                    _ => None,
                })
                .collect();
            assert_eq!(headers, [true, false, false]);
            app.layout(hwnd);
            let (col_left, col_right) = app.column();
            let grid: Vec<_> = app
                .tab()
                .rows
                .iter()
                .filter(|row| row.deco == Deco::Grid)
                .collect();
            assert_eq!(
                grid.len(),
                4,
                "top border plus one separator under each row"
            );
            assert!(grid.iter().all(|row| row.span.0 == col_left
                && row.span.1 <= col_right
                && row.span.1 > row.span.0));
            assert!(grid[1].height > grid[2].height, "header rule is heavier");

            document(
                &mut app,
                &format!(
                    "| {} | {} |\n|---|---|\n| {} | short |\n",
                    "Heading one".repeat(8),
                    "Heading two".repeat(8),
                    "body ".repeat(60)
                ),
            );
            app.layout(hwnd);
            let (col_left, col_right) = app.column();
            for row in &app.tab().rows {
                for part in app.tab().row_parts(row) {
                    assert!(part.x >= col_left && part.x + part.width <= col_right);
                }
            }
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn complex_paragraphs_keep_styles_share_storage_and_relayout_on_dpi() {
        unsafe {
            let hwnd = window(400, 340);
            let mut app = App::new();
            document(
                &mut app,
                "العربية **نص** English 123 😀\nالعربية **نص** English 123 😀\n# العربية **نص** English 123 😀",
            );
            assert!(Rc::ptr_eq(
                app.tab().piece_shaped(app.tab().blocks[0].body).unwrap(),
                app.tab().piece_shaped(app.tab().blocks[1].body).unwrap()
            ));
            assert!(app.tab().runs.is_empty());
            app.layout(hwnd);
            assert!(app.tab().rows.iter().all(|row| row.shaped != NO_SHAPED));
            assert_eq!(app.tab().rows[0].height, app.tab().rows[1].height);
            assert!(app.tab().rows[2].height > app.tab().rows[1].height);
            let height = app.tab().content_height;
            app.set_dpi(144);
            app.layout(hwnd);
            assert!(app.tab().content_height > height);
            app.set_dpi(96);
            app.layout(hwnd);
            assert_eq!(app.tab().content_height, height);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn native_layout_wraps_without_splitting_surrogates() {
        unsafe {
            let hwnd = window(340, 320);
            let mut app = App::new();
            let source = format!(
                "# Heading\n\n{}\n```\n{}\n```\n{}",
                "𝐡ello **bold** ".repeat(150),
                "x".repeat(12_000),
                "line\n".repeat(4000)
            );
            document(&mut app, &source);
            app.layout(hwnd);
            assert!(app.tab().content_height > 65535);
            assert!(app.tab().rows.len() > 4000);
            assert!(app
                .tab()
                .rows
                .windows(2)
                .all(|r| r[0].y + r[0].height <= r[1].y));
            let (_, col_right) = app.column();
            for row in &app.tab().rows {
                for part in app.tab().row_parts(row) {
                    let text = &app.tab().run_text(part.run as usize)[part.range()];
                    assert!(String::from_utf16(text).is_ok());
                    assert!(part.width > 0);
                    assert!(part.x + part.width <= col_right);
                }
            }
            app.scroll_to(hwnd, i32::MAX);
            assert_eq!(app.tab().scroll, app.tab().content_height - app.height);
            let mut info = ScrollInfo::new(4);
            GetScrollInfo(hwnd, 1, &mut info);
            assert_eq!(info.pos, app.tab().scroll);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn shared_text_keeps_font_context_and_document_order() {
        let mut app = App::new();
        document(&mut app, "# same\nsame\nsame\n**same**\n");
        assert!(app.tab().run_order.len() > app.tab().runs.len());
        assert_ne!(
            app.tab().run_order[0],
            app.tab().run_order[1],
            "heading text must not share metrics with body text"
        );
        assert_eq!(app.tab().run_order[1], app.tab().run_order[2]);
        assert_ne!(app.tab().run_order[1], app.tab().run_order[3]);
    }

    // ── New feature tests: selection/copy, outline, tabs, and clickable links ──

    #[test]
    fn selection_copies_plain_text_and_markdown_accurately() {
        unsafe {
            let hwnd = window(900, 600);
            let mut app = App::new();
            let src = "# Hello World\n\nThis is **bold** and [a link](./other.md) and `code`.\n\n> A wise quote\n";
            document(&mut app, src);
            app.layout(hwnd);

            // Select all: Ctrl+C plain text must have NO markdown syntax,
            // while Copy as Markdown preserves the exact original Markdown.
            app.select_all();
            let plain = app.tab().selected_plain();
            assert!(plain.contains("Hello World"));
            assert!(plain.contains("This is bold and a link and code."));
            assert!(plain.contains("A wise quote"));
            assert!(
                !plain.contains("**"),
                "plain copy must strip bold markers: {plain}"
            );
            assert!(
                !plain.contains("./other.md"),
                "plain copy must strip link URLs: {plain}"
            );
            assert!(
                !plain.contains("# "),
                "plain copy must strip heading hashes: {plain}"
            );

            let md = app.tab().selected_markdown();
            assert!(md.contains("# Hello World"));
            assert!(md.contains("**bold**"));
            assert!(md.contains("[a link](./other.md)"));
            assert!(md.contains("`code`"));
            assert!(md.contains("> A wise quote"));

            // Partial inline selection inside the paragraph row:
            let para_row = app
                .tab()
                .rows
                .iter()
                .enumerate()
                .position(|(i, r)| {
                    let s = app.tab().row_plain_slice(i, 0, app.tab().row_text_len(r));
                    s.contains("bold")
                })
                .expect("paragraph row");
            let full_line = app.tab().row_plain_slice(
                para_row,
                0,
                app.tab().row_text_len(&app.tab().rows[para_row]),
            );
            let b_start = full_line.find("bold").unwrap();
            let l_end = full_line.find("a link").unwrap() + "a link".len();
            app.tab_mut().selection = Some((
                SelPos {
                    row: para_row,
                    col: b_start,
                },
                SelPos {
                    row: para_row,
                    col: l_end,
                },
            ));
            assert_eq!(app.tab().selected_plain(), "bold and a link");
            assert_eq!(
                app.tab().selected_markdown(),
                "**bold** and [a link](./other.md)"
            );
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn outline_maps_headings_tracks_scroll_spy_and_jumps_on_click() {
        unsafe {
            let hwnd = window(1000, 500);
            let mut app = App::new();
            app.outline_open = true;
            let mut src = String::from("# Intro\n\n");
            src.push_str(&"Filler paragraph line.\n\n".repeat(30));
            src.push_str("## Architecture\n\n");
            src.push_str(&"More details here.\n\n".repeat(30));
            src.push_str("### Rendering Pipeline\n\nFinal text.\n");
            document(&mut app, &src);
            app.layout(hwnd);

            assert_eq!(app.tab().headings.len(), 3);
            assert_eq!(
                app.tab()
                    .headings
                    .iter()
                    .map(|h| (h.level, h.title.as_str(), h.slug.as_str()))
                    .collect::<Vec<_>>(),
                [
                    (1, "Intro", "intro"),
                    (2, "Architecture", "architecture"),
                    (3, "Rendering Pipeline", "rendering-pipeline"),
                ]
            );
            assert!(app.tab().headings[0].y < app.tab().headings[1].y);
            assert!(app.tab().headings[1].y < app.tab().headings[2].y);
            assert_eq!(app.tab().active_heading(app.px(48)), Some(0));

            // Anchor jump scrolls directly to the heading and updates scroll-spy.
            app.smooth = false;
            assert!(app.scroll_to_anchor(hwnd, "architecture"));
            assert_eq!(app.tab().active_heading(app.px(48)), Some(1));

            // Painting populates clickable outline hit rects.
            let buffer = Buffer::new(NULL, app.width, app.client_h()).unwrap();
            app.draw(
                buffer.dc,
                Rect {
                    left: 0,
                    top: 0,
                    right: app.width,
                    bottom: app.client_h(),
                },
            );
            assert_eq!(app.outline_hot.borrow().len(), 3);
            let (rect, idx) = app.outline_hot.borrow()[2];
            assert_eq!(idx, 2);
            assert_eq!(
                app.hit_outline((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2),
                Some(2)
            );
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn tabs_open_switch_deduplicate_and_follow_md_links_into_new_tab() {
        unsafe {
            let dir = std::env::temp_dir().join(format!("mdlite-tabs-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let a = dir.join("index.md");
            let b = dir.join("guide.md");
            std::fs::write(&a, "# Index\n\nRead the [Guide](./guide.md#details).\n").unwrap();
            std::fs::write(
                &b,
                &format!(
                    "# Guide\n\n{}## Details\n\nLinked target.\n",
                    "Body line.\n\n".repeat(40)
                ),
            )
            .unwrap();

            let hwnd = window(960, 560);
            let state = RefCell::new(App::new());
            state.borrow_mut().smooth = false;
            assert!(load_file_tab(hwnd, &state, a.clone(), false, None));
            assert_eq!(state.borrow().tabs.len(), 1);
            assert_eq!(state.borrow().tab().title, "index.md");

            // Clicking the `.md` link opens `guide.md` in a NEW tab and scrolls to `#details`!
            activate_link(hwnd, &state, "./guide.md#details");
            assert_eq!(state.borrow().tabs.len(), 2);
            assert_eq!(state.borrow().active, 1);
            assert_eq!(state.borrow().tab().title, "guide.md");
            assert!(
                state.borrow().tab().scroll > 0,
                "anchor fragment in .md link must scroll the newly opened tab"
            );

            // Clicking the same link again reuses the existing tab instead of duplicating.
            state.borrow_mut().switch_tab(hwnd, 0);
            assert_eq!(state.borrow().active, 0);
            activate_link(hwnd, &state, "./guide.md");
            assert_eq!(state.borrow().tabs.len(), 2);
            assert_eq!(state.borrow().active, 1);

            // Closing tab 1 returns cleanly to tab 0.
            state.borrow_mut().close_tab(hwnd, 1);
            assert_eq!(state.borrow().tabs.len(), 1);
            assert_eq!(state.borrow().active, 0);
            assert_eq!(state.borrow().tab().title, "index.md");

            DestroyWindow(hwnd);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn glyph_width_cache_is_pixel_exact_with_gdi() {
        unsafe {
            let hwnd = window(900, 600);
            let dc = GetDC(hwnd);
            assert!(!dc.is_null());
            let mut app = App::new();
            let ascii_all: Vec<u16> = (0x20u16..=0x7eu16).collect();
            let samples: Vec<Vec<u16>> = vec![
                ascii_all,
                "The quick brown fox jumps over the lazy dog 0123456789!@#$%^&*()"
                    .encode_utf16()
                    .collect(),
                "fn main() -> Result<(), Box<dyn std::error::Error>> { Ok(()) }"
                    .encode_utf16()
                    .collect(),
                "Grüße aus München — Ελληνικά — Українська — 日本語テスト"
                    .encode_utf16()
                    .collect(),
            ];
            for dpi in [96u32, 120, 144] {
                app.set_dpi(dpi);
                for zoom in [75, 100, 150] {
                    app.zoom = zoom;
                    app.rescale();
                    let font_specs = [
                        (Style::default(), 0),
                        (Style(Style::BOLD), 0),
                        (Style(Style::ITALIC), 0),
                        (Style(Style::CODE), 0),
                        (Style::default(), 1),
                        (Style::default(), 2),
                    ];
                    let mut selected = usize::MAX;
                    for (style, heading) in font_specs {
                        let fi = app.font(dc, style, heading);
                        for sample in &samples {
                            let cached_w =
                                app.fonts[fi].measure_width(dc, &mut selected, fi, sample);
                            let old = SelectObject(dc, app.fonts[fi].handle);
                            let mut exact = Size::default();
                            GetTextExtentPoint32W(
                                dc,
                                sample.as_ptr(),
                                sample.len() as i32,
                                &mut exact,
                            );
                            // Also test measure_fit at mid-line wrap width.
                            let max_w = (exact.cx / 2).max(1);
                            let mut gdi_fit = 0i32;
                            let mut gdi_sz = Size::default();
                            GetTextExtentExPointW(
                                dc,
                                sample.as_ptr(),
                                sample.len() as i32,
                                max_w,
                                &mut gdi_fit,
                                ptr::null_mut(),
                                &mut gdi_sz,
                            );
                            SelectObject(dc, old);
                            selected = usize::MAX;
                            let (cached_fit, fit_total) =
                                app.fonts[fi].measure_fit(dc, &mut selected, fi, sample, max_w);
                            assert_eq!(
                                cached_w, fit_total,
                                "measure_width and measure_fit total diverged at dpi={dpi} zoom={zoom}"
                            );
                            // Pure printable ASCII is strictly 0-px exact with GDI; multi-script
                            // BMP fallback strings at huge bold sizes can differ by a few subpixels.
                            let tol = if sample.iter().all(|&u| u < 128) {
                                0
                            } else {
                                (exact.cx / 250).max(2)
                            };
                            assert!(
                                (cached_w - exact.cx).abs() <= tol,
                                "width mismatch at dpi={dpi} zoom={zoom} style={style:?} h={heading}: cached={cached_w} gdi={}",
                                exact.cx
                            );
                            assert!(
                                (cached_fit as i32 - gdi_fit).abs() <= 1,
                                "fit mismatch at dpi={dpi} zoom={zoom}: cached={cached_fit} gdi={gdi_fit}"
                            );
                        }
                    }
                }
            }
            ReleaseDC(hwnd, dc);
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn selection_handles_backward_drag_word_code_tables_and_bidi() {
        unsafe {
            let hwnd = window(900, 600);
            let mut app = App::new();
            let src = concat!(
                "# Heading\n\n",
                "Alpha **beta gamma** delta.\n\n",
                "```\nlet a = 1;\nlet b = 2;\n```\n\n",
                "| Col1 | Col2 |\n|---|---|\n| left | right |\n\n",
                "العربية **نص** English\n"
            );
            document(&mut app, src);
            app.layout(hwnd);

            // Empty selection returns empty strings.
            app.tab_mut().selection = Some((SelPos { row: 0, col: 2 }, SelPos { row: 0, col: 2 }));
            assert_eq!(app.tab().selected_plain(), "");
            assert_eq!(app.tab().selected_markdown(), "");

            // Backward drag produces identical plain & Markdown output as forward drag.
            let para_row = app
                .tab()
                .rows
                .iter()
                .enumerate()
                .position(|(i, r)| {
                    app.tab()
                        .row_plain_slice(i, 0, app.tab().row_text_len(r))
                        .contains("beta gamma")
                })
                .expect("paragraph row");
            let line = app.tab().row_plain_slice(
                para_row,
                0,
                app.tab().row_text_len(&app.tab().rows[para_row]),
            );
            let s = line.find("beta").unwrap();
            let e = line.find("delta").unwrap();
            app.tab_mut().selection = Some((
                SelPos {
                    row: para_row,
                    col: e,
                },
                SelPos {
                    row: para_row,
                    col: s,
                },
            ));
            assert_eq!(app.tab().selected_plain(), "beta gamma ");
            assert_eq!(app.tab().selected_markdown(), "**beta gamma** ");

            // Double-click word selection picks the exact word under the cursor.
            app.select_word_at(SelPos {
                row: para_row,
                col: line.find("gamma").unwrap() + 2,
            });
            assert_eq!(app.tab().selected_plain(), "gamma");
            assert_eq!(app.tab().selected_markdown(), "**gamma**");

            // Full code block selection: plain text is raw code; Markdown wraps in fences.
            let code_rows: Vec<usize> = app
                .tab()
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.deco == Deco::Code)
                .map(|(i, _)| i)
                .collect();
            assert_eq!(code_rows.len(), 2);
            let last_code_len = app.tab().row_text_len(&app.tab().rows[code_rows[1]]);
            app.tab_mut().selection = Some((
                SelPos {
                    row: code_rows[0],
                    col: 0,
                },
                SelPos {
                    row: code_rows[1],
                    col: last_code_len,
                },
            ));
            assert_eq!(app.tab().selected_plain(), "let a = 1;\r\nlet b = 2;");
            let code_md = app.tab().selected_markdown();
            assert!(code_md.starts_with("```\r\n") && code_md.ends_with("\r\n```"));
            assert!(code_md.contains("let a = 1;") && code_md.contains("let b = 2;"));

            // Table selection: plain text separates cells with `\t`, no `|`.
            let left_row = app
                .tab()
                .rows
                .iter()
                .enumerate()
                .position(|(i, r)| {
                    app.tab().row_plain_slice(i, 0, app.tab().row_text_len(r)) == "left"
                })
                .expect("left cell");
            let right_row = app
                .tab()
                .rows
                .iter()
                .enumerate()
                .position(|(i, r)| {
                    app.tab().row_plain_slice(i, 0, app.tab().row_text_len(r)) == "right"
                })
                .expect("right cell");
            app.tab_mut().selection = Some((
                SelPos {
                    row: left_row,
                    col: 0,
                },
                SelPos {
                    row: right_row,
                    col: 5,
                },
            ));
            assert_eq!(app.tab().selected_plain(), "left\tright");

            // Direct2D shaped Arabic/RTL paragraph selection works for both plain and Markdown.
            let rtl_row = app
                .tab()
                .rows
                .iter()
                .enumerate()
                .position(|(_, r)| r.shaped != NO_SHAPED)
                .expect("shaped RTL row");
            let rtl_len = app.tab().row_text_len(&app.tab().rows[rtl_row]);
            app.tab_mut().selection = Some((
                SelPos {
                    row: rtl_row,
                    col: 0,
                },
                SelPos {
                    row: rtl_row,
                    col: rtl_len,
                },
            ));
            let rtl_plain = app.tab().selected_plain();
            let rtl_md = app.tab().selected_markdown();
            assert!(
                rtl_plain.contains("العربية")
                    && rtl_plain.contains("English")
                    && !rtl_plain.contains("**")
            );
            assert!(rtl_md.contains("**نص**"));

            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn outline_handles_empty_250_headings_clamp_and_pixel_exact_scroll_with_selection() {
        unsafe {
            let hwnd = window(960, 560);
            let mut app = App::new();
            app.outline_open = true;
            app.smooth = false;

            // Empty-headings document paints cleanly with 0 hot outline rects.
            document(&mut app, "Just a plain paragraph with no headings.\n");
            app.layout(hwnd);
            assert!(app.tab().headings.is_empty());
            assert_eq!(app.tab().active_heading(app.px(48)), None);
            let surface = Buffer::new(NULL, app.width, app.client_h()).unwrap();
            app.draw(
                surface.dc,
                Rect {
                    left: 0,
                    top: 0,
                    right: app.width,
                    bottom: app.client_h(),
                },
            );
            assert!(app.outline_hot.borrow().is_empty());

            // 250 headings across H1..H6: scroll-spy tracks every jump accurately.
            let mut src = String::new();
            for i in 0..250 {
                let hashes = "#".repeat((i % 6) + 1);
                src.push_str(&format!(
                    "{hashes} Section {i}\n\nParagraph {i} body text.\n\n"
                ));
            }
            document(&mut app, &src);
            app.layout(hwnd);
            assert_eq!(app.tab().headings.len(), 250);
            assert!(app.scroll_to_anchor(hwnd, "section-125"));
            assert_eq!(app.tab().active_heading(app.px(48)), Some(125));
            assert!(app.scroll_to_anchor(hwnd, "section-0"));
            assert_eq!(app.tab().active_heading(app.px(48)), Some(0));
            assert!(!app.scroll_to_anchor(hwnd, "nonexistent-heading"));

            // Sidebar width clamps cleanly when toggling outline off and on.
            let open_w = app.sidebar_w();
            assert!(open_w > 0);
            app.outline_open = false;
            assert_eq!(app.sidebar_w(), 0);
            app.layout(hwnd);
            app.outline_open = true;
            app.layout(hwnd);
            assert_eq!(app.sidebar_w(), open_w);

            // Pixel-exact incremental scrolling with BOTH active selection AND outline open!
            app.select_all();
            InvalidateRect(hwnd, ptr::null(), 0);
            app.paint(hwnd);
            let whole = Rect {
                left: 0,
                top: 0,
                right: app.width,
                bottom: app.client_h(),
            };
            for target in [45, 190, 620, 120, 0] {
                app.scroll_to(hwnd, target);
                let reference = Buffer::new(NULL, whole.right, whole.bottom).unwrap();
                app.draw(reference.dc, whole);
                let (shown, expected) = (app.buffer.as_ref().unwrap().pixels(), reference.pixels());
                let diff = (0..(whole.right * whole.bottom) as usize)
                    .find(|&i| shown[i] & 0xffffff != expected[i] & 0xffffff);
                assert_eq!(
                    diff, None,
                    "selection+outline scroll mismatch at y={target}"
                );
            }
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn tabs_stress_12_tabs_cache_hit_and_closing_invariants() {
        unsafe {
            let dir = std::env::temp_dir().join(format!("mdlite-tabstress-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let mut paths = Vec::new();
            for i in 0..12 {
                let p = dir.join(format!("doc {i}.md"));
                std::fs::write(&p, format!("# Doc {i}\n\nBody for tab {i}.\n")).unwrap();
                paths.push(p);
            }

            let hwnd = window(960, 560);
            let state = RefCell::new(App::new());
            state.borrow_mut().smooth = false;
            for (i, p) in paths.iter().enumerate() {
                assert!(load_file_tab(hwnd, &state, p.clone(), i > 0, None));
            }
            assert_eq!(state.borrow().tabs.len(), 12);
            assert_eq!(state.borrow().active, 11);

            // O(1) cached tab switch: layout_sig matches so `rows` pointer is preserved (no relayout!).
            let tab0_rows_ptr = state.borrow().tabs[0].rows.as_ptr();
            let tab0_rows_len = state.borrow().tabs[0].rows.len();
            state.borrow_mut().switch_tab(hwnd, 0);
            assert_eq!(state.borrow().active, 0);
            assert_eq!(state.borrow().tabs[0].rows.as_ptr(), tab0_rows_ptr);
            assert_eq!(state.borrow().tabs[0].rows.len(), tab0_rows_len);

            // Closing middle, first, and last tabs always leaves a valid `active` index.
            state.borrow_mut().close_tab(hwnd, 5);
            assert_eq!(state.borrow().tabs.len(), 11);
            assert!(state.borrow().active < state.borrow().tabs.len());

            state.borrow_mut().switch_tab(hwnd, 10);
            state.borrow_mut().close_tab(hwnd, 10);
            assert_eq!(state.borrow().active, 9);

            // Close all remaining file tabs; the last one resets cleanly to Welcome.
            while state.borrow().tabs.len() > 1 {
                state.borrow_mut().close_tab(hwnd, 0);
            }
            assert_eq!(state.borrow().tabs.len(), 1);
            state.borrow_mut().close_tab(hwnd, 0);
            assert_eq!(state.borrow().tabs.len(), 1);
            assert!(state.borrow().tab().file.is_none());
            assert!(state.borrow().tab().source.contains("MDLite"));

            DestroyWindow(hwnd);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn flat_arena_invariants_hold_across_tables_reloads_and_resizes() {
        unsafe {
            let hwnd = window(840, 560);
            let mut app = App::new();
            let docs = [
                include_str!("../examples/showcase.md"),
                include_str!("../examples/tables.md"),
                include_str!("../examples/emoji.md"),
                include_str!("../examples/languages.md"),
            ];
            for _ in 0..3 {
                for src in docs {
                    document(&mut app, src);
                    for width in [420, 960, 1280] {
                        SetWindowPos(hwnd, NULL, 0, 0, width, 560, 0x14);
                        app.layout(hwnd);
                        let tab = app.tab();
                        for run in &tab.runs {
                            assert!(
                                run.text_start as usize + run.text_len as usize
                                    <= tab.text_pool.len()
                            );
                        }
                        for row in &tab.rows {
                            let end = row.parts_start as usize + row.parts_len as usize;
                            assert!(end <= tab.parts_pool.len());
                            for part in tab.row_parts(row) {
                                let ri = part.run as usize;
                                assert!(ri < tab.runs.len());
                                assert!(
                                    part.range_start as usize + part.range_len as usize
                                        <= tab.runs[ri].text_len as usize
                                );
                            }
                        }
                    }
                }
            }
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn links_are_deduplicated_and_capacities_shrink_after_large_docs() {
        unsafe {
            let hwnd = window(900, 560);
            let mut app = App::new();
            let big = "Line with [repeated](./target.md) and **bold**.\n".repeat(10_000);
            document(&mut app, &big);
            app.layout(hwnd);
            assert_eq!(
                app.tab().links.len(),
                1,
                "10,000 identical link URLs must deduplicate to 1 String"
            );
            assert!(
                app.tab().runs.len() < 10,
                "repeated link and text runs must intern cleanly"
            );
            // Switching back to a small doc must release the 10k-line Vec capacities.
            document(&mut app, "# Small\n\nTiny doc.\n");
            app.layout(hwnd);
            assert!(app.tab().blocks.capacity() < 128);
            assert!(app.tab().rows.capacity() < 128);
            assert!(app.tab().parts_pool.capacity() < 128);
            assert!(app.tab().text_pool.capacity() < 128);
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn unified_chrome_exposes_theme_pill_settings_button_and_themed_settings_dialog() {
        unsafe {
            let faces = modern_fonts();
            assert!(!faces.ui.is_empty() && !faces.display.is_empty() && !faces.code.is_empty());

            let hwnd = window(1024, 620);
            let state = RefCell::new(App::new());
            {
                let mut app = state.borrow_mut();
                app.build_menu(hwnd);
                document(&mut app, include_str!("../examples/showcase.md"));
                app.layout(hwnd);
                app.apply_theme(hwnd);
                let buf = Buffer::new(NULL, app.width, app.client_h()).unwrap();
                app.draw(
                    buf.dc,
                    Rect {
                        left: 0,
                        top: 0,
                        right: app.width,
                        bottom: app.client_h(),
                    },
                );
                let hits: Vec<TabHit> = app.tab_hot.borrow().iter().map(|(_, h)| *h).collect();
                assert!(hits.contains(&TabHit::ToggleOutline));
                assert!(hits.contains(&TabHit::NewTab));
                assert!(hits.contains(&TabHit::CycleTheme));
                assert!(hits.contains(&TabHit::OpenSettings));
                assert!(!app.bar_brush.is_null() && !app.menu_brush.is_null());
            }

            // Settings dialog paints themed background and rounded cards matching active palette.
            let dlg = settings_window(hwnd, &state);
            assert!(!dlg.is_null());
            let mut rect = Rect::default();
            GetClientRect(dlg, &mut rect);
            let surface = Buffer::new(NULL, rect.right, rect.bottom).unwrap();
            SendMessageW(dlg, WM_ERASEBKGND, surface.dc as usize, 0);
            let bg = crate::theme::rgb(state.borrow().palette().background);
            let surf = crate::theme::rgb(state.borrow().palette().surface());
            let pixels = surface.pixels();
            assert!(
                pixels.iter().any(|&p| p & 0xffffff == bg),
                "settings dialog background must use palette.background"
            );
            assert!(
                pixels.iter().any(|&p| p & 0xffffff == surf),
                "settings dialog cards must paint with palette.surface()"
            );
            DestroyWindow(dlg);
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn crash_and_abuse_torture_emoji_multilingual_selection_tabs_and_outline() {
        unsafe {
            let hwnd = window(900, 560);
            let mut app = App::new();
            app.smooth = false;
            app.outline_open = true;

            let zalgo = format!("Z{}o", "\u{0301}\u{0308}\u{0323}".repeat(10));
            let mut torture = String::new();
            torture.push_str("# 👨‍👩‍👧‍👦 Emoji & 🌍 Multilingual Torture\n\n");
            torture.push_str("## !!! ??? ---\n\n");
            torture.push_str(&format!("### {}\n\n", "LongHeadingWord ".repeat(40)));
            torture.push_str("Flags: 🇲🇦 🇯🇵 🇺🇸 🏴󠁧󠁢󠁳󠁣󠁴󠁿 Keycaps: 0️⃣1️⃣#️⃣*️⃣ Modifiers: 👍🏻👍🏿 VS15/16: ❤︎ ❤️ Orphan: \u{200d}\u{fe0f}\n\n");
            torture.push_str("العَرَبِيَّةُ **نص عربي** [رابط](./other.md) English 123 😀\n\n");
            torture.push_str("שָׁלוֹם **עולם** — हिन्दी क्षि त्रि — বাংলা — தமிழ் — ภาษาไทย — ខ្មែរ — မြန်မာ — 日本語 𠀀 — 한국어 — Tiếng Việt\n\n");
            torture.push_str(&format!(
                "Bidi + Zalgo: \u{202e}rtl\u{202c} \u{2066}iso\u{2069} {zalgo}\n\n"
            ));
            torture.push_str("| Script | Emoji | Sample |\n|:---|---:|:---:|\n| Arabic | 👨‍👩‍👧‍👦 | **العربية** |\n| CJK | 🇯🇵 | `日本語` |\n\n");
            for i in 0..300 {
                let level = "#".repeat((i % 6) + 1);
                torture.push_str(&format!("{level} Heading {i} 😀 العربية 日本語\n\n"));
            }

            document(&mut app, &torture);

            // Layout + paint torture across extreme sizes, DPIs, zooms, and outline toggles.
            for &(w, h, dpi, zoom, outline) in &[
                (900, 560, 96u32, 100, true),
                (180, 140, 96, 150, true),
                (16, 16, 96, 400, true),
                (1, 1, 96, 50, false),
                (1200, 700, 144, 125, true),
                (900, 560, 96, 100, true),
            ] {
                SetWindowPos(hwnd, NULL, 0, 0, w, h, 0x14);
                app.outline_open = outline;
                app.set_dpi(dpi);
                app.zoom = zoom;
                app.rescale();
                app.layout(hwnd);
                if app.width > 0 && app.client_h() > 0 {
                    if let Some(buf) = Buffer::new(NULL, app.width, app.client_h()) {
                        let rect = Rect {
                            left: 0,
                            top: 0,
                            right: app.width,
                            bottom: app.client_h(),
                        };
                        app.draw(buf.dc, rect);
                        app.scroll_to(hwnd, i32::MAX);
                        app.draw(buf.dc, rect);
                        app.scroll_to(hwnd, 0);
                    }
                }
            }

            // Selection & copy abuse: out-of-bounds, inverted, extreme coordinates, word select.
            let n_rows = app.tab().rows.len();
            assert!(n_rows > 300);
            for bad_sel in [
                (
                    SelPos {
                        row: usize::MAX,
                        col: usize::MAX,
                    },
                    SelPos { row: 0, col: 0 },
                ),
                (
                    SelPos {
                        row: n_rows + 50,
                        col: 9999,
                    },
                    SelPos {
                        row: n_rows + 100,
                        col: 9999,
                    },
                ),
                (SelPos { row: 10, col: 9999 }, SelPos { row: 2, col: 9999 }),
            ] {
                app.tab_mut().selection = Some(bad_sel);
                let _ = app.tab().selected_plain();
                let _ = app.tab().selected_markdown();
            }
            assert_eq!(app.tab().row_plain_slice(0, 999, 5), "");
            assert_eq!(app.tab().row_markdown_slice(0, 999, 5), "");
            let _ = app.tab().row_plain_slice(0, 0, usize::MAX);
            let _ = app.tab().row_markdown_slice(0, 0, usize::MAX);

            for &(cx, cy) in &[(-9999, -9999), (0, 0), (450, 120), (99999, 99999)] {
                let _ = app.hit_doc(hwnd, cx, cy);
            }
            app.select_word_at(SelPos {
                row: usize::MAX,
                col: usize::MAX,
            });
            for r in [0, 1, 5, 10, n_rows - 1] {
                app.select_word_at(SelPos { row: r, col: 0 });
                let _ = app.tab().selected_plain();
            }

            // Select all across emoji + RTL + tables + CJK must never panic and strips Markdown.
            app.select_all();
            let all_plain = app.tab().selected_plain();
            let all_md = app.tab().selected_markdown();
            assert!(
                all_plain.contains("👨‍👩‍👧‍👦")
                    && all_plain.contains("العَرَبِيَّةُ")
                    && all_plain.contains("日本語")
            );
            assert!(!all_plain.contains("**نص عربي**"));
            assert!(all_md.contains("**نص عربي**") && all_md.contains("[رابط](./other.md)"));

            // Out-of-bounds tab switches & closes must be safe no-ops.
            app.switch_tab(hwnd, 999);
            app.close_tab(hwnd, 999);
            assert_eq!(app.tabs.len(), 1);
            assert_eq!(app.active, 0);

            // Empty document abuse: select_all, hit_doc, word select, outline all stay safe.
            document(&mut app, "");
            app.layout(hwnd);
            app.select_all();
            assert_eq!(app.tab().selected_plain(), "");
            assert_eq!(app.tab().selected_markdown(), "");
            assert_eq!(app.hit_doc(hwnd, 200, 200), (None, None));
            app.select_word_at(SelPos { row: 0, col: 0 });

            // Unicode filenames + rapid tab open/close stress + unsafe link rejection.
            let dir = std::env::temp_dir().join(format!("mdlite-abuse-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let state = RefCell::new(app);
            let names = [
                "日本語 문서 😀.md",
                "العربية ملف.markdown",
                "spaces & (parens).mdown",
            ];
            for (idx, name) in names.iter().enumerate() {
                let p = dir.join(name);
                std::fs::write(
                    &p,
                    format!("# {name}\n\n[next](./{})\n", names[(idx + 1) % names.len()]),
                )
                .unwrap();
                assert!(load_file_tab(hwnd, &state, p, idx > 0, None));
            }
            assert_eq!(state.borrow().tabs.len(), 3);
            for blocked in [
                "javascript:alert(1)",
                "data:text/html,hi",
                "vbscript:x",
                "calc.exe",
            ] {
                activate_link(hwnd, &state, blocked);
            }
            assert_eq!(state.borrow().tabs.len(), 3);
            while state.borrow().tabs.len() > 1 {
                state.borrow_mut().close_tab(hwnd, 0);
            }
            DestroyWindow(hwnd);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
