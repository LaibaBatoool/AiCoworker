// Compares benchmark runs with a feature OFF vs ON (same model, real verdicts only).
//   node benchmark/compare-routing.mjs                          -> tool routing, all tasks in both modes
//   node benchmark/compare-routing.mjs S1,A3                    -> only these tasks
//   node benchmark/compare-routing.mjs S2,S7 claim_check        -> compare the claim check instead
//   node benchmark/compare-routing.mjs S2,A3 checklist          -> checklist: off vs optional vs required plan
// Rows with no "routing" field are routing OFF (the benchmark default); rows tagged
// routing:"v1" (first keyword list, finding 23) are left out of both columns.
// Checklist rows: checklist:false = off; checklist:true without checklist_mode = "optional"
// (finding 32 batch); checklist_mode:"required" = plan request + plan check.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const RUNS = path.join(path.dirname(fileURLToPath(import.meta.url)), "runs.json");
const runs = JSON.parse(fs.readFileSync(RUNS, "utf8"));
const only = (process.argv[2] || "").split(",").map((s) => s.trim()).filter(Boolean);
const FEATURES = ["routing", "claim_check", "checklist"];
const by = process.argv[3] || "routing"; // one of FEATURES
if (!FEATURES.includes(by)) { console.log(`Unknown feature "${by}". Use one of: ${FEATURES.join(", ")}`); process.exit(1); }
const others = FEATURES.filter((f) => f !== by); // held OFF on both sides, so only one thing changes
const model = "openai/gpt-oss-120b";

// which mode a row is in, for the feature being compared
const modeOf = (r) => {
  if (by === "checklist") return r.checklist ? r.checklist_mode || "optional" : "off";
  if (r[by] === true) return "on";
  return (r[by] ?? false) === false ? "off" : null; // e.g. routing:"v1" -> left out
};
const MODES = by === "checklist" ? ["off", "optional", "required"] : ["off", "on"];

const real = runs.filter((r) => r.model === model && r.verify_status !== "API_ERROR" && (r.total_tokens || 0) > 0 && others.every((f) => (r[f] ?? false) === false));
const ids = [...new Set(real.map((r) => r.id))].filter((id) => !only.length || only.includes(id));

const stats = (rs) => {
  const n = rs.length;
  const sum = (k) => rs.reduce((a, r) => a + (r[k] || 0), 0);
  const calls = sum("api_calls");
  return {
    n,
    pass: rs.filter((r) => r.verify_status === "PASS").length,
    tok: n ? Math.round(sum("total_tokens") / n) : 0,
    calls: n ? (calls / n).toFixed(1) : "0",
    perCall: calls ? Math.round(sum("prompt_tokens") / calls) : 0,
    plans: rs.filter((r) => r.checklist_used).length,
    gates: rs.filter((r) => (r.checklist_checks || 0) > 0 || (r.plan_checks || 0) > 0).length,
  };
};

const pad = (s, n) => String(s).padEnd(n);
const extra = by === "checklist";
console.log(`${by}: ${MODES.join(" vs ")}  (model ${model}; prompt tokens per API call is the fair comparison)\n`);
console.log(pad("Task", 6) + pad("Mode", 10) + pad("Trials", 8) + pad("Pass", 6) + pad("Avg tok", 9) + pad("API calls", 11) + pad("Prompt tok / call", 19) + (extra ? pad("Plan made", 11) + "Gate fired" : ""));
console.log("-".repeat(extra ? 100 : 69));
let any = false;
for (const id of ids) {
  const per = MODES.map((m) => [m, stats(real.filter((r) => r.id === id && modeOf(r) === m))]);
  const off = per[0][1];
  if (!off.n || per.slice(1).every(([, s]) => !s.n)) continue;
  any = true;
  for (const [mode, s] of per) {
    if (!s.n) continue;
    const cut = mode !== "off" && off.perCall ? Math.round((1 - s.perCall / off.perCall) * 100) : null;
    const delta = cut === null ? "" : ` (${cut >= 0 ? "-" : "+"}${Math.abs(cut)}%)`;
    console.log(pad(id, 6) + pad(mode, 10) + pad(s.n, 8) + pad(`${s.pass}/${s.n}`, 6) + pad(s.tok, 9) + pad(s.calls, 11) + pad(s.perCall + delta, 19) +
      (extra ? pad(`${s.plans}/${s.n}`, 11) + `${s.gates}/${s.n}` : ""));
  }
  console.log("");
}
if (!any) console.log(`No task has finished runs both OFF and ON yet for ${by}.`);