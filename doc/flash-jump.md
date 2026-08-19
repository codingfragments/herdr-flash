# Flash-jump reference

Word-jump (`s`) and line-jump (`l`) let you move the cursor to any
visible position in two or three keystrokes. This doc covers the full
algorithm — see [`doc/keybinding.md`](keybinding.md) for how to bind
the actions, [`doc/config-reference.md`](config-reference.md) for
config keys, and press `?` inside the popup for the quick reference.

---

## Word-jump (`s` / `S`)

Press `s`, type enough characters to identify the target, then press
the label that appears on it.

| Step | What you do | What happens |
|---|---|---|
| `s` | Enter jump mode | Footer shows `jump: type to search…` |
| type chars | Narrow matches | Matches highlight; labels appear when count ≤ pool |
| press label | Jump | Cursor moves to match start; Normal mode restored |
| `Backspace` | Remove last char | Matches recomputed |
| `Esc` | Cancel | Return to Normal; cursor unchanged |

`S` (or `Shift-s`) enters **select-jump**: same jump, but plants the
selection anchor at the destination on completion.

### Matching

Matches are found **case-insensitively** across all **visible lines**
(the current viewport, not the full scrollback). Every position where the
typed string appears — as a substring, not necessarily at a word
boundary — is a candidate.

#### Trailing-space matching

If the typed string ends with a space (e.g. `"foo "`), each line gets
virtual trailing spaces appended before matching. This lets you match
`"foo"` at the end of a line by typing `"foo "` — useful for targeting
a word that is the last token on the line rather than a prefix of a
longer word.

The label is clamped to the last real character of the line so it
never renders past the line end.

### Label states

As you type, the UI cycles through three states depending on how many
matches exist and whether the label pool is large enough to cover them.

#### State 1 — too many matches (partial highlight)

When the number of matches exceeds the available label pool, **no labels
are assigned** but every match is still highlighted. This gives you
visual confirmation that the search is working and shows you where to
look as you refine the query.

- All matched characters render in `jump_partial_fg` (default: yellow `#eed49f`)
- Footer shows: `jump: <query>  (N matches, keep typing…)`

Type more characters to narrow toward state 2.

#### State 2 — labels assigned

When match count fits within the label pool, each match gets a unique
label character overlaid on its last matched character.

- The label glyph renders with `jump_label_bg` / `jump_label_fg` (peach on base)
- Any matched characters **before** the label render in `jump_match_fg` (red)
- Footer shows: `jump: <query>  (N matches)`

Press the label character to jump, or keep typing to narrow further.

#### State 3 — no matches

When the typed string has no matches in the visible viewport, nothing is
highlighted.

- Footer shows: `jump: <query>  (no matches)`

---

## Label assignment

Labels are not assigned randomly. The algorithm applies several rules
in order to make label characters predictable and non-conflicting.

### Distance-first ordering

Matches are sorted by distance from the cursor before labels are
assigned. Distance is measured as `line_distance × 10 000 +
col_distance`, so vertical proximity dominates. The nearest match gets
the first (alphabetically earliest) label from the pool.

### Typed-char exclusion

Characters already typed are excluded from the label pool (both cases).
This prevents a label from intercepting a character the user might
still want to type to narrow the search.

Example: typed `wo` → `w` and `W` and `o` and `O` are never labels.

### Continuation-aware exclusion

This is the most important rule. For each match, the plugin looks at
the character **immediately after** the typed prefix in the source
text — the "continuation character". Two sub-rules apply:

**Ambiguous continuation (count ≥ 2):** if the same next character
appears after the typed prefix in two or more matches, that character
is excluded from the label pool entirely. Typing it will always narrow
the search, never commit a jump to the wrong position.

```
text:   word   worm   woa
typed:  wo

'r' is the next char for both "word" and "worm" → 'r' excluded
'a' is the next char for only "woa"              → 'a' may be a label
```

