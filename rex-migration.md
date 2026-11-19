# Rex migration plan — one binary, two hosts, shared `muxhost` crate

> **Status: planned, not started.** This is the active plan for this repo.
> Agents: read this file before starting work, follow the phases in order
> (one `phase/rex-N-…` branch + PR per phase, see CLAUDE.md gitflow), and
> tick the checkboxes / update the "Status" lines as phases land.
>
> **Dependency:** the sister repo `herdr-zextract` goes first — its
> `rex-migration.md` builds the `Host` trait + Rex backend in-repo. This
> plan starts once zextract's Rex backend works live (its Phase 3 + 4),
> and then **extracts that host layer into a shared crate** that both
> tools (and later `herdr-nav`) depend on.

## Goal

Make `herdr-flash` serve **both** Herdr and Rex (superlogical's terminal
multiplexer, `com.superlogical.rex`) from the **same binary, same config,
same release**. Herdr keeps working exactly as today; Rex gets a thin Lua
launcher. The multiplexer glue lives in a shared crate, not in this repo.

```
         herdr-flash                      herdr-zextract
   (scrollback view, jumps,          (match extraction, picker)
    selection, decor strip)
              └──────────────┬──────────────┘
                     muxhost (shared crate)
               Host trait + capabilities
            ┌──────────────┴──────────────┐
       HerdrHost (socket)          RexHost (rex CLI)
              │                            │
     herdr-plugin.toml             ~/.config/rex/plugins/flash.lua
```

## Decisions already made

- **Keep Rust, don't go Lua-only.** Rex's config runtime is gopher-lua
  (Lua 5.1, no regex, no `io.popen`, no UI widgets). Any interactive UI
  must be a program in a floating layer anyway — the ratatui app stays.
- **Shared host crate (option B)** over copy-per-repo (A) or a monorepo
  (C). Rule of two: zextract designs the trait in-repo first; this plan's
  Phase 1 moves it into `muxhost`, extended by flash's needs, and
  switches zextract over too.
- **Distribution:** existing GitHub releases (aarch64-apple-darwin,
  x86_64/aarch64-unknown-linux-gnu) via **mise's GitHub backend**, pinned
  in the user's dotfiles:
  ```toml
  [tools]
  "github:codingfragments/herdr-flash" = "0.5.0"
  ```
  Verified: `mise ls-remote github:codingfragments/herdr-flash` resolves
  `0.2.1 … 0.4.0` (plus the rolling `latest` tag — see Phase 5).
  That is the **Rex** install path.
