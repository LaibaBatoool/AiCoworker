// Self-test for the email MCP server (mock mode, no network, no account).
//   node mcp-servers/email/test.mjs      -> prints "ALL n EMAIL TESTS PASSED"
// Starts the server like AI CoWorker does (stdio, cwd = a temp workspace) and talks JSON-RPC to it.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const SERVER = path.join(path.dirname(fileURLToPath(import.meta.url)), "server.mjs");
const root = fs.mkdtempSync(path.join(os.tmpdir(), "aicw-email-"));
const ws = path.join(root, "ws");
fs.mkdirSync(path.join(ws, "secrets"), { recursive: true });
fs.writeFileSync(path.join(ws, "report.pdf"), "%PDF-1.4 fake report");
fs.writeFileSync(path.join(ws, ".env"), "API_KEY=abcdef123456");
fs.writeFileSync(path.join(ws, "secrets", "notes.txt"), "secret");
fs.writeFileSync(path.join(root, "outside.txt"), "outside the workspace");

const child = spawn(process.execPath, [SERVER], {
  cwd: ws,
  env: { ...process.env, EMAIL_MODE: "mock", EMAIL_MOCK_FILE: "../mailbox.json", EMAIL_CALL_LOG: "../calls.jsonl" },
  stdio: ["pipe", "pipe", "inherit"],
});
const waiting = new Map();
readline.createInterface({ input: child.stdout }).on("line", (l) => {
  const m = JSON.parse(l);
  waiting.get(m.id)?.(m);
});
let nextId = 1;
const rpc = (method, params) => new Promise((res) => { const id = nextId++; waiting.set(id, res); child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n"); });
const call = async (name, args) => {
  const r = (await rpc("tools/call", { name, arguments: args })).result;
  return { text: r.content[0].text, error: !!r.isError };
};
const box = () => JSON.parse(fs.readFileSync(path.join(root, "mailbox.json"), "utf8"));

let n = 0;
const check = (cond, what) => { n++; if (!cond) { console.error("FAIL:", what); child.kill(); process.exit(1); } console.log("ok -", what); };

const init = await rpc("initialize", { protocolVersion: "2025-11-25", capabilities: {}, clientInfo: { name: "test" } });
check(init.result.protocolVersion === "2025-11-25", "initialize");
const tools = (await rpc("tools/list", {})).result.tools.map((t) => t.name);
check(tools.join(",") === "list_inbox,search_email,read_email,send_email,forward_email,draft_email,save_attachments,mark_email,delete_email", "9 tools listed");

// reading
let r = await call("list_inbox", { unread_only: true });
check(!r.error && r.text.includes("2 email(s)") && r.text.includes("[m1]") && r.text.includes("[m2]"), "seeded mailbox, 2 unread");
check(/^\[m2\] UNREAD 2026-10-20 11:02 \| From: Dr\. Ahmed <[^>]+> \| Subject: FYP progress meeting moved \| Attachments: 1 \| Preview: /m.test(r.text), "summary line format (the UI parses it)");
r = await call("read_email", { id: "m1" });
check(r.text.includes("quarterly report") && r.text.includes("From: Sara Khan"), "read_email shows the body");
r = await call("list_inbox", { unread_only: true });
check(r.text.includes("1 email(s)"), "reading marks it read");
r = await call("search_email", { query: "meeting" });
check(r.text.includes("[m2]") && !r.text.includes("[m1]"), "search by subject");
r = await call("read_email", { id: "nope" });
check(r.error, "unknown id is a tool error");

// sending
r = await call("send_email", { reply_to_id: "m1", body: "Hi Sara, report attached.", attachments: ["report.pdf"] });
check(!r.error && r.text.includes("sara.khan@example.com") && r.text.includes("Re: Quarterly report"), "reply fills to + Re: subject");
check(box().sent.length === 1 && box().sent[0].attachments[0].filename === "report.pdf" && box().sent[0].in_reply_to === "m1", "sent mail recorded with attachment hash");
r = await call("send_email", { to: ["Sara Khan <sara.khan@example.com>"], subject: "Hi", body: "x" });
check(!r.error && box().sent[1].to[0] === "sara.khan@example.com", "'Name <address>' recipients accepted");

for (const [bad, why] of [[".env", "secret file"], ["secrets/notes.txt", "secrets folder"], ["../outside.txt", "parent path"],
  [path.join(root, "outside.txt"), "absolute path"], ["missing.pdf", "missing file"]]) {
  r = await call("send_email", { to: ["a@example.com"], subject: "x", body: "x", attachments: [bad] });
  check(r.error, `attachment refused: ${why}`);
}
r = await call("send_email", { to: ["not-an-address"], subject: "x", body: "x" });
check(r.error, "bad address refused");
r = await call("send_email", { to: ["a@example.com"], subject: "hi\r\nBcc: evil@example.com", body: "x" });
check(r.error, "header injection refused");
r = await call("send_email", { to: Array.from({ length: 11 }, (_, i) => `u${i}@example.com`), subject: "x", body: "x" });
check(r.error, "more than 10 recipients refused");
check(box().sent.length === 2, "refused sends recorded nothing");

// forward, draft
r = await call("forward_email", { id: "m2", to: ["bilal@example.com"], note: "FYI, see the agenda." });
const fwd = box().sent[2];
check(!r.error && fwd.subject === "Fwd: FYP progress meeting moved" && fwd.attachments[0].filename === "meeting-agenda.txt" && fwd.body.startsWith("FYI"),
  "forward keeps the attachment, adds Fwd: and the note");
r = await call("forward_email", { id: "m2" });
check(r.error, "forward needs recipients");
r = await call("draft_email", { reply_to_id: "m2", body: "Thank you, I will be there." });
check(!r.error && box().drafts.length === 1 && box().drafts[0].subject === "Re: FYP progress meeting moved" && box().sent.length === 3, "draft saved, nothing sent");
r = await call("draft_email", { body: "ideas for later" });
check(!r.error && box().drafts.length === 2, "a draft may have no recipient yet");

// attachments into the workspace
r = await call("save_attachments", { id: "m2" });
check(!r.error && fs.readFileSync(path.join(ws, "email-attachments", "meeting-agenda.txt"), "utf8").includes("Demo of the agent"), "attachment saved to email-attachments/");
r = await call("save_attachments", { id: "m2" });
check(!r.error && fs.existsSync(path.join(ws, "email-attachments", "meeting-agenda (1).txt")), "never overwrites (second copy gets (1))");
r = await call("save_attachments", { id: "m2", folder: "../escape" });
check(r.error && !fs.existsSync(path.join(root, "escape")), "folder outside the workspace refused");
r = await call("save_attachments", { id: "m2", folder: ".git/hooks" });
check(r.error, "protected folder refused");
r = await call("save_attachments", { id: "m1" });
check(r.text.includes("no attachments"), "email without attachments");
// hostile attachment names stay inside the folder
const b = box();
b.inbox.push({ id: "m9", from: "x <x@evil.example>", to: "me@example.com", date: "2026-10-21 10:00", unread: true, subject: "evil", body: "x",
  attachments: [{ filename: "../../outside-evil.txt", content_base64: "aGk=" }, { filename: "CON", content_base64: "aGk=" }] });
fs.writeFileSync(path.join(root, "mailbox.json"), JSON.stringify(b));
r = await call("save_attachments", { id: "m9", folder: "in" });
check(!r.error && fs.existsSync(path.join(ws, "in", "outside-evil.txt")) && !fs.existsSync(path.join(root, "outside-evil.txt")) && fs.existsSync(path.join(ws, "in", "_CON")),
  "hostile attachment names sanitised");

// mark, delete
r = await call("mark_email", { id: "m1", read: false, starred: true });
check(!r.error && box().inbox.find((x) => x.id === "m1").unread === true && /\[m1\] UNREAD STARRED/.test((await call("list_inbox", {})).text), "mark unread + starred");
r = await call("mark_email", { id: "m1" });
check(r.error, "mark needs read or starred");
r = await call("delete_email", { id: "m3" });
check(!r.error && !box().inbox.some((x) => x.id === "m3") && box().trash[0].id === "m3", "delete moves to Trash");

check(fs.readFileSync(path.join(root, "calls.jsonl"), "utf8").trim().split("\n").length >= 25, "call log written");

console.log(`\nALL ${n} EMAIL TESTS PASSED`);
child.kill();
fs.rmSync(root, { recursive: true, force: true });