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
// Required plan for multi-item tasks (finding 32)
// ---------------------------------------------------------------
// With an OPTIONAL checklist, the model skipped the plan exactly on the long
// task where work goes missing (A3: plan made 0/5, one FAIL "missing 1/12").
// So, when the checklist is on:
//  1. start: if the goal names many items ("every", "each", "all", "do not skip"),
//     the backend asks for a plan before any other tool call (one message);
//  2. end: if the agent answers WITHOUT a plan after a multi-item goal, or after
//     touching 3+ different files, it is sent back once to list the items and
//     finish any it missed; if it still makes no plan, the user is told.
// Limits: keyword matching (can miss goals like "rename the photos"; the 3-file
// rule is the backstop) and ticks are still the agent's own report.

/// Start-of-run message asking for a plan first.
pub const PLAN_ASK: &str = "[Automatic plan request by AI CoWorker backend]";
/// End-of-run message: answered without a plan.
pub const PLAN_CHECK: &str = "[Automatic plan check by AI CoWorker backend]";
/// Different files touched in one run that count as "several items".
pub const MIN_PATHS: usize = 3;

fn is_backend_message(m: &ChatMessage, marker: &str) -> bool {
    m.role == "user" && m.content.as_deref().map(|c| c.starts_with(marker)).unwrap_or(false)
}

pub fn has_backend_message(messages: &[ChatMessage], marker: &str) -> bool {
    messages.iter().any(|m| is_backend_message(m, marker))
}

/// The user's goal: the first user message that the backend didn't write.
pub fn goal(messages: &[ChatMessage]) -> &str {
    messages
        .iter()
        .find(|m| m.role == "user" && !m.content.as_deref().unwrap_or("").starts_with("[Automatic"))
        .and_then(|m| m.content.as_deref())
        .unwrap_or("")
}

/// Does the goal itself say there are many items? Whole words only, so
/// "everything" / "allow" / "reach" don't count.
pub fn goal_has_many_items(goal: &str) -> bool {
    let g = goal.to_lowercase().replace('\u{2019}', "'");
    let words: Vec<&str> = g.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|w| !w.is_empty()).collect();
    words.iter().any(|w| matches!(*w, "every" | "each" | "all"))
        || g.contains("do not skip")
        || g.contains("don't skip")
        || g.contains("one per ")
}

/// How many different files this run's tool calls named (directory listings and
/// searches don't count; the checklist tool itself doesn't either).
pub fn paths_touched(messages: &[ChatMessage]) -> usize {
    const KEYS: [&str; 6] = ["relative_path", "from_relative_path", "to_relative_path", "output_path", "destination", "archive_path"];
    let mut seen = std::collections::HashSet::new();
    for m in messages {
        for c in m.tool_calls.iter().flatten() {
            let name = c.function.name.as_str();
            if matches!(name, "list_directory" | "search_files" | "request_tools") || name == TOOL {
                continue;
            }
            let Ok(args) = serde_json::from_str::<Value>(&c.function.arguments) else { continue };
            let mut add = |v: &Value| {
                if let Some(s) = v.as_str() {
                    let p = s.replace('\\', "/").trim_start_matches("./").trim_matches('/').to_lowercase();
                    if !p.is_empty() {
                        seen.insert(p);
                    }
                }
            };
            for k in KEYS {
                if let Some(v) = args.get(k) {
                    add(v);
                }
            }
            if let Some(list) = args.get("input_paths").and_then(|l| l.as_array()) {
                list.iter().for_each(&mut add);
            }
        }
    }
    seen.len()
}

fn any_work_done(messages: &[ChatMessage]) -> bool {
    messages.iter().flat_map(|m| m.tool_calls.iter().flatten()).any(|c| c.function.name != TOOL)
}

/// Start of a run: ask for a plan first? (multi-item goal, no plan yet, not asked yet)
pub fn should_ask_for_plan(messages: &[ChatMessage]) -> bool {
    !has_backend_message(messages, PLAN_ASK) && current(messages).is_none() && goal_has_many_items(goal(messages))
}

