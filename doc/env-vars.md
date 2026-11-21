# Environment variable reference

`herdr-flash` reads exactly one environment variable at launch:

| Variable | Purpose | Default |
| --- | --- | --- |
| `FLASH_PROFILE` | Selects a named profile — a scrollback-depth cycle list — defined under `[profiles.<name>]` in your own `config.toml` | `default` |

That's the whole surface. The depth cycle (e.g. `["200", "5000", "unlimited", "viewport"]`) used to be the `profiles` config key in the original `zellij-flash` plugin, passed per-keybind via the `configuration {}` block. On Herdr, it lives in `[profiles.<name>]` blocks in **your own** `config.toml`, keyed by the profile name a keybind's launcher action selects via `FLASH_PROFILE`. Full schema and the built-in `default` profile that works with zero config:
[`doc/config-reference.md`](config-reference.md#profilesname--per-keybind-depth-cycle).

`FLASH_PROFILE` itself is set on the launcher action's `command` in
`herdr-plugin.toml`, not on the `[[keys.command]]` binding — confirmed
directly: `[[keys.command]]` (even `type = "plugin_action"`) has no
`env`, `configuration`, or `args` field; `herdr config check` rejects
all three as unknown keys. See
[`doc/keybinding.md`](keybinding.md) for the full binding walkthrough.

## Other environment variables

These are set by Herdr itself for all plugin panes, not by this plugin's
config:

| Variable | Set by | Purpose |
| --- | --- | --- |
| `HERDR_SOCKET_PATH` | Herdr | Unix socket path for the Herdr API (`pane.read`, `pane.send_text`). Required. |
| `HERDR_PLUGIN_CONTEXT_JSON` | Herdr | Launch context: `focused_pane_id`, `focused_pane_cwd`, `tab_id`, `workspace_id`, `selected_text`, `clicked_url`, `invocation_source`. The plugin reads `focused_pane_id` to know which pane to grab scrollback from and insert text into. |
| `HERDR_PLUGIN_CONFIG_DIR` | Herdr | Legacy config dir: the plugin reads its `config.toml` only after `~/.config/cf-flash/config.toml` (and `$FLASH_CONFIG`) — see [`config-reference.md`](config-reference.md). |
| `FLASH_CONFIG` | this plugin | An explicit config file, used instead of the normal lookup (see [`config-reference.md`](config-reference.md)). |
| `FLASH_PROFILE` | this plugin | Selects the named `[profiles.<n>]` depth cycle at launch. |
| `FLASH_HOST` | this plugin | Forces the multiplexer host (`herdr`/`rex`) instead of detection; set by launchers. |
| `FLASH_CONFIG` | this plugin | An explicit config file, used instead of the normal lookup. |
| `FLASH_SOURCE_BLOCK` | this plugin (Rex) | The block to grab from and insert into; set by the Rex launcher. |

The binary also answers two flags before doing anything else: `--version`
and `--host-api` (an integer the Rex launcher checks).

`HERDR_ACTIVE_PANE_ID` is a fallback for manual dev-testing when
`HERDR_PLUGIN_CONTEXT_JSON` is not set (e.g. running the binary outside
a real Herdr plugin-pane invocation).
