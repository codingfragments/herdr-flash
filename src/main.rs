//! `herdr-flash` — Phase 1: socket client + raw popup echo.
//!
//! Smallest possible binary proving the popup-and-socket plumbing works
//! end-to-end: read the launch context for `focused_pane_id`, call
//! `pane.read` over `$HERDR_SOCKET_PATH`, and print the raw scrollback
//! text into the popup pane. No rendering, no flash nav, no ratatui yet.

mod socket_client;

/// Launch context: which pane this popup was opened relative to.
struct LaunchContext {
    focused_pane_id: String,
}

/// Reads the launch context from `HERDR_PLUGIN_CONTEXT_JSON` (set by Herdr
/// for a real plugin-pane invocation). Falls back to `HERDR_ACTIVE_PANE_ID`
/// for manual dev-testing via a `[[keys.command]]` custom-command popup
/// (same fallback shape as the sister `herdr-zextract` port).
fn launch_context() -> Result<LaunchContext, String> {
    if let Ok(context_json) = std::env::var("HERDR_PLUGIN_CONTEXT_JSON") {
        let context: serde_json::Value = serde_json::from_str(&context_json)
            .map_err(|e| format!("invalid context JSON: {e}"))?;
        let focused_pane_id = context
            .get("focused_pane_id")
            .and_then(|v| v.as_str())
            .ok_or(
                "context JSON has no focused_pane_id (nothing was focused before this popup opened)",
            )?
            .to_string();
        return Ok(LaunchContext { focused_pane_id });
    }
    let focused_pane_id = std::env::var("HERDR_ACTIVE_PANE_ID").map_err(|_| {
        "neither HERDR_PLUGIN_CONTEXT_JSON nor HERDR_ACTIVE_PANE_ID is set".to_string()
    })?;
    Ok(LaunchContext { focused_pane_id })
}

/// Read the source pane's scrollback via `pane.read` with
/// `source = "recent_unwrapped"` and return the text at `result.read.text`.
fn read_scrollback(socket_path: &str, pane_id: &str) -> Result<String, String> {
    let params = serde_json::json!({
        "pane_id": pane_id,
        "source": "recent_unwrapped",
    });
    let result = socket_client::request(socket_path, "pane.read", params)
        .map_err(|e| format!("pane.read failed: {e}"))?;
    result
        .get("read")
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "pane.read response had no \"read.text\" field".to_string())
}

fn run() -> Result<(), String> {
    let ctx = launch_context()?;
    let socket_path = std::env::var("HERDR_SOCKET_PATH")
        .map_err(|_| "HERDR_SOCKET_PATH is not set".to_string())?;
    let text = read_scrollback(&socket_path, &ctx.focused_pane_id)?;
    print!("{text}");

    // Phase 1 verification affordance: keep the popup open so the
    // scrollback can be read and the pane confirmed. Removed in Phase 2
    // when the render loop's own event loop (Esc to quit) takes over —
    // same shape as the sister herdr-zextract port's Phase 1 "press
    // Enter to close" prompt, which it dropped in 0.1.1 once the picker
    // existed.
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "\n\n--- press Enter to close ---");
    let _ = std::io::stdout().flush();
    let mut _line = String::new();
    let _ = std::io::stdin().read_line(&mut _line);
    Ok(())
}

fn main() {
    if let Err(message) = run() {
        eprintln!("herdr-flash error: {message}");
    }
}
