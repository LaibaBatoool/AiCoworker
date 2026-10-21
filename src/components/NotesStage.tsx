// Live-view visuals for the notes & to-do MCP server (mcp-servers/notes):
//  - a to-do list (ticks, due dates, overdue in red, priority, tags) for list_tasks
//  - a task card for add_task / update_task (a tick animates when a task is done)
//  - note cards for list_notes, and a rendered note (Markdown) for read/add/update_note
// Parses the server's text output (format defined in mcp-servers/notes/server.mjs).
import Markdown from "./Markdown";
import "./notesstage.css";

export interface NotesCard {
  name: string;
  args: any;
  status: "running" | "ok" | "error" | "waiting" | "declined";
  preview: string;
}

const TOOLS = ["list_tasks", "add_task", "update_task", "list_notes", "read_note", "add_note", "update_note", "delete_item", "export_markdown"];

/** "mcp__notes__add_task" -> "add_task" (null if it's not a notes tool). */
export function notesTool(name: string): string | null {
  if (!name.startsWith("mcp__")) return null;
  const tool = name.split("__").slice(2).join("__");
  return TOOLS.includes(tool) ? tool : null;
}

export function describeNotes(tool: string, a: any): string {
  switch (tool) {
    case "list_tasks": return a?.status === "done" ? "Checking finished tasks" : "Checking the to-do list";
    case "add_task": return `Adding a task: ${a?.title ?? ""}${a?.due ? ` (due ${a.due})` : ""}`;
    case "update_task": return a?.done === true ? `Ticking off task ${a?.id ?? ""}` : a?.done === false ? `Reopening task ${a?.id ?? ""}` : `Changing task ${a?.id ?? ""}`;
    case "list_notes": return a?.query ? `Searching notes for "${a.query}"` : "Looking through notes";
    case "read_note": return `Reading note ${a?.id ?? ""}`;
    case "add_note": return `Saving a note: ${a?.title ?? ""}`;
    case "update_note": return a?.append ? `Adding to note ${a?.id ?? ""}` : `Changing note ${a?.id ?? ""}`;
    case "delete_item": return `Deleting ${String(a?.id ?? "").startsWith("t") ? "task" : "note"} ${a?.id ?? ""}`;
    case "export_markdown": return `Exporting ${a?.what && a.what !== "all" ? a.what : "notes and tasks"} to ${a?.path || "notes-export.md"}`;
  }
  return tool;
}

export function notesIcon(tool: string): string {
  return ({ list_tasks: "✅", add_task: "➕", update_task: "☑️", list_notes: "🗒️", read_note: "🗒️", add_note: "📝", update_note: "✏️",
    delete_item: "🗑️", export_markdown: "📤" } as Record<string, string>)[tool] ?? "🗒️";
}

// ---------- parsing ----------
interface Task { id: string; done: boolean; priority?: string; title: string; due?: string; overdue: boolean; tags: string[]; notes?: string }
const TASK = /^\[(t\d+)\] (OPEN|DONE)(?: (HIGH|LOW))? \| (.*?)(?: \| Due: (\d{4}-\d\d-\d\d(?: \d\d:\d\d)?)( \(OVERDUE\))?)?(?: \| Tags: (.*?))?(?: \| Notes: (.*))?$/;
function parseTasks(t: string): Task[] {
  return t.split("\n").map((l) => TASK.exec(l.replace(/^(Before:|Added task:|Done:|Reopened:|Updated:|Deleted task:)\s*/, "").trim())).filter(Boolean).map((m) => ({
    id: m![1], done: m![2] === "DONE", priority: m![3]?.toLowerCase(), title: m![4], due: m![5], overdue: !!m![6],
    tags: m![7] ? m![7].split(", ") : [], notes: m![8],
  }));
}
interface Note { id: string; updated: string; title: string; tags: string[]; preview: string }
const NOTE = /^\[(n\d+)\] (\d{4}-\d\d-\d\d \d\d:\d\d) \| (.*?)(?: \| Tags: (.*?))? \| Preview: (.*)$/;
function parseNotes(t: string): Note[] {
  return t.split("\n").map((l) => NOTE.exec(l.replace(/^(Saved note:|Updated note:|Deleted note:)\s*/, "").trim())).filter(Boolean)
    .map((m) => ({ id: m![1], updated: m![2], title: m![3], tags: m![4] ? m![4].split(", ") : [], preview: m![5] }));
}
const cleanError = (p: string) => p.replace(/^Error:\s*/, "").replace(/^MCP tool '[^']+' reported an error:\s*/, "");

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
function dueLabel(due: string): { text: string; soon: boolean } {
  const m = /^(\d{4})-(\d\d)-(\d\d)(?: (\d\d:\d\d))?/.exec(due);
  if (!m) return { text: due, soon: false };
  const d = new Date(+m[1], +m[2] - 1, +m[3]);
  const today = new Date(); today.setHours(0, 0, 0, 0);
  const days = Math.round((d.getTime() - today.getTime()) / 86400000);
  const word = days === 0 ? "today" : days === 1 ? "tomorrow" : days === -1 ? "yesterday" : `${d.getDate()} ${MONTHS[d.getMonth()]}${d.getFullYear() !== today.getFullYear() ? ` ${d.getFullYear()}` : ""}`;
  return { text: `${word}${m[4] ? ` ${m[4]}` : ""}`, soon: days >= 0 && days <= 1 };
}

