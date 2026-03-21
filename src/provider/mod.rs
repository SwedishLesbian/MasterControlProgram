mod openai;
mod anthropic;
mod nvidia_nim;
mod huggingface;
mod bedrock;

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::ProviderEntry;

pub use openai::OpenAiProvider;
pub use anthropic::AnthropicProvider;
pub use nvidia_nim::NvidiaNimProvider;
pub use huggingface::HuggingFaceProvider;
pub use bedrock::BedrockProvider;

// ── Shared types ──────────────────────────────────────────────────────

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

// ── Provider trait ────────────────────────────────────────────────────

#[async_trait]
#[allow(dead_code)]
pub trait Provider: Send + Sync + std::fmt::Debug {
    /// Human-readable name for this provider instance.
    fn name(&self) -> &str;

    /// The model this provider is configured to use.
    fn model(&self) -> &str;

    /// Run a chat completion.
    async fn chat(
        &self,
        messages: &[ChatMessage],
        system_prompt: Option<&str>,
    ) -> Result<ChatResponse>;

    /// Lightweight connectivity / health check.
    async fn health_check(&self) -> Result<String>;

    /// List available models from this provider.
    /// Returns model IDs. Not all providers support this via API;
    /// those return a curated list or the currently configured model.
    async fn list_models(&self) -> Result<Vec<String>>;
}

// ── Factory ───────────────────────────────────────────────────────────

/// Build a concrete provider from a config entry.
pub fn build_provider(
    name: &str,
    entry: &ProviderEntry,
    default_model: &str,
) -> Result<Box<dyn Provider>> {
    let model = entry
        .model
        .clone()
        .unwrap_or_else(|| default_model.to_string());
    let api_key = entry.resolved_api_key().unwrap_or_default();
    let base_url = entry.base_url().unwrap_or_default();
    let timeout = entry.timeout;
    let max_retries = entry.max_retries;

    match entry.provider_type.as_str() {
        "openai" => Ok(Box::new(OpenAiProvider::new(
            name, &base_url, &model, &api_key, timeout, max_retries,
        )?)),
        "anthropic" => Ok(Box::new(AnthropicProvider::new(
            name, &base_url, &model, &api_key, timeout, max_retries,
        )?)),
        "nvidia-nim" | "nvidia_nim" => Ok(Box::new(NvidiaNimProvider::new(
            name, &base_url, &model, &api_key, timeout, max_retries,
        )?)),
        "huggingface" | "hf" => Ok(Box::new(HuggingFaceProvider::new(
            name, &base_url, &model, &api_key, timeout, max_retries,
        )?)),
        "amazon-bedrock" | "bedrock" => {
            let region = entry.region.clone().unwrap_or_else(|| "us-east-1".into());
            Ok(Box::new(BedrockProvider::new(name, &model, &region)?))
        }
        "openai-compatible" | "openai_compatible" => Ok(Box::new(OpenAiProvider::new(
            name, &base_url, &model, &api_key, timeout, max_retries,
        )?)),
        other => bail!("Unknown provider type: {other}"),
    }
}
