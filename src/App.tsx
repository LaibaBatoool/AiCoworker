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
    const confirmed = window.confirm(
      `The agent wants to WRITE to "${writeFilePath}".\n\nThis will create or overwrite this file. Approve?`
    );
    if (!confirmed) {
      setWriteStatus("Write cancelled by user.");
      return;
    }

    try {
      await invoke("write_file_tool", {
        workspaceRoot,
        relativePath: writeFilePath,
        content: writeContent,
      });
      setWriteStatus(`Successfully wrote to "${writeFilePath}".`);
    } catch (err) {
      setWriteStatus(`Error: ${err}`);
    }
  }

  async function handleConvertToPdf() {
    const confirmed = window.confirm(
      `The agent wants to CONVERT "${convertFilePath}" to PDF.\n\nThis will create a new PDF file alongside it. Approve?`
    );
    if (!confirmed) {
      setConvertStatus("Conversion cancelled by user.");
      return;
    }

    try {
      const resultFilename = await invoke<string>("convert_to_pdf_tool", {
        workspaceRoot,
        relativePath: convertFilePath,
      });
      setConvertStatus(`Successfully converted to "${resultFilename}".`);
    } catch (err) {
      setConvertStatus(`Error: ${err}`);
    }
  }

  async function handleEditFile() {
    const confirmed = window.confirm(
      `The agent wants to EDIT "${editFilePath}".\n\nReplace:\n"${oldText}"\n\nWith:\n"${newText}"\n\nApprove?`
    );
    if (!confirmed) {
      setEditStatus("Edit cancelled by user.");
      return;
    }

    try {
      await invoke("edit_file_tool", {
        workspaceRoot,
        relativePath: editFilePath,
        oldText,
        newText,
      });
      setEditStatus(`Successfully edited "${editFilePath}".`);
    } catch (err) {
      setEditStatus(`Error: ${err}`);
    }
  }

  async function handleDeleteFile() {
    const typed = window.prompt(
      `PRIVILEGED ACTION — this cannot be undone.\n\nThe agent wants to DELETE "${deleteFilePath}"${
        deleteRecursive ? " (and everything inside it, if it's a folder)" : ""
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
          `PRIVILEGED COMMAND\n\n"${command}"\n\nThis command was classified as privileged/high-risk. Run it anyway?`
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
      });
      setCreateDirStatus(`Successfully created "${newDirPath}".`);
    } catch (err) {
      setCreateDirStatus(`Error: ${err}`);
    }
  }

  async function handleMoveRename() {
    const confirmed = window.confirm(
      `The agent wants to MOVE/RENAME "${moveFrom}" to "${moveTo}".\n\nApprove?`
    );
    if (!confirmed) {
      setMoveStatus("Move cancelled by user.");
      return;
    }

    try {
      await invoke("move_rename_tool", {
        workspaceRoot,
        fromRelativePath: moveFrom,
        toRelativePath: moveTo,
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

      <h3>Write File (mutating — requires confirmation)</h3>
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

      <h3>Convert DOCX to PDF (mutating — requires confirmation)</h3>
      <input
        placeholder="Relative .docx path to convert (e.g. writetest2.docx)"
        value={convertFilePath}
        onChange={(e) => setConvertFilePath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleConvertToPdf}>Convert to PDF</button>
      <p style={{ marginTop: "0.5rem" }}>{convertStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Edit File (mutating — requires confirmation)</h3>
      <input
        placeholder="Relative file path to edit (e.g. notes.txt)"
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

      <h3 style={{ color: "#b00020" }}>Delete File/Folder (privileged — type name to confirm)</h3>
      <input
        placeholder="Relative path to delete (e.g. old-test.txt or old-folder)"
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

      <h3>Execute Terminal Command (risk-classified automatically)</h3>
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
        placeholder="Relative path for new directory (e.g. new-folder)"
        value={newDirPath}
        onChange={(e) => setNewDirPath(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleCreateDirectory}>Create Directory</button>
      <p style={{ marginTop: "0.5rem" }}>{createDirStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Move / Rename (mutating — requires confirmation)</h3>
      <input
        placeholder="From (e.g. old-name.txt)"
        value={moveFrom}
        onChange={(e) => setMoveFrom(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <input
        placeholder="To (e.g. new-name.txt or subfolder/new-name.txt)"
        value={moveTo}
        onChange={(e) => setMoveTo(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <button onClick={handleMoveRename}>Move / Rename</button>
      <p style={{ marginTop: "0.5rem" }}>{moveStatus}</p>

      <hr style={{ margin: "1.5rem 0" }} />

      <h3>Search Files (read-only)</h3>
      <input
        placeholder="Name pattern (optional, e.g. test)"
        value={searchNamePattern}
        onChange={(e) => setSearchNamePattern(e.target.value)}
        style={{ width: "100%", marginBottom: "0.5rem" }}
      />
      <input
        placeholder="Content pattern (optional, e.g. hello)"
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
        placeholder="Relative path (e.g. notes.txt)"
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
    </div>
  );
}

export default App;