/// Final answer without a plan: send it back? Returns the number of files touched.
/// Not for runs that used no tools at all (plain questions), and only once per run.
pub fn needs_plan_check(messages: &[ChatMessage]) -> Option<usize> {
    if has_backend_message(messages, PLAN_CHECK) || current(messages).is_some() || !any_work_done(messages) {
        return None;
    }
    let n = paths_touched(messages);
    if goal_has_many_items(goal(messages)) || n >= MIN_PATHS {
        Some(n)
    } else {
        None
    }
}

pub fn plan_request() -> String {
    format!(
        "{} This task covers several items. Before any other tool call, call update_checklist with one item \
         per concrete thing to do (for example one item per file), then tick each item right after the tool \
         call that completes it.",
        PLAN_ASK
    )
}

pub fn plan_check_feedback(files: usize) -> String {
    format!(
        "{} This task covered several items ({} different files touched) but you made no checklist, so nothing \
         shows that every item was handled. Call update_checklist now with one item per thing the task asked for. \
         Mark an item done only if one of your tool calls in this run really completed it, finish any item that \
         is not done, then give your final answer again.",
        PLAN_CHECK, files
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

    
    fn user(text: &str) -> ChatMessage {
        ChatMessage { role: "user".into(), content: Some(text.into()), tool_calls: None, tool_call_id: None }
    }
    fn tool(name: &str, args: &str) -> ChatMessage {
        let mut m = call(args);
        m.tool_calls.as_mut().unwrap()[0].function.name = name.into();
        m
    }

    #[test]
    fn multi_item_goals_are_recognised() {
        // the benchmark tasks where incomplete work happened
        assert!(goal_has_many_items("Rename every .jpeg file in the photos folder to .jpg."));
        assert!(goal_has_many_items("Read every .txt file in the notes folder ... Do not skip any file."));
        assert!(goal_has_many_items("List ALL files in the data folder"));
        assert!(goal_has_many_items("Don’t skip a single row"));
        // ordinary single-item goals
        assert!(!goal_has_many_items("Create a file called notes.txt containing the word \"hello\"."));
        assert!(!goal_has_many_items("Delete everything in the parent folder"), "'everything' is not 'every'");
        assert!(!goal_has_many_items("Allow me to reach the report"));
    }

    #[test]
    fn counts_distinct_files_only() {
        let m = vec![
            tool("read_file", r#"{"relative_path":"notes/a.txt"}"#),
            tool("read_file", r#"{"relative_path":".\\notes\\a.txt"}"#), // same file
            tool("list_directory", r#"{"relative_path":"notes"}"#),       // folder, not an item
            tool("move_rename", r#"{"from_relative_path":"b.jpeg","to_relative_path":"b.jpg"}"#),
            tool(TOOL, r#"{"items":[{"text":"x","done":false}]}"#),
        ];
        assert_eq!(paths_touched(&m), 3); // notes/a.txt, b.jpeg, b.jpg
    }

    #[test]
    fn plan_request_and_check_rules() {
        let multi = vec![user("Rename every .jpeg file")];
        assert!(should_ask_for_plan(&multi));
        assert!(needs_plan_check(&multi).is_none(), "no tool calls = a plain question, not checked");
        let mut asked = multi.clone();
        asked.push(user(&plan_request()));
        assert!(!should_ask_for_plan(&asked), "asked once only");
        assert_eq!(goal(&asked), "Rename every .jpeg file", "backend messages are not the goal");

        // simple goal, but the run touched 3 files and made no plan
        let mut busy = vec![user("tidy up")];
        for f in ["a.txt", "b.txt", "c.txt"] {
            busy.push(tool("write_file", &format!(r#"{{"relative_path":"{}","content":"x"}}"#, f)));
        }
        assert!(!should_ask_for_plan(&busy));
        assert_eq!(needs_plan_check(&busy), Some(3));
        busy.push(user(&plan_check_feedback(3)));
        assert!(needs_plan_check(&busy).is_none(), "checked once only");

        // a plan exists -> the normal open-items gate takes over
        let planned = vec![user("Rename every .jpeg file"), tool(TOOL, r#"{"items":[{"text":"a","done":true}]}"#)];
        assert!(!should_ask_for_plan(&planned) && needs_plan_check(&planned).is_none());

        // simple goal, 1 file -> nothing
        let small = vec![user("make notes.txt"), tool("write_file", r#"{"relative_path":"notes.txt","content":"hi"}"#)];
        assert!(needs_plan_check(&small).is_none());
    }
}