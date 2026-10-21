// Self-test for the calendar MCP server (no network, no account).
//   node mcp-servers/calendar/test.mjs      -> prints "ALL n CALENDAR TESTS PASSED"
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const SERVER = path.join(path.dirname(fileURLToPath(import.meta.url)), "server.mjs");
const root = fs.mkdtempSync(path.join(os.tmpdir(), "aicw-cal-"));
const ws = path.join(root, "ws");
fs.mkdirSync(ws, { recursive: true });
// an invitation as Google/Outlook send it (folded line, escaped comma, UTC time, alarm)
fs.writeFileSync(path.join(ws, "invite.ics"), [
  "BEGIN:VCALENDAR", "VERSION:2.0", "BEGIN:VEVENT", "UID:abc@example.com",
  "DTSTART:20301104T090000Z", "DTEND:20301104T100000Z",
  "SUMMARY:Thesis review\\, chapter 3", "LOCATION:Room 12",
  "DESCRIPTION:Bring the benchmark results. This line is long enough that it gets fol",
  " ded by the sender.", "BEGIN:VALARM", "TRIGGER:-PT1H", "ACTION:DISPLAY", "END:VALARM", "END:VEVENT",
  "BEGIN:VEVENT", "DTSTART;VALUE=DATE:20301105", "SUMMARY:Holiday", "END:VEVENT", "END:VCALENDAR", ""].join("\r\n"));

