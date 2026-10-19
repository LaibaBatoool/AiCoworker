// Compares benchmark runs with a feature OFF vs ON (same model, real verdicts only).
//   node benchmark/compare-routing.mjs                          -> tool routing, all tasks in both modes
//   node benchmark/compare-routing.mjs S1,A3                    -> only these tasks
//   node benchmark/compare-routing.mjs S2,S7 claim_check        -> compare the claim check instead
// Rows with no "routing" field are routing OFF (the benchmark default); rows tagged
// routing:"v1" (first keyword list, finding 23) are left out of both columns.
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
    prompt: n ? Math.round(sum("prompt_tokens") / n) : 0,
    calls: n ? (calls / n).toFixed(1) : "0",
    perCall: calls ? Math.round(sum("prompt_tokens") / calls) : 0,
  };
};

const pad = (s, n) => String(s).padEnd(n);
console.log(`${by} OFF vs ON  (model ${model}; prompt tokens per API call is the fair comparison)\n`);
console.log(pad("Task", 6) + pad("Mode", 6) + pad("Trials", 8) + pad("Pass", 6) + pad("Avg tok", 9) + pad("API calls", 11) + "Prompt tok / call");
console.log("-".repeat(62));
let any = false;
for (const id of ids) {
  const off = stats(real.filter((r) => r.id === id && (r[by] ?? false) === false));
  const on = stats(real.filter((r) => r.id === id && r[by] === true));
  if (!off.n || !on.n) continue;
  any = true;
  for (const [mode, s] of [["off", off], ["on", on]])
    console.log(pad(id, 6) + pad(mode, 6) + pad(s.n, 8) + pad(`${s.pass}/${s.n}`, 6) + pad(s.tok, 9) + pad(s.calls, 11) + s.perCall);
  const cut = off.perCall ? Math.round((1 - on.perCall / off.perCall) * 100) : 0;
  console.log(pad("", 12) + `→ prompt tokens per call ${cut >= 0 ? "-" : "+"}${Math.abs(cut)}%\n`);
}
if (!any) console.log(`No task has finished runs in BOTH modes yet for ${by}.`);