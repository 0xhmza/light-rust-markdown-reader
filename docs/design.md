# How MDLite works

The [README](../README.md) says what it is; this says how, and why each choice
was made. Measurements behind the claims here are in
[validation.md](validation.md).

## Rendering pipelines

Every block of text is drawn by the cheapest pipeline that can draw it correctly, and the choice escalates in one direction only:

| Pipeline | Used for | Why not the one before it |
| --- | --- | --- |
| GDI | Latin text, digits, punctuation | — |
| GDI+ | Greek, Cyrillic, CJK, Hangul, symbols | GDI+ blends glyphs and falls back between fonts more evenly |
| Direct2D | Arabic, Hebrew, Indic, Thai, Khmer, combining marks | GDI and GDI+ cannot shape these scripts or order them bidirectionally |

Colour emoji are the exception to choosing per block. A colour glyph is one indivisible cluster, so DirectWrite shapes just that cluster and draws it in place while the sentence around it keeps whichever pipeline its own text earned. A page of English with an emoji in it stays on GDI.

A document therefore usually uses more than one pipeline, and mixing is the point. Direct2D work goes through DirectWrite for shaping and an `ID2D1DCRenderTarget` bound to the window's device context, so it paints into the same window, scrollbar and clip region as the other two.

**Options → Settings** (Ctrl+,) pins one pipeline for the whole document, alongside the color and scrolling controls, and writes it to `[viewer] renderer` in the INI. Pinning never breaks a document: blocks that need shaping, bidirectional ordering or colour still escalate to Direct2D. Under the list, a note says what the mode under the pointer means, and a line beneath it costs that mode for the open document — `This document: 4 GDI, 22 Direct2D blocks.` Both refresh as you move between the choices, so the effect of a pin is visible before committing to it. Direct2D gets no warning, because nothing renders worse under it. Choosing a different pipeline re-renders the open document as soon as you press OK.

## Color emoji

Open `examples\emoji.md` to check faces, hearts, skin tones, ZWJ families/professions, keycaps, and text/emoji presentation selectors. Emoji sequences stay together when wrapping. Their sizes follow the text and monitor DPI.

Each distinct emoji sequence is split off as its own cluster, shaped once with the installed Segoe UI Emoji font, and cached, so a sentence of Latin text keeps its cheap GDI path and only the glyph that needs colour is shaped. `IDWriteFactory2::TranslateColorGlyphRun` supplies the font's COLR/CPAL color layers. A custom `IDWriteTextRenderer` draws those layers and, for whole shaped paragraphs, keeps inline code backgrounds, link colors and underlines tied to the palette, so switching themes never reshapes text. The same renderer paints through `ID2D1RenderTarget::DrawGlyphRun` when a paragraph is bound to a Direct2D device context, and through `IDWriteBitmapRenderTarget` for a single cluster sitting inside a line of GDI text, where copying the row's existing pixels first is what lets antialiasing blend with the code and quote backgrounds behind it. This uses Microsoft's documented [color-layer translation](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_2/nf-dwrite_2-idwritefactory2-translatecolorglyphrun), [DC render target](https://learn.microsoft.com/en-us/windows/win32/api/d2d1/nn-d2d1-id2d1dcrendertarget) and [GDI bitmap renderer](https://learn.microsoft.com/en-us/windows/win32/api/dwrite/nn-dwrite-idwritebitmaprendertarget).

Coverage and artwork depend on the installed Windows font. Unsupported sequences retain the font's fallback representation; country flags may appear as regional indicator letters. Arbitrary SVG/bitmap/COLRv1-only color fonts are outside this renderer's scope. The shipped viewer bundles no font or emoji images.

## Markdown support

The handwritten parser handles ATX and setext headings, bold, italic, combined emphasis, strikethrough, inline code, fenced and indented code, bullet and numbered lists, task markers, blockquotes, horizontal rules, escaped punctuation, links, image labels, and pipe tables. Text wraps to the window width, including long code lines and table cells, and follows the monitor DPI.

Tables follow the GitHub form: a header row, a delimiter row that also sets per-column alignment (`:---`, `:---:`, `---:`), then body rows. Columns are as wide as their widest cell and shrink proportionally when the table is wider than the window, wrapping inside each column instead of spilling off the page. Header cells are bold, short rows are padded rather than dropped, a backslash escapes a literal pipe, and cells keep their inline markup. A line of pipes with no delimiter row under it stays ordinary text. Open `examples\tables.md` to see all of this.

