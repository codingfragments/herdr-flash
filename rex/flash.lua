-- flash for Rex — the launcher.
--
-- Opens herdr-flash in a floating layer over the pane you are in:
-- scrollback view with jumps, selection, copy and insert. The binary
-- does the work; this file only finds the source pane, checks the
-- binary is usable, and starts it with the right context.
--
-- Install: copy this file to ~/.config/rex/plugins/flash.lua and add
--   require("plugins.flash")
-- to init.lua. Binding (herdr parity): ctrl+s>shift+s opens the
-- default profile (depth cycle 200/5000/unlimited/viewport). Further
-- profiles need no packaging — bind them with their name, e.g.
--   rex.bind("ctrl+s>g", "flash", { profile = "unlimited" })
-- Profiles live in ~/.config/cf-flash/config.toml under
-- [profiles.<name>] (https://github.com/codingfragments/herdr-flash,
-- doc/config-reference.md).
--
-- Self-contained: uses lib.env (login PATH, rex CLI path) when present
-- and falls back to inline equivalents otherwise.

-- ── Environment helpers (lib/env.lua when present, else inline) ──
local login_path = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
local rex_cli = "rex"
do
  local ok, env = pcall(require, "lib.env")
  if ok and env and env.path and env.path ~= "" then login_path = env.path end
  if ok and env and env.rex_cli and env.rex_cli ~= "" then
    rex_cli = env.rex_cli
  else
    local bundle = (os.getenv("HOME") or "") ..
      "/Library/Application Support/com.superlogical.rex/RexServer/Versions/Current/Rex Server.app/Contents/MacOS/rex"
    local f = io.open(bundle, "r")
    if f then f:close() rex_cli = bundle end
  end
end

-- The host-api version this launcher needs (the binary's --host-api).
local NEED_HOST_API = 1

-- POSIX sh, so it works on macOS and Linux hosts alike (the layer may
-- run on a remote host). Checks the binary, then execs it. Env comes
-- from `env` before sh, so the check and the run see the same context.
local PREFLIGHT = [[
if ! command -v herdr-flash >/dev/null 2>&1; then
  printf '\n  flash: herdr-flash not found on this host.\n\n'
  printf '  Install it:  mise use -g github:codingfragments/herdr-flash\n\n'
  printf '  Press Enter to close. '
  read -r _
  exit 0
fi
api=$(herdr-flash --host-api 2>/dev/null | head -n 1)
case "$api" in ''|*[!0-9]*) api=0 ;; esac
if [ "$api" -lt %d ]; then
  printf '\n  flash: herdr-flash is too old (host-api %%s, need %d).\n\n' "$api"
  printf '  Update it:  mise up herdr-flash\n\n'
  printf '  Press Enter to close. '
  read -r _
  exit 0
fi
exec herdr-flash
]]

-- ── Focus return on popup close ──────────────────────────────────
-- Server-side, focus returns to the source pane when the layer
-- closes (verified with zextract). The Rex *app*, though, can land its
-- own focus on the last pane of the tab instead. A one-shot watcher
-- re-focuses the source pane when the popup's block ends; it runs as a
-- detached `rex do` script via `rex.exec` + nohup (config-side
-- `rex.on` and `rex.exec.start` need a newer Rex).
local function watcher_code(popup_block, sid, source_block, source_window)
  return string.format(
    [[rex.on("block_closed", function(_, info)
  if type(info) ~= "table" or info.block_id ~= %q then return end
  local ok, v = pcall(rex.session.view, { session_id = %q })
  if ok and v and v.active_window_id == %q then
    pcall(rex.session.focus_block, { session_id = %q, block_id = %q })
  end
  rex.stop()
end)]],
    popup_block, sid, source_window, sid, source_block
  )
end

rex.action{
  name  = "flash",
  title = "flash",
  description = "Open flash over the focused pane: scrollback view with jumps, selection, copy and insert. Optional profile (depth cycle) from ~/.config/cf-flash/config.toml.",
  args  = { profile = "string?" },
  run   = function(ctx, args)
    -- The source pane: what the user is looking at, read BEFORE the
    -- layer takes focus (inside the popup, REX_BLOCK names the popup).
    local v = rex.session.view{ session_id = ctx.session_id }
    local source_block, window
    for _, w in ipairs(v.windows or {}) do
      if w.window_id == v.active_window_id then
        source_block = w.focused_block_id
        window = w.window_id
      end
    end
    if not source_block or not window then
      error("no focused pane to open over", 0)
    end

    local layer, err = rex.session.new_layer{
      session_id = ctx.session_id,
      bounds     = { 0.05, 0.075, 0.9, 0.85 },  -- herdr's 90% x 85% popup
      layout     = rex.layout.block{
        flavor  = "com.superlogical.terminal.shell",
        options = {
          command = {
            "env",
            "PATH=" .. login_path,
            "FLASH_HOST=rex",
            "REX_CLI=" .. rex_cli,
            "REX_SESSION=" .. ctx.session_id,
            "FLASH_SOURCE_BLOCK=" .. source_block,
            "FLASH_PROFILE=" .. (args.profile or "default"),
            "/bin/sh", "-c", (PREFLIGHT):format(NEED_HOST_API, NEED_HOST_API),
            "sh",
          },
          shell = "none",
        },
      },
      focus = true,
    }
    if err then error(err, 0) end
    -- Focus-return watcher (see above). The pieces are known to be
    -- free of single quotes (paths, ids, double-quoted Lua code).
    local cmdline = table.concat({
      "nohup '" .. rex_cli .. "'",
      "do",
      "-s '" .. ctx.session_id .. "'",
      "-e '" ..
        watcher_code(layer.block_ids[1], ctx.session_id, source_block, window) .. "'",
      ">/dev/null 2>&1 &",
    }, " ")
    pcall(rex.exec, { "/bin/sh", "-c", cmdline })
    return { layer_id = layer.layer_id, block_ids = layer.block_ids, source_block = source_block }
  end,
}

-- ── Binding (herdr parity: prefix+shift+S) ────────────────────────
rex.bind("ctrl+s>shift+s", "flash", { profile = "default" })
