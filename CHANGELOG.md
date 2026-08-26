# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-08-26

### Added

- **Scroll-follow (initial view position)** — new `scroll_follow` config
  key controls where the popup opens when the source pane is scrolled
  up. Three modes:
  - `off` (default) — always open at the bottom of the captured text;
    unchanged behavior; no extra socket calls.
  - `offset` — read the source pane's scroll offset (`pane.get`) and
    anchor the popup on the line the source viewport's bottom edge
    shows. Exact when nothing in the scrolled region wraps; drifts
    upward when long lines wrap in the source (the popup renders one
    unwrapped line per row, the source wraps).
  - `content` *(experimental)* — additionally read the source's current
    viewport text (`pane.read source=visible`) and fingerprint-match
    distinctive short lines back into the capture. Sidesteps the wrap
    drift; falls back to `offset` when no unique anchor is found. Most
    faithful to "what's on screen", but content-dependent. The matching
    heuristic may change in a future release.
  Source scrolled above what was captured (`Lines(N)` where offset ≥ N)
  clamps to the oldest available line with a footer hint.

### Fixed

- **Config loader silently ignored all config keys.** The loader parsed
  `config.toml` with `str::parse::<toml::Value>()`, but in the `toml`
  1.x crate `Value: FromStr` parses a single TOML value expression, not
  a full document — so every config file starting with a `#` comment
  failed to parse and fell back to built-in defaults. Switched to
  `toml::from_str(&text)`. This unblocks all config keys (`labels`,
  `scroll_follow`, theme overrides, profiles), not just scroll-follow.

### Limitations

- **Copy mode is not supported by scroll-follow.** Herdr's copy mode
  keeps its own scroll state separate from the terminal's normal scroll;
  exiting copy mode (required to trigger the flash keybind) resets the
  terminal scroll to the bottom before the popup can read it. So
  scroll-follow only tracks mouse-wheel / keyboard scroll, not
  copy-mode scroll. Terminal-level behavior, not workable from the
  plugin. Documented in `doc/config-reference.md`.

## [0.2.0] - 2026-08-22

### Added

- **Block (rectangular) selection mode** — `v` toggles an active
  selection between the existing Stream (character-flow) mode and a new
  Block (rectangular/visual-block) mode. The rectangle's opposite corners
  are the anchor and cursor; each line in the row range contributes
  columns `min(col)..=max(col)` clamped to its length. Short lines are
  right-padded with spaces on copy so pasted columns stay aligned.
- **Virtual-space cursor movement (block mode)** — in block mode the
  cursor and anchor can move past a line's end as if the line were padded
  with spaces (cap = `max(longest line, visible width)`), so the
  rectangle's right edge can extend beyond the shortest line. Horizontal
  moves don't wrap (keeping the rectangle's row stable); vertical moves
  preserve the column across lines of differing length. Leaving block
  mode clamps corners back to real positions. Stream mode is unchanged.
- Footer shows `BLOCK N lines M chars` while in block mode; the `?` help
  dialog lists `v` as "toggle stream / block selection".

## [0.1.0] - 2026-08-19

First release: a Herdr port of the `zellij-flash` plugin, reaching
functional parity with the original plus a few Herdr-native additions.

### Added

- **Scrollback view** with relative line numbers, cursor, 2-line footer,
  arrow-key + half-page navigation, horizontal scroll with `…` overflow
  indicators (Phase 2).
- **ANSI color reproduction** — the popup renders the source pane's
  ANSI colors and font attributes, not just plain text (beyond parity
  with the original, which rendered plain text only).
- **Word motions** `w`/`W`/`b`/`B`/`e`/`E`/`0`/`$` (Phase 3).
- **Selection model** with `Space` toggle, Esc cancel chain, and a
  teal gutter cue when selection mode is active (Phase 4).
- **Word-jump** `s`/`S` with nvim-flash-style labels: distance ordering,
  typed-char exclusion, continuation-aware exclusion, partial-match
  fallback. `S` plants the selection anchor at the destination (Phase 5).
- **Line-jump** `l`/`L` with gutter labels (directional scheme:
  a-z below, A-Z above). `L` plants the anchor (Phase 6).
- **Incremental search** `/` with input + nav phases, `n`/`N` cycling
  with wrap, `Space`-to-anchor (Phase 7).
- **Actions**: `Enter` copies the selection to the clipboard via
  `arboard`; `p` inserts into the source pane via `pane.send_text`.
  Multi-line inserts get a `y`/`Enter` confirm dialog (Phase 8).
- **Config + profile cycling**: `config.toml` with `log_level`,
  `labels`, `line_labels`, 16 `color_*` theme roles, and
  `[profiles.<name>]` depth cycle lists selected by `FLASH_PROFILE`.
  `g` cycles depths and re-grabs. Built-in default profile:
  `["200", "5000", "unlimited", "viewport"]` (Phase 9).
- **Keybinding dialog** `?` — replaces the always-visible footer hints
  with an on-demand two-column overlay (Phase 9b).
- **Manifest** with four `[[actions]]` (`flash-open`, `flash-200`,
  `flash-2000`, `flash-unlimited`), `[[build]]` step, popup at
  90%x85% (Phase 10).
- **Docs**: `doc/keybinding.md`, `doc/env-vars.md`,
  `doc/config-reference.md`, `doc/use-cases.md`, `doc/flash-jump.md`.
- **CI**: `ci.yml` (fmt/clippy/test on macOS + Linux) and `release.yml`
  (tag-triggered, 3 target triples, SHA-256, rolling `latest` release).
- 53 unit tests covering ANSI parsing, word motions, selection, word-jump
  labels, line-jump labels, search, and config.

### Changed from the original `zellij-flash`

- **Host**: Zellij WASM plugin → Herdr native process with a real PTY.
- **Rendering**: host-mediated draw calls → `crossterm` backend directly.
- **Scrollback acquisition**: host-provided → `pane.read` socket API
  (`source = "recent_unwrapped"` / `"visible"`).
- **Insert action**: host action → `pane.send_text` (not `send_input`).
- **Clipboard**: host `Clipboard` action → `arboard` crate.
- **Config**: KDL `configuration {}` block → TOML `config.toml` under
  `$HERDR_PLUGIN_CONFIG_DIR`; `labels`/`line_labels`/theme collapsed to
  global (not per-keybind), matching the sister `herdr-zextract` port.
- **Insert keybinding**: `Shift-Enter` → `p` (Shift-Enter is
  indistinguishable from Enter in legacy keyboard mode).
- **Default depths**: `viewport, 200, 2000` → `200, 5000, unlimited,
  viewport` (viewport last, since it's rarely the useful mode).
- **Keybinding hints**: always-visible footer line → on-demand `?` dialog.
