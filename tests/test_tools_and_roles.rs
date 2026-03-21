/// Tests for tool registry, role-bound tools, config defaults,
/// and agent spawning with roles and tool schemas.
/// These run without network access (no API keys needed).

fn test_db() -> std::sync::Arc<mcp::persistence::Database> {
    std::sync::Arc::new(mcp::persistence::Database::open_memory().unwrap())
}

// ── Config with role/tool defaults ──────────────────────────────────

#[test]
fn test_config_default_role_and_tool_fields() {
    let cfg = mcp::config::McpConfig::default();
    assert!(cfg.default.role.is_none());
    assert!(cfg.default.tool.is_none());
}

#[test]
fn test_config_parse_with_role_and_tool() {
    let toml_str = r#"
[default]
provider = "nvidia-nim"
model = "nvidia/llama-3.1-70b-instruct"
role = "local_coder"
tool = "coder_agent"

[provider.nvidia-nim]
type = "nvidia-nim"
url = "https://integrate.api.nvidia.com/v1"
model = "nvidia/llama-3.1-70b-instruct"
api_key = "test"
"#;
    let cfg: mcp::config::McpConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.default.role.as_deref(), Some("local_coder"));
    assert_eq!(cfg.default.tool.as_deref(), Some("coder_agent"));
    assert_eq!(cfg.default.provider, "nvidia-nim");
}

#[test]
fn test_config_save_and_reload() {
    // Create a temp dir to act as home
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");

    let cfg = mcp::config::McpConfig {
        default: mcp::config::DefaultConfig {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            role: Some("local_coder".into()),
            tool: Some("coder_agent".into()),
        },
        ..Default::default()
    };

    // Serialize and write
    let text = toml::to_string_pretty(&cfg).unwrap();
    std::fs::write(&config_path, &text).unwrap();

    // Read back and parse
    let loaded_text = std::fs::read_to_string(&config_path).unwrap();
    let loaded: mcp::config::McpConfig = toml::from_str(&loaded_text).unwrap();
    assert_eq!(loaded.default.role.as_deref(), Some("local_coder"));
    assert_eq!(loaded.default.tool.as_deref(), Some("coder_agent"));
    assert_eq!(loaded.default.provider, "openai");
    assert_eq!(loaded.default.model, "gpt-4o");
}

// ── Example role file parsing ───────────────────────────────────────

#[test]
fn test_local_coder_role_file_parses() {
    let toml_text = std::fs::read_to_string("examples/roles/local_coder.toml")
        .expect("examples/roles/local_coder.toml should exist");
    let role: mcp::role::RoleDefinition =
        toml::from_str(&toml_text).expect("local_coder.toml should parse as valid RoleDefinition");

    assert_eq!(role.name, "local_coder");
    assert_eq!(role.soul.as_deref(), Some("local-code-organizer"));
    assert_eq!(role.role.as_deref(), Some("code-gen"));
    assert!(role.system_prompt.is_some());
    assert!(role
        .system_prompt
        .as_ref()
        .unwrap()
        .contains("NEVER rename or delete"));
    assert_eq!(role.default_provider.as_deref(), Some("nvidia-nim"));

    // Check all 5 allowed tools
    assert_eq!(role.allowed_tools.len(), 5);
    assert!(role.allowed_tools.contains(&"read-file".to_string()));
    assert!(role.allowed_tools.contains(&"write-file".to_string()));
    assert!(role.allowed_tools.contains(&"edit-file".to_string()));
    assert!(role.allowed_tools.contains(&"run-command".to_string()));
    assert!(role.allowed_tools.contains(&"list-files".to_string()));
}

#[test]
fn test_local_coder_system_prompt_safety() {
    let toml_text = std::fs::read_to_string("examples/roles/local_coder.toml").unwrap();
    let role: mcp::role::RoleDefinition = toml::from_str(&toml_text).unwrap();
    let prompt = role.system_prompt.unwrap();

    // Verify the safety guardrails are present
    assert!(
        prompt.contains("NEVER rename or delete"),
        "Missing safety rule: never rename/delete"
    );
    assert!(
        prompt.contains("READ-ONLY"),
        "Missing safety rule: prefer read-only"
    );
    assert!(
        prompt.contains("destructive"),
        "Missing safety rule: destructive commands warning"
    );
}

