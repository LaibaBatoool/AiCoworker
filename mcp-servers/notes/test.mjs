// Self-test for the notes & to-do MCP server (no network, no account).
//   node mcp-servers/notes/test.mjs      -> prints "ALL n NOTES TESTS PASSED"
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const SERVER = path.join(path.dirname(fileURLToPath(import.meta.url)), "server.mjs");
const root = fs.mkdtempSync(path.join(os.tmpdir(), "aicw-notes-"));
const ws = path.join(root, "ws");
fs.mkdirSync(ws, { recursive: true });

const child = spawn(process.execPath, [SERVER], {
  cwd: ws, env: { ...process.env, NOTES_FILE: "../notes.json", NOTES_CALL_LOG: "../calls.jsonl" }, stdio: ["pipe", "pipe", "inherit"],
});
const waiting = new Map();
readline.createInterface({ input: child.stdout }).on("line", (l) => { const m = JSON.parse(l); waiting.get(m.id)?.(m); });
let nextId = 1;
const rpc = (method, params) => new Promise((res) => { const id = nextId++; waiting.set(id, res); child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n"); });
const call = async (name, args) => { const r = (await rpc("tools/call", { name, arguments: args })).result; return { text: r.content[0].text, error: !!r.isError }; };
const data = () => JSON.parse(fs.readFileSync(path.join(root, "notes.json"), "utf8"));

let n = 0;
const check = (cond, what) => { n++; if (!cond) { console.error("FAIL:", what); child.kill(); process.exit(1); } console.log("ok -", what); };

await rpc("initialize", { protocolVersion: "2025-11-25", capabilities: {}, clientInfo: { name: "test" } });
const tools = (await rpc("tools/list", {})).result.tools.map((t) => t.name);
check(tools.join(",") === "list_tasks,add_task,update_task,list_notes,read_note,add_note,update_note,delete_item,export_markdown", "9 tools listed");

// tasks
let r = await call("list_tasks", {});
check(!r.error && r.text.startsWith("Today is") && r.text.includes("0 open task(s)"), "empty list, today's date first");
r = await call("add_task", { title: "Write thesis chapter 4", due: "2030-11-10", priority: "high", tags: ["thesis", "#FYP"] });
check(!r.error && /\[t1\] OPEN HIGH \| Write thesis chapter 4 \| Due: 2030-11-10 \| Tags: thesis, fyp/.test(r.text), "add_task: line format, tags cleaned");
await call("add_task", { title: "Book lab GPU", due: "2030-11-05 09:00" });
await call("add_task", { title: "Buy printer paper" });
await call("add_task", { title: "Pay hostel fee", due: "2001-01-01" });
r = await call("list_tasks", {});
const order = [...r.text.matchAll(/^\[(t\d)\]/gm)].map((m) => m[1]).join(",");
check(order === "t4,t2,t1,t3" && r.text.includes("1 overdue") && r.text.includes("(OVERDUE)"), "sorted by due date, overdue flagged, no-date last");
r = await call("add_task", { title: "book lab gpu" });
check(!r.error && r.text.includes("same title already exists (t2)"), "duplicate warning");
for (const [args, why] of [[{ title: "" }, "empty title"], [{ title: "x", due: "next Friday" }, "relative date"], [{ title: "x", due: "2030-02-30" }, "impossible date"],
  [{ title: "x", priority: "urgent" }, "bad priority"], [{ title: "x", tags: ["no spaces allowed"] }, "bad tag"]]) {
  r = await call("add_task", args);
  check(r.error, `add_task refused: ${why}`);
}
r = await call("update_task", { id: "t2", done: true });
check(!r.error && r.text.startsWith("Done: [t2] DONE") && data().tasks.find((t) => t.id === "t2").done_at, "tick a task done");
r = await call("list_tasks", {});
check(!r.text.includes("[t2]"), "done tasks hidden by default");
r = await call("list_tasks", { status: "done" });
check(r.text.includes("[t2]") && !r.text.includes("[t1]"), "list done tasks");
r = await call("update_task", { id: "t2", done: false });
check(r.text.startsWith("Reopened") && !("done" in data().tasks.find((t) => t.id === "t2")), "reopen");
r = await call("update_task", { id: "t1", due: "" });
check(!r.error && !("due" in data().tasks.find((t) => t.id === "t1")), "remove a due date");
r = await call("list_tasks", { tag: "fyp" });
check(r.text.includes("[t1]") && !r.text.includes("[t3]"), "filter by tag");
r = await call("list_tasks", { due_before: "2030-11-06" });
check(r.text.includes("[t2]") && r.text.includes("[t4]") && !r.text.includes("[t3]"), "due_before filter");
r = await call("update_task", { id: "t99", done: true });
check(r.error, "unknown task id");

// notes
r = await call("add_note", { title: "Meeting with Dr. Ahmed", text: "- show the demo\n- benchmark table", tags: ["fyp"] });
check(!r.error && /\[n1\] \d{4}-\d\d-\d\d \d\d:\d\d \| Meeting with Dr\. Ahmed \| Tags: fyp \| Preview: - show the demo - benchmark table/.test(r.text), "add_note: line format");
await call("add_note", { title: "Grocery", text: "milk, eggs" });
r = await call("update_note", { id: "n1", append: "- ask about the defence date" });
check(!r.error && data().notes[0].text.endsWith("benchmark table\n- ask about the defence date"), "append keeps the text");
r = await call("update_note", { id: "n1", text: "x", append: "y" });
check(r.error, "text and append together refused");
r = await call("list_notes", { query: "defence" });
check(r.text.includes("[n1]") && !r.text.includes("[n2]"), "search notes");
r = await call("read_note", { id: "n1" });
check(r.text.includes("Title: Meeting with Dr. Ahmed") && r.text.includes("- ask about the defence date"), "read a note in full");

// export
r = await call("export_markdown", {});
const md = fs.readFileSync(path.join(ws, "notes-export.md"), "utf8");
check(!r.error && md.includes("- [ ] Pay hostel fee — due 2001-01-01") && md.includes("### Meeting with Dr. Ahmed") && md.includes("#fyp"), "export: tasks as checkboxes + notes");
r = await call("export_markdown", { what: "tasks" });
check(!r.error && fs.existsSync(path.join(ws, "notes-export (1).md")) && !fs.readFileSync(path.join(ws, "notes-export (1).md"), "utf8").includes("## Notes"), "export never overwrites; tasks only");
for (const [p, why] of [["../out.md", "parent path"], [path.join(root, "x.md"), "absolute path"], [".git/x.md", ".git"]]) {
  r = await call("export_markdown", { path: p });
  check(r.error, `export refused: ${why}`);
}

// delete
r = await call("delete_item", { id: "t3" });
check(!r.error && !data().tasks.some((t) => t.id === "t3"), "delete a task");
r = await call("delete_item", { id: "n2" });
check(!r.error && data().notes.length === 1, "delete a note");
r = await call("delete_item", { id: "x1" });
check(r.error, "bad id refused");

check(fs.readFileSync(path.join(root, "calls.jsonl"), "utf8").trim().split("\n").length >= 30, "call log written");
console.log(`\nALL ${n} NOTES TESTS PASSED`);
child.kill();
fs.rmSync(root, { recursive: true, force: true });