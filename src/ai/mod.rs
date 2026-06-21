pub mod parser;
pub mod prompts;

use anyhow::Context;
use reqwest::Client;
use serde_json::json;
use crate::error::{AnkrError, Result};
use crate::models::NewCard;

pub struct ClaudeClient {
    client: Client,
    api_key: String,
}

impl ClaudeClient {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| AnkrError::Ai("ANTHROPIC_API_KEY not set".into()))?;
        Ok(Self {
            client: Client::new(),
            api_key,
        })
    }

    pub async fn generate_cards(
        &self,
        source_text: &str,
        deck_name: &str,
        deck_id: i64,
        notetype_id: i64,
    ) -> Result<Vec<NewCard>> {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 2048,
            "system": prompts::SYSTEM_PROMPT,
            "messages": [{
                "role": "user",
                "content": format!("Deck: {deck_name}\n\nText:\n{source_text}")
            }]
        });

        let resp = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("sending request to Claude")
            .map_err(AnkrError::Other)?;

        let status = resp.status();
        let text = resp.text().await
            .context("reading response body")
            .map_err(AnkrError::Other)?;

        if !status.is_success() {
            return Err(AnkrError::Ai(format!("Claude API {status}: {text}")));
        }

        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| AnkrError::Ai(format!("parsing response: {e}")))?;

        let content = json["content"][0]["text"]
            .as_str()
            .ok_or_else(|| AnkrError::Ai("unexpected response shape".into()))?;

        parser::parse_cards(content, deck_id, notetype_id)
    }
}