// ── Example tool JSON schemas ───────────────────────────────────────

#[test]
fn test_tool_schema_read_file() {
    let text = std::fs::read_to_string("examples/tools/read-file.json").unwrap();
    let schema: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(schema["name"], "read-file");
    assert_eq!(
        schema["input_schema"]["required"][0], "path",
        "read-file must require 'path'"
    );
    assert!(schema["input_schema"]["properties"]["path"].is_object());
}

#[test]
fn test_tool_schema_write_file() {
    let text = std::fs::read_to_string("examples/tools/write-file.json").unwrap();
    let schema: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(schema["name"], "write-file");
    let required = schema["input_schema"]["required"]
        .as_array()
        .unwrap();
    assert!(required.contains(&serde_json::json!("path")));
    assert!(required.contains(&serde_json::json!("contents")));
}

#[test]
fn test_tool_schema_edit_file() {
    let text = std::fs::read_to_string("examples/tools/edit-file.json").unwrap();
    let schema: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(schema["name"], "edit-file");
    assert!(schema["input_schema"]["properties"]["path"].is_object());
    assert!(schema["input_schema"]["properties"]["search"].is_object());
    assert!(schema["input_schema"]["properties"]["replace"].is_object());
}

#[test]
fn test_tool_schema_run_command() {
    let text = std::fs::read_to_string("examples/tools/run-command.json").unwrap();
    let schema: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(schema["name"], "run-command");
    assert_eq!(schema["input_schema"]["required"][0], "command");
    assert!(schema["output_schema"]["properties"]["exit_code"].is_object());
    assert!(schema["output_schema"]["properties"]["stdout"].is_object());
}

#[test]
fn test_tool_schema_list_files() {
    let text = std::fs::read_to_string("examples/tools/list-files.json").unwrap();
    let schema: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(schema["name"], "list-files");
    assert_eq!(schema["input_schema"]["required"][0], "path");
    assert!(schema["output_schema"]["properties"]["entries"].is_object());
}

#[test]
fn test_all_tool_schemas_are_valid_tool_definitions() {
    // Each example tool JSON should deserialize as a partial ToolDefinition
    for file in &[
        "read-file.json",
        "write-file.json",
        "edit-file.json",
        "run-command.json",
        "list-files.json",
    ] {
        let path = format!("examples/tools/{file}");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("Missing {path}"));
        let val: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| panic!("{path} is not valid JSON"));
        assert!(val["name"].is_string(), "{path} missing name");
        assert!(val["description"].is_string(), "{path} missing description");
        assert!(val["input_schema"].is_object(), "{path} missing input_schema");
        assert!(val["output_schema"].is_object(), "{path} missing output_schema");
    }
}

// ── Tool CRUD lifecycle ─────────────────────────────────────────────

