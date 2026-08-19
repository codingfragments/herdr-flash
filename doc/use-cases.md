# Use cases

Worked walkthroughs showing `herdr-flash` in real scenarios. See
[`doc/keybinding.md`](keybinding.md) for how actions/keybinds fit
together, [`doc/config-reference.md`](config-reference.md) for the
full config schema, and press `?` inside the popup for the full
keybinding dialog.

---

## Copy a URL that scrolled past

You're watching a dev server's output and spot a URL in the logs,
but it's already scrolled off:

```
     Running server on http://localhost:3000
     API available at http://127.0.0.1:8080/api/v1
   [more output...]
```

**Flow:**
1. Press your `flash-open` keybind (`Alt f` in the example bindings).
2. The popup opens showing the source pane's scrollback (200 lines by
   default), with relative line numbers and a cursor at the bottom.
3. Press `s` to enter word-jump, type `loc` — labels appear on
   `localhost` matches. Type the label for `http://localhost:3000` to
   jump the cursor there.
4. Press `Space` to set the selection anchor, then `$` (or `e`) to
   extend to the end of the URL.
5. Press `Enter` to copy to the clipboard. The popup closes.
6. Paste (`Cmd-V` / `Ctrl-Shift-V`) wherever you need it.

---

## Grab a block of output for a ticket

A build produced a block of errors you want to paste into a Jira
ticket or chat message:

```
error[E0308]: mismatched types
  --> src/main.rs:42:17
   |
42 |     let x: u32 = "hello";
   |                 ^^^^^^^ expected `u32`, found `&str`
```

**Flow:**
1. Press your `flash-open` keybind.
2. Press `l` to enter line-jump — gutter labels appear on every line.
   Type the label for the `error[E0308]` line to jump there.
3. Press `Space` to set the anchor, then move down with `j` or `↓`
   to the end of the block.
4. Press `Enter` to copy. The popup closes.
5. Paste into your ticket.

---

## Insert a previous command without retyping

You ran a long `cargo` command earlier and want to run it again
without retyping or scrolling back:

**Flow:**
1. Press your `flash-open` keybind.
2. Press `/` to enter search, type `cargo build` — matches highlight
   live across all captured lines.
3. Press `Enter` to switch to nav phase, then `n`/`N` to cycle to
   the right match.
4. Press `Space` to set the anchor at the match start, then `$` to
   extend to the end of the line.
5. Press `p` to insert the selection into the source pane. The popup
   closes and the text appears in the pane's input line.
6. Press `Enter` in the source pane to run it.

---

## Deep scrollback: find something far back

You need to find something from 5000 lines ago:

**Flow:**
1. Press your `flash-2000` keybind (or press `g` inside the popup to
   cycle to the 2000-line depth).
2. Press `/` to search, type the query — matches highlight across all
   2000 captured lines, not just the visible window.
3. `n`/`N` to cycle through matches with wrap; the viewport
   re-centres on each jump.
4. `Space` to anchor at the match, then extend with motions or jump.
5. `Enter` to copy or `p` to insert.

---

## Jump to a specific line

You know the line number you need (e.g. from a compiler error):

**Flow:**
1. Press your `flash-open` keybind.
2. Press `l` to enter line-jump — every visible line gets a gutter
   label.
3. If the target line isn't visible, use `PgDn`/`PgUp` to scroll, then
   `l` again to re-label.
4. Type the label for the target line to jump the cursor there
   (column preserved, clamped to the line length).
5. `Space` to anchor, extend, then `Enter` to copy.
