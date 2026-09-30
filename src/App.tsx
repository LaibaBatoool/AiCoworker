import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface CommandOutput {
  risk_level: string;
  stdout: string;
  stderr: string;
  exit_code: number | null;
  timed_out: boolean;
}

interface SearchResult {
  relative_path: string;
  matched_on: string;
}

interface FileMetadata {
  relative_path: string;
  size_bytes: number;
  is_directory: boolean;
  modified_unix_timestamp: number | null;
  read_only: boolean;
}

interface GitDiffResult {
  diff: string;
  has_changes: boolean;
}

interface GitCommitResult {
  success: boolean;
  output: string;
}

interface AuditLogRecord {
  timestamp_unix: number;
  tier: string;
  action: string;
  success: boolean;
  detail: string;
}

interface SnapshotRecord {
  commit_hash: string;
  timestamp_unix: number;
  message: string;
}

interface ToolSchema {
  name: string;
  description: string;
  tier: string;
  parameters: unknown;
}

// --- Agent orchestration types (mirror src-tauri/src/agent/*) ---

interface ToolCallFunction {
  name: string;
  arguments: string; // JSON-encoded string
}

interface ToolCallRequest {
  id: string;
  type: string;
  function: ToolCallFunction;
}

interface ChatMessage {
  role: string; // "system" | "user" | "assistant" | "tool"
  content?: string | null;
  tool_calls?: ToolCallRequest[] | null;
  tool_call_id?: string | null;
}

type AgentStepResult =
  | { status: "Done"; messages: ChatMessage[]; answer: string }
  | { status: "AwaitingConfirmation"; messages: ChatMessage[]; tool_name: string; arguments: unknown }
  | { status: "StoppedForSafety"; messages: ChatMessage[]; reason: string }
  | { status: "Error"; message: string };

