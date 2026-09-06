# Validation

## 2026-09-06 — three pipelines, tables, palettes, chrome, scrolling, zoom, an icon and hardening

56 Rust tests pass, plus three ignored ones kept for looking at output rather than asserting it, and the offscreen Windows UI smoke test passes. The shipped binary is 440,832 bytes (431 KiB), built with Rust 1.90.0, the x86_64 GNU Windows toolchain, and the `small` profile, icon included.

### What each pipeline was shown to do

Each backend is verified by the pixels it produces, not by the fact that it was called:

- **Direct2D**: `direct2d_is_the_pipeline_that_actually_draws` requires the device-context render target to be created on this system. `clipped_paragraph_pixels_match_full_paint_and_bitmap_stays_small` then draws a mixed Arabic / Hindi / Latin / emoji paragraph into a DIB, requires more than a thousand non-background pixels, requires accent-coloured link pixels and warm emoji pixels, requires a clipped repaint to match the full repaint pixel for pixel, and requires the Direct2D target to have survived the paint. A drawing failure drops the target, so those two assertions together mean the glyphs came from Direct2D.
- **GDI+**: `gdi_plus_really_rasterizes_text_in_the_requested_colour` creates a canvas on a DIB, mirrors the selected `HFONT`, draws a word in pure red, and requires both solid red pixels and blended edge pixels.
- **GDI**: the existing layout tests measure and place every fragment through `GetTextExtentExPointW` and `TextOutW`.

### Palettes and window chrome

Every section of `mdlite.ini` except `[viewer]` is now a palette, and the whole window follows the chosen one.

- Parsing: extra sections become extra palettes; a palette naming one role keeps defaults for the rest; `theme=` may name a palette defined further down the file; invalid colours and unknown keys are ignored. Dark is decided from the background's weighted brightness, checked against black, white and Solarized's `#002B36`.
- Menu: every item is owner-drawn. A test drives `measure_menu_item` and `draw_menu_item` against a DIB and requires the palette's background behind a normal item, the accent behind a selected one, label pixels over both, and a separator that measures to zero width and paints a rule. Menu painting reads its font and palette from a thread-local, because `SetMenu` and `DrawMenuBar` ask for them while the viewer's own state is borrowed — doing it through the state made every menu item measure as zero and vanish, which is what the first attempt did.
- Scrollbar: drawn by the viewer, so it takes the palette's exact colours. A test checks the thumb sits at the top at rest, on the bottom at the end, never shrinks below a grabbable size, disappears when the document fits, maps a drag back to the scroll it came from, and pages towards a click on the track. The window keeps publishing its scroll state through `SetScrollInfo`, which is what the smoke test reads.
- Caption: the documented DWM attributes take the palette's background and foreground; verified on screen, not only through `PrintWindow`, for a dark and a light palette.
- Settings: the palette list, the seven swatches, and the standard colour picker. Cancel, Escape and the close box all put back the palette and the individual colour that the live preview changed; OK keeps them and writes the palette to its own section, leaving comments and the other palettes untouched.

### Scrolling

- Only one scrollbar is on screen. `SetScrollInfo` brings the system bar back whenever the range changes, so the viewer hides it again after every update; before that fix both bars were visible side by side.
- Smooth scrolling closes a quarter of the remaining distance per frame on an 8 ms timer, asking `timeBeginPeriod` for a finer tick only while a scroll is running. A test drives the frames directly and requires the position to move towards the target every frame, never past it, to settle in 4 to 60 frames, to honour a new destination mid-glide, and to respect the document's limits at both ends. Measured live through `GetScrollInfo`, one Page Down produced 16 distinct positions with steps easing from 266 px down to 2 px.
- Idle CPU over a one-second sample stayed at zero: the timer exists only while something is moving.
- The smoke test asserts scroll positions the instant a key is sent, so it now runs with `smooth=off` in its own INI; the animated path is covered by the Rust tests and the live measurement above.

### The settings note

The one fixed warning under the pipeline list was wrong for Direct2D, which degrades nothing, and never changed as the selection moved. Each mode now carries its own note, refreshed on selection, and a test reads the control's text back and requires it to match the selected mode.

### Flicker

The window flickered on every scroll. Three causes, each fixed and checked:

