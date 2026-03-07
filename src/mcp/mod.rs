pub mod handler;
pub mod types;

use std::io::{self, BufRead, Write};

use types::JsonRpcRequest;

use crate::Result;

/// Run the MCP server on stdin/stdout.
/// Reads JSON-RPC requests line by line, dispatches to handlers, writes responses.
pub async fn run_server() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();

    tracing::info!("MCP server starting on stdio");

    for line in stdin.lock().lines() {
        let line =
            line.map_err(|e| crate::TachikomaError::Mcp(format!("Failed to read stdin: {e}")))?;

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(e) => {
                let resp = types::JsonRpcResponse::error(None, -32700, format!("Parse error: {e}"));
                write_response(&stdout, &resp)?;
                continue;
            }
        };

        let response = dispatch(&request).await;

        if let Some(resp) = response {
            write_response(&stdout, &resp)?;
        }
    }

    Ok(())
}

async fn dispatch(request: &JsonRpcRequest) -> Option<types::JsonRpcResponse> {
    match request.method.as_str() {
        "initialize" => Some(handler::handle_initialize(request.id.clone())),
        "notifications/initialized" => None, // No response for notifications
        "tools/list" => Some(handler::handle_tools_list(request.id.clone())),
        "tools/call" => {
            // Tool calls would wire into actual commands here
            // For now, return a stub
            let tool_name = request
                .params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");

            let result = types::ToolResult::text(format!(
                "Tool '{tool_name}' called (stub — wire to cmd modules)"
            ));

            Some(types::JsonRpcResponse::success(
                request.id.clone(),
                serde_json::to_value(result).unwrap(),
            ))
        }
        method => Some(handler::handle_unknown_method(request.id.clone(), method)),
    }
}

fn write_response(stdout: &io::Stdout, response: &types::JsonRpcResponse) -> Result<()> {
    let json = serde_json::to_string(response)
        .map_err(|e| crate::TachikomaError::Mcp(format!("Failed to serialize response: {e}")))?;

    let mut lock = stdout.lock();
    writeln!(lock, "{json}")
        .map_err(|e| crate::TachikomaError::Mcp(format!("Failed to write response: {e}")))?;
    lock.flush()
        .map_err(|e| crate::TachikomaError::Mcp(format!("Failed to flush stdout: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dispatch_initialize() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "initialize".to_string(),
            params: serde_json::json!({}),
        };
        let resp = dispatch(&req).await.unwrap();
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn test_dispatch_notification() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "notifications/initialized".to_string(),
            params: serde_json::json!({}),
        };
        let resp = dispatch(&req).await;
        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn test_dispatch_tools_list() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(2)),
            method: "tools/list".to_string(),
            params: serde_json::json!({}),
        };
        let resp = dispatch(&req).await.unwrap();
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().len();
        assert!(tools > 0);
    }

    #[tokio::test]
    async fn test_dispatch_tools_call() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(3)),
            method: "tools/call".to_string(),
            params: serde_json::json!({"name": "list"}),
        };
        let resp = dispatch(&req).await.unwrap();
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn test_dispatch_unknown() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(4)),
            method: "nonexistent".to_string(),
            params: serde_json::json!({}),
        };
        let resp = dispatch(&req).await.unwrap();
        assert!(resp.error.is_some());
    }
}
