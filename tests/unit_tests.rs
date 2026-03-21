/// Unit tests for MCP core logic — config, roles, agent manager, provider factory.
/// These run without network access (no API keys needed).

// ── Config tests ──────────────────────────────────────────────────────

#[test]
fn test_default_config_values() {
    let cfg = mcp::config::McpConfig::default();
    assert_eq!(cfg.default.provider, "openai");
    assert_eq!(cfg.default.model, "gpt-4o");
    assert_eq!(cfg.limits.max_concurrent_agents, 8);
    assert_eq!(cfg.limits.max_depth, 2);
    assert_eq!(cfg.limits.max_children_per_parent, 5);
    assert_eq!(cfg.limits.agent_timeout_sec, 600);
    assert_eq!(cfg.server.bind, "127.0.0.1:29999");
    assert!(cfg.server.enabled);
    assert!(!cfg.server.tls);
    assert!(!cfg.cli.json_output);
}

#[test]
fn test_config_toml_parsing() {
    let toml_str = r#"
[default]
provider = "nvidia-nim"
model = "nvidia/llama-3-70b-instruct"

[server]
enabled = false
bind = "0.0.0.0:8080"

[limits]
max_concurrent_agents = 4
max_depth = 3
max_children_per_parent = 10
agent_timeout_sec = 1200

[cli]
json_output = true

[provider.nvidia-nim]
type = "nvidia-nim"
url = "https://integrate.api.nvidia.com/v1"
model = "nvidia/llama-3-70b-instruct"
api_key = "test-key"
timeout = 120
max_retries = 2
"#;

    let cfg: mcp::config::McpConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.default.provider, "nvidia-nim");
    assert!(!cfg.server.enabled);
    assert_eq!(cfg.server.bind, "0.0.0.0:8080");
    assert_eq!(cfg.limits.max_concurrent_agents, 4);
    assert_eq!(cfg.limits.max_depth, 3);
    assert!(cfg.cli.json_output);
    assert!(cfg.provider.contains_key("nvidia-nim"));

    let nim = &cfg.provider["nvidia-nim"];
    assert_eq!(nim.provider_type, "nvidia-nim");
    assert_eq!(nim.timeout, 120);
    assert_eq!(nim.max_retries, 2);
}

#[test]
fn test_env_resolution_in_api_key() {
    // Set a test env var
    std::env::set_var("MCP_TEST_KEY_12345", "resolved-secret");

    let entry = mcp::config::ProviderEntry {
        provider_type: "openai".into(),
        url: Some("https://api.openai.com/v1".into()),
        model: Some("gpt-4o".into()),
        api_key: Some("<env:MCP_TEST_KEY_12345>".into()),
        timeout: 300,
        max_retries: 3,
        region: None,
    };

    assert_eq!(entry.resolved_api_key(), Some("resolved-secret".into()));

    // Plain key (no env: prefix) should pass through
    let entry2 = mcp::config::ProviderEntry {
        api_key: Some("literal-key".into()),
        ..entry.clone()
    };
    assert_eq!(entry2.resolved_api_key(), Some("literal-key".into()));

    // No key
    let entry3 = mcp::config::ProviderEntry {
        api_key: None,
        ..entry
    };
    assert_eq!(entry3.resolved_api_key(), None);

    std::env::remove_var("MCP_TEST_KEY_12345");
}

#[test]
fn test_mcp_home_is_under_user_home() {
    let home = mcp::config::mcp_home();
    assert!(home.ends_with(".mastercontrolprogram"));
}

// ── Provider factory tests ────────────────────────────────────────────

#[test]
fn test_build_openai_provider() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "openai".into(),
        url: Some("https://api.openai.com/v1".into()),
        model: Some("gpt-4o".into()),
        api_key: Some("sk-test".into()),
        timeout: 60,
        max_retries: 1,
        region: None,
    };
    let provider = mcp::provider::build_provider("openai", &entry, "gpt-4o").unwrap();
    assert_eq!(provider.model(), "gpt-4o");
}

#[test]
fn test_build_anthropic_provider() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "anthropic".into(),
        url: Some("https://api.anthropic.com/v1".into()),
        model: Some("claude-sonnet-4-20250514".into()),
        api_key: Some("sk-ant-test".into()),
        timeout: 60,
        max_retries: 1,
        region: None,
    };
    let provider = mcp::provider::build_provider("anthropic", &entry, "claude-sonnet-4-20250514").unwrap();
    assert_eq!(provider.model(), "claude-sonnet-4-20250514");
}