function Banner({ card }: { card: NotesCard }) {
  if (card.status === "waiting") return <div className="nt-banner nt-wait">⏸ Waiting for your approval — nothing has changed yet.</div>;
  if (card.status === "declined") return <div className="nt-banner nt-declined">⊘ You declined — nothing was changed.</div>;
  if (card.status === "error") return <div className="nt-banner nt-error">✕ {cleanError(card.preview)}</div>;
  return null;
}

// ---------- pieces ----------
function TaskRow({ t, fresh }: { t: Task; fresh?: boolean }) {
  const d = t.due ? dueLabel(t.due) : null;
  return (
    <div className={`nt-task ${t.done ? "nt-done" : ""} ${fresh ? "nt-fresh" : ""}`}>
      <span className={`nt-box ${t.done ? "nt-box-on" : ""}`}>{t.done ? "✓" : ""}</span>
      <div className="nt-task-main">
        <div className="nt-task-title">{t.priority === "high" && <span className="nt-flag" title="high priority">!</span>}{t.title}</div>
        {(d || t.tags.length > 0 || t.notes) && (
          <div className="nt-meta">
            {d && <span className={`nt-due ${t.overdue ? "nt-overdue" : d.soon ? "nt-soon" : ""}`}>{t.overdue ? "overdue · " : "due "}{d.text}</span>}
            {t.tags.map((g) => <span key={g} className="nt-tag">#{g}</span>)}
            {t.notes && <span className="nt-muted">{t.notes}</span>}
          </div>
        )}
      </div>
      {t.priority === "low" && <span className="nt-muted nt-low">low</span>}
      <span className="nt-id">{t.id}</span>
    </div>
  );
}

function TaskList({ card }: { card: NotesCard }) {
  if (card.status !== "ok") return <div className="nt-panel"><div className="nt-head"><span>✅ To-do</span></div><Banner card={card} />{card.status === "running" && <div className="nt-empty">Loading…</div>}</div>;
  const tasks = parseTasks(card.preview);
  const head = card.preview.split("\n").find((l) => /task\(s\)/.test(l)) ?? "";
  return (
    <div className="nt-panel">
      <div className="nt-head"><span>✅ {card.args?.status === "done" ? "Done" : card.args?.status === "all" ? "All tasks" : "To-do"}</span><span className="nt-muted">{head.replace(/[:.]$/, "")}</span></div>
      {tasks.length === 0 && <div className="nt-empty">Nothing here. 🎉</div>}
      {tasks.map((t, i) => <div key={t.id} style={{ animationDelay: `${i * 40}ms` }} className="nt-in"><TaskRow t={t} /></div>)}
    </div>
  );
}

function TaskCard({ card, tool }: { card: NotesCard; tool: string }) {
  const a = card.args ?? {};
  const parsed = parseTasks(card.preview);
  const after = card.status === "ok" ? parsed[0] : undefined;
  const before = tool === "update_task" && card.status === "ok" ? parsed[1] : undefined;
  const asked: Task = { id: a.id ?? "new", done: a.done === true, priority: a.priority, title: a.title ?? (tool === "add_task" ? "(no title)" : `task ${a.id ?? ""}`),
    due: a.due || undefined, overdue: false, tags: Array.isArray(a.tags) ? a.tags : [], notes: a.notes };
  const notes = card.preview.split("\n").filter((l) => /^Note:/.test(l));
  return (
    <div className="nt-panel">
      <div className="nt-head">
        <span>{notesIcon(tool)} {tool === "add_task" ? "New task" : a.done === true ? "Task done" : a.done === false ? "Task reopened" : "Change task"}</span>
        {card.status === "ok" && <span className="nt-ok">{tool === "add_task" ? "✓ added" : a.done === true ? "✓ ticked off" : "✓ saved"}</span>}
      </div>
      <Banner card={card} />
      {card.status !== "error" && (tool === "add_task" || after) && <TaskRow t={after ?? asked} fresh={a.done === true && !!after} />}
      {card.status !== "error" && tool === "update_task" && !after && <div className="nt-empty">Changing task {a.id}…</div>}
      {before && a.done === undefined && <div className="nt-before">was: {before.title}{before.due ? ` · due ${before.due}` : ""}{before.priority ? ` · ${before.priority}` : ""}</div>}
      {notes.map((x) => <div key={x} className="nt-banner nt-wait">{x}</div>)}
    </div>
  );
}

function NoteList({ card }: { card: NotesCard }) {
  if (card.status !== "ok") return <div className="nt-panel"><Banner card={card} />{card.status === "running" && <div className="nt-empty">Loading…</div>}</div>;
  const notes = parseNotes(card.preview);
  return (
    <div className="nt-panel">
      <div className="nt-head"><span>🗒️ {card.args?.query ? `Notes matching "${card.args.query}"` : "Notes"}</span><span className="nt-muted">{notes.length} note{notes.length === 1 ? "" : "s"}</span></div>
      {notes.length === 0 && <div className="nt-empty">No notes.</div>}
      <div className="nt-grid">
        {notes.map((n) => (
          <div key={n.id} className="nt-note-tile">
            <div className="nt-note-title">{n.title}</div>
            <div className="nt-note-preview">{n.preview}</div>
            <div className="nt-meta">{n.tags.map((g) => <span key={g} className="nt-tag">#{g}</span>)}<span className="nt-muted">{n.updated}</span><span className="nt-id">{n.id}</span></div>
          </div>
        ))}
      </div>
    </div>
  );
}

function NoteCard({ card, tool }: { card: NotesCard; tool: string }) {
  const a = card.args ?? {};
  let title = a.title ?? "", body = "", tags: string[] = Array.isArray(a.tags) ? a.tags : [], when = "";
  if (tool === "read_note" && card.status === "ok") {
    const [head, ...rest] = card.preview.split("\n\n");
    const h = Object.fromEntries(head.split("\n").map((l) => /^(\w+):\s?(.*)$/.exec(l)).filter(Boolean).map((m) => [m![1].toLowerCase(), m![2]]));
    title = h.title ?? ""; when = h.updated ?? ""; tags = h.tags ? h.tags.split(", ") : []; body = rest.join("\n\n");
  } else if (tool === "add_note") body = String(a.text ?? "");
  else if (tool === "update_note") {
    const n = parseNotes(card.preview)[0];
    if (n) { title = n.title; tags = n.tags; when = n.updated; }
    body = a.append ? String(a.append) : String(a.text ?? "");
  }
  return (
    <div className="nt-panel nt-note">
      <div className="nt-head">
        <span>{notesIcon(tool)} {tool === "read_note" ? "Note" : tool === "add_note" ? "New note" : a.append ? "Adding to a note" : "Change note"}</span>
        {card.status === "ok" && tool !== "read_note" && <span className="nt-ok">✓ saved</span>}
      </div>
      <Banner card={card} />
      {card.status !== "error" && (
        <div className="nt-paper">
          <div className="nt-paper-title">{title || `note ${a.id ?? ""}`}</div>
          <div className="nt-meta">{tags.map((g) => <span key={g} className="nt-tag">#{g}</span>)}{when && <span className="nt-muted">{when}</span>}</div>
          {a.append && <div className="nt-muted nt-append-label">added at the end:</div>}
          <div className={a.append ? "nt-appended" : ""}>{body ? <Markdown text={body} /> : <span className="nt-muted">{card.status === "running" ? "…" : "(empty)"}</span>}</div>
        </div>
      )}
    </div>
  );
}

function Simple({ card, tool }: { card: NotesCard; tool: string }) {
  return (
    <div className="nt-simple">
      <div className="nt-simple-icon">{notesIcon(tool)}</div>
      <div>{describeNotes(tool, card.args)}</div>
      <Banner card={card} />
      {card.status === "ok" && <div className="nt-muted">{card.preview.split("\n")[0]}</div>}
    </div>
  );
}

export default function NotesStage({ card }: { card: NotesCard }) {
  const tool = notesTool(card.name) ?? "";
  switch (tool) {
    case "list_tasks": return <TaskList card={card} />;
    case "add_task":
    case "update_task": return <TaskCard card={card} tool={tool} />;
    case "list_notes": return <NoteList card={card} />;
    case "read_note":
    case "add_note":
    case "update_note": return <NoteCard card={card} tool={tool} />;
    default: return <Simple card={card} tool={tool} />;
  }
}