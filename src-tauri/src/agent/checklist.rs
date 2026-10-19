//! Task checklist: the agent writes a to-do list at the start of a task and
//! ticks items off as it goes. The user sees it live, and the backend will not
//! let the run end quietly while items are still open.
//!
//! Why: the false completions the benchmark actually found were mostly
//! INCOMPLETE work, not missing files (finding 29): S2 renamed 3 of 4 files,
//! A3 wrote 10 of 12 codes, S7 forgot count.txt. Making the agent list the
//! items up front gives it (and the user) something concrete to check against.
//!
//! Honest limit: ticking an item is the agent's own report. The gate makes it
//! account for every item it listed, but it cannot prove an item was done —
//! that still needs the claim check, the live file tree and external verifiers.
//!
//! `update_checklist` is handled by the orchestrator (like `request_tools`): it
//! touches no files and is tier "safe". The list lives in the conversation (the
//! last `update_checklist` call), so pause/resume needs no extra state.
//!
//! OFF by default (the benchmark stays comparable); the app switches it on.
//! `AICOWORKER_CHECKLIST=on|off` overrides both.

use crate::agent::model_client::ChatMessage;
use crate::tools::registry::ToolSchema;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

pub const TOOL: &str = "update_checklist";
/// Marks the backend's "items still open" message, so the gate runs once per run.
pub const MARKER: &str = "[Automatic checklist check by AI CoWorker backend]";
const MAX_ITEMS: usize = 40;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    match std::env::var("AICOWORKER_CHECKLIST").map(|v| v.to_lowercase()) {
        Ok(v) if v == "on" || v == "1" || v == "true" => true,
        Ok(v) if v == "off" || v == "0" || v == "false" => false,
        _ => ENABLED.load(Ordering::Relaxed),
    }
}

pub fn schema() -> ToolSchema {
    ToolSchema {
        name: TOOL.to_string(),
        description: "Your to-do list for this task, shown live to the user. For any task with more than one \
            thing to do (several files, rows, codes or steps), call this FIRST with one item per concrete thing \
            (e.g. one item per file to rename). After the tool call that completes an item, call it again with \
            the full list and that item's done set to true. Send the whole list every time. Before your final \
            answer every item must be done; the backend checks this."
            .to_string(),
        tier: "safe".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string", "description": "One concrete thing to do" },
                            "done": { "type": "boolean" }
                        },
                        "required": ["text", "done"]
                    }
                }
            },
            "required": ["items"]
        }),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub text: String,
    pub done: bool,
}

fn parse(args: &Value) -> Result<Vec<Item>, String> {
    let arr = args.get("items").and_then(|i| i.as_array()).ok_or("'items' must be a list of {text, done}")?;
    if arr.is_empty() {
        return Err("The checklist needs at least one item.".to_string());
    }
    if arr.len() > MAX_ITEMS {
        return Err(format!("At most {} items, please group smaller steps.", MAX_ITEMS));
    }
    arr.iter()
        .map(|v| {
            let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
            if text.is_empty() {
                return Err("Every item needs a non-empty 'text'.".to_string());
            }
            let done = v.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
            Ok(Item { text, done })
        })
        .collect()
}

/// Result of an `update_checklist` call (handled by the orchestrator, not dispatch).
pub fn handle(args: &Value) -> Result<Value, String> {
    let items = parse(args)?;
    let done = items.iter().filter(|i| i.done).count();
    let open: Vec<&str> = items.iter().filter(|i| !i.done).map(|i| i.text.as_str()).collect();
    Ok(Value::String(if open.is_empty() {
        format!("Checklist saved: all {} items done.", items.len())
    } else {
        format!("Checklist saved: {}/{} done. Still open: {}", done, items.len(), open.join("; "))
    }))
}

/// The latest valid checklist in the conversation, if the agent made one.
pub fn current(messages: &[ChatMessage]) -> Option<Vec<Item>> {
    for m in messages.iter().rev() {
        if let Some(calls) = &m.tool_calls {
            for c in calls.iter().rev() {
                if c.function.name == TOOL {
                    if let Ok(args) = serde_json::from_str::<Value>(&c.function.arguments) {
                        if let Ok(items) = parse(&args) {
                            return Some(items);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Items still open (empty if there is no checklist or all are done).
pub fn open_items(messages: &[ChatMessage]) -> Vec<String> {
    current(messages)
        .map(|items| items.into_iter().filter(|i| !i.done).map(|i| i.text).collect())
        .unwrap_or_default()
}

pub fn feedback(open: &[String]) -> String {
    format!(
        "{} Your checklist still has open items: {}. Finish them now with your tools and tick them, \
         or say clearly in your answer which ones you did not do and why.",
        MARKER,
        open.join("; ")
    )
}

// ---------------------------------------------------------------
// Tests: `cargo test checklist`
// ---------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model_client::{ToolCallFunction, ToolCallRequest};

    fn call(args: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(vec![ToolCallRequest {
                id: "c".into(),
                kind: "function".into(),
                function: ToolCallFunction { name: TOOL.into(), arguments: args.into() },
            }]),
            tool_call_id: None,
        }
    }

    #[test]
    fn latest_list_wins() {
        let m = vec![
            call(r#"{"items":[{"text":"rename a","done":false},{"text":"rename b","done":false}]}"#),
            call(r#"{"items":[{"text":"rename a","done":true},{"text":"rename b","done":false}]}"#),
        ];
        assert_eq!(open_items(&m), vec!["rename b"]);
        assert!(open_items(&[]).is_empty(), "no checklist = nothing to check");
    }

    #[test]
    fn rejects_bad_lists() {
        assert!(handle(&json!({"items": []})).is_err());
        assert!(handle(&json!({"items": [{"text": "  ", "done": false}]})).is_err());
        assert!(handle(&json!({})).is_err());
        let ok = handle(&json!({"items": [{"text": "a", "done": true}, {"text": "b", "done": false}]})).unwrap();
        assert!(ok.as_str().unwrap().contains("1/2 done") && ok.as_str().unwrap().contains("b"));
        // a broken later call doesn't erase the last good list
        let m = vec![call(r#"{"items":[{"text":"x","done":false}]}"#), call("not json")];
        assert_eq!(open_items(&m), vec!["x"]);
        assert!(feedback(&["x".into()]).starts_with(MARKER));
    }
}