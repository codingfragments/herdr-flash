# herdr-flash

> **Status: planning.** No code yet — see [PLANNING.md](PLANNING.md) for the
> full design. This repo exists to track the port and collect decisions
> before implementation starts.

A port of [zellij-flash](https://github.com/codingfragments/zellij-flash)
(a [Zellij](https://zellij.dev) plugin) to run as a plugin for
[Herdr](https://herdr.dev), a persistent terminal runtime built for AI
coding agents.

## What it does (unchanged from the original)

Neither Zellij nor Herdr has a built-in way to select arbitrary text from
a terminal pane's scrollback. `herdr-flash` opens a floating view of the
source pane's scrollback, renders it with relative line numbers, and lets
you jump to a word or line with nvim-`flash`-style label jumps, then
select text precisely and copy it to the clipboard or insert it directly
into the source pane.

Typical workflows:
- Copy a URL, path, or command that scrolled past.
- Grab a block of output for a ticket or chat message.
- Insert a previous command back into the shell without retyping.

## Why a separate repo instead of a fork

Herdr plugins are plain argv binaries talking to a JSON socket API, not WASM
modules on the `zellij-tile` ABI. The host-integration layer is being
rewritten from scratch; the scrollback-rendering and flash-jump navigation
logic are being carried over from the original crate with as few changes
as possible. See [PLANNING.md](PLANNING.md#relationship-to-the-original-repo)
for the fork-vs-standalone decision point.

## Relationship to the original project

| | |
|---|---|
| Original | [codingfragments/zellij-flash](https://github.com/codingfragments/zellij-flash) |
| Original host | Zellij (WASM plugin, `zellij-tile` crate) |
| This repo's host | [Herdr](https://herdr.dev) (native argv plugin, socket API) |
| License | MIT (same as original) |
| Rendering | `ratatui` (unchanged) |

## Planned install (not yet available)

Two install paths are planned once a stable release exists — see
[PLANNING.md](PLANNING.md#install--distribution-plan) for full detail:

1. **`herdr plugin install codingfragments/herdr-flash`** — clones the
   repo, builds from source, registers the plugin. Works on any machine
   with a Rust toolchain.
2. **`cargo install --git https://github.com/codingfragments/herdr-flash --tag vX.Y.Z`**
   — installs a labeled stable release binary onto `PATH`, then a minimal
   `herdr-plugin.toml` just references the installed binary name. No repo
   clone or build step needed at plugin-registration time.

Prebuilt binaries for macOS (arm64/x86_64) and Linux (x86_64/aarch64) will
be attached to tagged GitHub releases via CI — see PLANNING.md for the
GitHub Actions matrix.

## License

MIT, matching the original `zellij-flash` project.
