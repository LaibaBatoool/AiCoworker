use crate::agent::model_client::{ChatMessage, ModelClient, ModelTurn, ToolCallRequest};
use crate::tools::registry::all_tool_schemas;
use serde_json::Value;
use std::collections::VecDeque;

const MAX_STEPS: usize = 50;
const MAX_IDENTICAL_REPEATS: usize = 3;

/// Cap on how much of a single tool result gets fed back into the
/// conversation. Groq's free tier caps at 8,000 tokens/minute total
/// across the whole request (messages + tools + response) — at
/// roughly 4 chars/token, 4000 chars is ~1000 tokens per result,
/// leaving headroom for everything else in the same request. Without
/// this, one read_file on a large-ish file (or a list_directory over
/// a big tree) can blow the entire budget in a single turn, which
/// isn't retryable the way a plain rate limit is — the request is
/// just too big, no amount of waiting fixes it.
const MAX_TOOL_RESULT_CHARS: usize = 4000;

/// What one call to run_agent_loop produced. The caller (eventually
/// the frontend, for now devtools) owns `messages` between calls —
/// the loop itself keeps no state of its own.
pub enum LoopOutcome {
    Done { messages: Vec<ChatMessage>, answer: String },
    /// The agent wants a tool that needs explicit confirmation (a
    /// privileged action, or an execute_command whose classified
    /// risk turned out privileged). The loop does NOT retry this on
    /// its own — re-invoke with resume_confirmed=true, same messages,
    /// only after the user has actually approved it.
    AwaitingConfirmation {
        messages: Vec<ChatMessage>,
        tool_name: String,
        arguments: Value,
    },
    /// Hit the iteration cap, or the same tool+args got called
    /// MAX_IDENTICAL_REPEATS times in a row — stopped itself rather
    /// than spinning or burning API credits silently.
    StoppedForSafety { messages: Vec<ChatMessage>, reason: String },
    Error(String),
}

/// One plan -> act -> observe pass. Seed `messages` with a system +
/// user goal message on the first call; keep re-invoking with
/// whatever `messages` the previous LoopOutcome carried until you get
/// back Done, StoppedForSafety, or Error.
pub async fn run_agent_loop(
    client: &dyn ModelClient,
    workspace_root: &str,
    mut messages: Vec<ChatMessage>,
    resume_confirmed: bool,
) -> LoopOutcome {
    let tools = all_tool_schemas();
    let mut recent_calls: VecDeque<(String, String)> = VecDeque::new();

    // Resuming after user approval: execute the pending call (from
    // the last assistant message's tool_calls) with confirmed=true
    // before anything else, then fall into the normal loop.
    if resume_confirmed {
        if let Some(last) = messages.last().cloned() {
            if let Some(call) = last.tool_calls.as_ref().and_then(|tc| tc.first()) {
                match serde_json::from_str::<Value>(&call.function.arguments) {
                    Ok(parsed_args) => {
                        let result = crate::agent::dispatch::dispatch_tool_call(
                            workspace_root,
                            &call.function.name,
                            &parsed_args,
                            true,
                        );
                        messages.push(tool_result_message(call, &result));
                    }
                    Err(e) => {
                        return LoopOutcome::Error(format!(
                            "Could not parse arguments for the resumed tool call '{}': {}",
                            call.function.name, e
                        ))
                    }
                }
            }
        }
    }

    for _ in 0..MAX_STEPS {
        let turn = match client.next_turn(&messages, &tools).await {
            Ok(t) => t,
            Err(e) => return LoopOutcome::Error(format!("Model call failed: {}", e)),
        };

        match turn {
            ModelTurn::FinalAnswer(answer) => {
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: Some(answer.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                });
                return LoopOutcome::Done { messages, answer };
            }
            ModelTurn::ToolCalls(tool_calls) => {
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(tool_calls.clone()),
                    tool_call_id: None,
                });

                // Only the first call this turn is executed; if it
                // needs confirmation, we stop here rather than
                // silently running whatever else the model queued.
                let call = &tool_calls[0];

                let args: Value = match serde_json::from_str(&call.function.arguments) {
                    Ok(v) => v,
                    Err(e) => {
                        // Feed parse failures back to the MODEL, not
                        // the user — it can often self-correct.
                        messages.push(ChatMessage {
                            role: "tool".to_string(),
                            content: Some(format!("Error: arguments were not valid JSON: {}", e)),
                            tool_calls: None,
                            tool_call_id: Some(call.id.clone()),
                        });
                        continue;
                    }
                };

                let fingerprint = (call.function.name.clone(), args.to_string());
                if recent_calls.len() >= MAX_IDENTICAL_REPEATS - 1
                    && recent_calls.iter().all(|c| *c == fingerprint)
                {
                    return LoopOutcome::StoppedForSafety {
                        messages,
                        reason: format!(
                            "'{}' was called with identical arguments {} times in a row and appears stuck.",
                            call.function.name, MAX_IDENTICAL_REPEATS
                        ),
                    };
                }
                recent_calls.push_back(fingerprint);
                if recent_calls.len() > MAX_IDENTICAL_REPEATS {
                    recent_calls.pop_front();
                }

                let result = crate::agent::dispatch::dispatch_tool_call(
                    workspace_root,
                    &call.function.name,
                    &args,
                    false, // never auto-confirm from inside the loop
                );

                if let Err(e) = &result {
                    if e.starts_with("PRIVILEGED_CONFIRMATION_REQUIRED") {
                        return LoopOutcome::AwaitingConfirmation {
                            messages,
                            tool_name: call.function.name.clone(),
                            arguments: args,
                        };
                    }
                }

                messages.push(tool_result_message(call, &result));
            }
        }
    }

    LoopOutcome::StoppedForSafety {
        messages,
        reason: format!("Hit the {}-step iteration cap without a final answer.", MAX_STEPS),
    }
}

fn tool_result_message(call: &ToolCallRequest, result: &Result<Value, String>) -> ChatMessage {
    let content = match result {
        Ok(v) => truncate_for_model(&v.to_string()),
        Err(e) => truncate_for_model(&format!("Error: {}", e)),
    };
    ChatMessage {
        role: "tool".to_string(),
        content: Some(content),
        tool_calls: None,
        tool_call_id: Some(call.id.clone()),
    }
}

/// Caps a tool result's text before it goes back to the model.
/// Truncates by character count (not bytes) so a multi-byte UTF-8
/// character never gets split mid-codepoint.
fn truncate_for_model(s: &str) -> String {
    let total_chars = s.chars().count();
    if total_chars <= MAX_TOOL_RESULT_CHARS {
        return s.to_string();
    }

    let truncated: String = s.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    format!(
        "{}\n\n[truncated — {} characters total, first {} shown. Ask for a narrower read (e.g. a specific file instead of a whole directory, or a smaller search) if you need the rest.]",
        truncated, total_chars, MAX_TOOL_RESULT_CHARS
    )
}