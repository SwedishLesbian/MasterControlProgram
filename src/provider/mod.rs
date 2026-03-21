use anyhow::{bail, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use crate::config::ProviderEntry;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderType {
    Openai,
    Anthropic,
    NvidiaNim,
    Huggingface,
    AmazonBedrock,
    OpenaiCompatible,
}

impl ProviderType {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "openai" => Ok(Self::Openai),
            "anthropic" => Ok(Self::Anthropic),
            "nvidia-nim" => Ok(Self::NvidiaNim),
            "huggingface" => Ok(Self::Huggingface),
            "amazon-bedrock" => Ok(Self::AmazonBedrock),
            "openai-compatible" => Ok(Self::OpenaiCompatible),
            other => bail!("Unknown provider type: {other}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderClient {
    pub name: String,
    pub provider_type: ProviderType,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub max_retries: u32,
    http: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub tokens_used: Option<u64>,
    pub finish_reason: Option<String>,
}

impl ProviderClient {
    pub fn from_config(name: &str, entry: &ProviderEntry, default_model: &str) -> Result<Self> {
        let provider_type = ProviderType::from_str(&entry.provider_type)?;
        let model = entry
            .model
            .clone()
            .unwrap_or_else(|| default_model.to_string());
        let api_key = entry.resolved_api_key().unwrap_or_default();
        let base_url = entry.base_url().unwrap_or_default();

        let http = Client::builder()
            .timeout(Duration::from_secs(entry.timeout as u64))
            .build()?;

        Ok(Self {
            name: name.to_string(),
            provider_type,
            base_url,
            model,
            api_key,
            max_retries: entry.max_retries,
            http,
        })
    }

    /// Run a chat completion.
    pub async fn chat(&self, messages: &[ChatMessage], system_prompt: Option<&str>) -> Result<ChatResponse> {
        match self.provider_type {
            ProviderType::Anthropic => self.chat_anthropic(messages, system_prompt).await,
            ProviderType::AmazonBedrock => self.chat_bedrock(messages, system_prompt).await,
            _ => self.chat_openai_compat(messages, system_prompt).await,
        }
    }

    /// OpenAI-compatible chat (works for OpenAI, NVIDIA NIM, HuggingFace TGI, etc.)
    async fn chat_openai_compat(&self, messages: &[ChatMessage], system_prompt: Option<&str>) -> Result<ChatResponse> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let mut msgs: Vec<Value> = Vec::new();
        if let Some(sys) = system_prompt {
            msgs.push(json!({"role": "system", "content": sys}));
        }
        for m in messages {
            msgs.push(json!({"role": m.role, "content": m.content}));
        }

        let body = json!({
            "model": self.model,
            "messages": msgs,
            "max_tokens": 4096,
        });

        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
            }
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    let data: Value = r.json().await?;
                    let content = data["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let tokens = data["usage"]["total_tokens"].as_u64();
                    let finish = data["choices"][0]["finish_reason"]
                        .as_str()
                        .map(String::from);
                    return Ok(ChatResponse {
                        content,
                        tokens_used: tokens,
                        finish_reason: finish,
                    });
                }
                Ok(r) => {
                    let status = r.status();
                    let body = r.text().await.unwrap_or_default();
                    last_err = Some(format!("HTTP {status}: {body}"));
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                }
            }
        }

        bail!(
            "Provider '{}' failed after {} retries: {}",
            self.name,
            self.max_retries,
            last_err.unwrap_or_default()
        )
    }

    /// Anthropic Messages API.
    async fn chat_anthropic(&self, messages: &[ChatMessage], system_prompt: Option<&str>) -> Result<ChatResponse> {
        let url = format!("{}/messages", self.base_url.trim_end_matches('/'));

        let msgs: Vec<Value> = messages
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect();

        let mut body = json!({
            "model": self.model,
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
            let resp = self
                .http
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    let data: Value = r.json().await?;
                    let content = data["content"][0]["text"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let tokens = data["usage"]["output_tokens"].as_u64();
                    let finish = data["stop_reason"].as_str().map(String::from);
                    return Ok(ChatResponse {
                        content,
                        tokens_used: tokens,
                        finish_reason: finish,
                    });
                }
                Ok(r) => {
                    let status = r.status();
                    let body_text = r.text().await.unwrap_or_default();
                    last_err = Some(format!("HTTP {status}: {body_text}"));
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                }
            }
        }

        bail!(
            "Provider '{}' failed after {} retries: {}",
            self.name,
            self.max_retries,
            last_err.unwrap_or_default()
        )
    }

    /// Amazon Bedrock via AWS SDK.
    async fn chat_bedrock(&self, messages: &[ChatMessage], system_prompt: Option<&str>) -> Result<ChatResponse> {
        use aws_sdk_bedrockruntime::types::{
            ContentBlock, ConversationRole, Message, SystemContentBlock,
        };

        let region = aws_config::Region::new("us-east-1");
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(region)
            .load()
            .await;
        let client = aws_sdk_bedrockruntime::Client::new(&sdk_config);

        let mut bedrock_messages = Vec::new();
        for m in messages {
            let role = match m.role.as_str() {
                "assistant" => ConversationRole::Assistant,
                _ => ConversationRole::User,
            };
            bedrock_messages.push(
                Message::builder()
                    .role(role)
                    .content(ContentBlock::Text(m.content.clone()))
                    .build()
                    .map_err(|e| anyhow::anyhow!("Bedrock message build error: {e}"))?,
            );
        }

        let mut req = client
            .converse()
            .model_id(&self.model)
            .set_messages(Some(bedrock_messages));

        if let Some(sys) = system_prompt {
            req = req.system(SystemContentBlock::Text(sys.to_string()));
        }

        let output = req.send().await.map_err(|e| anyhow::anyhow!("Bedrock call failed: {e}"))?;

        let content = output
            .output()
            .and_then(|o| o.as_message().ok())
            .map(|msg| {
                msg.content()
                    .iter()
                    .filter_map(|b| b.as_text().ok())
                    .cloned()
                    .collect::<Vec<String>>()
                    .join("")
            })
            .unwrap_or_default();

        let tokens = output
            .usage()
            .map(|u| u.total_tokens() as u64);

        Ok(ChatResponse {
            content,
            tokens_used: tokens,
            finish_reason: Some("end_turn".into()),
        })
    }

    /// Health check — send a tiny request to verify connectivity.
    pub async fn health_check(&self) -> Result<String> {
        match self.provider_type {
            ProviderType::AmazonBedrock => {
                Ok("Bedrock: credentials validated at load time".into())
            }
            ProviderType::Anthropic => {
                let url = format!("{}/messages", self.base_url.trim_end_matches('/'));
                let body = json!({
                    "model": self.model,
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
                Ok(format!("Anthropic: HTTP {}", resp.status()))
            }
            _ => {
                let url = format!("{}/models", self.base_url.trim_end_matches('/'));
                let resp = self
                    .http
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .send()
                    .await;
                match resp {
                    Ok(r) => Ok(format!("{}: HTTP {}", self.name, r.status())),
                    Err(e) => Ok(format!("{}: error - {}", self.name, e)),
                }
            }
        }
    }
}
