# MDLite

A dependency-free Win32 Markdown viewer in Rust. `README.md` says what it is and `docs/design.md` how it works; this is what is needed to change it.

## Building and testing

- `.\build.ps1 -Test` runs the tests and builds `dist\mdlite.exe`. The toolchain may be project-local under `.tools\` (gitignored): `build.ps1` finds it, but a bare `cargo` needs `RUSTUP_HOME`, `CARGO_HOME` and `PATH` pointed there first.
- `cargo test --offline` and `cargo fmt -- --check` must both pass. Tests that touch the registry use a throwaway root; none may read or write the real `%APPDATA%\MDLite`.
- `.\installer\build-installer.ps1` then `.\installer\verify-installer.ps1` build the Inno Setup installer and prove install, upgrade and uninstall on this machine (needs Inno Setup 6.4 or later).
- `scripts\`, `docs\` and `examples\` are in `.gitignore`: files already tracked there stay tracked, but new ones need `git add -f`.
- Scrolling must stay pixel-exact: `a_scrolled_frame_is_pixel_for_pixel_what_a_full_repaint_draws` guards it.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- When graphify-out/graph.json exists, run `graphify query "<question>"` before your first grep, search or source-file read for a codebase question, and put this rule in every subagent prompt that explores code. Read raw files only for lines the graph or an earlier read already located. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
- A committed graphify-out/graph.json is rewritten whole on every build, so a large diff to it in a pull request is expected. Do not flag it as churn or suggest skipping the graph rebuild or its hooks.
- If `graphify` is not on PATH, run `python -m graphify` instead, with the same arguments.
