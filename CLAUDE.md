# herdr-flash — Claude Code conventions

Mirrors the gitflow used in the original `zellij-flash` repo, adapted for
this port's binary-release model.

## Gitflow

- **Always create a branch before making any change or commit** — even one-line fixes.
- Branch prefixes:
  - `bug/` — bug fixes
  - `feature/` — new features
  - `phase/` — larger milestone / multi-commit work (e.g. each item in
    PLANNING.md §11 Milestones)
  - `release/<version>` — release prep (version bump + CHANGELOG)
- If the type is unclear, ask before creating the branch.
- Every branch lands via PR to `main` — no direct commits to `main`.
- Stay on the working branch until the PR is explicitly merged; switch back
  to `main` only after merge.

## Workflow

- Commit frequently within a branch as work progresses.
- Do not push without user approval — summarise what's testable and wait
  for "push" / "looks good".
- End each phase with: what to test, how to trigger it, what works vs
  what's still a stub.
- Current phase: PLANNING.md's open questions (§12) are resolved
  (inherited from the sister `herdr-zextract` port, confirmed live
  against Herdr 0.8.0) — implementation branches now follow the phase
  sequence in PLANNING.md §11 Implementation phases, one `phase/` branch/PR
  per phase, in order. The two flash-specific items in §12 (mouse events
  in a plugin popup, live popup resize) get confirmed in the Phase 1 / Phase 2
  spikes rather than blocking the start.

## Release process

Two-step merge flow, same shape as the original Zellij plugin:

1. **Code PRs** (`bug/`, `feature/`, `phase/`) — code changes only; merge
   first.
2. **Release PR** (`release/<x.y.z>`) — separate branch/PR containing:
   - `Cargo.toml` version bump (semver: patch/minor/major)
   - `CHANGELOG.md` entry
3. Merge the release PR to `main`.
4. Tag the resulting merge commit: `git tag v<x.y.z> && git push origin v<x.y.z>`
5. Pushing the tag triggers `.github/workflows/release.yml`, which builds
   release binaries for all target triples (see PLANNING.md §9), computes
   SHA-256 checksums, and publishes the GitHub release automatically.

> Never push a `v*.*.*` tag from a feature branch or before the release PR
> is merged.

## Project conventions

- Target platforms: macOS (arm64 + x86_64) and Linux (x86_64 + aarch64) —
  no Windows support planned.
- Versioning follows semver.
- Once a manifest exists, plugin registration/config lives in Herdr's own
  config, not this repo — this repo owns the binary and its
  `herdr-plugin.toml` only.
