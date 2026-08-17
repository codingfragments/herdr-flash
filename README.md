# herdr-flash

> **Status: planning.** No code yet — see [PLANNING.md](PLANNING.md) for
> the full design and [MIGRATION_FROM_ZELLIJ.md](MIGRATION_FROM_ZELLIJ.md)
> for how this relates to the original Zellij plugin it's based on.

A [Herdr](https://herdr.dev) plugin for selecting and copying text from
pane scrollback — with nvim-`flash`-style jump-to-word and jump-to-line
navigation.

## What it does

Herdr has no built-in way to select arbitrary text from a terminal pane's
scrollback. `herdr-flash` opens a floating view of the source pane's
scrollback, renders it with relative line numbers, and lets you jump to a
word or line with `flash`-style label jumps, then select text precisely
and copy it to the clipboard or insert it directly into the source pane.

Typical workflows:
- Copy a URL, path, or command that scrolled past.
- Grab a block of output for a ticket or chat message.
- Insert a previous command back into the shell without retyping.

## Build

A native Rust binary — no WASM target involved.

```sh
git clone https://github.com/codingfragments/herdr-flash
cd herdr-flash
cargo build --release
```

Supported targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`. Tagged releases
ship prebuilt binaries for all four via GitHub Actions — see
[PLANNING.md §9](PLANNING.md#9-ci--release-plan-github-actions).

## Install

*(Not yet available — both paths below are planned; see
[PLANNING.md §8](PLANNING.md#8-install--distribution-plan) for full
detail.)*

**Option A — build from source via Herdr's plugin manager:**
```sh
herdr plugin install codingfragments/herdr-flash
```

**Option B — install a labeled stable release directly onto `PATH`
(recommended once releases exist):**
```sh
cargo install --git https://github.com/codingfragments/herdr-flash --tag v0.1.0
```
then point a minimal `herdr-plugin.toml` at the installed binary and bind
a key to it in your Herdr config.

Requires [Herdr](https://herdr.dev/install.sh) itself to be installed
first.

## Configure (planned)

| Key | Default | Description |
|---|---|---|
| `profiles` | `"viewport,200,2000"` | Comma-separated scrollback depth profiles. `viewport` = visible area only; a number = that many scrollback lines. |
| `size` | `"90%x85%"` | Popup dimensions as `WIDTHxHEIGHT`. Percentages or absolute cells. |

## Platform support

Built and tested for macOS (Apple Silicon + Intel) and Linux (x86_64 +
aarch64). No Windows support planned.

## License

MIT.
