# Migrating from zellij-flash

This project is a from-scratch reimplementation of
[zellij-flash](https://github.com/codingfragments/zellij-flash) targeting
[Herdr](https://herdr.dev) instead of [Zellij](https://zellij.dev). This
doc explains why it's a separate repo, what carries over, and what's being
rewritten.

## Why a separate repo instead of a fork

Herdr plugins are plain argv binaries talking to a JSON socket API, not
WASM modules on the `zellij-tile` ABI. The host-integration layer is being
rewritten from scratch; the scrollback-rendering and flash-jump navigation
logic are being carried over from the original crate with as few changes
as possible. See [PLANNING.md §12](PLANNING.md#12-open-questions-resolved-2026-08-18)
for the fork-vs-standalone decision point.

## Relationship to the original project

| | |
|---|---|
| Original | [codingfragments/zellij-flash](https://github.com/codingfragments/zellij-flash) |
| Original host | Zellij (WASM plugin, `zellij-tile` crate) |
| This repo's host | [Herdr](https://herdr.dev) (native argv plugin, socket API) |
| License | MIT (same as original) |
| Rendering | `ratatui` (unchanged) |

## What carries over as-is (or close to it)

- The relative-line-number scrollback rendering.
- The nvim-`flash`-style jump-to-word / jump-to-line label overlay and
  input handling.
- Precise text-range selection logic.
- The `profiles` (depth) and `size` (float dimensions) config concepts,
  adapted to Herdr's manifest shape.

## What's being rewritten

Everything that went through `zellij-tile`'s host ABI — and one thing that
flips direction (the plugin now owns the terminal backend directly instead
of the host owning it):

| Original (`zellij-tile`) | Herdr equivalent (confirmed live) |
|---|---|
| Plugin registration, `register_plugin!` | `herdr-plugin.toml` manifest, `[[panes]]` entry |
| Keybind → `LaunchOrFocusPlugin` | `[[actions]]` → `herdr plugin pane open --env FLASH_PROFILE=<name>`; bound via `[[keys.command]]` `type = "plugin_action"` in the user's own config |
| Host owns terminal I/O; plugin issues render calls | Plugin owns a real PTY; `crossterm` backend directly (no alt-screen) |
| Read focused pane content + scrollback depth (`profiles`) | `pane.read` with `source = "visible"` (viewport) or `"recent_unwrapped"` + `lines: u32` (N lines); response at `result.read.text` |
| Write/paste into pane | `pane.send_text` with `{"pane_id", "text"}` (not `send_input`) |
| Floating pane `size` config (`WIDTHxHEIGHT`) | Popup `width`/`height` in `[[panes]]` (cells or %) |
| Clipboard (host `Clipboard` action) | `arboard` crate directly |

Note: the original crate builds `ratatui` with `default-features = false`
because Zellij's WASM host owned actual terminal rendering. On Herdr the
plugin gets a real PTY, so this port enables the `crossterm` backend
directly — a simplification, not just a swap.

See [PLANNING.md](PLANNING.md) for the full architecture, migration
rationale, and the resolved open questions (inherited from the sister
`herdr-zextract` port, confirmed live against Herdr 0.8.0) that unblocked
implementation.

## Config/behavior differences to expect

- Zellij's plugin config (`profiles`, `size`, `labels`, `line_labels`, 16
  `color_*` roles) was passed per-keybind through the `configuration {}`
  block in `config.kdl` — each keybind could launch with a different
  depth list, charset, and theme. Herdr plugin config lives in a TOML file
  under the plugin's own config directory
  (`$HERDR_PLUGIN_CONFIG_DIR/config.toml`) instead — TOML to match Herdr's
  own config format, not the original's KDL.
- The config surface is split into three layers, mirroring the sister
  `herdr-zextract` port: **manifest** (`herdr-plugin.toml`) owns `size`
  (the popup dimensions, via `[[panes]]` `width`/`height`); **global
  config** (`config.toml` top-level) owns `log_level`, `labels`,
  `line_labels`, and the 16 `color_*` theme roles; **per-keybind
  profiles** (`[profiles.<name>]` in `config.toml`) own `depths` (the
  scrollback-depth cycle list for `g`), selected at launch by
  `FLASH_PROFILE=<name>` on the manifest action's `command`.
- The one deliberate simplification versus zellij: `labels`,
  `line_labels`, and the 16 `color_*` theme roles are **global** rather
  than per-keybind. The zellij version allowed these per-keybind; the
  Herdr port collapses preference-level settings to global (matching the
  sister port's philosophy), keeping per-keybind only the depth list —
  the main per-launch lever. See
  [PLANNING.md §11 Phase 9](PLANNING.md#phase-9--profile-cycling-g--config--theme)
  for the rationale.
- The Zellij plugin was pinned to a specific Zellij ABI version (0.44.3).
  Herdr plugins don't have an ABI to pin against — compatibility is
  tracked via `min_herdr_version` in the manifest instead.
- Mouse-driven selection may become possible where it wasn't before, since
  Herdr advertises first-class mouse support at the runtime level — this
  is an open question/idea, not a guaranteed v1 feature (see
  [PLANNING.md §12](PLANNING.md#flash-specific-to-confirm-in-the-phase-1--phase-2-spikes)).
- The popup pane is a singleton that can't be resized live mid-session
  (confirmed live in the sister port); `size` is therefore launch-time
  only, matching the original's launch-sized float.