#[test]
fn test_tool_crud_lifecycle() {
    mcp::config::ensure_dirs().unwrap();

    // Create a role first so we can bind a tool to it
    let role = mcp::role::RoleDefinition {
        name: "_mcp_test_tool_role".into(),
        soul: Some("test-soul".into()),
        role: Some("code-gen".into()),
        prompt_file: None,
        system_prompt: Some("Test prompt.".into()),
        default_model: None,
        default_provider: None,
        max_depth: 2,
        max_children: 5,
        allowed_tools: vec!["read-file".into(), "write-file".into()],
    };
    mcp::role::create_role(&role).unwrap();

    // Register a tool from the role
    let tool =
        mcp::tool::register_from_role("_mcp_test_tool", "_mcp_test_tool_role").unwrap();
    assert_eq!(tool.name, "_mcp_test_tool");
    assert_eq!(tool.role_binding, Some("_mcp_test_tool_role".into()));
    assert!(tool.description.contains("test-soul"));
    assert!(tool.description.contains("read-file"));
    assert!(tool.description.contains("write-file"));

    // Input schema should include allowed_tools
    let props = &tool.input_schema["properties"];
    assert!(props["task"].is_object(), "Missing task in input_schema");
    assert!(
        props["allowed_tools"].is_object(),
        "Missing allowed_tools in input_schema"
    );
    let allowed_enum = &props["allowed_tools"]["items"]["enum"];
    assert!(allowed_enum.as_array().unwrap().contains(&serde_json::json!("read-file")));
    assert!(allowed_enum.as_array().unwrap().contains(&serde_json::json!("write-file")));

    // List should include it
    let tools = mcp::tool::list_tools().unwrap();
    assert!(tools.iter().any(|t| t.name == "_mcp_test_tool"));

    // Get by name
    let fetched = mcp::tool::get_tool("_mcp_test_tool").unwrap();
    assert_eq!(fetched.name, "_mcp_test_tool");
    assert_eq!(fetched.invocation_mode, "spawn-on-demand");

    // Delete
    mcp::tool::delete_tool("_mcp_test_tool").unwrap();
    assert!(mcp::tool::get_tool("_mcp_test_tool").is_err());

    // Clean up role
    mcp::role::delete_role("_mcp_test_tool_role").unwrap();
}

#[test]
fn test_tool_register_nonexistent_role_fails() {
    let result = mcp::tool::register_from_role("bad_tool", "_no_such_role_ever_xyz");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_tool_get_nonexistent_fails() {
    assert!(mcp::tool::get_tool("_no_such_tool_xyz").is_err());
}

#[test]
fn test_tool_delete_nonexistent_fails() {
    assert!(mcp::tool::delete_tool("_no_such_tool_xyz").is_err());
}

#[test]
fn test_tool_register_from_workflow_nonexistent_fails() {
    let result = mcp::tool::register_from_workflow("wf_tool", "nonexistent_workflow.yaml");
    assert!(result.is_err());
}

// ── Agent spawning with roles ───────────────────────────────────────

#[tokio::test]
async fn test_agent_spawn_with_role_system_prompt() {
    mcp::config::ensure_dirs().unwrap();

    // Create a role with a known system prompt
    let role = mcp::role::RoleDefinition {
        name: "_mcp_test_spawn_role".into(),
        soul: Some("spawn-test-soul".into()),
        role: Some("code-gen".into()),
        prompt_file: None,
        system_prompt: Some("You are a specialized spawn test agent.".into()),
        default_model: None,
        default_provider: None,
        max_depth: 2,
        max_children: 3,
        allowed_tools: vec!["read-file".into()],
    };
    mcp::role::create_role(&role).unwrap();

    // Verify we can resolve the prompt
    let prompt = mcp::role::resolve_system_prompt(&role).unwrap();
    assert_eq!(prompt, "You are a specialized spawn test agent.");

    // Verify the role can be loaded
    let loaded = mcp::role::get_role("_mcp_test_spawn_role").unwrap();
    assert_eq!(loaded.soul.as_deref(), Some("spawn-test-soul"));
    assert_eq!(loaded.max_children, 3);
    assert_eq!(loaded.allowed_tools, vec!["read-file"]);

    // Clean up
    mcp::role::delete_role("_mcp_test_spawn_role").unwrap();
}

// ── Provider list_models (curated lists, no network) ────────────────

#[tokio::test]
async fn test_anthropic_list_models() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "anthropic".into(),
        url: Some("https://api.anthropic.com/v1".into()),
        model: Some("claude-sonnet-4-20250514".into()),
        api_key: Some("test-key".into()),
        timeout: 10,
        max_retries: 0,
        region: None,
    };
    let provider =
        mcp::provider::build_provider("anthropic", &entry, "claude-sonnet-4-20250514").unwrap();
    let models = provider.list_models().await.unwrap();
    assert!(!models.is_empty(), "Anthropic should return curated models");
    assert!(
        models.iter().any(|m| m.contains("claude")),
        "Should contain claude models"
    );
}