- `SetScrollInfo` adds `WS_VSCROLL` and shows a system scrollbar whenever the range changes, and the range was being resent on every animation frame. Each one showed the bar and the viewer hid it again, changing the client width twice a frame and re-laying out the document. Scrolling now sends the position alone, and only a real range change hides the bar. Measured live: `WS_VSCROLL` absent, the client width unchanged across twelve scrolls, idle CPU zero.
- Painting went straight to the window, so a frame was visible as it was built: background fill, then rows, then the scrollbar over the top. Frames are now composed in a client-sized back buffer and blitted once. A test paints a mixed Arabic and Latin document and reads the buffer's own pixels back, so the buffer is proved to hold a full frame, Direct2D text included.
- Every frame invalidated the full-height scrollbar strip, and the bounding box of that plus the newly exposed text covered the whole window, so each frame redrew the page. Only the part of the thumb that moved is invalidated now; a test requires a 60 px scroll to ask for less than half the window's height, and the whole bar only when there is no previous thumb to compare against.

Hiding the bar sends `WM_SIZE` from inside the call, which the viewer used to mistake for the reader resizing the window and answer with another full layout. A thread-local flag marks that window, so the resize is ignored.

### Consolidation

The same 42 tests and the same smoke run cover the source after a pass to remove duplication: 6,729 lines to 6,434, with no behaviour changed and the previews rendering pixel for pixel as before.

- The off-screen surface the viewer composes frames in is now `win32::Buffer`, and the four hand-rolled copies of the same `CreateDIBSection` scaffolding in the tests use it instead. The two preview tests share one bitmap writer.
- `SetWindowTheme` and `WM_VSCROLL` were left behind by the drawn scrollbar and are gone. The `dead_code` allowance that was hiding them has been removed from the bindings module, and the handful of declarations only the tests use are marked `#[cfg(test)]`, so the lint stays useful.
- The interface font and the window-class registration each had two copies; both are now one function, which also gives the settings window the application icon.
- Seven COM calls checked their status by hand; a small `ok` helper carries the failure on `?`.
- Painting the row decorations was four near-identical fills and is now one band.
- Two things now cost nothing when they cannot be seen: the footer, which formatted and measured its strings on every frame, and the scrollbar, both skipped unless the clip reaches them. Rows are only sorted when the document actually contains a table.

### Reporting the pipeline choice

Pinning a pipeline worked, but nothing said so: the footer named the busiest pipeline in use, so pinning GDI on a document containing Arabic still read `Direct2D + mixed` and looked as though the setting had been ignored. Driving the dialog from outside the process confirmed the setting was applied, saved to the INI and the document re-rendered; only the reporting was wrong.

- Blocks are now counted by what their content needs rather than by the pipeline they took, so the spread under any setting is a sum over eight buckets and needs no reparse. The footer, the dialog and the click-through explanation all read from it.
- The footer names the choice: `GDI (pinned)` or `Direct2D + mixed`, and when the pinned pipeline cannot draw everything it says what that costs, in the foreground colour beside the muted summary: `22 blocks still need Direct2D`. Checked live by pinning GDI on `complex-scripts.md` through the dialog.
- The settings window shows the same figure for whichever mode is highlighted, refreshed on selection, so the effect of a pin is visible before OK. A test reads the control's text back and requires it to match.
- Tests cover the wording in both directions: pinning Direct2D can never cost anything and produces no warning, pinning GDI on a document with one Arabic line reports exactly one block, and the singular reads `1 block still needs Direct2D`.

### Palettes of one's own, and text size

- **Adding a palette.** A name and an Add button beside the palette list. The new palette copies the one on screen, so it is adjusted rather than built from black. Tests cover the model: an existing name selects that palette instead of duplicating it and does not overwrite its colours, an empty name yields `Custom` then `Custom 2`, `  [Dark]=x;
evil  ` becomes `Darkxevil` so nothing can be smuggled into a section header, and what was added survives a round trip through the file. Driven live through the dialog: typing `Sepia` and pressing Add grew the list to eight entries with the new one selected, and OK wrote `theme=Sepia` and a `[Sepia]` section.
- **Text size.** Ctrl with the wheel, Ctrl+plus, Ctrl+minus and Ctrl+0, from 50% to 400%, also on the Options menu. Everything the viewer measures goes through one `px`, so a test checks that padding scales exactly, rows grow, the document gets taller, the reader keeps their place to within 2% of the document, both limits clamp, and returning to 100% reproduces the original padding and content height byte for byte.
- The palette, pipeline, scrolling and text size are written back when the window closes, so the viewer opens the way it was left.

