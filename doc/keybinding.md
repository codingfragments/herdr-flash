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
inside the popup for a full keybinding dialog. See
[`doc/use-cases.md`](use-cases.md) for worked walkthroughs.
