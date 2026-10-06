// AI CoWorker — Benchmark Dashboard generator
// Usage: node benchmark/dashboard.mjs   → writes benchmark/report.html

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const RESULTS = path.join(ROOT, "results.json");
const OUT = path.join(ROOT, "report.html");

let runs = [];
try { runs = JSON.parse(fs.readFileSync(RESULTS, "utf8")); } catch { /* no results yet */ }

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const pct = (n, d) => (d ? Math.round((n / d) * 100) : 0);

const total = runs.length;
const passed = runs.filter((r) => r.status === "PASS").length;
const failed = runs.filter((r) => r.status === "FAIL").length;
const advFails = runs.filter((r) => r.bucket === "adversarial" && r.status === "FAIL").length;
const advRuns = runs.filter((r) => r.bucket === "adversarial").length;

const buckets = ["synthetic", "real-world", "adversarial"].map((b) => {
  const rs = runs.filter((r) => r.bucket === b);
  const p = rs.filter((r) => r.status === "PASS").length;
  return { name: b, runs: rs.length, passed: p, rate: pct(p, rs.length) };
});

const byTask = {};
for (const r of runs) (byTask[r.id] ||= []).push(r);
const taskRows = Object.entries(byTask).map(([id, rs]) => {
  const p = rs.filter((r) => r.status === "PASS").length;
  const last = rs[rs.length - 1];
  return `<tr><td class="mono">${esc(id)}</td><td>${esc(last.bucket)}</td><td>${p}/${rs.length}</td>
  <td><span class="pill ${last.status.toLowerCase()}">${esc(last.status)}</span></td><td class="dim">${esc(last.reason)}</td></tr>`;
}).join("");

const historyRows = [...runs].reverse().map((r) =>
  `<tr><td class="dim mono">${esc(new Date(r.at).toLocaleString())}</td><td class="mono">${esc(r.id)}</td>
  <td><span class="pill ${r.status.toLowerCase()}">${esc(r.status)}</span></td><td class="dim">${esc(r.reason)}</td></tr>`
).join("");

const bucketBars = buckets.map((b) => `
  <div class="bucket">
    <div class="row"><span>${esc(b.name)}</span><span class="dim">${b.runs ? `${b.passed}/${b.runs} · ${b.rate}%` : "no runs yet"}</span></div>
    <div class="bar"><div class="fill" style="width:${b.rate}%"></div></div>
  </div>`).join("");

const safety = advFails === 0
  ? `<div class="safe">0 safety violations across ${advRuns} adversarial run${advRuns === 1 ? "" : "s"}</div>`
  : `<div class="unsafe">${advFails} adversarial FAIL${advFails === 1 ? "" : "s"} — review before claiming safety</div>`;

const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>AI CoWorker — Benchmark Report</title>
<style>
  :root { --bg:#0b0b17; --card:#14142a; --line:#26264a; --text:#ecebff; --dim:#8c8bb0; --v:#7c5cff; --b:#3fa9ff; --ok:#2dd4bf; --bad:#ff5d73; --warn:#ffb454; }
  * { box-sizing:border-box; }
  body { margin:0; background:var(--bg); color:var(--text); font-family:Inter,Segoe UI,system-ui,sans-serif; padding:32px 20px; }
  .wrap { max-width:960px; margin:0 auto; }
  h1 { font-size:26px; margin:0 0 4px; } h2 { font-size:15px; margin:0 0 14px; color:var(--dim); font-weight:600; letter-spacing:.04em; text-transform:uppercase; }
  .sub { color:var(--dim); margin-bottom:24px; }
  .grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(180px,1fr)); gap:14px; margin-bottom:18px; }
  .card { background:var(--card); border:1px solid var(--line); border-radius:14px; padding:18px; margin-bottom:18px; }
  .grid .card { margin:0; }
  .big { font-size:38px; font-weight:700; background:linear-gradient(90deg,var(--v),var(--b)); -webkit-background-clip:text; background-clip:text; color:transparent; }
  .label { color:var(--dim); font-size:13px; }
  .bucket { margin-bottom:14px; } .row { display:flex; justify-content:space-between; margin-bottom:6px; font-size:14px; }
  .bar { height:10px; background:#1d1d3b; border-radius:99px; overflow:hidden; }
  .fill { height:100%; background:linear-gradient(90deg,var(--v),var(--b)); border-radius:99px; }
  .safe { color:var(--ok); border:1px solid var(--ok); border-radius:10px; padding:12px 14px; background:rgba(45,212,191,.07); }
  .unsafe { color:var(--bad); border:1px solid var(--bad); border-radius:10px; padding:12px 14px; background:rgba(255,93,115,.07); }
  table { width:100%; border-collapse:collapse; font-size:14px; } th { text-align:left; color:var(--dim); font-weight:600; padding:8px 10px; border-bottom:1px solid var(--line); }
  td { padding:10px; border-bottom:1px solid var(--line); vertical-align:top; } tr:last-child td { border-bottom:none; }
  .scroll { overflow-x:auto; } .dim { color:var(--dim); } .mono { font-family:ui-monospace,Consolas,monospace; }
  .pill { padding:2px 10px; border-radius:99px; font-size:12px; font-weight:600; }
  .pill.pass { background:rgba(45,212,191,.15); color:var(--ok); } .pill.fail { background:rgba(255,93,115,.15); color:var(--bad); }
</style></head><body><div class="wrap">
  <h1>AI CoWorker — Agent Workstation Benchmark</h1>
  <div class="sub">Generated ${esc(new Date().toLocaleString())} · verifiers check real workspace state, not the agent's claims</div>

  <div class="grid">
    <div class="card"><div class="big">${pct(passed, total)}%</div><div class="label">overall pass rate</div></div>
    <div class="card"><div class="big">${total}</div><div class="label">verified runs</div></div>
    <div class="card"><div class="big">${passed}</div><div class="label">passed</div></div>
    <div class="card"><div class="big">${failed}</div><div class="label">failed</div></div>
  </div>

  <div class="card"><h2>Safety</h2>${safety}</div>
  <div class="card"><h2>By bucket</h2>${bucketBars}</div>
  <div class="card"><h2>Tasks</h2><div class="scroll"><table>
    <tr><th>Task</th><th>Bucket</th><th>Passed</th><th>Latest</th><th>Reason</th></tr>${taskRows || '<tr><td colspan="5" class="dim">No runs yet</td></tr>'}</table></div></div>
  <div class="card"><h2>Run history</h2><div class="scroll"><table>
    <tr><th>Time</th><th>Task</th><th>Status</th><th>Reason</th></tr>${historyRows || '<tr><td colspan="4" class="dim">No runs yet</td></tr>'}</table></div></div>
</div></body></html>`;

fs.writeFileSync(OUT, html);
console.log(`Dashboard written: ${OUT}`);