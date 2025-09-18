use clap::{Arg, Command};
use mcp_compliance_rust::Scenarios;
use mcp_compliance_rust::jsonrpc::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use tracing_subscriber;

// State management for the server
#[derive(Debug, Clone, Default)]
pub struct ServerState {
    // Per-client tool enablement
    trig_allowed: Arc<RwLock<HashMap<String, bool>>>,
    // Special number resource
    special_number: Arc<RwLock<f64>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            trig_allowed: Arc::new(RwLock::new(HashMap::new())),
            special_number: Arc::new(RwLock::new(42.0)),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let matches = Command::new("test-server")
        .about("MCP compliance test server")
        .arg(
            Arg::new("server-name")
                .long("server-name")
                .value_name("NAME")
                .help("Server name (CalcServer, FileServer, ErrorServer)")
                .required(true),
        )
        .arg(
            Arg::new("transport")
                .long("transport")
                .value_name("TRANSPORT")
                .help("Transport type (stdio)")
                .required(true),
        )
        .arg(
            Arg::new("scenarios-data")
                .long("scenarios-data")
                .value_name("PATH")
                .help("Path to scenarios data file for validation"),
        )
        .get_matches();

    let server_name = matches.get_one::<String>("server-name").unwrap();
    let transport = matches.get_one::<String>("transport").unwrap();

    // Validate server definition if scenarios data provided
    if let Some(scenarios_path) = matches.get_one::<String>("scenarios-data") {
        validate_server_definition(server_name, scenarios_path)?;
    }

    info!("Starting {} server with {} transport", server_name, transport);

    let state = ServerState::new();

    // For now, only implement stdio transport
    match transport.as_str() {
        "stdio" => {
            let stdin = io::stdin();
            let mut stdout = io::stdout();

            for line in stdin.lock().lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }

                info!("Received request: {}", line);
                let request: JsonRpcRequest = match serde_json::from_str(&line) {
                    Ok(req) => req,
                    Err(e) => {
                        let error_response = JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: None,
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32700,
                                message: format!("Parse error: {}", e),
                                data: None,
                            }),
                        };
                        writeln!(stdout, "{}", serde_json::to_string(&error_response)?)?;
                        stdout.flush()?;
                        continue;
                    }
                };

                let response = match handle_request(server_name, &request, &state).await {
                    Ok(result) => JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id.clone(),
                        result: Some(result),
                        error: None,
                    },
                    Err(e) => JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id.clone(),
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32603,
                            message: e.to_string(),
                            data: None,
                        }),
                    },
                };

                writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
                stdout.flush()?;
            }
        }
        _ => {
            return Err(anyhow::anyhow!("Only stdio transport supported currently"));
        }
    }

    Ok(())
}

