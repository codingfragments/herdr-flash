# Running flash under Rex

The same `herdr-flash` binary works under Herdr and
[Rex](https://rex.dev) (superlogical's terminal multiplexer) — one
config, one release.

## How it works

```
ctrl+s>shift+s ──► ~/.config/rex/plugins/flash.lua (the launcher)
                   │ finds the focused pane of the active window
                   │ opens a floating layer (90%×85%) over it
                   ▼
                 herdr-flash (FLASH_HOST=rex)
                   │ rex capture --format=vt …   colored scrollback read
                   │ rex send …                  insert (`p`)
```

## Install

**1. The binary** (Rex side; under herdr use `herdr plugin install`
instead):

```sh
mise use -g github:codingfragments/herdr-flash   # pin a version in your dotfiles
```

**2. The launcher** — copy [`rex/flash.lua`](../rex/flash.lua) to
`~/.config/rex/plugins/flash.lua` and add to `init.lua`:

```lua
require("plugins.flash")
```

**3. Config** — `~/.config/flash/config.toml` serves both hosts. A
`[host.rex]` table can override `scroll_follow` (forced `off` under Rex
anyway — Rex scrolls client-side) and `key_debounce_ms` (0 by default
under Rex if its keyboard mode delivers distinct events).

## Binding

| Key | Profile |
|---|---|
| `ctrl+s>shift+s` | `default` (200 / 5000 / unlimited / viewport) |

Any `[profiles.<name>]` can be bound without packaging:
`rex.bind("ctrl+s>g", "flash", { profile = "unlimited" })`.

## Limits under Rex

- **Scroll-follow** is `off` (Rex scrolls client-side; the host exposes
  no viewport offset). The `offset`/`content` modes are herdr-only.
- **Key debounce** defaults by host capability; see
  [`config-reference.md`](config-reference.md#key_debounce_ms--same-key-debounce-window).
- The layer runs on the host that runs the terminal: on a remote Rex
  session the binary must be installed *there*; the launcher's
  preflight names the host and the install command when missing.
- Copy uses the system clipboard, else OSC 52 (muxhost).