### Executable size

`opt-level = "z"` on top of the existing full LTO, single codegen unit, abort-on-panic and stripped symbols gives 411,648 bytes against 474,624 for the speed-first profile, a 13% reduction. The `bench` test measured 505 ms of layout under `release` and 485 ms under `small` on the same document, which is inside this machine's noise, so `build.ps1` now ships `small` by default and `-Profile release` builds the other. The smoke test passes on the shipped build.

One size lever was left alone deliberately: the interning tables still use the standard hasher. Replacing it with a small multiply-xor hash would drop the `bcryptprimitives` import and some code, but documents are untrusted input and a weak hash there turns a crafted file into quadratic work.

### The icon

Drawn rather than sourced, so nothing is checked in that the repository cannot rebuild: `scripts\make-icon.ps1` renders nine sizes, each at its own resolution, and assembles the `.ico` by hand. Small sizes are DIBs for the shell paths that cannot read PNG; large ones are PNG, which holds the whole icon to 20,688 bytes.

There is no resource compiler on this toolchain, so `build.ps1` embeds the icon with `BeginUpdateResource` / `UpdateResource`. Checked by extracting it back out of `dist\mdlite.exe` with `ExtractAssociatedIcon`, and by running the viewer and reading `WM_GETICON` back: before the change both returned 0, meaning the title bar was downscaling the 32-pixel image; the window now loads `SM_CXSMICON` and `SM_CXICON` explicitly and both return real handles. `LR_SHARED` leaves those two owned by Windows, which held the GDI object count at 42 rather than 45, flat across the smoke test's 50 theme and resize cycles either way.

The EXE grew from 411,648 to 433,152 bytes. That is the icon, and it is the one place this round where size went up on purpose.

### Reaching the zoom

Four ways to the same command: the footer cluster, the keyboard, the Options menu, and a control floating over the document.

- **The floating control is a child window**, not something painted onto the page. Drawing it with the document would put a fixed box inside the scrolled region, and the bounding box of that plus the strip a scroll exposes covers the window — the whole page would repaint on every animated frame. As a child under the parent's `WS_CLIPCHILDREN`, scrolling never touches it.
- **Which corner.** The far end of the text: top right, or top left when the document itself reads right to left. The first rule tried was "the document contains right-to-left text", which put the control over the heading of an English page that mentions one Arabic word in a table. It now asks whether most blocks read that way. A test drives an English page, an Arabic page, and an English page with one Arabic word, and requires the control on the right, the left, and the right.
- **A bug only a live click found.** The command ids and the control's cells were two separate lists that had drifted: clicking `+` reset the zoom and clicking `−` zoomed in. Driving the child window with `WM_LBUTTONDOWN` and watching the document's height through `GetScrollInfo` showed it. The two lists are now one table carrying the id, the action, the glyph and the menu label together, and a test requires each cell to ask for what its own label says. Re-driven afterwards: 1692 pixels of document became 2011 after two clicks on `+`, 1431 after three on `−`, and exactly 1692 again after clicking the percentage.
- The footer's cluster is hit-tested from rectangles measured while it paints, alongside the pipeline name; a test checks the three cells sit in order, answer clicks at their centres, and never overlap the name.

### Standing up to bad input

The viewer opens files it did not write, so this round went looking for the ones that would take it down. Each limit below was measured before it was chosen.