This is intentionally a small Markdown subset, not a CommonMark implementation. Source line breaks are preserved. HTML, math, reference links, and other extensions remain literal text. Nested block structures are simplified; a table cell holds one line of inline markup, not block content. Links are display-only; images are labels, not decoded bitmaps. No content is fetched from the network. There are no editing, selection, search, or printing controls.

Input supports UTF-8 and BOM-marked UTF-16 LE/BE. Invalid encodings and binary files show an error without replacing the current document. Parsing and layout are synchronous, so very large documents can briefly block the window when opening or resizing.

## Limits

A viewer opens files it did not write, so the ones that would otherwise take it down are bounded, and every limit is visible rather than silent:

| Limit | Value | Why |
| --- | ---: | --- |
| File | 64 MiB | Read in one piece; larger files are refused with a message |
| Document | 1,000,000 blocks | Every block costs a row and a little heap. A file of nothing but newlines would otherwise be an out-of-memory abort; past this the footer says *only the first million shown* |
| Block | 8,192 bytes | About fourteen hundred words on one unbroken line. Shaping a single run of mixed scripts costs time quadratic in its length: 200,000 characters took 40 seconds before this cap and 1.1 seconds after. The break lands on a space where there is one, so a long paragraph reads as if it had wrapped |
| Table | 64 columns | Cells are columns times rows, so a malformed delimiter row is where a file turns into gigabytes |
| Palettes | 64 | As many as the Options menu has command ids for |
| Emphasis nesting | 12 deep | Beyond it the markup is left as text |
| List nesting | 16 deep | Deeper indents render at the last level |

Ordinary text is nowhere near any of these: the caps exist for files that are machine-made, corrupt, or hostile. Everything above is covered by tests, including three thousand fuzzed documents and a two-million-line file.

## Themes

Copy `mdlite.ini` beside the executable. **Every section except `[viewer]` is a palette, named by the section**, and a palette may list only the roles it wants to change:

```ini
[Nord]
background=#2E3440
foreground=#E5E9F0
accent=#88C0D0
```

That palette appears in the Options menu and in the settings window straight away. Colors use `#RRGGBB`, and the seven roles are `background`, `foreground`, `muted`, `accent`, `code_background`, `code_foreground` and `rule`. The file ships with Dark, Light, Nord, Solarized Dark, Solarized Light, Gruvbox and High Contrast; Ctrl+D steps through whatever it defines.

Options → Settings (Ctrl+,) edits the palette that is showing. Each role is a swatch; clicking one opens the standard Windows color picker, and the page behind the window updates as you go, so a color is judged against real text rather than a preview strip. OK writes the palette back to its own section — nothing else in the file is touched — and Cancel puts back exactly what was there before.

**Add** makes a palette of your own. It starts as a copy of the one showing, so you adjust rather than start from black, and it takes the name you type beside it (or `Custom` if you type nothing). A name that already exists selects that palette instead of making a second one, and INI syntax cannot be smuggled into a section name. The new palette appears in the Options menu and in the list straight away, and is written to the file as its own section when you press OK.

The text size follows, between 50% and 400%. Everything the viewer measures goes through one scale, so headings, code, indents, table columns and the scrollbar all grow together, and your place in the document is kept across the change. There are four ways to reach it, and they are the same command underneath:

- **The footer**, at the near end: `−  100%  +`. The percentage is a button too — it goes back to 100%.
- **The keyboard**: Ctrl with the wheel, Ctrl+plus, Ctrl+minus, Ctrl+0. Both the top-row and numeric-keypad keys.
- **The Options menu**, where the shortcuts are written next to them.
- **The floating control**, if you want one: the same three cells over the document itself, in the far corner of the text — the top right, or the top left when the document reads right to left. One Arabic word in a table does not turn an English page around; it moves when most of the document does. Turn it off with the **Zoom control** box in the settings window or `[viewer] floating=off`.

The floating control is a child window rather than something painted onto the page, which is what keeps scrolling from having to repaint the document underneath it every frame. The size is remembered in `[viewer] zoom`.

