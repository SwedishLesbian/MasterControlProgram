use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

use super::{ChatMessage, ChatResponse, Provider};

/// HuggingFace Inference API provider.
/// Uses the /models/{model}/v1/chat/completions endpoint (OpenAI-compat).
#[derive(Debug)]
pub struct HuggingFaceProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    max_retries: u32,
    http: Client,
}

impl HuggingFaceProvider {
    pub fn new(
        name: &str,
        base_url: &str,
        model: &str,
        api_key: &str,
        timeout_secs: u32,
        max_retries: u32,
    ) -> Result<Self> {
        let url = if base_url.is_empty() {
            "https://api-inference.huggingface.co/models".to_string()
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
impl Provider for HuggingFaceProvider {
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
        // HF Inference API supports OpenAI-compatible chat completions
        let url = format!(
            "{}/{}/v1/chat/completions",
            self.base_url.trim_end_matches('/'),
            self.model
        );

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
            "HuggingFace provider '{}' failed after {} retries: {}",
            self.name,
            self.max_retries,
            last_err.unwrap_or_default()
        )
    }

    async fn health_check(&self) -> Result<String> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            self.model
        );
        match self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
        {
            Ok(r) => Ok(format!("HuggingFace ({}): HTTP {}", self.name, r.status())),
            Err(e) => Ok(format!("HuggingFace ({}): error — {}", self.name, e)),
        }
    }
}
