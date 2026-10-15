# Decoration-aware copy — manual test fixtures

Fixtures for testing `feature/decoration-aware-copy` (decoration
detection on copy: box frames + line-number gutters, live dimming,
`Enter` strips, `P` copies raw) against real scrollback.

- `short.txt` — every shape in one screen: rounded box, bat gutter,
  `grep -n`, a markdown table (must *not* strip), a mermaid box (known
  false positive — `P` escape hatch).
- `long.txt` — the full pi fish block, a 30-line gutter, rg multi-file
  prefixes, boxed+numbered content, light/heavy/double frames,
  side-bars-only, and not-covered look-alikes.

## Run

```sh
# 1. Print a fixture into a pane (pane must be ≥ 90 columns wide, or
#    the boxes wrap — wrapped frames legitimately do NOT detect):
cat fixtures/short.txt        # or fixtures/long.txt

# 2. Open herdr-flash over that pane (your normal keybind), e.g.:
herdr plugin pane open --placement popup --env FLASH_PROFILE=default

# 3. Select a whole box (e.g. `l` then the gutter labels, or Space +
#    motions), watch the live feedback, then act.
```

## What to check

| # | Fixture block | Expect while selecting | Enter copies | `P` copies |
|---|---|---|---|---|
| 1 | fish box | frame + rule rows dim, footer `stripping box → N lines · P raw` | clean commands, no `│`/`╭`/`╰` | verbatim box |
| 2 | bat gutter | `N │` prefixes dim, `stripping numbers → N lines` | content only | with prefixes |
| 3 | `grep -n` | same as 2 | content only | with `N:` |
| 4 | markdown table | **no hint, no dim** | verbatim | verbatim |
| 5 | mermaid box | dims (it *is* a box) | `Start` | verbatim box |

Also worth trying: `p` (insert) over the box — inserts the stripped
text into the pane; block mode (`v`) dragged across a box — strips the
rectangle's slice; a mid-line-start stream selection of the box — no
detection (extend the selection to the whole box instead).

The same expectations are pinned by the automated tests
(`fixtures_detect_and_strip_their_fish_boxes` in `src/main.rs`,
`decor::tests` in `src/decor.rs`), so a fixture edit that breaks
detection fails CI, not just your eyes.
