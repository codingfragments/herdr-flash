# Keybinding reference

`herdr-flash` never binds its own keys — Herdr's own
`~/.config/herdr/config.toml` owns all keybindings, via
`[[keys.command]]` entries with `type = "plugin_action"`. This doc
covers the actions this plugin ships, how to bind them, and how to
configure a combination the shipped ones don't cover — without editing
this repo's manifest.

## Shipped actions

`herdr-plugin.toml` declares four `[[actions]]`, each a thin launcher
that opens the real interactive popup (`herdr plugin pane open`) with
`--env FLASH_PROFILE=<name>` selecting a named profile. The profile's
actual depth cycle list lives in **your own** `config.toml`, not here —
see [`doc/config-reference.md`](config-reference.md#profilesname--per-keybind-depth-cycle).

All four use the built-in `default` profile (which carries
`["200", "5000", "unlimited", "viewport"]`) so they work with zero
config, but each selects a different *starting* depth by referencing a
profile named after that depth. Define matching `[profiles.<name>]`
blocks in your own `config.toml` to customise the cycle list for each
keybind:

| Action id          | Profile name   | Starting depth | Description |
| ------------------- | -------------- | -------------- | ----------- |
| `flash-open`        | `default`      | `200`         | Default scrollback selector (200 → 5000 → unlimited → viewport) |
| `flash-200`        | `200`          | `200`         | Start at 200 lines |
| `flash-2000`       | `2000`         | `2000`        | Start at 2000 lines |
| `flash-unlimited`   | `unlimited`    | `unlimited`   | Start with everything the terminal has |

Only `flash-open` has a built-in profile (`default`); the other three
reference profile names (`200`, `2000`, `unlimited`) that fall back to
the `default` profile's depth list if you don't define them. To give
each its own cycle, add `[profiles.200]`, `[profiles.2000]`,
`[profiles.unlimited]` blocks to your `config.toml` — see
[`doc/config-reference.md`](config-reference.md#profilesname--per-keybind-depth-cycle).

## Binding a shipped action

Add a `[[keys.command]]` entry to your `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "Alt f"
action = "flash-open"
type = "plugin_action"
```

That's it — `Alt f` now opens the flash popup with the default profile.
Bind multiple keys to different actions for quick access to different
depths:

```toml
[[keys.command]]
key = "Alt f"
action = "flash-open"
type = "plugin_action"

[[keys.command]]
key = "Alt Shift f"
action = "flash-2000"
type = "plugin_action"
```

## Adding your own action

Want a profile the shipped four don't cover? Define a `[profiles.<name>]`
block in your `config.toml` with a custom `depths` list:

```toml
[profiles.deep]
depths = ["2000", "5000", "unlimited"]
```

Then add a `[[keys.command]]` entry that opens the popup with
`FLASH_PROFILE=deep`. Since `[[keys.command]]` has no `env` field
(confirmed: `herdr config check` rejects `env`, `configuration`, and
`args` as unknown keys), you need a matching `[[actions]]` entry in a
*local* manifest — or just use one of the shipped action ids and set
`FLASH_PROFILE` on its `command`. The simplest path is to define the
profile and bind to `flash-open` (which uses `default`), then make
`default` point at the depths you want.

## In-popup keybindings

Once the popup is open, the plugin handles all keys itself. Press `?`
inside the popup for the quick-reference dialog; this section is the
full reference, with examples. See [`doc/use-cases.md`](use-cases.md)
for end-to-end walkthroughs and
[`doc/flash-jump.md`](flash-jump.md) for the jump algorithms.

### Normal mode — movement

| Key | Action | Example |
|---|---|---|
| `←`/`→` | move one char; wraps across line ends | cursor at EOL → `→` moves to next line's first char |
| `↑`/`↓` | move one logical line (clamped to line length) | |
| `PgUp`/`PgDn` | half-page up / down | |
| `Shift-←`/`Shift-→` | pan the view horizontally | long wrapped lines: `Shift-→` reveals `+N` cols (footer shows `+N`) |
| `w`/`W` | next word start (word / WORD) | on `foo bar.baz`: `w` → `bar`, `w` → `baz`, `W` skips to end of `bar.baz` |
| `b`/`B` | previous word start (word / WORD) | on the `.` in `foo bar.baz`: `b` → `bar`, again → `foo` |
| `e`/`E` | next word end (word / WORD) | on `f` in `foo`: `e` lands on the 2nd `o` |
| `0` | line start | |
| `$` | line end | |

### Normal mode — jumps (`flash`-style labels)

All jumps work inside the visible viewport, in 2–3 keystrokes: type
enough of the phrase (or nothing at all for `l`), then press the label
that appears at the target.

| Key | Action | Example |
|---|---|---|
| `s` | word-jump — type a phrase, then its label; cursor lands on the **first** char of the phrase | `s` `ero` `a` on `cicero` → cursor on the `e` |
| `S` | select-jump — same jump, plants the selection anchor at the destination | `S` `ero` `a` on `cicero` → anchor on `e`, cursor follows for extending |
| `t` | end-jump — same matching as `s`, cursor lands on the **last** char of the typed phrase | `s`+`ero`+`a` lands on `e`; `t` `ero` `a` on `cicero` → cursor on the `o` |
| `T` | select-end-jump — `t`'s sibling with the anchor planted at the destination | |
| `l` | line-jump — every visible line gets a gutter label instantly (`a`-`z` below, `A`-`Z` above, nearest first); column preserved | `l` `c` → 3rd line below the cursor |
| `L` | select-line-jump — anchor at destination | |

> `line_labels = "unified"` in `config.toml` splits the label charset
> in half (one pool for both directions) instead of the default
> `a`-below / `A`-above scheme — see
> [`doc/config-reference.md`](config-reference.md#line_labels--line-jump-scheme).

Jump-mode keys while typing (`s`/`S`/`t`/`T`): chars narrow the match,
`Backspace` removes one, `Esc` cancels. In line-jump, any label jumps;
any **other** character cancels the mode.

**When to use which:** `s` to *start* something at the target (insert
point, next motion); `t` to *end* something at the target — see the
selection example below.

### Normal mode — selection

| Key | Action | Example |
|---|---|---|
| `Space` | set selection anchor; if an anchor is already set, swap cursor ↔ anchor | anchor at `foo`, cursor elsewhere → `Space` flips to extend from the other end |
| `v` | toggle the active selection between stream (character flow) and block (rectangular) | select a column of a table: anchor top-left, `v`, move to bottom-right |

With an anchor set, **any** jump or motion extends the selection
(anchor fixed, cursor = moving endpoint):

1. anchor at the start of the range (`Space`)
2. `t` + type the last word of the range + its label — the selection now
   ends **on** that word's last character (an `s`-jump would stop at the
   word start, a couple of chars short)
3. `Enter` to copy, or `p` to insert

Block selections clamp back to line ends when you leave block mode.

### Normal mode — search

| Key | Action |
|---|---|
| `/` | incremental search (only when no anchor is set) — cursor follows matches live as you type |

**Input phase:** type the query; `Enter` → navigation phase;
`Backspace` deletes a char; `Esc` cancels.

**Navigation phase:**

| Key | Action |
|---|---|
| `n` | next match (wraps) |
| `N` | previous match (wraps) |
| `Space` | exit search, anchor a selection at the current match |
| any other key | exit search, cursor stays at the current match |

Example: `/` `E0308` `Enter` `n` `n` `Space` → anchor at the third
`E0308` occurrence, then extend with motions or a `t`-jump.

### Normal mode — actions

| Key | Action | Example |
|---|---|---|
| `Enter` | copy the selection to the clipboard — box frames and line-number gutters inside it are stripped (the footer names what will be stripped while you select) | copy the command output from a rendered `╭─╮` box, text only |
| `P` | copy **verbatim** — keep frames/numbers (for ASCII art, diagrams) | copy a boxed mermaid diagram whole |
| `p` | insert the selection (stripped) into the source pane; multi-line inserts ask first | re-run a command from scrollback: `p`, then `y` on the confirm prompt |
| `g` | cycle the scrollback depth profile (e.g. 200 → 5000 → unlimited → viewport) and re-grab | the URL you want is beyond 200 lines: `g` until it appears |
| `?` | help dialog (any key dismisses it) | |
| `Esc` | cancel chain: active mode → clear anchor → close the popup | one `Esc` per step back |

### Confirm mode (multi-line insert)

Shown before `p` inserts text that spans multiple lines (the insert
lands in the source pane, so the popup asks):

| Key | Action |
|---|---|
| `y` / `Enter` | insert | |
| `Esc` | cancel — the selection is preserved (anchor stays set) | |
