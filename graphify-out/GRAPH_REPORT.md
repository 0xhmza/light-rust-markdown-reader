# Graph Report - light-rust-markdown-reader  (2026-10-16)

## Corpus Check
- 30 files · ~52,517 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 7 file(s) not represented in the graph (top: (none) 3, .toml 1, .ico 1)

## Summary
- 680 nodes · 1812 edges · 42 communities (28 shown, 14 thin omitted)
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 281 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `bee0da21`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- window_proc
- dwrite.rs
- markdown.rs
- App
- win32.rs
- render.rs
- viewer.rs
- instance.rs
- association.rs
- theme.rs
- Handle
- Run
- Hwnd
- Deco
- Hdc
- Hit
- 2026-09-06 — three pipelines, tables, palettes, chrome, scrolling, zoom, an icon and hardening
- .save
- How MDLite works
- inspect-pe.ps1
- Control
- mdlite
- Languages and Unicode
- RTL, shaping, and combining marks
- A small window for your words
- Validation
- 2026-10-16 — sharper scrolling, a settings window that cannot overlap, an installer and an uninstaller
- MDLite
- SetScrollInfo
- DwmSetWindowAttribute
- MDLite
- .claude/CLAUDE.md
- emoji.md
- tables.md
- wheel_pixels

## God Nodes (most connected - your core abstractions)
1. `App` - 59 edges
2. `window_proc()` - 48 edges
3. `Run` - 32 edges
4. `window()` - 28 edges
5. `DestroyWindow()` - 27 edges
6. `settings_proc()` - 26 edges
7. `wide()` - 26 edges
8. `settings_window_at()` - 24 edges
9. `floater_proc()` - 22 edges
10. `document()` - 22 edges

## Surprising Connections (you probably didn't know these)
- `window_proc()` --calls--> `GetKeyState()`  [INFERRED]
  src/viewer.rs → src/win32.rs
- `window_proc()` --calls--> `ReleaseCapture()`  [INFERRED]
  src/viewer.rs → src/win32.rs
- `write()` --calls--> `wide()`  [INFERRED]
  src/association.rs → src/win32.rs
- `read_default()` --calls--> `wide()`  [INFERRED]
  src/association.rs → src/win32.rs
- `unregister_from()` --calls--> `wide()`  [INFERRED]
  src/association.rs → src/win32.rs

## Import Cycles
- None detected.

## Communities (42 total, 14 thin omitted)

### Community 0 - "window_proc"
Cohesion: 0.12
Nodes (47): a_document_too_large_to_hold_is_cut_and_says_so(), a_scrolled_frame_is_pixel_for_pixel_what_a_full_repaint_draws(), absurd_window_sizes_and_zooms_are_survivable(), adding_an_existing_palette_does_not_duplicate_it(), bench(), choose_file(), closing_the_settings_window_puts_a_previewed_palette_back(), complex_paragraphs_keep_styles_share_storage_and_relayout_on_dpi() (+39 more)

### Community 1 - "dwrite.rs"
Cohesion: 0.08
Nodes (55): add(), CachedLayout, checked(), clipped_paragraph_pixels_match_full_paint_and_bitmap_stays_small(), Cluster, ColorF, ColorRun, Com (+47 more)

### Community 2 - "markdown.rs"
Cohesion: 0.07
Nodes (48): alignments(), Block, blocks_and_fences(), Cell, cells(), closing_bracket(), closing_paren(), closing_ticks() (+40 more)

### Community 3 - "App"
Cohesion: 0.11
Nodes (25): App, Block, draw_menu_item(), fill(), Font, Frame, measure_menu_item(), menu_items_and_the_scrollbar_paint_from_the_palette() (+17 more)

### Community 4 - "win32.rs"
Cohesion: 0.07
Nodes (33): AdjustWindowRectExForDpi(), ChooseColor, ChooseColorW(), CreateWindowExW(), GetKeyState(), GetOpenFileNameW(), GetTextExtentExPointW(), GetTextMetricsW() (+25 more)

### Community 5 - "render.rs"
Cohesion: 0.08
Nodes (19): Canvas, CAUSES, COLOR, colored(), delete_font(), gdi_plus_really_rasterizes_text_in_the_requested_colour(), flat!, NON_LATIN (+11 more)

### Community 6 - "viewer.rs"
Cohesion: 0.05
Nodes (29): ADD, CANCEL, CHOICE, decode(), DEFAULT_DONE, DEFAULT_HINT, DEFAULT_READER, DEFAULT_STATUS (+21 more)

### Community 7 - "instance.rs"
Cohesion: 0.10
Nodes (25): a_missing_template_or_profile_is_not_an_error(), an_ini_beside_the_program_keeps_it_portable(), an_installed_program_keeps_its_settings_in_the_profile(), path(), prepare(), resolve(), scratch(), TEMPLATE (+17 more)

### Community 8 - "association.rs"
Cohesion: 0.14
Nodes (24): choose_default(), command(), EXTENSIONS, failed(), gone(), HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, NOT_FOUND (+16 more)

### Community 9 - "theme.rs"
Cohesion: 0.14
Nodes (14): brightness_decides_dark_from_the_background_alone(), custom_palette_cannot_replace_the_viewer_section(), every_role_round_trips_by_index(), extra_sections_become_extra_palettes(), invalid_values_keep_defaults(), overrides_are_partial_and_case_insensitive(), Palette, .ROLES (+6 more)

### Community 10 - "Handle"
Cohesion: 0.13
Nodes (18): register(), AppendMenuW(), CheckMenuRadioItem(), CreateMenu(), CreatePopupMenu(), DestroyMenu(), DragFinish(), DragQueryFileW() (+10 more)