- **A single line of mixed scripts was quadratic.** 12,500 characters laid out in 172 ms, 100,000 in 6.4 seconds, 200,000 in **40 seconds** — a 200 KB file froze the window. Measuring the shapes separately showed ordinary text is linear (120,000 characters of Arabic in 90 ms, 240,000 of English in 653 ms); only a line that changes script at almost every character is quadratic, because each change is another shaping run to fall back on. Blocks are now capped at 8,192 bytes, split at a space where there is one, which turned that 40 seconds into 1.1. The whole hostile-document suite went from 17.4 seconds to 2.1.
- **A malformed table was an out-of-memory waiting to happen.** Cells are columns times rows, so a delimiter row of a million pipes multiplies. Capped at 64 columns: a 20,000-column table went from 40,002 rows and 166 ms to 130 rows and 8 ms.
- **A file of nothing but newlines was an abort.** At roughly 230 bytes of state per line, a 64 MiB file of short lines is gigabytes of rows, and with `panic = "abort"` an allocation failure is a dead process. Documents are cut at a million blocks and the footer says *only the first million shown*. Checked end to end: a 66 MB, three-million-line file opens in 8.8 seconds, holds 395 MiB, scrolls to the end, and still answers messages.
- **The footer stopped overlapping itself.** That truncation notice made the summary long enough to run underneath the pipeline name. The summary now takes whatever room is left between the zoom cluster and the name, and is elided.
- **Panic paths.** Every `unwrap` in non-test code was audited; the six that remain are each guarded by the check immediately above them. `save` no longer indexes the palette list with a possibly stale index, the shaped-text fallback no longer assumes a font exists, and the zoom lookup no longer unwraps a search its own guard already performed. A swatch repaint takes the state with `try_borrow`, so a redraw arriving at the wrong moment cannot panic instead of skipping a frame.
- **A dangling font.** A DPI change deletes the menu font, but the thread-local the menu paints from still held the old handle until something rebuilt the menu. The DPI handler now rebuilds it and republishes the palette before anything can paint with a freed object.
- **Bounded everywhere else**: palettes at 64, so every one has a command id; DPI clamped to 96–960 so the scaling arithmetic cannot overflow; a font Windows refuses falls back to the stock UI font rather than none.

Three new tests hold the line: ten hostile documents that must lay out, paint and scroll inside 20 seconds each; three thousand fuzzed documents from a fixed seed, which must not panic, must respect every cap, and must not invent characters; and six window sizes down to 1×1 crossed with the zoom extremes, painting at each.

### Measured performance

Wall-clock observations from `scripts\smoke.ps1` on this machine, against the previous release's numbers for the same generated documents and window sizes:

| Operation | 2026-09-05 EXE | Updated EXE |
| --- | ---: | ---: |
| Open/reload 10,000 repeated lines | 49.8 ms | 73.7 ms |
| Resize repeated-line document | 21.8 ms | 42.1 ms |
| Open/reload 10,000 unique lines | 749.2 ms | 599.1 ms |
| Resize unique-line document | 20.0 ms | 46.8 ms |
| Open/reload 10,000 complex-script lines | — | 54.5 ms |
| Startup and document readiness | 149.3 ms | 197.4 ms |
| Working set after large-document checks | 23.3 MiB | 54.2 MiB |

Run-to-run spread on this machine is wide, and it widened further while this work was finishing: repeat runs of the unique-line reload ranged from 604 ms to 2,280 ms with the same binary. The `bench` test, which parses and lays out without painting anything and whose code none of the flicker work touches, moved from 587 ms to about 1,000 ms of layout over the same period with 42% of the CPU going to other processes, so the later numbers measure the machine rather than the change. Only the order of magnitude is meaningful. The working set grew because Direct2D and GDI+ are now resident once a document needs them. GDI objects rose from 22 to 34 for the menu font, the menu background brush and the owner-drawn items, and then stayed flat across 50 theme and resize cycles and across the multilingual passes.

The ignored `bench` test separates the phases on a 10,000 unique-line document: parsing 41 ms, first layout 587 ms, relayout 20 ms. First layout is dominated by GDI text measurement of runs that nothing can share; relayout reuses the cached widths. A 2,000-row table parses in 5.8 ms and lays out in 2.4 ms.

An earlier version of this change sent any block containing an emoji through Direct2D paragraph layout. The smoke test caught what that cost — unique-line reload 2,916 ms and resize 3,493 ms — because every line became its own text layout. Colour glyphs are now split into single clusters instead, and resize returned to 50 ms and then to 24.5 ms once the pipeline classifier was reduced to a single pass with an ASCII fast path.

### Correctness checks

