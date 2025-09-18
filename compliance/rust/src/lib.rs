pub mod jsonrpc;
pub mod servers;
pub mod client;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Scenarios {
    pub servers: HashMap<String, ServerDefinition>,
    pub scenarios: Vec<ScenarioDefinition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerDefinition {
    pub description: String,
    pub tools: HashMap<String, ToolDefinition>,
    pub resources: HashMap<String, ResourceDefinition>,
    #[serde(rename = "resourceTemplates")]
    pub resource_templates: HashMap<String, ResourceTemplateDefinition>,
    pub prompts: HashMap<String, PromptDefinition>,
    #[serde(rename = "promptTemplates")]
    pub prompt_templates: HashMap<String, PromptTemplateDefinition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolDefinition {
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceDefinition {
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceTemplateDefinition {
    pub description: String,
    pub params: HashMap<String, ParamDefinition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PromptDefinition {
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PromptTemplateDefinition {
    pub description: String,
    pub params: HashMap<String, ParamDefinition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParamDefinition {
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScenarioDefinition {
    pub id: u32,
    pub description: String,
    pub client_ids: Vec<String>,
    pub server_name: String,
    #[serde(default)]
    pub http_only: bool,
}

pub fn load_scenarios() -> anyhow::Result<Scenarios> {
    let scenarios_path = std::env::current_dir()?.join("../scenarios/data.json");
    let content = std::fs::read_to_string(&scenarios_path)
        .map_err(|e| anyhow::anyhow!("Failed to read scenarios file at {:?}: {}", scenarios_path, e))?;
    let scenarios: Scenarios = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse scenarios JSON: {}", e))?;
    Ok(scenarios)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    #[test]
    fn test_load_scenarios() {
        let result = load_scenarios();
        assert!(result.is_ok(), "Should be able to load scenarios");

        let scenarios = result.unwrap();
        assert!(scenarios.servers.contains_key("CalcServer"));
        assert!(scenarios.servers.contains_key("FileServer"));
        assert!(scenarios.servers.contains_key("ErrorServer"));
        assert!(!scenarios.scenarios.is_empty());
    }

    #[test]
    fn test_scenario_structure() {
        let scenarios = load_scenarios().unwrap();

        for scenario in &scenarios.scenarios {
            assert!(scenario.id > 0);
            assert!(!scenario.description.is_empty());
            assert!(!scenario.client_ids.is_empty());
            assert!(scenarios.servers.contains_key(&scenario.server_name));
        }
    }

    #[test]
    fn test_server_definitions() {
        let scenarios = load_scenarios().unwrap();

        // Test CalcServer
        let calc_server = scenarios.servers.get("CalcServer").unwrap();
        assert!(calc_server.tools.contains_key("add"));
        assert!(calc_server.tools.contains_key("ambiguous_add"));
        assert!(calc_server.tools.contains_key("eval_with_sampling"));
        assert!(calc_server.resources.contains_key("resource://special-number"));
        assert!(calc_server.prompts.contains_key("example-maths"));

        // Test FileServer
        let file_server = scenarios.servers.get("FileServer").unwrap();
        assert!(file_server.tools.contains_key("write_file"));
        assert!(file_server.tools.contains_key("delete_file"));

        // Test ErrorServer
        let error_server = scenarios.servers.get("ErrorServer").unwrap();
        assert!(error_server.tools.contains_key("always_error"));
        assert!(error_server.tools.contains_key("timeout"));
    }

    #[test]
    fn test_binaries_can_be_built() {
        // Test that the binaries can be built
        let output = Command::new("cargo")
            .args(&["build", "--bins"])
            .output()
            .expect("Failed to run cargo build");

        if !output.status.success() {
            panic!(
                "Failed to build binaries:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[tokio::test]
    async fn test_server_binary_help() {
        // Test that server binary shows help
        let output = Command::new("cargo")
            .args(&["run", "--bin", "test-server", "--", "--help"])
            .output()
            .expect("Failed to run test-server help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("MCP compliance test server"));
        assert!(stdout.contains("--server-name"));
        assert!(stdout.contains("--transport"));
    }

    #[tokio::test]
    async fn test_client_binary_help() {
        // Test that client binary shows help
        let output = Command::new("cargo")
            .args(&["run", "--bin", "test-client", "--", "--help"])
            .output()
            .expect("Failed to run test-client help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("MCP compliance test client"));
        assert!(stdout.contains("--scenario-id"));
        assert!(stdout.contains("--id"));
        assert!(stdout.contains("stdio"));
    }

    #[tokio::test]
    async fn test_basic_server_startup() {
        // Test that the server can start up without crashing
        let mut child = Command::new("cargo")
            .args(&["run", "--bin", "test-server", "--", "--server-name", "CalcServer", "--transport", "stdio"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to start test-server");

        // Give it a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Kill the process
        child.kill().expect("Failed to kill test-server");
        let output = child.wait_with_output().expect("Failed to wait for test-server");

        // Check stderr for any panic messages
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("panic"), "Server should not panic on startup: {}", stderr);
    }
}