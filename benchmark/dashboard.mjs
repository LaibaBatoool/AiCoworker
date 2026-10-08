// AI CoWorker — Benchmark Dashboard generator
// Usage: node benchmark/dashboard.mjs   → writes benchmark/report.html

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const RESULTS = path.join(ROOT, "results.json");
const RUNS = path.join(ROOT, "runs.json");
const OUT = path.join(ROOT, "report.html");

let runs = [];
try { runs = JSON.parse(fs.readFileSync(RESULTS, "utf8")); } catch { /* no results yet */ }
let batch = [];
try { batch = JSON.parse(fs.readFileSync(RUNS, "utf8")); } catch { /* no automatic runs yet */ }

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

// ---- automatic runner (runs.json): pass rate over trials + efficiency metrics
const byBatch = {};
for (const r of batch) (byBatch[r.id] ||= []).push(r);
const avg = (rs, k) => (rs.length ? rs.reduce((a, r) => a + (r[k] || 0), 0) / rs.length : 0);
const batchRows = Object.entries(byBatch).map(([id, rs]) => {
  const p = rs.filter((r) => r.verify_status === "PASS").length;
  const f = rs.filter((r) => r.verify_status === "FAIL").length;
  const inc = rs.filter((r) => r.verify_status === "INCONCLUSIVE").length;
  const decided = p + f;
  const cls = f > 0 ? "fail" : p > 0 ? "pass" : "";
  return `<tr><td class="mono">${esc(id)}</td><td>${esc(rs[0].bucket)}</td><td>${rs.length}</td>
  <td><span class="pill ${cls}">${decided ? `${p}/${decided}` : "–"}</span></td><td class="dim">${inc ? inc + " inconcl." : ""} ${rs.filter((r) => r.verify_status === "API_ERROR").length ? rs.filter((r) => r.verify_status === "API_ERROR").length + " API err" : ""}</td>
  <td>${avg(rs, "seconds").toFixed(1)}s</td><td>${avg(rs, "tool_calls").toFixed(1)}</td>
  <td>${avg(rs, "tool_errors").toFixed(1)}</td><td>${avg(rs, "confirmations").toFixed(1)}</td></tr>`;
}).join("");
const batchDecided = batch.filter((r) => r.verify_status === "PASS" || r.verify_status === "FAIL");
const batchPass = batchDecided.filter((r) => r.verify_status === "PASS").length;
const batchSummary = batch.length
  ? `<div class="label" style="margin-bottom:12px">${batch.length} automatic trials · pass rate ${pct(batchPass, batchDecided.length)}% (${batchPass}/${batchDecided.length} decided) · avg ${avg(batch, "seconds").toFixed(1)}s and ${avg(batch, "tool_calls").toFixed(1)} tool calls per run · model ${esc(batch[batch.length - 1].model || "")}</div>`
  : "";

const bucketBars = buckets.map((b) => `
  <div class="bucket">
    <div class="row"><span>${esc(b.name)}</span><span class="dim">${b.runs ? `${b.passed}/${b.runs} · ${b.rate}%` : "no runs yet"}</span></div>
    <div class="bar"><div class="fill" style="width:${b.rate}%"></div></div>
  </div>`).join("");

const batchAdvFails = batch.filter((r) => r.bucket === "adversarial" && r.verify_status === "FAIL").length;
const allAdvFails = advFails + batchAdvFails;
const allAdvRuns = advRuns + batch.filter((r) => r.bucket === "adversarial").length;
const safety = allAdvFails === 0
  ? `<div class="safe">0 safety violations across ${allAdvRuns} adversarial run${allAdvRuns === 1 ? "" : "s"}</div>`
  : `<div class="unsafe">${allAdvFails} adversarial FAIL${allAdvFails === 1 ? "" : "s"} — review before claiming safety</div>`;

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
  <div class="card"><h2>Automatic runs (repeated trials)</h2>${batchSummary}<div class="scroll"><table>
    <tr><th>Task</th><th>Bucket</th><th>Trials</th><th>Pass</th><th>Inconcl.</th><th>Avg time</th><th>Avg calls</th><th>Avg tool errors</th><th>Avg confirms</th></tr>${batchRows || '<tr><td colspan="9" class="dim">No automatic runs yet — run: node benchmark/run.mjs</td></tr>'}</table></div></div>
  <div class="card"><h2>By bucket (manual verifies)</h2>${bucketBars}</div>
  <div class="card"><h2>Tasks (manual verifies)</h2><div class="scroll"><table>
    <tr><th>Task</th><th>Bucket</th><th>Passed</th><th>Latest</th><th>Reason</th></tr>${taskRows || '<tr><td colspan="5" class="dim">No runs yet</td></tr>'}</table></div></div>
  <div class="card"><h2>Run history</h2><div class="scroll"><table>
    <tr><th>Time</th><th>Task</th><th>Status</th><th>Reason</th></tr>${historyRows || '<tr><td colspan="4" class="dim">No runs yet</td></tr>'}</table></div></div>
</div></body></html>`;

fs.writeFileSync(OUT, html);
console.log(`Dashboard written: ${OUT}`);