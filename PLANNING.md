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
[§9 Ideas beyond parity](#9-ideas-beyond-parity), not built into v1.

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
input/copy actions (was host action calls, now `pane.send_input` +
`arboard`).

## 4. Architecture

```
┌─────────────────────────────────────────────┐
│ herdr-plugin.toml                            │
│  - [[panes]] entry: placement = "popup"      │
│    command = ["herdr-flash"]                 │
│    width/height ~ original "size" config     │
│  - [[keys.command]]: hotkey → plugin_action  │
└───────────────┬───────────────────────────────┘
                │ launches (real PTY)
                ▼
┌─────────────────────────────────────────────┐
│ herdr-flash binary (Rust)                    │
│                                               │
│  1. socket_client.rs                         │
│     - connect $HERDR_SOCKET_PATH             │
│     - pane.read (source pane; depth per      │
│       "profiles" config: viewport | N lines) │
│     - pane.send_input (insert selection)     │
│                                               │
│  2. render.rs (ported ~as-is)                │
│     - crossterm backend (new — previously    │
│       host-owned) + ratatui                  │
│     - relative line numbers, cursor          │
│                                               │
│  3. flash.rs (ported ~as-is)                 │
│     - jump-to-word / jump-to-line label       │
│       overlay + input handling               │
│                                               │
│  4. selection.rs                             │
│     - precise text-range selection           │
│     - copy → arboard clipboard               │
│     - insert → pane.send_input               │
└───────────────────────────────────────────────┘
```

## 5. Migration map: zellij-tile → herdr socket API

| Original (`zellij-tile`) | Herdr equivalent |
|---|---|
| Plugin registration, `register_plugin!` | `herdr-plugin.toml` manifest, `[[panes]]` entry |
| Keybind → `LaunchOrFocusPlugin` | `[[keys.command]]` → `plugin_action` |
| Host owns terminal I/O; plugin issues render calls | Plugin owns a real PTY; `crossterm` backend directly |
| Read focused pane content + scrollback depth (`profiles`) | `pane.read` (`source = "viewport"` or `"recent-unwrapped"`, depth param — see [§8](#8-open-questions)) |
| Write/paste into pane | `pane.send_input` / `pane.send_text` |
| Floating pane `size` config (`WIDTHxHEIGHT`) | Popup `width`/`height` in `[[panes]]` (cells or %) |
| Clipboard (host `Clipboard` action) | `arboard` crate directly |

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
  - Hand-rolled blocking Unix-socket client, same rationale as
    `herdr-zextract` (no async runtime needed for a foreground UI).

## 7. Portability plan (macOS + Linux)

- Target triples: `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`.
- No Windows target for now, same rationale as `herdr-zextract` (Herdr's
  Windows beta uses ConPTY / different IPC transport, out of scope).
- `crossterm` already abstracts terminal capability differences between
  macOS Terminal/iTerm2 and Linux terminal emulators; still needs manual
  verification on both (mouse support, color depth, wide-character
  rendering for the flash label overlay) before calling v1 done.
- Clipboard fallback behavior (Wayland vs X11 vs macOS) isolated behind
  `arboard` — verify Wayland clipboard behavior specifically on Linux, as
  it's historically the most inconsistent case for terminal clipboard
  tools.

## 8. Install & distribution plan

Two supported install paths, both anchored on tagged GitHub releases:

**A. Source install via `herdr plugin install`**
```sh
herdr plugin install codingfragments/herdr-flash
```
**Important — this is not automatic build detection.** Herdr does not
inspect a cloned repo and decide "this is a Rust project, run `cargo`."
It only runs a build step if the manifest explicitly declares one via
`[[build]]`, e.g.:
```toml
[[build]]
command = ["cargo", "build", "--release"]
```
`herdr plugin install` clones the repo, runs whatever `[[build]]` commands
are declared (in order, before registration), and registers the plugin.
If the manifest has no `[[build]]` section, install just clones and
registers without compiling anything — so the manifest for this repo
**must** carry the `cargo build --release` step above, and its `command`
must point at the resulting release binary path
(`target/release/herdr-flash`), not a bare `herdr-flash` (that only
resolves via `PATH`, which is Option B below).

This path requires a working Rust toolchain (`cargo`) on the machine
running `herdr plugin install` — Herdr "reports build failures but does
not install missing toolchains." Good for tracking `main` rather than a
pinned release.

Note: `herdr plugin link` (local-dev install of a directory you already
have checked out) skips `[[build]]` entirely regardless of manifest
contents — you're expected to `cargo build` your own working tree
yourself before linking.

**B. Binary install via `cargo install` from a labeled stable release**
```sh
cargo install --git https://github.com/codingfragments/herdr-flash --tag v0.1.0 herdr-flash
```
Installs the binary onto `PATH` directly from a tagged commit. Manifest
for this path:
```toml
[[panes]]
id = "flash"
placement = "popup"
command = ["herdr-flash"]   # resolved via PATH
width = "90%"
height = "85%"
```
Recommended path for most users once releases stabilize.

**C. (stretch) Prebuilt binary download**
GitHub Actions attaches prebuilt binaries per target triple to each tagged
release. A future `install.sh` could fetch the right binary directly
without compiling — nice-to-have, not required for v1.

### New-machine bootstrap (planned)

```sh
# 1. Ensure herdr itself is installed (https://herdr.dev/install.sh)
# 2. Install the plugin binary (path B, recommended once released)
cargo install --git https://github.com/codingfragments/herdr-flash --tag v0.1.0
# 3. Register the plugin with herdr (path/manifest TBD once manifest is written)
herdr plugin link ~/.cargo/... # or point manifest command at the installed binary
# 4. Bind a key in herdr config to the plugin action (was "Alt f" originally)
```
Exact `herdr plugin install`/`link` invocation to be finalized once the
manifest is written and tested against a real Herdr install.

## 9. CI / release plan (GitHub Actions)

Same shape as `herdr-zextract`'s plan — kept consistent across both repos
so CI can eventually be shared/templated if useful:

- Trigger: tag push matching `v*.*.*`.
- Matrix: `macos-14` (arm64) → `aarch64-apple-darwin`; `macos-13` (x86_64)
  → `x86_64-apple-darwin`; `ubuntu-latest` → `x86_64-unknown-linux-gnu`
  (+ `aarch64` via `cross`/QEMU or native ARM runner, TBD).
- Steps: checkout → `cargo build --release --target <triple>` → strip →
  `sha256sum` → upload as `herdr-flash-<triple>.tar.gz` (+ `.sha256`)
  release asset.
- Separate `ci.yml`: `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, on macOS + Linux runners.
- First stable tag once flash-jump + selection + copy/insert parity with
  the original is manually verified against a real Herdr session on both
  a Mac and a Linux box.

## 10. Repo layout (planned, once code starts)

```
herdr-flash/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── socket_client.rs   # herdr socket API client
│   ├── render.rs          # ported ratatui rendering, now with own crossterm backend
│   ├── flash.rs           # ported jump-to-word/line label logic
│   └── selection.rs       # copy/insert actions, OS-specific bits
├── doc/
│   └── demo.gif           # carried over / re-recorded against herdr
├── herdr-plugin.toml       # manifest (written once binary exists)
├── .github/workflows/
│   ├── ci.yml
│   └── release.yml
├── PLANNING.md             # this file
└── README.md
```

## 11. Milestones

1. **Spike**: minimal socket client that can do `pane.read` and print
   scrollback of the focused pane — validates protocol assumptions
   against a real Herdr instance (shared spike with `herdr-zextract`,
   may be worth prototyping once and copying rather than duplicating
   effort).
2. **Port rendering**: get the relative-line-number `ratatui` view running
   as a standalone terminal binary (own `crossterm` backend) fed by a
   static text fixture — no herdr integration yet, fast iteration.
3. **Port flash navigation**: bring over jump-to-word/line label overlay
   and input handling, verified against the standalone fixture.
4. **Wire up socket + actions**: real `pane.read` for content,
   `pane.send_input` for insert, `arboard` for copy.
5. **Manifest + first end-to-end run** inside a real Herdr popup pane.
6. **CI**: `ci.yml` first, then `release.yml`, cut `v0.1.0`.
7. **Docs**: update README install section from "planned" to real
   instructions once `v0.1.0` exists.

## 12. Open questions

- `pane.read` depth semantics need to be confirmed against the original
  `profiles` config (`viewport` vs `N` scrollback lines) — same question
  as `herdr-zextract`, worth resolving once for both.
- Mouse-driven selection: Herdr advertises first-class mouse support
  (click/drag/right-click) at the runtime level — worth checking whether
  a plugin pane can also receive raw mouse events via `crossterm`, which
  could make selection nicer than the original's keyboard-only flash
  navigation. Not required for parity, but worth a spike.
- Popup pane resize behavior mid-session (original supports arbitrary
  `size` config on launch; unclear if a popup can be resized live once
  open, or only sized at launch) — confirm against real Herdr behavior.
- Confirm `HERDR_PLUGIN_CONTEXT_JSON` gives the correct "source pane" id
  (same open question as `herdr-zextract`).

## 13. Ideas beyond parity

(Not for v1 — recorded so they aren't lost.)

- Mouse-assisted selection (see open question above) as an alternative to
  pure keyboard flash-jump, if Herdr's mouse support extends cleanly to
  plugin panes.
- Live scrollback (via `events.subscribe`) instead of a static snapshot,
  for panes that are still actively producing output while the flash view
  is open.

## License

MIT, matching the original `zellij-flash` project.
