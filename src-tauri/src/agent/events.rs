//! Live agent events, so the UI can show the agent working in real time
//! (a step timeline, files lighting up, a live preview of what's being
//! written) instead of only the final answer.
//!
//! The orchestrator calls `emit(...)` at each step. In the desktop app the
//! sink forwards every event to the frontend as a Tauri event named
//! "agent-event"; in the headless benchmark no sink is set, so emitting
//! costs nothing and changes nothing.

use serde::Serialize;
use serde_json::Value;
use std::sync::OnceLock;

const MAX_PREVIEW_CHARS: usize = 4000; // enough for an inbox list of 20 emails in the Live view
/// Big string arguments (e.g. write_file content) are clipped before they are
/// sent to the UI — enough for a live "typing" preview, never megabytes.
const MAX_ARG_STRING_CHARS: usize = 20_000;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The model is deciding what to do next (step 1, 2, ...).
    Thinking { step: usize },
    /// A tool is about to run. `tier` is the REAL tier the backend will enforce.
    ToolStarted { call_id: String, name: String, arguments: Value, tier: String },
    /// The tool finished (or failed). `preview` is the start of its result.
    ToolFinished { call_id: String, name: String, ok: bool, preview: String, duration_ms: u64 },
    /// Paused: this action needs the user's approval.
    NeedsConfirmation { name: String, arguments: Value },
    /// The agent's final answer.
    Answer { text: String },
    /// The run ended without an answer (safety stop or error).
    Stopped { reason: String },
        /// The main model ran out of daily quota; the run continues on a backup model.
    ModelSwitched { from: String, to: String, reason: String },
}

type Sink = Box<dyn Fn(&AgentEvent) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();

/// Install the event receiver once (the app does this at startup).
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

pub fn emit(event: AgentEvent) {
    if let Some(sink) = SINK.get() {
        sink(&event);
    }
}

pub fn preview(text: &str) -> String {
    let mut p: String = text.chars().take(MAX_PREVIEW_CHARS).collect();
    if text.chars().count() > MAX_PREVIEW_CHARS {
        p.push('…');
    }
    p
}

/// Copy of the arguments with every long string clipped.
pub fn clip_args(v: &Value) -> Value {
    match v {
        Value::String(s) if s.chars().count() > MAX_ARG_STRING_CHARS => {
            let mut c: String = s.chars().take(MAX_ARG_STRING_CHARS).collect();
            c.push_str("…[clipped]");
            Value::String(c)
        }
        Value::Array(a) => Value::Array(a.iter().map(clip_args).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), clip_args(x))).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_with_a_kind_tag() {
        let e = AgentEvent::ToolStarted { call_id: "c1".into(), name: "write_file".into(), arguments: json!({"relative_path": "a.txt"}), tier: "mutating".into() };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "tool_started");
        assert_eq!(v["tier"], "mutating");
        assert_eq!(serde_json::to_value(AgentEvent::Thinking { step: 2 }).unwrap(), json!({"kind": "thinking", "step": 2}));
    }

    #[test]
    fn clips_big_arguments_only() {
        let big = "x".repeat(MAX_ARG_STRING_CHARS + 50);
        let c = clip_args(&json!({ "relative_path": "a.txt", "content": big, "rows": [[1, "ok"]] }));
        assert!(c["content"].as_str().unwrap().ends_with("…[clipped]"));
        assert_eq!(c["relative_path"], "a.txt");
        assert_eq!(c["rows"], json!([[1, "ok"]]));
        assert!(preview(&"y".repeat(5000)).chars().count() <= MAX_PREVIEW_CHARS + 1);
    }
}