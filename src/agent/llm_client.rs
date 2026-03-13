use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::types::{LlmDecision, TuneDecision};

/// Claude Sonnet API client for trade decisions and signal tuning.
pub struct LlmClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<ApiMessage>,
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

impl LlmClient {
    pub fn new(api_key: String, model: String, timeout_ms: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .expect("Failed to build HTTP client");

        info!(model = %model, "LlmClient initialized");
        Self {
            client,
            api_key,
            model,
        }
    }

    /// Send signal context to Claude and parse trade decision.
    pub async fn decide(
        &self,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<LlmDecision> {
        let text = self.call_api(system_prompt, user_prompt, 512).await?;

        // Extract JSON from the response (may be wrapped in markdown code blocks)
        let json_str = extract_json(&text);

        for attempt in 0..3 {
            match serde_json::from_str::<LlmDecision>(json_str) {
                Ok(decision) => return Ok(decision),
                Err(e) => {
                    if attempt < 2 {
                        warn!(attempt = attempt + 1, error = %e, "Failed to parse LLM decision, retrying");
                        // Retry with a hint
                        let retry_prompt = format!(
                            "Your previous response was not valid JSON. Error: {e}\n\
                             Please respond with ONLY valid JSON matching the LlmDecision schema.\n\
                             Original request:\n{user_prompt}"
                        );
                        let retry_text = self.call_api(system_prompt, &retry_prompt, 512).await?;
                        let retry_json = extract_json(&retry_text);
                        if let Ok(d) = serde_json::from_str::<LlmDecision>(retry_json) {
                            return Ok(d);
                        }
                    }
                }
            }
        }

        // Default to Skip on parse failure
        warn!("All LLM parse attempts failed, defaulting to Skip");
        Ok(LlmDecision::Skip {
            reasoning: "LLM response parse failure — auto-skip".to_string(),
        })
    }

    /// Generic prompt: send system + user prompt, return raw text response.
    pub async fn prompt(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: u32,
    ) -> Result<String> {
        self.call_api(system_prompt, user_prompt, max_tokens).await
    }

    /// Send trade history + memory to Claude for signal engine tuning.
    pub async fn tune(
        &self,
        system_prompt: &str,
        context: &str,
    ) -> Result<TuneDecision> {
        let text = self.call_api(system_prompt, context, 1024).await?;
        let json_str = extract_json(&text);

        serde_json::from_str::<TuneDecision>(json_str)
            .context("Failed to parse TuneDecision from LLM response")
    }

    async fn call_api(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: u32,
    ) -> Result<String> {
        let request = ApiRequest {
            model: self.model.clone(),
            max_tokens,
            system: system_prompt.to_string(),
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: user_prompt.to_string(),
            }],
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Claude API")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Claude API returned {status}: {body}");
        }

        let api_response: ApiResponse = response
            .json()
            .await
            .context("Failed to parse Claude API response")?;

        api_response
            .content
            .first()
            .and_then(|b| b.text.clone())
            .context("Empty response from Claude API")
    }
}

/// Extract JSON from a string that may contain markdown code blocks.
fn extract_json(text: &str) -> &str {
    let trimmed = text.trim();
    // Try to extract from ```json ... ``` blocks
    if let Some(start) = trimmed.find("```json") {
        let json_start = start + 7;
        if let Some(end) = trimmed[json_start..].find("```") {
            return trimmed[json_start..json_start + end].trim();
        }
    }
    // Try to extract from ``` ... ``` blocks
    if let Some(start) = trimmed.find("```") {
        let json_start = start + 3;
        if let Some(end) = trimmed[json_start..].find("```") {
            return trimmed[json_start..json_start + end].trim();
        }
    }
    // Try to find raw JSON object
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return &trimmed[start..=end];
        }
    }
    trimmed
}
