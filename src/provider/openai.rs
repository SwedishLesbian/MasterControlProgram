use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

use super::{ChatMessage, ChatResponse, Provider};

#[derive(Debug)]
pub struct OpenAiProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    max_retries: u32,
    http: Client,
}

impl OpenAiProvider {
    pub fn new(
        name: &str,
        base_url: &str,
        model: &str,
        api_key: &str,
        timeout_secs: u32,
        max_retries: u32,
    ) -> Result<Self> {
        let url = if base_url.is_empty() {
            "https://api.openai.com/v1".to_string()
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
impl Provider for OpenAiProvider {
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
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let mut msgs: Vec<Value> = Vec::new();
        if let Some(sys) = system_prompt {
            msgs.push(json!({"role": "system", "content": sys}));
        }
        for m in messages {
            msgs.push(json!({"role": &m.role, "content": &m.content}));
        }

        let body = json!({
            "model": &self.model,
            "messages": msgs,
            "max_tokens": 4096,
        });

        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
            }

            match self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {
                    let data: Value = r.json().await?;
                    return Ok(ChatResponse {
                        content: data["choices"][0]["message"]["content"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                        tokens_used: data["usage"]["total_tokens"].as_u64(),
                        finish_reason: data["choices"][0]["finish_reason"]
                            .as_str()
                            .map(String::from),
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
            "OpenAI provider '{}' failed after {} retries: {}",
            self.name,
            self.max_retries,
            last_err.unwrap_or_default()
        )
    }

    async fn health_check(&self) -> Result<String> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        match self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
        {
            Ok(r) => Ok(format!("OpenAI ({}): HTTP {}", self.name, r.status())),
            Err(e) => Ok(format!("OpenAI ({}): error — {}", self.name, e)),
        }
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !resp.status().is_success() {
            bail!("Failed to list models: HTTP {}", resp.status());
        }

        let data: Value = resp.json().await?;
        let mut models: Vec<String> = data["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        models.sort();
        Ok(models)
    }
}
