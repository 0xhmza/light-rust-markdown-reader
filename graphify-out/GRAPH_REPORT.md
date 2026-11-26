# Graph Report - light-rust-markdown-reader  (2026-11-27)

## Corpus Check
- 33 files · ~64,719 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 7 file(s) not represented in the graph (top: (none) 3, .toml 1, .ico 1)

## Summary
- 901 nodes · 2641 edges · 53 communities (35 shown, 18 thin omitted)
- Extraction: 86% EXTRACTED · 14% INFERRED · 0% AMBIGUOUS · INFERRED: 366 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `bdf84b39`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- App
- dwrite.rs
- markdown.rs
- Tab
- win32.rs
- render.rs
- viewer.rs
- links.rs
- Handle
- theme.rs
- settings_window_at
- run
- parse_each
- read_document
- wheel_pixels
- Hit
- 2026-09-06 — three pipelines, tables, palettes, chrome, scrolling, zoom, an icon and hardening
- .save
- How MDLite works
- inspect-pe.ps1
- wide
- mdlite
- Languages and Unicode
- RTL, shaping, and combining marks
- A small window for your words
- Validation
- 2026-10-16 — sharper scrolling, a settings window that cannot overlap, an installer and an uninstaller
- MDLite
- Span
- Hwnd
- MDLite
- .claude/CLAUDE.md
- emoji.md
- tables.md
- Style
- Hdc
- RichText
- Engine
- Renderer
- TabHit
- .build_menu
- Intern
- FxHasher
- SetScrollInfo
- LineIter<'a>

## God Nodes (most connected - your core abstractions)
1. `Hwnd` - 100 edges
2. `App` - 88 edges
3. `Handle` - 85 edges
4. `window_proc()` - 74 edges
5. `Hdc` - 56 edges
6. `Palette` - 36 edges
7. `window()` - 36 edges
8. `DestroyWindow()` - 35 edges
9. `settings_proc()` - 34 edges
10. `Tab` - 32 edges

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

## Communities (53 total, 18 thin omitted)

### Community 0 - "App"
Cohesion: 0.10
Nodes (39): a_document_too_large_to_hold_is_cut_and_says_so(), a_scrolled_frame_is_pixel_for_pixel_what_a_full_repaint_draws(), absurd_window_sizes_and_zooms_are_survivable(), activate_link(), App, bench(), complex_paragraphs_keep_styles_share_storage_and_relayout_on_dpi(), document() (+31 more)

### Community 1 - "dwrite.rs"
Cohesion: 0.09
Nodes (27): add(), Cluster, ColorF, CONSOLAS, Effect, effect_add(), effect_query(), effect_release() (+19 more)

### Community 2 - "markdown.rs"
Cohesion: 0.10
Nodes (25): alignments(), blocks_and_fences(), code_delimiters_must_match_the_whole_run(), empty_and_malformed_are_safe(), heading_underlines_allow_trailing_spaces(), inline(), inline_styles_and_unicode(), languages_survive_all_inline_styles() (+17 more)

### Community 3 - "Tab"
Cohesion: 0.09
Nodes (22): Block, Deco, Code, Grid, None, Quote, Rule, Fragment (+14 more)

### Community 4 - "win32.rs"
Cohesion: 0.04
Nodes (65): copy_text(), choose_file(), ChooseColor, ChooseColorW(), ClientToScreen(), CloseClipboard(), CreateWindowExW(), DwmSetWindowAttribute() (+57 more)

### Community 5 - "render.rs"
Cohesion: 0.10
Nodes (15): CAUSES, COLOR, colored(), flat!, NON_LATIN, Pipe, .ALL, D2d (+7 more)

### Community 6 - "viewer.rs"
Cohesion: 0.03
Nodes (49): ADD, CANCEL, CHOICE, CLOSE_TAB, count_lines_and_words(), CTX_COPY_LINK, CTX_COPY_MD, CTX_COPY_PLAIN (+41 more)

### Community 7 - "links.rs"
Cohesion: 0.10
Nodes (19): a_missing_template_or_profile_is_not_an_error(), an_ini_beside_the_program_keeps_it_portable(), an_installed_program_keeps_its_settings_in_the_profile(), path(), prepare(), resolve(), scratch(), TEMPLATE (+11 more)

### Community 8 - "Handle"
Cohesion: 0.06
Nodes (55): choose_default(), command(), EXTENSIONS, failed(), gone(), HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, NOT_FOUND (+47 more)

### Community 9 - "theme.rs"
Cohesion: 0.09
Nodes (18): brightness_decides_dark_from_the_background_alone(), built_in_rmdv_presets_classify_dark_and_light_correctly(), custom_palette_cannot_replace_the_viewer_section(), derived_surface_and_ui_colors_are_valid_for_every_preset(), every_role_round_trips_by_index(), extra_sections_become_extra_palettes(), invalid_values_keep_defaults(), mix() (+10 more)

