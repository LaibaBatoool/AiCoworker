// AI CoWorker notes & to-do MCP server (stdio, JSON-RPC 2.0, one message per line). No dependencies.
//
// Local-first: notes and tasks live in a JSON file on this computer (NOTES_FILE, relative paths are
// resolved against the server's working directory = the workspace; default
// %APPDATA%\AiCoworker\notes.json). Nothing is sent anywhere.
//
// Notes have ids n1, n2...; tasks t1, t2... Dates are local: "YYYY-MM-DD" or "YYYY-MM-DD HH:MM".
//
// Tools (tiers are set in AI CoWorker's mcp.json, NEVER by this server):
//   read-only : list_notes, read_note, list_tasks
//   changes   : add_note, update_note, add_task, update_task (also ticks a task done), export_markdown
//   risky     : delete_item
//
// Safety: export only inside the workspace (no absolute paths, "..", .git), never overwrites.
// Note text is the user's own data, but can contain pasted outside text: it is shown to the model
// as data (the app spotlights every MCP result).
//
// NOTES_CALL_LOG=<path> appends every tools/call as JSON lines (used by the benchmark).
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";

const WORKSPACE = process.cwd();
const MAX_ITEMS = 5000;

// ---------------------------------------------------------------- helpers
const out = (m) => process.stdout.write(JSON.stringify(m) + "\n");
const ok = (id, text) => out({ jsonrpc: "2.0", id, result: { content: [{ type: "text", text }] } });
const toolError = (id, text) => out({ jsonrpc: "2.0", id, result: { content: [{ type: "text", text }], isError: true } });
const oneLine = (s) => String(s ?? "").replace(/\s+/g, " ").trim();
const pad2 = (n) => String(n).padStart(2, "0");
const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const ymd = (d) => `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
const hm = (d) => `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
const nice = (d) => `${DAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
const now = () => new Date();
const todayLine = () => `Today is ${nice(now())}, ${hm(now())} (local time).`;

function logCall(name, args) {
  const p = process.env.NOTES_CALL_LOG;
  if (!p) return;
  try { fs.appendFileSync(path.resolve(WORKSPACE, p), JSON.stringify({ t: Date.now(), name, arguments: args }) + "\n"); } catch {}
}

function text(v, what, max, required) {
  const s = String(v ?? "").trim();
  if (required && !s) throw new Error(`'${what}' is empty.`);
  if (s.length > max) throw new Error(`'${what}' is too long (${max} characters max).`);
  return s;
}
function tags(v) {
  if (v === undefined || v === null) return undefined;
  const list = (Array.isArray(v) ? v : String(v).split(/[,;]/)).map((t) => String(t).trim().replace(/^#/, "").toLowerCase()).filter(Boolean);
  if (list.length > 20) throw new Error("At most 20 tags.");
  for (const t of list) if (!/^[\p{L}\p{N}_-]{1,40}$/u.test(t)) throw new Error(`Bad tag "${t}": letters, digits, - and _ only.`);
  return list.length ? [...new Set(list)] : undefined;
}
/** "2026-10-24" or "2026-10-24 17:00" -> normalised string (local time). */
function due(raw) {
  if (raw === null || raw === "") return undefined;
  const s = String(raw).trim();
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{1,2}):(\d{2})(?::\d{2})?)?$/.exec(s);
  if (!m) throw new Error(`due "${s}" is not a date. Use YYYY-MM-DD or YYYY-MM-DD HH:MM (local time). ${todayLine()}`);
  const d = new Date(+m[1], +m[2] - 1, +m[3], +(m[4] ?? 0), +(m[5] ?? 0));
  if (d.getMonth() !== +m[2] - 1 || d.getDate() !== +m[3] || +(m[4] ?? 0) > 23 || +(m[5] ?? 0) > 59) throw new Error(`due "${s}" is not a real date.`);
  return m[4] === undefined ? ymd(d) : `${ymd(d)} ${hm(d)}`;
}
const dueDate = (t) => (t.due ? new Date(t.due.length === 10 ? `${t.due}T23:59:59` : t.due.replace(" ", "T")) : null);
const PRIORITIES = ["low", "normal", "high"];
function priority(v) {
  if (v === undefined || v === null || v === "") return undefined;
  const p = String(v).toLowerCase();
  if (!PRIORITIES.includes(p)) throw new Error("priority must be low, normal or high.");
  return p === "normal" ? undefined : p;
}

// ---- storage ----
const dataFile = () => {
  const p = process.env.NOTES_FILE;
  if (p) return path.resolve(WORKSPACE, p);
  const base = process.env.APPDATA || process.env.XDG_CONFIG_HOME || path.join(process.env.HOME || ".", ".config");
  return path.join(base, "AiCoworker", "notes.json");
};
function load() {
  const f = dataFile();
  const d = fs.existsSync(f) ? JSON.parse(fs.readFileSync(f, "utf8").replace(/^\uFEFF/, "")) : {};
  d.notes ||= [];
  d.tasks ||= [];
  d.next_note ||= d.notes.length + 1;
  d.next_task ||= d.tasks.length + 1;
  return d;
}
function save(d) {
  const f = dataFile();
  fs.mkdirSync(path.dirname(f), { recursive: true });
  fs.writeFileSync(f, JSON.stringify(d, null, 2));
}
const stamp = () => `${ymd(now())} ${hm(now())}`;

// ---- one-line formats (the app's Live view parses these) ----
function taskLine(t) {
  const d = dueDate(t);
  const overdue = !t.done && d && d < now();
  return `[${t.id}] ${t.done ? "DONE" : "OPEN"}${t.priority ? ` ${t.priority.toUpperCase()}` : ""} | ${oneLine(t.title)}` +
    (t.due ? ` | Due: ${t.due}${overdue ? " (OVERDUE)" : ""}` : "") +
    (t.tags?.length ? ` | Tags: ${t.tags.join(", ")}` : "") +
    (t.notes ? ` | Notes: ${oneLine(t.notes).slice(0, 100)}` : "");
}
function noteLine(n) {
  return `[${n.id}] ${n.updated} | ${oneLine(n.title)}` + (n.tags?.length ? ` | Tags: ${n.tags.join(", ")}` : "") +
    ` | Preview: ${oneLine(n.text).slice(0, 90)}`;
}
function findIn(list, id, kind) {
  const it = list.find((x) => x.id === String(id ?? "").trim());
  if (!it) throw new Error(`No ${kind} with id ${id}. Use list_${kind}s to get ids.`);
  return it;
}

// ---- workspace export ----
function safeRel(raw) {
  const rel = String(raw ?? "").replace(/\\/g, "/").replace(/^(\.\/)+/, "");
  if (!rel || path.isAbsolute(rel) || /^[a-zA-Z]:/.test(rel) || rel.split("/").includes(".."))
    throw new Error(`path must be inside the workspace: ${raw}`);
  if (rel.split("/").some((p) => [".git", ".aicoworker"].includes(p.toLowerCase()))) throw new Error(`path is in a protected folder: ${rel}`);
  return rel;
}
function insideWorkspace(full) {
  const root = fs.realpathSync(WORKSPACE);
  let p = full;
  while (!fs.existsSync(p)) p = path.dirname(p);
  return fs.realpathSync(p).startsWith(root);
}

// ---------------------------------------------------------------- tools
const TAGS = { type: "array", items: { type: "string" }, description: "Short tags like fyp, thesis, personal" };
const TOOLS = [
  {
    name: "list_tasks",
    description: "The user's to-do list: open tasks first (overdue ones flagged), with due dates and priorities. Starts with today's date.",
    inputSchema: { type: "object", properties: {
      status: { type: "string", enum: ["open", "done", "all"], description: "Default open" },
      tag: { type: "string" },
      due_before: { type: "string", description: "YYYY-MM-DD: only tasks due on or before this day" } } },
  },
  {
    name: "add_task",
    description: "Add one task to the user's to-do list. For several tasks, call it once per task.",
    inputSchema: { type: "object", properties: {
      title: { type: "string" },
      due: { type: "string", description: "YYYY-MM-DD or YYYY-MM-DD HH:MM, local time" },
      priority: { type: "string", enum: ["low", "normal", "high"] },
      notes: { type: "string" }, tags: TAGS }, required: ["title"] },
  },
  {
    name: "update_task",
    description: "Change a task: mark it done (done: true) or reopen it, or change its title, due date, priority, notes or tags. Give only the fields to change; due: \"\" removes the due date.",
    inputSchema: { type: "object", properties: {
      id: { type: "string", description: "Task id, e.g. t3" }, done: { type: "boolean" }, title: { type: "string" },
      due: { type: "string" }, priority: { type: "string", enum: ["low", "normal", "high"] }, notes: { type: "string" }, tags: TAGS },
      required: ["id"] },
  },
  {
    name: "list_notes",
    description: "The user's notes, newest first (title, tags, a short preview). Optional search text or tag.",
    inputSchema: { type: "object", properties: { query: { type: "string" }, tag: { type: "string" } } },
  },
  {
    name: "read_note",
    description: "Read one note in full.",
    inputSchema: { type: "object", properties: { id: { type: "string", description: "Note id, e.g. n2" } }, required: ["id"] },
  },
  {
    name: "add_note",
    description: "Save a new note (Markdown text is fine).",
    inputSchema: { type: "object", properties: { title: { type: "string" }, text: { type: "string" }, tags: TAGS }, required: ["title", "text"] },
  },
  {
    name: "update_note",
    description: "Change a note: new title, tags, replace its text, or append text to the end (append keeps what is there).",
    inputSchema: { type: "object", properties: {
      id: { type: "string" }, title: { type: "string" }, text: { type: "string", description: "Replaces the whole text" },
      append: { type: "string", description: "Added at the end on a new line" }, tags: TAGS }, required: ["id"] },
  },
  {
    name: "delete_item",
    description: "Delete a note (n…) or a task (t…). The app asks the user to approve it. To finish a task, use update_task with done: true instead.",
    inputSchema: { type: "object", properties: { id: { type: "string" } }, required: ["id"] },
  },
  {
    name: "export_markdown",
    description: "Write the to-do list and/or notes into a Markdown file in the workspace (default notes-export.md). Never overwrites a file.",
    inputSchema: { type: "object", properties: {
      path: { type: "string" }, what: { type: "string", enum: ["tasks", "notes", "all"], description: "Default all" } } },
  },
];

function callTool(name, a) {
  if (name === "list_tasks") {
    const d = load();
    const status = a.status ?? "open";
    if (!["open", "done", "all"].includes(status)) throw new Error("status must be open, done or all.");
    const tag = a.tag ? tags([a.tag])?.[0] : undefined;
    const before = a.due_before ? due(a.due_before) : undefined;
    const pr = (t) => (t.priority === "high" ? 0 : t.priority === "low" ? 2 : 1);
    const list = d.tasks
      .filter((t) => status === "all" || (status === "done") === !!t.done)
      .filter((t) => !tag || t.tags?.includes(tag))
      .filter((t) => !before || (t.due && t.due.slice(0, 10) <= before.slice(0, 10)))
      // open before done, then soonest due (no due date last), then high priority first
      .sort((x, y) => (x.done ? 1 : 0) - (y.done ? 1 : 0) || (dueDate(x) ?? Infinity) - (dueDate(y) ?? Infinity) || pr(x) - pr(y));
    const overdue = list.filter((t) => !t.done && dueDate(t) && dueDate(t) < now()).length;
    return `${todayLine()}\n${list.length} ${status === "all" ? "" : status + " "}task(s)${overdue ? `, ${overdue} overdue` : ""}${list.length ? ":" : "."}\n` + list.map(taskLine).join("\n");
  }
  if (name === "add_task") {
    const d = load();
    if (d.tasks.length >= MAX_ITEMS) throw new Error("The to-do list is full (5000 tasks).");
    const t = { id: `t${d.next_task}`, title: text(a.title, "title", 200, true), created: stamp() };
    if (a.due !== undefined) t.due = due(a.due);
    if (a.priority !== undefined) t.priority = priority(a.priority);
    if (a.notes !== undefined) t.notes = text(a.notes, "notes", 2000) || undefined;
    if (a.tags !== undefined) t.tags = tags(a.tags);
    const dup = d.tasks.find((x) => !x.done && x.title.toLowerCase() === t.title.toLowerCase());
    d.next_task++;
    d.tasks.push(JSON.parse(JSON.stringify(t)));
    save(d);
    return `Added task: ${taskLine(t)}` + (dup ? `\nNote: an open task with the same title already exists (${dup.id}).` : "") +
      (t.due && dueDate(t) < now() ? "\nNote: the due date is already past." : "");
  }
  if (name === "update_task") {
    const d = load();
    const t = findIn(d.tasks, a.id, "task");
    const before = taskLine(t);
    if (a.title !== undefined) t.title = text(a.title, "title", 200, true);
    if (a.due !== undefined) t.due = due(a.due);
    if (a.priority !== undefined) t.priority = priority(a.priority);
    if (a.notes !== undefined) t.notes = text(a.notes, "notes", 2000) || undefined;
    if (a.tags !== undefined) t.tags = tags(a.tags);
    if (a.done !== undefined) { t.done = !!a.done || undefined; t.done_at = a.done ? stamp() : undefined; }
    Object.keys(t).forEach((k) => t[k] === undefined && delete t[k]);
    save(d);
    return `${a.done === true ? "Done" : a.done === false ? "Reopened" : "Updated"}: ${taskLine(t)}\nBefore: ${before}`;
  }
  if (name === "list_notes") {
    const d = load();
    const q = a.query ? text(a.query, "query", 200).toLowerCase() : "";
    const tag = a.tag ? tags([a.tag])?.[0] : undefined;
    const list = d.notes
      .filter((n) => !q || `${n.title}\n${n.text}\n${(n.tags ?? []).join(" ")}`.toLowerCase().includes(q))
      .filter((n) => !tag || n.tags?.includes(tag))
      .sort((x, y) => String(y.updated).localeCompare(String(x.updated)));
    return `${list.length} note(s)${list.length ? ":" : "."}\n` + list.slice(0, 50).map(noteLine).join("\n");
  }
  if (name === "read_note") {
    const n = findIn(load().notes, a.id, "note");
    return `id: ${n.id}\nTitle: ${n.title}\nUpdated: ${n.updated}${n.tags?.length ? `\nTags: ${n.tags.join(", ")}` : ""}\n\n${n.text}`;
  }
  if (name === "add_note") {
    const d = load();
    if (d.notes.length >= MAX_ITEMS) throw new Error("Too many notes (5000).");
    const n = { id: `n${d.next_note}`, title: text(a.title, "title", 200, true), text: text(a.text, "text", 50000, true), tags: tags(a.tags), updated: stamp() };
    if (!n.tags) delete n.tags;
    d.next_note++;
    d.notes.push(n);
    save(d);
    return `Saved note: ${noteLine(n)}`;
  }
  if (name === "update_note") {
    const d = load();
    const n = findIn(d.notes, a.id, "note");
    if (a.text !== undefined && a.append !== undefined) throw new Error("Give either 'text' (replace) or 'append', not both.");
    if (a.title !== undefined) n.title = text(a.title, "title", 200, true);
    if (a.text !== undefined) n.text = text(a.text, "text", 50000, true);
    if (a.append !== undefined) {
      const add = text(a.append, "append", 50000, true);
      if (n.text.length + add.length > 50000) throw new Error("The note would be longer than 50,000 characters.");
      n.text = `${n.text.replace(/\s+$/, "")}\n${add}`;
    }
    if (a.tags !== undefined) { n.tags = tags(a.tags); if (!n.tags) delete n.tags; }
    n.updated = stamp();
    save(d);
    return `Updated note: ${noteLine(n)}`;
  }
  if (name === "delete_item") {
    const d = load();
    const id = String(a.id ?? "").trim();
    const kind = id.startsWith("t") ? "task" : id.startsWith("n") ? "note" : null;
    if (!kind) throw new Error(`Bad id ${id}: notes are n…, tasks are t….`);
    const list = kind === "task" ? d.tasks : d.notes;
    const it = findIn(list, id, kind);
    if (kind === "task") d.tasks = d.tasks.filter((x) => x !== it); else d.notes = d.notes.filter((x) => x !== it);
    save(d);
    return `Deleted ${kind}: ${kind === "task" ? taskLine(it) : noteLine(it)}`;
  }
  if (name === "export_markdown") {
    const d = load();
    const what = a.what ?? "all";
    if (!["tasks", "notes", "all"].includes(what)) throw new Error("what must be tasks, notes or all.");
    let rel = safeRel(a.path || "notes-export.md");
    if (!/\.md$/i.test(rel)) rel += ".md";
    let candidate = rel, i = 1;
    while (fs.existsSync(path.join(WORKSPACE, candidate))) candidate = `${rel.slice(0, -3)} (${i++}).md`;
    const full = path.join(WORKSPACE, candidate);
    if (!insideWorkspace(full)) throw new Error(`path escapes the workspace: ${rel}`);
    const L = [`# Notes and tasks`, ``, `Exported ${nice(now())}, ${hm(now())}.`, ``];
    if (what !== "notes") {
      const open = d.tasks.filter((t) => !t.done).sort((x, y) => (dueDate(x) ?? Infinity) - (dueDate(y) ?? Infinity));
      const done = d.tasks.filter((t) => t.done);
      L.push(`## To-do (${open.length} open, ${done.length} done)`, ``);
      for (const t of [...open, ...done])
        L.push(`- [${t.done ? "x" : " "}] ${t.title}${t.due ? ` — due ${t.due}` : ""}${t.priority === "high" ? " (high priority)" : ""}${t.tags?.length ? ` ${t.tags.map((x) => `#${x}`).join(" ")}` : ""}`);
      L.push(``);
    }
    if (what !== "tasks") {
      L.push(`## Notes (${d.notes.length})`, ``);
      for (const n of d.notes.slice().sort((x, y) => String(y.updated).localeCompare(String(x.updated))))
        L.push(`### ${n.title}`, ``, `_${n.updated}${n.tags?.length ? ` · ${n.tags.map((x) => `#${x}`).join(" ")}` : ""}_`, ``, n.text, ``);
    }
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, L.join("\n"), { flag: "wx" });
    return `Exported ${what === "notes" ? "" : `${d.tasks.length} task(s)`}${what === "all" ? " and " : ""}${what === "tasks" ? "" : `${d.notes.length} note(s)`} to ${candidate}.`;
  }
  throw new Error(`Unknown tool: ${name}`);
}

// ---------------------------------------------------------------- JSON-RPC loop
readline.createInterface({ input: process.stdin }).on("line", (raw) => {
  let m;
  try { m = JSON.parse(raw); } catch { return; }
  if (m.method === "initialize") {
    const asked = m.params?.protocolVersion;
    const versions = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
    return out({ jsonrpc: "2.0", id: m.id, result: {
      protocolVersion: versions.includes(asked) ? asked : "2025-06-18",
      capabilities: { tools: {} },
      serverInfo: { name: "aicoworker-notes", version: "1.0.0" },
    } });
  }
  if (m.id === undefined) return;
  if (m.method === "ping") return out({ jsonrpc: "2.0", id: m.id, result: {} });
  if (m.method === "tools/list") return out({ jsonrpc: "2.0", id: m.id, result: { tools: TOOLS } });
  if (m.method === "tools/call") {
    const { name, arguments: a = {} } = m.params || {};
    logCall(name, a);
    try { return ok(m.id, callTool(name, a || {})); }
    catch (e) { return toolError(m.id, String(e?.message || e)); }
  }
  out({ jsonrpc: "2.0", id: m.id, error: { code: -32601, message: `Method not found: ${m.method}` } });
});