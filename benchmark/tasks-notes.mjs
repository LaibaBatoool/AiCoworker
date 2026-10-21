// Notes & to-do tasks. Merged into TASKS by bench.mjs.
// run.mjs writes benchmark/.mcp-notes.json pointing at mcp-servers/notes/server.mjs:
//   data = <task>/notes.json, call log = <task>/notes_calls.jsonl (next to the workspace).
import fs from "node:fs";
import path from "node:path";

const write = (p, c) => { fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, c); };
const read = (p) => (fs.existsSync(p) ? fs.readFileSync(p, "utf8") : null);
const pass = (reason = "ok") => ({ status: "PASS", reason });
const fail = (reason) => ({ status: "FAIL", reason });
const inconclusive = (reason) => ({ status: "INCONCLUSIVE", reason });
const status = (w) => { try { return JSON.parse(read(path.join(w, "..", "agent_trace.json"))).status; } catch { return null; } };

const MEETING = `# FYP progress meeting — 20 Oct 2026

Attendees: Laiba (me), Dr. Ahmed, Bilal

## Discussion
- Benchmark: email tasks pass, A14 still to run.
- Thesis: results chapter is in good shape.

## Action items
- Laiba: send the benchmark results table to Dr. Ahmed — by 23 Oct 2026
- Bilal: book the lab GPU for the local models — by 24 Oct 2026
- Laiba: finish the email and calendar section of the thesis — by 27 Oct 2026
- Dr. Ahmed: share last year's thesis template — by 22 Oct 2026
- Laiba: practise the demo (high priority) — by 30 Oct 2026
`;

// the user's own items: [what the title must mention, due date]
const MINE = [
  [/benchmark|results/i, "2026-10-23", "send results table"],
  [/thesis|email|calendar/i, "2026-10-27", "finish thesis section"],
  [/demo|practi/i, "2026-10-30", "practise the demo"],
];
const OTHERS = [/gpu/i, /template/i];

export const NOTES_TASKS = {
  S17: {
    mcp: "notes",
    bucket: "synthetic",
    prompt: "Read meeting-notes.md and add each of my action items (I'm Laiba) to my to-do list with its due date. Skip the items that belong to other people.",
    note: "Multi-item extraction with a filter: 3 of the 5 action items are the user's. Checks the to-do list, not the agent's answer.",
    setup(w) {
      write(path.join(w, "meeting-notes.md"), MEETING);
      write(path.join(w, "..", "notes.json"), JSON.stringify({ next_note: 1, next_task: 2, notes: [], tasks: [
        { id: "t1", title: "Pay hostel fee", due: "2026-10-31", created: "2026-10-19 10:00" }] }, null, 2));
    },
    verify(w) {
      let d;
      try { d = JSON.parse(read(path.join(w, "..", "notes.json"))); } catch { d = { tasks: [] }; }
      const added = (d.tasks || []).filter((t) => t.id !== "t1");
      if (!(d.tasks || []).some((t) => t.id === "t1")) return fail("the existing task 'Pay hostel fee' was removed");
      if (!added.length) return status(w) === "done" ? fail("FALSE COMPLETION: no task was added") : inconclusive("no task added — agent did not finish");
      const problems = [];
      for (const [re, dueDay, label] of MINE) {
        const hit = added.filter((t) => re.test(t.title));
        if (!hit.length) problems.push(`missing: ${label}`);
        else if (!hit.some((t) => String(t.due || "").slice(0, 10) === dueDay)) problems.push(`${label}: due ${hit[0].due ?? "none"} (want ${dueDay})`);
      }
      const wrong = added.filter((t) => OTHERS.some((re) => re.test(t.title)));
      if (wrong.length) problems.push(`added other people's items: ${wrong.map((t) => t.title).join("; ")}`);
      if (added.length > MINE.length) problems.push(`${added.length} tasks added (want 3)`);
      if (problems.length) return fail(problems.join("; "));
      const demo = added.find((t) => /demo|practi/i.test(t.title));
      return pass(`3 of 3 own items with correct due dates, others skipped${demo?.priority === "high" ? "; demo marked high priority" : ""}`);
    },
  },
};