#[tokio::test]
async fn test_bedrock_list_models() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "amazon-bedrock".into(),
        url: None,
        model: Some("anthropic.claude-3-sonnet-20240229-v1:0".into()),
        api_key: None,
        timeout: 10,
        max_retries: 0,
        region: Some("us-east-1".into()),
    };
    let provider = mcp::provider::build_provider(
        "bedrock",
        &entry,
        "anthropic.claude-3-sonnet-20240229-v1:0",
    )
    .unwrap();
    let models = provider.list_models().await.unwrap();
    assert!(!models.is_empty(), "Bedrock should return curated models");
    assert!(
        models.iter().any(|m| m.contains("anthropic.")),
        "Should contain anthropic model IDs"
    );
    // Should include the configured model
    assert!(models
        .iter()
        .any(|m| m == "anthropic.claude-3-sonnet-20240229-v1:0"));
}

#[tokio::test]
async fn test_huggingface_list_models() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "huggingface".into(),
        url: Some("https://api-inference.huggingface.co/models".into()),
        model: Some("meta-llama/Meta-Llama-3-70B-Instruct".into()),
        api_key: Some("test".into()),
        timeout: 10,
        max_retries: 0,
        region: None,
    };
    let provider = mcp::provider::build_provider(
        "huggingface",
        &entry,
        "meta-llama/Meta-Llama-3-70B-Instruct",
    )
    .unwrap();
    let models = provider.list_models().await.unwrap();
    assert!(!models.is_empty());
    // Should include the configured model
    assert!(models
        .iter()
        .any(|m| m == "meta-llama/Meta-Llama-3-70B-Instruct"));
}

// ── Agent manager list_models (not found) ───────────────────────────

#[tokio::test]
async fn test_agent_manager_list_models_not_found() {
    let cfg = mcp::config::McpConfig::default();
    let mgr = mcp::agent::AgentManager::new(&cfg, test_db()).unwrap();
    let result = mgr.list_models("nonexistent").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not configured"));
}

// ── Tool discovery response ─────────────────────────────────────────

#[test]
fn test_discovery_includes_registered_tools() {
    mcp::config::ensure_dirs().unwrap();

    // Ensure dirs exist
    mcp::config::ensure_dirs().unwrap();

    // Create a role and register a tool
    let role = mcp::role::RoleDefinition {
        name: "_mcp_test_disc_role".into(),
        soul: Some("disc-soul".into()),
        role: Some("test".into()),
        prompt_file: None,
        system_prompt: Some("Test".into()),
        default_model: None,
        default_provider: None,
        max_depth: 1,
        max_children: 1,
        allowed_tools: vec![],
    };
    mcp::role::create_role(&role).unwrap();
    mcp::tool::register_from_role("_mcp_disc_tool", "_mcp_test_disc_role").unwrap();

    // Discovery should include it
    let resp = mcp::tool::discovery_response().unwrap();
    let tools = resp["tools"].as_array().unwrap();
    assert!(
        tools.iter().any(|t| t["name"] == "_mcp_disc_tool"),
        "Discovery response should include the registered tool"
    );

    // Clean up
    mcp::tool::delete_tool("_mcp_disc_tool").unwrap();
    mcp::role::delete_role("_mcp_test_disc_role").unwrap();
}

// ── Workflow runner with mock agents ────────────────────────────────

#[tokio::test]
async fn test_workflow_runner_lifecycle() {
    // Create a config with a real-ish provider (will fail to chat, but that's ok)
    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "openai".to_string(),
        mcp::config::ProviderEntry {
            provider_type: "openai".into(),
            url: Some("https://api.openai.com/v1".into()),
            model: Some("gpt-4o".into()),
            api_key: Some("sk-fake".into()),
            timeout: 5,
            max_retries: 0,
            region: None,
        },
    );

    let config = mcp::config::McpConfig {
        default: mcp::config::DefaultConfig {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            role: None,
            tool: None,
        },
        provider: providers,
        ..Default::default()
    };

    let manager = std::sync::Arc::new(mcp::agent::AgentManager::new(&config, test_db()).unwrap());
    let runner = mcp::workflow::WorkflowRunner::new();

    // A simple workflow that spawns an agent (will fail due to fake key, but runner should handle it)
    let yaml = r#"