#[test]
fn test_build_nvidia_nim_provider() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "nvidia-nim".into(),
        url: Some("https://integrate.api.nvidia.com/v1".into()),
        model: Some("nvidia/llama-3.1-nemotron-70b-instruct".into()),
        api_key: Some("nvapi-test".into()),
        timeout: 60,
        max_retries: 1,
        region: None,
    };
    let provider = mcp::provider::build_provider("nvidia-nim", &entry, "nvidia/llama-3.1-nemotron-70b-instruct").unwrap();
    assert_eq!(provider.model(), "nvidia/llama-3.1-nemotron-70b-instruct");
}

#[test]
fn test_build_huggingface_provider() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "huggingface".into(),
        url: Some("https://api-inference.huggingface.co/models".into()),
        model: Some("meta-llama/Meta-Llama-3-70B-Instruct".into()),
        api_key: Some("hf-test".into()),
        timeout: 60,
        max_retries: 1,
        region: None,
    };
    let provider = mcp::provider::build_provider("huggingface", &entry, "meta-llama/Meta-Llama-3-70B-Instruct").unwrap();
    assert_eq!(provider.model(), "meta-llama/Meta-Llama-3-70B-Instruct");
}

#[test]
fn test_build_bedrock_provider() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "amazon-bedrock".into(),
        url: None,
        model: Some("anthropic.claude-3-sonnet-20240229-v1:0".into()),
        api_key: None,
        timeout: 60,
        max_retries: 1,
        region: Some("us-west-2".into()),
    };
    let provider = mcp::provider::build_provider("bedrock", &entry, "anthropic.claude-3-sonnet-20240229-v1:0").unwrap();
    assert_eq!(provider.model(), "anthropic.claude-3-sonnet-20240229-v1:0");
}

#[test]
fn test_build_openai_compatible_provider() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "openai-compatible".into(),
        url: Some("http://localhost:8000/v1".into()),
        model: Some("local-model".into()),
        api_key: Some("none".into()),
        timeout: 60,
        max_retries: 0,
        region: None,
    };
    let provider = mcp::provider::build_provider("local", &entry, "local-model").unwrap();
    assert_eq!(provider.model(), "local-model");
}

#[test]
fn test_build_unknown_provider_fails() {
    let entry = mcp::config::ProviderEntry {
        provider_type: "does-not-exist".into(),
        url: None,
        model: None,
        api_key: None,
        timeout: 60,
        max_retries: 0,
        region: None,
    };
    assert!(mcp::provider::build_provider("bad", &entry, "model").is_err());
}

// ── Agent manager tests (async) ───────────────────────────────────────

#[tokio::test]
async fn test_agent_manager_empty_list() {
    let cfg = mcp::config::McpConfig::default();
    let mgr = mcp::agent::AgentManager::new(&cfg).unwrap();
    let agents = mgr.list_agents(None, None).await.unwrap();
    assert!(agents.is_empty());
}

#[tokio::test]
async fn test_agent_manager_spawn_no_provider_fails() {
    let cfg = mcp::config::McpConfig::default(); // no providers configured
    let mgr = mcp::agent::AgentManager::new(&cfg).unwrap();

    let req = mcp::agent::SpawnRequest {
        task: "test task".into(),
        role: None,
        soul: None,
        model: None,
        provider: Some("nonexistent".into()),
        depth: None,
        max_children: None,
        max_depth: None,
        timeout_sec: None,
        system_prompt: None,
        parent_id: None,
    };

    let result = mgr.spawn(req).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not configured"));
}