### Community 10 - "settings_window_at"
Cohesion: 0.14
Nodes (19): adding_an_existing_palette_does_not_duplicate_it(), register(), settings_save_reading_preferences_and_all_edited_palettes(), settings_window(), settings_window_at(), AdjustWindowRectExForDpi(), GetDlgItem(), GetDpiForWindow() (+11 more)

### Community 11 - "run"
Cohesion: 0.15
Nodes (19): run(), settings(), DispatchMessageW(), DragAcceptFiles(), EnableWindow(), GetMessageW(), GetSystemMetrics(), IsDialogMessageW() (+11 more)

### Community 12 - "parse_each"
Cohesion: 0.16
Nodes (15): expand_tabs(), fence(), Kind, Code, Gap, Heading, List, Paragraph (+7 more)

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

### Community 24 - "wide"
Cohesion: 0.13
Nodes (26): floater_proc(), Font, font_installed(), glyph_width_cache_is_pixel_exact_with_gdi(), measure_menu_item(), modern_fonts(), the_settings_window_never_overlaps_or_clips_at_any_scale(), ui_font() (+18 more)

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

### Community 35 - "Span"
Cohesion: 0.25
Nodes (10): Block, Block<'a>, Cell, emit_split(), fill_table_row(), plain(), plain_text(), push_cow() (+2 more)

### Community 36 - "Hwnd"
Cohesion: 0.16
Nodes (29): collect(), GetWindowThreadProcessId(), apply_settings_theme(), apply_window_frame_theme(), closing_the_settings_window_puts_a_previewed_palette_back(), Control, controls(), error() (+21 more)

### Community 37 - "MDLite"
Cohesion: 0.50
Nodes (3): Building and testing, graphify, MDLite

### Community 41 - "Style"
Cohesion: 0.14
Nodes (13): format_markdown_span(), closing_bracket(), closing_paren(), closing_ticks(), parse_inline(), parse_link_target(), push(), Style (+5 more)

### Community 42 - "Hdc"
Cohesion: 0.17
Nodes (21): fill(), rectangle(), draw_menu_item(), fill(), fill_rounded(), menu_items_and_the_scrollbar_paint_from_the_palette(), save(), BeginPaint() (+13 more)

### Community 43 - "RichText"
Cohesion: 0.17
Nodes (12): checked(), clipped_paragraph_pixels_match_full_paint_and_bitmap_stays_small(), Com, direct2d_is_the_pipeline_that_actually_draws(), effect(), for_each_cluster(), hit_testing_and_selection_highlight_work_on_shaped_paragraphs(), multilingual_wrapping_preserves_native_clusters_and_bidi() (+4 more)

### Community 44 - "Engine"
Cohesion: 0.29
Nodes (7): CachedLayout, Direct2D, direct2d(), Engine, Layout, call!, BitBlt()

### Community 45 - "Renderer"
Cohesion: 0.23
Nodes (15): ColorRun, decoration(), draw_run(), foreground(), GlyphRun, glyphs(), inline_object(), pixels() (+7 more)

### Community 47 - "TabHit"
Cohesion: 0.25
Nodes (7): TabHit, Close, CycleTheme, NewTab, OpenSettings, Select, ToggleOutline

### Community 48 - ".build_menu"
Cohesion: 0.33
Nodes (5): AppendMenuW(), CreateMenu(), CreatePopupMenu(), DestroyMenu(), SetMenu()

### Community 51 - "SetScrollInfo"
Cohesion: 0.50
Nodes (4): scroll_state_survives_without_a_system_scrollbar(), GetScrollInfo(), ScrollInfo, SetScrollInfo()

## Knowledge Gaps
- **24 isolated node(s):** `mdlite`, `MarkdownFile`, `Paragraph`, `Quote`, `Code` (+19 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 286 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **18 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `Tab`, `Hwnd`, `viewer.rs`, `Handle`, `theme.rs`, `Hdc`, `run`, `Engine`, `settings_window_at`, `Hit`, `.build_menu`, `TabHit`, `.save`, `wide`?**
  _High betweenness centrality (0.064) - this node is a cross-community bridge._
- **Are the 21 inferred relationships involving `window_proc()` (e.g. with `DefWindowProcW()` and `DestroyWindow()`) actually correct?**
  _`window_proc()` has 21 INFERRED edges - model-reasoned connections that need verification._
- **What connects `mdlite`, `MarkdownFile`, `Paragraph` to the rest of the system?**
  _24 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `App` be split into smaller, more focused modules?**
  _Cohesion score 0.0976225541763097 - nodes in this community are weakly interconnected._
- **Why does `Palette` connect `theme.rs` to `dwrite.rs`, `Engine`, `Renderer`, `viewer.rs`?**
  _High betweenness centrality (0.061) - this node is a cross-community bridge._
- **Should `dwrite.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0873440285204991 - nodes in this community are weakly interconnected._
- **Why does `Handle` connect `Handle` to `App`, `dwrite.rs`, `win32.rs`, `Hwnd`, `settings_window_at`, `RichText`, `Hdc`, `Renderer`, `run`, `.build_menu`, `wide`?**
  _High betweenness centrality (0.048) - this node is a cross-community bridge._