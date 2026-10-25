//! Tool routing: send the model only the tools a goal is likely to need.
//!
//! Every tool description is sent with EVERY model call (findings 11 and 20:
//! ~100 tokens per tool per call). With ~30 tools that is ~3k tokens per call,
//! which on an 8k-tokens-per-minute free tier means slow, rate-limited runs.
//!
//! How it works (stateless — recomputed from the conversation each turn):
//! - "core" file tools are always sent;
//! - other groups (office, web, archive, git, apps, connections) are added when
//!   the user's words suggest them, or when a tool from that group was already
//!   used in this conversation;
//! - the model can always load a missing group itself with `request_tools`.
//! Unknown tool names fall into core, so a new tool is never hidden by mistake.
//!
//! OFF by default (the benchmark stays comparable); the desktop app switches it
//! on at startup. `AICOWORKER_TOOL_ROUTING=on|off` overrides both.

use crate::agent::model_client::ChatMessage;
use crate::tools::registry::ToolSchema;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

pub const REQUEST_TOOLS: &str = "request_tools";

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    match std::env::var("AICOWORKER_TOOL_ROUTING").map(|v| v.to_lowercase()) {
        Ok(v) if v == "on" || v == "1" || v == "true" => true,
        Ok(v) if v == "off" || v == "0" || v == "false" => false,
        _ => ENABLED.load(Ordering::Relaxed),
    }
}

/// (group, short label for the model, trigger words)
const GROUPS: &[(&str, &str, &[&str])] = &[
        // Only words that clearly name a format. Finding 23: benchmark S1 says 'containing the
    // WORD "hello"' and the old list ("word", "table", "report", ...) loaded all office tools.
    ("office", "Word/Excel/PDF documents and spreadsheets", &[
        "docx", "xlsx", "xls", "csv", "pdf", "pdfs", "excel", "spreadsheet", "spreadsheets",
        "word document", "word file", "ms word", "microsoft word", "powerpoint",
    ]),
    ("web", "fetch public web pages", &["http", "www", "url", "website", "web", "webpage", "link", "online", "internet", "fetch", "download", ".com", ".org"]),
    ("archive", "zip archives", &["zip", "archive", "compress", "unzip", "extract", "tar", "7z"]),
    ("git", "git history, commits, snapshots and undo", &["git", "commit", "diff", "repo", "repository", "snapshot", "snapshots", "undo", "restore", "revert"]),
    ("apps", "open a file in its default app", &["open it", "open the", "open in", "launch", "show me", "preview"]),
        ("desktop", "this computer: system status (RAM, disk, battery), running apps and windows, close an app, clipboard, screenshot", &[
        "screen", "screenshot", "clipboard", "copied", "paste", "window", "windows", "running", "process", "processes",
        "close", "quit", "ram", "cpu", "disk space", "storage", "battery", "slow", "laptop", "computer", "my pc", "system info",
    ]),
    ("connections", "external tools from connected MCP servers", &["remember", "recall", "forget", "memory", "told you", "mcp", "tool"]),
];

/// Which group a built-in tool belongs to. Anything unknown is core.
pub fn group_of(tool: &str) -> &'static str {
    if tool.starts_with("mcp__") {
        return "connections";
    }
    match tool {
        "create_docx" | "create_xlsx" | "read_spreadsheet" | "convert_to_pdf" | "merge_pdfs" | "extract_pdf_pages" => "office",
        "fetch_url" => "web",
        "extract_archive" | "create_archive" => "archive",
        "git_diff" | "git_status" | "git_log" | "git_commit" | "list_snapshots" | "restore_snapshot" => "git",
        "open_in_default_app" => "apps",
        "system_info" | "list_processes" | "list_windows" | "close_app" | "clipboard_read" | "clipboard_write" | "take_screenshot" => "desktop",
        _ => "core",
    }
}

/// Words that say nothing about which MCP tool is meant.
const GENERIC: &[&str] = &[
    "get", "set", "list", "read", "write", "create", "delete", "update", "search", "open", "add", "remove",
    "nodes", "node", "graph", "item", "items", "data", "info", "tool", "tools", "file", "files", "with", "from",
];

