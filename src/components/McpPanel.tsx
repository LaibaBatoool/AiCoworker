// "Connections" panel: shows the MCP servers from %APPDATA%\AiCoworker\mcp.json,
// every tool they expose to the agent, and the tier the BACKEND enforces for each.
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./mcppanel.css";

interface McpToolInfo { qualified: string; server: string; tool: string; tier: string; description: string }
interface McpStatus {
  config_path: string | null;
  config_exists: boolean;
  running_servers: string[];
  tools: McpToolInfo[];
  problems: string[];
}

export default function McpPanel({ workspaceRoot }: { workspaceRoot: string }) {
  const [st, setSt] = useState<McpStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  async function load(reload: boolean) {
    if (!workspaceRoot) { setErr("Pick a workspace folder first."); return; }
    setBusy(true); setErr("");
    try {
      setSt(await invoke<McpStatus>("mcp_status_tool", { workspaceRoot, reload }));
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  const servers = st ? [...new Set([...st.running_servers, ...st.tools.map((t) => t.server)])] : [];

  return (
    <div className="mcp-panel">
      <div className="mcp-head">
        <div>
          <div className="mcp-title">🔌 Connections (MCP servers)</div>
          <div className="mcp-sub">External tools the agent can use. Tiers come from your config, never from the server.</div>
        </div>
        <div className="mcp-actions">
          <button onClick={() => load(false)} disabled={busy}>{busy ? "Starting…" : "Check"}</button>
          <button onClick={() => load(true)} disabled={busy} title="Stop all servers and re-read mcp.json">Reload config</button>
        </div>
      </div>

      {err && <div className="mcp-problem">{err}</div>}

      {st && (
        <>
          <div className="mcp-path">
            Config: <code>{st.config_path ?? "(no config location)"}</code>{" "}
            {st.config_exists ? <span className="mcp-ok">found</span> : <span className="mcp-warn">not found — no MCP servers</span>}
          </div>

          {st.problems.map((p, i) => <div key={i} className="mcp-problem">⚠ {p}</div>)}

          {servers.map((s) => {
            const tools = st.tools.filter((t) => t.server === s);
            return (
              <div key={s} className="mcp-server">
                <div className="mcp-server-head">
                  <span className={`mcp-dot ${st.running_servers.includes(s) ? "on" : ""}`} />
                  <b>{s}</b>
                  <span className="mcp-muted">{tools.length} tool{tools.length === 1 ? "" : "s"}</span>
                </div>
                <table className="mcp-table">
                  <tbody>
                    {tools.map((t) => (
                      <tr key={t.qualified}>
                        <td className="mcp-tool">{t.tool}</td>
                        <td><span className={`mcp-tier mcp-tier-${t.tier}`}>{t.tier}</span></td>
                        <td className="mcp-desc">{t.description}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            );
          })}

          {st.config_exists && servers.length === 0 && st.problems.length === 0 && (
            <div className="mcp-muted">The config has no enabled servers.</div>
          )}
        </>
      )}
    </div>
  );
}