### Community 11 - "Run"
Cohesion: 0.14
Nodes (20): Run, SETTINGS, DispatchMessageW(), DragAcceptFiles(), EnableWindow(), GetDpiForWindow(), GetMessageW(), GetSystemMetrics() (+12 more)

### Community 12 - "Hwnd"
Cohesion: 0.15
Nodes (19): controls(), floater_proc(), BeginPaint(), DrawMenuBar(), EndPaint(), GetClientRect(), GetCursorPos(), GetParent() (+11 more)

### Community 13 - "Deco"
Cohesion: 0.33
Nodes (6): Deco, Code, Grid, None, Quote, Rule

### Community 14 - "Hdc"
Cohesion: 0.17
Nodes (12): Buffer, CreateCompatibleDC(), CreateDIBSection(), DeleteDC(), DrawItem, FrameRect(), GetPixel(), IntersectClipRect() (+4 more)

### Community 15 - "Hit"
Cohesion: 0.22
Nodes (7): every_zoom_control_asks_for_what_its_label_says(), Hit, Engine, ZoomIn, ZoomOut, ZoomReset, ZOOM

### Community 16 - "2026-09-06 — three pipelines, tables, palettes, chrome, scrolling, zoom, an icon and hardening"
Cohesion: 0.13
Nodes (15): 2026-09-06 — three pipelines, tables, palettes, chrome, scrolling, zoom, an icon and hardening, Consolidation, Correctness checks, Executable size, Flicker, Measured performance, Palettes and window chrome, Palettes of one's own, and text size (+7 more)

### Community 17 - ".save"
Cohesion: 0.40
Nodes (4): a_new_palette_starts_from_the_one_on_screen_and_is_saved(), a_stale_palette_index_cannot_reach_past_the_list(), saved_settings_round_trip_without_disturbing_the_rest_of_the_ini(), WritePrivateProfileStringW()

### Community 18 - "How MDLite works"
Cohesion: 0.15
Nodes (13): Building, Color emoji, Design, How MDLite works, Icon, Installing, Keys, Limits (+5 more)

### Community 29 - "Languages and Unicode"
Cohesion: 0.15
Nodes (12): Combining marks / Tiếng Việt, Deutsch, English, Español / Português, Français, Languages and Unicode, Polski / Čeština / Türkçe, Unicode filenames and paths (+4 more)

### Community 30 - "RTL, shaping, and combining marks"
Cohesion: 0.20
Nodes (9): Deliberately decomposed characters, Khmer / Myanmar, Mixed directions, RTL, shaping, and combining marks, עברית, العربية, हिन्दी, বাংলা / தமிழ் (+1 more)

### Community 31 - "A small window for your words"
Cohesion: 0.22
Nodes (8): A long paragraph, A small window for your words, Code stays literal, Lists and tasks, Plain and predictable, Tables, Text, without the machinery, Unicode text

### Community 32 - "Validation"
Cohesion: 0.29
Nodes (5): 2026-09-05 — GDI layout and color emoji, Correctness checks, Measured performance, Reproduce, Validation

### Community 33 - "2026-10-16 — sharper scrolling, a settings window that cannot overlap, an installer and an uninstaller"
Cohesion: 0.29
Nodes (7): 2026-10-16 — sharper scrolling, a settings window that cannot overlap, an installer and an uninstaller, Graphify, Not checked, Scrolling, The installer, The settings window, Where settings live

### Community 34 - "MDLite"
Cohesion: 0.29
Nodes (7): Build, Documentation, Install, License, Make MDLite your default reader, MDLite, Run without installing

### Community 35 - "SetScrollInfo"
Cohesion: 0.50
Nodes (4): scroll_state_survives_without_a_system_scrollbar(), GetScrollInfo(), ScrollInfo, SetScrollInfo()

### Community 36 - "DwmSetWindowAttribute"
Cohesion: 0.50
Nodes (5): DwmSetWindowAttribute(), GetProcAddress(), LoadLibraryExW(), system_function(), wait_for_refresh()

### Community 37 - "MDLite"
Cohesion: 0.50
Nodes (3): Building and testing, graphify, MDLite

## Knowledge Gaps
- **19 isolated node(s):** `mdlite`, `Paragraph`, `Quote`, `List`, `Code` (+14 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 223 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **14 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `window_proc`, `dwrite.rs`, `viewer.rs`, `theme.rs`, `Handle`, `Run`, `Hdc`, `Hit`, `.save`?**
  _High betweenness centrality (0.043) - this node is a cross-community bridge._
- **Are the 20 inferred relationships involving `window_proc()` (e.g. with `.drop()` and `DefWindowProcW()`) actually correct?**
  _`window_proc()` has 20 INFERRED edges - model-reasoned connections that need verification._
- **What connects `mdlite`, `Paragraph`, `Quote` to the rest of the system?**
  _19 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `window_proc` be split into smaller, more focused modules?**
  _Cohesion score 0.12152777777777778 - nodes in this community are weakly interconnected._
- **Why does `wide()` connect `window_proc` to `App`, `win32.rs`, `render.rs`, `DwmSetWindowAttribute`, `association.rs`, `Run`, `Hwnd`, `.save`?**
  _High betweenness centrality (0.032) - this node is a cross-community bridge._
- **Are the 18 inferred relationships involving `Run` (e.g. with `CreateWindowExW()` and `DispatchMessageW()`) actually correct?**
  _`Run` has 18 INFERRED edges - model-reasoned connections that need verification._
- **Should `dwrite.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.07663828211773417 - nodes in this community are weakly interconnected._