use crate::tools::registry::ToolSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Backup models to switch to when the current one hits its DAILY limit
    /// (Groq limits are per model, so each backup has its own quota).
    /// Empty = never switch (the benchmark uses this, to stay on one model).
    fallbacks: Vec<String>,
    /// Running token/request totals for everything this client has sent.
    /// Read it with `usage()`. A Mutex (not &mut self) so the
    /// ModelClient trait and the orchestrator don't have to change.
    usage: std::sync::Mutex<TokenUsage>,
}

/// Token + request counters, summed over every successful model call.
/// Groq and Ollama both return an OpenAI-style `usage` object; if a
/// provider leaves it out, the token fields simply stay 0.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    /// Successful chat-completion responses.
    pub api_calls: u64,
    /// Automatic retries (429 rate limits, tool_use_failed, connection drops).
    pub api_retries: u64,
}

impl OpenAiCompatibleClient {
    pub fn groq(api_key: String, model: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.groq.com/openai/v1".to_string(),
            api_key: Some(api_key),
            model: model.into(),
            fallbacks: Vec::new(),
            usage: Default::default(),
        }
    }

    pub fn local_ollama(model: impl Into<String>) -> Self {
        Self {
            base_url: "http://localhost:11434/v1".to_string(),
            api_key: None,
            model: model.into(),
            fallbacks: Vec::new(),
            usage: Default::default(),
        }
    }

    /// Adds backup models, tried in order once the main model's daily quota is gone.
    pub fn with_fallbacks(mut self, fallbacks: Vec<String>) -> Self {
        self.fallbacks = fallbacks.into_iter().filter(|m| !m.trim().is_empty() && *m != self.model).collect();
        self
    }

    /// The main model followed by its backups.
    fn chain(&self) -> Vec<String> {
        std::iter::once(self.model.clone()).chain(self.fallbacks.iter().cloned()).collect()
    }

    /// The model the next request will use: the first one in the chain whose
    /// daily quota isn't known to be used up. If all are, the main model.
    pub fn current_model(&self) -> String {
        let chain = self.chain();
        chain.iter().find(|m| !is_exhausted(m)).cloned().unwrap_or_else(|| self.model.clone())
    }

    /// Snapshot of the totals so far (for the benchmark / UI).
    pub fn usage(&self) -> TokenUsage {
        self.usage.lock().map(|u| u.clone()).unwrap_or_default()
    }

    fn record_success(&self, raw: &Value) {
        let n = |k: &str| raw.pointer(&format!("/usage/{}", k)).and_then(|v| v.as_u64()).unwrap_or(0);
        if let Ok(mut u) = self.usage.lock() {
            u.prompt_tokens += n("prompt_tokens");
            u.completion_tokens += n("completion_tokens");
            u.total_tokens += n("total_tokens");
            u.api_calls += 1;
        }
    }

    fn record_retry(&self) {
        if let Ok(mut u) = self.usage.lock() {
            u.api_retries += 1;
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

/// reqwest::Error's default Display only prints the top-level
/// message and silently drops the actual chained cause (DNS failure,
/// TLS handshake failure, connection refused, timeout, etc.) — which
/// is exactly the information needed to tell those apart. This walks
/// the full std::error::Error source chain so the real reason is
/// visible instead of a generic "error sending request" with nothing
/// else to go on.
fn describe_reqwest_error(e: &reqwest::Error) -> String {
    let mut msg = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        msg.push_str(&format!(" | caused by: {}", s));
        source = s.source();
    }
    msg
}

// ---- Daily-limit memory, shared by every client in this app session ----
// run_agent_tool builds a fresh client for every step, so "this model is out of
// quota until <time>" must live outside the client, or every step would first
// waste a request on the exhausted model again.

static EXHAUSTED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>> =
    std::sync::OnceLock::new();

fn exhausted_map() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, std::time::Instant>> {
    EXHAUSTED
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn is_exhausted(model: &str) -> bool {
    exhausted_map().get(model).map(|until| std::time::Instant::now() < *until).unwrap_or(false)
}

fn mark_exhausted(model: &str, for_secs: f64) {
    let secs = for_secs.clamp(60.0, 24.0 * 3600.0);
    exhausted_map().insert(model.to_string(), std::time::Instant::now() + std::time::Duration::from_secs_f64(secs));
}

/// A 429 that won't clear in a minute: Groq says "... on tokens per day (TPD)"
/// or "requests per day (RPD)", or asks us to wait longer than we auto-wait.
pub fn is_daily_limit(message: &str, wait_secs: Option<f64>) -> bool {
    let m = message.to_lowercase();
    m.contains("per day") || m.contains("(tpd)") || m.contains("(rpd)") || wait_secs.map(|w| w > MAX_AUTO_WAIT_SECS).unwrap_or(false)
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

        let mut model = self.current_model();
        let mut attempt = 0;
        let raw: Value = loop {
            attempt += 1;

            let body = serde_json::json!({
                "model": model,
                "messages": messages,
                "tools": tool_defs,
                "tool_choice": "auto",
            });
            let mut req = client.post(&url).json(&body);
            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    // Network/connection-level failure (no HTTP
                    // response at all) — different from a 4xx/5xx
                    // response, and worth retrying a couple of times
                    // in case it was transient, same as a 429.
                    if attempt < MAX_ATTEMPTS {
                        eprintln!(
                            "Model API connection failed ({}), retrying in 2s (attempt {}/{})",
                            describe_reqwest_error(&e), attempt, MAX_ATTEMPTS
                        );
                        self.record_retry();
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                    return Err(format!("Model API request failed: {}", describe_reqwest_error(&e)));
                }
            };

            let status = resp.status();
            let retry_after_header = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<f64>().ok());

            let raw: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    return Err(format!(
                        "Model API returned unparseable response: {}",
                        describe_reqwest_error(&e)
                    ))
                }
            };

            if status.is_success() {
                self.record_success(&raw);
                break raw;
            }

            let code = raw.pointer("/error/code").and_then(|c| c.as_str()).unwrap_or("");
            let message = raw.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("");

            // Two failure kinds are worth retrying automatically:
            //  - 429: rate limit (free tiers have small tokens-per-minute caps)
            //  - 400 tool_use_failed: the model emitted a malformed/hallucinated
            //    tool call and Groq's validator rejected it. Non-deterministic,
            //    so trying the same turn again often works.
            // Daily quota gone on this model → switch to the next backup right away
            // (its quota is separate), instead of failing the whole run.
            if status.as_u16() == 429 {
                let hinted = retry_after_header.or_else(|| parse_retry_seconds(message));
                if is_daily_limit(message, hinted) {
                    mark_exhausted(&model, hinted.unwrap_or(3600.0));
                    let chain = self.chain();
                    if let Some(next) = chain.iter().find(|m| !is_exhausted(m)).cloned() {
                        eprintln!("Model {} hit its daily limit; switching to backup model {}", model, next);
                        crate::agent::events::emit(crate::agent::events::AgentEvent::ModelSwitched {
                            from: model.clone(),
                            to: next.clone(),
                            reason: "daily limit reached".to_string(),
                        });
                        model = next;
                        attempt = 0;
                        self.record_retry();
                        continue;
                    }
                    return Err(format!(
                        "Model API error ({}): daily limit reached on every configured model ({}). {}",
                        status,
                        chain.join(", "),
                        raw
                    ));
                }
            }

            let wait_secs: Option<f64> = if status.as_u16() == 429 {
                Some(
                    retry_after_header
                        .or_else(|| parse_retry_seconds(message))
                        .unwrap_or(10.0)
                        + 0.5,
                )
            } else if status.as_u16() == 400 && (code == "tool_use_failed" || code == "output_parse_failed") {
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
                    self.record_retry();
                    tokio::time::sleep(std::time::Duration::from_secs_f64(w)).await;
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
// ---------------------------------------------------------------
// Tests: `cargo test model_client`  (a tiny fake HTTP server, no internet)
// ---------------------------------------------------------------
#[cfg(test)]
mod fallback_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serves one canned response per connection, in order, and records which
    /// model each request asked for.
    async fn fake_api(responses: Vec<(u16, String)>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            for (code, body) in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    let n = sock.read(&mut tmp).await.unwrap();
                    buf.extend_from_slice(&tmp[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let len: usize = text[..h]
                            .lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                            .unwrap_or(0);
                        if buf.len() >= h + 4 + len {
                            let req: Value = serde_json::from_str(&text[h + 4..]).unwrap_or(Value::Null);
                            seen2.lock().unwrap().push(req["model"].as_str().unwrap_or("").to_string());
                            break;
                        }
                    }
                    if n == 0 { break; }
                }
                let reason = if code == 200 { "OK" } else { "Too Many Requests" };
                let resp = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    code, reason, body.len(), body
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{}", addr), seen)
    }

    fn ok_answer() -> String {
        serde_json::json!({ "choices": [{ "message": { "content": "hi from backup" } }], "usage": { "total_tokens": 5 } }).to_string()
    }

    fn daily_429(model: &str) -> String {
        serde_json::json!({ "error": { "code": "rate_limit_exceeded", "message": format!(
            "Rate limit reached for model `{}` in organization `org_x` service tier `on_demand` on tokens per day (TPD): Limit 200000, Used 199990, Requested 900. Please try again in 7m12.5s.", model) } }).to_string()
    }

    fn client(base: &str, main: &str, backups: &[&str]) -> OpenAiCompatibleClient {
        let mut c = OpenAiCompatibleClient::groq("test-key".into(), main.to_string())
            .with_fallbacks(backups.iter().map(|s| s.to_string()).collect());
        c.base_url = base.to_string();
        c
    }

    #[test]
    fn recognises_daily_limits() {
        assert!(is_daily_limit("... on tokens per day (TPD): Limit 200000 ...", None));
        assert!(is_daily_limit("... on requests per day (RPD) ...", Some(3.0)));
        assert!(is_daily_limit("... tokens per minute (TPM) ...", Some(600.0)), "a long wait counts too");
        assert!(!is_daily_limit("... on tokens per minute (TPM): Limit 8000 ... try again in 4.5s", Some(4.5)));
    }

    #[tokio::test]
    async fn switches_to_backup_on_daily_limit_and_remembers() {
        let (main, b1) = ("test-main-a", "test-backup-a");
        let (base, seen) = fake_api(vec![(429, daily_429(main)), (200, ok_answer()), (200, ok_answer())]).await;
        let c = client(&base, main, &[b1]);

        match c.next_turn(&[], &[]).await.unwrap() {
            ModelTurn::FinalAnswer(t) => assert_eq!(t, "hi from backup"),
            _ => panic!("expected an answer"),
        }
        assert_eq!(c.current_model(), b1);

        // a NEW client (like the next run_agent_tool step) goes straight to the backup
        let c2 = client(&base, main, &[b1]);
        c2.next_turn(&[], &[]).await.unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![main.to_string(), b1.to_string(), b1.to_string()], "no wasted request on the exhausted model");
    }

    #[tokio::test]
    async fn no_backups_means_a_clear_error() {
        let main = "test-main-b";
        let (base, _) = fake_api(vec![(429, daily_429(main))]).await;
        let c = client(&base, main, &[]);
        let err = c.next_turn(&[], &[]).await.err().expect("should fail");
        assert!(err.contains("daily limit reached on every configured model") && err.contains(main), "{}", err);
    }

    #[test]
    fn fallbacks_skip_blanks_and_the_main_model() {
        let c = OpenAiCompatibleClient::groq("k".into(), "m1").with_fallbacks(vec!["".into(), "m1".into(), "m2".into()]);
        assert_eq!(c.chain(), vec!["m1".to_string(), "m2".to_string()]);
    }
}