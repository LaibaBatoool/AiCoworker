use crate::agent::model_client::{ChatMessage, ModelClient, ModelTurn, ToolCallRequest};
use crate::tools::registry::all_tool_schemas;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

const MAX_STEPS: usize = 40;
const MAX_IDENTICAL_REPEATS: usize = 3;

/// A tool is considered "stuck" once it has failed this many times
/// across the WHOLE run — not required to be back-to-back, since a
/// model can interleave legitimate successful calls to OTHER tools
/// between failed retries of the same one.
const MAX_TOOL_FAILURES: usize = 4;

/// Cap on how much of a single tool result gets fed back into the
/// conversation.
const MAX_TOOL_RESULT_CHARS: usize = 4000;

/// Cap on the TOTAL size of conversation history sent to the model
/// per turn (separate from MAX_TOOL_RESULT_CHARS, which caps one
/// result at a time). Found live: a long "list and read everything"
/// run accumulated enough turns that the WHOLE request exceeded
/// Groq's free-tier 8,000-token/minute limit even though no single
/// result was oversized — 413 Request Too Large. This is a blunt
/// stopgap (drop oldest complete turns first), not real summarizing
/// context pruning — that's explicitly Semester 8 scope.
const MAX_HISTORY_CHARS: usize = 8000;

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
    /// Hit the iteration cap, the same tool+args got called
    /// MAX_IDENTICAL_REPEATS times in a row, or the same tool
    /// accumulated MAX_TOOL_FAILURES failures (consecutive or not)
    /// across the run — stopped itself rather than spinning or
    /// burning API credits silently.
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
    let mut tool_failure_counts: HashMap<String, usize> = HashMap::new();

    if resume_confirmed {
        if let Some(assistant_idx) = messages
            .iter()
            .rposition(|m| m.role == "assistant" && m.tool_calls.is_some())
        {
            let tool_calls = messages[assistant_idx]
                .tool_calls
                .clone()
                .unwrap_or_default();
            let resolved = resolved_call_ids_since(&messages, assistant_idx);
            let pending: Vec<ToolCallRequest> = tool_calls
                .into_iter()
                .filter(|tc| !resolved.contains(&tc.id))
                .collect();

            messages = match process_pending_tool_calls(
                workspace_root,
                messages,
                &pending,
                &mut recent_calls,
                &mut tool_failure_counts,
                true, // the first pending call is the one the user just approved
            ) {
                Ok(m) => m,
                Err(outcome) => return outcome,
            };
        }
    }

    for _ in 0..MAX_STEPS {
        let trimmed_for_model = trim_history_for_request(&messages);

        let turn = match client.next_turn(&trimmed_for_model, &tools).await {
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

                messages = match process_pending_tool_calls(
                    workspace_root,
                    messages,
                    &tool_calls,
                    &mut recent_calls,
                    &mut tool_failure_counts,
                    false,
                ) {
                    Ok(m) => m,
                    Err(outcome) => return outcome,
                };
            }
        }
    }

    LoopOutcome::StoppedForSafety {
        messages,
        reason: format!("Hit the {}-step iteration cap without a final answer.", MAX_STEPS),
    }
}

/// Finds every tool_call_id that already has a matching "tool" role
/// result message AFTER the assistant message at `assistant_idx` —
/// i.e. calls from that batch that were already executed before a
/// pause happened partway through it.
fn resolved_call_ids_since(messages: &[ChatMessage], assistant_idx: usize) -> HashSet<String> {
    messages[assistant_idx + 1..]
        .iter()
        .filter(|m| m.role == "tool")
        .filter_map(|m| m.tool_call_id.clone())
        .collect()
}

fn message_char_len(m: &ChatMessage) -> usize {
    let mut len = m.content.as_deref().map(|c| c.len()).unwrap_or(0);
    if let Some(calls) = &m.tool_calls {
        for c in calls {
            len += c.function.name.len() + c.function.arguments.len();
        }
    }
    len
}

/// Groups messages after the fixed header into complete "turns" —
/// one assistant message (with its tool_calls, if any) plus every
/// "tool" role message that responds to it. Trimming only ever drops
/// WHOLE turns, never an assistant message without its matching tool
/// results (or vice versa) — dropping a partial turn would leave an
/// orphaned tool_call_id, which Groq's API rejects outright.
fn group_into_turns(messages: &[ChatMessage]) -> Vec<Vec<ChatMessage>> {
    let mut turns: Vec<Vec<ChatMessage>> = Vec::new();
    for m in messages {
        if m.role == "assistant" {
            turns.push(vec![m.clone()]);
        } else if let Some(last) = turns.last_mut() {
            last.push(m.clone());
        }
    }
    turns
}

