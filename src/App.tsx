import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function App() {
  const [workspaceRoot, setWorkspaceRoot] = useState("");
  const [filePath, setFilePath] = useState("");
  const [output, setOutput] = useState("");

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
    </div>
  );
}

export default App;