/// Extra trigger words taken from the MCP tool names themselves, e.g.
/// mcp__bench__get_weather -> "weather", "bench".
fn mcp_words(tools: &[ToolSchema]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for t in tools.iter().filter(|t| t.name.starts_with("mcp__")) {
        for w in t.name.trim_start_matches("mcp__").split(|c: char| c == '_' || c == '-') {
            let w = w.to_lowercase();
            if w.len() >= 4 && !GENERIC.contains(&w.as_str()) {
                out.insert(w);
            }
        }
    }
    out
}

fn user_text(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .filter(|m| m.role == "user")
        .filter_map(|m| m.content.clone())
        // the backend's own check messages ("...finish them with your tools") are not the user's words
        .filter(|c| !c.starts_with("[Automatic"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

/// True if `word` appears in `text` as a word (or, for words with a dot, a
/// space, a hyphen or other punctuation like "to-do", as a substring).
fn mentions(text: &str, word: &str) -> bool {
    if word.chars().any(|c| !c.is_alphanumeric()) {
        return text.contains(word);
    }
    text.split(|c: char| !c.is_alphanumeric())
        .any(|tok| tok == word || (word.len() >= 4 && tok.starts_with(word)))
}

/// Groups to send this turn.
pub fn active_groups(all: &[ToolSchema], messages: &[ChatMessage]) -> BTreeSet<&'static str> {
    active_groups_with(all, messages, &[])
}

/// Same, plus the user's own routing keywords for MCP servers (mcp.json "keywords").
pub fn active_groups_with(all: &[ToolSchema], messages: &[ChatMessage], keywords: &[String]) -> BTreeSet<&'static str> {
    let mut on: BTreeSet<&'static str> = BTreeSet::new();
    on.insert("core");
    let text = user_text(messages);
    for (g, _, words) in GROUPS {
        if words.iter().any(|w| mentions(&text, w)) {
            on.insert(g);
        }
    }
    let has_mcp = all.iter().any(|t| t.name.starts_with("mcp__"));
    if mcp_words(all).iter().any(|w| mentions(&text, w)) || (has_mcp && keywords.iter().any(|w| mentions(&text, w))) {
        on.insert("connections");
    }
    // groups already used, or explicitly requested, in this conversation
    for m in messages.iter().filter(|m| m.role == "assistant") {
        for c in m.tool_calls.iter().flatten() {
            if c.function.name == REQUEST_TOOLS {
                let asked: Value = serde_json::from_str(&c.function.arguments).unwrap_or(Value::Null);
                if let Some(g) = asked.get("group").and_then(|g| g.as_str()) {
                    if let Some((name, _, _)) = GROUPS.iter().find(|(n, _, _)| *n == g) {
                        on.insert(name);
                    }
                }
            } else {
                on.insert(group_of(&c.function.name));
            }
        }
    }
    on
}

/// The `request_tools` meta-tool, listing only groups that exist and are not loaded yet.
fn request_tools_schema(all: &[ToolSchema], on: &BTreeSet<&'static str>) -> Option<ToolSchema> {
    let missing: Vec<(&str, &str, usize)> = GROUPS
        .iter()
        .map(|(g, label, _)| (*g, *label, all.iter().filter(|t| group_of(&t.name) == *g).count()))
        .filter(|(g, _, n)| *n > 0 && !on.contains(g))
        .collect();
    if missing.is_empty() {
        return None;
    }
    let list = missing.iter().map(|(g, l, _)| format!("{} ({})", g, l)).collect::<Vec<_>>().join("; ");
    Some(ToolSchema {
        name: REQUEST_TOOLS.to_string(),
        description: format!(
            "Load more tools for this task. Only some tools are loaded to save tokens. If you need a tool that is not in your list, call this first with the group name. Groups not loaded yet: {}.",
            list
        ),
        tier: "safe".to_string(),
        parameters: json!({
            "type": "object",
            "properties": { "group": { "type": "string", "enum": missing.iter().map(|(g, _, _)| *g).collect::<Vec<_>>() } },
            "required": ["group"]
        }),
    })
}

/// ToolSchema isn't Clone (registry.rs stays untouched), so copy by hand.
fn copy(t: &ToolSchema) -> ToolSchema {
    ToolSchema { name: t.name.clone(), description: t.description.clone(), tier: t.tier.clone(), parameters: t.parameters.clone() }
}

/// The tools to send this turn. With routing off, simply all of them.
pub fn select(all: &[ToolSchema], messages: &[ChatMessage]) -> Vec<ToolSchema> {
    if !enabled() {
        return all.iter().map(copy).collect();
    }
    let on = active_groups_with(all, messages, &crate::agent::mcp::route_keywords());
    let mut out: Vec<ToolSchema> = all.iter().filter(|t| on.contains(group_of(&t.name))).map(copy).collect();
    if let Some(req) = request_tools_schema(all, &on) {
        out.push(req);
    }
    out
}

/// Result of a `request_tools` call (handled by the orchestrator, not dispatch).
pub fn handle_request(all: &[ToolSchema], args: &Value) -> Result<Value, String> {
    let g = args.get("group").and_then(|g| g.as_str()).unwrap_or("");
    if !GROUPS.iter().any(|(n, _, _)| *n == g) {
        return Err(format!("Unknown tool group '{}'. Groups: {}", g, GROUPS.iter().map(|(n, _, _)| *n).collect::<Vec<_>>().join(", ")));
    }
    let names: Vec<String> = all.iter().filter(|t| group_of(&t.name) == g).map(|t| t.name.clone()).collect();
    if names.is_empty() {
        return Err(format!("No tools are available in group '{}'.", g));
    }
    Ok(Value::String(format!("Loaded the '{}' tools: {}. You can call them from your next step.", g, names.join(", "))))
}

// ---------------------------------------------------------------
// Tests: `cargo test routing`
// ---------------------------------------------------------------
/// Tests that flip the global on/off switch hold this, so they don't race.
#[cfg(test)]
pub static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model_client::{ToolCallFunction, ToolCallRequest};

    fn t(name: &str) -> ToolSchema {
        ToolSchema { name: name.into(), description: format!("{} does things", name), tier: "safe".into(), parameters: json!({"type": "object"}) }
    }
    fn all() -> Vec<ToolSchema> {
        ["read_file", "write_file", "list_directory", "delete_file", "execute_command", "create_docx", "create_xlsx",
         "fetch_url", "extract_archive", "git_commit", "restore_snapshot", "open_in_default_app", "brand_new_tool",
         "mcp__memory__create_entities", "mcp__memory__search_nodes", "mcp__bench__get_weather"]
            .iter().map(|n| t(n)).collect()
    }
    fn user(text: &str) -> ChatMessage {
        ChatMessage { role: "user".into(), content: Some(text.into()), tool_calls: None, tool_call_id: None }
    }
    fn called(name: &str, args: &str) -> ChatMessage {
        ChatMessage { role: "assistant".into(), content: None, tool_call_id: None, tool_calls: Some(vec![ToolCallRequest {
            id: "c".into(), kind: "function".into(), function: ToolCallFunction { name: name.into(), arguments: args.into() } }]) }
    }
    fn names(v: &[ToolSchema]) -> Vec<String> { v.iter().map(|t| t.name.clone()).collect() }

    #[test]
    fn grouping() {
        assert_eq!(group_of("create_xlsx"), "office");
        assert_eq!(group_of("mcp__x__y"), "connections");
        assert_eq!(group_of("brand_new_tool"), "core", "unknown tools are never hidden");
    }

    #[test]
    fn simple_goal_gets_core_plus_request_tools() {
        let on = active_groups(&all(), &[user("Write a file poem.md with a 6-line poem about rain")]);
        assert_eq!(on.into_iter().collect::<Vec<_>>(), vec!["core"]);
    }

    #[test]
    fn words_pick_groups() {
        let on = active_groups(&all(), &[user("Read data/sales.csv and build an Excel spreadsheet, then commit it to git")]);
        assert!(on.contains("office") && on.contains("git") && !on.contains("web"));
        let on = active_groups(&all(), &[user("Fetch https://example.com and summarise it")]);
        assert!(on.contains("web"));
        let on = active_groups(&all(), &[user("Use the weather tool to get the weather for Lahore")]);
        assert!(on.contains("connections"), "MCP words come from the tool names");
        let on = active_groups(&all(), &[user("Remember that my deadline is 20 December")]);
        assert!(on.contains("connections"));
        assert!(!active_groups(&all(), &[user("organise my photos")]).contains("git"), "'log' inside a word doesn't count");
    }

    
    #[test]
    fn everyday_words_dont_load_groups() {
        // benchmark S1: "word" here is not Microsoft Word (finding 23)
        let on = active_groups(&all(), &[user("Create a file called notes.txt containing the word \"hello\".")]);
        assert_eq!(on.into_iter().collect::<Vec<_>>(), vec!["core"]);
        let on = active_groups(&all(), &[user("Make a table of contents in README.md and check the log file version")]);
        assert_eq!(on.into_iter().collect::<Vec<_>>(), vec!["core"]);
        assert!(active_groups(&all(), &[user("Write a Word document about rain")]).contains("office"));
        assert!(active_groups(&all(), &[user("merge a.pdf and b.pdf")]).contains("office"));
    }

    #[test]
    fn backend_messages_dont_load_groups() {
        // the checklist / claim-check messages say "tool" and "file"; only the user's words count
        let msgs = vec![user("Rename every .jpeg file in photos"), user(&crate::agent::checklist::plan_request())];
        assert_eq!(active_groups(&all(), &msgs).into_iter().collect::<Vec<_>>(), vec!["core"]);
    }

    #[test]
    fn user_keywords_load_mcp_tools() {
        // finding 36: "to-do" is in no tool name, so list_tasks was never loaded
        let tools: Vec<ToolSchema> = all().into_iter().chain([t("mcp__notes__list_tasks"), t("mcp__notes__add_task")]).collect();
        let ask = [user("What's on my to-do list?")];
        assert!(!active_groups(&tools, &ask).contains("connections"), "tool names alone miss it");
        let kw = vec!["to-do".to_string(), "todo".to_string()];
        assert!(active_groups_with(&tools, &ask, &kw).contains("connections"), "hyphenated keyword matches");
        assert!(active_groups_with(&tools, &[user("add a todo: buy milk")], &kw).contains("connections"));
        assert!(!active_groups_with(&tools, &[user("Write a poem to poem.md")], &kw).contains("connections"));
        let no_mcp: Vec<ToolSchema> = tools.into_iter().filter(|t| !t.name.starts_with("mcp__")).collect();
        assert!(!active_groups_with(&no_mcp, &ask, &kw).contains("connections"), "no MCP tools = keywords do nothing");
    }

    #[test]
    fn history_and_requests_keep_groups_loaded() {
        let msgs = vec![user("tidy up the folder"), called(REQUEST_TOOLS, r#"{"group":"archive"}"#), called("git_commit", "{}")];
        let on = active_groups(&all(), &msgs);
        assert!(on.contains("archive") && on.contains("git"));
    }

    #[test]
    fn select_respects_the_switch() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_enabled(false);
        std::env::remove_var("AICOWORKER_TOOL_ROUTING");
        assert_eq!(select(&all(), &[user("poem")]).len(), all().len(), "off = everything");

        set_enabled(true);
        let s = select(&all(), &[user("Write a poem to poem.md")]);
        let n = names(&s);
        assert!(n.contains(&"write_file".to_string()) && n.contains(&"brand_new_tool".to_string()));
        assert!(!n.contains(&"create_docx".to_string()) && !n.iter().any(|x| x.starts_with("mcp__")));
        assert_eq!(n.last().unwrap(), REQUEST_TOOLS);
        let req = s.last().unwrap();
        assert!(req.parameters["properties"]["group"]["enum"].as_array().unwrap().iter().any(|g| g == "office"));
        set_enabled(false);
    }

    #[test]
    fn request_tools_answers() {
        let ok = handle_request(&all(), &json!({"group": "office"})).unwrap();
        assert!(ok.as_str().unwrap().contains("create_docx"));
        assert!(handle_request(&all(), &json!({"group": "nope"})).is_err());
    }

    #[test]
    fn size_report() {
        // how much smaller the tool list gets for a simple goal, with the REAL registry
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let real = crate::tools::registry::all_tool_schemas();
        let size = |v: &[ToolSchema]| v.iter().map(|t| t.name.len() + t.description.len() + t.parameters.to_string().len()).sum::<usize>();
        set_enabled(true);
        let small = select(&real, &[user("Write a file poem.md with a 6-line poem about rain")]);
        set_enabled(false);
        println!("ROUTING: {} tools ~{} tokens -> {} tools ~{} tokens", real.len(), size(&real) / 4, small.len(), size(&small) / 4);
        assert!(size(&small) < size(&real));
    }
}