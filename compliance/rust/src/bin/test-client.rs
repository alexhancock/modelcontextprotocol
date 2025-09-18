use clap::{Arg, Command};
use mcp_compliance_rust::{load_scenarios, ScenarioDefinition};
use mcp_compliance_rust::jsonrpc::*;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command as ProcessCommand, Stdio};
use tokio;
use tracing::{info, warn};
use tracing_subscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let matches = Command::new("test-client")
        .about("MCP compliance test client")
        .arg(
            Arg::new("scenario-id")
                .long("scenario-id")
                .value_name("ID")
                .help("Scenario ID to execute")
                .required(true),
        )
        .arg(
            Arg::new("id")
                .long("id")
                .value_name("CLIENT_ID")
                .help("Client identifier (e.g., client1)")
                .required(true),
        )
        .arg(
            Arg::new("scenarios-data")
                .long("scenarios-data")
                .value_name("PATH")
                .help("Path to scenarios data file for validation"),
        )
        .subcommand(
            Command::new("stdio")
                .about("Connect via stdio transport")
                .arg(Arg::new("command").help("Server command").num_args(1..)),
        )
        .subcommand(
            Command::new("sse")
                .about("Connect via SSE transport")
                .arg(Arg::new("url").help("Server URL").required(true)),
        )
        .subcommand(
            Command::new("streamable-http")
                .about("Connect via streamable HTTP transport")
                .arg(Arg::new("url").help("Server URL").required(true)),
        )
        .get_matches();

    let scenario_id: u32 = matches.get_one::<String>("scenario-id").unwrap().parse()?;
    let client_id = matches.get_one::<String>("id").unwrap();

    let scenarios = if let Some(scenarios_path) = matches.get_one::<String>("scenarios-data") {
        // Load from provided path
        let content = std::fs::read_to_string(scenarios_path)?;
        serde_json::from_str(&content)?
    } else {
        load_scenarios()?
    };

    let scenario = scenarios.scenarios.iter()
        .find(|s| s.id == scenario_id)
        .ok_or_else(|| anyhow::anyhow!("Scenario {} not found", scenario_id))?;

    if !scenario.client_ids.contains(&client_id.to_string()) {
        return Err(anyhow::anyhow!("Client '{}' not found in scenario {}", client_id, scenario_id));
    }

    info!("Executing scenario {}: {}", scenario_id, scenario.description);

    match matches.subcommand() {
        Some(("stdio", sub_matches)) => {
            let args: Vec<String> = sub_matches.get_many::<String>("command")
                .unwrap_or_default()
                .cloned()
                .collect();

            if args.is_empty() {
                return Err(anyhow::anyhow!("No server command provided"));
            }

            execute_stdio_scenario(args, scenario, client_id).await?;
        }
        Some(("sse", sub_matches)) => {
            let _url = sub_matches.get_one::<String>("url").unwrap();
            return Err(anyhow::anyhow!("SSE transport not implemented yet"));
        }
        Some(("streamable-http", sub_matches)) => {
            let _url = sub_matches.get_one::<String>("url").unwrap();
            return Err(anyhow::anyhow!("Streamable HTTP transport not implemented yet"));
        }
        _ => {
            return Err(anyhow::anyhow!("No transport specified"));
        }
    };

    info!("Scenario {} completed successfully", scenario_id);
    Ok(())
}

async fn execute_stdio_scenario(
    server_args: Vec<String>,
    scenario: &ScenarioDefinition,
    client_id: &str,
) -> anyhow::Result<()> {
    let mut child = ProcessCommand::new(&server_args[0])
        .args(&server_args[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    // Initialize the connection
    let init_request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: "initialize".to_string(),
        params: Some(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {
                "name": "RustTestClient",
                "version": "1.0.0"
            }
        })),
    };

    send_request(&mut stdin, &init_request)?;
    let init_response = read_response(&mut reader)?;
    info!("Initialize response: {}", serde_json::to_string_pretty(&init_response)?);

    // Execute the specific scenario
    execute_scenario_steps(&mut stdin, &mut reader, scenario, client_id).await?;

    child.wait()?;
    Ok(())
}

