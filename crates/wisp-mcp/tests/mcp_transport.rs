//! Stdio MCP transport round-trip: concurrent `tools/call` must not steal
//! each other's JSON-RPC responses.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;
use wisp_mcp::McpClient;

const ECHO_ARG: &str = "--fake-echo-mcp";

fn main() -> ExitCode {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some(ECHO_ARG) {
        return fake_echo_server();
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    match runtime.block_on(run_stdio_transport_regressions()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("stdio concurrent MCP transport failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn fake_echo_server() -> ExitCode {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return ExitCode::FAILURE;
        };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return ExitCode::FAILURE;
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request.get("method").and_then(Value::as_str);
        let result = match method {
            Some("initialize") => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake-echo", "version": "1" }
            }),
            Some("tools/list") => json!({
                "tools": [{
                    "name": "echo",
                    "description": "Echo a token after an optional delay",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "token": { "type": "string" },
                            "delay_ms": { "type": "integer" }
                        },
                        "required": ["token"]
                    }
                }]
            }),
            Some("tools/call") => {
                let arguments = request
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(json!({}));
                let delay = arguments
                    .get("delay_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if delay > 0 {
                    std::thread::sleep(Duration::from_millis(delay));
                }
                json!({
                    "content": [{ "type": "text", "text": "ok" }],
                    "structuredContent": {
                        "token": arguments.get("token").cloned().unwrap_or(Value::Null)
                    },
                    "isError": false
                })
            }
            _ => json!({}),
        };
        let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

async fn run_stdio_transport_regressions() -> Result<(), String> {
    concurrent_stdio_calls_keep_matching_ids().await?;
    cancelled_isolated_call_leaves_connection_usable().await
}

async fn concurrent_stdio_calls_keep_matching_ids() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let args = vec![ECHO_ARG.to_string()];
    let client = McpClient::launch(&executable.to_string_lossy(), &args)
        .await
        .map_err(|error| error.to_string())?;
    let slow_args = json!({ "token": "slow", "delay_ms": 180 });
    let fast_args = json!({ "token": "fast", "delay_ms": 20 });
    let slow = client.tool_call_rich("echo", &slow_args);
    let fast = client.tool_call_rich("echo", &fast_args);
    let (slow, fast) = tokio::join!(slow, fast);
    let slow = slow.map_err(|error| error.to_string())?;
    let fast = fast.map_err(|error| error.to_string())?;
    if slow
        .structured_content
        .as_ref()
        .and_then(|v| v.get("token"))
        != Some(&json!("slow"))
    {
        return Err(format!("slow call received {:?}", slow.structured_content));
    }
    if fast
        .structured_content
        .as_ref()
        .and_then(|v| v.get("token"))
        != Some(&json!("fast"))
    {
        return Err(format!("fast call received {:?}", fast.structured_content));
    }
    client.shutdown().await.map_err(|error| error.to_string())?;
    Ok(())
}

async fn cancelled_isolated_call_leaves_connection_usable() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let args = vec![ECHO_ARG.to_string()];
    let client = McpClient::launch(&executable.to_string_lossy(), &args)
        .await
        .map_err(|error| error.to_string())?;
    let hang_args = json!({ "token": "hang", "delay_ms": 400 });
    let hang = client.tool_call_rich_isolated("echo", &hang_args);
    if tokio::time::timeout(Duration::from_millis(60), hang)
        .await
        .is_ok()
    {
        let _ = client.shutdown().await;
        return Err("isolated call returned before the host timeout".into());
    }
    let recovered_args = json!({ "token": "recovered" });
    let recovered = client
        .tool_call_rich_isolated("echo", &recovered_args)
        .await
        .map_err(|error| error.to_string())?;
    if recovered
        .structured_content
        .as_ref()
        .and_then(|value| value.get("token"))
        != Some(&json!("recovered"))
    {
        let _ = client.shutdown().await;
        return Err(format!(
            "connection unusable after isolated cancel: {:?}",
            recovered.structured_content
        ));
    }
    client.shutdown().await.map_err(|error| error.to_string())?;
    Ok(())
}