name: test_wf
version: 1
description: Test workflow
steps:
  - id: agent1
    action: spawn
    task: "Say hello"
    provider: openai
"#;
    let wf = mcp::workflow::parse_workflow_yaml(yaml).unwrap();
    let run_id = runner.run(wf, manager.clone()).await.unwrap();
    assert!(run_id > 0);

    // Wait for the workflow to finish (it should fail since the API key is fake)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let info = runner.get_run(run_id).await.unwrap();
        if info.status != mcp::workflow::WorkflowRunStatus::Running {
            // Workflow completed or failed — both are fine for this test
            assert!(
                info.status == mcp::workflow::WorkflowRunStatus::Completed
                    || info.status == mcp::workflow::WorkflowRunStatus::Failed
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("Workflow did not finish within 15 seconds");
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn test_workflow_runner_stop() {
    let config = mcp::config::McpConfig::default();
    let manager = std::sync::Arc::new(mcp::agent::AgentManager::new(&config, test_db()).unwrap());
    let runner = mcp::workflow::WorkflowRunner::new();

    // Workflow with a step that references a non-existent provider (will fail at spawn)
    let yaml = r#"
name: stoppable
steps:
  - id: s1
    action: spawn
    task: "test"
"#;
    let wf = mcp::workflow::parse_workflow_yaml(yaml).unwrap();
    let run_id = runner.run(wf, manager).await.unwrap();

    // Stop it immediately
    runner.stop(run_id).await.unwrap();

    // Should eventually be stopped or already failed
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let info = runner.get_run(run_id).await.unwrap();
    assert!(
        info.status == mcp::workflow::WorkflowRunStatus::Stopped
            || info.status == mcp::workflow::WorkflowRunStatus::Failed
    );
}

#[tokio::test]
async fn test_workflow_runner_not_found() {
    let runner = mcp::workflow::WorkflowRunner::new();
    assert!(runner.get_run(9999).await.is_err());
    assert!(runner.stop(9999).await.is_err());
}

// ── Integration: spawn agent via NVIDIA NIM with role ───────────────

fn nim_key() -> Option<String> {
    std::env::var("MCP_NVIDIA_NIM_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

#[tokio::test]
async fn test_spawn_agent_with_local_coder_role_via_nim() {
    let api_key = match nim_key() {
        Some(k) => k,
        None => {
            eprintln!("SKIP: MCP_NVIDIA_NIM_KEY not set");
            return;
        }
    };

    mcp::config::ensure_dirs().unwrap();

    // Install the local_coder role from examples
    let role_text = std::fs::read_to_string("examples/roles/local_coder.toml").unwrap();
    let mut role: mcp::role::RoleDefinition = toml::from_str(&role_text).unwrap();
    // Use the 8b model (cheaper) for CI
    role.default_model = Some("meta/llama-3.1-8b-instruct".into());
    role.name = "_mcp_test_local_coder".into();
    mcp::role::create_role(&role).unwrap();

    // Build config
    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "nvidia-nim".to_string(),
        mcp::config::ProviderEntry {
            provider_type: "nvidia-nim".into(),
            url: Some("https://integrate.api.nvidia.com/v1".into()),
            model: Some("meta/llama-3.1-8b-instruct".into()),
            api_key: Some(api_key),
            timeout: 60,
            max_retries: 2,
            region: None,
        },
    );

    let config = mcp::config::McpConfig {
        default: mcp::config::DefaultConfig {
            provider: "nvidia-nim".into(),
            model: "meta/llama-3.1-8b-instruct".into(),
            role: Some("_mcp_test_local_coder".into()),
            tool: None,
        },
        provider: providers,
        ..Default::default()
    };

    let mgr = mcp::agent::AgentManager::new(&config, test_db()).unwrap();

    // Resolve the role manually (as main.rs would)
    let loaded_role = mcp::role::get_role("_mcp_test_local_coder").unwrap();
    let sys_prompt = mcp::role::resolve_system_prompt(&loaded_role).unwrap();
    assert!(
        sys_prompt.contains("read-file"),
        "System prompt should mention available tools"
    );
    assert!(
        sys_prompt.contains("NEVER rename or delete"),
        "System prompt should contain safety rules"
    );

    // Spawn with the role's system prompt
    let req = mcp::agent::SpawnRequest {
        task: "List the tools you have available. Reply in one short sentence.".into(),
        role: Some("_mcp_test_local_coder".into()),
        soul: loaded_role.soul.clone(),
        model: Some("meta/llama-3.1-8b-instruct".into()),
        provider: Some("nvidia-nim".into()),
        depth: None,
        max_children: None,
        max_depth: None,
        timeout_sec: Some(60),
        system_prompt: Some(sys_prompt),
        parent_id: None,
    };

    let resp = mgr.spawn(req).await.unwrap();
    assert_eq!(resp.provider, "nvidia-nim");
    assert_eq!(resp.soul.as_deref(), Some("local-code-organizer"));

    // Wait for completion
    let start = std::time::Instant::now();
    loop {
        let status = mgr.get_status(resp.id).await.unwrap();
        match status.status {
            mcp::agent::AgentStatus::Completed => {
                eprintln!("Agent output: {:?}", status.last_output);
                assert!(status.last_output.is_some(), "Agent should have output");
                let output = status.last_output.unwrap();
                assert!(!output.is_empty(), "Output should not be empty");
                break;
            }
            mcp::agent::AgentStatus::Failed => {
                eprintln!("Agent failed (API may be unavailable): {:?}", status.last_output);
                break;
            }
            _ => {
                if start.elapsed() > std::time::Duration::from_secs(90) {
                    panic!("Agent did not complete within 90s");
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }

    // Register as a tool and verify schema enrichment
    let tool = mcp::tool::register_from_role("_mcp_test_coder_tool", "_mcp_test_local_coder")
        .unwrap();
    assert!(tool.description.contains("read-file"));
    assert!(tool.description.contains("write-file"));
    let allowed = &tool.input_schema["properties"]["allowed_tools"]["items"]["enum"];
    let arr = allowed.as_array().unwrap();
    assert_eq!(arr.len(), 5);
    assert!(arr.contains(&serde_json::json!("read-file")));
    assert!(arr.contains(&serde_json::json!("list-files")));
    assert!(arr.contains(&serde_json::json!("run-command")));

    // Clean up
    mcp::tool::delete_tool("_mcp_test_coder_tool").unwrap();
    mcp::role::delete_role("_mcp_test_local_coder").unwrap();
}

// ── NIM list_models live test ───────────────────────────────────────

#[tokio::test]
async fn test_nvidia_nim_list_models_live() {
    let api_key = match nim_key() {
        Some(k) => k,
        None => {
            eprintln!("SKIP: MCP_NVIDIA_NIM_KEY not set");
            return;
        }
    };

    let entry = mcp::config::ProviderEntry {
        provider_type: "nvidia-nim".into(),
        url: Some("https://integrate.api.nvidia.com/v1".into()),
        model: Some("meta/llama-3.1-8b-instruct".into()),
        api_key: Some(api_key),
        timeout: 30,
        max_retries: 1,
        region: None,
    };

    let provider =
        mcp::provider::build_provider("nvidia-nim", &entry, "meta/llama-3.1-8b-instruct")
            .unwrap();
    let models = provider.list_models().await.unwrap();
    eprintln!("NIM models count: {}", models.len());
    assert!(!models.is_empty(), "NIM should return available models");
    // Models should be sorted
    let mut sorted = models.clone();
    sorted.sort();
    assert_eq!(models, sorted, "Models should be sorted alphabetically");
}
