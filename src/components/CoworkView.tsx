import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import PermissionDialog from "./PermissionDialog";
import StatusOrb, { type OrbState } from "./StatusOrb";

interface ToolCallFunction {
  name: string;
  arguments: string;
}

interface ToolCallRequest {
  id: string;
  type: string;
  function: ToolCallFunction;
}

interface ChatMessage {
  role: string;
  content?: string | null;
  tool_calls?: ToolCallRequest[] | null;
  tool_call_id?: string | null;
}

interface ToolSchema {
  name: string;
  description: string;
  tier: string;
  parameters: unknown;
}

interface PendingConfirmation {
  tool_name: string;
  arguments: unknown;
}

interface CoworkViewProps {
  workspaceRoot: string;
  setWorkspaceRoot: (v: string) => void;
  agentGoal: string;
  setAgentGoal: (v: string) => void;
  agentApiKey: string;
  setAgentApiKey: (v: string) => void;
  agentModel: string;
  setAgentModel: (v: string) => void;
  agentMessages: ChatMessage[];
  agentStatus: string;
  agentRunning: boolean;
  agentFinalAnswer: string | null;
  toolSchemas: ToolSchema[];
  pendingConfirmation: PendingConfirmation | null;
  onStart: () => void;
  onApprove: () => void;
  onDecline: () => void;
}

interface TraceItem {
  callId: string;
  toolName: string;
  argsPretty: string;
  tier: string;
  resultContent: string | null;
  isError: boolean;
  isPending: boolean;
}

function buildTraceItems(messages: ChatMessage[], tierByTool: Record<string, string>): TraceItem[] {
  const resultByCallId: Record<string, { content: string; isError: boolean }> = {};

  for (const m of messages) {
    if (m.role === "tool" && m.tool_call_id) {
      const content = m.content ?? "";
      resultByCallId[m.tool_call_id] = { content, isError: content.startsWith("Error:") };
    }
  }

  const items: TraceItem[] = [];
  for (const m of messages) {
    if (m.role === "assistant" && m.tool_calls) {
      for (const call of m.tool_calls) {
        let argsPretty = call.function.arguments;
        try {
          argsPretty = JSON.stringify(JSON.parse(call.function.arguments), null, 2);
        } catch {
          // leave as raw string if not parseable yet
        }
        const result = resultByCallId[call.id];
        items.push({
          callId: call.id,
          toolName: call.function.name,
          argsPretty,
          tier: tierByTool[call.function.name] ?? "mutating",
          resultContent: result ? result.content : null,
          isError: result ? result.isError : false,
          isPending: !result,
        });
      }
    }
  }
  return items;
}

function deriveOrbState(agentRunning: boolean, hasPending: boolean, status: string, hasFinal: boolean): OrbState {
  if (hasPending) return "waiting";
  if (agentRunning) return "thinking";
  if (status.startsWith("Error") || status.startsWith("Stopped")) return "error";
  if (hasFinal) return "done";
  return "idle";
}

export default function CoworkView({
  workspaceRoot,
  setWorkspaceRoot,
  agentGoal,
  setAgentGoal,
  agentApiKey,
  setAgentApiKey,
  agentModel,
  setAgentModel,
  agentMessages,
  agentStatus,
  agentRunning,
  agentFinalAnswer,
  toolSchemas,
  pendingConfirmation,
  onStart,
  onApprove,
  onDecline,
}: CoworkViewProps) {
  const [showAdvanced, setShowAdvanced] = useState(false);

  const tierByTool: Record<string, string> = {};
  for (const t of toolSchemas) tierByTool[t.name] = t.tier;

  const traceItems = buildTraceItems(agentMessages, tierByTool);
  const orbState = deriveOrbState(agentRunning, !!pendingConfirmation, agentStatus, !!agentFinalAnswer);
  const canRun = !agentRunning && !!agentGoal && !!workspaceRoot;

  async function handleBrowse() {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === "string") {
        setWorkspaceRoot(selected);
      }
    } catch (err) {
      console.error("Folder picker failed:", err);
    }
  }

  return (
    <div className="cowork-view">
      <div className="cowork-header">
        <h1 className="cowork-title">What should I do?</h1>
        <StatusOrb state={orbState} label={agentStatus || "idle"} />
      </div>

      <div className="workspace-bar">
        <label>Workspace</label>
        <input
          placeholder="D:\path\to\your\workspace"
          value={workspaceRoot}
          onChange={(e) => setWorkspaceRoot(e.target.value)}
        />
        <button className="browse-button" onClick={handleBrowse}>
          Browse…
        </button>
      </div>

      <div className="goal-card">
        <textarea
          className="goal-textarea"
          placeholder="Describe what you want done. AI CoWorker will show you every step — and ask before anything risky."
          value={agentGoal}
          onChange={(e) => setAgentGoal(e.target.value)}
        />
        <div className="goal-footer">
          <button className="run-button" disabled={!canRun} onClick={onStart}>
            {agentRunning ? "Running..." : "Run"}
          </button>
          <button className="advanced-toggle" onClick={() => setShowAdvanced((s) => !s)}>
            {showAdvanced ? "Hide advanced" : "Advanced"}
          </button>
        </div>
        {showAdvanced && (
          <div className="advanced-panel">
            <input
              placeholder="Groq API key (optional — falls back to .env)"
              type="password"
              value={agentApiKey}
              onChange={(e) => setAgentApiKey(e.target.value)}
            />
            <input
              placeholder="Model (e.g. openai/gpt-oss-120b)"
              value={agentModel}
              onChange={(e) => setAgentModel(e.target.value)}
            />
          </div>
        )}
      </div>

      {traceItems.length === 0 && !agentFinalAnswer && (
        <div className="empty-state">Nothing's run yet — type a goal above and hit Run.</div>
      )}

      {traceItems.length > 0 && (
        <div className="trace-list">
          {traceItems.map((item) => (
            <div className={`trace-item tier-${item.tier}`} key={item.callId}>
              <div className="trace-item-header">
                <span className={`trace-tier-badge tier-${item.tier}`}>{item.tier}</span>
                <span className="trace-tool-name">{item.toolName}</span>
                <span className="trace-args">({item.argsPretty.replace(/\s+/g, " ")})</span>
              </div>
              {item.resultContent !== null && (
                <div className={`trace-result ${item.isError ? "error" : ""}`}>{item.resultContent}</div>
              )}
              {item.isPending && <div className="trace-pending">Waiting for your approval...</div>}
            </div>
          ))}
        </div>
      )}

      {agentFinalAnswer && <div className="final-answer-card">{agentFinalAnswer}</div>}

      {pendingConfirmation && (
        <PermissionDialog
          toolName={pendingConfirmation.tool_name}
          args={pendingConfirmation.arguments}
          onApprove={onApprove}
          onDecline={onDecline}
        />
      )}
    </div>
  );
}