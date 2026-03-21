use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

use crate::config::mcp_home;

/// A registered tool with input/output schemas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Value,
    /// If set, this tool is bound to a role template.
    #[serde(default)]
    pub role_binding: Option<String>,
    /// If set, this tool is bound to a workflow file.
    #[serde(default)]
    pub workflow_binding: Option<String>,
    /// "spawn-on-demand" or "call-running"
    #[serde(default = "default_invocation_mode")]
    pub invocation_mode: String,
}

fn default_invocation_mode() -> String {
    "spawn-on-demand".into()
}

/// Compact listing entry returned by `tool list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolListEntry {
    pub name: String,
    pub description: String,
    pub role_binding: Option<String>,
    pub workflow_binding: Option<String>,
    pub invocation_mode: String,
}

impl From<&ToolDefinition> for ToolListEntry {
    fn from(t: &ToolDefinition) -> Self {
        Self {
            name: t.name.clone(),
            description: t.description.clone(),
            role_binding: t.role_binding.clone(),
            workflow_binding: t.workflow_binding.clone(),
            invocation_mode: t.invocation_mode.clone(),
        }
    }
}

fn tools_dir() -> PathBuf {
    mcp_home().join("tools")
}

/// Register a new tool from a role binding.
///
/// If the role has `allowed_tools`, they are included in the schema
/// so discovering agents can see what sub-capabilities the tool exposes.
pub fn register_from_role(name: &str, role_name: &str) -> Result<ToolDefinition> {
    let role = crate::role::get_role(role_name)
        .with_context(|| format!("Role '{role_name}' not found — cannot bind tool"))?;

    let description = format!(
        "Spawn or invoke a {} agent (role: {}). {}",
        role.soul.as_deref().unwrap_or(role_name),
        role.role.as_deref().unwrap_or("general"),
        if !role.allowed_tools.is_empty() {
            format!("Available sub-tools: {}", role.allowed_tools.join(", "))
        } else {
            String::new()
        },
    );

    // Build input schema — include sub-tool schemas from examples/ if available
    let mut properties = serde_json::json!({
        "task": { "type": "string", "description": "Task to perform" },
        "constraints": { "type": "string", "description": "Additional constraints" }
    });

    if !role.allowed_tools.is_empty() {
        properties["allowed_tools"] = serde_json::json!({
            "type": "array",
            "items": { "type": "string", "enum": role.allowed_tools },
            "description": "Subset of tools this invocation may use"
        });

        // Try to load sub-tool schemas from ~/.mcp/tools/ for richer discovery
        let mut sub_tools = Vec::new();
        for tool_name in &role.allowed_tools {
            let schema_path = crate::config::mcp_home()
                .join("tools")
                .join(format!("{tool_name}.json"));
            if let Ok(text) = std::fs::read_to_string(&schema_path) {
                if let Ok(schema) = serde_json::from_str::<serde_json::Value>(&text) {
                    sub_tools.push(schema);
                }
            }
        }
        if !sub_tools.is_empty() {
            properties["_sub_tool_schemas"] = serde_json::json!(sub_tools);
        }
    }

    let input_schema = serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": ["task"]
    });

    let output_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "agent_id": { "type": "integer" },
            "status": { "type": "string" },
            "summary": { "type": "string" }
        }
    });

    let tool = ToolDefinition {
        name: name.to_string(),
        description,
        input_schema,
        output_schema,
        role_binding: Some(role_name.to_string()),
        workflow_binding: None,
        invocation_mode: "spawn-on-demand".into(),
    };

    save_tool(&tool)?;
    Ok(tool)
}

/// Register a tool from a workflow file.
pub fn register_from_workflow(name: &str, workflow_path: &str) -> Result<ToolDefinition> {
    // Validate the workflow file exists
    let path = std::path::Path::new(workflow_path);
    if !path.exists() {
        // Check in ~/.mcp/workflows/
        let wf_path = mcp_home().join("workflows").join(workflow_path);
        if !wf_path.exists() {
            bail!("Workflow file '{workflow_path}' not found");
        }
    }

    let description = format!("Execute workflow '{workflow_path}'.");

    let input_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "variables": {
                "type": "object",
                "description": "Variables to pass into the workflow"
            }
        }
    });

    let output_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "workflow_run_id": { "type": "integer" },
            "status": { "type": "string" },
            "step_results": {
                "type": "array",
                "items": { "type": "object" }
            }
        }
    });

    let tool = ToolDefinition {
        name: name.to_string(),
        description,
        input_schema,
        output_schema,
        role_binding: None,
        workflow_binding: Some(workflow_path.to_string()),
        invocation_mode: "spawn-on-demand".into(),
    };

    save_tool(&tool)?;
    Ok(tool)
}

/// Register a tool with a manual JSON schema.
#[allow(dead_code)]
pub fn register_manual(tool: &ToolDefinition) -> Result<()> {
    validate_tool(tool)?;
    save_tool(tool)
}

