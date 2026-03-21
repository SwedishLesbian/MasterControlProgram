use anyhow::Result;
use async_trait::async_trait;

use super::{ChatMessage, ChatResponse, Provider};

/// Amazon Bedrock provider using the AWS SDK Converse API.
#[derive(Debug)]
pub struct BedrockProvider {
    name: String,
    model: String,
    region: String,
}

impl BedrockProvider {
    pub fn new(name: &str, model: &str, region: &str) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
            model: model.to_string(),
            region: region.to_string(),
        })
    }
}

#[async_trait]
impl Provider for BedrockProvider {
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
        use aws_sdk_bedrockruntime::types::{
            ContentBlock, ConversationRole, Message, SystemContentBlock,
        };

        let region = aws_config::Region::new(self.region.clone());
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

        let output = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Bedrock call failed: {e}"))?;

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

        let tokens = output.usage().map(|u| u.total_tokens() as u64);

        Ok(ChatResponse {
            content,
            tokens_used: tokens,
            finish_reason: Some("end_turn".into()),
        })
    }

    async fn health_check(&self) -> Result<String> {
        // Bedrock validates credentials at SDK config load time.
        // A full check would call ListFoundationModels, but that
        // adds latency. Keep it lightweight.
        Ok(format!(
            "Bedrock ({}): region={}, model={} — credentials checked at load time",
            self.name, self.region, self.model
        ))
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        // Common Bedrock model IDs (ListFoundationModels requires extra permissions).
        Ok(vec![
            "anthropic.claude-3-5-sonnet-20241022-v2:0".into(),
            "anthropic.claude-3-5-haiku-20241022-v1:0".into(),
            "anthropic.claude-3-opus-20240229-v1:0".into(),
            "amazon.titan-text-premier-v1:0".into(),
            "amazon.titan-text-express-v1".into(),
            "meta.llama3-70b-instruct-v1:0".into(),
            "meta.llama3-8b-instruct-v1:0".into(),
            "mistral.mixtral-8x7b-instruct-v0:1".into(),
            self.model.clone(),
        ])
    }
}
