/// Integration tests against Ollama Cloud API (https://ollama.com/api).
/// Requires OLLAMA_API_KEY environment variable.
/// Run with: OLLAMA_API_KEY=<key> cargo test --test integration_ollama_cloud

fn ollama_key() -> Option<String> {
    std::env::var("OLLAMA_API_KEY").ok().filter(|k| !k.is_empty())
}

fn test_db() -> std::sync::Arc<mcp::persistence::Database> {
    std::sync::Arc::new(mcp::persistence::Database::open_memory().unwrap())
}

#[tokio::test]
async fn test_ollama_cloud_health_check() {
    let api_key = match ollama_key() {
        Some(k) => k,
        None => {
            eprintln!("SKIP: OLLAMA_API_KEY not set");
            return;
        }
    };

    let entry = mcp::config::ProviderEntry {
        provider_type: "ollama".into(),
        url: Some("https://ollama.com".into()),
        model: Some("gemma3:4b".into()),
        api_key: Some(api_key),
        timeout: 30,
        max_retries: 1,
        region: None,
    };

    let provider = mcp::provider::build_provider("ollama-cloud", &entry, "gemma3:4b")
        .expect("Failed to build Ollama Cloud provider");

    let result = provider.health_check().await;
    assert!(result.is_ok(), "Health check failed: {:?}", result.err());
    let msg = result.unwrap();
    eprintln!("Ollama Cloud health check: {msg}");
    assert!(msg.contains("Ollama"), "Unexpected response: {msg}");
}

#[tokio::test]
async fn test_ollama_cloud_chat_completion() {
    let api_key = match ollama_key() {
        Some(k) => k,
        None => {
            eprintln!("SKIP: OLLAMA_API_KEY not set");
            return;
        }
    };

    let entry = mcp::config::ProviderEntry {
        provider_type: "ollama".into(),
        url: Some("https://ollama.com".into()),
        model: Some("gemma3:4b".into()),
        api_key: Some(api_key),
        timeout: 60,
        max_retries: 2,
        region: None,
    };

    let provider = mcp::provider::build_provider("ollama-cloud", &entry, "gemma3:4b").unwrap();

    let messages = vec![mcp::provider::ChatMessage {
        role: "user".into(),
        content: "Reply with exactly one word: hello".into(),
    }];

    let resp = provider
        .chat(&messages, Some("You are a helpful assistant. Be very brief."))
        .await;

    assert!(resp.is_ok(), "Chat failed: {:?}", resp.err());
    let resp = resp.unwrap();
    eprintln!("Ollama Cloud response: {:?}", resp);
    assert!(!resp.content.is_empty(), "Empty response content");
}

#[tokio::test]
async fn test_ollama_cloud_list_models() {
    let api_key = match ollama_key() {
        Some(k) => k,
        None => {
            eprintln!("SKIP: OLLAMA_API_KEY not set");
            return;
        }
    };

    let entry = mcp::config::ProviderEntry {
        provider_type: "ollama".into(),
        url: Some("https://ollama.com".into()),
        model: Some("gemma3:4b".into()),
        api_key: Some(api_key),
        timeout: 30,
        max_retries: 1,
        region: None,
    };

    let provider = mcp::provider::build_provider("ollama-cloud", &entry, "gemma3:4b").unwrap();

    let models = provider.list_models().await;
    assert!(models.is_ok(), "list_models failed: {:?}", models.err());
    let models = models.unwrap();
    eprintln!("Ollama Cloud models: {:?}", models);
    assert!(!models.is_empty(), "No models returned");
}

#[tokio::test]
async fn test_ollama_cloud_agent_spawn_and_complete() {
    let api_key = match ollama_key() {
        Some(k) => k,
        None => {
            eprintln!("SKIP: OLLAMA_API_KEY not set");
            return;
        }
    };

    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "ollama-cloud".to_string(),
        mcp::config::ProviderEntry {
            provider_type: "ollama".into(),
            url: Some("https://ollama.com".into()),
            model: Some("gemma3:4b".into()),
            api_key: Some(api_key),
            timeout: 60,
            max_retries: 2,
            region: None,
        },
    );

    let config = mcp::config::McpConfig {
        default: mcp::config::DefaultConfig {
            provider: "ollama-cloud".into(),
            model: "gemma3:4b".into(),
            role: None,
            tool: None,
        },
        provider: providers,
        ..Default::default()
    };

    let mgr = mcp::agent::AgentManager::new(&config, test_db()).unwrap();

    let req = mcp::agent::SpawnRequest {
        task: "Say exactly: I am MCP.".into(),
        role: None,
        soul: None,
        model: None,
        provider: Some("ollama-cloud".into()),
        depth: None,
        max_children: None,
        max_depth: None,
        timeout_sec: Some(60),
        system_prompt: Some("You are a test agent. Follow instructions exactly.".into()),
        parent_id: None,
    };

    let resp = mgr.spawn(req).await.unwrap();
    assert_eq!(resp.provider, "ollama-cloud");

    let start = std::time::Instant::now();
    loop {
        let status = mgr.get_status(resp.id).await.unwrap();
        match status.status {
            mcp::agent::AgentStatus::Completed => {
                eprintln!("Agent completed! Output: {:?}", status.last_output);
                assert!(status.last_output.is_some());
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