const child = spawn(process.execPath, [SERVER], {
  cwd: ws, env: { ...process.env, CALENDAR_FILE: "../calendar.json", CALENDAR_CALL_LOG: "../calls.jsonl" }, stdio: ["pipe", "pipe", "inherit"],
});
const waiting = new Map();
readline.createInterface({ input: child.stdout }).on("line", (l) => { const m = JSON.parse(l); waiting.get(m.id)?.(m); });
let nextId = 1;
const rpc = (method, params) => new Promise((res) => { const id = nextId++; waiting.set(id, res); child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n"); });
const call = async (name, args) => { const r = (await rpc("tools/call", { name, arguments: args })).result; return { text: r.content[0].text, error: !!r.isError }; };
const cal = () => JSON.parse(fs.readFileSync(path.join(root, "calendar.json"), "utf8"));

let n = 0;
const check = (cond, what) => { n++; if (!cond) { console.error("FAIL:", what); child.kill(); process.exit(1); } console.log("ok -", what); };

await rpc("initialize", { protocolVersion: "2025-11-25", capabilities: {}, clientInfo: { name: "test" } });
const tools = (await rpc("tools/list", {})).result.tools.map((t) => t.name);
check(tools.join(",") === "list_events,search_events,find_free_time,add_event,update_event,delete_event,export_ics,import_ics", "8 tools listed");

let r = await call("list_events", {});
check(!r.error && /^Today is \w{3} \d+ \w{3} \d{4}, \d\d:\d\d/.test(r.text) && r.text.includes("0 event(s)"), "empty calendar, today's date first");

// adding
r = await call("add_event", { title: "FYP progress meeting", start: "2030-11-07 14:00", location: "Room 3", reminder_minutes: 30 });
check(!r.error && r.text.includes("Thu 7 Nov 2030, 14:00–15:00") && cal().events[0].end === "2030-11-07 15:00", "add_event: default 1 hour, weekday shown");
check(/^\[e1\] 2030-11-07 14:00–15:00 \| FYP progress meeting \| Location: Room 3 \| Reminder: 30 min$/m.test(r.text), "event line format (the UI parses it)");
r = await call("add_event", { title: "Lab", start: "2030-11-07T14:30", duration_minutes: 90 });
check(!r.error && r.text.includes("overlaps with \"FYP progress meeting\" (e1)") && cal().events[1].end === "2030-11-07 16:00", "overlap warning + duration");
r = await call("add_event", { title: "Old", start: "2001-01-01 10:00" });
check(!r.error && r.text.includes("in the past"), "past events get a note");
r = await call("add_event", { title: "Trip", start: "2030-11-10" });
check(!r.error && cal().events[3].all_day === true && r.text.includes("(all day)"), "date only = all-day event");
for (const [args, why] of [[{ title: "x", start: "next Thursday 2pm" }, "relative words"], [{ title: "x", start: "2030-02-30 10:00" }, "impossible date"],
  [{ title: "x", start: "2030-11-07 25:00" }, "bad hour"], [{ title: "", start: "2030-11-07 10:00" }, "empty title"],
  [{ title: "x", start: "2030-11-07 10:00", end: "2030-11-07 09:00" }, "end before start"], [{ title: "x", start: "2030-11-07 10:00", attendees: ["not-an-email"] }, "bad attendee"]]) {
  r = await call("add_event", args);
  check(r.error, `add_event refused: ${why}`);
}
check((await call("add_event", { title: "x", start: "tomorrow" })).text.includes("Today is"), "date errors tell the model today's date");

// reading
r = await call("list_events", { from: "2030-11-07", days: 1 });
check(r.text.includes("2 event(s)") && r.text.indexOf("[e1]") < r.text.indexOf("[e2]"), "list a day, sorted by time");
r = await call("search_events", { query: "room 3" });
check(r.text.includes("[e1]") && !r.text.includes("[e2]"), "search by location");
r = await call("find_free_time", { date: "2030-11-07", duration_minutes: 60 });
check(r.text.includes("- 09:00–14:00") && r.text.includes("- 16:00–18:00"), "free slots around the busy blocks");
r = await call("find_free_time", { date: "2030-11-07", duration_minutes: 300, day_start: "13:00", day_end: "17:00" });
check(r.text.includes("No free slot"), "no slot long enough");

// updating / deleting
r = await call("update_event", { id: "e1", start: "2030-11-07 16:00" });
check(!r.error && cal().events[0].end === "2030-11-07 17:00" && r.text.includes("Before:") && cal().events[0].location === "Room 3", "move keeps length and other fields");
r = await call("update_event", { id: "e1", location: "" });
check(!r.error && cal().events[0].location === undefined, "clear a field");
r = await call("update_event", { id: "e99", title: "x" });
check(r.error, "unknown id");
r = await call("delete_event", { id: "e3" });
check(!r.error && !cal().events.some((e) => e.id === "e3"), "delete");

// .ics
r = await call("import_ics", { path: "invite.ics" });
const imp = cal().events.find((e) => e.title === "Thesis review, chapter 3");
check(!r.error && r.text.includes("Imported 2 event(s)") && imp && imp.location === "Room 12" && imp.reminder_minutes === 60 && imp.notes.includes("folded by the sender"),
  "import: escapes, folded lines, alarm");
const local = new Date(Date.UTC(2030, 10, 4, 9, 0));
check(imp.start === `2030-11-04 ${String(local.getHours()).padStart(2, "0")}:00`, "import: UTC time converted to local");
check(cal().events.some((e) => e.title === "Holiday" && e.all_day), "import: all-day event");
r = await call("import_ics", { path: "invite.ics" });
check(r.text.includes("Imported 0 event(s)") && r.text.includes("skipped 2"), "import twice = no duplicates");
for (const [p, why] of [["../outside.ics", "parent path"], [path.join(root, "x.ics"), "absolute path"], ["missing.ics", "missing file"]]) {
  r = await call("import_ics", { path: p });
  check(r.error, `import refused: ${why}`);
}
r = await call("export_ics", { ids: ["e1", "e2"] });
const ics = fs.readFileSync(path.join(ws, "calendar-export.ics"), "utf8");
check(!r.error && ics.startsWith("BEGIN:VCALENDAR") && (ics.match(/BEGIN:VEVENT/g) || []).length === 2 && ics.includes("TRIGGER:-PT30M") && ics.includes("\r\n"), "export: valid .ics with reminders");
r = await call("export_ics", { ids: ["e1"] });
check(!r.error && fs.existsSync(path.join(ws, "calendar-export (1).ics")), "export never overwrites");
r = await call("export_ics", { ids: ["e1"], path: "../escape.ics" });
check(r.error && !fs.existsSync(path.join(root, "escape.ics")), "export outside the workspace refused");
r = await call("export_ics", { ids: ["e1"], path: ".git/x.ics" });
check(r.error, "export into .git refused");
// round trip: what we export, we can import back
fs.copyFileSync(path.join(ws, "calendar-export (1).ics"), path.join(ws, "rt.ics"));
await call("delete_event", { id: "e1" });
r = await call("import_ics", { path: "rt.ics" });
const back = cal().events.find((e) => e.title === "FYP progress meeting");
check(back && back.start === "2030-11-07 16:00" && back.end === "2030-11-07 17:00" && back.reminder_minutes === 30, "export → import round trip");

check(fs.readFileSync(path.join(root, "calls.jsonl"), "utf8").trim().split("\n").length >= 30, "call log written");
console.log(`\nALL ${n} CALENDAR TESTS PASSED`);
child.kill();
fs.rmSync(root, { recursive: true, force: true });