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
│  - [[panes]] entry: placement = "popup"      │
│    command = ["./target/release/herdr-flash"] │
│    width/height ~ original "size" config      │
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
│     - no alt-screen; draws over the popup's   │
│       own buffer, terminal.clear() on enter   │
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
| Keybind → `LaunchOrFocusPlugin` | `[[actions]]` → `herdr plugin pane open --env FLASH_PROFILE=<name>`; bound via `[[keys.command]]` `type = "plugin_action"` in the user's own config |
| Host owns terminal I/O; plugin issues render calls | Plugin owns a real PTY; `crossterm` backend directly (no alt-screen) |
| Read focused pane content + scrollback depth (`profiles`) | `pane.read` with `source = "visible"` (viewport) or `"recent_unwrapped"` + `lines: u32` (N scrollback); response at `result.read.text` |
| Write/paste into pane | `pane.send_text` with `{"pane_id", "text"}` (not `send_input`) |
| Floating pane `size` config (`WIDTHxHEIGHT`) | Popup `width`/`height` in `[[panes]]` (cells or %) |
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
├── config.example.toml      # starter config template (profiles + size)
├── src/
│   ├── main.rs
│   ├── socket_client.rs     # herdr socket API client (fresh conn per call)
│   ├── config.rs            # $HERDR_PLUGIN_CONFIG_DIR/config.toml loader
│   ├── render.rs            # ported ratatui rendering, own crossterm backend
│   ├── flash.rs             # ported jump-to-word/line label logic
│   └── selection.rs         # copy/insert actions, OS-specific bits
├── doc/
│   ├── config-reference.md  # full config.toml schema
│   ├── keybinding.md        # shipped actions, binding a key, adding your own
│   ├── flash-jump.md        # jump-to-word / jump-to-line mechanic reference
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
— no async runtime.

**Scope:** `socket_client.rs` (connect, one request/response round trip),
`main.rs` reading context env + printing result, minimal manifest.

**Out of scope:** render view, flash navigation, selection, config file,
keybinding (`herdr plugin pane open` from the CLI is an acceptable trigger
for this phase — real keybinding wiring is Phase 5).

**Manual test plan:**
1. `cargo build --release`.
2. `herdr plugin link .` from the repo root (or `just link`).
3. Focus any pane with some scrollback text, then trigger the plugin pane
   (`herdr plugin pane open --plugin herdr-flash --entrypoint flash
   --placement popup`, or `just open`).
4. Confirm the popup shows the scrollback of the pane that was focused
   *before* the popup opened, not the popup's own (empty) buffer.
5. `herdr plugin unlink herdr-flash` when done (or `just unlink`).

### Phase 2 — Scrollback view with relative line numbers

