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

A binary installed with mise is only found if you also add a
`lib/env.lua`; see [Environment](#environment-the-rex-cli-and-your-login-path)
below.

**3. Config** — `~/.config/cf-flash/config.toml` serves both hosts. A
`[host.rex]` table can override `scroll_follow` (forced `off` under Rex
anyway — Rex scrolls client-side) and `key_debounce_ms` (0 by default
under Rex if its keyboard mode delivers distinct events).

## Environment: the `rex` CLI and your login PATH

The launcher needs two things from your Rex config, because the Rex
server is started by the macOS app with a minimal `PATH`
(`/usr/bin:/bin:/usr/sbin:/sbin`, no Homebrew, no mise shims):

| It needs | Without it |
|---|---|
| the **`rex` CLI's absolute path** | the popup opens, but `herdr-flash` cannot call back into Rex |
| a **login `PATH`** that contains `herdr-flash` | the popup says `flash: herdr-flash not found on this host` |

On a standard macOS install the CLI is found with no setup (the launcher
falls back to the app bundle under your own `$HOME`). Its fallback
`PATH`, though, has **no mise shims and no `~/.local/bin`**, so a binary
installed with `mise use -g` is **not found until you add a
`~/.config/rex/lib/env.lua`**. One file serves every Rex launcher:

```sh
mkdir -p ~/.config/rex/lib
curl -fsSL https://raw.githubusercontent.com/codingfragments/muxhost/main/examples/rex/lib/env.lua \
  -o ~/.config/rex/lib/env.lua
```

That sample finds the CLI and asks your login shell (fish, zsh, bash, …)
for its `PATH`, with nothing tied to an account or a home directory. To
pin values by hand instead, to check what it resolves to, or to
troubleshoot, see the muxhost guide:
[The Rex environment for launchers](https://github.com/codingfragments/muxhost/blob/main/doc/rex-environment.md).
Reload your config afterwards (`ctrl+s>shift+r`).

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
