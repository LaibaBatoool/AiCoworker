// Audit database panel (Debug Tools). Reads the central SQLite store
// (%APPDATA%\AiCoworker\aicoworker.db, store.rs): chain health + newest rows.
// Read-only: the UI can never change or delete audit rows.
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./auditstore.css";

interface StoreStatus {
  path: string;
  enabled: boolean;
  rows: number;
  chain_ok: boolean;
  head_ok: boolean;
  problems: string[];
  alerts: string[];
}
interface StoreRow {
  id: number;
  timestamp_unix: number;
  workspace: string;
  tier: string;
  action: string;
  success: boolean;
  detail: string;
  hash: string;
}

export default function AuditStorePanel({ workspaceRoot }: { workspaceRoot: string }) {
  const [st, setSt] = useState<StoreStatus | null>(null);
  const [rows, setRows] = useState<StoreRow[]>([]);
  const [allWs, setAllWs] = useState(false);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  async function load(all = allWs) {
    setBusy(true);
    setErr("");
    try {
      const status = await invoke<StoreStatus>("audit_store_status_tool");
      setSt(status);
      if (status.enabled) {
        const ws = all ? "" : workspaceRoot;
        if (!all && !workspaceRoot) {
          setRows([]);
          setErr("Pick a workspace folder, or tick \"All workspaces\".");
        } else {
          setRows(await invoke<StoreRow[]>("audit_store_recent_tool", { workspaceRoot: ws, limit: 200 }));
        }
      }
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  const healthy = st && st.enabled && st.chain_ok && st.head_ok;

  return (
    <div className="as-panel">
      <div className="as-head">
        <div>
          <div className="as-title">🗄️ Audit database (SQLite, hash-chained)</div>
          <div className="as-sub">Stored outside every workspace, so agent tools can't reach it. Any change made outside the app is detected.</div>
        </div>
        <div className="as-actions">
          <label className="as-check">
            <input
              type="checkbox"
              checked={allWs}
              onChange={(e) => {
                setAllWs(e.target.checked);
                load(e.target.checked);
              }}
            />
            All workspaces
          </label>
          <button onClick={() => load()} disabled={busy}>{busy ? "Checking…" : "Check & refresh"}</button>
        </div>
      </div>

      {err && <div className="as-problem">{err}</div>}

      {st && (
        <>
          <div className="as-path">
            <code>{st.path}</code> · {st.rows} rows
          </div>
          {!st.enabled ? (
            <div className="as-problem">Database not in use: {st.problems.join("; ")}</div>
          ) : healthy ? (
            <div className="as-ok">✅ Chain intact and unchanged since the app's last write.</div>
          ) : (
            <div className="as-alarm">
              <div className="as-alarm-title">⚠ The audit log was modified outside the app</div>
              {st.problems.map((p, i) => (
                <div key={i}>• {p}</div>
              ))}
            </div>
          )}
          {st.alerts.length > 0 && (
            <div className="as-alerts">
              <div className="as-muted">Tamper alerts this session (also written into the log):</div>
              {st.alerts.map((a, i) => (
                <div key={i}>• {a}</div>
              ))}
            </div>
          )}
        </>
      )}

      {rows.length > 0 && (
        <table className="as-table">
          <thead>
            <tr>
              <th>#</th>
              <th>Time</th>
              <th>Tier</th>
              <th>Action</th>
              <th>OK</th>
              <th>Hash</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id} className={r.action === "AUDIT_TAMPER_DETECTED" ? "as-row-alert" : ""}>
                <td>{r.id}</td>
                <td className="as-nowrap">{new Date(r.timestamp_unix * 1000).toLocaleString()}</td>
                <td>
                  <span className={`as-tier as-tier-${r.tier}`}>{r.tier}</span>
                </td>
                <td className="as-action" title={r.detail || undefined}>
                  {r.action}
                  {allWs && <div className="as-muted">{r.workspace}</div>}
                  {r.action === "AUDIT_TAMPER_DETECTED" && <div className="as-detail">{r.detail}</div>}
                </td>
                <td>{r.success ? "✅" : "❌"}</td>
                <td className="as-hash" title={r.hash}>{r.hash.slice(0, 10)}…</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}