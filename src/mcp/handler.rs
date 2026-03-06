use serde_json::json;

use super::types::*;

pub fn handle_initialize(id: Option<serde_json::Value>) -> JsonRpcResponse {
    let result = InitializeResult {
        protocol_version: "2024-11-05".to_string(),
        capabilities: Capabilities {
            tools: ToolsCapability { list_changed: false },
        },
        server_info: ServerInfo {
            name: "tachikoma".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    };

    JsonRpcResponse::success(id, serde_json::to_value(result).unwrap())
}

pub fn handle_tools_list(id: Option<serde_json::Value>) -> JsonRpcResponse {
    let tools = vec![
        Tool {
            name: "spawn".to_string(),
            description: "Spawn or reconnect to a VM for a git branch".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "branch": {
                        "type": "string",
                        "description": "Branch name (optional, defaults to current)"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory (required)"
                    }
                },
                "required": ["cwd"]
            }),
        },
        Tool {
            name: "exec".to_string(),
            description: "Execute a command in a running VM".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "vm_name": {
                        "type": "string",
                        "description": "VM name"
                    },
                    "command": {
                        "type": "string",
                        "description": "Command to execute"
                    }
                },
                "required": ["vm_name", "command"]
            }),
        },
        Tool {
            name: "list".to_string(),
            description: "List all VMs".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo": {
                        "type": "string",
                        "description": "Filter by repository name"
                    }
                }
            }),
        },
        Tool {
            name: "status".to_string(),
            description: "Get status of a VM".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "vm_name": {
                        "type": "string",
                        "description": "VM name"
                    }
                },
                "required": ["vm_name"]
            }),
        },
        Tool {
            name: "halt".to_string(),
            description: "Stop a running VM".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "vm_name": {
                        "type": "string",
                        "description": "VM name"
                    }
                },
                "required": ["vm_name"]
            }),
        },
        Tool {
            name: "destroy".to_string(),
            description: "Destroy a VM and remove its state".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "vm_name": {
                        "type": "string",
                        "description": "VM name"
                    }
                },
                "required": ["vm_name"]
            }),
        },
    ];

    JsonRpcResponse::success(
        id,
        json!({ "tools": serde_json::to_value(tools).unwrap() }),
    )
}

pub fn handle_unknown_method(
    id: Option<serde_json::Value>,
    method: &str,
) -> JsonRpcResponse {
    JsonRpcResponse::error(id, -32601, format!("Method not found: {method}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_initialize() {
        let resp = handle_initialize(Some(json!(1)));
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "tachikoma");
    }

    #[test]
    fn test_handle_tools_list() {
        let resp = handle_tools_list(Some(json!(2)));
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(!tools.is_empty());

        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"spawn"));
        assert!(names.contains(&"exec"));
        assert!(names.contains(&"list"));
        assert!(names.contains(&"halt"));
        assert!(names.contains(&"destroy"));
    }

    #[test]
    fn test_handle_unknown_method() {
        let resp = handle_unknown_method(Some(json!(3)), "nonexistent");
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }
}
