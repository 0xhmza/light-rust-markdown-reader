//! The window: block layout, painting through three pipelines, and settings.
use crate::{
    dwrite::{self, RichText},
    markdown::{self, Kind, Span, Style},
    render::{self, Canvas, Pipe},
    theme::{Palette, Themes},
    win32::*,
};
use std::time::Instant;
use std::{
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
const FLOAT: usize = 324;
const ADD: usize = 323;
/// The animation timer, and the tick it asks Windows for.
const GLIDE: usize = 1;
const FRAME: u32 = 8;
/// One owner-drawn swatch per palette role.
const SWATCH: usize = 330;

struct Font {
    handle: Handle,
    key: (u8, u8),
    height: i32,
    ascent: i32,
    size: i32,
    /// The GDI+ twin of `handle`, built on first GDI+ use. `Some(NULL)` records
    /// a font GDI+ cannot represent, so the attempt is made only once.
    plus: Cell<Option<Handle>>,
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
struct Run {
    text: Box<[u16]>,
    style: Style,
    width: Cell<Option<i32>>,
    /// One colour cluster, shaped and drawn by DirectWrite inside whatever
    /// pipeline draws the rest of the line.
    is_emoji: bool,
    emoji: Option<dwrite::Layout>,
}
/// One flowable unit of text: a whole block, or a single table cell.
#[derive(Clone)]
struct Piece {
    runs: Range<usize>,
    shaped: Option<Rc<RichText>>,
    pipe: Pipe,
    align: u8,
}
struct Block {
    kind: Kind,
    body: Piece,
    cells: Vec<Piece>,
}
/// The parts of the footer that answer a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    ZoomOut,
    ZoomReset,
    ZoomIn,
    Engine,
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
    run: usize,
    range: Range<usize>,
    x: i32,
    width: i32,
    font: usize,
}
struct Row {
    y: i32,
    height: i32,
    ascent: i32,
    deco: Deco,
    pipe: Pipe,
    /// Horizontal extent of this row's decoration.
    span: (i32, i32),
    parts: Vec<Fragment>,
    shaped: Option<(Rc<RichText>, i32, i32, i32, bool)>, // text, x, width, pixels, bold
}
/// Where a piece may flow and how it is decorated.
#[derive(Clone, Copy)]
struct Frame {
    left: i32,
    right: i32,
    span: (i32, i32),
    heading: u8,
    deco: Deco,
    align: u8,
}
/// What the status bar reports about the open document.
#[derive(Default)]
struct Stats {
    /// Text size in use, shown in the footer when it is not the normal 100.
    zoom: i32,
    bytes: usize,
    lines: usize,
    words: usize,
    micros: u128,
    /// Blocks of text counted by what their content needs, so the pipeline
    /// each one would take under any setting can be worked out without
    /// reparsing the document.
    blocks: [usize; 8],
    /// Why any of them escalated past GDI, over the whole document.
    reasons: u8,
    /// Blocks that read right to left, for deciding which way the page runs.
    rtl: usize,
    /// True when the document was too long to hold and was cut short.
    truncated: bool,
}
impl Stats {
    /// What this document would cost under `pinned`, for the settings window.
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
    /// True when the document as a whole reads right to left. One Arabic word
    /// in a table does not turn an English page around.
    fn rtl(&self) -> bool {
        self.rtl * 2 > self.blocks.iter().sum::<usize>()
    }
    /// How many blocks each pipeline would draw under `pinned`.
    fn spread(&self, pinned: Option<Pipe>) -> [usize; 3] {
        let mut counts = [0; 3];
        for (reasons, blocks) in self.blocks.iter().enumerate() {
            counts[Pipe::choose(reasons as u8, pinned) as usize] += blocks;
        }
        counts
    }
    /// Blocks the pinned pipeline could not draw, and had to give to Direct2D.
    fn escalated(&self, pinned: Option<Pipe>) -> usize {
        match pinned {
            Some(pinned) if pinned != Pipe::D2d => self.spread(Some(pinned))[Pipe::D2d as usize],
            _ => 0,
        }
    }
    /// The clickable name in the footer: what is drawing this document, said
    /// in terms of the choice that was made rather than the busiest pipeline.
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
    /// What the pinned pipeline costs this document, if anything. The footer
    /// says it plainly rather than leaving the reader to wonder.
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
    /// The text behind the clickable engine label.
    fn explain(&self, pinned: Option<Pipe>) -> String {
        let spread = self.spread(pinned);
        let mut text = match pinned {
            None => "Chosen automatically: every block gets the cheapest pipeline\nthat can draw it correctly.\n\n"
                .to_owned(),
            Some(pinned) => format!(
                "Pinned to {} in Settings.\n\n",
                pinned.short()
            ),
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
    /// Set while the viewer is putting the system scrollbar away. Windows
    /// sends WM_SIZE from inside that call, and mistaking it for the reader
    /// resizing the window schedules a second full layout of the document.
    static ADJUSTING: Cell<bool> = const { Cell::new(false) };
    /// Windows paints the menu from inside `SetMenu` and `DrawMenuBar`, which
    /// the viewer calls with its state already borrowed. Menu painting
    /// therefore reads its font, palette and DPI from here instead.
    static MENU: Cell<(Handle, Palette, u32)> = Cell::new((NULL, Palette::dark(), 96));
}

/// Paint one owner-drawn menu item, so the menu matches the document.
unsafe fn draw_menu_item(item: &DrawItem) {
    let (font, palette, dpi) = MENU.get();
    let px = |n: i32| ((n as i64 * dpi as i64) / 96) as i32;
    let selected = item.state & 1 != 0; // ODS_SELECTED.
    fill(
        item.dc,
        item.rect,
        if selected {
            palette.accent
        } else {
            palette.background
        },
    );
    let text = &*(item.data as *const Box<[u16]>);
    if text.is_empty() {
        // A separator: one rule across the middle of its row.
        let middle = (item.rect.top + item.rect.bottom) / 2;
        return fill(
            item.dc,
            Rect {
                left: item.rect.left + px(8),
                right: item.rect.right - px(8),
                top: middle,
                bottom: middle + px(1).max(1),
            },
            palette.rule,
        );
    }
    let old = SelectObject(item.dc, font);
    SetBkMode(item.dc, 1);
    SetTextColor(
        item.dc,
        if selected {
            Palette::contrasting(palette.accent)
        } else {
            palette.foreground
        },
    );
    let mut rect = Rect {
        left: item.rect.left + px(10),
        right: item.rect.right - px(10),
        ..item.rect
    };
    // DT_SINGLELINE | DT_VCENTER for the label, then the accelerator right-aligned.
    let tab = text.iter().position(|&c| c == 9).unwrap_or(text.len());
    DrawTextW(item.dc, text.as_ptr(), tab as i32, &mut rect, 0x20 | 0x4);
    if tab + 1 < text.len() {
        SetTextColor(item.dc, palette.muted);
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
        (item.width, item.height) = (0, px(7) as u32);
    } else {
        // Menu-bar items get only their margins; an item with an accelerator
        // also needs the column that holds it.
        let gap = if text.contains(&9) { px(32) } else { 0 };
        (item.width, item.height) = ((size.cx + px(20) + gap) as u32, (size.cy + px(10)) as u32);
    }
}

/// Bounded interning shared by every piece of one document.
#[derive(Default)]
struct Intern {
    runs: HashMap<u16, HashMap<String, usize>>,
    entries: usize,
    rich: HashMap<Rc<RichText>, Rc<RichText>>,
}

struct App {
    runs: Vec<Run>,
    run_order: Vec<usize>,
    blocks: Vec<Block>,
    rows: Vec<Row>,
    fonts: Vec<Font>,
    themes: Themes,
    /// Index into `themes.palettes`.
    theme: usize,
    /// Menu background brush, rebuilt whenever the palette changes.
    menu_brush: Handle,
    menu_font: Handle,
    /// Owner-drawn menu items carry their text by pointer, so it must outlive
    /// the menu. Rebuilding the menu frees the previous set.
    menu_labels: Vec<*mut Box<[u16]>>,
    /// Palettes as they were when the settings window opened, for Cancel.
    restore: Option<(Themes, usize)>,
    /// Pipeline the user pinned in settings; `None` chooses one per block.
    pinned: Option<Pipe>,
    restyle: bool,
    stats: Stats,
    /// Clickable areas of the footer, measured while it is painted.
    hot: RefCell<Vec<(Rect, Hit)>>,
    /// The zoom control floating over the document, when the reader wants one.
    floater: Hwnd,
    floating: bool,
    theme_path: PathBuf,
    file: Option<PathBuf>,
    source: String,
    dpi: u32,
    /// Text size as a percentage, on top of the monitor's DPI.
    zoom: i32,
    width: i32,
    height: i32,
    content_height: i32,
    tallest: i32,
    scroll: i32,
    wheel_remainder: i32,
    /// Grab offset inside the scrollbar thumb while dragging it.
    grab: Option<i32>,
    hover: bool,
    buffer: Option<Buffer>,
    /// Where a smooth scroll is heading; equal to `scroll` when at rest.
    target: i32,
    smooth: bool,
    menu: Handle,
    layout_pending: bool,
    engine: RefCell<Option<dwrite::Engine>>,
    engine_failed: bool,
}

impl App {
    fn new() -> Self {
        let theme_path = std::env::current_exe()
            .unwrap_or_default()
            .with_file_name("mdlite.ini");
        let themes = Themes::load(&theme_path);
        Self {
            theme: themes.start,
            pinned: pinned(&themes),
            smooth: themes.smooth,
            zoom: themes.zoom,
            floating: themes.floating,
            themes,
            menu_brush: NULL,
            menu_font: NULL,
            menu_labels: vec![],
            restore: None,
            theme_path,
            restyle: false,
            stats: Stats::default(),
            hot: RefCell::new(vec![]),
            floater: NULL,
            file: None,
            source: String::new(),
            runs: vec![],
            run_order: vec![],
            blocks: vec![],
            rows: vec![],
            fonts: vec![],
            dpi: 96,
            width: 0,
            height: 0,
            content_height: 0,
            tallest: 0,
            scroll: 0,
            wheel_remainder: 0,
            grab: None,
            hover: false,
            buffer: None,
            target: 0,
            menu: NULL,
            layout_pending: false,
            engine: RefCell::new(None),
            engine_failed: false,
        }
    }
    fn px(&self, n: i32) -> i32 {
        ((n as i64 * self.dpi as i64 * self.zoom as i64) / (96 * 100)) as i32
    }
    /// Height of the footer. `self.height` is the text viewport above it.
    fn status(&self) -> i32 {
        self.px(26)
    }
    /// The scrollbar is drawn by the viewer, not by Windows, so that it takes
    /// the palette's own colours. Width, then the x it starts at.
    fn bar(&self) -> (i32, i32) {
        let width = self.px(11).max(4);
        (width, self.width - width)
    }
    /// Top and height of the thumb, or `None` when everything already fits.
    fn thumb(&self) -> Option<(i32, i32)> {
        let span = self.content_height - self.height;
        if span <= 0 || self.height <= 0 {
            return None;
        }
        let height = (self.height as i64 * self.height as i64 / self.content_height.max(1) as i64)
            .max(self.px(28) as i64) as i32;
        let travel = (self.height - height).max(0);
        Some((
            (travel as i64 * self.scroll as i64 / span as i64) as i32,
            height,
        ))
    }
    /// Repaint the scrollbar. Only the thumb ever changes, so when its old
    /// place is known the update region stays that small: a scroll otherwise
    /// invalidates a full-height strip, and the bounding box of that and the
    /// newly exposed text covers the whole window.
    fn bar_update(&self, previous: Option<(i32, i32)>) -> Rect {
        let (width, x) = self.bar();
        let mut rect = Rect {
            left: x,
            top: 0,
            right: x + width,
            bottom: self.height,
        };
        if let (Some((top, height)), Some((was, was_height))) = (self.thumb(), previous) {
            rect.top = top.min(was);
            rect.bottom = (top + height).max(was + was_height);
        }
        rect
    }
    unsafe fn invalidate_bar(&self, hwnd: Hwnd, previous: Option<(i32, i32)>) {
        InvalidateRect(hwnd, &self.bar_update(previous), 0);
    }
    /// Route a click or drag on the scrollbar. Returns the scroll it asks for.
    fn bar_target(&self, y: i32) -> i32 {
        let Some((top, height)) = self.thumb() else {
            return self.scroll;
        };
        let span = self.content_height - self.height;
        match self.grab {
            Some(grab) => {
                let travel = (self.height - height).max(1);
                ((y - grab).clamp(0, travel) as i64 * span as i64 / travel as i64) as i32
            }
            // A click on the track pages towards it, as a scrollbar does.
            None if y < top => self.scroll - (self.height - self.px(24)).max(1),
            None => self.scroll + (self.height - self.px(24)).max(1),
        }
    }
    fn palette(&self) -> Palette {
        self.themes.palettes[self.theme.min(self.themes.palettes.len() - 1)].1
    }
    fn set_document(&mut self, source: &str) {
        self.rows.clear();
        self.runs.clear();
        self.run_order.clear();
        self.blocks.clear();
        if let Some(engine) = self.engine.get_mut() {
            engine.clear_paragraphs();
        }
        let started = Instant::now();
        self.stats = Stats {
            zoom: self.zoom,
            bytes: source.len(),
            lines: source.lines().count(),
            words: source.split_whitespace().count(),
            ..Stats::default()
        };
        let blocks = markdown::parse(source);
        // The parser stops at its own limit; say so rather than pretend.
        self.stats.truncated = blocks.len() >= markdown::LONGEST_DOCUMENT;
        let mut intern = Intern::default();
        for block in blocks {
            let heading = match block.kind {
                Kind::Heading(level) => level,
                _ => 0,
            };
            let header = block.kind == Kind::Row(true);
            let body = self.piece(block.spans, heading, false, 0, &mut intern);
            let cells = block
                .cells
                .into_iter()
                .map(|cell| self.piece(cell.spans, 0, header, cell.align, &mut intern))
                .collect();
            self.blocks.push(Block {
                kind: block.kind,
                body,
                cells,
            });
        }
        self.stats.micros = started.elapsed().as_micros();
        self.scroll = 0;
        self.wheel_remainder = 0;
    }
    /// Turn spans into a piece, choosing its pipeline and sharing storage and
    /// measurements with identical text elsewhere in the document.
    fn piece(
        &mut self,
        mut spans: Vec<Span>,
        heading: u8,
        bold: bool,
        align: u8,
        intern: &mut Intern,
    ) -> Piece {
        if bold {
            for span in &mut spans {
                span.style.0 |= Style::BOLD;
            }
        }
        let reasons = spans
            .iter()
            .fold(0, |flags, span| flags | Pipe::reasons(&span.text));
        let pipe = Pipe::choose(reasons, self.pinned);
        if !spans.is_empty() {
            self.stats.reasons |= reasons;
            self.stats.blocks[(reasons & 7) as usize] += 1;
        }
        let start = self.run_order.len();
        if pipe == Pipe::D2d {
            let text = Rc::new(RichText::new(&spans));
            if text.rtl() {
                self.stats.reasons |= render::RTL;
                self.stats.rtl += 1;
            }
            let text = if let Some(shared) = intern.rich.get(&text) {
                shared.clone()
            } else {
                if text.text.len() <= 1024 && intern.rich.len() < 4096 {
                    intern.rich.insert(text.clone(), text.clone());
                }
                text
            };
            return Piece {
                runs: start..start,
                shaped: Some(text),
                pipe,
                align,
            };
        }
        for span in spans {
            for (text, is_emoji) in dwrite::split(&span.text) {
                // Heading level selects a different font, so it must not share
                // a measurement with the same text at body size.
                let key = span.style.0 as u16 | ((heading as u16) << 8) | ((is_emoji as u16) << 12);
                let group = intern.runs.entry(key).or_default();
                let index = if let Some(&index) = group.get(text) {
                    index
                } else {
                    let index = self.runs.len();
                    self.runs.push(Run {
                        text: text.encode_utf16().collect(),
                        style: span.style,
                        width: Cell::new(None),
                        is_emoji,
                        emoji: None,
                    });
                    if text.len() <= 256 && intern.entries < 8192 {
                        group.insert(text.to_owned(), index);
                        intern.entries += 1;
                    }
                    index
                };
                self.run_order.push(index);
            }
        }
        Piece {
            runs: start..self.run_order.len(),
            shaped: None,
            pipe,
            align,
        }
    }
    fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi.clamp(96, 960);
        self.rescale();
    }
    /// Throw away everything that was measured at the old size.
    fn rescale(&mut self) {
        unsafe { DeleteObject(std::mem::replace(&mut self.menu_font, NULL)) };
        self.fonts.clear();
        for run in &mut self.runs {
            run.width.set(None);
            run.emoji = None;
        }
        *self.engine.get_mut() = None;
        self.engine_failed = false;
    }
    /// Resize the text. Everything the viewer measures goes through `px`, so
    /// the layout follows with no special cases.
    /// The size a zoom control asks for.
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
        // Keep whatever is on screen roughly where it is.
        let anchor = self.scroll as f64 / self.content_height.max(1) as f64;
        self.zoom = zoom;
        self.stats.zoom = zoom;
        self.rescale();
        self.layout(hwnd);
        let target = (anchor * self.content_height as f64) as i32;
        self.scroll_to(hwnd, target);
    }
    /// Reparse the current document, e.g. after the pipeline choice changed.
    unsafe fn rebuild(&mut self, hwnd: Hwnd) {
        let source = std::mem::take(&mut self.source);
        let scroll = self.scroll;
        self.set_document(&source);
        self.source = source;
        self.scroll = scroll;
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
        let size = match heading {
            1 => 30,
            2 => 25,
            3 => 21,
            4 => 19,
            _ => 17,
        };
        let face = wide(if code { "Consolas" } else { "Segoe UI" });
        let handle = CreateFontW(
            -self.px(if code { 16 } else { size }),
            0,
            0,
            0,
            if heading > 0 || style.0 & Style::BOLD != 0 {
                700
            } else {
                400
            },
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
            GetStockObject(17) // DEFAULT_GUI_FONT, rather than no font at all.
        } else {
            handle
        };
        let old = SelectObject(dc, handle);
        let mut metric = TextMetric::default();
        GetTextMetricsW(dc, &mut metric);
        SelectObject(dc, old);
        let index = self.fonts.len();
        self.fonts.push(Font {
            handle,
            key,
            height: metric.height.max(self.px(17)),
            ascent: metric.ascent.max(1),
            size: self.px(if code { 16 } else { size }),
            plus: Cell::new(None),
        });
        index
    }
    fn blank(&self, y: i32, base: usize, frame: &Frame, pipe: Pipe) -> Row {
        Row {
            y,
            height: self.fonts[base].height + self.px(5).max(1),
            ascent: self.fonts[base].ascent,
            deco: frame.deco,
            pipe,
            span: frame.span,
            parts: vec![],
            shaped: None,
        }
    }
    unsafe fn layout(&mut self, hwnd: Hwnd) {
        self.layout_pending = false;
        let mut client = Rect::default();
        GetClientRect(hwnd, &mut client);
        self.width = client.right;
        self.height = client.bottom - self.status();
        if self.width <= 0 || self.height <= 0 {
            return;
        }
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return;
        }
        let normal = self.font(dc, Style::default(), 0);
        let old = SelectObject(dc, self.fonts[normal].handle);
        self.rows.clear();
        let padding = self.px(24);
        let edge = (self.width - padding).max(padding + 1);
        let mut y = padding;
        let mut bi = 0;
        while bi < self.blocks.len() {
            if matches!(self.blocks[bi].kind, Kind::Row(_)) {
                (bi, y) = self.table(dc, bi, padding, edge, y);
                continue;
            }
            let kind = &self.blocks[bi].kind;
            if *kind == Kind::Gap {
                y = y.saturating_add(self.px(10));
                bi += 1;
                continue;
            }
            if *kind == Kind::Rule {
                let frame = Frame {
                    left: padding,
                    right: edge,
                    span: (padding, edge),
                    heading: 0,
                    deco: Deco::Rule,
                    align: 0,
                };
                let mut row = self.blank(y, normal, &frame, Pipe::Gdi);
                row.height = self.px(17);
                row.ascent = 0;
                self.rows.push(row);
                y = y.saturating_add(self.px(21));
                bi += 1;
                continue;
            }
            let heading = match kind {
                Kind::Heading(level) => *level,
                _ => 0,
            };
            let deco = match kind {
                Kind::Code => Deco::Code,
                Kind::Quote => Deco::Quote,
                _ => Deco::None,
            };
            let indent = match kind {
                Kind::List(depth) => self.px(12 + (*depth as i32 * 18)),
                Kind::Code | Kind::Quote => self.px(14),
                _ => 0,
            };
            let left = padding + indent.min((self.width / 3).max(0));
            let right = (edge - if deco == Deco::Code { self.px(10) } else { 0 }).max(left + 1);
            if heading > 0 && y > padding {
                y = y.saturating_add(self.px(9));
            }
            let piece = self.blocks[bi].body.clone();
            let frame = Frame {
                left,
                right,
                span: (padding, edge),
                heading,
                deco,
                align: 0,
            };
            y = self.flow(dc, piece, frame, y);
            if heading > 0 {
                y = y.saturating_add(self.px(5));
            }
            bi += 1;
        }
        self.content_height = y.saturating_add(padding);
        // Only table cells emit rows side by side; without one the rows are
        // already in order, and a large document need not be sorted at all.
        if self.blocks.iter().any(|b| matches!(b.kind, Kind::Row(_))) {
            self.rows.sort_by_key(|row| row.y);
        }
        self.tallest = self.rows.iter().map(|row| row.height).max().unwrap_or(0);
        self.scroll = self
            .scroll
            .clamp(0, (self.content_height - self.height).max(0));
        SelectObject(dc, old);
        ReleaseDC(hwnd, dc);
        self.update_scroll(hwnd);
        self.place_floater();
        InvalidateRect(hwnd, ptr::null(), 0);
    }

    /// Lay out the consecutive table rows starting at `first`, returning the
    /// block after the table and the y below it.
    unsafe fn table(
        &mut self,
        dc: Hdc,
        first: usize,
        left: i32,
        right: i32,
        mut y: i32,
    ) -> (usize, i32) {
        let last = (first..self.blocks.len())
            .find(|&i| !matches!(self.blocks[i].kind, Kind::Row(_)))
            .unwrap_or(self.blocks.len());
        let columns = self.blocks[first..last]
            .iter()
            .map(|block| block.cells.len())
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return (last, y);
        }
        let (pad, gap) = (self.px(8), self.px(4));
        // Columns are as wide as their widest cell, then shrink in proportion
        // when the table does not fit. Text still wraps inside its column.
        let mut widths = vec![0; columns];
        for bi in first..last {
            for ci in 0..self.blocks[bi].cells.len() {
                let cell = self.blocks[bi].cells[ci].clone();
                widths[ci] = widths[ci].max(self.measure(dc, cell));
            }
        }
        let room = (right - left - columns as i32 * 2 * pad).max(columns as i32 * self.px(16));
        let natural: i32 = widths.iter().sum();
        if natural > room {
            for width in &mut widths {
                *width = (*width as i64 * room as i64 / natural.max(1) as i64) as i32;
            }
        }
        y = y.saturating_add(gap);
        for bi in first..last {
            let header = self.blocks[bi].kind == Kind::Row(true);
            let (mut x, mut bottom) = (left, y);
            for ci in 0..columns {
                let end = x + widths[ci] + 2 * pad;
                if let Some(cell) = self.blocks[bi].cells.get(ci).cloned() {
                    let frame = Frame {
                        left: x + pad,
                        right: (end - pad).max(x + pad + 1),
                        span: (x, end),
                        heading: 0,
                        deco: Deco::None,
                        align: cell.align,
                    };
                    bottom = bottom.max(self.flow(dc, cell, frame, y));
                }
                x = end;
            }
            y = bottom.saturating_add(gap);
            let thickness = if header { self.px(2) } else { self.px(1) }.max(1);
            let frame = Frame {
                left,
                right: x,
                span: (left, x),
                heading: 0,
                deco: Deco::Grid,
                align: 0,
            };
            let mut rule = self.blank(y, 0, &frame, Pipe::Gdi);
            rule.height = thickness;
            rule.ascent = 0;
            self.rows.push(rule);
            y = y.saturating_add(thickness + gap);
        }
        (last, y)
    }

    /// The width a piece wants when nothing forces it to wrap.
    unsafe fn measure(&mut self, dc: Hdc, piece: Piece) -> i32 {
        let base = self.font(dc, Style::default(), 0);
        if let Some(text) = &piece.shaped {
            let pixels = self.fonts[base].size;
            return self
                .engine()
                .and_then(|engine| engine.paragraph(text, pixels, 1 << 20, false).ok())
                .map_or(0, |layout| layout.natural);
        }
        let mut total = 0;
        for order in piece.runs.clone() {
            let ri = self.run_order[order];
            if let Some(width) = self.runs[ri].width.get() {
                total += width;
                continue;
            }
            let font = self.font(dc, self.runs[ri].style, 0);
            SelectObject(dc, self.fonts[font].handle);
            let count = self.runs[ri].text.len().min(4096);
            let mut size = Size::default();
            GetTextExtentPoint32W(dc, self.runs[ri].text.as_ptr(), count as i32, &mut size);
            if count == self.runs[ri].text.len() {
                self.runs[ri].width.set(Some(size.cx));
            }
            total += size.cx;
        }
        total
    }

    /// Break one piece into rows inside `frame`, returning the y below it.
    unsafe fn flow(&mut self, dc: Hdc, piece: Piece, frame: Frame, mut y: i32) -> i32 {
        let leading = self.px(5).max(1);
        let code = frame.deco == Deco::Code;
        let base = self.font(dc, Style(if code { Style::CODE } else { 0 }), frame.heading);
        let width = (frame.right - frame.left).max(1);
        let start_row = self.rows.len();
        if let Some(text) = piece.shaped {
            let bold = frame.heading > 0;
            let pixels = self.fonts[base].size;
            let layout = self
                .engine()
                .and_then(|engine| engine.paragraph(&text, pixels, width, bold).ok());
            // Retain a readable GDI fallback if system text services fail.
            let height = layout
                .as_ref()
                .map_or(self.fonts[base].height, |layout| layout.height)
                + leading;
            let natural = layout.as_ref().map_or(width, |layout| layout.natural);
            let mut row = self.blank(y, base, &frame, Pipe::D2d);
            row.height = height;
            row.shaped = Some((
                text,
                frame.left + offset(frame.align, width, natural.min(width)),
                width,
                pixels,
                bold,
            ));
            self.rows.push(row);
            return y.saturating_add(height);
        }
        let mut row = self.blank(y, base, &frame, piece.pipe);
        let mut x = frame.left;
        for order in piece.runs.clone() {
            let ri = self.run_order[order];
            let font = self.font(dc, self.runs[ri].style, frame.heading);
            if self.runs[ri].is_emoji && self.runs[ri].emoji.is_none() && !self.engine_failed {
                let pixels = self.fonts[font].size;
                let engine = self.engine.get_mut();
                if engine.is_none() {
                    *engine = dwrite::Engine::new().ok();
                }
                match engine {
                    Some(engine) => {
                        self.runs[ri].emoji = engine.cluster(&self.runs[ri].text, pixels).ok()
                    }
                    None => self.engine_failed = true,
                }
            }
            if let Some(emoji) = &self.runs[ri].emoji {
                if x + emoji.width > frame.right && !row.parts.is_empty() {
                    y = y.saturating_add(row.height);
                    self.rows.push(row);
                    row = self.blank(y, base, &frame, piece.pipe);
                    x = frame.left;
                }
                // A colour cluster can be taller than the text it sits in.
                let descent = (row.height - leading - row.ascent).max(emoji.height - emoji.ascent);
                row.ascent = row.ascent.max(emoji.ascent);
                row.height = row.ascent + descent + leading;
                row.parts.push(Fragment {
                    run: ri,
                    range: 0..self.runs[ri].text.len(),
                    x,
                    width: emoji.width,
                    font,
                });
                x += emoji.width;
                continue;
            }
            SelectObject(dc, self.fonts[font].handle);
            let text = &self.runs[ri].text;
            let mut pos = 0;
            while pos < text.len() {
                // Bound each GDI measurement; never pass enormous lines to GDI.
                let mut count = (text.len() - pos).min(4096);
                if count < text.len() - pos && is_high_surrogate(text[pos + count - 1]) {
                    count -= 1;
                }
                let mut fit = 0;
                let mut size = Size::default();
                let cached = if pos == 0 && count == text.len() {
                    self.runs[ri].width.get()
                } else {
                    None
                };
                if let Some(width) = cached.filter(|&width| width <= (frame.right - x).max(0)) {
                    fit = count as i32;
                    size.cx = width;
                } else {
                    GetTextExtentExPointW(
                        dc,
                        text[pos..].as_ptr(),
                        count as i32,
                        (frame.right - x).max(0),
                        &mut fit,
                        ptr::null_mut(),
                        &mut size,
                    );
                    if pos == 0 && count == text.len() {
                        self.runs[ri].width.set(Some(size.cx));
                    }
                }
                let mut take = (fit.max(0) as usize).min(count);
                if take > 0 && take < text.len() - pos && is_high_surrogate(text[pos + take - 1]) {
                    take -= 1;
                }
                if take > 0
                    && take < count
                    && !code
                    && !row.parts.is_empty()
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
                        // Move a word intact to the next row before resorting
                        // to character wrapping for a truly overlong word.
                        take = 0;
                    }
                }
                if take == 0 && !row.parts.is_empty() {
                    y = y.saturating_add(row.height);
                    self.rows.push(row);
                    row = self.blank(y, base, &frame, piece.pipe);
                    x = frame.left;
                    if !code {
                        while pos < text.len() && (text[pos] == 32 || text[pos] == 9) {
                            pos += 1;
                        }
                    }
                    continue;
                }
                if take == 0 {
                    take = if is_high_surrogate(text[pos]) && pos + 1 < text.len() {
                        2
                    } else {
                        1
                    };
                }
                let wraps = take < count;
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
                // The extent call already measured the whole chunk. Only
                // remeasure when wrapping selected a shorter substring.
                if take != count {
                    GetTextExtentPoint32W(dc, text[pos..].as_ptr(), take as i32, &mut size);
                }
                row.ascent = row.ascent.max(self.fonts[font].ascent);
                row.height = row.height.max(self.fonts[font].height + leading);
                row.parts.push(Fragment {
                    run: ri,
                    range: pos..pos + take,
                    x,
                    width: size.cx,
                    font,
                });
                x += size.cx;
                pos += take;
                if wraps || x >= frame.right {
                    y = y.saturating_add(row.height);
                    self.rows.push(row);
                    row = self.blank(y, base, &frame, piece.pipe);
                    x = frame.left;
                    if !code {
                        while pos < text.len() && (text[pos] == 32 || text[pos] == 9) {
                            pos += 1;
                        }
                    }
                }
            }
        }
        if !row.parts.is_empty()
            || piece
                .runs
                .clone()
                .all(|i| self.runs[self.run_order[i]].text.is_empty())
        {
            y = y.saturating_add(row.height);
            self.rows.push(row);
        }
        if frame.align != 0 {
            for row in &mut self.rows[start_row..] {
                let used = row.parts.last().map_or(0, |part| part.x + part.width) - frame.left;
                let shift = offset(frame.align, width, used);
                for part in &mut row.parts {
                    part.x += shift;
                }
            }
        }
        y
    }
    unsafe fn update_scroll(&self, hwnd: Hwnd) {
        // The viewer draws its own scrollbar, but Windows keeps the numbers:
        // they stay the window's published scroll state. Never redraw, or the
        // hidden system bar comes back.
        let mut info = ScrollInfo::new(1 | 2 | 4);
        info.max = (self.content_height - 1).max(0);
        info.page = self.height.max(0) as u32;
        info.pos = self.scroll;
        // Setting a range is what makes Windows add WS_VSCROLL and show a bar
        // of its own, so put it away again here. A position-only update, which
        // is what scrolling sends, never brings it back.
        SetScrollInfo(hwnd, 1, &info, 0);
        ADJUSTING.set(true);
        ShowScrollBar(hwnd, 1, 0);
        ADJUSTING.set(false);
    }
    /// Move the view now. Callers that animate go through `glide_to`.
    unsafe fn scroll_to(&mut self, hwnd: Hwnd, target: i32) {
        self.shift(hwnd, target);
        self.target = self.scroll;
    }
    /// Move the view over the next few frames, easing towards `target`. A
    /// second request while one is running simply moves the destination.
    unsafe fn glide_to(&mut self, hwnd: Hwnd, target: i32) {
        let target = target.clamp(0, (self.content_height - self.height).max(0));
        if !self.smooth || target == self.scroll {
            return self.scroll_to(hwnd, target);
        }
        if self.target == self.scroll {
            frame_clock(true);
            SetTimer(hwnd, GLIDE, FRAME, ptr::null());
        }
        self.target = target;
    }
    /// One animation frame: close a quarter of the remaining distance, which
    /// settles in about a fifth of a second however far the jump was.
    unsafe fn glide(&mut self, hwnd: Hwnd) {
        self.target = self
            .target
            .clamp(0, (self.content_height - self.height).max(0));
        let remaining = self.target - self.scroll;
        let step = if remaining.abs() < 8 {
            remaining
        } else {
            remaining / 4
        };
        self.shift(hwnd, self.scroll + step);
        if self.scroll == self.target || step == 0 {
            self.target = self.scroll;
            KillTimer(hwnd, GLIDE);
            frame_clock(false);
        }
    }
    unsafe fn shift(&mut self, hwnd: Hwnd, target: i32) {
        let target = target.clamp(0, (self.content_height - self.height).max(0));
        if target == self.scroll {
            return;
        }
        let thumb = self.thumb();
        let delta = self.scroll - target;
        self.scroll = target;
        // Only the position moves here. Touching the range would ask Windows
        // to reconsider its own scrollbar on every animation frame.
        let mut info = ScrollInfo::new(4);
        info.pos = self.scroll;
        SetScrollInfo(hwnd, 1, &info, 0);
        // Copy existing pixels and invalidate only the newly exposed strip.
        if delta.abs() < self.height {
            let text = Rect {
                left: 0,
                top: 0,
                right: self.bar().1,
                bottom: self.height,
            };
            ScrollWindowEx(hwnd, 0, delta, &text, &text, NULL, ptr::null_mut(), 2);
            self.invalidate_bar(hwnd, thumb);
        } else {
            InvalidateRect(hwnd, ptr::null(), 0);
        }
    }
    unsafe fn paint(&mut self, hwnd: Hwnd) {
        let mut paint: PaintStruct = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut paint);
        let (width, height) = (self.width, self.height + self.status());
        if !self
            .buffer
            .as_ref()
            .is_some_and(|buffer| (buffer.width, buffer.height) == (width, height))
        {
            self.buffer = Buffer::new(dc, width.max(1), height.max(1));
        }
        match &self.buffer {
            // Compose off-screen, then show the frame in one blit.
            Some(buffer) => {
                let clip = paint.rect;
                self.draw(buffer.dc, clip);
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
        let palette = self.palette();
        fill(dc, clip, palette.background);
        SetBkMode(dc, 1);
        let mut previous = NULL;
        let mut selected = usize::MAX;
        let mut canvas = None;
        let top = self.scroll.saturating_add(clip.top);
        let bottom = self.scroll.saturating_add(clip.bottom.min(self.height));
        // Rows are sorted by y but table cells share one, so step back a row
        // height before the clip and skip what really falls above it.
        let first = self
            .rows
            .partition_point(|row| row.y.saturating_add(self.tallest) <= top);
        for row in self.rows[first..].iter().take_while(|row| row.y < bottom) {
            if row.y.saturating_add(row.height) <= top {
                continue;
            }
            let y = row.y - self.scroll;
            let (left, right) = row.span;
            // Each decoration is one band: how tall, how wide, what colour.
            let band = match row.deco {
                Deco::Code => Some((y, row.height, right, palette.code_background)),
                Deco::Grid => Some((y, row.height, right, palette.rule)),
                Deco::Quote => Some((y, row.height, left + self.px(3).max(1), palette.rule)),
                Deco::Rule => Some((y + row.height / 2, self.px(1).max(1), right, palette.rule)),
                Deco::None => None,
            };
            if let Some((top, height, right, color)) = band {
                fill(
                    dc,
                    Rect {
                        left,
                        top,
                        right,
                        bottom: top + height,
                    },
                    color,
                );
            }
            let foreground = match row.deco {
                Deco::Code => palette.code_foreground,
                Deco::Quote => palette.muted,
                _ => palette.foreground,
            };
            if let Some((text, x, width, pixels, bold)) = &row.shaped {
                let background = if row.deco == Deco::Code {
                    palette.code_background
                } else {
                    palette.background
                };
                let mut engine = self.engine.borrow_mut();
                if let Some(engine) = engine.as_mut() {
                    if let Ok(layout) = engine.paragraph(text, *pixels, *width, *bold) {
                        if engine.draw_paragraph(
                            &layout, dc, *x, y, clip, palette, foreground, background,
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
                    *x,
                    y,
                    text.text.as_ptr(),
                    text.text.len().min(4096) as i32,
                );
                SelectObject(dc, old);
                continue;
            }
            for part in &row.parts {
                let run = &self.runs[part.run];
                let font = &self.fonts[part.font];
                if selected != part.font {
                    let old = SelectObject(dc, font.handle);
                    if selected == usize::MAX {
                        previous = old;
                    }
                    selected = part.font;
                }
                let inline_code = run.style.0 & Style::CODE != 0;
                if inline_code && row.deco != Deco::Code {
                    fill(
                        dc,
                        Rect {
                            left: part.x,
                            top: y,
                            right: part.x + part.width,
                            bottom: y + row.height,
                        },
                        palette.code_background,
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
                let text = &run.text[part.range.clone()];
                if let Some(emoji) = &run.emoji {
                    if let Some(engine) = self.engine.borrow_mut().as_mut() {
                        if engine.draw_cluster(emoji, dc, part.x, y + row.ascent, color) {
                            continue;
                        }
                    }
                }
                if row.pipe == Pipe::GdiPlus {
                    let canvas = canvas.get_or_insert_with(|| Canvas::new(dc));
                    if let Some(canvas) = canvas {
                        // The GDI+ font mirrors the HFONT selected above, so
                        // fragment positions measured with GDI still hold.
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
                SetTextColor(dc, color);
                TextOutW(dc, part.x, baseline, text.as_ptr(), text.len() as i32);
            }
        }
        if selected != usize::MAX {
            SelectObject(dc, previous);
        }
        if clip.bottom > self.height {
            self.draw_status(dc);
        }
        if clip.right > self.bar().1 {
            self.draw_bar(dc);
        }
    }
    /// The scrollbar: a track only as loud as a rule, and a thumb that brightens
    /// while the pointer is on it.
    unsafe fn draw_bar(&self, dc: Hdc) {
        let Some((top, height)) = self.thumb() else {
            return;
        };
        let palette = self.palette();
        let (width, x) = self.bar();
        fill(
            dc,
            Rect {
                left: x,
                top: 0,
                right: x + width,
                bottom: self.height,
            },
            palette.code_background,
        );
        let inset = (width / 4).max(1);
        fill(
            dc,
            Rect {
                left: x + inset,
                top: top + inset,
                right: x + width - inset,
                bottom: top + height - inset,
            },
            if self.hover || self.grab.is_some() {
                palette.muted
            } else {
                palette.rule
            },
        );
    }
    /// The footer: what the document is, how long it took, and which pipeline
    /// drew it. The pipeline name is clickable and says why it was chosen.
    unsafe fn draw_status(&self, dc: Hdc) {
        let Some(font) = self.fonts.first() else {
            return;
        };
        let palette = self.palette();
        let bar = Rect {
            left: 0,
            top: self.height,
            right: self.width,
            bottom: self.height + self.status(),
        };
        fill(dc, bar, palette.code_background);
        fill(
            dc,
            Rect {
                bottom: bar.top + self.px(1).max(1),
                ..bar
            },
            palette.rule,
        );
        let old = SelectObject(dc, font.handle);
        let y = bar.top + ((self.status() - font.height) / 2).max(0);
        let padding = self.px(24);
        let mut hot = self.hot.borrow_mut();
        hot.clear();
        SetBkMode(dc, 1);

        // Zoom, at the near end: take a step out, back to normal, a step in.
        let mut x = padding;
        for (_, spot, glyph, _) in ZOOM {
            let text = wide(&glyph.replace('%', &format!("{}%", self.zoom)));
            let count = text.len() as i32 - 1;
            let mut size = Size::default();
            GetTextExtentPoint32W(dc, text.as_ptr(), count, &mut size);
            let width = size.cx + self.px(14);
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

        // The pipeline name, and to its left whatever the choice costs this
        // document. Only the name answers a click.
        let label = wide(&self.stats.label(self.pinned));
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
        if let Some(warning) = self.stats.warning(self.pinned) {
            let warning = wide(&format!("{warning}  \u{b7}  "));
            let count = warning.len() as i32 - 1;
            GetTextExtentPoint32W(dc, warning.as_ptr(), count, &mut size);
            left -= size.cx;
            // Brighter than the summary beside it: this is worth reading.
            SetTextColor(dc, palette.foreground);
            TextOutW(dc, left, y, warning.as_ptr(), count);
        }

        // Whatever room is left between the two goes to the summary, which is
        // shortened rather than allowed to run underneath them.
        let summary = wide(&format!("\u{b7}  {}", self.stats.summary()));
        let mut room = Rect {
            left: x + self.px(8),
            top: bar.top,
            right: left - self.px(8),
            bottom: bar.bottom,
        };
        if room.right > room.left {
            SetTextColor(dc, palette.muted);
            // DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS.
            DrawTextW(
                dc,
                summary.as_ptr(),
                summary.len() as i32 - 1,
                &mut room,
                0x20 | 0x4 | 0x8000,
            );
        }
        SelectObject(dc, old);
    }
    /// What a client-area point lands on, if anything.
    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        self.hot
            .borrow()
            .iter()
            .find(|(rect, _)| {
                (rect.left..rect.right).contains(&x) && (rect.top..rect.bottom).contains(&y)
            })
            .map(|(_, spot)| *spot)
    }
    /// Build the menu bar from the palettes the INI defines. Called again
    /// whenever that list changes.
    unsafe fn build_menu(&mut self, hwnd: Hwnd) {
        if self.menu_font.is_null() {
            self.menu_font = ui_font(self.px(12));
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
        item(options, 1, 0, ""); // A grayed, unselectable separator rule.
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

    /// Put the floating control where it belongs: the far corner of the text,
    /// which is the left one when the document itself reads right to left.
    unsafe fn place_floater(&mut self) {
        if self.floater.is_null() {
            return;
        }
        let showing = self.floating && self.width > self.px(200);
        let (width, height) = (self.px(108), self.px(28));
        let inset = self.px(16);
        let x = if self.stats.rtl() {
            inset
        } else {
            self.width - self.bar().0 - inset - width
        };
        SetWindowPos(self.floater, NULL, x, inset, width, height, 0x4);
        ShowWindow(self.floater, showing.into());
        SetWindowLongPtrW(self.floater, size_of::<isize>() as i32, self.zoom as isize);
        InvalidateRect(self.floater, ptr::null(), 0);
    }
    unsafe fn apply_theme(&mut self, hwnd: Hwnd) {
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
        let dark: i32 = palette.is_dark().into();
        DwmSetWindowAttribute(hwnd, 20, &dark as *const _ as _, 4);
        // Windows 11 caption colors; older versions safely ignore these attributes.
        DwmSetWindowAttribute(hwnd, 35, &palette.background as *const _ as _, 4);
        DwmSetWindowAttribute(hwnd, 36, &palette.foreground as *const _ as _, 4);
        // Menu items are owner-drawn; this brush fills the bar around them.
        let brush = CreateSolidBrush(palette.background);
        let mut info = MenuInfo {
            size: std::mem::size_of::<MenuInfo>() as u32,
            mask: 0x02 | 0x80000000, // MIM_BACKGROUND | MIM_APPLYTOSUBMENUS.
            back: brush,
            ..Default::default()
        };
        for menu in [self.menu, GetMenu(hwnd)] {
            SetMenuInfo(menu, &info);
            info.mask = 0x02;
        }
        if !self.menu_brush.is_null() {
            DeleteObject(self.menu_brush);
        }
        self.menu_brush = brush;
        DrawMenuBar(hwnd);
        // Repaint the frame so the scrollbar picks up its new theme.
        SetWindowPos(hwnd, NULL, 0, 0, 0, 0, 0x27);
        InvalidateRect(hwnd, ptr::null(), 0);
    }

    /// Persist the choices the settings window can change. A read-only folder
    /// simply keeps the session's choice without storing it.
    unsafe fn save(&self) {
        let path = wide(&self.theme_path.to_string_lossy());
        let section = wide("viewer");
        let (name, palette) = &self.themes.palettes[self.theme.min(self.themes.palettes.len() - 1)];
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
        ] {
            WritePrivateProfileStringW(
                section.as_ptr(),
                wide(key).as_ptr(),
                wide(&value).as_ptr(),
                path.as_ptr(),
            );
        }
        let section = wide(name);
        for (index, (key, _)) in Palette::ROLES.iter().enumerate() {
            let color = palette.role(index);
            // Back to #RRGGBB from the COLORREF byte order.
            let hex = format!(
                "#{:02X}{:02X}{:02X}",
                color & 0xff,
                (color >> 8) & 0xff,
                (color >> 16) & 0xff
            );
            WritePrivateProfileStringW(
                section.as_ptr(),
                wide(key).as_ptr(),
                wide(&hex).as_ptr(),
                path.as_ptr(),
            );
        }
    }
}

/// The zoom control that floats over the document. A child window, so the
/// parent's `WS_CLIPCHILDREN` keeps scrolling from ever touching it: drawing it
/// with the page would mean repainting the whole page on every scrolled frame.
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
            fill(dc, client, palette.code_background);
            let border = CreateSolidBrush(palette.rule);
            FrameRect(dc, &client, border);
            DeleteObject(border);
            let old = SelectObject(dc, font);
            SetBkMode(dc, 1);
            let zoom = GetWindowLongPtrW(hwnd, size_of::<isize>() as i32) as i32;
            let width = (client.right / 3).max(1);
            for (index, (_, _, label, _)) in ZOOM.iter().enumerate() {
                let text = wide(&label.replace('%', &format!("{zoom}%")));
                let count = text.len() as i32 - 1;
                let mut cell = client;
                cell.left = width * index as i32;
                cell.right = cell.left + width;
                if hovered == index as isize {
                    fill(dc, cell, palette.accent);
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
                // One request is enough; Windows sends WM_MOUSELEAVE once.
                let mut track = TrackMouse {
                    size: size_of::<TrackMouse>() as u32,
                    flags: 2, // TME_LEAVE.
                    window: hwnd,
                    time: 0,
                };
                TrackMouseEvent(&mut track);
            }
            0
        }
        0x2a3 => {
            // WM_MOUSELEAVE.
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

/// The interface font, shared by the menu and the settings window.
unsafe fn ui_font(height: i32) -> Handle {
    CreateFontW(
        -height,
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
        wide("Segoe UI").as_ptr(),
    )
}
/// Register a window class, once per name.
unsafe fn register(name: &[u16], proc: Wndproc, background: Handle, extra: i32) -> bool {
    let instance = GetModuleHandleW(ptr::null());
    // Resource 1 is the application icon. A build without it still gets a
    // window, wearing the system's default.
    let icon = match LoadIconW(instance, 1usize as *const u16) {
        icon if icon.is_null() => LoadIconW(NULL, 32512usize as *const u16),
        icon => icon,
    };
    RegisterClassW(&WndClass {
        style: 0,
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
    // A UTF-8 byte order mark is an encoding signature, not document text.
    Ok(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_owned(),
        None => text,
    })
}

unsafe fn error(hwnd: Hwnd, text: &str) {
    MessageBoxW(hwnd, wide(text).as_ptr(), wide("MDLite").as_ptr(), 0x10);
}

unsafe fn load_file(hwnd: Hwnd, state: &RefCell<App>, path: PathBuf) {
    use std::io::Read;
    let started = Instant::now();
    // Bounded read also handles a file growing while it is being read.
    let result = (|| {
        let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Choose a regular text file.".to_owned());
        }
        const LIMIT: u64 = 64 * 1024 * 1024;
        let mut bytes = Vec::new();
        file.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > LIMIT {
            return Err("The document exceeds the 64 MiB size limit.".to_owned());
        }
        decode(bytes)
    })();
    match result {
        Ok(text) => {
            let mut app = state.borrow_mut();
            let saved_scroll = if app.file.as_ref() == Some(&path) {
                app.scroll
            } else {
                0
            };
            app.set_document(&text);
            app.source = text;
            app.scroll = saved_scroll;
            app.file = Some(path.clone());
            SetWindowTextW(
                hwnd,
                wide(&format!(
                    "{} — MDLite",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ))
                .as_ptr(),
            );
            app.layout(hwnd);
            app.stats.micros = started.elapsed().as_micros();
        }
        Err(message) => error(
            hwnd,
            &format!("Could not open {}\n\n{}", path.display(), message),
        ),
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
    info.flags = 0x80000 | 0x1000 | 0x800 | 8; // Explorer, existing file/path, no CWD mutation.
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
            // The application is quitting; let the outer loop see it too.
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
    }
    // A palette may have been added, or an addition undone by Cancel.
    app.build_menu(owner);
    app.apply_theme(owner);
    app.place_floater();
}

/// Build the settings window: an owner-modal popup of plain Win32 controls.
unsafe fn settings_window(owner: Hwnd, state: &RefCell<App>) -> Hwnd {
    const STYLE: u32 = 0x8000_0000 | 0x00c0_0000 | 0x0008_0000; // Popup, caption, menu.
    const CHILD: u32 = 0x5000_0000; // WS_CHILD | WS_VISIBLE.
    const RADIO: u32 = 9; // BS_AUTORADIOBUTTON.
    const FIRST: u32 = 0x0002_0000 | 0x0001_0000; // WS_GROUP | WS_TABSTOP.
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    let class = wide("MDLite.Settings");
    let instance = GetModuleHandleW(ptr::null());
    REGISTERED.call_once(|| {
        // COLOR_BTNFACE + 1, and room for the font handle.
        register(
            &class,
            settings_proc,
            16 as Handle,
            size_of::<Handle>() as i32,
        );
    });
    let dpi = GetDpiForWindow(owner).max(96) as i32;
    let s = |n: i32| n * dpi / 96;
    let mut rect = Rect {
        left: 0,
        top: 0,
        right: s(380),
        bottom: s(470),
    };
    AdjustWindowRect(&mut rect, STYLE, 0);
    let mut frame = Rect::default();
    GetWindowRect(owner, &mut frame);
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    let hwnd = CreateWindowExW(
        1, // WS_EX_DLGMODALFRAME.
        class.as_ptr(),
        wide("Settings").as_ptr(),
        STYLE,
        frame.left + (frame.right - frame.left - width) / 2,
        frame.top + (frame.bottom - frame.top - height) / 3,
        width,
        height,
        owner,
        NULL,
        instance,
        ptr::null_mut(),
    );
    if hwnd.is_null() {
        return NULL;
    }
    SetWindowLongPtrW(hwnd, -21, state as *const RefCell<App> as isize);
    let font = ui_font(s(12));
    let control = |class: &str, text: &str, style: u32, (x, y, w, h), id: usize| {
        let handle = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            CHILD | style,
            s(x),
            s(y),
            s(w),
            s(h),
            hwnd,
            id as Handle,
            instance,
            ptr::null_mut(),
        );
        SendMessageW(handle, 0x30, font as usize, 1); // WM_SETFONT.
        handle
    };
    let (chosen, smooth, floating, theme, names, breakdown) = {
        let mut app = state.borrow_mut();
        // Remember what to put back if the reader cancels after previewing.
        app.restore = Some((app.themes.clone(), app.theme));
        (
            app.pinned,
            app.smooth,
            app.floating,
            app.theme,
            app.themes
                .palettes
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
            app.stats.breakdown(app.pinned),
        )
    };
    control("BUTTON", "Rendering pipeline", 7, (12, 8, 356, 196), 0); // BS_GROUPBOX.
    let pipes = [(PIPE, "&Automatic — best pipeline per block (recommended)")]
        .into_iter()
        .chain(Pipe::ALL.map(|pipe| (PIPE + 1 + pipe as usize, pipe.label())));
    for (index, (id, label)) in pipes.enumerate() {
        let style = RADIO | if index == 0 { FIRST } else { 0 };
        let button = control(
            "BUTTON",
            label,
            style,
            (26, 30 + 24 * index as i32, 330, 22),
            id,
        );
        let on = chosen.map_or(index == 0, |pipe| index == 1 + pipe as usize);
        SendMessageW(button, 0xf1, on as usize, 0); // BM_SETCHECK.
    }
    // What the choice under the pointer means, and what it costs this
    // document. Both are refreshed as the selection moves.
    control("STATIC", Pipe::note(chosen), 0, (26, 128, 330, 46), NOTE);
    control("STATIC", &breakdown, 0, (26, 176, 330, 18), SPREAD);
    control("BUTTON", "Colours", 7, (12, 212, 356, 208), 0);
    control("STATIC", "&Palette:", 0, (26, 238, 56, 20), 0);
    // CBS_DROPDOWNLIST, tall enough to drop open over the buttons below.
    let list = control(
        "COMBOBOX",
        "",
        3 | 0x0020_0000 | FIRST,
        (86, 234, 270, 260),
        CHOICE,
    );
    control("STATIC", "&New:", 0, (26, 266, 56, 20), 0);
    // WS_BORDER | ES_AUTOHSCROLL: a name for a palette of one's own.
    control(
        "EDIT",
        "",
        0x0080_0000 | 0x0001_0080,
        (86, 262, 176, 22),
        NAME,
    );
    control("BUTTON", "&Add", 0x0001_0000, (270, 261, 86, 24), ADD);
    for name in &names {
        SendMessageW(list, 0x143, 0, wide(name).as_ptr() as isize); // CB_ADDSTRING.
    }
    SendMessageW(list, 0x14e, theme, 0); // CB_SETCURSEL.
    for (index, (_, label)) in Palette::ROLES.iter().enumerate() {
        let (column, row) = (index as i32 / 4, index as i32 % 4);
        let (x, y) = (26 + column * 170, 296 + row * 26);
        // BS_OWNERDRAW: the swatch is the colour itself.
        control(
            "BUTTON",
            "",
            0x0b | 0x0001_0000,
            (x, y, 26, 20),
            SWATCH + index,
        );
        control("STATIC", label, 0, (x + 32, y + 3, 134, 18), 0);
    }
    let hint = "Click a colour to change it. The page updates live.";
    control("STATIC", hint, 0, (26, 394, 330, 18), 0);
    // BS_AUTOCHECKBOX, on the button row so the window stays this tall.
    let scrolling = control(
        "BUTTON",
        "&Smooth scrolling",
        3 | FIRST,
        (26, 436, 132, 20),
        SMOOTH,
    );
    SendMessageW(scrolling, 0xf1, smooth as usize, 0);
    let floater = control("BUTTON", "&Zoom control", 3, (164, 436, 116, 20), FLOAT);
    SendMessageW(floater, 0xf1, floating as usize, 0);
    control("BUTTON", "OK", 1 | FIRST, (198, 430, 80, 28), OK); // BS_DEFPUSHBUTTON.
    control("BUTTON", "Cancel", 0x0001_0000, (286, 430, 80, 28), CANCEL);
    // The font outlives the controls that use it and is released with them.
    SetWindowLongPtrW(hwnd, 0, font as isize);
    hwnd
}

/// Apply a change made in the settings window to the document behind it.
unsafe fn preview(hwnd: Hwnd, state: &RefCell<App>) {
    // GWLP_HWNDPARENT is the owner of a popup window.
    let owner = GetWindowLongPtrW(hwnd, -8) as Hwnd;
    if !owner.is_null() {
        state.borrow_mut().apply_theme(owner);
    }
}

unsafe extern "system" fn settings_proc(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize {
    let state = GetWindowLongPtrW(hwnd, -21) as *const RefCell<App>;
    if msg == WM_CLOSE {
        DestroyWindow(hwnd);
        return 0;
    }
    if msg == WM_DRAWITEM && !state.is_null() {
        let item = &*(lp as *const DrawItem);
        if let Some(role) = (item.id as usize).checked_sub(SWATCH).filter(|r| *r < 7) {
            let Ok(app) = (*state).try_borrow() else {
                return 1;
            };
            fill(item.dc, item.rect, app.palette().role(role));
            let border = CreateSolidBrush(Palette::contrasting(app.palette().role(role)));
            FrameRect(item.dc, &item.rect, border);
            DeleteObject(border);
            return 1;
        }
    }
    if msg == WM_COMMAND && !state.is_null() {
        let id = wp & 0xffff;
        // A different palette, chosen from the list.
        if id == CHOICE && (wp >> 16) & 0xffff == 1 {
            let chosen = SendMessageW(GetDlgItem(hwnd, CHOICE as i32), 0x147, 0, 0); // CB_GETCURSEL.
            if chosen >= 0 {
                (*state).borrow_mut().theme = chosen as usize;
                preview(hwnd, &*state);
                InvalidateRect(hwnd, ptr::null(), 1);
            }
            return 0;
        }
        // Add: a palette of the reader's own, starting from what is showing.
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
            let index = app.themes.add(&typed, from);
            if index == app.themes.palettes.len() - 1 {
                let name = wide(&app.themes.palettes[index].0);
                SendMessageW(list, 0x143, 0, name.as_ptr() as isize); // CB_ADDSTRING.
            }
            app.theme = index;
            drop(app);
            SendMessageW(list, 0x14e, index, 0); // CB_SETCURSEL.
            SetWindowTextW(GetDlgItem(hwnd, NAME as i32), wide("").as_ptr());
            preview(hwnd, &*state);
            InvalidateRect(hwnd, ptr::null(), 1);
            return 0;
        }
        // A swatch: the system colour picker edits that role in place.
        if let Some(role) = id.checked_sub(SWATCH).filter(|r| *r < 7) {
            let mut custom = [0xffffffu32; 16];
            let mut choose: ChooseColor = std::mem::zeroed();
            choose.size = std::mem::size_of::<ChooseColor>() as u32;
            choose.owner = hwnd;
            choose.custom = custom.as_mut_ptr();
            choose.result = (*state).borrow().palette().role(role);
            choose.flags = 0x1 | 0x2; // CC_RGBINIT | CC_FULLOPEN.
            if ChooseColorW(&mut choose) != 0 {
                {
                    let mut app = (*state).borrow_mut();
                    let theme = app.theme;
                    app.themes.palettes[theme].1.set_role(role, choose.result);
                }
                preview(hwnd, &*state);
                InvalidateRect(GetDlgItem(hwnd, id as i32), ptr::null(), 1);
            }
            return 0;
        }
    }
    if msg == WM_DESTROY {
        DeleteObject(GetWindowLongPtrW(hwnd, 0) as Handle);
        // Anything still held here means the window closed without OK, so the
        // live preview has to be put back. Escape and the X land here too.
        if !state.is_null() {
            let restore = (*state).borrow_mut().restore.take();
            if let Some((themes, theme)) = restore {
                let mut app = (*state).borrow_mut();
                app.themes = themes;
                app.theme = theme;
                drop(app);
                preview(hwnd, &*state);
            }
        }
        return 0;
    }
    if msg == WM_COMMAND && (PIPE..PIPE + 4).contains(&(wp & 0xffff)) {
        // Say what this choice means, and costs, now rather than after OK.
        let chosen = (wp & 0xffff)
            .checked_sub(PIPE + 1)
            .and_then(|index| Pipe::ALL.get(index).copied());
        SetWindowTextW(
            GetDlgItem(hwnd, NOTE as i32),
            wide(Pipe::note(chosen)).as_ptr(),
        );
        SetWindowTextW(
            GetDlgItem(hwnd, SPREAD as i32),
            wide(&(*state).borrow().stats.breakdown(chosen)).as_ptr(),
        );
        return 0;
    }
    if msg == WM_COMMAND && matches!(wp & 0xffff, OK | CANCEL) {
        if wp & 0xffff == OK && !state.is_null() {
            let mut app = (*state).borrow_mut();
            // Keeping the previewed palette is simply not restoring it.
            app.restore = None;
            let checked = |id: usize| SendMessageW(GetDlgItem(hwnd, id as i32), 0xf0, 0, 0) == 1;
            let chosen = Pipe::ALL
                .into_iter()
                .find(|pipe| checked(PIPE + 1 + *pipe as usize));
            app.restyle = app.pinned != chosen;
            app.pinned = chosen;
            app.smooth = checked(SMOOTH);
            app.floating = checked(FLOAT);
            app.save();
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
    // The Box stays alive until the message loop exits. RefCell guards against
    // synchronous Win32 reentrancy (e.g. scrollbar changes generating WM_SIZE).
    let state = &*state;
    if msg == WM_COMMAND {
        match wp & 0xffff {
            OPEN => {
                if let Some(path) = choose_file(hwnd) {
                    load_file(hwnd, state, path);
                }
                return 0;
            }
            RELOAD => {
                let path = state.borrow().file.clone();
                if let Some(path) = path {
                    load_file(hwnd, state, path);
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
    // Painting a menu must not need the application state: Windows asks for
    // it from inside calls the viewer makes while holding that state.
    if msg == WM_MEASUREITEM {
        let item = &mut *(lp as *mut MeasureItem);
        if item.kind == 1 {
            measure_menu_item(item); // ODT_MENU.
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
        let length = DragQueryFileW(drop, 0, ptr::null_mut(), 0);
        let mut buffer = vec![0u16; length as usize + 1];
        DragQueryFileW(drop, 0, buffer.as_mut_ptr(), buffer.len() as u32);
        DragFinish(drop);
        if length > 0 {
            load_file(
                hwnd,
                state,
                PathBuf::from(OsString::from_wide(&buffer[..length as usize])),
            );
        }
        return 0;
    }
    // DefWindowProc may synchronously ask us to paint (WM_PRINT, for example).
    // Do not borrow application state while Windows handles unrelated messages.
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
            | WM_MOUSEMOVE
            | WM_TIMER
            | WM_SETCURSOR
            | 0x318
    ) {
        return DefWindowProcW(hwnd, msg, wp, lp);
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
        WM_LBUTTONDOWN => {
            let (x, y) = ((lp as i16) as i32, ((lp >> 16) as i16) as i32);
            if x >= app.bar().1 && y < app.height {
                if let Some((top, height)) = app.thumb() {
                    // Grabbing the thumb drags it; the track pages towards the click.
                    app.grab = (top..top + height).contains(&y).then(|| y - top);
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
            match app.hit(x, y) {
                Some(Hit::Engine) => {
                    let text = app.stats.explain(app.pinned);
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
            0
        }
        WM_TIMER => {
            app.glide(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = ((lp as i16) as i32, ((lp >> 16) as i16) as i32);
            if app.grab.is_some() {
                let target = app.bar_target(y);
                app.scroll_to(hwnd, target);
                return 0;
            }
            let hover = x >= app.bar().1 && y < app.height && app.thumb().is_some();
            if hover != app.hover {
                app.hover = hover;
                app.invalidate_bar(hwnd, app.thumb());
            }
            0
        }
        WM_LBUTTONUP => {
            if app.grab.take().is_some() {
                ReleaseCapture();
                app.invalidate_bar(hwnd, app.thumb());
            }
            0
        }
        WM_SETCURSOR => {
            // Only the client area, and only over the clickable label.
            let mut point = Point::default();
            GetCursorPos(&mut point);
            ScreenToClient(hwnd, &mut point);
            if wp == hwnd as usize
                && (lp as u32 & 0xffff) == 1
                && app.hit(point.x, point.y).is_some()
            {
                SetCursor(LoadCursorW(NULL, 32649usize as *const u16));
                return 1;
            }
            drop(app);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        0x318 => {
            // WM_PRINTCLIENT: native window previews and capture.
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
            } // Minimized.
            if width != app.width {
                if !app.layout_pending {
                    app.layout_pending = true;
                    PostMessageW(hwnd, WM_APP_LAYOUT, 0, 0);
                }
            } else {
                app.height = height - app.status();
                let scroll = app.scroll;
                app.scroll_to(hwnd, scroll);
                app.update_scroll(hwnd);
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
            // `set_dpi` threw the menu font away; rebuild it and republish the
            // palette before anything asks to paint a menu with the old one.
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
                    app.themes = Themes::load(&app.theme_path);
                    app.theme = app.theme.min(app.themes.palettes.len() - 1);
                    app.smooth = app.themes.smooth;
                    app.floating = app.themes.floating;
                    app.build_menu(hwnd);
                    let chosen = pinned(&app.themes);
                    if chosen != app.pinned {
                        app.pinned = chosen;
                        app.rebuild(hwnd);
                    }
                }
                _ => return DefWindowProcW(hwnd, msg, wp, lp),
            }
            app.apply_theme(hwnd);
            0
        }
        WM_MOUSEWHEEL => {
            app.wheel_remainder += ((wp >> 16) as u16 as i16) as i32;
            let steps = app.wheel_remainder / 120;
            app.wheel_remainder %= 120;
            // Ctrl and the wheel resize the text, as everywhere else.
            if wp & 8 != 0 {
                if steps != 0 {
                    let zoom = app.zoom + steps * Themes::ZOOM.2;
                    app.set_zoom(hwnd, zoom);
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
            // Notches during an animation add to where it is already going.
            let target = app.target.saturating_sub(steps.saturating_mul(amount));
            app.glide_to(hwnd, target);
            0
        }
        WM_KEYDOWN => {
            let ctrl = GetKeyState(0x11) < 0;
            // Ctrl+O, Ctrl+comma, F5 alone or with Ctrl, and Ctrl+D for the
            // next palette. Everything else scrolls.
            let command = match (ctrl, wp) {
                (true, 0x4f) => Some(OPEN),
                (true, 0xbc) => Some(SETTINGS),
                (_, 0x74) => Some(if ctrl { RELOAD_THEME } else { RELOAD }),
                (true, 0x44) => Some(PALETTE + (app.theme + 1) % app.themes.palettes.len()),
                _ => None,
            };
            // Ctrl with plus, minus or zero: bigger, smaller, back to normal.
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
            let from = app.target;
            let target = match wp {
                0x26 => from.saturating_sub(app.px(24)),
                0x28 => from.saturating_add(app.px(24)),
                0x21 => from.saturating_sub(page),
                0x22 => from.saturating_add(page),
                0x20 => {
                    if GetKeyState(0x10) < 0 {
                        from.saturating_sub(page)
                    } else {
                        from.saturating_add(page)
                    }
                }
                0x24 => 0,
                0x23 => app.content_height,
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
            // WS_CLIPCHILDREN so scrolling never paints over the floater.
            0x00cf0000 | 0x0200_0000,
            i32::MIN,
            i32::MIN,
            920,
            720,
            NULL,
            NULL,
            instance,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            error(NULL, "Could not create the native window.");
            return;
        }
        // The class icon is one size for every use; give the title bar and the
        // task switcher the images drawn for them instead of a downscale.
        for (which, metric) in [(0, 49), (1, 11)] {
            let side = GetSystemMetrics(metric);
            // LR_SHARED: system-standard sizes, cached and owned by Windows.
            let icon = LoadImageW(instance, 1usize as *const u16, 1, side, side, 0x8000);
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, which, icon as isize);
            }
        }
        // Two pointer slots: the hovered cell, and the zoom it displays.
        let floating = wide("MDLite.Zoom");
        let floater = register(&floating, floater_proc, NULL, 2 * size_of::<isize>() as i32)
            .then(|| {
                let child = CreateWindowExW(
                    0,
                    floating.as_ptr(),
                    ptr::null(),
                    0x4000_0000, // WS_CHILD, shown once it has a place.
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
            app.source =
                "# MDLite\n\nOpen a Markdown document with Ctrl+O, or drop a file here.".to_owned();
            let source = std::mem::take(&mut app.source);
            app.set_document(&source);
            app.source = source;
            app.layout(hwnd);
            app.apply_theme(hwnd);
        }
        DragAcceptFiles(hwnd, 1);
        if let Some(path) = std::env::args_os().nth(1) {
            load_file(hwnd, &state, Path::new(&path).to_owned());
        }
        ShowWindow(hwnd, 5);
        UpdateWindow(hwnd);
        let mut message = Msg::default();
        loop {
            let result = GetMessageW(&mut message, NULL, 0, 0);
            if result <= 0 {
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        // Remember the palette, pipeline and text size for next time.
        state.borrow().save();
        // The window owns its menus; fonts are released by App when state drops.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A hidden native window gives real GDI font metrics without touching the
    /// desktop. Layout and scrollbar range can then be verified headlessly.
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
        app.source = source.to_owned();
    }
    /// Save an off-screen surface as `%TEMP%\mdlite-<name>.bmp`, for looking
    /// at a layout change rather than only asserting it.
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
    /// Render a document off-screen and write it out. Ignored by default: it
    /// exists so a change to layout or painting can be looked at. Run with
    /// `cargo test --offline -- --ignored --nocapture preview`.
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
            ("emoji", true, include_str!("../examples/emoji.md")),
        ] {
            unsafe {
                let hwnd = window(900, 760);
                let mut app = App::new();
                app.theme = usize::from(!dark);
                document(&mut app, source);
                app.layout(hwnd);
                let (width, height) = (app.width, app.height + app.status());
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
    /// Build the settings window off-screen, check its controls, and save a
    /// picture of it next to the document previews.
    #[test]
    #[ignore]
    fn preview_settings() {
        unsafe {
            for (name, pinned) in [("auto", None), ("pinned", Some(Pipe::Gdi))] {
                let owner = window(900, 700);
                let state = RefCell::new(App::new());
                {
                    let mut app = state.borrow_mut();
                    app.pinned = pinned;
                    // A real document, so the breakdown has something to say.
                    document(&mut app, include_str!("../examples/complex-scripts.md"));
                }
                let hwnd = settings_window(owner, &state);
                assert!(!hwnd.is_null());
                // Exactly one pipeline radio is selected, and it is the right one.
                let checked =
                    |id: usize| SendMessageW(GetDlgItem(hwnd, id as i32), 0xf0, 0, 0) == 1;
                assert!(checked(PIPE) == pinned.is_none());
                assert_eq!(
                    Pipe::ALL
                        .into_iter()
                        .find(|pipe| checked(PIPE + 1 + *pipe as usize)),
                    pinned
                );
                // The note describes the selected mode, not a fixed warning.
                let note = GetDlgItem(hwnd, NOTE as i32);
                assert!(!note.is_null());
                let mut text = [0u16; 512];
                let length = GetWindowTextW(note, text.as_mut_ptr(), text.len() as i32);
                assert_eq!(
                    String::from_utf16(&text[..length as usize]).unwrap(),
                    Pipe::note(pinned),
                    "the note must describe the mode that is selected"
                );
                // And the line under it costs that mode for the open document.
                let spread = GetDlgItem(hwnd, SPREAD as i32);
                let length = GetWindowTextW(spread, text.as_mut_ptr(), text.len() as i32);
                assert_eq!(
                    String::from_utf16(&text[..length as usize]).unwrap(),
                    state.borrow().stats.breakdown(pinned)
                );
                assert!(
                    state.borrow().stats.breakdown(pinned).contains("Direct2D"),
                    "a complex-script document needs Direct2D under any setting"
                );
                // PrintWindow needs a visible window, so show it far off-screen.
                SetWindowPos(hwnd, NULL, -4000, -4000, 0, 0, 0x5); // NOSIZE | NOZORDER.
                ShowWindow(hwnd, 8); // SW_SHOWNA: no activation, no focus stolen.
                let mut rect = Rect::default();
                GetWindowRect(hwnd, &mut rect);
                let surface =
                    Buffer::new(NULL, rect.right - rect.left, rect.bottom - rect.top).unwrap();
                PrintWindow(hwnd, surface.dc, 0);
                save(&surface, &format!("settings-{name}"));
                DestroyWindow(hwnd);
                DestroyWindow(owner);
            }
        }
    }
    /// Where the time goes when a large document is opened and resized.
    /// `cargo test --offline -- --ignored --nocapture bench`
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
                    app.rows.len()
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
    /// Documents built to break the viewer. Nothing here asserts what the
    /// output looks like: the requirement is that it lays out at all, inside a
    /// sane amount of time and memory, whatever the file contains.
    #[test]
    #[ignore]
    fn shaping_cost_by_paragraph_length() {
        unsafe {
            let hwnd = window(700, 500);
            let mut app = App::new();
            let units = [
                (
                    "everything, unbalanced override",
                    "a\u{5d0}\u{628}\u{939}\u{e01}\u{1f600}\u{301}\u{200d}\u{fe0f}\u{202e}",
                ),
                (
                    "the same without the override",
                    "a\u{5d0}\u{628}\u{939}\u{e01}\u{1f600}\u{301}\u{200d}\u{fe0f}b",
                ),
                ("plain Arabic words", "\u{645}\u{631}\u{62d}\u{628}\u{627} "),
                ("plain English words", "hello world "),
            ];
            for (label, unit) in units {
                for repeats in [2_500, 5_000, 10_000, 20_000] {
                    let source = unit.repeat(repeats);
                    let started = Instant::now();
                    document(&mut app, &source);
                    let parsed = started.elapsed();
                    let started = Instant::now();
                    app.layout(hwnd);
                    println!(
                        "{label}: {:>7} chars, parse {:?}, layout {:?}",
                        source.chars().count(),
                        parsed,
                        started.elapsed()
                    );
                }
            }
            DestroyWindow(hwnd);
        }
    }
    /// The limits that keep a hostile or simply enormous file from taking the
    /// process down with it.
    #[test]
    fn a_document_too_large_to_hold_is_cut_and_says_so() {
        unsafe {
            let hwnd = window(700, 500);
            let mut app = App::new();
            // Two million lines: without a limit this is gigabytes of rows.
            let source = "line\n".repeat(2_000_000);
            let started = Instant::now();
            document(&mut app, &source);
            app.layout(hwnd);
            assert_eq!(app.blocks.len(), markdown::LONGEST_DOCUMENT);
            assert!(app.stats.truncated);
            assert!(app.stats.summary().contains("only the first million"));
            assert!(
                started.elapsed().as_secs() < 30,
                "took {:?}",
                started.elapsed()
            );
            // Still a working document: it scrolls, paints and ends somewhere.
            app.scroll_to(hwnd, i32::MAX);
            assert!(app.scroll > 0 && app.scroll == app.content_height - app.height);
            let buffer = Buffer::new(NULL, app.width, app.height + app.status()).unwrap();
            app.draw(
                buffer.dc,
                Rect {
                    left: 0,
                    top: 0,
                    right: app.width,
                    bottom: app.height + app.status(),
                },
            );

            // An ordinary document says nothing about being cut.
            document(&mut app, "a line\n\nanother\n");
            app.layout(hwnd);
            assert!(!app.stats.truncated);
            assert!(!app.stats.summary().contains("only the first"));
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
        // Every palette still has a command id the Options menu can carry.
        assert!(Themes::MOST_PALETTES <= PALETTES);
        // A file full of sections is bounded the same way.
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
            // Whatever leaves the index stale, neither reading nor writing the
            // palette may index past the end.
            app.theme = 999;
            assert_eq!(app.palette(), app.themes.palettes.last().unwrap().1);
            app.save();
            assert!(app.theme_path.exists());
            std::fs::remove_file(&app.theme_path).unwrap();
        }
    }
    /// Windows the viewer should not be asked to draw in, but might be.
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
                    assert!(app.content_height >= 0 && app.scroll >= 0);
                    // Painting is where the arithmetic would bite.
                    if let Some(buffer) =
                        Buffer::new(NULL, app.width.max(1), (app.height + app.status()).max(1))
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
                // Painting is where most of the arithmetic lands.
                let buffer = Buffer::new(NULL, app.width, app.height + app.status()).unwrap();
                app.draw(
                    buffer.dc,
                    Rect {
                        left: 0,
                        top: 0,
                        right: app.width,
                        bottom: app.height + app.status(),
                    },
                );
                app.scroll_to(hwnd, i32::MAX);
                app.draw(
                    buffer.dc,
                    Rect {
                        left: 0,
                        top: 0,
                        right: app.width,
                        bottom: app.height + app.status(),
                    },
                );
                assert!(
                    app.content_height >= 0 && app.scroll >= 0,
                    "{name}: {} tall, scrolled to {}",
                    app.content_height,
                    app.scroll
                );
                println!(
                    "{name}: {:?}, {} blocks, {} rows, {} tall",
                    started.elapsed(),
                    app.blocks.len(),
                    app.rows.len(),
                    app.content_height
                );
                assert!(
                    started.elapsed().as_secs() < 20,
                    "{name} took {:?}",
                    started.elapsed()
                );
            }
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn every_zoom_control_asks_for_what_its_label_says() {
        let mut app = App::new();
        app.zoom = 100;
        // The floating control divides its width into these three cells, left
        // to right, and posts ZOOM[cell].0. A click on "+" must not reset.
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
            // The menu item for the same command says the same thing.
            let wording = label.to_ascii_lowercase();
            assert_eq!(wording.contains("out"), *spot == Hit::ZoomOut);
            assert_eq!(wording.contains("in	"), *spot == Hit::ZoomIn);
            assert_eq!(wording.contains("reset"), *spot == Hit::ZoomReset);
        }
        // Command ids are distinct, so the menu and the control cannot collide.
        assert_eq!(
            ZOOM.iter().map(|(id, ..)| *id).collect::<Vec<_>>(),
            vec![105, 106, 107]
        );
    }
    #[test]
    fn the_floating_zoom_control_keeps_out_of_the_text() {
        unsafe {
            let hwnd = window(800, 600);
            let mut app = App::new();
            // A stand-in child: this checks where the control is put, not how
            // it paints itself.
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

            // An English page: the far corner is the right-hand one.
            document(
                &mut app,
                &"An English paragraph.

"
                .repeat(20),
            );
            app.layout(hwnd);
            assert!(!app.stats.rtl());
            let point = corner(&app);
            assert!(
                point.x > app.width / 2,
                "expected the right, got {point:?}",
                point = point.x
            );
            assert!(point.y < app.px(40) && shown(&app));

            // An Arabic page reads the other way, so the control moves over.
            document(
                &mut app,
                &"هذا نص عربي يقرأ من اليمين إلى اليسار.

"
                .repeat(20),
            );
            app.layout(hwnd);
            assert!(app.stats.rtl(), "this document reads right to left");
            assert!(corner(&app).x < app.width / 2, "expected the left");

            // One Arabic word does not turn an English page around.
            document(
                &mut app,
                &format!(
                    "{}

كلمة
",
                    "An English paragraph.

"
                    .repeat(20)
                ),
            );
            app.layout(hwnd);
            assert!(!app.stats.rtl());
            assert!(corner(&app).x > app.width / 2);

            // And it can be turned off entirely.
            app.floating = false;
            app.place_floater();
            assert!(!shown(&app));
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn zoom_scales_every_measurement_and_keeps_your_place() {
        unsafe {
            let hwnd = window(700, 500);
            let mut app = App::new();
            document(
                &mut app,
                &"A line of text.

"
                .repeat(300),
            );
            app.layout(hwnd);
            let (padding, height, content) = (app.px(24), app.rows[0].height, app.content_height);
            app.scroll_to(hwnd, content / 2);

            app.set_zoom(hwnd, 150);
            assert_eq!(app.zoom, 150);
            assert_eq!(app.px(24), padding * 3 / 2, "padding scales");
            assert!(app.rows[0].height > height, "and so does the text");
            assert!(app.content_height > content);
            // The reader stays at roughly the same place in the document.
            let before = content as f64 / 2.0 / content as f64;
            let after = app.scroll as f64 / app.content_height as f64;
            assert!((before - after).abs() < 0.02, "{before} became {after}");
            assert_eq!(app.stats.zoom, 150, "and the footer's cluster says so");

            // Bounded at both ends, and exactly reversible.
            app.set_zoom(hwnd, 10_000);
            assert_eq!(app.zoom, Themes::ZOOM.1);
            app.set_zoom(hwnd, 0);
            assert_eq!(app.zoom, Themes::ZOOM.0);
            app.set_zoom(hwnd, 100);
            app.layout(hwnd);
            assert_eq!((app.px(24), app.content_height), (padding, content));
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
            app.theme = 1; // Light.

            let from = app.palette();
            let index = app.themes.add("Sepia", from);
            app.theme = index;
            assert_eq!(app.palette(), from, "a copy of what was on screen");
            app.themes.palettes[index]
                .1
                .set_role(0, crate::theme::rgb(0x704214));
            app.save();

            let read = Themes::load(&app.theme_path);
            let sepia = read.find("Sepia").expect("the new section is written");
            assert_eq!(read.start, sepia, "and it is the startup palette");
            assert_eq!(
                read.palettes[sepia].1.background,
                crate::theme::rgb(0x704214)
            );
            assert_eq!(
                read.palettes[sepia].1.foreground,
                Palette::light().foreground,
                "the roles it did not change came from the palette it copied"
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
            app.theme = 1; // Light.
            app.themes.palettes[1].1.background = crate::theme::rgb(0x102030);
            app.save();
            let text = std::fs::read_to_string(&app.theme_path).unwrap();
            assert!(text.contains("; a comment the viewer must not eat"));
            let themes = Themes::load(&app.theme_path);
            assert_eq!(themes.renderer, Some(Pipe::GdiPlus as u8));
            assert_eq!(pinned(&themes), Some(Pipe::GdiPlus));
            assert_eq!(themes.start, 1, "the palette is named, not numbered");
            assert_eq!(themes.palettes[1].1.background, crate::theme::rgb(0x102030));
            assert_eq!(
                themes.palettes[0].1.accent,
                crate::theme::rgb(0x123456),
                "saving one palette must leave the others alone"
            );

            app.pinned = None;
            app.theme = 0;
            app.save();
            let themes = Themes::load(&app.theme_path);
            assert_eq!(pinned(&themes), None, "automatic must be storable too");
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
            app.blocks
                .iter()
                .filter(|block| block.kind == Kind::Paragraph)
                .map(|block| block.body.pipe)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            pipes(&app),
            [Pipe::Gdi, Pipe::GdiPlus, Pipe::D2d, Pipe::Gdi],
            "GDI, then GDI+, then Direct2D as the content demands"
        );
        // Only Direct2D blocks are shaped; the others keep interned GDI runs.
        assert!(app.blocks[4].body.shaped.is_some() && app.blocks[0].body.shaped.is_none());
        // The emoji paragraph stays on GDI, its glyph split into a cluster for
        // the shaping engine to draw in place.
        let emoji: Vec<_> = app.blocks[6]
            .body
            .runs
            .clone()
            .map(|i| &app.runs[app.run_order[i]])
            .collect();
        assert_eq!(emoji.len(), 2);
        assert!(emoji[0].is_emoji && !emoji[1].is_emoji);

        // Pinning a pipeline moves everything it can render, and nothing it cannot.
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
            let source = std::mem::take(&mut app.source);
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
                ("&Open…\tCtrl+O", 1, palette.accent), // ODS_SELECTED.
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
                assert!(
                    measure.height > 12 && measure.width > 100,
                    "an item with an accelerator needs a column for it: {measure:?}",
                    measure = (measure.width, measure.height)
                );
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
                assert!(
                    count(background) > (width * height / 2) as usize,
                    "the item background must come from the palette"
                );
                assert!(
                    count(palette.foreground) + count(palette.muted) > 20
                        || state == 1 && count(Palette::contrasting(palette.accent)) > 20,
                    "and the label must be drawn over it"
                );
            }
            // A separator is a rule, and asks for no accelerator column.
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
            let hwnd = window(700, 500);
            let mut app = App::new();
            document(
                &mut app,
                &format!(
                    "{}{}",
                    "العربية **نص** and English 😀

"
                    .repeat(20),
                    "A plain line.

"
                    .repeat(200)
                ),
            );
            app.layout(hwnd);
            InvalidateRect(hwnd, ptr::null(), 0);
            app.paint(hwnd);
            let (dc, width, height) = {
                let buffer = app.buffer.as_ref().expect("a back buffer must be built");
                (buffer.dc, buffer.width, buffer.height)
            };
            assert_eq!(
                (width, height),
                (app.width, app.height + app.status()),
                "the buffer covers the whole client, so one blit shows a frame"
            );
            // A frame composes into it, Direct2D shaped text included.
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
            assert_eq!(GetPixel(dc, 4, 4), background);
            let mut painted = 0;
            for y in (0..app.height).step_by(3) {
                for x in (0..app.width).step_by(3) {
                    painted += u32::from(GetPixel(dc, x, y) != background);
                }
            }
            assert!(painted > 500, "only {painted} painted pixels in the buffer");

            // Scrolling repaints the strip it exposes plus the thumb that moved.
            // Invalidating the whole bar instead would make the update region's
            // bounding box cover the window, and every frame would redraw it.
            let thumb = app.thumb();
            app.scroll_to(hwnd, 60);
            let update = app.bar_update(thumb);
            let (moved, _) = app.thumb().unwrap();
            assert!(update.right - update.left == app.bar().0);
            assert!(
                update.top <= moved && update.bottom - update.top < app.height / 2,
                "a 60 px scroll asked for {} rows of scrollbar",
                update.bottom - update.top
            );
            // Without a previous thumb the whole bar is repainted, once.
            let all = app.bar_update(None);
            assert_eq!((all.top, all.bottom), (0, app.height));
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn scroll_state_survives_without_a_system_scrollbar() {
        unsafe {
            // The helper builds a window without WS_VSCROLL, as the viewer does.
            let hwnd = window(400, 300);
            let mut info = ScrollInfo::new(1 | 2 | 4);
            info.max = 5000;
            info.page = 300;
            info.pos = 1234;
            SetScrollInfo(hwnd, 1, &info, 0);
            let mut read = ScrollInfo::new(1 | 2 | 4);
            GetScrollInfo(hwnd, 1, &mut read);
            assert_eq!(
                (read.max, read.page, read.pos),
                (5000, 300, 1234),
                "the window still publishes scroll state with no bar of its own"
            );
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn smooth_scrolling_eases_to_its_target_and_then_stops() {
        unsafe {
            let hwnd = window(700, 400);
            let mut app = App::new();
            document(
                &mut app,
                &"A line of text.

"
                .repeat(400),
            );
            app.layout(hwnd);
            let settle = |app: &mut App| {
                let mut frames = 0;
                while app.target != app.scroll && frames < 500 {
                    app.glide(hwnd);
                    frames += 1;
                }
                frames
            };
            // Switched off, a scroll is the jump it always was.
            app.smooth = false;
            app.glide_to(hwnd, 500);
            assert_eq!((app.scroll, app.target), (500, 500));

            // Switched on, it takes frames, always towards the target and never past it.
            app.smooth = true;
            app.scroll_to(hwnd, 0);
            app.glide_to(hwnd, 500);
            assert_eq!(app.scroll, 0, "no frame has run yet");
            assert_eq!(app.target, 500);
            let mut previous = -1;
            let mut frames = 0;
            while app.target != app.scroll && frames < 500 {
                app.glide(hwnd);
                assert!(app.scroll > previous && app.scroll <= 500);
                previous = app.scroll;
                frames += 1;
            }
            assert_eq!(app.scroll, 500);
            assert!(
                (4..60).contains(&frames),
                "{frames} frames is not a fifth of a second"
            );

            // A second request during a glide only moves the destination.
            app.glide_to(hwnd, 1000);
            app.glide(hwnd);
            app.glide_to(hwnd, 100);
            assert_eq!(app.target, 100);
            settle(&mut app);
            assert_eq!(app.scroll, 100);

            // And the document's own limits still win.
            app.glide_to(hwnd, i32::MAX);
            settle(&mut app);
            assert_eq!(app.scroll, app.content_height - app.height);
            app.glide_to(hwnd, -50);
            settle(&mut app);
            assert_eq!(app.scroll, 0);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn the_scrollbar_tracks_the_document_and_the_thumb_stays_grabbable() {
        unsafe {
            let hwnd = window(700, 400);
            let mut app = App::new();
            document(&mut app, &"A line of text.\n\n".repeat(400));
            app.layout(hwnd);
            let (bar_width, bar_x) = app.bar();
            assert!(bar_width > 0 && bar_x + bar_width == app.width);
            let (top, height) = app.thumb().expect("a long document needs a thumb");
            assert_eq!(top, 0, "at the start the thumb is at the top");
            assert!(height >= app.px(28) && height < app.height);
            app.scroll_to(hwnd, i32::MAX);
            let (bottom_top, _) = app.thumb().unwrap();
            assert_eq!(
                bottom_top,
                app.height - height,
                "at the end it rests on the bottom"
            );
            // Dragging from a grab point maps back to the same scroll.
            app.grab = Some(4);
            let target = app.bar_target(4);
            assert_eq!(target, 0);
            assert_eq!(app.bar_target(app.height), app.content_height - app.height);
            app.grab = None;
            // Clicking the track pages towards the click rather than jumping.
            app.scroll_to(hwnd, app.content_height / 2);
            assert!(app.bar_target(0) < app.scroll);
            assert!(app.bar_target(app.height - 1) > app.scroll);
            // A document that fits has no thumb at all.
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
            assert!(state.borrow().restore.is_some(), "Cancel needs a snapshot");

            // Preview a different palette and a hand-picked colour, as the
            // combo box and a swatch would.
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

            // Closing without OK is a cancel, whether by the button or the X.
            DestroyWindow(hwnd);
            assert!(state.borrow().restore.is_none());
            assert_eq!(state.borrow().palette(), original);
            assert_eq!(
                state.borrow().themes.palettes[2].1.background,
                Themes::parse(include_str!("../mdlite.ini")).palettes[2]
                    .1
                    .background,
                "an edited colour is rolled back too"
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
            assert!(
                app.themes.palettes.len() >= 4,
                "the shipped INI defines several palettes to choose from"
            );
            app.build_menu(hwnd);
            for index in 0..app.themes.palettes.len() {
                app.theme = index;
                app.apply_theme(hwnd);
                let (font, palette, dpi) = MENU.get();
                assert!(!font.is_null());
                assert_eq!(palette, app.palette(), "the menu follows the document");
                assert_eq!(dpi, app.dpi);
                assert!(!app.menu_brush.is_null());
            }
            // Rebuilding the menu frees the previous labels rather than leaking.
            let labels = app.menu_labels.len();
            app.build_menu(hwnd);
            assert_eq!(app.menu_labels.len(), labels);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn the_status_bar_reports_the_document_and_answers_a_click() {
        unsafe {
            let hwnd = window(700, 500);
            let mut app = App::new();
            document(&mut app, "# Title\n\nplain latin\n\nالعربية and 😀\n");
            app.layout(hwnd);
            assert_eq!(app.stats.lines, 5);
            // Counted from the source, so markup such as "#" counts as a word.
            assert_eq!(app.stats.words, 7);
            let spread = app.stats.spread(None);
            assert!(spread[Pipe::Gdi as usize] > 0 && spread[Pipe::D2d as usize] > 0);
            assert!(app.stats.label(None).starts_with("Direct2D + mixed"));
            assert_eq!(app.stats.warning(None), None, "automatic costs nothing");

            // Pinning names the choice, and says what this document pays for it.
            assert!(app.stats.label(Some(Pipe::Gdi)).starts_with("GDI (pinned)"));
            let escalated = app.stats.escalated(Some(Pipe::Gdi));
            assert!(escalated > 0, "the Arabic line cannot be drawn by GDI");
            assert_eq!(
                app.stats.warning(Some(Pipe::Gdi)),
                Some("1 block still needs Direct2D".to_owned()),
                "one Arabic line, and the wording agrees with it"
            );
            // Pinning Direct2D can never cost anything: it draws everything.
            assert_eq!(app.stats.warning(Some(Pipe::D2d)), None);
            assert_eq!(
                app.stats.spread(Some(Pipe::D2d))[Pipe::D2d as usize],
                app.stats.blocks.iter().sum::<usize>()
            );
            assert_eq!(
                app.stats.reasons & (render::RTL | render::COLOR),
                render::RTL | render::COLOR,
                "an Arabic line with an emoji is both right-to-left and coloured"
            );
            let explained = app.stats.explain(app.pinned);
            assert!(explained.contains("Chosen automatically"));
            assert!(explained.contains("right-to-left"));
            assert!(app.stats.explain(Some(Pipe::Gdi)).contains("Pinned to GDI"));

            // Painting measures the clickable label, which then hit-tests.
            let dc = GetDC(hwnd);
            app.draw(
                dc,
                Rect {
                    left: 0,
                    top: 0,
                    right: app.width,
                    bottom: app.height + app.status(),
                },
            );
            ReleaseDC(hwnd, dc);
            let (edge, footer) = (app.width - app.px(24), app.height + 2);
            assert_eq!(app.hit(edge - 1, footer), Some(Hit::Engine));
            assert_eq!(app.hit(edge - 1, app.height - 1), None, "that is the text");
            // Zoom sits at the near end: out, the percentage, in.
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
                assert!(rect.right > rect.left && rect.left >= app.px(24));
            }
            assert!(
                cells[2].0.right < cells[3].0.left,
                "the zoom cluster and the pipeline name never overlap"
            );
            drop(cells);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn tables_size_columns_align_cells_and_stay_inside_the_page() {
        unsafe {
            let hwnd = window(700, 500);
            let mut app = App::new();
            document(
                &mut app,
                "intro\n\n| Fruit | Qty | Price |\n|:------|:---:|------:|\n| Apple | 3 | 1.50 |\n| Kiwi | 12 | 0.25 |\n\nafter\n",
            );
            let headers: Vec<_> = app
                .blocks
                .iter()
                .filter_map(|block| match block.kind {
                    Kind::Row(header) => Some(header),
                    _ => None,
                })
                .collect();
            assert_eq!(headers, [true, false, false], "a header and two body rows");
            assert_eq!(app.blocks[2].cells.len(), 3);
            assert_eq!(
                app.blocks[2]
                    .cells
                    .iter()
                    .map(|cell| cell.align)
                    .collect::<Vec<_>>(),
                [0, 1, 2],
                "left, centre and right alignment come from the delimiter row"
            );
            // Header cells are bold, so they never share a measurement with body text.
            assert!(app.blocks[2].cells[0]
                .runs
                .clone()
                .all(|i| app.runs[app.run_order[i]].style.0 & Style::BOLD != 0));

            app.layout(hwnd);
            let padding = app.px(24);
            let grid: Vec<_> = app
                .rows
                .iter()
                .filter(|row| row.deco == Deco::Grid)
                .collect();
            assert_eq!(grid.len(), 3, "one separator under every table row");
            assert!(grid.iter().all(|row| row.span.0 == padding
                && row.span.1 <= app.width - padding
                && row.span.1 > row.span.0));
            assert!(
                grid[0].height > grid[1].height,
                "the header rule is the heavy one"
            );
            // Cells of one table row start together and never overlap.
            let cells: Vec<_> = app
                .rows
                .iter()
                .filter(|row| !row.parts.is_empty() && row.y > grid[0].y && row.y < grid[1].y)
                .collect();
            assert_eq!(cells.len(), 3);
            assert!(cells.iter().all(|row| row.y == cells[0].y));
            for pair in cells.windows(2) {
                let left = pair[0].parts.last().unwrap();
                assert!(left.x + left.width <= pair[1].parts[0].x, "columns overlap");
            }
            let first = &cells[0].parts[0];
            assert_eq!(
                String::from_utf16(&app.runs[first.run].text[first.range.clone()]).unwrap(),
                "Apple"
            );
            let right = cells[2].parts.last().unwrap();
            assert!(
                (right.x + right.width - (cells[2].span.1 - app.px(8))).abs() <= 1,
                "a right-aligned cell ends at its column edge"
            );
            assert!(
                cells[1].parts[0].x > cells[1].span.0 + app.px(8),
                "a centred cell is indented from its column edge"
            );
            assert!(app.rows.windows(2).all(|rows| rows[0].y <= rows[1].y));

            // A table far wider than the window shrinks instead of overflowing.
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
            for row in &app.rows {
                for part in &row.parts {
                    assert!(
                        part.x >= padding && part.x + part.width <= app.width - padding,
                        "table content escaped the page"
                    );
                }
            }
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn complex_paragraphs_keep_styles_share_storage_and_relayout_on_dpi() {
        unsafe {
            let hwnd = window(400, 300);
            let mut app = App::new();
            document(
                &mut app,
                "العربية **نص** English 123 😀\nالعربية **نص** English 123 😀\n# العربية **نص** English 123 😀",
            );
            assert!(Rc::ptr_eq(
                app.blocks[0].body.shaped.as_ref().unwrap(),
                app.blocks[1].body.shaped.as_ref().unwrap()
            ));
            assert!(app.runs.is_empty());
            app.layout(hwnd);
            assert!(app.rows.iter().all(|row| row.shaped.is_some()));
            assert_eq!(app.rows[0].height, app.rows[1].height);
            assert!(app.rows[2].height > app.rows[1].height);
            let height = app.content_height;
            app.set_dpi(144);
            app.layout(hwnd);
            assert!(app.content_height > height);
            app.set_dpi(96);
            app.layout(hwnd);
            assert_eq!(app.content_height, height);
            document(&mut app, include_str!("../examples/complex-scripts.md"));
            app.layout(hwnd);
            assert!(app
                .rows
                .windows(2)
                .all(|rows| rows[0].y + rows[0].height <= rows[1].y));
            assert!(app
                .rows
                .iter()
                .any(|row| row.deco == Deco::Code && row.shaped.is_some()));
            assert!(app
                .rows
                .iter()
                .any(|row| row.deco == Deco::Quote && row.shaped.is_some()));
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn native_layout_wraps_without_splitting_surrogates() {
        unsafe {
            let hwnd = window(340, 250);
            let mut app = App::new();
            let source = format!(
                "# Heading\n\n{}\n```\n{}\n```\n{}",
                "𝐡ello **bold** ".repeat(150),
                "x".repeat(12_000),
                "line\n".repeat(4000)
            );
            document(&mut app, &source);
            app.layout(hwnd);
            assert!(app.content_height > 65535);
            assert!(app.rows.len() > 4000);
            assert!(app.rows.windows(2).all(|r| r[0].y + r[0].height <= r[1].y));
            for row in &app.rows {
                for part in &row.parts {
                    let text = &app.runs[part.run].text[part.range.clone()];
                    assert!(String::from_utf16(text).is_ok());
                    assert!(part.width > 0);
                    assert!(part.x + part.width <= app.width - app.px(24));
                }
            }
            app.scroll_to(hwnd, i32::MAX);
            assert_eq!(app.scroll, app.content_height - app.height);
            let mut info = ScrollInfo::new(4);
            GetScrollInfo(hwnd, 1, &mut info);
            assert_eq!(info.pos, app.scroll);
            let narrow_rows = app.rows.len();
            SetWindowPos(hwnd, NULL, 0, 0, 1000, 500, 0x14);
            app.layout(hwnd);
            assert!(app.rows.len() < narrow_rows);
            let snapshot = |app: &App| {
                app.rows
                    .iter()
                    .map(|row| {
                        (
                            row.y,
                            row.height,
                            row.ascent,
                            row.parts
                                .iter()
                                .map(|p| (p.run, p.range.clone(), p.x, p.width))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let cached = snapshot(&app);
            for run in &app.runs {
                run.width.set(None);
            }
            app.layout(hwnd);
            assert_eq!(
                cached,
                snapshot(&app),
                "Cached widths must match fresh GDI measurements"
            );
            let old_height = app.rows[0].height;
            app.set_dpi(144);
            app.layout(hwnd);
            assert!(
                app.rows[0].height > old_height,
                "DPI changes must invalidate font and width caches"
            );
            app.scroll_to(hwnd, -100);
            assert_eq!(app.scroll, 0);
            app.set_dpi(96);
            SetWindowPos(hwnd, NULL, 0, 0, 220, 250, 0x14);
            document(&mut app, "prefix prefix **unbroken**");
            app.layout(hwnd);
            let bold: Vec<_> = app
                .rows
                .iter()
                .flat_map(|row| &row.parts)
                .filter(|part| app.runs[part.run].style.0 == Style::BOLD)
                .collect();
            assert_eq!(
                bold.len(),
                1,
                "A word that fits on a fresh line must not be split"
            );
            assert_eq!(bold[0].range, 0..8);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn shared_text_keeps_font_context_and_document_order() {
        let mut app = App::new();
        document(&mut app, "# same\nsame\nsame\n**same**\n");
        assert!(app.run_order.len() > app.runs.len());
        assert_ne!(
            app.run_order[0], app.run_order[1],
            "Headings need separate measurements"
        );
        assert_eq!(app.run_order[1], app.run_order[2]);
        let text: String = app
            .run_order
            .iter()
            .map(|&i| String::from_utf16(&app.runs[i].text).unwrap())
            .collect();
        assert_eq!(text, "samesamesamesame");
        assert!(
            app.engine.borrow().is_none(),
            "DirectWrite is initialized lazily during layout"
        );
    }
}