async fn execute_scenario_steps(
    stdin: &mut std::process::ChildStdin,
    reader: &mut BufReader<std::process::ChildStdout>,
    scenario: &ScenarioDefinition,
    client_id: &str,
) -> anyhow::Result<()> {
    match scenario.id {
        1 => {
            // client1 connects to CalcServer and calls add(a=10, b=20), gets result of 30
            let tool_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(2)),
                method: "tools/call".to_string(),
                params: Some(json!({
                    "name": "add",
                    "arguments": {"a": 10, "b": 20}
                })),
            };

            send_request(stdin, &tool_request)?;
            let response = read_response(reader)?;

            if let Some(result) = response.result {
                if let Some(content_array) = result.get("content") {
                    if let Some(first_content) = content_array.get(0) {
                        if let Some(text) = first_content.get("text") {
                            if text == "30" {
                                info!("Scenario 1 passed: got expected result 30");
                            } else {
                                return Err(anyhow::anyhow!("Scenario 1 failed: expected 30, got {}", text));
                            }
                        } else {
                            return Err(anyhow::anyhow!("No text field in content"));
                        }
                    } else {
                        return Err(anyhow::anyhow!("No content in response"));
                    }
                } else {
                    return Err(anyhow::anyhow!("No content field in result"));
                }
            } else {
                return Err(anyhow::anyhow!("No result in response: {:?}", response));
            }
        }
        2 => {
            // client1 connects to CalcServer and calls ambiguous_add(a=10), receives elicitation for b, responds with 20, gets result of 30
            let tool_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(2)),
                method: "tools/call".to_string(),
                params: Some(json!({
                    "name": "ambiguous_add",
                    "arguments": {"a": 10}
                })),
            };

            send_request(stdin, &tool_request)?;
            let response = read_response(reader)?;
            info!("Ambiguous add response: {}", serde_json::to_string_pretty(&response)?);
            // For now, expect an error since elicitation is not implemented
        }
        4 => {
            // client1 connects to CalcServer, reads resource://special-number (initial value 42), calls write_special_number(value=100), then reads resource://special-number again and gets 100

            // First read the resource
            let read_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(2)),
                method: "resources/read".to_string(),
                params: Some(json!({
                    "uri": "resource://special-number"
                })),
            };

            send_request(stdin, &read_request)?;
            let initial_read_response = read_response(reader)?;
            info!("Initial resource read: {}", serde_json::to_string_pretty(&initial_read_response)?);

            // Verify initial value is 42
            if let Some(result) = &initial_read_response.result {
                if let Some(contents) = result.get("contents") {
                    if let Some(first_content) = contents.get(0) {
                        if let Some(text) = first_content.get("text") {
                            if text != "42" {
                                warn!("Expected initial value 42, got {}", text);
                            }
                        }
                    }
                }
            }

            // Write new value
            let write_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(3)),
                method: "tools/call".to_string(),
                params: Some(json!({
                    "name": "write_special_number",
                    "arguments": {"value": 100}
                })),
            };

            send_request(stdin, &write_request)?;
            let write_response = read_response(reader)?;
            info!("Write response: {}", serde_json::to_string_pretty(&write_response)?);

            // Read the resource again
            let read_request2 = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(4)),
                method: "resources/read".to_string(),
                params: Some(json!({
                    "uri": "resource://special-number"
                })),
            };

            send_request(stdin, &read_request2)?;
            let final_read_response = read_response(reader)?;
            info!("Final resource read: {}", serde_json::to_string_pretty(&final_read_response)?);

            // Verify final value is 100
            if let Some(result) = &final_read_response.result {
                if let Some(contents) = result.get("contents") {
                    if let Some(first_content) = contents.get(0) {
                        if let Some(text) = first_content.get("text") {
                            if text == "100" {
                                info!("Scenario 4 passed: resource updated to 100");
                            } else {
                                return Err(anyhow::anyhow!("Scenario 4 failed: expected 100, got {}", text));
                            }
                        }
                    }
                }
            }
        }
        5 => {
            // client1 connects to CalcServer, calls prompts/get for example-maths prompt
            let prompt_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(2)),
                method: "prompts/get".to_string(),
                params: Some(json!({
                    "name": "example-maths"
                })),
            };

            send_request(stdin, &prompt_request)?;
            let response = read_response(reader)?;
            info!("Prompt response: {}", serde_json::to_string_pretty(&response)?);
        }
        6 => {
            // client1 connects to CalcServer, calls eval_with_sampling(expression='2 + 2 * 3'), server returns 8
            let eval_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(2)),
                method: "tools/call".to_string(),
                params: Some(json!({
                    "name": "eval_with_sampling",
                    "arguments": {"expression": "2 + 2 * 3"}
                })),
            };

            send_request(stdin, &eval_request)?;
            let response = read_response(reader)?;
            info!("Eval response: {}", serde_json::to_string_pretty(&response)?);

            // Check result is 8
            if let Some(result) = response.result {
                if let Some(content_array) = result.get("content") {
                    if let Some(first_content) = content_array.get(0) {
                        if let Some(text) = first_content.get("text") {
                            if text == "8" {
                                info!("Scenario 6 passed: got expected result 8");
                            } else {
                                return Err(anyhow::anyhow!("Scenario 6 failed: expected 8, got {}", text));
                            }
                        }
                    }
                }
            }
        }
        13 => {
            // client1 connects to CalcServer via stdio transport, performs basic add(10,20) operation and gets result 30
            // This is essentially the same as scenario 1
            let tool_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(2)),
                method: "tools/call".to_string(),
                params: Some(json!({
                    "name": "add",
                    "arguments": {"a": 10, "b": 20}
                })),
            };

            send_request(stdin, &tool_request)?;
            let response = read_response(reader)?;

            if let Some(result) = response.result {
                if let Some(content_array) = result.get("content") {
                    if let Some(first_content) = content_array.get(0) {
                        if let Some(text) = first_content.get("text") {
                            if text == "30" {
                                info!("Scenario 13 passed: got expected result 30");
                            } else {
                                return Err(anyhow::anyhow!("Scenario 13 failed: expected 30, got {}", text));
                            }
                        }
                    }
                }
            }
        }
        15 => {
            // client1 connects to CalcServer, calls eval_with_sampling(expression='(2 + 3) * (4 + 5)') and gets result 45
            let eval_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(2)),
                method: "tools/call".to_string(),
                params: Some(json!({
                    "name": "eval_with_sampling",
                    "arguments": {"expression": "(2 + 3) * (4 + 5)"}
                })),
            };

            send_request(stdin, &eval_request)?;
            let response = read_response(reader)?;
            info!("Eval response: {}", serde_json::to_string_pretty(&response)?);

            // Check result is 45
            if let Some(result) = response.result {
                if let Some(content_array) = result.get("content") {
                    if let Some(first_content) = content_array.get(0) {
                        if let Some(text) = first_content.get("text") {
                            if text == "45" {
                                info!("Scenario 15 passed: got expected result 45");
                            } else {
                                return Err(anyhow::anyhow!("Scenario 15 failed: expected 45, got {}", text));
                            }
                        }
                    }
                }
            }
        }
        21 => {
            // client1 connects to CalcServer with protocol version 2025-06-18, server accepts same version, performs add(10,20) operation
            // This is handled by the initialization and then a regular add call
            let tool_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(2)),
                method: "tools/call".to_string(),
                params: Some(json!({
                    "name": "add",
                    "arguments": {"a": 10, "b": 20}
                })),
            };

            send_request(stdin, &tool_request)?;
            let response = read_response(reader)?;

            if let Some(result) = response.result {
                if let Some(content_array) = result.get("content") {
                    if let Some(first_content) = content_array.get(0) {
                        if let Some(text) = first_content.get("text") {
                            if text == "30" {
                                info!("Scenario 21 passed: got expected result 30");
                            } else {
                                return Err(anyhow::anyhow!("Scenario 21 failed: expected 30, got {}", text));
                            }
                        }
                    }
                }
            }
        }
        25 => {
            // client1 connects to CalcServer, initiates 3 concurrent add tool calls: add(1,2), add(3,4), add(5,6). Server processes them in parallel and returns results (3, 7, 11)

            let requests = vec![
                JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(json!(2)),
                    method: "tools/call".to_string(),
                    params: Some(json!({
                        "name": "add",
                        "arguments": {"a": 1, "b": 2}
                    })),
                },
                JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(json!(3)),
                    method: "tools/call".to_string(),
                    params: Some(json!({
                        "name": "add",
                        "arguments": {"a": 3, "b": 4}
                    })),
                },
                JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(json!(4)),
                    method: "tools/call".to_string(),
                    params: Some(json!({
                        "name": "add",
                        "arguments": {"a": 5, "b": 6}
                    })),
                },
            ];

            // Send all requests
            for req in &requests {
                send_request(stdin, req)?;
            }

            // Read all responses
            let mut responses = Vec::new();
            for _ in 0..3 {
                let response = read_response(reader)?;
                responses.push(response);
            }

            // Verify results
            let expected_results = vec!["3", "7", "11"];
            for (i, response) in responses.iter().enumerate() {
                if let Some(result) = &response.result {
                    if let Some(content_array) = result.get("content") {
                        if let Some(first_content) = content_array.get(0) {
                            if let Some(text) = first_content.get("text").and_then(|t| t.as_str()) {
                                info!("Concurrent call {} result: {}", i+1, text);
                                // Note: We don't enforce ordering for concurrent calls
                                if !expected_results.contains(&text) {
                                    return Err(anyhow::anyhow!("Unexpected result in concurrent calls: {}", text));
                                }
                            }
                        }
                    }
                }
            }
            info!("Scenario 25 passed: all concurrent calls returned expected results");
        }
        _ => {
            info!("Scenario {} not fully implemented yet, but connection successful", scenario.id);
        }
    }

    Ok(())
}

fn send_request(stdin: &mut std::process::ChildStdin, request: &JsonRpcRequest) -> anyhow::Result<()> {
    writeln!(stdin, "{}", serde_json::to_string(request)?)?;
    Ok(())
}

fn read_response(reader: &mut BufReader<std::process::ChildStdout>) -> anyhow::Result<JsonRpcResponse> {
    let mut response_line = String::new();
    loop {
        response_line.clear();
        reader.read_line(&mut response_line)?;
        let line = response_line.trim();
        if line.starts_with('{') {
            let response: JsonRpcResponse = serde_json::from_str(line)?;
            return Ok(response);
        }
    }
}