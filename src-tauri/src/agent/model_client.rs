use crate::tools::registry::ToolSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// One message in the OpenAI-style chat transcript sent to the
/// model, and accumulated as the loop progresses. Using this wire
/// format directly — rather than inventing our own — is what makes
/// "hosted now, local Ollama later" a config change instead of a
/// rewrite: Groq's API is OpenAI-compatible, and Ollama exposes an
/// OpenAI-compatible /v1/chat/completions endpoint too.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "system" | "user" | "assistant" | "tool"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String, // always "function"
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String, // JSON-encoded string, per the OpenAI wire format
}

/// What the orchestration loop gets back from one model turn.
pub enum ModelTurn {
    ToolCalls(Vec<ToolCallRequest>),
    FinalAnswer(String),
}

#[async_trait::async_trait]
pub trait ModelClient: Send + Sync {
    async fn next_turn(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
    ) -> Result<ModelTurn, String>;
}

/// A client for ANY OpenAI-compatible chat-completions API.
/// Deliberately not Groq-specific — Groq's API IS this shape, and
/// Ollama serves the same shape locally. Swapping hosted -> local
/// GPU later is `OpenAiCompatibleClient::groq(...)` ->
/// `OpenAiCompatibleClient::local_ollama(...)`, nothing else changes.
pub struct OpenAiCompatibleClient {
    pub base_url: String,
    pub api_key: Option<String>, // None for local Ollama — no key needed
    pub model: String,
}

impl OpenAiCompatibleClient {
    pub fn groq(api_key: String, model: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.groq.com/openai/v1".to_string(),
            api_key: Some(api_key),
            model: model.into(),
        }
    }

    pub fn local_ollama(model: impl Into<String>) -> Self {
        Self {
            base_url: "http://localhost:11434/v1".to_string(),
            api_key: None,
            model: model.into(),
        }
    }
}

fn to_openai_tool(schema: &ToolSchema) -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": schema.name,
            "description": schema.description,
            "parameters": schema.parameters,
        }
    })
}

/// Total attempts (first try + retries) for a single model turn.
const MAX_ATTEMPTS: usize = 5;

/// If the API asks us to wait longer than this (e.g. a daily quota
/// message like "try again in 14m26s"), don't sit there silently —
/// fail with the real message so the user knows what happened.
const MAX_AUTO_WAIT_SECS: f64 = 60.0;

/// Parses Groq-style hints such as "Please try again in 4.8s",
/// "try again in 1m2.5s" or "try again in 850ms" into seconds.
fn parse_retry_seconds(msg: &str) -> Option<f64> {
    let marker = "try again in ";
    let rest = &msg[msg.find(marker)? + marker.len()..];
    let chars: Vec<char> = rest.chars().collect();

    let mut i = 0;
    let mut total = 0.0;
    let mut parsed_any = false;

    while i < chars.len() {
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        if start == i {
            break;
        }
        let value: f64 = match chars[start..i].iter().collect::<String>().parse() {
            Ok(v) => v,
            Err(_) => break, // e.g. a sentence-ending "." after "4.8s"
        };
        if i >= chars.len() {
            break;
        }
        match chars[i] {
            'm' if i + 1 < chars.len() && chars[i + 1] == 's' => {
                total += value / 1000.0;
                i += 2;
            }
            'm' => {
                total += value * 60.0;
                i += 1;
            }
            's' => {
                total += value;
                i += 1;
            }
            _ => break,
        }
        parsed_any = true;
    }

    if parsed_any {
        Some(total)
    } else {
        None
    }
}

#[async_trait::async_trait]
impl ModelClient for OpenAiCompatibleClient {
    async fn next_turn(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
    ) -> Result<ModelTurn, String> {
        let client = reqwest::Client::new();
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let tool_defs: Vec<Value> = tools.iter().map(to_openai_tool).collect();

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": tool_defs,
            "tool_choice": "auto",
        });

        let mut attempt = 0;
        let raw: Value = loop {
            attempt += 1;

            let mut req = client.post(&url).json(&body);
            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }

            let resp = req
                .send()
                .await
                .map_err(|e| format!("Model API request failed: {}", e))?;

            let status = resp.status();
            let retry_after_header = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<f64>().ok());

            let raw: Value = resp
                .json()
                .await
                .map_err(|e| format!("Model API returned unparseable response: {}", e))?;

            if status.is_success() {
                break raw;
            }

            let code = raw.pointer("/error/code").and_then(|c| c.as_str()).unwrap_or("");
            let message = raw.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("");

            // Two failure kinds are worth retrying automatically:
            //  - 429: rate limit (free tiers have small tokens-per-minute caps)
            //  - 400 tool_use_failed: the model emitted a malformed/hallucinated
            //    tool call and Groq's validator rejected it. Non-deterministic,
            //    so trying the same turn again often works.
            let wait_secs: Option<f64> = if status.as_u16() == 429 {
                Some(
                    retry_after_header
                        .or_else(|| parse_retry_seconds(message))
                        .unwrap_or(10.0)
                        + 0.5,
                )
            } else if status.as_u16() == 400 && code == "tool_use_failed" {
                Some(1.0)
            } else {
                None
            };

            match wait_secs {
                Some(w) if attempt < MAX_ATTEMPTS && w <= MAX_AUTO_WAIT_SECS => {
                    eprintln!(
                        "Model API {} ({}), retrying in {:.1}s (attempt {}/{})",
                        status, code, w, attempt, MAX_ATTEMPTS
                    );
                    tokio::time::sleep(Duration::from_secs_f64(w)).await;
                    continue;
                }
                _ => return Err(format!("Model API error ({}): {}", status, raw)),
            }
        };

        let message = raw
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .ok_or_else(|| format!("Model API response had no choices/message: {}", raw))?;

        if let Some(tool_calls_raw) = message.get("tool_calls") {
            let tool_calls: Vec<ToolCallRequest> = serde_json::from_value(tool_calls_raw.clone())
                .map_err(|e| format!("Could not parse tool_calls: {}", e))?;
            if !tool_calls.is_empty() {
                return Ok(ModelTurn::ToolCalls(tool_calls));
            }
        }

        let content = message.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
        Ok(ModelTurn::FinalAnswer(content))
    }
}