`[viewer] theme=` names the startup palette, `[viewer] renderer` pins a pipeline, and `smooth` and `zoom` hold the scrolling and text-size choices. The palette, pipeline and text size in use are written back when the viewer closes, so it opens the way you left it. Menu changes last for the current session; only the settings window writes to the file, and a read-only folder keeps the choice for the session instead. Ctrl+F5 rereads the file, picking up palettes you added while the viewer was open. Missing files, unknown keys, and invalid colors fall back to defaults, and a palette that names only one role keeps readable defaults for the rest.

The whole window follows the palette, not just the document. The title bar takes the background and foreground colors through the documented DWM attributes (Windows 11; older versions ignore them and keep the system caption). The menu bar and its popups are owner-drawn from the palette, with the accent color behind the highlighted item. The scrollbar is drawn by the viewer rather than by Windows, because a system scrollbar can only follow the system's light or dark theme, and matching an arbitrary palette needs the exact color — the alternative was the undocumented dark-mode theme ordinals, which this project does not use. The scroll position stays published through the normal `SetScrollInfo` state.

The file dialog and the settings window keep the system's own appearance: they are standard Windows dialogs, and repainting them would trade familiarity and accessibility for a few matching pixels.

## Scrolling

The wheel, the arrows and the Page keys ease into place over about a fifth of a second, asking Windows for a finer timer tick while a scroll is running and giving it back the moment one finishes, so an idle window still costs nothing. Notches during a scroll add to where it is already going rather than restarting it. Dragging the scrollbar thumb is never animated: it is direct manipulation, and easing between the pointer and the page only feels like lag. Uncheck **Smooth scrolling** in the settings window, or set `[viewer] smooth=off`, for the instant jump.

The scrollbar itself is drawn by the viewer in the palette's own colours. Setting a scroll range is what makes Windows add `WS_VSCROLL` and put its own bar beside it, so the viewer hides that again whenever the range changes — never on a position update, which is what scrolling sends. The window still publishes its scroll position, range and page through the normal `SetScrollInfo` state.

Every frame is composed in a client-sized back buffer and reaches the window in one blit, so nothing is ever seen half-painted. A scroll copies the pixels that stay with `ScrollWindowEx` and repaints only the strip that exposes plus the part of the scrollbar thumb that moved; invalidating the whole bar instead would make the update region's bounding box cover the window, and every frame would redraw the page.

## Status bar

The footer reports the document's size, line and word counts, and how long the last open took, parsing and layout included. On the right it names the pipeline in use: `Direct2D + mixed` when the choice is automatic, or `GDI (pinned)` when one is pinned, so the setting is visible rather than inferred.

When a pinned pipeline cannot draw everything, the footer says so beside the name — `22 blocks still need Direct2D`. That is the honest consequence of the choice, and it is reported where the reader is rather than buried in a dialog. Clicking the name explains the rest: how many blocks each pipeline takes, whether the choice was automatic or pinned, and which of right-to-left text, complex-script shaping, colour glyphs or non-Latin characters was detected.

## Icon

`assets\mdlite.ico` is drawn by `scripts\make-icon.ps1`: a rounded tile in the viewer's own blue carrying a heading bar and two lines of text. Every size is rendered at its own resolution rather than downscaled from one large image, so 16 pixels stays as sharp as 256. The small sizes are stored as DIBs, which every part of the shell can read, and the large ones as PNG, which is what keeps nine sizes inside 20 KB.

The toolchain here ships no resource compiler, so `build.ps1` puts the icon into the built EXE afterwards with `BeginUpdateResource` / `UpdateResource`, the documented API for exactly that. The window then asks for the sizes it needs — `SM_CXSMICON` for the title bar, `SM_CXICON` for the task switcher — instead of letting Windows downscale one image for both. A build made with `cargo build` alone simply has no icon and falls back to the system default.

For the native UI smoke test and executable inspection:

```powershell
.\scripts\smoke.ps1
.\scripts\inspect-pe.ps1
```

The smoke test runs a copy of the app offscreen, captures its light/dark/custom palettes and color emoji under `.tools\qa`, checks scrolling beyond 65,535 pixels and reload position, exercises resizing and theme changes, and reports resource counts and timings. It checks actual colored pixels and benchmarks both repeated and unique document lines. Results are saved as `.tools\qa\smoke-result.json`. It closes its test process afterward.

MSVC builds statically link the CRT. The interning tables keep the standard hasher rather than a faster, smaller one: documents are untrusted input, and a weak hash there would turn a crafted file into quadratic work.

## Design

See [validation.md](validation.md) for test coverage, before-and-after timings, and colour-emoji screenshots.

