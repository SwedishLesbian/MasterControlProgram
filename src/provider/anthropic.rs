use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

use super::{ChatMessage, ChatResponse, Provider};

#[derive(Debug)]
pub struct AnthropicProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    max_retries: u32,
    http: Client,
}

impl AnthropicProvider {
    pub fn new(
        name: &str,
        base_url: &str,
        model: &str,
        api_key: &str,
        timeout_secs: u32,
        max_retries: u32,
    ) -> Result<Self> {
        let url = if base_url.is_empty() {
            "https://api.anthropic.com/v1".to_string()
        } else {
            base_url.to_string()
        };

        let http = Client::builder()
            .timeout(Duration::from_secs(timeout_secs as u64))
            .build()?;

        Ok(Self {
            name: name.to_string(),
            base_url: url,
            model: model.to_string(),
            api_key: api_key.to_string(),
            max_retries,
            http,
        })
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        system_prompt: Option<&str>,
    ) -> Result<ChatResponse> {
        let url = format!("{}/messages", self.base_url.trim_end_matches('/'));

        let msgs: Vec<Value> = messages
            .iter()
            .map(|m| json!({"role": &m.role, "content": &m.content}))
            .collect();

        let mut body = json!({
            "model": &self.model,
            "messages": msgs,
            "max_tokens": 4096,
        });

        if let Some(sys) = system_prompt {
            body["system"] = json!(sys);
        }

        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
            }

            match self
                .http
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {
                    let data: Value = r.json().await?;
                    return Ok(ChatResponse {
                        content: data["content"][0]["text"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                        tokens_used: data["usage"]["output_tokens"].as_u64(),
                        finish_reason: data["stop_reason"].as_str().map(String::from),
                    });
                }
                Ok(r) => {
                    let status = r.status();
                    let text = r.text().await.unwrap_or_default();
                    last_err = Some(format!("HTTP {status}: {text}"));
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                }
            }
        }

        bail!(
            "Anthropic provider '{}' failed after {} retries: {}",
            self.name,
            self.max_retries,
            last_err.unwrap_or_default()
        )
    }

    async fn health_check(&self) -> Result<String> {
        let url = format!("{}/messages", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": &self.model,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
        });
        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;
        Ok(format!("Anthropic ({}): HTTP {}", self.name, resp.status()))
    }
}