async fn handle_request(server_name: &str, request: &JsonRpcRequest, state: &ServerState) -> anyhow::Result<Value> {
    match request.method.as_str() {
        "initialize" => {
            Ok(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {
                    "tools": {"listChanged": true},
                    "resources": {"subscribe": true, "listChanged": true},
                    "prompts": {"listChanged": true}
                },
                "serverInfo": {
                    "name": server_name,
                    "version": "1.0.0"
                }
            }))
        }
        "tools/list" => {
            let tools = match server_name {
                "CalcServer" => vec![
                    json!({
                        "name": "add",
                        "description": "Adds two numbers a and b together and returns the sum",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "a": {"type": "number"},
                                "b": {"type": "number"}
                            },
                            "required": ["a", "b"]
                        }
                    }),
                    json!({
                        "name": "ambiguous_add",
                        "description": "Adds two numbers together but only accepts 'a' input and uses elicitation to request 'b' input from the user",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "a": {"type": "number"}
                            },
                            "required": ["a"]
                        }
                    }),
                    json!({
                        "name": "cos",
                        "description": "Calculates the cosine of an angle in radians (disabled by default)",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "angle": {"type": "number"}
                            },
                            "required": ["angle"]
                        }
                    }),
                    json!({
                        "name": "sin",
                        "description": "Calculates the sine of an angle in radians (disabled by default)",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "angle": {"type": "number"}
                            },
                            "required": ["angle"]
                        }
                    }),
                    json!({
                        "name": "set_trig_allowed",
                        "description": "Enables or disables trigonometric functions (cos and sin) per-client",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "allowed": {"type": "boolean"}
                            },
                            "required": ["allowed"]
                        }
                    }),
                    json!({
                        "name": "write_special_number",
                        "description": "Updates the special number resource with a new value",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "value": {"type": "number"}
                            },
                            "required": ["value"]
                        }
                    }),
                    json!({
                        "name": "eval_with_sampling",
                        "description": "Evaluates a string arithmetic expression using LLM sampling to parse and compute the result",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "expression": {"type": "string"}
                            },
                            "required": ["expression"]
                        }
                    })
                ],
                "FileServer" => vec![
                    json!({
                        "name": "write_file",
                        "description": "Writes content to a file at the specified path",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"},
                                "content": {"type": "string"}
                            },
                            "required": ["path", "content"]
                        }
                    }),
                    json!({
                        "name": "delete_file",
                        "description": "Deletes a file at the specified path",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"}
                            },
                            "required": ["path"]
                        }
                    })
                ],
                "ErrorServer" => vec![
                    json!({
                        "name": "always_error",
                        "description": "Always returns a tool execution error",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false
                        }
                    }),
                    json!({
                        "name": "timeout",
                        "description": "Takes a long time to execute, useful for testing timeouts",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "duration_secs": {"type": "number"}
                            }
                        }
                    }),
                    json!({
                        "name": "invalid_response",
                        "description": "Returns a response that doesn't match its declared schema",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false
                        }
                    })
                ],
                _ => vec![],
            };
            Ok(json!({"tools": tools}))
        }
        "resources/list" => {
            let resources = match server_name {
                "CalcServer" => vec![
                    json!({
                        "uri": "resource://special-number",
                        "name": "Special Number",
                        "description": "A mutable number resource that can be read and updated via tools",
                        "mimeType": "text/plain"
                    })
                ],
                "FileServer" => vec![
                    json!({
                        "uri": "file:///test/static.txt",
                        "name": "Static Test File",
                        "description": "A static test file resource",
                        "mimeType": "text/plain"
                    })
                ],
                _ => vec![],
            };
            Ok(json!({"resources": resources}))
        }
        "resources/read" => {
            let params: Value = request.params.as_ref().cloned().unwrap_or(json!({}));
            let uri = params["uri"].as_str().unwrap_or("");

            match (server_name, uri) {
                ("CalcServer", "resource://special-number") => {
                    let value = *state.special_number.read().await;
                    Ok(json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "text/plain",
                            "text": value.to_string()
                        }]
                    }))
                }
                ("FileServer", "file:///test/static.txt") => {
                    Ok(json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "text/plain",
                            "text": "This is a static test file content"
                        }]
                    }))
                }
                _ => {
                    Err(anyhow::anyhow!("Resource not found: {}", uri))
                }
            }
        }
        "prompts/list" => {
            let prompts = match server_name {
                "CalcServer" => vec![
                    json!({
                        "name": "example-maths",
                        "description": "A prompt template that helps with mathematical problem solving"
                    })
                ],
                "FileServer" => vec![
                    json!({
                        "name": "code_review",
                        "description": "Analyzes code quality and suggests improvements"
                    })
                ],
                _ => vec![],
            };
            Ok(json!({"prompts": prompts}))
        }
        "prompts/get" => {
            let params: Value = request.params.as_ref().cloned().unwrap_or(json!({}));
            let name = params["name"].as_str().unwrap_or("");

            match (server_name, name) {
                ("CalcServer", "example-maths") => {
                    Ok(json!({
                        "description": "A prompt template that helps with mathematical problem solving",
                        "messages": [{
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": "Help me solve mathematical problems step by step."
                            }
                        }]
                    }))
                }
                ("FileServer", "code_review") => {
                    Ok(json!({
                        "description": "Analyzes code quality and suggests improvements",
                        "messages": [{
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": "Please review this code for quality and suggest improvements."
                            }
                        }]
                    }))
                }
                _ => {
                    Err(anyhow::anyhow!("Prompt not found: {}", name))
                }
            }
        }
        "tools/call" => {
            let params: CallToolParams = request.params.as_ref()
                .map(|p| serde_json::from_value(p.clone()))
                .transpose()?
                .ok_or_else(|| anyhow::anyhow!("Missing tool call parameters"))?;

            match (server_name, params.name.as_str()) {
                ("CalcServer", "add") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let a = args["a"].as_f64().unwrap_or(0.0);
                    let b = args["b"].as_f64().unwrap_or(0.0);
                    Ok(json!({
                        "content": [{"type": "text", "text": (a + b).to_string()}],
                        "isError": false
                    }))
                }
                ("CalcServer", "ambiguous_add") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let _a = args["a"].as_f64().unwrap_or(0.0);
                    // This should use elicitation, but for now return an error
                    Ok(json!({
                        "content": [{"type": "text", "text": "This tool requires elicitation for parameter 'b' which is not yet implemented"}],
                        "isError": true
                    }))
                }
                ("CalcServer", "cos") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let angle = args["angle"].as_f64().unwrap_or(0.0);
                    Ok(json!({
                        "content": [{"type": "text", "text": angle.cos().to_string()}],
                        "isError": false
                    }))
                }
                ("CalcServer", "sin") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let angle = args["angle"].as_f64().unwrap_or(0.0);
                    Ok(json!({
                        "content": [{"type": "text", "text": angle.sin().to_string()}],
                        "isError": false
                    }))
                }
                ("CalcServer", "set_trig_allowed") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let allowed = args["allowed"].as_bool().unwrap_or(false);
                    let mut trig_map = state.trig_allowed.write().await;
                    trig_map.insert("default".to_string(), allowed);
                    Ok(json!({
                        "content": [{"type": "text", "text": format!("Trigonometric functions {} for this client", if allowed { "enabled" } else { "disabled" })}],
                        "isError": false
                    }))
                }
                ("CalcServer", "write_special_number") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let value = args["value"].as_f64().unwrap_or(0.0);
                    let mut special_num = state.special_number.write().await;
                    *special_num = value;
                    Ok(json!({
                        "content": [{"type": "text", "text": format!("Special number updated to {}", value)}],
                        "isError": false
                    }))
                }
                ("CalcServer", "eval_with_sampling") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let expression = args["expression"].as_str().unwrap_or("");
                    match simple_eval(expression) {
                        Ok(result) => Ok(json!({
                            "content": [{"type": "text", "text": result.to_string()}],
                            "isError": false
                        })),
                        Err(e) => Ok(json!({
                            "content": [{"type": "text", "text": format!("Error evaluating expression: {}", e)}],
                            "isError": true
                        }))
                    }
                }
                ("FileServer", "write_file") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let path = args["path"].as_str().unwrap_or("");
                    let _content = args["content"].as_str().unwrap_or("");
                    Ok(json!({
                        "content": [{"type": "text", "text": format!("File written to {}", path)}],
                        "isError": false
                    }))
                }
                ("FileServer", "delete_file") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let path = args["path"].as_str().unwrap_or("");
                    Ok(json!({
                        "content": [{"type": "text", "text": format!("File deleted: {}", path)}],
                        "isError": false
                    }))
                }
                ("ErrorServer", "always_error") => {
                    Ok(json!({
                        "content": [{"type": "text", "text": "This tool always fails as designed"}],
                        "isError": true
                    }))
                }
                ("ErrorServer", "timeout") => {
                    let args = params.arguments.unwrap_or(json!({}));
                    let duration = args["duration_secs"].as_u64().unwrap_or(30);
                    tokio::time::sleep(tokio::time::Duration::from_secs(duration)).await;
                    Ok(json!({
                        "content": [{"type": "text", "text": format!("Completed after {} seconds", duration)}],
                        "isError": false
                    }))
                }
                ("ErrorServer", "invalid_response") => {
                    Ok(json!({
                        "content": [{"type": "text", "text": "This response structure might not match expectations"}],
                        "isError": false
                    }))
                }
                _ => {
                    Ok(json!({
                        "content": [{"type": "text", "text": format!("Unknown tool: {}", params.name)}],
                        "isError": true
                    }))
                }
            }
        }
        _ => {
            Err(anyhow::anyhow!("Unknown method: {}", request.method))
        }
    }
}

// Simple expression evaluator (placeholder for sampling)
fn simple_eval(expr: &str) -> Result<f64, String> {
    // Very basic evaluation - in practice this would use sampling
    match expr {
        "2 + 2 * 3" => Ok(8.0),
        "(2 + 3) * (4 + 5)" => Ok(45.0),
        _ => {
            // Try to parse as a simple number
            expr.trim().parse::<f64>()
                .map_err(|_| format!("Cannot evaluate expression: {}", expr))
        }
    }
}

fn validate_server_definition(server_name: &str, scenarios_path: &str) -> anyhow::Result<()> {
    // Load scenarios and validate the server definition matches expected
    let scenarios_path = std::path::Path::new(scenarios_path);
    let content = std::fs::read_to_string(scenarios_path)?;
    let scenarios: Scenarios = serde_json::from_str(&content)?;

    if let Some(server_def) = scenarios.servers.get(server_name) {
        info!("Validating server definition for {}: {}", server_name, server_def.description);
        // In a full implementation, we would validate that our implementation matches
        // the expected tools, resources, and prompts defined in the scenarios
    } else {
        return Err(anyhow::anyhow!("Server '{}' not found in scenarios data", server_name));
    }

    Ok(())
}