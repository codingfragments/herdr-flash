# Planning: herdr-flash

## 1. Goal

Port [zellij-flash](https://github.com/codingfragments/zellij-flash)'s
functionality — render a source pane's scrollback in a floating view,
navigate/select with nvim-`flash`-style jump-to-word and jump-to-line, then
copy or insert the selection — to run as a native [Herdr](https://herdr.dev)
plugin, with the same feature set and config surface as the original where
practical.

Not a goal: feature growth beyond parity during the initial port. Ideas
that only make sense on Herdr are tracked in
[§13 Ideas beyond parity](#13-ideas-beyond-parity), not built into v1.

## 2. Original plugin reference

- Repo: https://github.com/codingfragments/zellij-flash
- Host: Zellij, WASM plugin via the `zellij-tile` crate (0.44.3 pinned)
- Rendering: `ratatui` (with `default-features = false`, i.e. no backend
  crate bundled — Zellij's WASM host owns the actual terminal I/O)
- Config: `profiles` (depth: `viewport` or N scrollback lines), `size`
  (float dimensions `WIDTHxHEIGHT`)
- License: MIT

## 3. Why Herdr is a different shape of host

Zellij plugins are WASM modules sandboxed behind the `zellij-tile` ABI —
the host owns the actual terminal rendering/input loop and the plugin only
issues draw calls into it. Herdr plugins are **plain native processes**
that get a **real PTY** when declared as a popup pane — the plugin itself
owns the render loop via `crossterm`, same as any standalone terminal app.

Practical consequence: this is actually a more natural fit for `ratatui`
than the Zellij WASM model was — no host-mediated draw calls, just a
normal `crossterm` backend. The part that gets rewritten is scrollback
acquisition (was host-provided, now via socket `pane.read`) and
input/copy actions (was host action calls, now `pane.send_text` +
`arboard`).

## 4. Architecture

```
┌─────────────────────────────────────────────┐
│ herdr-plugin.toml                            │
│  - [[panes]] entry: placement = "popup"       │
│    command = ["./target/release/herdr-flash"] │
│    width/height = 90%/90% (matches the       │
│     original "size" config; overlay was       │
│     investigated but covers the whole         │
│     workspace, not just the active pane)      │
│  - [[actions]]: thin `herdr plugin pane open` │
│    invocations selecting a profile via --env  │
│    FLASH_PROFILE (profile values live in the  │
│    user's config.toml, never here)            │
└───────────────┬───────────────────────────────┘
                │ launches (real PTY)
                ▼
┌─────────────────────────────────────────────┐
│ herdr-flash binary (Rust)                    │
│                                               │
│  1. socket_client.rs                         │
│     - connect $HERDR_SOCKET_PATH             │
│     - one request/response per fresh          │
│       UnixStream (server closes after one)   │
│     - pane.read (source per "profiles":      │
│       viewport → "visible", N →               │
│       "recent_unwrapped" + lines)            │
│     - pane.send_text (insert selection)       │
│                                               │
│  2. render.rs (ported ~as-is)                │
│     - crossterm backend (new — previously     │
│       host-owned) + ratatui                  │
│     - relative line numbers, cursor           │
│     - renders into a centered popup box        │
│       (90%x90% of the workspace, matching       │
│       the original float); no alt-screen;      │
│       draws over the popup's own buffer,       │
│       terminal.clear() on enter                │
│                                               │
│  3. flash.rs (ported ~as-is)                 │
│     - jump-to-word / jump-to-line label       │
│       overlay + input handling               │
│                                               │
│  4. selection.rs                             │
│     - precise text-range selection           │
│     - copy → arboard clipboard               │
│     - insert → pane.send_text               │
│                                               │
│  5. config.rs                                │
│     - $HERDR_PLUGIN_CONFIG_DIR/config.toml   │
│     - profiles + size, loaded once per launch│
└───────────────────────────────────────────────┘
```

## 5. Migration map: zellij-tile → herdr socket API

| Original (`zellij-tile`) | Herdr equivalent (confirmed live) |
|---|---|
| Plugin registration, `register_plugin!` | `herdr-plugin.toml` manifest, `[[panes]]` entry |
| Keybind → `LaunchOrFocusPlugin` | `[[actions]]` → `herdr plugin pane open --placement popup --env FLASH_PROFILE=<name>`; bound via `[[keys.command]]` `type = "plugin_action"` in the user's own config |
| Host owns terminal I/O; plugin issues render calls | Plugin owns a real PTY; `crossterm` backend directly (no alt-screen) |
| Read focused pane content + scrollback depth (`profiles`) | `pane.read` with `source = "visible"` (viewport) or `"recent_unwrapped"` + `lines: u32` (N scrollback); response at `result.read.text` |
| Write/paste into pane | `pane.send_text` with `{"pane_id", "text"}` (not `send_input`) |
| Floating pane `size` config (`WIDTHxHEIGHT`) | Popup `width`/`height` in `[[panes]]` (cells or %); advisory since popup can't be resized live (per sister port). Overlay was investigated but covers the whole workspace, not just the active pane — not adopted. |
| Clipboard (host `Clipboard` action) | `arboard` crate directly |

> The `pane.send_text` (not `pane.send_input`) and `recent_unwrapped`
> source spellings, and the `result.read.text` response path, are
> confirmed-live findings inherited from the sister `herdr-zextract`
> port — see [§12](#12-open-questions-resolved-2026-08-18).

## 6. Language & dependency choice

**Rust**, carrying over the rendering and flash-navigation logic from the
original crate.

Rationale:
- The relative-line-number rendering and flash jump-label logic are
  `ratatui`-based already and don't touch `zellij-tile` beyond drawing —
  they move over close to unchanged, now driving a real `crossterm`
  backend instead of a host-mediated one (arguably simpler than before).
- No more `wasm32-wasip1` target — native `cargo build --release` per
  platform.
- Cross-platform crates needed:
  - `ratatui` + `crossterm` (this time with the terminal backend enabled,
    since the plugin now owns the PTY directly — original had
    `default-features = false` because Zellij owned it).
  - `serde` / `serde_json` — socket protocol.
  - `arboard` — clipboard, macOS + Linux (X11/Wayland).
  - `toml` — user config (`config.toml`).
  - Hand-rolled blocking Unix-socket client, same rationale as
    `herdr-zextract` (no async runtime needed for a foreground UI).

### Socket client contract (confirmed live)

Herdr's socket server closes the connection after serving **exactly one
request** — reusing a connection yields `BrokenPipe` even milliseconds
later, regardless of idle time. This was traced to a single root cause in
the sister port (surfaced as two separate-looking bugs there). The
`socket_client::request(socket_path, method, params)` helper is therefore
the **only** way anything in the plugin talks to the socket — it opens a
fresh `UnixStream` per call, sends one newline-delimited JSON line
(`{"id","method","params"}`), and reads one response line (`result` or
`error`). No persistent-connection API exists to accidentally reuse.

### Terminal setup contract (confirmed live)

The popup pane is a real PTY whose buffer starts as whatever scrollback
was there. `ratatui`'s diff renderer assumes it starts from a blank
terminal, so on entry: `enable_raw_mode()`, hide the cursor,
`CrosstermBackend::new(io::stdout())`, `Terminal::new()`, then
**`terminal.clear()`** — without the clear, cells that render blank in
the first frame don't get force-written and old content shows through. On
exit: `disable_raw_mode()`, show the cursor, `terminal.clear()` again.
No alternate screen is entered (the sister port draws over the popup's
own buffer directly and this works); revisit if flash's full-screen
overlay turns out to need it.

## 7. Portability plan (macOS + Linux)

- Target triples: `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`.
- No Windows target, same rationale as `herdr-zextract` (Herdr's Windows
  beta uses ConPTY / different IPC transport, out of scope).
- `crossterm` already abstracts terminal capability differences between
  macOS Terminal/iTerm2 and Linux terminal emulators; still needs manual
  verification on both (mouse support, color depth, wide-character
  rendering for the flash label overlay) before calling v1 done.
- Clipboard fallback behavior (Wayland vs X11 vs macOS) isolated behind
  `arboard` — verify Wayland clipboard behavior specifically on Linux, as
  it's historically the most inconsistent case for terminal clipboard
  tools.
- **No `x86_64-apple-darwin` (Intel Mac) release binary** — not worth the
  CI minutes for a platform with negligible remaining install base among
  this plugin's users; build from source there with `cargo build --release`.
  (Inherited finding from the sister port.)
- **`aarch64-unknown-linux-gnu` needs no `cross`/QEMU** — GitHub-hosted
  `ubuntu-24.04-arm` runners are GA and available in private repos; use
  native ARM runners directly. (Confirmed live.)

## 8. Install & distribution plan

Two supported install paths, both anchored on tagged GitHub releases —
mirroring `herdr-zextract` so the two repos stay consistent.

**A. Source install via `herdr plugin install` (recommended):**
```sh
herdr plugin install codingfragments/herdr-flash --ref v0.1.0
```
Clones the repo at that ref, runs the `[[build]]` step
(`cargo build --release`) `herdr-plugin.toml` declares, and registers
the plugin — one command, no separate build step. Pin `--ref` to a tagged
version (`v0.1.0`) for a reproducible install, or use `--ref latest` to
track the newest tagged release (a rolling tag, force-moved on every
release). Omit `--ref` entirely to track `main`. Add `--yes` to skip the
confirmation prompt (needed for non-interactive/scripted installs, e.g.
from dotfiles). To update later, re-run the same command — it re-resolves
the ref and rebuilds in place; there's no separate `herdr plugin update`.

> **Important — this is not automatic build detection.** Herdr does not
> inspect a cloned repo and decide "this is a Rust project, run `cargo`."
> It only runs a build step because this repo's `herdr-plugin.toml`
> declares one via `[[build]]`. With no `[[build]]` section, install just
> clones and registers without compiling anything — so the manifest
> **must** carry `command = ["cargo", "build", "--release"]` and its
> `[[panes]].command` must point at `./target/release/herdr-flash`, not a
> bare `herdr-flash` (that only resolves via `PATH`, which is Option B).
> `herdr plugin link` (local-dev of a checkout you already have) skips
> `[[build]]` entirely — you `cargo build` your own working tree first.

**B. Binary install via `cargo install` from a labeled stable release:**
```sh
cargo install --git https://github.com/codingfragments/herdr-flash --tag v0.1.0 herdr-flash
```
This is `cargo install`, so it also compiles from source — just without a
manifest-driven `[[build]]` step or a repo clone to manage. Installs the
binary onto `PATH` directly. Point a minimal `herdr-plugin.toml` at the
installed binary (`command = ["herdr-flash"]`, resolved via `PATH`) and
bind a key to it in your Herdr config.

**C. (stretch) Prebuilt binary download**
GitHub Actions attaches prebuilt binaries per target triple to each tagged
release. A future `install.sh` could fetch the right binary directly
without compiling — nice-to-have, not required for v1.

### New-machine bootstrap (planned)

```sh
# 1. Ensure herdr itself is installed (https://herdr.dev/install.sh)
# 2. Install the plugin (path A, recommended):
herdr plugin install codingfragments/herdr-flash --ref v0.1.0
# 3. Bind a key in herdr config to the plugin action (was "Alt f" originally)
```

## 9. CI / release plan (GitHub Actions)

Same shape as `herdr-zextract`'s released workflow — kept consistent across
both repos:

- `ci.yml` — on every PR and push to `main`: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`, on
  `ubuntu-latest` + `macos-latest`.
- `release.yml` — trigger: tag push matching `v*.*.*`. Matrix (native
  runners only, no cross/QEMU, no Intel Mac):
  - `macos-14` → `aarch64-apple-darwin`
  - `ubuntu-latest` → `x86_64-unknown-linux-gnu`
  - `ubuntu-24.04-arm` → `aarch64-unknown-linux-gnu`
  - Steps: checkout → `cargo build --release --target <triple>` → strip →
    `tar czf herdr-flash-<triple>.tar.gz` → `sha256sum` → upload as
    artifact.
  - Release job: `softprops/action-gh-release@v2` with
    `generate_release_notes`, attaching all artifacts.
  - Rolling `latest` tag: force-moved to this release's commit and
    published as a second release with `make_latest: false`, so
    `herdr plugin install <owner>/<repo> --ref latest` tracks the newest
    tagged release without naming a version.
- First stable tag once flash-jump + selection + copy/insert parity with
  the original is manually verified against a real Herdr session on both
  a Mac and a Linux box.

## 10. Repo layout (planned, once code starts)

```
herdr-flash/
├── Cargo.toml
├── Cargo.lock
├── CHANGELOG.md
├── justfile                 # build / check / link / unlink / relink / open
├── config.example.toml      # starter config template (profiles + size + colors)
├── src/
│   ├── main.rs              # launch context, mode dispatch, run loop
│   ├── socket_client.rs     # herdr socket API client (fresh conn per call)
│   ├── config.rs            # $HERDR_PLUGIN_CONFIG_DIR/config.toml loader + Theme
│   ├── render.rs            # ported ratatui rendering, own crossterm backend
│   ├── flash.rs             # word-jump + line-jump label logic (Modes Jump/LineJump)
│   ├── search.rs            # incremental search (Mode Search: input + nav)
│   └── selection.rs         # anchor/extend/selected_text + copy/insert actions
├── doc/
│   ├── config-reference.md  # full config.toml schema (profiles, size, labels, colors)
│   ├── keybinding.md        # shipped actions, binding a key, adding your own
│   ├── flash-jump.md        # word-jump algorithm + line-jump mechanic reference
│   ├── use-cases.md         # worked walkthroughs
│   └── env-vars.md          # the one env var involved
├── herdr-plugin.toml        # manifest (written in Phase 5)
├── .github/workflows/
│   ├── ci.yml
│   └── release.yml
├── PLANNING.md              # this file
├── MIGRATION_FROM_ZELLIJ.md
└── README.md
```

## 11. Implementation phases

Superseded the original horizontal milestone list once [§12](#12-open-questions-resolved-2026-08-18)
unblocked real Herdr testing (the shared findings are inherited from the
sister `herdr-zextract` port, confirmed live against Herdr 0.8.0). Each
phase below is a **vertical slice**: it runs as a real popup inside a real
Herdr session and produces an observable result end-to-end, even though
most of the feature set is still stubbed. Each phase is one `phase/<slug>`
branch/PR per the gitflow in `CLAUDE.md`, merged before the next phase
starts.

When starting a phase, open it with the "Prompt" text below verbatim (it
carries the goal and scope so a fresh session doesn't need the rest of this
document to get oriented) and don't consider the phase done until its
manual test plan passes against a real Herdr install.

### Feature coverage (parity with zellij-flash v0.2.1)

Every feature of the original v0.2.1 is delivered by exactly one phase
below. Features that change shape on Herdr are called out in the phase
scope.

| Original feature | Herdr phase | Note |
|---|---|---|
| Source-pane 4-tier picker (`source_pane.rs`) | Phase 1 | **Replaced** by reading `focused_pane_id` from `HERDR_PLUGIN_CONTEXT_JSON` — no `source_pane.rs` needed |
| Scrollback extraction (`viewport` / `Lines(N)`) | Phase 1, 9 | `pane.read` `visible` / `recent_unwrapped`+`lines` |
| Render: relative line numbers, 2-line footer, `…` overflow, buffer reuse | Phase 2 | crossterm backend (no ANSI emitter) |
| Cursor/viewport: auto-follow, half-page recenter, `Shift-←/→` pan | Phase 2, 3 | |
| Word motions `w/W/b/B/e/E/0/$` | Phase 3 | word vs WORD semantics |
| Selection: anchor, extend, `Space` toggle, Esc chain | Phase 4 | orthogonal to mode |
| Word-jump `s` + select-jump `S` (label algorithm) | Phase 5 | distance ordering, typed-char + continuation-aware exclusion, partial fallback |
| Line-jump `l` + select-jump `L` (directional/unified) | Phase 6 | gutter labels |
| Search `/` (input + nav phases, `n`/`N`, Space-to-anchor) | Phase 7 | only when no anchor |
| Actions: `Enter` copy, `Shift-Enter` insert + Confirm dialog | Phase 8 | `arboard` + `pane.send_text` |
| Profile cycling `g` (re-grab) | Phase 9 | |
| Config: `profiles`, `size`, `labels`, `line_labels`, 16 `color_*` | Phase 9 | TOML under `$HERDR_PLUGIN_CONFIG_DIR`; per-keybind depth via `[profiles.<name>]` |
| Manifest + keybinding actions | Phase 10 | `[[actions]]` per profile |
| CI, release, docs | Phase 11 | |

### Phase 1 — Socket client + raw popup echo

**Prompt:** Build the smallest possible `herdr-flash` binary and manifest
that proves the popup-and-socket plumbing works end-to-end: on launch,
read `HERDR_PLUGIN_CONTEXT_JSON` for `focused_pane_id`, call `pane.read`
over `$HERDR_SOCKET_PATH` with `source = "recent_unwrapped"`, and print
the raw scrollback text into the popup pane (no rendering, no flash nav,
no ratatui yet — just prove the pipes connect). Write `herdr-plugin.toml`
with a `[[panes]]` popup entry and a `[[build]]` step
(`cargo build --release`), per §8. Keep the socket client hand-rolled
(`UnixStream`, newline-delimited JSON, fresh connection per call) per §6
— no async runtime. The original's `source_pane.rs` 4-tier picker is
**not ported** — Herdr hands the source pane id directly via context JSON.

**Scope:** `socket_client.rs` (connect, one request/response round trip),
`main.rs` reading context env + printing result, minimal manifest.

**Out of scope:** render view, flash navigation, selection, config file,
keybinding (`herdr plugin pane open` from the CLI is an acceptable trigger
for this phase — real keybinding wiring is Phase 10).

**Manual test plan:**
1. `cargo build --release`.
2. `herdr plugin link .` from the repo root (or `just link`).
3. Focus any pane with some scrollback text, then trigger the plugin pane
   (`herdr plugin pane open --plugin herdr-flash --entrypoint flash
   --placement popup`, or `just open`).
4. Confirm the popup shows the scrollback of the pane that was focused
   *before* the popup opened, not the popup's own (empty) buffer.
5. `herdr plugin unlink herdr-flash` when done (or `just unlink`).

### Phase 2 — Scrollback view: render + relative line numbers + cursor + footer

**Prompt:** Port the original's relative-line-number `ratatui` rendering
into a terminal binary that owns its `crossterm` backend directly (the
popup's real PTY), fed by the real `pane.read` from Phase 1. Render the
scrollback with relative line numbers (cursor row = 0, others show
distance), a cursor cell, and the 2-line footer (status line: profile
label, line count, cursor pos; key-hint line). Implement arrow-key
movement (`↑↓←→`, wrapping at line edges), `scroll_y`/`scroll_x`
auto-follow after every move, horizontal scroll with `…` overflow
indicators on both sides, and `Esc` closes the popup. Follow the
terminal-setup contract in §6 exactly: `enable_raw_mode` → hide cursor →
`CrosstermBackend` → `Terminal::new` → `terminal.clear()` on entry;
reverse on exit. No alternate screen. Hold a reused `Buffer` for the
render area (reallocate only on resize) — less critical on native than
it was under WASM, but cheap to keep.

**Scope:** `render.rs` (ported `render_all`/`render_content`/
`render_footer` + `build_line_spans` helper), `main.rs` wiring it to the
Phase 1 socket read, `Cargo.toml` with `ratatui` + `crossterm` (backend
enabled).

**Out of scope:** word motions, flash jump labels, selection, search,
copy/insert, config, theme colors (use built-in Catppuccin Macchiato
defaults hardcoded for now).

**Manual test plan:**
1. `just relink`.
2. Trigger the popup on a pane with substantial scrollback.
3. Confirm relative line numbers are correct (cursor row = 0), the cursor
cell is visible, and no leftover scrollback bleeds through around the
view.
4. Arrow keys move the cursor (wrapping at line edges); the viewport
follows the cursor.
5. A line longer than the viewport shows `…` on the overflow side;
`Shift-←`/`Shift-→` pan 5 columns without moving the cursor.
6. `Esc` closes the popup cleanly — cursor visible, no raw-mode leak.

### Phase 3 — Word motions + half-page navigation

**Prompt:** Port the original's vim-style cursor vocabulary into the
Phase 2 render loop: word motions `w`/`W`/`b`/`B`/`e`/`E` (word =
`[a-zA-Z0-9_]+`, WORD = non-whitespace run), `0` (line start), `$` (last
char), and half-page `PgUp`/`PgDn` that move the cursor by
`content_rows / 2` and re-center the viewport on the cursor. All motions
clamp the column to the target line's length and scroll the cursor into
view. These work identically with or without a selection (selection
lands in Phase 4 — motions just move the cursor for now).

**Scope:** `motion_w`/`motion_b`/`motion_e` (with `cclass`/
`next_pos`/`prev_pos` helpers), `motion_line_start`/`motion_line_end`,
`page_up`/`page_down` + `recenter_scroll`, wired into the key handler.

**Out of scope:** selection, jump, search, config.

**Manual test plan:**
1. `just relink`.
2. On a pane with mixed tokens, confirm `w`/`W`/`b`/`B`/`e`/`E` land on
the right word/WORD boundaries; `0`/`$` hit line ends.
3. `PgUp`/`PgDn` jump half a page and the cursor lands at the vertical
centre; the viewport doesn't leave blank rows near the buffer end.
4. Motions on the last/first line clamp without panicking.

### Phase 4 — Selection model + Esc cancel chain

**Prompt:** Port the original's selection model: an `anchor:
Option<(usize, usize)>` field on state, orthogonal to mode. `Space`
toggles — set anchor at cursor, or if already set, swap cursor/anchor
(jump cursor to the old anchor end), or clear. The selection spans
`min(anchor, cursor)..=max(anchor, cursor)` in stream order and renders
with a blue background. Every cursor move (arrows, motions — and later,
jump/search-nav) extends the selection while the anchor is set. The
footer shows `SEL N lines M chars` when a selection is active. Implement
the `Esc` cancel chain: in a mode (jump/line-jump/search/confirm) →
cancel mode; else if anchor set → clear anchor; else → close the popup.

**Scope:** `anchor` field, `selection_range`/`selected_text`/
`selection_info`, `Space` toggle, selection rendering in
`build_line_spans`, Esc chain in the key handler.

**Out of scope:** jump modes, search, copy/insert actions (selection is
visible and queryable but not yet actionable — that's Phase 8).

**Manual test plan:**
1. `just relink`.
2. Move the cursor, press `Space` — anchor sets, footer shows `SEL`.
3. Move with arrows/motions — the selection extends and highlights.
4. Press `Space` again — cursor jumps to the anchor end (swap).
5. Press `Esc` with anchor set — anchor clears (not close). Press `Esc`
again with no anchor — popup closes.

### Phase 5 — Word-jump `s` / `S` (label algorithm + select-jump)

**Prompt:** Port the original's nvim-`flash`-style word-jump into
`flash.rs`, layered on the Phase 2/3 render and the Phase 4 selection.
`s` enters Jump mode; typing narrows case-insensitive substring matches
across **visible lines** (with trailing-space virtual matching so `"foo "`
matches `"foo"` at line end). Labels are assigned by distance from the
cursor (nearest first), excluding typed chars and continuation chars
(ambiguous continuation → excluded from the pool; unique continuation →
pre-assigned to that match). When matches exceed the label pool, fall
back to partial-match highlighting (no labels). Typing a label jumps the
cursor to the match start and returns to Normal. `S` (and `Shift-s`)
enters **select-jump**: same jump, but plants the selection anchor at
the destination on completion — this works because selection (Phase 4)
already exists, so no rework. Render labels/partial/prefix-match per the
priority table (label > prefix-match > partial > cursor > selection >
text). `Esc` cancels the jump without touching an existing anchor.

**Scope:** `flash.rs` — `Mode::Jump`, `compute_jump_labels` (distance
ordering, typed-char exclusion, continuation-aware exclusion, partial
fallback), `handle_key_jump`, `jump_to`, jump rendering in
`render_content`/`build_line_spans`, `start_selection` flag wired to the
Phase 4 anchor.

**Out of scope:** line-jump (Phase 6), search (Phase 7), config-driven
`labels` charset (Phase 9 — hardcode the 52-char `a-zA-Z` pool for now).

**Manual test plan:**
1. `just relink`.
2. `s` then type a prefix — confirm labels appear on matches when count
fits the pool; typing a label jumps the cursor to the right match.
3. Type a prefix with too many matches — confirm partial-highlight (no
labels, all matches yellow) and the "keep typing…" footer.
4. Type a prefix where two matches share a continuation char — confirm
that char is never a label (typing it narrows, doesn't jump).
5. Type a prefix with a unique continuation — confirm that char is
pre-assigned and jumps directly to its match.
6. `S` then label — confirm the anchor plants at the destination and
the footer shows `[SEL]`.
7. `Esc` mid-jump returns to Normal without clearing an existing anchor.

### Phase 6 — Line-jump `l` / `L` (gutter labels + select-jump)

**Prompt:** Port the original's line-jump into `flash.rs`. `l` enters
LineJump mode: every visible line gets a label **in the gutter**
(replacing the line number) instantly. Default directional scheme: `a`-`z`
for lines below the cursor (`a` = nearest), `A`-`Z` for lines above (`A`
= nearest); the cursor line has no label. Typing a label jumps the
cursor to that line (preserving column, clamped to the line length). `L`
(and `Shift-l`) enters **select-line-jump**: same jump, plants the
anchor at the destination. `Esc` cancels. (The `unified` scheme —
splitting the `labels` charset in half — is config-driven and lands in
Phase 9; ship the directional scheme now.)

**Scope:** `Mode::LineJump`, `compute_line_labels` (directional),
`handle_key_line_jump`, gutter-label rendering in `render_content`,
`start_selection` flag wired to the Phase 4 anchor.

**Out of scope:** `unified` line-label scheme (Phase 9), search.

**Manual test plan:**
1. `just relink`.
2. `l` — confirm gutter labels appear on every visible line except the
cursor line; lowercase below, uppercase above.
3. Type a label — cursor jumps to that line, column clamped.
4. `L` then label — anchor plants at the destination, footer shows
`[SEL]`.
5. `Esc` cancels without clearing an existing anchor.

### Phase 7 — Search mode `/`

**Prompt:** Port the original's incremental search into `search.rs`. `/`
(only when no anchor is active) enters the **input phase**: typing appends
to the query, matches highlight live across **all captured lines** (not
just visible), `Backspace` removes, `Enter` commits → switches to the
**navigation phase** (cursor jumps to the first match at or after the
current position). In nav phase: `n`/`N` jump to next/previous match
(wrapping) and re-center; `Space` sets the anchor at the current match
start and returns to Normal (the "search then select" power move); `Esc`
or any unrecognised key returns to Normal with the cursor staying at the
current match. Render non-current matches in one color, the current
match in another (bold). Footer shows `/query█` in input phase and
`/query  M/N  n:next  N:prev  Space:select  Esc:done` in nav phase.

**Scope:** `search.rs` — `Mode::Search` (input + nav),
`compute_search_matches`, `search_current_from_cursor`, `handle_key_search`,
search rendering in `render_content`/`build_line_spans`.

**Out of scope:** config.

**Manual test plan:**
1. `just relink`.
2. `/` then type — confirm matches highlight live across the whole
capture; footer shows the query with a cursor block.
3. `Enter` — switch to nav; `n`/`N` cycle through matches with wrap;
viewport re-centres.
4. `Space` in nav — anchor sets at the current match, returns to Normal;
the selection can then be extended with motions/jump.
5. `/` does nothing while an anchor is active (per the original).
6. `Esc` in input phase returns to Normal without moving the cursor.

### Phase 8 — Actions: copy + insert + Confirm dialog

**Prompt:** Wire the two terminal actions. `Enter` copies the selection
to the clipboard via `arboard` and closes the popup; warn (footer message,
stay open) if there's no selection. `Shift-Enter` inserts the selection
into the source pane via `pane.send_text` (not `send_input` — see §5/§12)
and closes; insert always targets `focused_pane_id` from the launch
context, regardless of cursor position. If the selection contains
newlines, enter `Mode::Confirm` showing "Insert N lines into pane?
y/Enter:confirm  Esc:cancel"; `y`/`Enter` confirms and inserts,
`Esc` cancels back to Normal (selection preserved). Single-line
selections insert immediately without confirmation.

**Scope:** `action_copy`/`action_insert`/`do_insert`, `Mode::Confirm`,
`arboard` dependency, `pane.send_text` call via `socket_client`.

**Out of scope:** config, manifest actions.

**Manual test plan:**
1. `just relink`.
2. Select a single-line range, `Enter` — confirm the exact text lands on
the system clipboard (test macOS, Linux X11 **and** Wayland) and the
popup closes.
3. Select a single-line range, `Shift-Enter` — confirm the text appears
in the source pane's input and the popup closes.
4. Select a multi-line range, `Shift-Enter` — confirm the Confirm dialog
appears; `y` inserts, `Esc` returns to Normal with the selection
intact.
5. Press `Enter`/`Shift-Enter` with no selection — confirm the warning
footer and that the popup stays open.

### Phase 9 — Profile cycling `g` + config + theme

**Prompt:** Port the original's config surface, adapted to Herdr's TOML
model. The zellij version passes the **entire** config block per-keybind
(`profiles`, `size`, `labels`, `line_labels`, 16 `color_*` roles) via the
`configuration {}` block — so each keybind can launch with a different
depth list, charset, and theme. The Herdr port splits this into three
layers, mirroring the sister `herdr-zextract` port:

- **Manifest** (`herdr-plugin.toml`, Phase 10): `size` → `[[panes]]`
  `width`/`height`. Single popup, single size — a deliberate
  simplification (no live resize, per §12). Per-keybind size is
  achievable by declaring multiple `[[panes]]` entries, but one ships by
  default.
- **Global config** (`config.toml`, top-level): `log_level`, `labels`
  (word-jump charset), `line_labels` (`directional`/`unified`), and the
  16 `color_*` theme roles (hex `#rrggbb` parsing, Catppuccin Macchiato
  defaults). These are preferences, not per-launch concerns, so they
  live globally — matching how the sister port keeps `[colors]` global.
- **Per-keybind profiles** (`[profiles.<name>]` in `config.toml`):
  `depths` (the scrollback-depth cycle list for `g` — e.g.
  `["viewport", "200", "2000"]`), selected at launch by
  `FLASH_PROFILE=<name>` (set on the manifest action's `command` via
  `--env`). This is the direct analog of the sister port's
  `[profiles.<name>].grab` and the primary per-keybind lever from the
  original. A built-in `default` profile (used when `FLASH_PROFILE` is
  unset) carries `["viewport", "200", "2000"]`.

This reaches practical parity with the original: every config key the
zellij version accepts is settable, and per-keybind depth (the main
per-launch lever) is preserved. The one deliberate simplification versus
zellij is that `labels`/`line_labels`/theme are global rather than
per-keybind — documented in `doc/config-reference.md` as a deliberate
collapse of preference-level settings, matching the sister port's
philosophy.

Add `config.rs` loading `$HERDR_PLUGIN_CONFIG_DIR/config.toml` once per
launch (missing/unset/parse-error → built-in defaults, never crash;
parse errors reported on stderr). Ship `config.example.toml` as the
starter template (single source of truth, `include_str!`'d for a
`Ctrl-W`-style write-default affordance if added). Wire `g` to cycle
the active profile's `depths` list and re-grab via `pane.read` with the
new depth (reset cursor to bottom, clear selection).

**Scope:** `config.rs` (full schema: global keys + `[profiles.<name>]`
with `depths`), `config.example.toml`, `Theme` struct with hex parsing
for all 16 roles, `cycle_profile` + re-grab, `g` keybinding,
`doc/config-reference.md`.

**Out of scope:** manifest `[[actions]]` (Phase 10), CI (Phase 11).

**Manual test plan:**
1. `just relink`.
2. `g` cycles through `viewport`/`200`/`2000` (the built-in `default`
profile) and re-grabs; cursor resets to the bottom, selection clears,
footer profile label updates.
3. Drop a `config.toml` with `[profiles.deep] depths = ["2000", "5000"]`
and bind a key to an action that sets `FLASH_PROFILE=deep`; confirm `g`
cycles that keybind's own list, not the default.
4. Set global `labels = "asdfjkl;"` (home-row only) — confirm word-jump
uses the shorter charset (reaches labeled state faster).
5. Set `line_labels = "unified"` — confirm line-jump splits the
configured `labels` charset in half.
6. Override a `color_*` key — confirm the new color renders.
7. Malformed `config.toml` → stderr message + built-in defaults, no
crash.

### Phase 10 — Manifest + keybinding actions + docs

**Prompt:** Write the real `herdr-plugin.toml`: `[[panes]]` popup with
`width`/`height` mirroring the original `size` default (`90%x85%`), plus
`[[actions]]` entries that open the popup via
`herdr plugin pane open --plugin herdr-flash --entrypoint flash --env
FLASH_PROFILE=<name>` — one action per built-in profile (e.g.
`flash-open` for viewport, `flash-200` / `flash-2000` for the line-capped
defaults), mirroring the sister port's action shape. Add `min_herdr_version`
and `platforms = ["macos", "linux"]`. Document the keybinding story in
`doc/keybinding.md` — Herdr owns all keybindings, the plugin never binds
its own; `[[keys.command]]` has no `env` field, so profile selection
lives on the action's `command` via `--env`. Write `doc/env-vars.md`
(`FLASH_PROFILE`) and update `README.md` from "planned" to real install
instructions (Option A/B per §8) with the Docs table.

**Scope:** `herdr-plugin.toml`, `doc/keybinding.md`, `doc/env-vars.md`,
`README.md` install rewrite, `doc/use-cases.md`.

**Out of scope:** `doc/flash-jump.md` (Phase 11), CI (Phase 11).

**Manual test plan:**
1. `just relink`.
2. Bind a key to `flash-open` in your Herdr config; confirm the popup
opens with the viewport-depth profile.
3. Bind a key to `flash-2000`; confirm it opens with 2000 lines of
scrollback.
4. `herdr plugin install codingfragments/herdr-flash --ref <this-tag>`
works on a clean machine (once Phase 11 cuts a tag).

### Phase 11 — CI, first release, docs pass

**Prompt:** Add `.github/workflows/ci.yml` and `release.yml` per §9. Cut
`v0.1.0` via the two-step release flow in `CLAUDE.md` (code PRs already
merged; `release/0.1.0` PR with `Cargo.toml` bump + `CHANGELOG.md` entry;
merge; tag `v0.1.0`; push tag). Confirm the release workflow builds all
three target triples, attaches `herdr-flash-<triple>.tar.gz` + `.sha256`,
and publishes both the versioned and rolling `latest` releases. Write
`doc/flash-jump.md` (the full word-jump algorithm reference, ported from
the original's `doc/jump-mode.md`) and verify every doc link resolves.

**Scope:** `ci.yml`, `release.yml`, `CHANGELOG.md`, version bump,
`doc/flash-jump.md`, final doc-link audit.

**Manual test plan:**
1. `ci.yml` passes on a PR (fmt/clippy/test, macOS + Linux).
2. After tagging, the release workflow publishes the three binaries and
the rolling `latest` release.
3. `herdr plugin install codingfragments/herdr-flash --ref v0.1.0` works
on a clean machine.
4. Fresh-eyes read of README + docs follows a clean install from zero to
a working keybind with no external references needed.

## 12. Open questions (resolved 2026-08-18)

All four shared questions below are resolved — inherited from the sister
`herdr-zextract` port, which verified them live against a real Herdr 0.8.0
install (a throwaway `herdr plugin link`'ed probe plugin, socket API
schema dump, and binary string inspection). They apply identically here
because both plugins talk to the same host. The two flash-specific items
that the sister port did *not* exercise are called out separately and get
confirmed in the Phase 1 / Phase 2 spikes.

- **Focused-pane signal**: `HERDR_PLUGIN_CONTEXT_JSON` reliably includes
  `focused_pane_id` for the pane that was focused *before* the popup
  opened — confirmed live. Also carries `focused_pane_cwd`, `tab_id`,
  `workspace_id`, `selected_text`, `clicked_url`, `invocation_source`.
  Correction to the original §4 assumption: the popup process is **not**
  told its own pane_id via env (no `HERDR_PANE_ID`) — only the source
  pane's id, via context JSON. Insert targets that `focused_pane_id`.
- **`pane.read` scrollback depth**: confirmed shape — `pane_id`, `source`
  (enum `visible` | `recent` | `recent_unwrapped` | `detection`; wire value
  uses underscores), optional `lines: u32`, `format` (`text`/`ansi`),
  `strip_ansi`. Response text is at `result.read.text`. Maps to the
  original `profiles` config as: `viewport` → `source = "visible"`, `N`
  lines → `source = "recent_unwrapped"` + `lines = N`.
- **Insert action**: `pane.send_text` with `{"pane_id", "text"}` — not
  `pane.send_input` (the original §4/§5 wording was wrong; corrected
  throughout this document).
- **Popup singleton behavior**: confirmed — opening a second popup while
  one is open fails with `"popup already open"`; `pane.list` and
  `api snapshot` never show the popup even while its process is alive.
  Design implication: don't hold a persistent pane_id/handle across
  invocations — each launch is a fresh singleton tied to whatever's
  focused at that moment.
- **`aarch64-unknown-linux-gnu` CI runners**: resolved — no `cross`/QEMU
  needed. GitHub-hosted `ubuntu-24.04-arm` runners are GA and available in
  private repos too. Use native ARM runners directly in `release.yml`.

### Flash-specific, confirmed in the Phase 1 spike

- **Overlay placement (investigated, not adopted)**: `herdr plugin pane
  open --placement overlay` works for plugin panes, but Herdr's overlay
  covers the **whole workspace**, not just the active pane's rect —
  confirmed live (the binary string "overlay and popup plugin panes
  target the active pane" means they tie to the active pane for
  *context*, but the overlay rect is workspace-wide; `--target-pane` is
  rejected for overlay/popup with that same error). A pane-scoped overlay
  ("overlay just the left pane") is not achievable with Herdr's current
  placement model. The faithful port of the original zellij-flash float
  (`size "90%x90%"`) is **popup at 90%x90%** — centered, sizable, and
  smaller than a full-workspace overlay. The `size` config key is
  advisory on Herdr (popup can't be resized live either, per the sister
  port's finding): the manifest's `[[panes]]` `width`/`height` set the
  launch size, and the `size` config key in `config.toml` is vestigial.
  The sister `herdr-zextract` port also uses popup (80%x80%); both
  plugins now converge on popup as the right Herdr placement.
- **Mouse events in a plugin popup pane**: Herdr advertises first-class
  mouse support (click/drag/right-click) at the runtime level — worth
  checking whether a plugin popup pane can receive raw mouse events via
  `crossterm`, which could make selection nicer than the original's
  keyboard-only flash navigation. Not required for parity; spike in Phase 2
  once the render loop is live.
- **Popup live resize**: the sister port confirmed there is **no**
  live-resize equivalent for a popup pane mid-session — its preview pane
  is an internal split of the picker's own fixed render area instead.
  Flash's whole view *is* the popup, so mid-session resize behavior
  (does `crossterm` see a `Resize` event? does the popup honor it?)
  needs confirming in Phase 2. If unresizable live, the `size` config is
  launch-time only, matching the original.

## 13. Ideas beyond parity

(Not for v1 — recorded so they aren't lost.)

- Mouse-assisted selection (see open question above) as an alternative to
  pure keyboard flash-jump, if Herdr's mouse support extends cleanly to
  plugin popup panes.
- Live scrollback (via `events.subscribe`) instead of a static snapshot,
  for panes that are still actively producing output while the flash view
  is open.

## License

MIT, matching the original `zellij-flash` project.