**Unique continuation (count = 1):** if a character is the next char
for exactly one match, it is **pre-assigned** as that match's label.
It is simultaneously removed from the general pool so it cannot
accidentally land on any other match.

```
text:   wor   woa   won
typed:  wo

'r' → unique for "wor"   → pre-assigned label r on "wor"
'a' → unique for "woa"   → pre-assigned label a on "woa"
'n' → unique for "won"   → pre-assigned label n on "won"
```

Pressing `r` jumps directly to `wor`, not to whichever match happened
to be nearest the cursor.

**Matches with no continuation** (match sits at the end of a line)
receive a label from the general pool like any other unresolved match.

### Why this matters

Without these rules, pressing a label could jump to one word while the
same character would have narrowed the search to a different word if
used as a prefix character. The continuation rules guarantee that:

> The next character after the typed prefix is always safe to type —
> it either selects the target directly (unique continuation,
> pre-assigned label) or narrows the search further (ambiguous
> continuation, excluded from labels). It will never commit a jump to
> an unintended position.

---

## Line-jump (`l` / `L`)

Line-jump is instant — no prefix typing. Press `l` and every visible
line (except the cursor line) gets a label **in the gutter**
(replacing the line number).

### Directional scheme (default)

- `a`-`z` for lines **below** the cursor (`a` = nearest)
- `A`-`Z` for lines **above** the cursor (`A` = nearest)
- The cursor line has no label

Type a label to jump the cursor to that line (column preserved, clamped
to the line length). `L` (or `Shift-l`) enters **select-line-jump**:
same jump, plants the anchor at the destination.

### Unified scheme

Set `line_labels = "unified"` in `config.toml` to split the `labels`
charset in half: first half → below, second half → above. Useful when
the charset is short (e.g. home-row) and you want both directions to
use the same pool.

---

## Rendering priority

When multiple visual elements overlap on the same character cell, the
highest-priority element wins (replace policy — the overlay fully
overrides the base ANSI style on touched cells):

| Priority | Element | Style |
|---|---|---|
| 1 | Jump label glyph | `jump_label_bg` bg + `jump_label_fg` fg + bold |
| 2 | Labeled-match prefix chars | `jump_match_fg` fg + bold |
| 3 | Partial-match chars | `jump_partial_fg` fg + bold |
| 4 | Cursor cell | Inverted |
| 5 | Current search match | `search_current_bg` bg |
| 6 | Non-current search match | `search_match_bg` bg |
| 7 | Selection range | `sel_bg` bg |
| 8 | Base ANSI text | Parsed from `pane.read` SGR escapes |

Labels are always visible even if they overlap with the cursor or a
selection.

---

## Interaction with selection mode

If a selection anchor is set when you enter jump mode, the jump
**extends the selection** rather than moving the cursor freely. The
anchor stays fixed; the cursor (and therefore the selection endpoint)
moves to the jump target. This lets you select a range entirely by
keyboard:

1. Position cursor at one end of the desired range
2. `Space` to set anchor
3. `s` → type → press label to set the other end
4. `Enter` to copy or `p` to insert

---

## Label pool size

The default pool is the 52-character string
`abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ`. After excluding
typed characters and all continuation characters, the effective pool
shrinks. If the remaining pool is smaller than the number of
unresolved matches, the UI falls back to partial highlighting (state 1)
until you type more.

Make the pool smaller (and therefore reach labeled state faster with
fewer matches) by restricting the `labels` config key:

```toml
labels = "asdfjkl;"   # 8 home-row chars — snappy but needs longer queries
```

See [`doc/config-reference.md`](config-reference.md) for the full
config reference.

---

## Colors

| Config key | Default | Used for |
|---|---|---|
| `color_jump_label_bg` | `#f5a97f` (Peach) | Label glyph background |
| `color_jump_label_fg` | `#24273a` (Base) | Label glyph foreground |
| `color_jump_match_fg` | `#ed8796` (Red) | Prefix chars before a label |
| `color_jump_partial_fg` | `#eed49f` (Yellow) | Partial matches (too many to label) |