- Tables: header and delimiter parsing, per-column alignment, escaped pipes, short rows padded rather than dropped, inline markup inside cells, and pipes without a delimiter row staying literal text. After layout: one separator per row, a heavier rule under the header, cells of a row sharing a top edge, columns that never overlap, right-aligned cells ending at their column edge, centred cells indented from it, and a table far wider than the window staying inside the page margins.
- Pipeline choice: GDI for Latin, GDI+ for Greek/Cyrillic/CJK/Hangul, Direct2D for Arabic/Hebrew/Indic/Thai and combining marks; a colour glyph staying inside its GDI sentence as its own cluster; and pinning moving every block it can render while blocks needing shaping still escalate.
- Settings window: the radio matching the pinned pipeline, the note describing it, and the smooth-scrolling checkbox. Saving writes only `theme` and `renderer` back to the INI, leaving comments and colours in place, and reloads to the same choice. In a live run the window opens owner-modal, disables the main window, and Cancel restores it.
- Status bar: line, word and pipeline counts for a mixed document; the reported reasons; the wording of the click-through explanation for automatic and pinned choices; the measured label hit-testing only over itself; and the viewport shrinking by the footer height so scrolling and painting stop above it.
- Everything checked in the previous entry below, unchanged: encodings, wrapping without splitting surrogates, cached widths matching fresh measurements, DPI invalidation, scrolling beyond 65,535 pixels, F5 retaining position, emoji sequences, and `cargo fmt -- --check`.
- GDI objects were unchanged across 50 theme/resize cycles and across the multilingual passes. Imported DLLs are still Windows system libraries only; gdiplus.dll, d2d1.dll, dwrite.dll and dwmapi.dll are loaded on demand.

Release SHA-256: `C90AE5BE7B596F93D0A4326AE0F09019F1C23523FCD470E021A656CF981A9925`.

## 2026-09-05 — GDI layout and color emoji

The updated release passes 14 Rust tests and the offscreen Windows UI smoke test. The binary is 358,400 bytes (350 KiB), built with Rust 1.90.0, the x86_64 GNU Windows toolchain, and the release profile.

### Measured performance

Wall-clock observations on this machine, using the same generated documents and window sizes before and after the change:

| Operation | Previous EXE | Updated EXE |
| --- | ---: | ---: |
| Open/reload 10,000 repeated lines | 1,733.5 ms | 49.8 ms |
| Resize repeated-line document | 7,807.4 ms | 21.8 ms |
| Open/reload 10,000 unique lines | 4,249.8 ms | 749.2 ms |
| Resize unique-line document | 8,386.2 ms | 20.0 ms |
| Startup and document readiness | 275.9 ms | 149.3 ms |
| Working set after large-document checks | 20.7 MiB | 23.3 MiB |

These are individual local measurements, not a universal speedup claim. The documents include styled text, CJK characters, and emoji. The resize measurements include processing the resize and synchronizing layout completion; the updated viewer also avoids duplicate queued layouts. DirectWrite color rendering adds some memory while supporting emoji that the previous viewer rendered in monochrome.

Measured idle CPU time was zero over a one-second sample. GDI objects remained at 22 before and after 50 theme/resize cycles. The executable imports only Windows system DLLs; it loads the system DirectWrite DLL on demand for emoji.

### Correctness checks

- Dark, light, and custom INI background pixels; ordinary-text screenshots matched the previous build byte for byte.
- Actual yellow/orange emoji pixels in both an in-memory native renderer test and app screenshots.
- Emoji skin modifiers, family/profession ZWJ sequences, regional pairs, keycaps, and VS15/VS16 preservation.
- Native Unicode wrapping without splitting surrogate pairs, cached widths matching fresh GDI measurements, and DPI cache invalidation.
- Keeping a word together when it fits on the next line.
- Full-range scrolling beyond 65,535 pixels, Home/End, and F5 retaining the reading position.
- Encoding errors, malformed Markdown, partial INI overrides, exact code-span delimiters, and literal brackets before links.
- `cargo fmt -- --check`.

Country flags appear as regional indicator letters with the installed Windows font. Rendering coverage follows that font; the viewer does not bundle replacement artwork.

## Reproduce

```powershell
.\build.ps1 -Test
.\scripts\smoke.ps1
.\scripts\inspect-pe.ps1
```

The smoke test saves metrics and screenshots under `.tools\qa`. Checked previews: [dark emoji](emoji-dark.png), [light emoji](emoji-light.png).

The 2026-09-05 release SHA-256 was `095B5AFA98361DD47B42C1604C0FD40C309373292FC5BD3D99072886BC69B45C`.
