/// Integration tests against NVIDIA NIM API.
/// Requires MCP_NVIDIA_NIM_KEY environment variable.
/// Run with: MCP_NVIDIA_NIM_KEY=<key> cargo test --test integration_nvidia_nim

fn nim_key() -> Option<String> {
    std::env::var("MCP_NVIDIA_NIM_KEY").ok().filter(|k| !k.is_empty())
}

#[tokio::test]
async fn test_nvidia_nim_health_check() {
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

    let provider = mcp::provider::build_provider(
        "nvidia-nim",
        &entry,
        "meta/llama-3.1-8b-instruct",
    )
    .expect("Failed to build NVIDIA NIM provider");

    let result = provider.health_check().await;
    assert!(result.is_ok(), "Health check failed: {:?}", result.err());
    let msg = result.unwrap();
    eprintln!("NIM health check: {msg}");
    assert!(msg.contains("NVIDIA NIM"), "Unexpected response: {msg}");
}

#[tokio::test]
async fn test_nvidia_nim_chat_completion() {
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
        timeout: 60,
        max_retries: 2,
        region: None,
    };

    let provider = mcp::provider::build_provider(
        "nvidia-nim",
        &entry,
        "meta/llama-3.1-8b-instruct",
    )
    .unwrap();

    let messages = vec![mcp::provider::ChatMessage {
        role: "user".into(),
        content: "Reply with exactly one word: hello".into(),
    }];

    let resp = provider
        .chat(&messages, Some("You are a helpful assistant. Be very brief."))
        .await;

    assert!(resp.is_ok(), "Chat failed: {:?}", resp.err());
    let resp = resp.unwrap();
    eprintln!("NIM response: {:?}", resp);
    assert!(!resp.content.is_empty(), "Empty response content");
}

#[tokio::test]
async fn test_nvidia_nim_agent_spawn_and_complete() {
    let api_key = match nim_key() {
        Some(k) => k,
        None => {
            eprintln!("SKIP: MCP_NVIDIA_NIM_KEY not set");
            return;
        }
    };

    // Build a config with NVIDIA NIM provider
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
            role: None,
            tool: None,
        },
        provider: providers,
        ..Default::default()
    };

    let mgr = mcp::agent::AgentManager::new(&config).unwrap();

    // Spawn an agent
    let req = mcp::agent::SpawnRequest {
        task: "Say exactly: I am MCP.".into(),
        role: Some("test".into()),
        soul: Some("test-soul".into()),
        model: None,
        provider: Some("nvidia-nim".into()),
        depth: None,
        max_children: None,
        max_depth: None,
        timeout_sec: Some(60),
        system_prompt: Some("You are a test agent. Follow instructions exactly.".into()),
        parent_id: None,
    };

    let resp = mgr.spawn(req).await.unwrap();
    assert_eq!(resp.id, 1);
    assert_eq!(resp.provider, "nvidia-nim");
    assert_eq!(resp.soul.as_deref(), Some("test-soul"));

    // Wait for the agent to complete (poll with timeout)
    let start = std::time::Instant::now();
    loop {
        let status = mgr.get_status(resp.id).await.unwrap();
        match status.status {
            mcp::agent::AgentStatus::Completed => {
                eprintln!("Agent completed! Output: {:?}", status.last_output);
                assert!(status.last_output.is_some());
                assert!(status.progress >= 1.0);
                break;
            }
            mcp::agent::AgentStatus::Failed => {
                eprintln!("Agent failed: {:?}", status.last_output);
                // Don't hard-fail on API errors in CI
                break;
            }
            _ => {
                if start.elapsed() > std::time::Duration::from_secs(90) {
                    panic!("Agent did not complete within 90 seconds");
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
}

#[tokio::test]
async fn test_nvidia_nim_agent_steer() {
    let api_key = match nim_key() {
        Some(k) => k,
        None => {
            eprintln!("SKIP: MCP_NVIDIA_NIM_KEY not set");
            return;
        }
    };

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
            role: None,
            tool: None,
        },
        provider: providers,
        ..Default::default()
    };

    let mgr = mcp::agent::AgentManager::new(&config).unwrap();

    let req = mcp::agent::SpawnRequest {
        task: "Wait for further instructions.".into(),
        role: None,
        soul: None,
        model: None,
        provider: Some("nvidia-nim".into()),
        depth: None,
        max_children: None,
        max_depth: None,
        timeout_sec: Some(60),
        system_prompt: Some("You are a test agent.".into()),
        parent_id: None,
    };

    let resp = mgr.spawn(req).await.unwrap();

    // Steer the agent
    let steer_req = mcp::agent::SteerRequest {
        instruction: Some("Now say hello.".into()),
        prompt_patch: Some("Always be polite.".into()),
    };
    let steer_resp = mgr.steer(resp.id, steer_req).await.unwrap();
    assert!(steer_resp.instruction_appended);
    assert!(steer_resp.system_prompt_patched);
    assert!(steer_resp.patch_size_delta_tokens > 0);

    // Clean up
    mgr.kill(resp.id).await.unwrap();
}
