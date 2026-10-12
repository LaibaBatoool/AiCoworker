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

// ---------- types ----------
type AgentEvent =
  | { kind: "thinking"; step: number }
  | { kind: "tool_started"; call_id: string; name: string; arguments: any; tier: string }
  | { kind: "tool_finished"; call_id: string; name: string; ok: boolean; preview: string; duration_ms: number }
  | { kind: "needs_confirmation"; name: string; arguments: any }
  | { kind: "answer"; text: string }
  | { kind: "stopped"; reason: string };

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
    default:
      if (name.startsWith("mcp__")) {
        const [, server, tool] = name.split("__");
        return `External tool ${tool} (${server})`;
      }
      return name.replace(/_/g, " ") + (a?.relative_path ? ` ${p}` : "");
  }
}

function icon(name: string): string {
  if (name.startsWith("mcp__")) return "🔌";
  const m: Record<string, string> = {
    write_file: "✍️", edit_file: "🩹", read_file: "📖", create_docx: "📝", create_xlsx: "📊",
    read_spreadsheet: "📊", list_directory: "📂", search_files: "🔎", get_file_metadata: "ℹ️",
    create_directory: "📁", delete_file: "🗑️", move_rename: "↪️", execute_command: "⌨️",
    fetch_url: "🌐", convert_to_pdf: "📄", git_commit: "🌿", git_diff: "🌿", git_status: "🌿", git_log: "🌿",
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

function Stage({ card }: { card: Card }) {
  const a = card.args ?? {};
  switch (card.name) {
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
  const [focusId, setFocusId] = useState<string | null>(null);
  const [follow, setFollow] = useState(true);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [endedAt, setEndedAt] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now());
  const [baseline, setBaseline] = useState<Map<string, FileInfo>>(new Map());
  const [current, setCurrent] = useState<Map<string, FileInfo>>(new Map());
  const scanSeq = useRef(0);
  const wasRunning = useRef(false);
  const timelineEnd = useRef<HTMLDivElement>(null);
  const followRef = useRef(follow);
  followRef.current = follow;

  const refreshTree = async (asBaseline = false) => {
    if (!workspaceRoot) return;
    const seq = ++scanSeq.current;
    const m = await scanWorkspace(workspaceRoot);
    if (seq !== scanSeq.current) return; // a newer scan started
    if (asBaseline) setBaseline(m);
    setCurrent(m);
  };

  // a new run starts: reset everything (layout effect = before any event can land)
  useLayoutEffect(() => {
    if (running && !wasRunning.current) {
      setCards([]); setSteps(0); setStopReason(null); setFocusId(null); setFollow(true);
      setPhase("thinking"); setStartedAt(Date.now()); setEndedAt(null); setOpen(true);
      refreshTree(true);
    }
    if (!running && wasRunning.current) {
      setEndedAt(Date.now());
      // declined (or errored) while waiting
      setCards((cs) => cs.map((c) => (c.status === "waiting" ? { ...c, status: "declined" } : c.status === "running" ? { ...c, status: "error" } : c)));
      setPhase((p) => (p === "done" ? p : "stopped"));
      refreshTree();
    }
    wasRunning.current = running;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [running]);

  // the live event stream
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listen<AgentEvent>("agent-event", ({ payload: e }) => {
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
          refreshTree();
          break;
        case "needs_confirmation":
          setPhase("waiting");
          setCards((cs) => {
            const copy = [...cs];
            for (let i = copy.length - 1; i >= 0; i--) {
              if (copy[i].name === e.name && copy[i].status === "running") { copy[i] = { ...copy[i], status: "waiting" }; break; }
            }
            return copy;
          });
          break;
        case "answer":
          setPhase("done");
          setEndedAt(Date.now());
          refreshTree();
          break;
        case "stopped":
          setPhase("stopped");
          setStopReason(e.reason);
          setEndedAt(Date.now());
          break;
      }
    }).then((u) => { if (cancelled) u(); else unlisten = u; });
    return () => { cancelled = true; unlisten?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceRoot]);

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

  const tree = useMemo(() => buildTree(current, marks, ghosts), [current, marks, ghosts]);
  const changedFiles = [...marks.entries()].filter(([p]) => !(current.get(p)?.isDir ?? ghosts.get(p)?.isDir));
  const activeCard = cards.find((c) => c.id === focusId) ?? cards[cards.length - 1];
  const activePaths = new Set(cards.filter((c) => c.status === "running" || c.status === "waiting").flatMap((c) => pathsOf(c.name, c.args)));
  const elapsed = startedAt ? ((endedAt ?? now) - startedAt) / 1000 : 0;

  const phaseText: Record<Phase, string> = {
    idle: "Ready",
    thinking: `Thinking · step ${steps || 1}`,
    acting: activeCard ? describe(activeCard.name, activeCard.args) : "Working",
    waiting: "Waiting for your approval",
    done: "Done",
    stopped: "Stopped",
  };

  if (!open) {
    return cards.length > 0 || running ? (
      <button className={`lr-pill lr-phase-${phase}`} onClick={() => setOpen(true)}>
        <span className="lr-orb-mini" /> Live view {running ? `· ${phaseText[phase]}` : ""}
      </button>
    ) : null;
  }

  return (
    <div className="lr-overlay">
      <div className={`lr-shell lr-phase-${phase}`}>
        {/* header */}
        <header className="lr-header">
          <div className={`lr-orb lr-orb-${phase}`}><span /></div>
          <div className="lr-head-text">
            <div className="lr-goal" title={goal}>{goal || "Agent run"}</div>
            <div className="lr-phase">{phaseText[phase]}</div>
          </div>
          <div className="lr-stats">
            <div><b>{elapsed.toFixed(1)}s</b><span>time</span></div>
            <div><b>{steps}</b><span>steps</span></div>
            <div><b>{cards.length}</b><span>actions</span></div>
            <div><b>{changedFiles.length}</b><span>files changed</span></div>
          </div>
          <button className="lr-close" onClick={() => setOpen(false)} title="Hide live view">✕</button>
        </header>

        <div className="lr-body">
          {/* timeline */}
          <section className="lr-col lr-timeline">
            <div className="lr-col-title">Steps</div>
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
                {c.status === "waiting" && pendingConfirmation && (
                  <div className="lr-confirm" onClick={(ev) => ev.stopPropagation()}>
                    <button className="lr-btn lr-btn-ok" onClick={onApprove}>Approve</button>
                    <button className="lr-btn lr-btn-no" onClick={onDecline}>Decline</button>
                  </div>
                )}
              </div>
            ))}
            {phase === "thinking" && running && (
              <div className="lr-card lr-thinking"><span className="lr-shimmer">Thinking about the next step…</span></div>
            )}
            <div ref={timelineEnd} />
          </section>

          {/* stage */}
          <section className="lr-col lr-stage">
            <div className="lr-col-title">
              Live
              {!follow && <button className="lr-follow" onClick={() => { setFollow(true); setFocusId(null); }}>follow live ⟳</button>}
            </div>
            <div className="lr-stage-inner">
              {phase === "done" && follow ? (
                <div className="lr-done">
                  <div className="lr-done-check">✓</div>
                  <div className="lr-done-title">Finished in {elapsed.toFixed(1)}s</div>
                  <pre className="lr-answer">{finalAnswer ?? ""}</pre>
                  {changedFiles.length > 0 && (
                    <div className="lr-chips">
                      {changedFiles.map(([p, m]) => <span key={p} className={`lr-chip lr-badge-${m}`}>{m} · {p}</span>)}
                    </div>
                  )}
                </div>
              ) : phase === "stopped" && follow && !running ? (
                <div className="lr-done lr-stopped">
                  <div className="lr-done-check">!</div>
                  <div className="lr-done-title">Run stopped</div>
                  <pre className="lr-answer">{friendlyReason(stopReason ?? status)}</pre>
                </div>
              ) : activeCard ? (
                <>
                  <div className="lr-stage-head">
                    <span>{icon(activeCard.name)} {describe(activeCard.name, activeCard.args)}</span>
                    <span className={`lr-tier lr-tier-${activeCard.tier}`}>{TIER_LABEL[activeCard.tier] ?? activeCard.tier}</span>
                  </div>
                  <Stage card={activeCard} />
                  {activeCard.status === "waiting" && pendingConfirmation && (
                    <div className="lr-approval">
                      <div><b>AI CoWorker wants to do something that needs your OK.</b><br />The backend blocked it until you decide.</div>
                      <div className="lr-confirm">
                        <button className="lr-btn lr-btn-ok" onClick={onApprove}>Approve</button>
                        <button className="lr-btn lr-btn-no" onClick={onDecline}>Decline</button>
                      </div>
                    </div>
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