- A blocking `GetMessageW` loop uses no timers or continuous rendering.
- Parsing happens on open/reload. Text is converted to UTF-16 once and reused.
- A bounded temporary index shares repeated text spans and is freed after parsing.
- Layout caches full-span widths and reuses them when they fit the new line width. Fonts and widths are invalidated on DPI changes.
- Queued resize work is coalesced so an already-handled resize does not trigger a second layout.
- Painting binary-searches the visible rows and respects the invalidated paint region.
- Scrolling copies existing pixels with `ScrollWindowEx` and paints the exposed strip.
- GDI's stock brush avoids allocating brushes during painting.
- DirectWrite, Direct2D and GDI+ load lazily from System32, and only when a block needs them. A document of plain Latin text touches none of them.
- Shaped paragraph layouts are cached and shared between identical paragraphs; the cache is bounded and keyed by text, size and width. Emoji clusters are interned across the whole document, so a page of identical emoji is shaped once.
- Direct2D binds only the visible slice of a paragraph, so a long paragraph never allocates a document-height surface.
- GDI+ fonts mirror the GDI font already selected into the device context, so one set of measurements serves both pipelines.
- Table columns are measured once per table from cached span widths, then laid out like any other text.
- Menu painting reads its font and palette from a small thread-local, because Windows asks for it from inside `SetMenu` and `DrawMenuBar`, which the viewer calls while its own state is borrowed.
- The scrollbar costs two fills and no window of its own; scrolling copies pixels around it rather than through it.
- A scroll animation runs on one timer that exists only while it is moving, and each frame repaints the strip that ScrollWindowEx exposes, not the page.
- The back buffer is the size of the client area, not of the document, and is rebuilt only when that size changes.
- No GPU swap chain, offscreen full-document bitmap, file watcher, or background worker.

The approach uses the documented [Windows GDI text API](https://learn.microsoft.com/en-us/windows/win32/api/_gdi/) and [per-monitor DPI messages](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged). Minimal architecture does not imply a measured claim of being the fastest or smallest possible viewer.

`src/markdown.rs` contains the parser, `src/theme.rs` the named palettes and INI loader, `src/viewer.rs` layout, painting, the status bar and the settings window, `src/render.rs` the pipeline choice and the GDI+ backend, `src/dwrite.rs` the DirectWrite and Direct2D backend, and `src/win32.rs` the basic ABI bindings.

Two ignored tests exist for working on layout rather than asserting it: `cargo test --offline -- --ignored --nocapture preview` renders the example documents and the settings window off-screen into bitmaps in `%TEMP%`, and `... --ignored --nocapture bench` reports where parsing, first layout and relayout time goes on a large document.

## Building

Install stable Rust with a Windows toolchain. MSVC builds need the Visual C++ build tools and Windows SDK; the GNU toolchain is also supported. `build.ps1` runs the tests, builds without network access, draws and embeds the icon, and copies the EXE to `dist`. It also recognizes a project-local toolchain under `.tools`.

It builds the `small` profile by default: `opt-level="z"` on top of full LTO, one codegen unit, abort-on-panic and stripped symbols. That is 402 KiB against 463 KiB for `release`, and the `bench` test measured no difference in layout time between them, so the smaller one is what ships. `.\build.ps1 -Profile release` builds the speed-first profile if you want to compare.

`scripts\smoke.ps1` drives a copy of the application off-screen and checks palettes, scrolling, reload position, resource counts and timings; `scripts\inspect-pe.ps1` reports the executable's size and imports. Two ignored tests exist for looking at output rather than asserting it: `cargo test --offline -- --ignored --nocapture preview` renders the examples and the settings window to bitmaps in `%TEMP%`, and `... --ignored --nocapture bench` reports where parsing and layout time goes.

## Keys

| Action | Shortcut |
| --- | --- |
| Open | Ctrl+O, or File → Open |
| Reload document, retaining scroll position | F5 |
| Next palette | Ctrl+D, or pick one from Options |
| Bigger / smaller text | Ctrl+wheel, Ctrl+plus / Ctrl+minus, Ctrl+0 to reset, or the Options menu |
| Settings | Ctrl+, or Options → Settings |
| Reread the INI | Ctrl+F5 |
| Scroll | Wheel, Up/Down, Page Up/Down, Space / Shift+Space |
| Drag the scrollbar | Click the thumb, or the track to page towards it |
| Start / end | Home / End |
