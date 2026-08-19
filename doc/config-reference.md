# Configuration reference

`herdr-flash` reads `$HERDR_PLUGIN_CONFIG_DIR/config.toml` once at
startup (config is a per-invocation snapshot — no live reload while the
popup is open). Find your config dir with:

```sh
herdr plugin config-dir herdr-flash
```

If the file is missing, or fails to parse, built-in defaults are used —
a broken config never crashes the plugin (a parse error is reported on
stderr, visible in `herdr plugin log`).

**Template:** [`config.example.toml`](../config.example.toml) at the
repo root is a ready-to-copy starter with every key from this doc,
commented with its default value.

**Format:** TOML, matching Herdr's own config conventions. The original
`zellij-flash` plugin uses KDL (Zellij's config format); this port uses
TOML instead.

---

## `log_level` — stderr diagnostic verbosity

```toml
log_level = "info"   # default
```

One of `off`, `error`, `warn`, `info`, `debug`, `trace`. Governs
`herdr-flash`-prefixed stderr diagnostics (visible via `herdr plugin
log`). `off` shows nothing; `debug` shows everything.

---

## `labels` — word-jump label charset

```toml
labels = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"   # default (52 chars)
```

Characters used as word-jump (`s`) labels, in assignment order
(nearest match = first char). Any printable non-whitespace chars;
duplicates removed; order preserved. A shorter charset (e.g.
`"asdfjkl;"` — home-row only) reaches the labeled state faster but
covers fewer matches before falling back to partial-highlight.

---

## `line_labels` — line-jump scheme

```toml
line_labels = "directional"   # default
```

Controls how line-jump (`l`) labels are assigned:

- `"directional"` (default) — `a`-`z` for lines below the cursor
  (`a` = nearest), `A`-`Z` for lines above (`A` = nearest). The
  cursor line has no label.
- `"unified"` — split the `labels` charset in half: first half →
  below, second half → above. Useful when the charset is short
  (e.g. home-row) and you want both directions to use the same pool.

---

## `color_*` — theme (16 roles)

All 16 color roles accept hex `#rrggbb` strings. Defaults are
[Catppuccin Macchiato](https://catppuccin.com). Omit any to keep the
default.

```toml
# Selection (blue bg, base fg)
color_sel_bg = "#8aadf4"
color_sel_fg = "#24273a"

# Cursor (inverted: text bg, base fg)
color_cursor_bg = "#cad3f5"
color_cursor_fg = "#24273a"

# Gutter: cursor-line marker (yellow), dim line numbers (overlay0)
color_gutter_mark = "#eed49f"
color_gutter_dim = "#6e738d"

# Selection indicator (teal) — footer SEL count + gutter cue when anchored
color_sel_label = "#8bd5ca"

# Footer: dim text (overlay0), key hints (subtext1)
color_footer_dim = "#6e738d"
color_footer_key = "#b8c0e0"

# Word-jump: labels (peach bg, base fg), prefix match (red fg), partial (yellow fg)
color_jump_label_bg = "#f5a97f"
color_jump_label_fg = "#24273a"
color_jump_match_fg = "#ed8796"
color_jump_partial_fg = "#eed49f"

# Search: non-current match (green bg), current match (yellow bg), text (base fg)
color_search_match_bg = "#a6da95"
color_search_current_bg = "#eed49f"
color_search_fg = "#24273a"
```

---

## `[profiles.<name>]` — per-keybind depth cycle

Each profile defines a `depths` list that `g` cycles through. Select
a profile at launch by setting `FLASH_PROFILE=<name>` on the manifest
action's `command` (via `herdr plugin pane open --env`).

```toml
[profiles.default]
depths = ["200", "5000", "unlimited", "viewport"]
```

### Depth values

| Value | `pane.read` call | Description |
| --- | --- | --- |
| `"viewport"` | `source = "visible"` | Just what's on screen |
| `"N"` (e.g. `"200"`) | `source = "recent_unwrapped"` + `lines = N` | Last N lines of scrollback |
| `"unlimited"` (or `"all"`) | `source = "recent_unwrapped"` (no `lines` cap) | Everything the terminal has |

### Built-in `default` profile

Used when `FLASH_PROFILE` is unset or references an unknown profile.
Carries `["200", "5000", "unlimited", "viewport"]` — `viewport` is
last since it's rarely the useful mode (the whole point is to navigate
scrollback), but available in the cycle for confirming what's
currently visible.

### Custom profiles

Define as many as you need:

```toml
[profiles.deep]
depths = ["2000", "5000", "unlimited"]

[profiles.quick]
depths = ["viewport"]
```

Then bind a key to an action that sets `FLASH_PROFILE=deep` — see
[`doc/keybinding.md`](keybinding.md#adding-your-own-action).

### Deliberate simplification vs the original

The original `zellij-flash` passes the **entire** config block
per-keybind (`profiles`, `size`, `labels`, `line_labels`, 16
`color_*` roles) via the `configuration {}` block — so each keybind
can launch with a different depth list, charset, and theme. The Herdr
port collapses `labels`/`line_labels`/theme to **global** preferences
(not per-keybind), matching the sister `herdr-zextract` port's
philosophy. Per-keybind **depth** (the main per-launch lever) is
preserved via `[profiles.<name>]`. This is a deliberate collapse of
preference-level settings, not a missing feature.