function App() {
  const [runningJobId, setRunningJobId] = useState<number | null>(null);

  const [dirPath, setDirPath] = useState("");
  const [dirEntries, setDirEntries] = useState<{ name: string; is_directory: boolean }[]>([]);

  const [workspaceRoot, setWorkspaceRoot] = useState("");
  const [filePath, setFilePath] = useState("");
  const [output, setOutput] = useState("");

  const [writeFilePath, setWriteFilePath] = useState("");
  const [writeContent, setWriteContent] = useState("");
  const [writeStatus, setWriteStatus] = useState("");

  const [convertFilePath, setConvertFilePath] = useState("");
  const [convertStatus, setConvertStatus] = useState("");

  const [editFilePath, setEditFilePath] = useState("");
  const [oldText, setOldText] = useState("");
  const [newText, setNewText] = useState("");
  const [editStatus, setEditStatus] = useState("");

  const [deleteFilePath, setDeleteFilePath] = useState("");
  const [deleteRecursive, setDeleteRecursive] = useState(false);
  const [deleteStatus, setDeleteStatus] = useState("");

  const [command, setCommand] = useState("");
  const [commandResult, setCommandResult] = useState<CommandOutput | null>(null);
  const [commandStatus, setCommandStatus] = useState("");

  const [newDirPath, setNewDirPath] = useState("");
  const [createDirStatus, setCreateDirStatus] = useState("");

  const [moveFrom, setMoveFrom] = useState("");
  const [moveTo, setMoveTo] = useState("");
  const [moveStatus, setMoveStatus] = useState("");

  const [searchNamePattern, setSearchNamePattern] = useState("");
  const [searchContentPattern, setSearchContentPattern] = useState("");
  const [searchResults, setSearchResults] = useState<SearchResult[]>([]);
  const [searchStatus, setSearchStatus] = useState("");

  const [metadataPath, setMetadataPath] = useState("");
  const [metadataResult, setMetadataResult] = useState<FileMetadata | null>(null);
  const [metadataStatus, setMetadataStatus] = useState("");

  const [gitDiffResult, setGitDiffResult] = useState<GitDiffResult | null>(null);
  const [gitDiffStatus, setGitDiffStatus] = useState("");

  const [commitMessage, setCommitMessage] = useState("");
  const [commitStatus, setCommitStatus] = useState("");

  const [auditLog, setAuditLog] = useState<AuditLogRecord[]>([]);
  const [auditLogStatus, setAuditLogStatus] = useState("");

  const [snapshots, setSnapshots] = useState<SnapshotRecord[]>([]);
  const [snapshotStatus, setSnapshotStatus] = useState("");

  const [toolSchemas, setToolSchemas] = useState<ToolSchema[]>([]);
  const [toolSchemasStatus, setToolSchemasStatus] = useState("");

  // --- Agent orchestration (ReAct loop) ---
  const [agentGoal, setAgentGoal] = useState("");
  const [agentApiKey, setAgentApiKey] = useState(""); // leave blank to use GROQ_API_KEY from src-tauri/.env
  const [agentModel, setAgentModel] = useState("openai/gpt-oss-120b");
  const [agentMessages, setAgentMessages] = useState<ChatMessage[]>([]);
  const [agentStatus, setAgentStatus] = useState("");
  const [agentRunning, setAgentRunning] = useState(false);
  const [agentFinalAnswer, setAgentFinalAnswer] = useState<string | null>(null);

  async function handleReadFile() {
    try {
      const result = await invoke<string>("read_file_tool", {
        workspaceRoot,
        relativePath: filePath,
      });
      setOutput(result);
    } catch (err) {
      setOutput(`Error: ${err}`);
    }
  }

  async function handleListDirectory() {
    try {
      const result = await invoke<{ name: string; is_directory: boolean }[]>(
        "list_directory_tool",
        { workspaceRoot, relativePath: dirPath }
      );
      setDirEntries(result);
    } catch (err) {
      setOutput(`Error: ${err}`);
    }
  }

  async function handleWriteFile() {
    const confirmedByUser = window.confirm(
      `The agent wants to WRITE to "${writeFilePath}".\n\nThis will create or overwrite this file. Approve?`
    );
    if (!confirmedByUser) {
      setWriteStatus("Write cancelled by user.");
      return;
    }

    try {
      await invoke("write_file_tool", {
        workspaceRoot,
        relativePath: writeFilePath,
        content: writeContent,
        confirmed: true,
      });
      setWriteStatus(`Successfully wrote to "${writeFilePath}".`);
    } catch (err) {
      setWriteStatus(`Error: ${err}`);
    }
  }

  async function handleConvertToPdf() {
    const confirmedByUser = window.confirm(
      `The agent wants to CONVERT "${convertFilePath}" to PDF.\n\nApprove?`
    );
    if (!confirmedByUser) {
      setConvertStatus("Conversion cancelled by user.");
      return;
    }

    try {
      const resultFilename = await invoke<string>("convert_to_pdf_tool", {
        workspaceRoot,
        relativePath: convertFilePath,
        confirmed: true,
      });
      setConvertStatus(`Successfully converted to "${resultFilename}".`);
    } catch (err) {
      setConvertStatus(`Error: ${err}`);
    }
  }

  async function handleEditFile() {
    const confirmedByUser = window.confirm(
      `The agent wants to EDIT "${editFilePath}".\n\nReplace:\n"${oldText}"\n\nWith:\n"${newText}"\n\nApprove?`
    );
    if (!confirmedByUser) {
      setEditStatus("Edit cancelled by user.");
      return;
    }

    try {
      await invoke("edit_file_tool", {
        workspaceRoot,
        relativePath: editFilePath,
        oldText,
        newText,
        confirmed: true,
      });
      setEditStatus(`Successfully edited "${editFilePath}".`);
    } catch (err) {
      setEditStatus(`Error: ${err}`);
    }
  }

  async function handleDeleteFile() {
    const typed = window.prompt(
      `PRIVILEGED ACTION — enforced by the backend, not just this dialog.\n\nThe agent wants to DELETE "${deleteFilePath}"${deleteRecursive ? " (and everything inside it)" : ""
      }.\n\nType the exact file/folder name to confirm:`
    );

    if (typed !== deleteFilePath) {
      setDeleteStatus("Delete cancelled — confirmation text did not match.");
      return;
    }

    try {
      await invoke("delete_file_tool", {
        workspaceRoot,
        relativePath: deleteFilePath,
        recursive: deleteRecursive,
        confirmed: true,
      });
      setDeleteStatus(`Successfully deleted "${deleteFilePath}".`);
    } catch (err) {
      setDeleteStatus(`Error: ${err}`);
    }
  }

  async function handleRunCommand(confirmed: boolean) {
    try {
      const jobId = await invoke<number>("start_command_tool", {
        workspaceRoot,
        command,
        confirmed,
      });
      setRunningJobId(jobId);
      setCommandResult(null);
      setCommandStatus("Running... you can cancel it below while it's in progress.");

      const unlisten = await listen<{ jobId: number; output: CommandOutput }>(
        "command-finished",
        (event) => {
          if (event.payload.jobId === jobId) {
            setCommandResult(event.payload.output);
            setCommandStatus("");
            setRunningJobId(null);
            unlisten();
          }
        }
      );
    } catch (err) {
      const errMsg = String(err);
      if (errMsg.includes("PRIVILEGED_CONFIRMATION_REQUIRED")) {
        const approve = window.confirm(
          `PRIVILEGED COMMAND (backend-enforced)\n\n"${command}"\n\nThis command was classified as privileged. Run it anyway?`
        );
        if (approve) {
          await handleRunCommand(true);
        } else {
          setCommandStatus("Command cancelled by user.");
        }
      } else {
        setCommandStatus(`Error: ${errMsg}`);
      }
    }
  }

  async function handleCancelCommand() {
    if (runningJobId === null) return;
    try {
      await invoke("cancel_command_tool", { jobId: runningJobId });
      setCommandStatus("Cancelling...");
    } catch (err) {
      setCommandStatus(`Error cancelling: ${err}`);
    }
  }

  async function handleCreateDirectory() {
    try {
      await invoke("create_directory_tool", {
        workspaceRoot,
        relativePath: newDirPath,
        confirmed: true,
      });
      setCreateDirStatus(`Successfully created "${newDirPath}".`);
    } catch (err) {
      setCreateDirStatus(`Error: ${err}`);
    }
  }

  async function handleMoveRename() {
    const confirmedByUser = window.confirm(
      `The agent wants to MOVE/RENAME "${moveFrom}" to "${moveTo}".\n\nApprove?`
    );
    if (!confirmedByUser) {
      setMoveStatus("Move cancelled by user.");
      return;
    }

    try {
      await invoke("move_rename_tool", {
        workspaceRoot,
        fromRelativePath: moveFrom,
        toRelativePath: moveTo,
        confirmed: true,
      });
      setMoveStatus(`Successfully moved "${moveFrom}" to "${moveTo}".`);
    } catch (err) {
      setMoveStatus(`Error: ${err}`);
    }
  }

  async function handleSearchFiles() {
    try {
      const results = await invoke<SearchResult[]>("search_files_tool", {
        workspaceRoot,
        namePattern: searchNamePattern || null,
        contentPattern: searchContentPattern || null,
      });
      setSearchResults(results);
      setSearchStatus(`Found ${results.length} result(s).`);
    } catch (err) {
      setSearchStatus(`Error: ${err}`);
      setSearchResults([]);
    }
  }

  async function handleGetMetadata() {
    try {
      const result = await invoke<FileMetadata>("get_file_metadata_tool", {
        workspaceRoot,
        relativePath: metadataPath,
      });
      setMetadataResult(result);
      setMetadataStatus("");
    } catch (err) {
      setMetadataStatus(`Error: ${err}`);
      setMetadataResult(null);
    }
  }

  async function handleGitDiff() {
    try {
      const result = await invoke<GitDiffResult>("git_diff_tool", { workspaceRoot });
      setGitDiffResult(result);
      setGitDiffStatus("");
    } catch (err) {
      setGitDiffStatus(`Error: ${err}`);
      setGitDiffResult(null);
    }
  }

  async function handleGitCommit() {
    const confirmedByUser = window.confirm(
      `The agent wants to COMMIT with message:\n"${commitMessage}"\n\nThis stages ALL changes and commits them. Approve?`
    );
    if (!confirmedByUser) {
      setCommitStatus("Commit cancelled by user.");
      return;
    }

    try {
      const result = await invoke<GitCommitResult>("git_commit_tool", {
        workspaceRoot,
        message: commitMessage,
        confirmed: true,
      });
      setCommitStatus(`Commit ${result.success ? "succeeded" : "failed"}: ${result.output}`);
    } catch (err) {
      setCommitStatus(`Error: ${err}`);
    }
  }

  async function handleGetAuditLog() {
    try {
      const result = await invoke<AuditLogRecord[]>("get_audit_log_tool", { workspaceRoot });
      setAuditLog(result);
      setAuditLogStatus(`Loaded ${result.length} entries.`);
    } catch (err) {
      setAuditLogStatus(`Error: ${err}`);
      setAuditLog([]);
    }
  }

  async function handleListSnapshots() {
    try {
      const result = await invoke<SnapshotRecord[]>("list_snapshots_tool", { workspaceRoot });
      setSnapshots(result);
      setSnapshotStatus(`Loaded ${result.length} snapshot(s).`);
    } catch (err) {
      setSnapshotStatus(`Error: ${err}`);
      setSnapshots([]);
    }
  }

  async function handleRestoreSnapshot(commitHash: string, message: string) {
    const typed = window.prompt(
      `PRIVILEGED ACTION — enforced by the backend, not just this dialog.\n\nThis will HARD RESET the entire workspace back to:\n"${message}"\n(${commitHash.slice(
        0,
        8
      )})\n\nEverything done since then will be lost. Type RESTORE to confirm:`
    );

    if (typed !== "RESTORE") {
      setSnapshotStatus("Restore cancelled — confirmation text did not match.");
      return;
    }

    try {
      await invoke("restore_snapshot_tool", {
        workspaceRoot,
        commitHash,
        confirmed: true,
      });
      setSnapshotStatus(`Successfully restored to snapshot ${commitHash.slice(0, 8)}.`);
    } catch (err) {
      setSnapshotStatus(`Error: ${err}`);
    }
  }

  async function handleListToolSchemas() {
    try {
      const result = await invoke<ToolSchema[]>("list_tool_schemas_tool", {});
      setToolSchemas(result);
      setToolSchemasStatus(`Loaded ${result.length} tool schema(s).`);
    } catch (err) {
      setToolSchemasStatus(`Error: ${err}`);
      setToolSchemas([]);
    }
  }

  // --- Agent orchestration (ReAct loop) ---

  async function callAgentStep(messages: ChatMessage[], resumeConfirmed: boolean): Promise<AgentStepResult> {
    return await invoke<AgentStepResult>("run_agent_tool", {
      workspaceRoot,
      apiKey: agentApiKey,
      model: agentModel,
      messagesJson: JSON.stringify(messages),
      resumeConfirmed,
    });
  }

  async function processAgentResult(result: AgentStepResult) {
    if (result.status === "Done") {
      setAgentMessages(result.messages);
      setAgentFinalAnswer(result.answer);
      setAgentStatus("Done.");
      setAgentRunning(false);
      return;
    }

    if (result.status === "AwaitingConfirmation") {
      setAgentMessages(result.messages);
      const approve = window.confirm(
        `PRIVILEGED ACTION requested by the agent (backend-enforced, same checkpoint as everywhere else):\n\n${result.tool_name}\n${JSON.stringify(
          result.arguments,
          null,
          2
        )}\n\nApprove?`
      );
      if (approve) {
        setAgentStatus(`Approved "${result.tool_name}" — resuming...`);
        try {
          const next = await callAgentStep(result.messages, true);
          await processAgentResult(next);
        } catch (err) {
          setAgentStatus(`Error: ${err}`);
          setAgentRunning(false);
        }
      } else {
        setAgentStatus(`You declined "${result.tool_name}". Agent paused — click Run Agent again to start a fresh attempt.`);
        setAgentRunning(false);
      }
      return;
    }

    if (result.status === "StoppedForSafety") {
      setAgentMessages(result.messages);
      setAgentStatus(`Stopped itself: ${result.reason}`);
      setAgentRunning(false);
      return;
    }

    // status === "Error"
    setAgentStatus(`Error: ${result.message}`);
    setAgentRunning(false);
  }

  async function handleStartAgent() {
    setAgentRunning(true);
    setAgentFinalAnswer(null);
    setAgentStatus("Running...");

    const initialMessages: ChatMessage[] = [
      {
        role: "system",
        content:
          "You are a helpful coding assistant with access to filesystem and terminal tools in this workspace. Use the tools to accomplish the user's goal step by step. When you give your final answer: write it for a human to read, not as a single run-on line — use short paragraphs or a markdown-style list with one item per line (each starting with '- '), and put each item on its own line using an actual newline character. Never use the execute_command tool for simple computations you can do yourself, such as converting a Unix timestamp to a readable date, doing arithmetic, or formatting text — reason about those directly and only use execute_command for things that genuinely require running a program.",
      },
      { role: "user", content: agentGoal },
    ];
    setAgentMessages(initialMessages);

    try {
      const result = await callAgentStep(initialMessages, false);
      await processAgentResult(result);
    } catch (err) {
      setAgentStatus(`Error: ${err}`);
      setAgentRunning(false);
    }
  }

  function renderAgentTranscript() {
    return agentMessages
      .filter((m) => m.role !== "system")
      .flatMap((m, i) => {
        if (m.role === "user") {
          return [
            <div key={`u-${i}`} style={{ marginBottom: "0.5rem" }}>
              <strong>Goal:</strong> {m.content}
            </div>,
          ];
        }
        if (m.role === "assistant" && m.tool_calls && m.tool_calls.length > 0) {
          return m.tool_calls.map((tc, j) => (
            <div key={`a-${i}-${j}`} style={{ marginBottom: "0.25rem", fontFamily: "monospace" }}>
              → <strong>{tc.function.name}</strong>({tc.function.arguments})
            </div>
          ));
        }
        if (m.role === "assistant" && m.content) {
          return [
            <div key={`af-${i}`} style={{ marginBottom: "0.5rem", whiteSpace: "pre-wrap" }}>
              <strong>Final answer:</strong> {m.content}
            </div>,
          ];
        }
        if (m.role === "tool") {
          return [
            <div key={`t-${i}`} style={{ marginBottom: "0.5rem", color: "#555", fontFamily: "monospace" }}>
              ← {m.content}
            </div>,
          ];
        }
        return [];
      });
  }

  return (
    <div style={{ padding: "2rem" }}>
      <h2>AI CoWorker</h2>
      <input
        placeholder="Workspace root folder (e.g. D:\test-workspace)"
        value={workspaceRoot}
        onChange={(e) => setWorkspaceRoot(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <input
        placeholder="Relative file path (e.g. notes.txt)"
        value={filePath}
        onChange={(e) => setFilePath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button type="button" onClick={handleReadFile}>Read File</button>
      <pre style={{ marginTop: "1rem", whiteSpace: "pre-wrap" }}>{output}</pre>

      <hr style={{ margin: "1.5rem 0" }} />

      <input
        placeholder="Relative directory path (e.g. . for workspace root)"
        value={dirPath}
        onChange={(e) => setDirPath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleListDirectory}>List Directory</button>
      <ul>
        {dirEntries.map((entry) => (
          <li key={entry.name}>
            {entry.is_directory ? "📁" : "📄"} {entry.name}
          </li>
        ))}
      </ul>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Write File (mutating)</h3>
      <input
        placeholder="Relative file path to write (e.g. new-note.txt)"
        value={writeFilePath}
        onChange={(e) => setWriteFilePath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <textarea
        placeholder="File content"
        value={writeContent}
        onChange={(e) => setWriteContent(e.target.value)}
        style={{ width: "100%", height: "100px", marginBottom: "0.5rem" }}
      />
      <button onClick={handleWriteFile}>Write File</button>
      <p style={{ marginTop: "0.5rem" }}>{writeStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Convert DOCX to PDF (mutating)</h3>
      <input
        placeholder="Relative .docx path to convert"
        value={convertFilePath}
        onChange={(e) => setConvertFilePath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleConvertToPdf}>Convert to PDF</button>
      <p style={{ marginTop: "0.5rem" }}>{convertStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Edit File (mutating)</h3>
      <input
        placeholder="Relative file path to edit"
        value={editFilePath}
        onChange={(e) => setEditFilePath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <textarea
        placeholder="Text to find (must match exactly, once)"
        value={oldText}
        onChange={(e) => setOldText(e.target.value)}
        style={{ width: "100%", height: "60px", marginBottom: "0.5rem" }}
      />
      <textarea
        placeholder="Replacement text"
        value={newText}
        onChange={(e) => setNewText(e.target.value)}
        style={{ width: "100%", height: "60px", marginBottom: "0.5rem" }}
      />
      <button onClick={handleEditFile}>Edit File</button>
      <p style={{ marginTop: "0.5rem" }}>{editStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3 style={{ color: "#b00020" }}>Delete File/Folder (privileged — backend-enforced)</h3>
      <input
        placeholder="Relative path to delete"
        value={deleteFilePath}
        onChange={(e) => setDeleteFilePath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <label style={{ display: "block", marginBottom: "0.5rem" }}>
        <input
          type="checkbox"
          checked={deleteRecursive}
          onChange={(e) => setDeleteRecursive(e.target.checked)}
        />
        {" "}Recursive (required for non-empty folders)
      </label>
      <button onClick={handleDeleteFile} style={{ color: "#b00020" }}>Delete</button>
      <p style={{ marginTop: "0.5rem" }}>{deleteStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Execute Terminal Command (risk-classified, privileged backend-enforced)</h3>
      <input
        placeholder="Command to run (e.g. dir, git status, npm test)"
        value={command}
        onChange={(e) => setCommand(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={() => handleRunCommand(false)}>Run Command</button>
      {runningJobId !== null && (
        <button onClick={handleCancelCommand} style={{ marginLeft: "0.5rem", color: "#b00020" }}>
          Cancel
        </button>
      )}
      <p style={{ marginTop: "0.5rem" }}>{commandStatus}</p>
      {commandResult && (
        <div style={{ marginTop: "1rem" }}>
          <p>
            <strong>Risk level:</strong> {commandResult.risk_level} |{" "}
            <strong>Exit code:</strong> {commandResult.exit_code ?? "N/A"} |{" "}
            <strong>Timed out:</strong> {commandResult.timed_out ? "yes" : "no"}
          </p>
          <p><strong>stdout:</strong></p>
          <pre style={{ whiteSpace: "pre-wrap", background: "#f5f5f5", padding: "0.5rem" }}>
            {commandResult.stdout || "(empty)"}
          </pre>
          <p><strong>stderr:</strong></p>
          <pre style={{ whiteSpace: "pre-wrap", background: "#fff0f0", padding: "0.5rem" }}>
            {commandResult.stderr || "(empty)"}
          </pre>
        </div>
      )}

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Create Directory (mutating)</h3>
      <input
        placeholder="Relative path for new directory"
        value={newDirPath}
        onChange={(e) => setNewDirPath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleCreateDirectory}>Create Directory</button>
      <p style={{ marginTop: "0.5rem" }}>{createDirStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Move / Rename (mutating)</h3>
      <input
        placeholder="From"
        value={moveFrom}
        onChange={(e) => setMoveFrom(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <input
        placeholder="To"
        value={moveTo}
        onChange={(e) => setMoveTo(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleMoveRename}>Move / Rename</button>
      <p style={{ marginTop: "0.5rem" }}>{moveStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Search Files (read-only)</h3>
      <input
        placeholder="Name pattern (optional)"
        value={searchNamePattern}
        onChange={(e) => setSearchNamePattern(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <input
        placeholder="Content pattern (optional)"
        value={searchContentPattern}
        onChange={(e) => setSearchContentPattern(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleSearchFiles}>Search</button>
      <p style={{ marginTop: "0.5rem" }}>{searchStatus}</p>
      <ul>
        {searchResults.map((r, i) => (
          <li key={i}>{r.relative_path} — matched on {r.matched_on}</li>
        ))}
      </ul>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Get File Metadata (read-only)</h3>
      <input
        placeholder="Relative path"
        value={metadataPath}
        onChange={(e) => setMetadataPath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleGetMetadata}>Get Metadata</button>
      <p style={{ marginTop: "0.5rem" }}>{metadataStatus}</p>
      {metadataResult && (
        <pre style={{ whiteSpace: "pre-wrap", background: "#f5f5f5", padding: "0.5rem" }}>
          {JSON.stringify(metadataResult, null, 2)}
        </pre>
      )}

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Git Diff (read-only)</h3>
      <button onClick={handleGitDiff}>Show Diff</button>
      <p style={{ marginTop: "0.5rem" }}>{gitDiffStatus}</p>
      {gitDiffResult && (
        <pre style={{ whiteSpace: "pre-wrap", background: "#f5f5f5", padding: "0.5rem" }}>
          {gitDiffResult.has_changes ? gitDiffResult.diff : "No changes."}
        </pre>
      )}

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Git Commit (mutating)</h3>
      <input
        placeholder="Commit message"
        value={commitMessage}
        onChange={(e) => setCommitMessage(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleGitCommit}>Stage All &amp; Commit</button>
      <p style={{ marginTop: "0.5rem" }}>{commitStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Audit Log (backend-generated, read-only)</h3>
      <button onClick={handleGetAuditLog}>Refresh Audit Log</button>
      <p style={{ marginTop: "0.5rem" }}>{auditLogStatus}</p>
      <table style={{ width: "100%", marginTop: "0.5rem", borderCollapse: "collapse" }}>
        <thead>
          <tr style={{ textAlign: "left", borderBottom: "1px solid #ccc" }}>
            <th>Time</th>
            <th>Tier</th>
            <th>Action</th>
            <th>Success</th>
          </tr>
        </thead>
        <tbody>
          {auditLog.map((entry, i) => (
            <tr key={i} style={{ borderBottom: "1px solid #eee" }}>
              <td>{new Date(entry.timestamp_unix * 1000).toLocaleString()}</td>
              <td style={{ color: entry.tier === "privileged" ? "#b00020" : "inherit" }}>
                {entry.tier}
              </td>
              <td>{entry.action}</td>
              <td>{entry.success ? "✅" : "❌"}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3 style={{ color: "#b00020" }}>Snapshots / Undo (privileged restore — backend-enforced)</h3>
      <p style={{ fontSize: "0.85rem", color: "#666" }}>
        A snapshot is taken automatically before every mutating/privileged action. Restoring
        hard-resets the entire workspace to that point in time.
      </p>
      <button onClick={handleListSnapshots}>Refresh Snapshots</button>
      <p style={{ marginTop: "0.5rem" }}>{snapshotStatus}</p>
      <table style={{ width: "100%", marginTop: "0.5rem", borderCollapse: "collapse" }}>
        <thead>
          <tr style={{ textAlign: "left", borderBottom: "1px solid #ccc" }}>
            <th>Time</th>
            <th>Message</th>
            <th>Commit</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {snapshots.map((s) => (
            <tr key={s.commit_hash} style={{ borderBottom: "1px solid #eee" }}>
              <td>{new Date(s.timestamp_unix * 1000).toLocaleString()}</td>
              <td>{s.message}</td>
              <td style={{ fontFamily: "monospace" }}>{s.commit_hash.slice(0, 8)}</td>
              <td>
                <button
                  onClick={() => handleRestoreSnapshot(s.commit_hash, s.message)}
                  style={{ color: "#b00020" }}
                >
                  Restore
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Tool Registry (debug — schemas the agent loop will use)</h3>
      <p style={{ fontSize: "0.85rem", color: "#666" }}>
        The "tier" shown here is informational only — the backend independently re-checks and
        enforces the real tier on every call, regardless of what a model claims.
      </p>
      <button onClick={handleListToolSchemas}>Load Tool Schemas</button>
      <p style={{ marginTop: "0.5rem" }}>{toolSchemasStatus}</p>
      {toolSchemas.map((t) => (
        <details key={t.name} style={{ marginTop: "0.5rem" }}>
          <summary>
            <strong>{t.name}</strong>{" "}
            <span
              style={{
                color:
                  t.tier === "privileged" ? "#b00020" : t.tier === "mutating" ? "#a15c00" : "#2a7a2a",
              }}
            >
              [{t.tier}]
            </span>{" "}
            — {t.description}
          </summary>
          <pre style={{ whiteSpace: "pre-wrap", background: "#f5f5f5", padding: "0.5rem" }}>
            {JSON.stringify(t.parameters, null, 2)}
          </pre>
        </details>
      ))}

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Run Agent (ReAct loop — privileged actions still backend-enforced)</h3>
      <p style={{ fontSize: "0.85rem", color: "#666" }}>
        Leave API key blank to use GROQ_API_KEY from src-tauri/.env. Any privileged tool call the
        agent wants to make still pauses here for your approval, exactly like the manual controls
        above — the model has no way to skip that.
      </p>
      <input
        placeholder="Groq API key (optional — falls back to .env)"
        value={agentApiKey}
        onChange={(e) => setAgentApiKey(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
        type="password"
      />
      <input
        placeholder="Model (e.g. openai/gpt-oss-120b)"
        value={agentModel}
        onChange={(e) => setAgentModel(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <textarea
        placeholder="Goal (e.g. List the files in the workspace, then read the first one and tell me what it contains.)"
        value={agentGoal}
        onChange={(e) => setAgentGoal(e.target.value)}
        style={{ width: "100%", height: "80px", marginBottom: "0.5rem" }}
      />
      <button onClick={handleStartAgent} disabled={agentRunning || !agentGoal || !workspaceRoot}>
        {agentRunning ? "Running..." : "Run Agent"}
      </button>
      <p style={{ marginTop: "0.5rem" }}>{agentStatus}</p>
      {agentFinalAnswer && (
        <div style={{ marginTop: "0.5rem", padding: "0.5rem", background: "#e8f5e9", whiteSpace: "pre-wrap" }}>
          <strong>Final answer:</strong> {agentFinalAnswer}
        </div>
      )}
      <div style={{ marginTop: "1rem", padding: "0.5rem", background: "#f5f5f5" }}>
        {renderAgentTranscript()}
      </div>
    </div>
  );
}

export default App;