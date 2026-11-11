//! `cryzo-desktop-mcp`: a stdio MCP server (JSON-RPC, one message per line) over the
//! desktop-control crate. Any MCP client can use it, including local-model clients.

use std::io::{BufRead, Write};

use base64::Engine;
use cryzo_desktop_control::{Desktop, INSTRUCTIONS, Output, Settings, start_stop_hotkey, tools};
use serde_json::{Value, json};

fn main() -> anyhow::Result<()> {
    let mut desktop = Desktop::new(Settings::load()?)?;
    start_stop_hotkey();
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            writeln!(
                stdout,
                "{}",
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } })
            )?;
            stdout.flush()?;
            continue;
        };
        // Notifications (no id) need no reply.
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "cryzo-desktop", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                Ok(match desktop.call(name, &arguments) {
                    Ok(Output::Text(text)) => {
                        json!({ "content": [{ "type": "text", "text": text }], "isError": false })
                    }
                    Ok(Output::Image { header, jpeg }) => json!({ "content": [
                        { "type": "text", "text": header },
                        { "type": "image", "mimeType": "image/jpeg", "data": base64::engine::general_purpose::STANDARD.encode(jpeg) }
                    ], "isError": false }),
                    Err(error) => {
                        json!({ "content": [{ "type": "text", "text": format!("{error:#}") }], "isError": true })
                    }
                })
            }
            method => Err(json!({ "code": -32601, "message": format!("Unknown method {method}") })),
        };
        let reply = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        };
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
    }
    Ok(())
}
