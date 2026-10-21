// Live Run view: shows the agent working in real time.
// - left:   step timeline (each tool call appears as it starts, with its real safety tier)
// - middle: live "stage" — the file being typed, the sheet filling in, the doc being built,
//           the command running in a terminal...
// - right:  the workspace file tree; new / edited / deleted files light up
// Fed by the backend's "agent-event" stream (src-tauri/src/agent/events.rs).
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./liverun.css";
import Markdown, { markdownToHtml, MARKDOWN_REPORT_CSS } from "./Markdown";
import EmailStage, { emailTool, describeEmail, emailIcon } from "./EmailStage";
import CalendarStage, { calTool, describeCal, calIcon } from "./CalendarStage";
import NotesStage, { notesTool, describeNotes, notesIcon } from "./NotesStage";

// ---------- types ----------
type AgentEvent =
  | { kind: "thinking"; step: number }
  | { kind: "tool_started"; call_id: string; name: string; arguments: any; tier: string }
  | { kind: "tool_finished"; call_id: string; name: string; ok: boolean; preview: string; duration_ms: number }
  | { kind: "needs_confirmation"; name: string; arguments: any }
  | { kind: "answer"; text: string }
  | { kind: "stopped"; reason: string }
  | { kind: "model_switched"; from: string; to: string; reason: string };

type CardStatus = "running" | "ok" | "error" | "waiting" | "declined";
interface Card {
  id: string;
  name: string;
  args: any;
  tier: string;
  status: CardStatus;
  preview: string;
  durationMs?: number;
}
type Phase = "idle" | "thinking" | "acting" | "waiting" | "done" | "stopped";

interface DirEntry {
  name: string;
  is_directory: boolean;
  size_bytes?: number;
  modified_unix_timestamp?: number;
}
interface SnapshotRecord { commit_hash: string; timestamp_unix: number; message: string }
interface FileInfo { isDir: boolean; size?: number; mtime?: number }
type FileMark = "new" | "edited" | "deleted" | undefined;

interface Props {
  goal: string;
  workspaceRoot: string;
  running: boolean;
  finalAnswer: string | null;
  status: string;
  pendingConfirmation: { tool_name: string; arguments: unknown } | null;
  onApprove: () => void;
  onDecline: () => void;
}

// ---------- helpers ----------
const norm = (p: unknown) =>
  typeof p === "string" ? p.replace(/\\/g, "/").replace(/^(\.\/)+/, "").replace(/^\/+/, "").replace(/\/+$/, "") : "";

const SKIP_DIRS = new Set([".aicoworker", ".git", "node_modules", "target"]);
const MAX_TREE = 300;

async function scanWorkspace(root: string): Promise<Map<string, FileInfo>> {
  const out = new Map<string, FileInfo>();
  const queue: { rel: string; depth: number }[] = [{ rel: "", depth: 0 }];
  while (queue.length && out.size < MAX_TREE) {
    const { rel, depth } = queue.shift()!;
    let entries: DirEntry[] = [];
    try {
      entries = await invoke<DirEntry[]>("list_directory_tool", {
        workspaceRoot: root,
        relativePath: rel || ".",
        includeMetadata: true,
      });
    } catch {
      continue;
    }
    for (const e of entries) {
      if (SKIP_DIRS.has(e.name)) continue;
      const p = rel ? `${rel}/${e.name}` : e.name;
      out.set(p, { isDir: e.is_directory, size: e.size_bytes, mtime: e.modified_unix_timestamp });
      if (e.is_directory && depth < 4) queue.push({ rel: p, depth: depth + 1 });
    }
  }
  return out;
}

function pathsOf(name: string, args: any): string[] {
  if (!args || typeof args !== "object") return [];
  const keys = ["relative_path", "from_relative_path", "to_relative_path", "output_path", "destination", "archive_path"];
  const ps = keys.map((k) => norm(args[k])).filter(Boolean);
  if (Array.isArray(args.input_paths)) ps.push(...args.input_paths.map(norm));
  return name === "list_directory" ? [] : ps;
}

function shortName(p: string) {
  const parts = norm(p).split("/");
  return parts[parts.length - 1] || p;
}

function describe(name: string, a: any): string {
  const p = norm(a?.relative_path) || "the workspace";
  switch (name) {
    case "write_file": return `Writing ${p}`;
    case "edit_file": return `Editing ${p}`;
    case "read_file": return `Reading ${p}`;
    case "create_docx": return `Building Word doc ${p}`;
    case "create_xlsx": return `Building spreadsheet ${p}`;
    case "read_spreadsheet": return `Reading spreadsheet ${p}`;
    case "list_directory": return `Looking inside ${p}`;
    case "search_files": return `Searching for ${a?.name_pattern || a?.content_pattern || "files"}`;
    case "get_file_metadata": return `Checking ${p}`;
    case "create_directory": return `Creating folder ${p}`;
    case "delete_file": return `Deleting ${p}`;
    case "move_rename": return `Moving ${norm(a?.from_relative_path)} → ${norm(a?.to_relative_path)}`;
    case "execute_command": return `Running: ${a?.command ?? ""}`;
    case "fetch_url": return `Opening ${a?.url ?? "a web page"}`;
    case "convert_to_pdf": return `Converting ${p} to PDF`;
    case "git_commit": return `Committing: ${a?.message ?? ""}`;
    case "git_diff": return "Reviewing changes (git diff)";
    case "git_status": return "Checking git status";
    case "git_log": return "Reading git history";
    case "open_in_default_app": return `Opening ${p}`;
    case "request_tools": return `Loading the ${a?.group ?? ""} tools`;
    case "update_checklist": {
      const it = Array.isArray(a?.items) ? a.items : [];
      const d = it.filter((x: any) => x?.done).length;
      return d === 0 ? `Planning: ${it.length} item${it.length === 1 ? "" : "s"}` : `Checklist: ${d}/${it.length} done`;
    }
    case "plan_check": return a?.when === "start" ? "Asking for a plan first" : `No plan made — asking for one (${a?.files ?? 0} files touched)`;
    case "checklist_check": return Array.isArray(a?.open) && a.open.length ? `Checking the list: ${a.open.length} still open` : "Checking the checklist";
    case "claim_check": return Array.isArray(a?.missing) && a.missing.length ? `Checking claims: ${a.missing.join(", ")} missing` : "Checking the agent's claims";

    default:
      if (emailTool(name)) return describeEmail(emailTool(name)!, a);
      if (calTool(name)) return describeCal(calTool(name)!, a);
               if (notesTool(name)) return describeNotes(notesTool(name)!, a);
               if (name.startsWith("mcp__")) {
        const [, server, tool] = name.split("__");
        return `External tool ${tool} (${server})`;
      }
      return name.replace(/_/g, " ") + (a?.relative_path ? ` ${p}` : "");
  }
}