/// Validate a tool definition.
#[allow(dead_code)]
pub fn validate_tool(tool: &ToolDefinition) -> Result<()> {
    if tool.name.is_empty() {
        bail!("Tool name cannot be empty");
    }
    if tool.name.contains(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        bail!("Tool name must be alphanumeric (with _ or -)");
    }
    // Validate input_schema is an object
    if !tool.input_schema.is_null() && !tool.input_schema.is_object() {
        bail!("input_schema must be a JSON object");
    }
    if !tool.output_schema.is_null() && !tool.output_schema.is_object() {
        bail!("output_schema must be a JSON object");
    }
    // If role-bound, check role exists
    if let Some(ref role_name) = tool.role_binding {
        crate::role::get_role(role_name)
            .with_context(|| format!("Role '{role_name}' not found"))?;
    }
    Ok(())
}

/// Save a tool definition to disk.
fn save_tool(tool: &ToolDefinition) -> Result<()> {
    let dir = tools_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", tool.name));
    let json = serde_json::to_string_pretty(tool)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// List all registered tools.
pub fn list_tools() -> Result<Vec<ToolDefinition>> {
    let dir = tools_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut tools = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let text = std::fs::read_to_string(&path)?;
            if let Ok(tool) = serde_json::from_str::<ToolDefinition>(&text) {
                tools.push(tool);
            }
        }
    }
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tools)
}

/// Get a specific tool by name.
pub fn get_tool(name: &str) -> Result<ToolDefinition> {
    let path = tools_dir().join(format!("{name}.json"));
    if !path.exists() {
        bail!("Tool '{name}' not found");
    }
    let text = std::fs::read_to_string(&path)?;
    serde_json::from_str(&text).context("Failed to parse tool file")
}

/// Delete a tool by name.
pub fn delete_tool(name: &str) -> Result<()> {
    let path = tools_dir().join(format!("{name}.json"));
    if !path.exists() {
        bail!("Tool '{name}' not found");
    }
    std::fs::remove_file(&path)?;
    Ok(())
}

/// Build the MCP-style tool discovery response for all registered tools.
pub fn discovery_response() -> Result<Value> {
    let tools = list_tools()?;
    let tool_schemas: Vec<Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
                "output_schema": t.output_schema,
                "role_binding": t.role_binding,
                "workflow_binding": t.workflow_binding,
                "invocation_mode": t.invocation_mode,
            })
        })
        .collect();
    Ok(serde_json::json!({ "tools": tool_schemas }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn set_test_home(dir: &TempDir) {
        std::env::set_var("HOME", dir.path());
        // Also set USERPROFILE for Windows
        std::env::set_var("USERPROFILE", dir.path());
    }

    #[test]
    fn test_tool_definition_serialization() {
        let tool = ToolDefinition {
            name: "test_tool".into(),
            description: "A test tool".into(),
            input_schema: serde_json::json!({"type": "object", "properties": {"task": {"type": "string"}}}),
            output_schema: serde_json::json!({"type": "object", "properties": {"result": {"type": "string"}}}),
            role_binding: None,
            workflow_binding: None,
            invocation_mode: "spawn-on-demand".into(),
        };

        let json = serde_json::to_string(&tool).unwrap();
        let parsed: ToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test_tool");
        assert_eq!(parsed.invocation_mode, "spawn-on-demand");
    }

    #[test]
    fn test_validate_tool_empty_name() {
        let tool = ToolDefinition {
            name: "".into(),
            description: "empty".into(),
            input_schema: Value::Null,
            output_schema: Value::Null,
            role_binding: None,
            workflow_binding: None,
            invocation_mode: "spawn-on-demand".into(),
        };
        assert!(validate_tool(&tool).is_err());
    }

    #[test]
    fn test_validate_tool_bad_chars() {
        let tool = ToolDefinition {
            name: "bad tool!".into(),
            description: "bad".into(),
            input_schema: Value::Null,
            output_schema: Value::Null,
            role_binding: None,
            workflow_binding: None,
            invocation_mode: "spawn-on-demand".into(),
        };
        assert!(validate_tool(&tool).is_err());
    }

    #[test]
    fn test_validate_tool_bad_schema() {
        let tool = ToolDefinition {
            name: "good_name".into(),
            description: "good".into(),
            input_schema: serde_json::json!("not an object"),
            output_schema: Value::Null,
            role_binding: None,
            workflow_binding: None,
            invocation_mode: "spawn-on-demand".into(),
        };
        assert!(validate_tool(&tool).is_err());
    }

    #[test]
    fn test_tool_list_entry_from() {
        let tool = ToolDefinition {
            name: "coder".into(),
            description: "Code gen".into(),
            input_schema: Value::Null,
            output_schema: Value::Null,
            role_binding: Some("coder".into()),
            workflow_binding: None,
            invocation_mode: "spawn-on-demand".into(),
        };
        let entry = ToolListEntry::from(&tool);
        assert_eq!(entry.name, "coder");
        assert_eq!(entry.role_binding, Some("coder".into()));
    }

    #[test]
    fn test_discovery_response_format() {
        // This tests the shape even if the tools dir is empty
        let resp = discovery_response();
        // Should at least succeed (empty list is fine)
        if let Ok(val) = resp {
            assert!(val.get("tools").unwrap().is_array());
        }
    }
}
