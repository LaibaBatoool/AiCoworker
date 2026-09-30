pub mod model_client;
pub mod dispatch;
pub mod orchestrator;

use serde::Serialize;
use serde_json::Value;

/// Wire-friendly version of orchestrator::LoopOutcome, returned to
/// whatever calls run_agent_tool. Tagged so the frontend (whenever
/// it exists) can switch on `status` directly.
#[derive(Serialize)]
#[serde(tag = "status")]
pub enum AgentStepResult {
    Done { messages: Vec<model_client::ChatMessage>, answer: String },
    AwaitingConfirmation {
        messages: Vec<model_client::ChatMessage>,
        tool_name: String,
        arguments: Value,
    },
    StoppedForSafety { messages: Vec<model_client::ChatMessage>, reason: String },
    Error { message: String },
}