//! Hand-rolled blocking Unix-socket client for Herdr's JSON socket API.
//!
//! Herdr's socket server closes the connection after serving exactly one
//! request — reusing a connection yields `BrokenPipe` even milliseconds
//! later, regardless of idle time (confirmed live in the sister
//! `herdr-zextract` port). [`request`] therefore opens a fresh
//! `UnixStream` per call and is the *only* way anything in this plugin
//! talks to the socket, so the persistent-connection bug class can't recur
//! by construction.
//!
//! No async runtime — the plugin is a short-lived foreground UI process.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use serde_json::Value;

/// Send one newline-delimited JSON request over `socket_path` and block for
/// the matching response line, on a **fresh connection every call**.
///
/// Request shape: `{"id","method","params"}`. Response shape: `{"result"}`
/// on success (this returns the `result` value) or `{"error"}` on failure
/// (returned as an `io::Error`).
pub fn request(socket_path: &str, method: &str, params: Value) -> std::io::Result<Value> {
    let stream = UnixStream::connect(socket_path)?;
    let mut reader = BufReader::new(stream);

    let req = serde_json::json!({
        "id": format!("herdr-flash-{method}"),
        "method": method,
        "params": params,
    });
    let mut line = serde_json::to_string(&req).map_err(std::io::Error::other)?;
    line.push('\n');
    reader.get_mut().write_all(line.as_bytes())?;

    let mut response_line = String::new();
    reader.read_line(&mut response_line)?;

    let response: Value =
        serde_json::from_str(response_line.trim_end()).map_err(std::io::Error::other)?;

    if let Some(error) = response.get("error") {
        return Err(std::io::Error::other(format!(
            "herdr socket error: {error}"
        )));
    }

    response
        .get("result")
        .cloned()
        .ok_or_else(|| std::io::Error::other("herdr socket response missing \"result\""))
}