#[tokio::test]
async fn test_agent_manager_status_not_found() {
    let cfg = mcp::config::McpConfig::default();
    let mgr = mcp::agent::AgentManager::new(&cfg).unwrap();
    let result = mgr.get_status(999).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[tokio::test]
async fn test_agent_manager_kill_not_found() {
    let cfg = mcp::config::McpConfig::default();
    let mgr = mcp::agent::AgentManager::new(&cfg).unwrap();
    let result = mgr.kill(999).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_agent_manager_kill_all_empty() {
    let cfg = mcp::config::McpConfig::default();
    let mgr = mcp::agent::AgentManager::new(&cfg).unwrap();
    let count = mgr.kill_all().await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_agent_manager_steer_not_found() {
    let cfg = mcp::config::McpConfig::default();
    let mgr = mcp::agent::AgentManager::new(&cfg).unwrap();
    let req = mcp::agent::SteerRequest {
        instruction: Some("do something".into()),
        prompt_patch: None,
    };
    assert!(mgr.steer(999, req).await.is_err());
}

#[tokio::test]
async fn test_agent_manager_pause_not_found() {
    let cfg = mcp::config::McpConfig::default();
    let mgr = mcp::agent::AgentManager::new(&cfg).unwrap();
    assert!(mgr.pause(999).await.is_err());
}

#[tokio::test]
async fn test_agent_manager_resume_not_found() {
    let cfg = mcp::config::McpConfig::default();
    let mgr = mcp::agent::AgentManager::new(&cfg).unwrap();
    assert!(mgr.resume(999).await.is_err());
}

#[tokio::test]
async fn test_agent_manager_check_provider_not_found() {
    let cfg = mcp::config::McpConfig::default();
    let mgr = mcp::agent::AgentManager::new(&cfg).unwrap();
    assert!(mgr.check_provider("nonexistent").await.is_err());
}

// ── Role tests ────────────────────────────────────────────────────────

#[test]
fn test_role_roundtrip() {
    // Ensure dirs exist
    mcp::config::ensure_dirs().unwrap();

    let role = mcp::role::RoleDefinition {
        name: "_mcp_test_role_roundtrip".into(),
        soul: Some("test-soul".into()),
        role: Some("code-gen".into()),
        prompt_file: None,
        system_prompt: Some("You are a test agent.".into()),
        default_model: Some("gpt-4o".into()),
        default_provider: Some("openai".into()),
        max_depth: 2,
        max_children: 5,
        allowed_tools: vec!["read_file".into()],
    };

    // Create
    mcp::role::create_role(&role).unwrap();

    // Get
    let loaded = mcp::role::get_role("_mcp_test_role_roundtrip").unwrap();
    assert_eq!(loaded.name, "_mcp_test_role_roundtrip");
    assert_eq!(loaded.soul.as_deref(), Some("test-soul"));
    assert_eq!(loaded.system_prompt.as_deref(), Some("You are a test agent."));

    // List should include it
    let roles = mcp::role::list_roles().unwrap();
    assert!(roles.iter().any(|r| r.name == "_mcp_test_role_roundtrip"));

    // Patch
    let patched = mcp::role::patch_role(
        "_mcp_test_role_roundtrip",
        Some("Also be concise."),
        None,
        None,
    )
    .unwrap();
    assert!(patched.system_prompt.unwrap().contains("Also be concise."));

    // Delete
    mcp::role::delete_role("_mcp_test_role_roundtrip").unwrap();
    assert!(mcp::role::get_role("_mcp_test_role_roundtrip").is_err());
}

#[test]
fn test_role_get_nonexistent() {
    assert!(mcp::role::get_role("_mcp_no_such_role_ever").is_err());
}

#[test]
fn test_role_delete_nonexistent() {
    assert!(mcp::role::delete_role("_mcp_no_such_role_ever").is_err());
}

// ── Logging tests ─────────────────────────────────────────────────────

#[test]
fn test_read_agent_log_missing() {
    let result = mcp::logging::read_agent_log(999999).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_read_logs_since_empty() {
    let logs = mcp::logging::read_logs_since("1s").unwrap();
    // Just ensure it doesn't crash; content depends on state
    let _ = logs;
}

// ── Chat types tests ──────────────────────────────────────────────────

#[test]
fn test_chat_message_serialization() {
    let msg = mcp::provider::ChatMessage {
        role: "user".into(),
        content: "Hello".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("user"));
    assert!(json.contains("Hello"));

    let deser: mcp::provider::ChatMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deser.role, "user");
    assert_eq!(deser.content, "Hello");
}

#[test]
fn test_spawn_request_serialization() {
    let req = mcp::agent::SpawnRequest {
        task: "write code".into(),
        role: Some("coder".into()),
        soul: Some("rust-engineer".into()),
        model: Some("gpt-4o".into()),
        provider: Some("openai".into()),
        depth: Some(0),
        max_children: Some(3),
        max_depth: Some(2),
        timeout_sec: Some(300),
        system_prompt: Some("Be helpful.".into()),
        parent_id: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    let deser: mcp::agent::SpawnRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(deser.task, "write code");
    assert_eq!(deser.soul.as_deref(), Some("rust-engineer"));
}

#[test]
fn test_agent_status_display() {
    assert_eq!(mcp::agent::AgentStatus::Running.to_string(), "running");
    assert_eq!(mcp::agent::AgentStatus::Completed.to_string(), "completed");
    assert_eq!(mcp::agent::AgentStatus::Failed.to_string(), "failed");
    assert_eq!(mcp::agent::AgentStatus::Killed.to_string(), "killed");
    assert_eq!(mcp::agent::AgentStatus::Paused.to_string(), "paused");
    assert_eq!(mcp::agent::AgentStatus::Queued.to_string(), "queued");
    assert_eq!(
        mcp::agent::AgentStatus::WaitingOnUser.to_string(),
        "waiting-on-user"
    );
}