/// Trims the conversation history sent to the model so a long run
/// doesn't eventually exceed Groq's free-tier request-size limit.
/// Always keeps the fixed header (system + the original user goal)
/// and the MOST RECENT complete turns that fit under
/// MAX_HISTORY_CHARS — older turns are dropped as whole units,
/// oldest first. The full, untrimmed `messages` the caller holds is
/// untouched; this only affects what gets sent to the model THIS
/// turn.
fn trim_history_for_request(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let header_len = messages
        .iter()
        .take_while(|m| m.role == "system" || m.role == "user")
        .count();
    let header = &messages[..header_len];
    let turns = group_into_turns(&messages[header_len..]);

    let mut kept: Vec<Vec<ChatMessage>> = Vec::new();
    let mut total: usize = header.iter().map(message_char_len).sum();
    let mut dropped_count = 0;

    for turn in turns.into_iter().rev() {
        let turn_len: usize = turn.iter().map(message_char_len).sum();
        if total + turn_len > MAX_HISTORY_CHARS && !kept.is_empty() {
            dropped_count += 1;
            continue;
        }
        total += turn_len;
        kept.push(turn);
    }
    kept.reverse();

    let mut result: Vec<ChatMessage> = header.to_vec();
    if dropped_count > 0 {
        result.push(ChatMessage {
            role: "system".to_string(),
            content: Some(format!(
                "[{} earlier tool call(s)/result(s) were omitted from this context to stay within the model's request size limit. If you still need information from an earlier step, call the relevant tool again rather than assuming.]",
                dropped_count
            )),
            tool_calls: None,
            tool_call_id: None,
        });
    }
    for turn in kept {
        result.extend(turn);
    }
    result
}

/// Decides whether a tool's result counts as a "failure" for the
/// stuck-tool detector. A plain Err from dispatch always counts.
/// Additionally, since execute_command_tool returns Ok(CommandOutput)
/// even when the command itself failed or hung, this also checks
/// INSIDE a successful Ok payload for those same signals.
fn tool_result_counts_as_failure(result: &Result<Value, String>) -> bool {
    match result {
        Err(_) => true,
        Ok(value) => {
            let timed_out = value
                .get("timed_out")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let nonzero_exit = value
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .map(|code| code != 0)
                .unwrap_or(false);
            timed_out || nonzero_exit
        }
    }
}

/// Executes a batch of pending tool calls in order, appending a
/// "tool" role result message for each one to `messages`. If
/// `first_is_confirmed` is true, ONLY the first call in `pending` runs
/// with confirmed=true (used when resuming right after user
/// approval) — every other call is always attempted with
/// confirmed=false first.
fn process_pending_tool_calls(
    workspace_root: &str,
    mut messages: Vec<ChatMessage>,
    pending: &[ToolCallRequest],
    recent_calls: &mut VecDeque<(String, String)>,
    tool_failure_counts: &mut HashMap<String, usize>,
    first_is_confirmed: bool,
) -> Result<Vec<ChatMessage>, LoopOutcome> {
    for (i, call) in pending.iter().enumerate() {
        let confirmed_this_call = first_is_confirmed && i == 0;

        let args: Value = match serde_json::from_str(&call.function.arguments) {
            Ok(v) => v,
            Err(e) => {
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
            return Err(LoopOutcome::StoppedForSafety {
                messages,
                reason: format!(
                    "'{}' was called with identical arguments {} times in a row and appears stuck.",
                    call.function.name, MAX_IDENTICAL_REPEATS
                ),
            });
        }
        recent_calls.push_back(fingerprint);
        if recent_calls.len() > MAX_IDENTICAL_REPEATS {
            recent_calls.pop_front();
        }

        let result = crate::agent::dispatch::dispatch_tool_call(
            workspace_root,
            &call.function.name,
            &args,
            confirmed_this_call,
        );

        if let Err(e) = &result {
            if e.starts_with("PRIVILEGED_CONFIRMATION_REQUIRED") {
                return Err(LoopOutcome::AwaitingConfirmation {
                    messages,
                    tool_name: call.function.name.clone(),
                    arguments: args,
                });
            }
        }

        let is_failure = tool_result_counts_as_failure(&result);
        messages.push(tool_result_message(call, &result));

        let count = tool_failure_counts.entry(call.function.name.clone()).or_insert(0);
        if is_failure {
            *count += 1;
            if *count >= MAX_TOOL_FAILURES {
                return Err(LoopOutcome::StoppedForSafety {
                    messages,
                    reason: format!(
                        "'{}' failed {} times across this run (even with different arguments/approaches) and appears unable to succeed — likely missing a required program, or attempting an unsupported approach.",
                        call.function.name, MAX_TOOL_FAILURES
                    ),
                });
            }
        } else {
            *count = 0;
        }
    }

    Ok(messages)
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