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
as possible. See [PLANNING.md §12](PLANNING.md#12-open-questions) for the
fork-vs-standalone decision point.

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

Everything that went through `zellij-tile`'s host ABI — and one thing
that flips direction (the plugin now owns the terminal backend directly
instead of the host owning it):

| Original (`zellij-tile`) | Herdr equivalent |
|---|---|
| Plugin registration, `register_plugin!` | `herdr-plugin.toml` manifest, `[[panes]]` entry |
| Keybind → `LaunchOrFocusPlugin` | `[[keys.command]]` → `plugin_action` |
| Host owns terminal I/O; plugin issues render calls | Plugin owns a real PTY; `crossterm` backend directly |
| Read focused pane content + scrollback depth (`profiles`) | `pane.read` (`source = "viewport"` or `"recent-unwrapped"`, depth param — see PLANNING.md open questions) |
| Write/paste into pane | `pane.send_input` / `pane.send_text` |
| Floating pane `size` config (`WIDTHxHEIGHT`) | Popup `width`/`height` in `[[panes]]` (cells or %) |
| Clipboard (host `Clipboard` action) | `arboard` crate directly |

Note: the original crate builds `ratatui` with `default-features = false`
because Zellij's WASM host owned actual terminal rendering. On Herdr the
plugin gets a real PTY, so this port enables the `crossterm` backend
directly — a simplification, not just a swap.

See [PLANNING.md](PLANNING.md) for the full architecture, migration
rationale, and open questions being tracked before implementation starts.

## Config/behavior differences to expect

- Zellij's plugin config (`profiles`, `size`) lived in your `config.kdl`;
  Herdr plugin config lives in a file under the plugin's own config
  directory (`$HERDR_PLUGIN_CONFIG_DIR`) instead.
- The Zellij plugin was pinned to a specific Zellij ABI version (0.44.3).
  Herdr plugins don't have an ABI to pin against — compatibility is
  tracked via `min_herdr_version` in the manifest instead.
- Mouse-driven selection may become possible where it wasn't before, since
  Herdr advertises first-class mouse support at the runtime level — this
  is an open question/idea, not a guaranteed v1 feature (see
  [PLANNING.md §12](PLANNING.md#12-open-questions)).
