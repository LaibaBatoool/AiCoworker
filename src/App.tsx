import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

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

function App() {
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
      `PRIVILEGED ACTION — enforced by the backend, not just this dialog.\n\nThe agent wants to DELETE "${deleteFilePath}"${
        deleteRecursive ? " (and everything inside it)" : ""
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
      const result = await invoke<CommandOutput>("execute_command_tool", {
        workspaceRoot,
        command,
        confirmed,
      });
      setCommandResult(result);
      setCommandStatus("");
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
    </div>
  );
}

export default App;