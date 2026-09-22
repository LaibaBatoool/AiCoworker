import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

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
    </div>
  );
}

export default App;