**Prompt:** Port the original's relative-line-number `ratatui` rendering
into a standalone terminal binary that owns its `crossterm` backend
directly (the popup's real PTY), fed by the real `pane.read` from Phase 1
(no static fixture needed now that the socket works). Render the scrollback
with relative line numbers and a cursor, scrollable with the original's
movement keys. Follow the terminal-setup contract in §6 exactly:
`enable_raw_mode` → hide cursor → `CrosstermBackend` → `Terminal::new` →
`terminal.clear()` on entry; reverse on exit. No alternate screen.

**Scope:** `render.rs` (ported), `main.rs` wiring it to the Phase 1 socket
read, `Cargo.toml` with `ratatui` + `crossterm` (backend enabled).

**Out of scope:** flash jump labels, selection, copy/insert, config.

**Manual test plan:**
1. `just relink`.
2. Trigger the popup on a pane with substantial scrollback.
3. Confirm the popup renders the scrollback with correct relative line
   numbers, no leftover scrollback bleeding through around the view, and
   that scrolling moves through the scrollback.
4. Quit key closes the popup cleanly with the terminal left in a sane
   state (cursor visible, no raw-mode leak).

### Phase 3 — Flash jump-to-word / jump-to-line navigation

**Prompt:** Port the original's nvim-`flash`-style jump-to-word and
jump-to-line label overlay and its input handling into `flash.rs`, layered
on the Phase 2 render. Typing the jump trigger shows label overlays over
words/lines; typing a label moves the cursor there. Verify against the
live popup, not a fixture.

**Scope:** `flash.rs` (ported), integration into the render + input loop.

**Out of scope:** selection, copy/insert, config.

**Manual test plan:**
1. `just relink`.
2. Trigger the popup, enter jump-to-word, confirm labels appear over
   words and typing a label lands the cursor on the right word.
3. Same for jump-to-line.
4. Escape/cancel returns to normal movement without a dangling overlay.

### Phase 4 — Selection + actions (copy / insert)

**Prompt:** Port the original's precise text-range selection into
`selection.rs`, and wire the two terminal actions: **copy** the selection
to the clipboard via `arboard`, and **insert** the selection back into the
source pane via `pane.send_text` (not `send_input` — see §5/§12). Insert
always targets the pane the plugin was launched from (`focused_pane_id`
from the launch context), regardless of cursor position in the view.

**Scope:** `selection.rs`, the copy/insert dispatch, `arboard` dependency.

**Out of scope:** config, keybinding, manifest actions.

**Manual test plan:**
1. `just relink`.
2. Trigger the popup, select a range, copy — confirm the exact text lands
   on the system clipboard (test on macOS and on Linux X11 *and* Wayland).
3. Select a range, insert — confirm the text appears in the source pane's
   input (the pane that was focused before the popup opened).
4. Quit cleanly.

### Phase 5 — Manifest, keybinding, config, starter template

**Prompt:** Write the real `herdr-plugin.toml`: `[[panes]]` popup with
`width`/`height` mirroring the original `size` default (`90%x85%`), plus
`[[actions]]` entries that open the popup via
`herdr plugin pane open --plugin herdr-flash --entrypoint flash --env
FLASH_PROFILE=<name>` — one action per built-in profile (e.g.
`flash-open` for the default viewport profile, `flash-deep` for a deep
scrollback profile), mirroring the sister port's action shape. Add
`config.rs` loading `profiles` + `size` from
`$HERDR_PLUGIN_CONFIG_DIR/config.toml` once per launch (missing/unset/
parse-error → built-in defaults, never crash), and ship
`config.example.toml` as the starter template. Document the keybinding
story in `doc/keybinding.md` — Herdr owns all keybindings, the plugin
never binds its own; `[[keys.command]]` has no `env` field, so profile
selection lives on the action's `command` via `--env`.

**Scope:** `herdr-plugin.toml`, `config.rs`, `config.example.toml`,
`doc/keybinding.md`, `doc/config-reference.md`, `doc/env-vars.md`.

**Out of scope:** CI, release, README install rewrite (Phase 7).

**Manual test plan:**
1. `just relink`.
2. Bind a key to `flash-open` in your Herdr config; confirm the popup
   opens with the viewport-depth profile.
3. Bind a key to `flash-deep`; confirm it opens with the deep-scrollback
   profile (more lines visible).
4. Drop a `config.toml` with a custom profile + size override; confirm it
   takes effect. Confirm a malformed `config.toml` falls back to defaults
   with a stderr message rather than crashing.

### Phase 6 — CI & first release

**Prompt:** Add `.github/workflows/ci.yml` and `release.yml` per §9. Cut
`v0.1.0` via the two-step release flow in `CLAUDE.md` (code PRs already
merged; `release/0.1.0` PR with `Cargo.toml` bump + `CHANGELOG.md` entry;
merge; tag `v0.1.0`; push tag). Confirm the release workflow builds all
three target triples, attaches `herdr-flash-<triple>.tar.gz` + `.sha256`,
and publishes both the versioned and rolling `latest` releases.

**Scope:** `ci.yml`, `release.yml`, `CHANGELOG.md`, version bump.

**Manual test plan:**
1. `ci.yml` passes on a PR (fmt/clippy/test, macOS + Linux).
2. After tagging, the release workflow publishes the three binaries and
   the rolling `latest` release.
3. `herdr plugin install codingfragments/herdr-flash --ref v0.1.0` works
   on a clean machine.

### Phase 7 — Docs pass

**Prompt:** Update `README.md` from "planned" to real install instructions
(Option A/B per §8), fill in the Docs table, and write `doc/flash-jump.md`
and `doc/use-cases.md`. Verify every doc link resolves.

**Scope:** `README.md`, `doc/flash-jump.md`, `doc/use-cases.md`.

**Manual test plan:** fresh-eyes read of README + docs follows a clean
install from zero to a working keybind with no external references needed.

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

### Flash-specific, to confirm in the Phase 1 / Phase 2 spikes

- **Mouse events in a plugin popup pane**: Herdr advertises first-class
  mouse support (click/drag/right-click) at the runtime level — worth
  checking whether a plugin popup pane can receive raw mouse events via
  `crossterm`, which could make selection nicer than the original's
  keyboard-only flash navigation. The sister port did not pursue this
  (its picker is keyboard-only); flash should spike it in Phase 2 once
  the render loop is live, since mouse-drag selection is a natural fit
  for a scrollback view. Not required for parity.
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