- **herdr install stays self-sufficient (option B, decided 2026-11 for
  all plugins, see `../herdr-zextract/rex-migration.md` "Decisions"):**
  `herdr plugin install codingfragments/herdr-flash` stays a single
  command and needs no mise. Its `[[build]]` runs `bin/fetch-binary`
  (download the release binary matching the checked-out version into
  `target/release/`, sha256-checked, fall back to `cargo build
  --release`), and the pane starts `bin/flash-launch` (prefer the
  plugin's own binary, then `herdr-flash` on `PATH`). herdr and Rex each
  keep their own copy; update both. Reuse zextract's scripts, renamed.
- **Scroll-follow on Rex: degrade to `off`** (current default anyway).
  Rex scrolls client-side, per GUI client; the server API exposes no
  viewport scroll offset. Revisit if Rex adds one (Phase 5 feature
  request note).
- **Why flash matters on Rex:** Rex has no keyboard copy-mode — only
  `client.find.*`, `client.prompt.*`, `client.scroll.page_*`,
  `pane.select_all`, mouse selection. flash fills a real gap.

## Host-coupled surface today (all in `src/main.rs`)

| Today (herdr) | Function | Rex equivalent |
|---|---|---|
| `pane.read` `format=ansi`, `visible` / `recent_unwrapped` (+`lines`) | `read_scrollback` | `rex capture -s S -b B --format=vt --unwrap`; `lines` cap applied in Rust; viewport = last *rows* lines (`block call … size`) |
| `pane.get` → `scroll` (offset_from_bottom, viewport_rows) | `read_pane_scroll` | **none** — capability absent → `None` (existing bottom-anchored fallback) |
| `pane.read` `visible`, plain text | `read_visible_text` (Content scroll-follow) | not meaningful without scroll offset → capability absent |
| `pane.send_text` | `State::do_insert` | `rex send -s S -b B -- TEXT` |
| `HERDR_PLUGIN_CONTEXT_JSON` / `HERDR_ACTIVE_PANE_ID` | `launch_context` | env from the Lua launcher (same scheme as zextract): `FLASH_SOURCE_BLOCK`, `REX_SESSION` (inside the popup `REX_BLOCK` is the **popup itself**) |
| `HERDR_SOCKET_PATH` | `main`, `State.socket_path` | `REX_CLI` (absolute rex CLI path; Rex server's PATH lacks it) |
| `$HERDR_PLUGIN_CONFIG_DIR/config.toml` | `config::load` | neutral `~/.config/flash/config.toml` (Phase 2) |
| `KEY_DEBOUNCE` (40 ms, herdr popup = legacy keyboard mode) | event loop | probably unnecessary on Rex — make it host-dependent / configurable (spike S2) |

Verified 2026-11 against a live Rex block: `--format=vt --unwrap` output
is line-structured — SGR (`ESC[…m`) plus a few `ESC[nC` (cursor-forward,
run-length spaces), CR/LF line ends. `parse_ansi_lines` needs to expand
`CSI n C` to `n` spaces and ignore other non-SGR CSI; see spike S1 for
edge cases.

## Phases

### Phase 1 — `muxhost` crate + `HerdrHost` adoption (pure refactor) — `phase/rex-1-muxhost`

Status: ☑ done (2026-11-19; the crate carries the extension sketch
below verbatim)

- [x] Create repo `codingfragments/muxhost` — library crate, MIT, CI on
  Blacksmith runners, releases are plain tags. Seeded with zextract's
  `src/host/`; zextract switched to it (zextract#48, patch release
  v0.7.1).
- [x] Extend the trait for flash, as **optional capabilities** so hosts
  without them stay honest (as shipped in muxhost v0.1.0):
  ```rust
  pub enum ReadSource { Viewport, Scrollback }
  pub enum ReadFormat { Plain, Ansi }            // Rex: text | vt
  pub struct ScrollInfo { pub offset_from_bottom: u64, pub max_offset: u64, pub viewport_rows: u64 }

  pub trait Host {
      fn name(&self) -> &'static str;
      fn source(&self) -> &SourceRef;
      fn read_pane(&self, pane: &str, src: ReadSource, fmt: ReadFormat, lines: Option<u32>) -> Result<String, String>;
      fn list_tab_panes(&self) -> Result<Vec<PaneInfo>, String>;
      fn insert_text(&self, pane: &str, text: &str) -> Result<(), String>;
      fn scroll_info(&self, _pane: &str) -> Result<Option<ScrollInfo>, String> { Ok(None) }
      fn caps(&self) -> Caps;                     // e.g. caps.scroll_follow, caps.legacy_keyboard
  }
  pub fn detect() -> Result<Box<dyn Host>, String>;
  ```
- [x] Clipboard helper in `muxhost` (arboard, OSC 52 fallback) — both
  tools copy; kept in one place.
- [x] herdr-flash: depends on `muxhost` (tag `v0.1.0`);
  `read_scrollback` → `read_pane(Ansi)`, `read_visible_text` →
  `read_pane(Viewport, Plain)`, `read_pane_scroll` → `scroll_info()`
  (`PaneScrollInfo` is an alias of `ScrollInfo`), `do_insert` →
  `insert_text`, `launch_context` + `HERDR_SOCKET_PATH` →
  `muxhost::detect("FLASH")`, arboard copies → `muxhost::clipboard`
  (OSC 52 fallback included, arboard dep dropped). `State` holds
  `Option<Box<dyn Host>>`; `socket_client.rs` deleted.
- [x] herdr-zextract: switched to `muxhost` (zextract#48; v0.7.1).
- [x] No behavior change under Herdr; 110 tests green (scroll-anchor
  tests unchanged), clippy/fmt clean.

### Phase 2 — neutral config location — `phase/rex-2-config-path`

Status: ☐ not started

- [x] Lookup order (same scheme as zextract; `config::locate`, pure
  and unit-tested): `$FLASH_CONFIG` → `$XDG_CONFIG_HOME/flash/config.toml`
  (absolute only) / `~/.config/flash/config.toml` → legacy
  `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.
- [x] `[host.<name>]` override tables for `scroll_follow` and the new
  `key_debounce_ms` (root-level scalars, replacing the root values for
  that host only; other keys warn). `key_debounce_ms` replaces the
  hard-coded 40 ms `KEY_DEBOUNCE` — the event loop now reads the
  config, so Rex can set 0 via `[host.rex]` when its keyboard mode
  delivers distinct events (spike S2 pending).
- [x] Docs: `doc/config-reference.md` (lookup order, debounce,
  host-override sections), `doc/env-vars.md`, README,
  `config.example.toml` header. 117 tests green, clippy/fmt clean.

### Phase 3 — Rex support in flash — `phase/rex-3-rex-host`

Status: ☑ done (2026-11-19, `phase/rex-3-rex-host`) — live-tested in a
scratch Rex session

- [x] `RexHost::read_pane(…, Ansi, …)` → `capture --format=vt --unwrap`
  (in `muxhost` v0.1.0 since the extraction; verified live: an
  RGB-styled line renders with its exact color inside the popup).
- [x] `vt_prepare` before `ansi-to-tui` (which *drops* cursor-forward,
  confirmed by probe): `CSI n C` → n spaces, SGR kept verbatim, OSC
  dropped (hyperlink *text* stays visible), other CSI and charset
  escapes dropped. 4 unit tests (expansion, SGR, OSC/CSI shapes,
  column alignment through `parse_ansi_lines`). The recorded full-
  fixture set (cleared panes, full-screen apps, wide chars) stays open
  for Phase 4 — the launcher's real use surfaces anything the unit
  shapes miss.
- [x] Scroll-follow: `main` forces `Off` when `!caps.scroll_follow`
  (Rex scrolls client-side; the modes need the host's scroll offset).
- [x] Key debounce: `key_debounce_ms` is `Option<u64>` — unset means
  `caps.legacy_keyboard` decides (herdr → 40 ms; a host with distinct
  key events → 0); explicit root / `[host.<name>]` values win. Spike
  S2 (Rex's keyboard behavior in a floating layer) is checked when
  Phase 4's launcher lands.
- [ ] Shift-Enter as an insert alias: pending spike S2.
- [x] `--version` and `--host-api` (`HOST_API` = 1), answered before
  host detection.
- [x] **Herdr install, option B** (copied from zextract's Phase 3):
  `bin/fetch-binary` (`herdr-flash-<triple>.tar.gz`,
  `FLASH_BUILD=source|download` forces one way) and `bin/flash-launch`
  (plugin binary → PATH → install hint; pins `FLASH_HOST=herdr`);
  manifest `[[build]]` / `[[panes]]` updated. The live
  `herdr plugin install --ref` test happens at the v0.5.0 release,
  like zextract.

Live-tested under Rex (scratch session, binary run in a split with the
launcher's env): popup opens over the source pane, vt colors render,
line numbers, `k`/`Space`/`$` motion + selection, `p` inserted the
selection into the source block's prompt and closed the popup.

### Phase 4 — Rex launcher (`rex/flash.lua`) — `phase/rex-4-lua-launcher`

Status: ☐ not started

Reference implementation shipped in this repo as `rex/flash.lua`; the
user copies/symlinks it to `~/.config/rex/plugins/flash.lua` and adds
`require("plugins.flash")` to `init.lua`. Same structure as zextract's
`rex/zextract.lua` (and the user's `~/.config/rex/plugins/edit_scrollback.lua`).

- [ ] Action `flash{ profile = "string?" }`:
  1. `rex.session.view` → active window → `focused_block_id`
     **before** the layer takes focus.
  2. Floating layer, bounds `{0.05, 0.075, 0.9, 0.85}` (herdr 90 %×85 %),
     `shell = "none"`, command
     `env PATH=<login PATH> FLASH_HOST=rex REX_CLI=<abs> REX_SESSION=<sid> \
      FLASH_SOURCE_BLOCK=<b> FLASH_PROFILE=<p> herdr-flash`.
  3. Preflight: `command herdr-flash` (hint
     `mise use -g github:codingfragments/herdr-flash`), `--host-api` ≥
     expected (hint `mise up herdr-flash`).
- [ ] Bindings (herdr parity, prefix `ctrl+s`): `ctrl+s>shift+s` →
  `default`; further depth profiles as the user's herdr config had them
  (`200`, `2000`, …) via `rex.bind(key, "flash", {profile = "…"})`.
- [ ] Docs: `doc/rex.md`, README "Hosts" section.

### Phase 5 — distribution polish — `phase/rex-5-distribution`

Status: ☐ not started

- [ ] Rolling `latest` tag/release: **keep it** (option B: `herdr plugin
  install --ref latest` still uses it, and `bin/fetch-binary` resolves
  the matching release from that ref's `Cargo.toml`). Document "pin
  exact versions in mise" instead of `= "latest"`.
- [ ] Verify `mise use -g github:codingfragments/herdr-flash@<ver>` on
  macOS arm64 + one Linux host.
- [ ] README Install rewritten: "Herdr" (`herdr plugin install`, no Rust
  needed for tagged versions) and "Rex" (mise), `cargo install --git` as
  source fallback.
- [ ] Note for later: ask superlogical for a client/viewport scroll-offset
  API (would re-enable scroll-follow on Rex).

### Phase 6 — release `v0.5.0` — `release/0.5.0`

Status: ☐ not started

- [ ] Version bump + CHANGELOG ("Rex support", "muxhost", "neutral config
  path", "herdr plugin install downloads the release binary"). Tagging is automatic via
  `tag-on-release-merge.yml` (see CLAUDE.md release process).
- [ ] Update the user's Rex `migration-plan.md` (`~/.config/rex/`,
  dotfiles repo — separate PR): flash from "impossible" to "done";
  `prefix+shift+S` binding live.

## Spike checklist (answer early, record results here)

| # | Question | Result |
|---|---|---|
| S1 | `--format=vt --unwrap` edge cases: after `clear`, after a full-screen app exited, wide chars, OSC 8 hyperlinks — still line-structured SGR + `CSI n C` only? Latency on ~100k lines? | ☐ |
| S2 | Keyboard inside a Rex floating-layer terminal: legacy vs kitty protocol? Duplicate key events (is debounce needed)? Shift-Enter distinct from Enter? Mouse events (flash's §12 item)? | ☐ |
| S3 | Focus returns to the source block after the layer closes (insert lands right) — shared with zextract S1 | ☐ |
| S4 | OSC 52 clipboard from a floating layer, local + remote — shared with zextract S2 | ☐ |
| S5 | Live resize of the floating layer: does the ratatui app get SIGWINCH / correct size? | ☐ |
| S6 | Any Rex API (server or client action) that reveals the client's scroll position? (expected: no) | ☐ |
| S7 | `muxhost` as git dependency: CI in both tool repos fetches it fine (public repo, tag pin) | ☐ |

## Rex facts an agent needs (verified 2026-11, rex 0.1.0-preview.19, api rev 14)

- CLI: `rex capture -s S -b B [--unwrap] [--trim] [--format text|html|vt]`,
  `rex send -s S -b B TEXT`, `rex block ls -s S [-w W] --json`,
  `rex session inspect S --json` (`active_window_id`,
  `windows[].window_id`, `windows[].focused_block_id`),
  `rex block call -s S -b B size|title|pwd|modes`.
  Server API: `com.superlogical.terminal.format` (`format`, `unwrap`,
  `trim`, `max_bytes` + `blob_read` for big outputs).
- Inside a Rex terminal `REX_SESSION` / `REX_BLOCK` are set; in the popup
  `REX_BLOCK` = the popup's own block.
- Rex server runs with launchd's minimal PATH; rex CLI at
  `~/Library/Application Support/com.superlogical.rex/RexServer/Versions/Current/Rex Server.app/Contents/MacOS/rex`
  (Lua side resolves it, pass as `REX_CLI`).
- Lua config: `rex.action{name,title,args,run}`, `rex.bind(key, action, args?)`,
  `rex.session.view`, `rex.session.new_layer{bounds={x,y,w,h}, layout=rex.layout.block{flavor="com.superlogical.terminal.shell", options={command=argv, shell="none"}}, focus=true}`,
  `rex.exec{argv}`, `rex.json`. No UI widgets, no regex, no `io.popen`.
- Launcher pattern references: `~/.config/rex/plugins/edit_scrollback.lua`,
  `~/.config/rex/lib/popup.lua`, `~/.config/rex/lib/env.lua`;
  sister plan: `../herdr-zextract/rex-migration.md`.
- Headless test recipe: `rex new selftest && rex do -s selftest selftest.lua && rex session kill selftest`.

## Out of scope

- Windows, Intel-mac release binaries (unchanged policy).
- `herdr-nav` port — should adopt `muxhost` afterwards with its own plan.
- Monorepo merge of the plugins (option C) — revisit only if `muxhost`
  version bumps across three repos become a real burden.
