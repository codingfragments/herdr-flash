# herdr-flash

[Releases](https://github.com/codingfragments/herdr-flash/releases) ·
[PLANNING.md](PLANNING.md) for the full design and phase history ·
[MIGRATION_FROM_ZELLIJ.md](MIGRATION_FROM_ZELLIJ.md) for how this
relates to the original Zellij plugin it's ported from.

A [Herdr](https://herdr.dev) plugin for selecting and copying text from
pane scrollback — with nvim-`flash`-style jump-to-word and jump-to-line
navigation.

## What it does

Herdr has no built-in way to select arbitrary text from a terminal pane's
scrollback. `herdr-flash` opens a floating view of the source pane's
scrollback, renders it with relative line numbers, and lets you navigate
with a cursor and vim-style word motions (`w`/`b`/`e`/`0`/`$`), jump to a
word or line with `flash`-style label jumps (`s`/`l`), or search
incrementally (`/`). Select a precise range, then copy it to the clipboard
or insert it directly into the source pane.

Typical workflows:
- Copy a URL, path, or command that scrolled past.
- Grab a block of output for a ticket or chat message.
- Insert a previous command back into the shell without retyping.

See [`doc/flash-jump.md`](doc/flash-jump.md) for the jump algorithm and
[`doc/keybinding.md`](doc/keybinding.md) for the full key reference.

## Docs

| Doc | Covers |
|---|---|
| [`doc/config-reference.md`](doc/config-reference.md) | Full `config.toml` schema (profiles, size, labels, line_labels, colors) |
| [`doc/keybinding.md`](doc/keybinding.md) | Shipped actions, binding a key, adding your own |
| [`doc/flash-jump.md`](doc/flash-jump.md) | The word-jump algorithm + line-jump mechanic |
| [`doc/use-cases.md`](doc/use-cases.md) | Worked walkthroughs |
| [`doc/env-vars.md`](doc/env-vars.md) | The one env var involved |

> **Status: planning.** The docs above are planned targets — see
> [PLANNING.md §11](PLANNING.md#11-implementation-phases) for the phase
> that writes each one. No code ships yet.

## Configuration

Optional — the plugin works with zero config, using the built-in
`profiles` and `size` defaults. To change the scrollback depth profiles or
the popup dimensions, copy [`config.example.toml`](config.example.toml) to:

```sh
herdr plugin config-dir herdr-flash   # prints the target directory
```

as `config.toml`. Full schema: [`doc/config-reference.md`](doc/config-reference.md).

| Key | Default | Description |
|---|---|---|
| `profiles` | `"viewport,200,2000"` | Comma-separated scrollback depth profiles. `viewport` = visible area only; a number = that many scrollback lines. Cycled with `g`. |
| `size` | `"90%x85%"` | Popup dimensions as `WIDTHxHEIGHT`. **Advisory on Herdr** — the popup's actual size is set by `[[panes]]` `width`/`height` at manifest time (no live resize); recorded here for parity. |
| `labels` | `"a-zA-Z"` (52 chars) | Characters used as word-jump (`s`) labels. Any printable non-whitespace chars; duplicates removed; order preserved. |
| `line_labels` | `"directional"` | Line-jump (`l`) scheme: `directional` (a-z below, A-Z above) or `unified` (split `labels` in half). |
| `color_*` | Catppuccin Macchiato | 15 theme roles (`color_sel_bg`, `color_cursor_bg`, `color_jump_label_bg`, …) as `#rrggbb`. Omit any to keep the default. |

## Keybinding

The plugin ships ready-to-bind actions (one per built-in profile — e.g.
`flash-open` for the default viewport profile, `flash-deep` for a deep
scrollback profile), bound via `[[keys.command]]` entries with
`type = "plugin_action"` in your own `~/.config/herdr/config.toml` —
Herdr owns all keybindings, the plugin never binds its own keys. Each
action just names a *profile*; the profile's actual scrollback depth and
popup size live in your own `config.toml` under `[profiles.<name>]` (see
Configuration above), not in the plugin's packaging. Full binding
reference is in [`doc/keybinding.md`](doc/keybinding.md).

## Build

A native Rust binary — no WASM target involved.

**Requires a working Rust/`cargo` toolchain** (e.g. via
[rustup](https://rustup.rs)) on the machine doing the build.

```sh
git clone https://github.com/codingfragments/herdr-flash
cd herdr-flash
cargo build --release
```

Supported targets: `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`. Tagged releases ship prebuilt binaries for
all three via GitHub Actions — see
[PLANNING.md §9](PLANNING.md#9-ci--release-plan-github-actions).

## Install

*(Not yet available — both paths below are planned; see
[PLANNING.md §8](PLANNING.md#8-install--distribution-plan) for full
detail. Both build from source — there's no prebuilt-binary `install.sh`
yet; GitHub Releases attach prebuilt binaries per target triple, but
nothing consumes them automatically today.)*

Requires [Herdr](https://herdr.dev/install.sh) itself, and a working
Rust/`cargo` toolchain on the machine running the install command.

**Option A — `herdr plugin install` (recommended):**
```sh
herdr plugin install codingfragments/herdr-flash --ref v0.1.0
```
Clones the repo at that ref, runs the `[[build]]` step
(`cargo build --release`) `herdr-plugin.toml` declares, and registers the
plugin — one command, no separate build step. Pin `--ref` to a tagged
version (`v0.1.0`) for a reproducible install, or use `--ref latest` to
track the newest tagged release (a rolling tag, force-moved on every
release — see [CHANGELOG.md](CHANGELOG.md) for what's in it). Omit `--ref`
entirely to track `main`. Add `--yes` to skip the confirmation prompt
(needed for non-interactive/scripted installs, e.g. from dotfiles).

To update later, just re-run the same command — it re-resolves the ref
and rebuilds in place; there's no separate `herdr plugin update`.

**Option B — install a labeled release directly onto `PATH`:**
```sh
cargo install --git https://github.com/codingfragments/herdr-flash --tag v0.1.0
```
This is `cargo install`, so it also compiles from source — just without a
manifest-driven `[[build]]` step or a repo clone to manage. Point a minimal
`herdr-plugin.toml` at the installed binary (`command = ["herdr-flash"]`,
resolved via `PATH`) and bind a key to it in your Herdr config.

Once installed, bind a key — see [`doc/keybinding.md`](doc/keybinding.md).

## Platform support

Built and tested for macOS (Apple Silicon) and Linux (x86_64 + aarch64).
No Windows support planned. No Intel Mac (`x86_64-apple-darwin`) release
binary — build from source with `cargo build --release` if needed on that
architecture.

## License

MIT.