function icon(name: string): string {
  if (emailTool(name)) return emailIcon(emailTool(name)!);
  if (calTool(name)) return calIcon(calTool(name)!);
       if (notesTool(name)) return notesIcon(notesTool(name)!);
       if (name.startsWith("mcp__")) return "🔌";
  const m: Record<string, string> = {
    write_file: "✍️", edit_file: "🩹", read_file: "📖", create_docx: "📝", create_xlsx: "📊",
    read_spreadsheet: "📊", list_directory: "📂", search_files: "🔎", get_file_metadata: "ℹ️",
    create_directory: "📁", delete_file: "🗑️", move_rename: "↪️", execute_command: "⌨️",
    fetch_url: "🌐", request_tools: "🧰", claim_check: "🔍", update_checklist: "📋", checklist_check: "☑️", plan_check: "🗒️", convert_to_pdf: "📄", git_commit: "🌿", git_diff: "🌿", git_status: "🌿", git_log: "🌿",
  };
  return m[name] ?? "🛠️";
}

const TIER_LABEL: Record<string, string> = {
  safe: "safe · read-only",
  mutating: "changes files · snapshotted",
  privileged: "needs your approval",
};

function fmtMs(ms?: number) {
  if (ms === undefined) return "";
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

// Types text out over ~1.5–2.5 s, whatever its length.
function useTypewriter(text: string, active = true) {
  const [n, setN] = useState(active ? 0 : text.length);
  useEffect(() => {
    if (!active) { setN(text.length); return; }
    setN(0);
    const step = Math.max(1, Math.ceil(text.length / 120));
    const t = setInterval(() => {
      setN((x) => {
        if (x + step >= text.length) { clearInterval(t); return text.length; }
        return x + step;
      });
    }, 16);
    return () => clearInterval(t);
  }, [text, active]);
  return { shown: text.slice(0, n), done: n >= text.length };
}

// ---------- stage previews ----------
function EditorStage({ path, content }: { path: string; content: string }) {
  const { shown, done } = useTypewriter(content);
  const lines = shown.split("\n");
  return (
    <div className="lr-window">
      <div className="lr-window-bar"><span className="lr-dots" /><span className="lr-tab">{shortName(path)}</span></div>
      <div className="lr-editor">
        {lines.map((l, i) => (
          <div className="lr-line" key={i}>
            <span className="lr-ln">{i + 1}</span>
            <span className="lr-code">{l}{i === lines.length - 1 && !done && <span className="lr-caret" />}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function DiffStage({ path, oldText, newText }: { path: string; oldText: string; newText: string }) {
  const { shown, done } = useTypewriter(newText);
  return (
    <div className="lr-window">
      <div className="lr-window-bar"><span className="lr-dots" /><span className="lr-tab">{shortName(path)} — edit</span></div>
      <div className="lr-editor">
        <pre className="lr-del">{oldText}</pre>
        <pre className="lr-add">{shown}{!done && <span className="lr-caret" />}</pre>
      </div>
    </div>
  );
}

const colName = (i: number) => {
  let s = "";
  for (i += 1; i > 0; i = Math.floor((i - 1) / 26)) s = String.fromCharCode(65 + ((i - 1) % 26)) + s;
  return s;
};

function SheetStage({ args }: { args: any }) {
  const headers: string[] = Array.isArray(args?.headers) ? args.headers.map(String) : [];
  const rows: any[][] = Array.isArray(args?.rows) ? args.rows.slice(0, 60) : [];
  const cols = Math.max(headers.length, ...rows.map((r) => (Array.isArray(r) ? r.length : 0)), 1);
  return (
    <div className="lr-window">
      <div className="lr-window-bar"><span className="lr-dots" /><span className="lr-tab">{shortName(args?.relative_path ?? "sheet.xlsx")}</span></div>
      <div className="lr-sheet-wrap">
        <table className="lr-sheet">
          <thead>
            <tr><th className="lr-corner" />{Array.from({ length: cols }, (_, i) => <th key={i}>{colName(i)}</th>)}</tr>
          </thead>
          <tbody>
            <tr className="lr-row lr-header-row" style={{ animationDelay: "0ms" }}>
              <td className="lr-rownum">1</td>
              {Array.from({ length: cols }, (_, i) => <td key={i}>{headers[i] ?? ""}</td>)}
            </tr>
            {rows.map((r, ri) => (
              <tr className="lr-row" key={ri} style={{ animationDelay: `${Math.min(ri + 1, 40) * 70}ms` }}>
                <td className="lr-rownum">{ri + 2}</td>
                {Array.from({ length: cols }, (_, i) => {
                  const v = Array.isArray(r) ? r[i] : undefined;
                  return <td key={i} className={typeof v === "number" ? "lr-num" : ""}>{v === undefined || v === null ? "" : String(v)}</td>;
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {args?.sheet_name && <div className="lr-sheet-tab">{String(args.sheet_name)}</div>}
    </div>
  );
}

function DocStage({ args }: { args: any }) {
  const blocks: any[] = Array.isArray(args?.blocks) ? args.blocks.slice(0, 80) : [];
  return (
    <div className="lr-page-wrap">
      <div className="lr-page">
        {blocks.map((b, i) => {
          const style = { animationDelay: `${Math.min(i, 40) * 160}ms` };
          if (b?.type === "heading") {
            const lvl = Math.min(Math.max(Number(b.level) || 1, 1), 3);
            return <div key={i} className={`lr-block lr-h${lvl}`} style={style}>{String(b.text ?? "")}</div>;
          }
          if (b?.type === "table") {
            return (
              <table key={i} className="lr-block lr-doc-table" style={style}>
                <thead><tr>{(b.headers ?? []).map((h: any, j: number) => <th key={j}>{String(h)}</th>)}</tr></thead>
                <tbody>{(b.rows ?? []).slice(0, 30).map((r: any[], j: number) => (
                  <tr key={j}>{(r ?? []).map((c, k) => <td key={k}>{String(c)}</td>)}</tr>
                ))}</tbody>
              </table>
            );
          }
          return <p key={i} className="lr-block lr-p" style={style}>{String(b?.text ?? "")}</p>;
        })}
      </div>
    </div>
  );
}

function TerminalStage({ command, card }: { command: string; card: Card }) {
  const { shown, done } = useTypewriter(command);
  return (
    <div className="lr-window lr-terminal">
      <div className="lr-window-bar"><span className="lr-dots" /><span className="lr-tab">PowerShell</span></div>
      <pre className="lr-term-body">
        <span className="lr-prompt">PS&gt; </span>{shown}{!done && <span className="lr-caret" />}
        {"\n"}
        {done && card.status === "running" && <span className="lr-muted">running…</span>}
        {card.status !== "running" && card.status !== "waiting" && card.preview}
        {card.status === "waiting" && <span className="lr-amber">⏸ waiting for your approval</span>}
      </pre>
    </div>
  );
}

function BrowserStage({ url, card }: { url: string; card: Card }) {
  return (
    <div className="lr-window">
      <div className="lr-window-bar"><span className="lr-dots" /><span className="lr-url">🔒 {url}</span></div>
      {card.status === "running" && <div className="lr-loadbar" />}
      <pre className="lr-browser-body">
        {card.status === "running" ? "Loading…" : card.status === "waiting" ? "⏸ waiting for your approval" : card.preview}
      </pre>
    </div>
  );
}

function ReadStage({ card }: { card: Card }) {
  return (
    <div className={`lr-read ${card.status === "running" ? "lr-scanning" : ""}`}>
      <div className="lr-read-title">{icon(card.name)} {describe(card.name, card.args)}</div>
      <pre className="lr-read-body">{card.status === "running" ? "…" : card.preview || "(empty)"}</pre>
    </div>
  );
}

function GenericStage({ card }: { card: Card }) {
  return (
    <div className="lr-read">
      <div className="lr-read-title">{icon(card.name)} {describe(card.name, card.args)}</div>
      <div className="lr-sub">Arguments</div>
      <pre className="lr-read-body">{JSON.stringify(card.args, null, 2)}</pre>
      {card.status !== "running" && (
        <>
          <div className="lr-sub">Result</div>
          <pre className="lr-read-body">{card.preview}</pre>
        </>
      )}
    </div>
  );
}
// the agent's to-do list (update_checklist): pinned on top of the steps, and on the stage
function Checklist({ items, pinned }: { items: any; pinned?: boolean }) {
  const list: { text: string; done: boolean }[] = Array.isArray(items) ? items.filter((x: any) => x && typeof x.text === "string") : [];
  const done = list.filter((x) => x.done).length;
  return (
    <div className={`lr-checklist ${pinned ? "lr-checklist-pinned" : ""}`}>
      <div className="lr-checklist-head">
        <span>📋 Plan</span>
        <span className={done === list.length ? "lr-checklist-all" : "lr-muted"}>{done}/{list.length}</span>
      </div>
      <div className="lr-checklist-bar"><div style={{ width: `${list.length ? (done / list.length) * 100 : 0}%` }} /></div>
      <ul>
        {list.map((x, i) => (
          <li key={i} className={x.done ? "done" : ""}><span className="lr-check">{x.done ? "✓" : ""}</span><span>{x.text}</span></li>
        ))}
      </ul>
    </div>
  );
}

function Stage({ card }: { card: Card }) {
  const a = card.args ?? {};
  switch (card.name) {
    case "update_checklist": return <Checklist items={a.items} />;
    case "write_file": return <EditorStage key={card.id} path={a.relative_path ?? ""} content={String(a.content ?? "")} />;
    case "edit_file": return <DiffStage key={card.id} path={a.relative_path ?? ""} oldText={String(a.old_text ?? "")} newText={String(a.new_text ?? "")} />;
    case "create_xlsx": return <SheetStage key={card.id} args={a} />;
    case "create_docx": return <DocStage key={card.id} args={a} />;
    case "execute_command": return <TerminalStage key={card.id} command={String(a.command ?? "")} card={card} />;
    case "fetch_url": return <BrowserStage url={String(a.url ?? "")} card={card} />;
    case "delete_file":
      return (
        <div className={`lr-delete ${card.status === "ok" ? "lr-shred" : ""}`}>
          <div className="lr-big-icon">{a.recursive ? "📁" : "📄"}</div>
          <div>{norm(a.relative_path)}</div>
          <div className="lr-muted">{card.status === "waiting" ? "waiting for your approval" : card.status === "ok" ? "deleted (a snapshot was taken first)" : card.status === "declined" ? "you declined — nothing deleted" : ""}</div>
        </div>
      );
    case "move_rename":
      return (
        <div className="lr-move">
          <span className="lr-chip">{norm(a.from_relative_path)}</span>
          <span className="lr-arrow">⟶</span>
          <span className="lr-chip">{norm(a.to_relative_path)}</span>
        </div>
      );
    case "create_directory":
      return <div className="lr-delete lr-pop"><div className="lr-big-icon">📁</div><div>{norm(a.relative_path)}</div></div>;
    default:
      if (emailTool(card.name)) return <EmailStage card={card} />;
      if (calTool(card.name)) return <CalendarStage card={card} />;
               if (notesTool(card.name)) return <NotesStage card={card} />;
               if (["read_file", "read_spreadsheet", "list_directory", "search_files", "get_file_metadata", "git_diff", "git_status", "git_log"].includes(card.name))
        return <ReadStage card={card} />;
      return <GenericStage card={card} />;
  }
}

// ---------- file tree ----------
interface TreeNode { name: string; path: string; isDir: boolean; mark: FileMark; children: TreeNode[] }

function buildTree(current: Map<string, FileInfo>, marks: Map<string, FileMark>, ghosts: Map<string, FileInfo>): TreeNode[] {
  const root: TreeNode = { name: "", path: "", isDir: true, mark: undefined, children: [] };
  const all = new Map<string, FileInfo>([...ghosts, ...current]);
  const paths = [...all.keys()].sort();
  const find = (parent: TreeNode, name: string, path: string, isDir: boolean) => {
    let n = parent.children.find((c) => c.name === name);
    if (!n) { n = { name, path, isDir, mark: marks.get(path), children: [] }; parent.children.push(n); }
    return n;
  };
  for (const p of paths) {
    const parts = p.split("/");
    let node = root;
    parts.forEach((part, i) => {
      const sub = parts.slice(0, i + 1).join("/");
      const isLast = i === parts.length - 1;
      node = find(node, part, sub, isLast ? all.get(p)!.isDir : true);
    });
  }
  const sort = (n: TreeNode) => {
    n.children.sort((a, b) => (a.isDir === b.isDir ? a.name.localeCompare(b.name) : a.isDir ? -1 : 1));
    n.children.forEach(sort);
  };
  sort(root);
  return root.children;
}

function Tree({ nodes, active, depth = 0 }: { nodes: TreeNode[]; active: Set<string>; depth?: number }) {
  return (
    <>
      {nodes.map((n) => (
        <div key={n.path}>
          <div
            key={`${n.path}:${n.mark ?? ""}`}
            className={`lr-node ${n.mark ? `lr-mark-${n.mark}` : ""} ${active.has(n.path) ? "lr-active" : ""}`}
            style={{ paddingLeft: 10 + depth * 14 }}
          >
            <span className="lr-node-icon">{n.isDir ? "📁" : "📄"}</span>
            <span className="lr-node-name">{n.name}</span>
            {n.mark && <span className={`lr-badge lr-badge-${n.mark}`}>{n.mark}</span>}
          </div>
          {n.children.length > 0 && <Tree nodes={n.children} active={active} depth={depth + 1} />}
        </div>
      ))}
    </>
  );
}

// ---------- recordings (replay) ----------
type TreeEvent = { kind: "tree"; baseline: boolean; files: [string, boolean, number | null, number | null][] };
type DecisionEvent = { kind: "decision"; approved: boolean };
type RecEvent = AgentEvent | TreeEvent | DecisionEvent;
interface Recording { version: number; goal: string; workspace: string; started_at: number; events: { t: number; e: RecEvent }[] }
interface RecordingInfo { file: string; goal: string; workspace: string; started_at: number; duration_ms: number; actions: number; outcome: string }

const MAX_REPLAY_GAP_MS = 1800; // long API waits are skipped during replay
const SPEEDS = [1, 2, 4];

const treeToEvent = (m: Map<string, FileInfo>, baseline: boolean): TreeEvent => ({
  kind: "tree",
  baseline,
  files: [...m.entries()].map(([p, f]) => [p, f.isDir, f.size ?? null, f.mtime ?? null]),
});
const eventToTree = (e: TreeEvent) =>
  new Map<string, FileInfo>(e.files.map(([p, isDir, size, mtime]) => [p, { isDir, size: size ?? undefined, mtime: mtime ?? undefined }]));

function fmtDate(ms: number) {
  try { return new Date(ms).toLocaleString(); } catch { return String(ms); }
}

function ReplayList({ onPick, onClose }: { onPick: (file: string) => void; onClose: () => void }) {
  const [items, setItems] = useState<RecordingInfo[] | null>(null);
  const [err, setErr] = useState("");
  useEffect(() => {
    invoke<RecordingInfo[]>("list_recordings_tool").then(setItems).catch((e) => setErr(String(e)));
  }, []);
  return (
    <div className="lr-overlay" onClick={onClose}>
      <div className="lr-shell lr-list-shell" onClick={(ev) => ev.stopPropagation()}>
        <header className="lr-header">
          <div className="lr-head-text">
            <div className="lr-goal">▶ Replays</div>
            <div className="lr-phase">Every run is recorded. Replay one like a video — no AI calls, works offline.</div>
          </div>
          <button className="lr-close" onClick={onClose}>✕</button>
        </header>
        <div className="lr-list">
          {err && <div className="lr-card-err">{err}</div>}
          {items === null && !err && <div className="lr-muted">Loading…</div>}
          {items && items.length === 0 && <div className="lr-muted">No recordings yet. Run the agent once and it will appear here.</div>}
          {items?.map((r) => (
            <button key={r.file} className="lr-rec" onClick={() => onPick(r.file)}>
              <span className={`lr-rec-dot lr-rec-${r.outcome}`} />
              <span className="lr-rec-main">
                <span className="lr-rec-goal">{r.goal || "(no goal)"}</span>
                <span className="lr-muted">{fmtDate(r.started_at)} · {r.actions} action{r.actions === 1 ? "" : "s"} · {(r.duration_ms / 1000).toFixed(1)}s · {r.outcome}</span>
              </span>
              <span className="lr-rec-play">▶</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

// ---------- run report (📄 Export report): one self-contained HTML page ----------
interface ReportData {
  goal: string; startedAt: number; seconds: number; outcome: "done" | "stopped"; answer: string;
  cards: Card[]; plan: any; files: [string, FileMark][]; decisions: Record<string, boolean>; modelNote: string | null; replay: boolean;
}

const esc = (v: unknown) =>
  String(v ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c] as string));

function buildReport(d: ReportData): string {
  const plan: { text: string; done: boolean }[] = Array.isArray(d.plan) ? d.plan.filter((x: any) => x && typeof x.text === "string") : [];
  const checks = d.cards.filter((c) => c.name === "claim_check" || c.name === "checklist_check" || c.name === "plan_check");
  const tiers = { safe: 0, mutating: 0, privileged: 0 } as Record<string, number>;
  d.cards.forEach((c) => { if (c.tier in tiers) tiers[c.tier]++; });
  const approvals = Object.values(d.decisions);
  const stepRows = d.cards.map((c, i) => {
    const decided = d.decisions[c.id];
    const result = checks.includes(c) ? (c.status === "ok" ? "✓ passed" : "! caught") : c.status === "ok" ? "✓ ok" : c.status === "declined" ? "⊘ declined" : c.status === "waiting" ? "⏸ waiting" : c.status === "running" ? "…" : "✕ error";
    return `<tr class="${c.status}"><td>${i + 1}</td><td>${esc(describe(c.name, c.args))}<div class="tool">${esc(c.name)}</div>${c.status === "error" || checks.includes(c) ? `<div class="note">${esc(c.preview.slice(0, 300))}</div>` : ""
      }</td><td><span class="tier ${esc(c.tier)}">${esc(TIER_LABEL[c.tier] ?? c.tier)}</span>${decided === undefined ? "" : `<div class="note">${decided ? "you approved" : "you declined"}</div>`
      }</td><td>${result}</td><td>${c.durationMs !== undefined ? esc(fmtMs(c.durationMs)) : ""}</td></tr>`;
  }).join("");
  const fileRows = d.files.map(([p, m]) => `<li><span class="mark ${esc(m)}">${esc(m)}</span> ${esc(p)}</li>`).join("");
  return `<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>AI CoWorker run report</title>
<style>
  :root { --ink:#1d1b2e; --muted:#6b6785; --line:#e4e1f0; --violet:#6d28d9; --green:#15803d; --amber:#b45309; --red:#b91c1c; --bg:#fbfaff; }
  * { box-sizing: border-box; }
  body { margin: 0; background: var(--bg); color: var(--ink); font: 14px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
  main { max-width: 920px; margin: 0 auto; padding: 32px 20px 48px; }
  .brand { color: var(--violet); font-weight: 700; letter-spacing: .04em; font-size: 12px; text-transform: uppercase; }
  h1 { font-size: 22px; margin: 4px 0 4px; }
  h2 { font-size: 15px; margin: 28px 0 10px; padding-bottom: 6px; border-bottom: 1px solid var(--line); }
  .meta { color: var(--muted); }
  .badge { display: inline-block; padding: 1px 8px; border-radius: 99px; font-size: 12px; font-weight: 600; }
  .badge.done { background: #dcfce7; color: var(--green); } .badge.stopped { background: #fee2e2; color: var(--red); }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(120px, 1fr)); gap: 10px; margin-top: 16px; }
  .stats div { background: #fff; border: 1px solid var(--line); border-radius: 10px; padding: 10px 12px; }
  .stats b { display: block; font-size: 20px; } .stats span { color: var(--muted); font-size: 12px; }
  .box { background: #fff; border: 1px solid var(--line); border-radius: 10px; padding: 12px 14px; }
  pre { white-space: pre-wrap; word-break: break-word; margin: 0; font: 13px/1.5 ui-monospace, Consolas, monospace; }
  ul { margin: 0; padding-left: 0; list-style: none; } li { padding: 3px 0; }
  .tick { display: inline-block; width: 18px; color: var(--green); font-weight: 700; } .open { color: var(--red); }
  table { width: 100%; border-collapse: collapse; background: #fff; border: 1px solid var(--line); border-radius: 10px; overflow: hidden; }
  th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid var(--line); vertical-align: top; }
  th { font-size: 12px; color: var(--muted); font-weight: 600; background: #f6f4fd; }
  tr.error td:nth-child(4) { color: var(--red); } tr.ok td:nth-child(4) { color: var(--green); } tr.declined td:nth-child(4) { color: var(--amber); }
  .tool { color: var(--muted); font: 11px ui-monospace, Consolas, monospace; } .note { color: var(--muted); font-size: 12px; margin-top: 2px; }
  .tier { font-size: 11.5px; padding: 1px 7px; border-radius: 99px; white-space: nowrap; }
  .tier.safe { background: #dcfce7; color: var(--green); } .tier.mutating { background: #dbeafe; color: #1d4ed8; } .tier.privileged { background: #fef3c7; color: var(--amber); }
  .mark { font-size: 11.5px; padding: 1px 7px; border-radius: 99px; } .mark.new { background: #dcfce7; color: var(--green); }
  .mark.edited { background: #dbeafe; color: #1d4ed8; } .mark.deleted { background: #fee2e2; color: var(--red); }
  footer { margin-top: 32px; color: var(--muted); font-size: 12px; }
  @media print { body { background: #fff; } main { padding: 0; } }
       ${MARKDOWN_REPORT_CSS}
</style></head>
<body><main>
  <div class="brand">AI CoWorker · run report</div>
  <h1>${esc(d.goal || "Agent run")}</h1>
  <div class="meta"><span class="badge ${d.outcome}">${d.outcome === "done" ? "Finished" : "Stopped"}</span>
    · ${esc(fmtDate(d.startedAt))} · ${d.seconds.toFixed(1)} s${d.replay ? " · from a saved replay" : ""}</div>
  <div class="stats">
    <div><b>${d.cards.length}</b><span>actions</span></div>
    <div><b>${tiers.safe}</b><span>read-only</span></div>
    <div><b>${tiers.mutating}</b><span>changed files (snapshotted)</span></div>
    <div><b>${tiers.privileged}</b><span>needed approval</span></div>
    <div><b>${d.files.length}</b><span>files changed</span></div>
  </div>
  ${d.modelNote ? `<h2>Model</h2><div class="box">${esc(d.modelNote)}</div>` : ""}
  ${plan.length ? `<h2>Plan (${plan.filter((x) => x.done).length}/${plan.length} done)</h2><div class="box"><ul>${plan
      .map((x) => `<li><span class="tick ${x.done ? "" : "open"}">${x.done ? "✓" : "○"}</span>${esc(x.text)}</li>`).join("")}</ul></div>` : ""}
  ${checks.length ? `<h2>Backend checks</h2><div class="box"><ul>${checks
      .map((c) => `<li><span class="tick ${c.status === "ok" ? "" : "open"}">${c.status === "ok" ? "✓" : "!"}</span>${esc(c.preview)}</li>`).join("")}</ul></div>` : ""}
  <h2>Steps</h2>
  <table><thead><tr><th>#</th><th>Action</th><th>Tier (enforced by the backend)</th><th>Result</th><th>Time</th></tr></thead><tbody>${stepRows || `<tr><td colspan="5">No tool calls.</td></tr>`}</tbody></table>
  <h2>Files changed</h2>
  <div class="box">${fileRows ? `<ul>${fileRows}</ul>` : "No files changed."}</div>
  <h2>${d.outcome === "done" ? "Final answer" : "Why it stopped"}</h2>
       <div class="box">${d.outcome === "done" ? markdownToHtml(d.answer) : `<pre>${esc(d.answer)}</pre>`}</div>
  <footer>Generated by AI CoWorker. Tiers are enforced in the backend (safe = read-only, mutating = snapshotted before it runs, privileged = blocked until approved). The full audit log is in the workspace's .aicoworker folder.${approvals.length ? ` Approvals in this run: ${approvals.filter(Boolean).length} approved, ${approvals.filter((x) => !x).length} declined.` : ""}</footer>
</main></body></html>`;
}

function friendlyReason(r: string): string {
  if (/429|rate.?limit|Too Many Requests/i.test(r))
    return "The AI provider's free-tier rate limit was reached (HTTP 429), so the run paused here.\nEverything done so far is saved (see the steps on the left). Wait a bit, or try again tomorrow if the daily quota is used up.";
  if (/Model call failed|error sending request|dns|timed out/i.test(r))
    return "Couldn't reach the AI provider (network problem). Check your internet connection and run it again.\n\nDetails: " + r;
  return r;
}

// ---------- main ----------
export default function LiveRun(props: Props) {
  const { goal, workspaceRoot, running, finalAnswer, status, pendingConfirmation, onApprove, onDecline } = props;
  const [open, setOpen] = useState(false);
  const [cards, setCards] = useState<Card[]>([]);
  const [phase, setPhase] = useState<Phase>("idle");
  const [steps, setSteps] = useState(0);
  const [stopReason, setStopReason] = useState<string | null>(null);
  const [answerText, setAnswerText] = useState<string | null>(null);
  const [modelNote, setModelNote] = useState<string | null>(null);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [follow, setFollow] = useState(true);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [endedAt, setEndedAt] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now());
  const [baseline, setBaseline] = useState<Map<string, FileInfo>>(new Map());
  const [current, setCurrent] = useState<Map<string, FileInfo>>(new Map());
  const [undo, setUndo] = useState<"idle" | "confirm" | "working" | "undone" | "error">("idle");
  const [undoMsg, setUndoMsg] = useState("");
  const [reportMsg, setReportMsg] = useState("");

  // replay
  const [mode, setMode] = useState<"live" | "replay">("live");
  const [showList, setShowList] = useState(false);
  const [replay, setReplay] = useState<Recording | null>(null);
  const [replayIdx, setReplayIdx] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [decisions, setDecisions] = useState<Record<string, boolean>>({});

  const scanSeq = useRef(0);
  const wasRunning = useRef(false);
  const timelineEnd = useRef<HTMLDivElement>(null);
  const followRef = useRef(follow);
  followRef.current = follow;
  const modeRef = useRef(mode);
  modeRef.current = mode;
  const rec = useRef<Recording | null>(null); // the live run being recorded
  const waitingCard = useRef<string | null>(null);

  const record = (e: RecEvent) => {
    if (rec.current && modeRef.current === "live") rec.current.events.push({ t: Date.now() - rec.current.started_at, e });
  };

  function resetView() {
    setUndo("idle"); setUndoMsg(""); setReportMsg(""); setAnswerText(null); setDecisions({}); setModelNote(null);
    setCards([]); setSteps(0); setStopReason(null); setFocusId(null); setFollow(true);
    setBaseline(new Map()); setCurrent(new Map());
  }

  // One place that turns an event into screen state — used by BOTH the live stream and replay.
  function apply(e: RecEvent) {
    switch (e.kind) {
      case "thinking":
        setSteps((s) => s + 1);
        setPhase("thinking");
        break;
      case "tool_started":
        setPhase("acting");
        setCards((cs) => {
          const i = cs.findIndex((c) => c.id === e.call_id);
          const card: Card = { id: e.call_id, name: e.name, args: e.arguments, tier: e.tier, status: "running", preview: "" };
          if (i >= 0) { const copy = [...cs]; copy[i] = card; return copy; } // resumed after approval
          return [...cs, card];
        });
        setFocusId((f) => (followRef.current ? e.call_id : f));
        break;
      case "tool_finished":
        setCards((cs) => cs.map((c) => (c.id === e.call_id ? { ...c, status: e.ok ? "ok" : "error", preview: e.preview, durationMs: e.duration_ms } : c)));
        break;
      case "needs_confirmation":
        setPhase("waiting");
        setCards((cs) => {
          const copy = [...cs];
          for (let i = copy.length - 1; i >= 0; i--) {
            if (copy[i].name === e.name && copy[i].status === "running") {
              copy[i] = { ...copy[i], status: "waiting" };
              waitingCard.current = copy[i].id;
              break;
            }
          }
          return copy;
        });
        break;
      case "answer":
        setPhase("done");
        setAnswerText(e.text);
        break;
      case "stopped":
        setPhase("stopped");
        setStopReason(e.reason);
        break;
      case "model_switched":
        setModelNote(`⚡ Daily limit reached on ${e.from} — continuing on backup model ${e.to}`);
        break;
      case "tree": {
        const m = eventToTree(e);
        if (e.baseline) setBaseline(m);
        setCurrent(m);
        break;
      }
      case "decision": {
        const id = waitingCard.current;
        if (id) setDecisions((d) => ({ ...d, [id]: e.approved }));
        if (!e.approved) setCards((cs) => cs.map((c) => (c.status === "waiting" ? { ...c, status: "declined" } : c)));
        break;
      }
    }
  }

  const refreshTree = async (asBaseline = false) => {
    if (!workspaceRoot || modeRef.current === "replay") return;
    const seq = ++scanSeq.current;
    const m = await scanWorkspace(workspaceRoot);
    if (seq !== scanSeq.current) return; // a newer scan started
    record(treeToEvent(m, asBaseline));
    if (asBaseline) setBaseline(m);
    setCurrent(m);
  };

  // ---- Undo / Redo the whole run, using the shadow-git snapshots ----
  // Every mutating/privileged action snapshots the workspace BEFORE it runs, so the
  // oldest snapshot taken during this run = the workspace exactly as it was before the run.
  async function undoRun() {
    setUndo("working");
    try {
      const snaps = await invoke<SnapshotRecord[]>("list_snapshots_tool", { workspaceRoot }); // newest first
      const since = Math.floor((startedAt ?? Date.now()) / 1000);
      const inRun = snaps.filter((x) => x.timestamp_unix >= since && !/^(before )?restore_snapshot/.test(x.message));
      const first = inRun[inRun.length - 1];
      if (!first) { setUndo("error"); setUndoMsg("This run didn't change any files, so there's nothing to undo."); return; }
      await invoke("restore_snapshot_tool", { workspaceRoot, commitHash: first.commit_hash, confirmed: true });
      const m = await scanWorkspace(workspaceRoot);
      setBaseline(m); setCurrent(m);
      setUndo("undone");
      setUndoMsg("The workspace is back to exactly how it was before this run. Files the run created are gone and anything it deleted is back.");
    } catch (err) {
      setUndo("error"); setUndoMsg(`Undo failed: ${err}`);
    }
  }

  async function redoRun() {
    setUndo("working");
    try {
      const snaps = await invoke<SnapshotRecord[]>("list_snapshots_tool", { workspaceRoot });
      const r = snaps.find((x) => x.message.startsWith("before restore_snapshot"));
      if (!r) { setUndo("error"); setUndoMsg("Couldn't find the state to redo."); return; }
      await invoke("restore_snapshot_tool", { workspaceRoot, commitHash: r.commit_hash, confirmed: true });
      await refreshTree();
      setUndo("idle"); setUndoMsg("Redone: the run's changes are back.");
    } catch (err) {
      setUndo("error"); setUndoMsg(`Redo failed: ${err}`);
    }
  }

  // approve / decline also go into the recording
  const approve = () => { record({ kind: "decision", approved: true }); if (waitingCard.current) setDecisions((d) => ({ ...d, [waitingCard.current!]: true })); onApprove(); };
  const decline = () => { record({ kind: "decision", approved: false }); onDecline(); };

  // a live run starts / ends (layout effect = before any event can land)
  useLayoutEffect(() => {
    if (running && !wasRunning.current) {
      setMode("live"); setPlaying(false); setReplay(null); setShowList(false);
      resetView();
      const t0 = Date.now();
      rec.current = { version: 1, goal, workspace: workspaceRoot, started_at: t0, events: [] };
      setPhase("thinking"); setStartedAt(t0); setEndedAt(null); setOpen(true);
      refreshTree(true);
    }
    if (!running && wasRunning.current) {
      setEndedAt(Date.now());
      // declined (or errored) while waiting
      setCards((cs) => cs.map((c) => (c.status === "waiting" ? { ...c, status: "declined" } : c.status === "running" ? { ...c, status: "error" } : c)));
      setPhase((p) => (p === "done" ? p : "stopped"));
      refreshTree();
      // save the recording once the final file scan has landed
      const r = rec.current;
      rec.current = r; // keep recording the final tree
      setTimeout(() => {
        if (r && r.events.length > 0) invoke("save_recording_tool", { content: JSON.stringify(r) }).catch(() => { });
        if (rec.current === r) rec.current = null;
      }, 2500);
    }
    wasRunning.current = running;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [running]);

  // the live event stream
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listen<AgentEvent>("agent-event", ({ payload: e }) => {
      if (modeRef.current === "replay") return; // a replay owns the screen
      record(e);
      apply(e);
      if (e.kind === "tool_finished" || e.kind === "answer") refreshTree();
      if (e.kind === "answer" || e.kind === "stopped") setEndedAt(Date.now());
    }).then((u) => { if (cancelled) u(); else unlisten = u; });
    return () => { cancelled = true; unlisten?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceRoot]);

  // ---- replay player ----
  async function startReplay(file: string) {
    try {
      const r = JSON.parse(await invoke<string>("load_recording_tool", { file })) as Recording;
      setShowList(false);
      setMode("replay");
      resetView();
      waitingCard.current = null;
      setReplay(r); setReplayIdx(0); setPhase("thinking"); setStartedAt(null); setEndedAt(null);
      setPlaying(true); setOpen(true);
    } catch (err) {
      alert(`Couldn't load that recording: ${err}`);
    }
  }

  function restartReplay() {
    if (!replay) return;
    resetView(); waitingCard.current = null;
    setReplayIdx(0); setPhase("thinking"); setPlaying(true);
  }

  function exitReplay() {
    setPlaying(false); setReplay(null); setMode("live"); resetView(); setPhase("idle"); setOpen(false);
  }

  useEffect(() => {
    if (mode !== "replay" || !playing || !replay) return;
    if (replayIdx >= replay.events.length) { setPlaying(false); return; }
    const prevT = replayIdx > 0 ? replay.events[replayIdx - 1].t : 0;
    const gap = Math.min(Math.max(replay.events[replayIdx].t - prevT, 0), MAX_REPLAY_GAP_MS) / speed;
    const timer = setTimeout(() => {
      apply(replay.events[replayIdx].e);
      setReplayIdx((i) => i + 1);
    }, gap);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, playing, replayIdx, speed, replay]);

  // ticking clock while running
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(t);
  }, [running]);

  useEffect(() => { timelineEnd.current?.scrollIntoView({ behavior: "smooth", block: "end" }); }, [cards.length, phase]);

  // which files changed during this run
  const { marks, ghosts } = useMemo(() => {
    const marks = new Map<string, FileMark>();
    const ghosts = new Map<string, FileInfo>();
    if (baseline.size === 0 && current.size === 0) return { marks, ghosts };
    for (const [p, info] of current) {
      const b = baseline.get(p);
      if (!b) marks.set(p, "new");
      else if (!info.isDir && (b.size !== info.size || b.mtime !== info.mtime)) marks.set(p, "edited");
    }
    for (const [p, info] of baseline) {
      if (!current.has(p)) { marks.set(p, "deleted"); ghosts.set(p, info); }
    }
    return { marks, ghosts };
  }, [baseline, current]);

  const isReplay = mode === "replay" && replay !== null;
  const replayDone = isReplay && replayIdx >= replay.events.length;
  const active = isReplay ? !replayDone : running; // "is the run in progress" for the screen
  const tree = useMemo(() => buildTree(current, marks, ghosts), [current, marks, ghosts]);
  const changedFiles = [...marks.entries()].filter(([p]) => !(current.get(p)?.isDir ?? ghosts.get(p)?.isDir));
  const activeCard = cards.find((c) => c.id === focusId) ?? cards[cards.length - 1];
  const plan = [...cards].reverse().find((c) => c.name === "update_checklist" && c.status === "ok")?.args?.items;
  const activePaths = new Set(cards.filter((c) => c.status === "running" || c.status === "waiting").flatMap((c) => pathsOf(c.name, c.args)));
  const elapsed = isReplay
    ? (replayIdx > 0 ? replay.events[replayIdx - 1].t : 0) / 1000
    : startedAt ? ((endedAt ?? now) - startedAt) / 1000 : 0;
  const shownGoal = isReplay ? replay.goal : goal;
  const shownAnswer = answerText ?? finalAnswer ?? "";
  const progress = isReplay && replay.events.length ? (replayIdx / replay.events.length) * 100 : 0;

  const changedSomething = cards.some((c) => c.status === "ok" && c.tier !== "safe");
  const undoBar = !isReplay && !running && changedSomething ? (
    <div className="lr-undo">
      {undo === "idle" && (
        <button className="lr-btn lr-btn-undo" onClick={() => setUndo("confirm")}>↶ Undo this run</button>
      )}
      {undo === "confirm" && (
        <>
          <span>Put the workspace back to how it was before this run?</span>
          <button className="lr-btn lr-btn-ok" onClick={undoRun}>Yes, undo</button>
          <button className="lr-btn lr-btn-no" onClick={() => setUndo("idle")}>Cancel</button>
        </>
      )}
      {undo === "working" && <span className="lr-muted"><span className="lr-spinner" /> restoring snapshot…</span>}
      {undo === "undone" && (
        <button className="lr-btn lr-btn-undo" onClick={redoRun}>↷ Redo</button>
      )}
      {undoMsg && <div className="lr-undo-msg">{undoMsg}</div>}
    </div>
  ) : null;

  async function exportReport() {
    const start = isReplay ? replay!.started_at : rec.current?.started_at ?? startedAt ?? Date.now();
    const html = buildReport({
      goal: shownGoal, startedAt: start, seconds: elapsed, outcome: phase === "done" ? "done" : "stopped",
      answer: phase === "done" ? shownAnswer : friendlyReason(stopReason ?? status),
      cards, plan, files: changedFiles, decisions, modelNote, replay: isReplay,
    });
    try {
      const path = await invoke<string>("save_report_tool", { html, startedAt: start });
      setReportMsg(`Saved and opened: ${path}`);
    } catch (err) {
      setReportMsg(`Couldn't save the report: ${err}`);
    }
  }
  const reportBar = !active && cards.length > 0 ? (
    <div className="lr-report">
      <button className="lr-btn lr-btn-undo" onClick={exportReport}>📄 Export report</button>
      {reportMsg && <div className="lr-undo-msg">{reportMsg}</div>}
    </div>
  ) : null;

  // what goes where Approve/Decline would be: real buttons live, the recorded choice in a replay
  const confirmArea = (c: Card) => {
    if (isReplay) {
      const d = decisions[c.id];
      return d === undefined ? null : <span className={`lr-decision ${d ? "yes" : "no"}`}>{d ? "✓ You approved" : "⊘ You declined"}</span>;
    }
    if (c.status === "waiting" && pendingConfirmation)
      return (
        <div className="lr-confirm" onClick={(ev) => ev.stopPropagation()}>
          <button className="lr-btn lr-btn-ok" onClick={approve}>Approve</button>
          <button className="lr-btn lr-btn-no" onClick={decline}>Decline</button>
        </div>
      );
    if (decisions[c.id] !== undefined) return <span className={`lr-decision ${decisions[c.id] ? "yes" : "no"}`}>{decisions[c.id] ? "✓ You approved" : "⊘ You declined"}</span>;
    return null;
  };

  const phaseText: Record<Phase, string> = {
    idle: "Ready",
    thinking: `Thinking · step ${steps || 1}`,
    acting: activeCard ? describe(activeCard.name, activeCard.args) : "Working",
    waiting: isReplay ? "Waiting for approval" : "Waiting for your approval",
    done: "Done",
    stopped: "Stopped",
  };

  if (showList) return <ReplayList onPick={startReplay} onClose={() => setShowList(false)} />;

  if (!open) {
    return (
      <div className="lr-pills">
        <button className="lr-pill lr-pill-replay" onClick={() => setShowList(true)} title="Replay a recorded run">▶ Replays</button>
        {(cards.length > 0 || running) && (
          <button className={`lr-pill lr-phase-${phase}`} onClick={() => setOpen(true)}>
            <span className="lr-orb-mini" /> Live view {running ? `· ${phaseText[phase]}` : ""}
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="lr-overlay">
      <div className={`lr-shell lr-phase-${phase}`}>
        {/* header */}
        <header className="lr-header">
          <div className={`lr-orb lr-orb-${phase}`}><span /></div>
          <div className="lr-head-text">
            <div className="lr-goal" title={shownGoal}>
              {isReplay && <span className="lr-replay-badge">REPLAY</span>}
              {shownGoal || "Agent run"}
            </div>
            <div className="lr-phase">
              {phaseText[phase]}
              {isReplay && <span className="lr-muted"> · recorded {fmtDate(replay.started_at)}</span>}
            </div>
          </div>
          {isReplay && (
            <div className="lr-player">
              {replayDone ? (
                <button className="lr-btn lr-btn-undo" onClick={restartReplay}>↺ Replay again</button>
              ) : (
                <button className="lr-btn lr-btn-undo" onClick={() => setPlaying((p) => !p)}>{playing ? "⏸ Pause" : "▶ Play"}</button>
              )}
              {SPEEDS.map((s) => (
                <button key={s} className={`lr-speed ${speed === s ? "on" : ""}`} onClick={() => setSpeed(s)}>{s}×</button>
              ))}
            </div>
          )}
          <div className="lr-stats">
            <div><b>{elapsed.toFixed(1)}s</b><span>time</span></div>
            <div><b>{steps}</b><span>steps</span></div>
            <div><b>{cards.length}</b><span>actions</span></div>
            <div><b>{changedFiles.length}</b><span>files changed</span></div>
          </div>
          <button className="lr-close" onClick={isReplay ? exitReplay : () => setOpen(false)} title={isReplay ? "Exit replay" : "Hide live view"}>✕</button>
        </header>
        {isReplay && <div className="lr-progress">
          {modelNote && <div className="lr-model-note">{modelNote}</div>}
          <div style={{ width: `${progress}%` }} /></div>}

        <div className="lr-body">
          {/* timeline */}
          <section className="lr-col lr-timeline">
            <div className="lr-col-title">Steps</div>
            {Array.isArray(plan) && plan.length > 0 && <Checklist items={plan} pinned />}
            {cards.map((c, i) => (
              <div
                key={c.id}
                className={`lr-card lr-card-${c.status} ${activeCard?.id === c.id ? "lr-card-focus" : ""}`}
                onClick={() => { setFocusId(c.id); setFollow(false); }}
              >
                <div className="lr-card-top">
                  <span className="lr-step-num">{i + 1}</span>
                  <span className="lr-card-icon">{icon(c.name)}</span>
                  <span className="lr-card-title">{describe(c.name, c.args)}</span>
                  <span className={`lr-state lr-state-${c.status}`}>
                    {c.status === "running" ? <span className="lr-spinner" /> : c.status === "ok" ? "✓" : c.status === "waiting" ? "⏸" : c.status === "declined" ? "⊘" : "✕"}
                  </span>
                </div>
                <div className="lr-card-meta">
                  <span className={`lr-tier lr-tier-${c.tier}`}>{TIER_LABEL[c.tier] ?? c.tier}</span>
                  {c.durationMs !== undefined && <span className="lr-muted">{fmtMs(c.durationMs)}</span>}
                </div>
                {c.status === "error" && c.preview && <div className="lr-card-err">{c.preview.slice(0, 160)}</div>}
                {confirmArea(c)}
              </div>
            ))}
            {phase === "thinking" && active && (
              <div className="lr-card lr-thinking"><span className="lr-shimmer">Thinking about the next step…</span></div>
            )}
            <div ref={timelineEnd} />
          </section>

          {/* stage */}
          <section className="lr-col lr-stage">
            <div className="lr-col-title">
              {isReplay ? "Replay" : "Live"}
              {!follow && <button className="lr-follow" onClick={() => { setFollow(true); setFocusId(null); }}>follow live ⟳</button>}
            </div>
            <div className="lr-stage-inner">
              {phase === "done" && follow ? (
                <div className="lr-done">
                  <div className="lr-done-check">✓</div>
                  <div className="lr-done-title">Finished in {elapsed.toFixed(1)}s</div>
                  <Markdown className="lr-answer" text={shownAnswer} />
                  {changedFiles.length > 0 && (
                    <div className="lr-chips">
                      {changedFiles.map(([p, m]) => <span key={p} className={`lr-chip lr-badge-${m}`}>{m} · {p}</span>)}
                    </div>
                  )}
                  {undoBar}
                  {reportBar}
                </div>
              ) : phase === "stopped" && follow && !active ? (
                <div className="lr-done lr-stopped">
                  <div className="lr-done-check">!</div>
                  <div className="lr-done-title">Run stopped</div>
                  <pre className="lr-answer">{friendlyReason(stopReason ?? status)}</pre>
                  {undoBar}
                  {reportBar}
                </div>
              ) : activeCard ? (
                <>
                  <div className="lr-stage-head">
                    <span>{icon(activeCard.name)} {describe(activeCard.name, activeCard.args)}</span>
                    <span className={`lr-tier lr-tier-${activeCard.tier}`}>{TIER_LABEL[activeCard.tier] ?? activeCard.tier}</span>
                  </div>
                  <Stage card={activeCard} />
                  {activeCard.status === "waiting" && !isReplay && pendingConfirmation && (
                    <div className="lr-approval">
                      <div><b>AI CoWorker wants to do something that needs your OK.</b><br />The backend blocked it until you decide.</div>
                      <div className="lr-confirm">
                        <button className="lr-btn lr-btn-ok" onClick={approve}>Approve</button>
                        <button className="lr-btn lr-btn-no" onClick={decline}>Decline</button>
                      </div>
                    </div>
                  )}
                  {activeCard.status === "waiting" && isReplay && (
                    <div className="lr-approval"><div><b>Paused for approval.</b> The backend blocked this action until the user decided.</div></div>
                  )}
                </>
              ) : (
                <div className="lr-empty"><div className={`lr-orb lr-orb-big lr-orb-${phase}`}><span /></div><div>Planning…</div></div>
              )}
            </div>
          </section>

          {/* files */}
          <section className="lr-col lr-files">
            <div className="lr-col-title">Workspace</div>
            <div className="lr-tree">
              {tree.length ? <Tree nodes={tree} active={activePaths} /> : <div className="lr-muted">empty folder</div>}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}