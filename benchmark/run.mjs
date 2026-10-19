// AI CoWorker — one-command benchmark runner (v3: API-error aware, per-task confirm policy).
// Usage (from the repo root):
//   node benchmark/run.mjs                              run ALL tasks, 3 trials each
//   node benchmark/run.mjs --tasks S1,S2 --trials 1     pick tasks / trials
// Options: --model <name>  --delay <sec between trials, default 20>  --timeout <sec per run, default 300>
//          --confirm allow|deny (default allow)  --skip-build  --retries <API retries, default 2>  --retry-wait <sec, default 45>

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import dns from "node:dns/promises";

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(ROOT, "..");
const TAURI = path.join(REPO, "src-tauri");
const RUNS = path.join(ROOT, "runs.json");
const BENCH = path.join(ROOT, "bench.mjs");
const EXE = path.join(TAURI, "target", "debug", "examples", "bench_run" + (process.platform === "win32" ? ".exe" : ""));

const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : def;
};
const flag = (name) => process.argv.includes(`--${name}`);

const trials = Number(arg("trials", "3"));
const model = arg("model", "openai/gpt-oss-120b");
const delay = Number(arg("delay", "20"));
const timeout = Number(arg("timeout", "300"));
const confirm = arg("confirm", "allow");
const retries = Number(arg("retries", "2"));
const retryWait = Number(arg("retry-wait", "45"));
const only = arg("tasks", "").split(",").map((s) => s.trim()).filter(Boolean);
// --since 2026-10-12 : count trials already finished (not API errors) since that date
// for the same model, and only run the missing ones (re-running a batch costs nothing extra)
const since = arg("since", "");
// tool routing on/off (env AICOWORKER_TOOL_ROUTING) is recorded per trial, and --since only
// counts trials made in the SAME mode, so routing-on and routing-off results never mix
const ROUTING = /^(on|1|true)$/i.test(process.env.AICOWORKER_TOOL_ROUTING || "");
// claim check on/off (env AICOWORKER_CLAIM_CHECK), recorded and matched the same way
const CLAIM = /^(on|1|true)$/i.test(process.env.AICOWORKER_CLAIM_CHECK || "");
const BENCH_VERSION = (fs.readFileSync(BENCH, "utf8").match(/\((v[\d.]+)\)/) || [])[1] || "unknown";

const sleep = (s) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, s * 1000);
const bench = (args) => spawnSync(process.execPath, [BENCH, ...args], { encoding: "utf8" });
const isApiFailure = (res) => ["error", "crash", "timeout"].includes(res.status);
// Groq's DAILY limits (tokens/requests per day) reset in hours: retrying after 45 s is pointless
const isDailyLimit = (res) => isApiFailure(res) && /per day|\(TPD\)|\(RPD\)/i.test(String(res.reason || ""));
const retryHint = (res) => ((String(res.reason || "").match(/try again in ([0-9hms.]+)/i) || [])[1] || "").replace(/\.$/, "");

function listTasks() {
  const out = bench(["list"]).stdout || "";
  const tasks = [];
  for (const line of out.split(/\r?\n/)) {
    const m = line.match(/^(\S+)\s+\[([^\]]+)\]\s+(.*)$/);
    if (m) {
      const [bucket, ...tags] = m[2].split(":"); // e.g. "adversarial:deny" or "synthetic:mcp"
      const taskConfirm = tags.find((x) => x === "deny" || x === "allow") || null;
      tasks.push({ id: m[1], bucket, confirm: taskConfirm, mcp: tags.includes("mcp"), prompt: m[3] });
    }
  }
  return tasks;
}

function loadRuns() {
  try { return JSON.parse(fs.readFileSync(RUNS, "utf8")); } catch { return []; }
}

// MCP config for the benchmark's own test server (absolute paths, regenerated every run)
const MCP_BENCH_CONFIG = path.join(ROOT, ".mcp-bench.json");
fs.writeFileSync(MCP_BENCH_CONFIG, JSON.stringify({
  servers: {
    bench: {
      command: process.execPath, // this exact node.exe, no PATH lookup
      args: [path.join(ROOT, "mcp-test-server.mjs")],
      tier: "safe", // read-only test tools; the user decides tiers, never the server
    },
  },
}, null, 2));

function runAgent(t) {
  bench(["setup", t.id]); // fresh workspace for every attempt
  const ws = path.join(ROOT, "workspaces", t.id, "ws");
  // a task can pin its own confirmation policy (A9/A9b: careful user = deny)
  const r = spawnSync(EXE, [ws, model, String(timeout), t.confirm || confirm, t.prompt], {
    cwd: TAURI,
    env: {
      ...process.env,
      AICOWORKER_NO_LAUNCH: "1", // open_in_default_app validates but never launches
      // MCP only for MCP tasks (keeps every other task's tool list + token cost identical);
      // a missing file means "no MCP servers" — so the user's own %APPDATA% config never leaks in
      AICOWORKER_MCP_CONFIG: t.mcp ? MCP_BENCH_CONFIG : path.join(ROOT, ".mcp-none.json"),
    },
    encoding: "utf8",
    timeout: (timeout + 60) * 1000,
    maxBuffer: 50e6,
  });
  const line = (r.stdout || "").split(/\r?\n/).find((l) => l.startsWith("BENCH_RESULT:"));
  try { return JSON.parse(line.slice("BENCH_RESULT:".length)); }
  catch { return { status: "crash", reason: (r.stderr || String(r.error || "no output")).slice(-300), seconds: 0 }; }
}

// ---- 1. build the headless runner once
if (!flag("skip-build")) {
  console.log("Building the headless runner (first time takes a few minutes)...");
  const b = spawnSync("cargo", ["build", "--example", "bench_run"], { cwd: TAURI, stdio: "inherit" });
  if (b.status !== 0) {
    console.error("\nBuild failed. Copy the error above and send it to Claude.");
    process.exit(1);
  }
}
if (!fs.existsSync(EXE)) {
  console.error(`Runner not found at ${EXE}. Run without --skip-build.`);
  process.exit(1);
}

// ---- 1b. pre-flight: fail fast instead of burning minutes of retries
const localModel = model.startsWith("ollama:") ? model.slice("ollama:".length) : null;
if (localModel) {
  // local model: Ollama must be running and the model must be pulled
  let names = null;
  try {
    const r = await fetch("http://127.0.0.1:11434/api/tags", { signal: AbortSignal.timeout(3000) });
    names = ((await r.json()).models || []).map((m) => m.name);
  } catch {
    console.error("\nOllama isn't reachable on 127.0.0.1:11434. Start the Ollama app (or run: ollama serve), then re-run.");
    process.exit(1);
  }
  if (!names.includes(localModel) && !names.includes(localModel + ":latest")) {
    console.error(`\nOllama is running but has no model "${localModel}". Installed: ${names.join(", ") || "(none)"}`);
    process.exit(1);
  }
} else {
  try {
    await dns.lookup("api.groq.com");
  } catch (e) {
    console.error(`\nCan't reach api.groq.com (DNS lookup failed: ${e.code}). This is a network problem on this PC, not the agent.`);
    console.error("Check:  Resolve-DnsName api.groq.com   and   Test-NetConnection api.groq.com -Port 443");
    console.error("Nothing was run or recorded.");
    process.exit(1);
  }
}

// ---- 2. choose tasks
let tasks = listTasks();
if (only.length) tasks = tasks.filter((t) => only.includes(t.id));
if (!tasks.length) { console.error("No matching tasks."); process.exit(1); }

console.log(`\nRunning ${tasks.length} task(s) x ${trials} trial(s) | model ${model} | confirmations: ${confirm} | benchmark ${BENCH_VERSION}${ROUTING ? " | tool routing: ON" : ""}${CLAIM ? " | claim check: ON" : ""} ${since ? ` | resuming since ${since}` : ""}\n`);

const session = [];
const all = loadRuns();
// trials already finished for this task (same model, since --since, real verdicts only)
const doneBefore = (id) =>
  since ? all.filter((r) => r.id === id && r.model === model && r.at >= since && r.verify_status !== "API_ERROR" && (r.routing ?? false) === ROUTING && (r.claim_check ?? false) === CLAIM).length : 0;
let apiErrorStreak = 0;
let aborted = false;

outer:
for (const t of tasks) {
  const already = doneBefore(t.id);
  if (already >= trials) {
    console.log(`[${t.id}] already has ${already} finished trial(s) since ${since} — skipped`);
    continue;
  }
  for (let n = already + 1; n <= trials; n++) {
    const label = `[${t.id} ${n}/${trials}${t.confirm ? " confirm:" + t.confirm : ""}]`;

    // run the real agent, retrying when the API itself failed (but never for a DAILY limit)
    let res = runAgent(t);
    let attempt = 0;
    while (isApiFailure(res) && !isDailyLimit(res) && attempt < retries) {
      attempt++;
      console.log(`${label} API problem: ${String(res.reason || res.status).slice(0, 160)}`);
      console.log(`${label} waiting ${retryWait * attempt}s, then retry ${attempt}/${retries}...`);
      sleep(retryWait * attempt);
      res = runAgent(t);
    }

    let verifyStatus, verifyReason;
    if (isApiFailure(res)) {
      // The agent never got to do the task. Do NOT verify, and do NOT count it as agent behaviour.
      verifyStatus = "API_ERROR";
      verifyReason = String(res.reason || res.status).slice(0, 200);
      apiErrorStreak++;
    } else {
      apiErrorStreak = 0;
      const v = bench(["verify", t.id]).stdout || "";
      const vm = v.match(/\[\S+\]\s+(PASS|FAIL|INCONCLUSIVE)\s+—\s+(.*)/);
      verifyStatus = vm ? vm[1] : "ERROR";
      verifyReason = vm ? vm[2] : v.trim().slice(0, 200);
    }

    const rec = {
      id: t.id, bucket: t.bucket, trial: n, at: new Date().toISOString(), model, confirm: t.confirm || confirm,
      bench_version: BENCH_VERSION,
      routing: ROUTING,
      claim_check: CLAIM,
      agent_status: res.status, agent_reason: res.reason || "",
      seconds: Number((res.seconds || 0).toFixed(1)),
      model_turns: res.model_turns || 0, tool_calls: res.tool_calls || 0, tool_errors: res.tool_errors || 0,
      confirmations: res.confirmations || 0, tools: res.tools || {},
      prompt_tokens: res.prompt_tokens || 0, completion_tokens: res.completion_tokens || 0,
      total_tokens: res.total_tokens || 0, api_calls: res.api_calls || 0, api_retries: res.api_retries || 0,
      verify_status: verifyStatus, verify_reason: verifyReason,
    };
    session.push(rec);
    all.push(rec);
    fs.writeFileSync(RUNS, JSON.stringify(all, null, 2)); // saved after every trial

    console.log(`${label} agent:${rec.agent_status} | ${rec.seconds}s | ${rec.tool_calls} calls | ${rec.tool_errors} tool errors | ${rec.confirmations} confirms | ${rec.total_tokens} tok | ${verifyStatus}${verifyStatus === "PASS" ? "" : " — " + verifyReason}`);

    if (isDailyLimit(res)) {
      const hint = retryHint(res);
      console.log(`\nSTOPPING: Groq's DAILY limit for ${model} is used up${hint ? ` — it frees up again in about ${hint}` : ""}.`);
      console.log(`Finished trials are saved. Next time add  --since ${since || new Date().toISOString().slice(0, 10)}  to the same command and only the missing trials will run.`);
      aborted = true;
      break outer;
    }
    if (apiErrorStreak >= 3) {
      const lastReason = String(session[session.length - 1]?.verify_reason || "");
      if (/dns error|connect|timed out|network/i.test(lastReason)) {
        console.log("\nSTOPPING EARLY: 3 API failures in a row, and they look like a NETWORK/DNS problem on this PC (not a rate limit).");
        console.log("Check:  Resolve-DnsName api.groq.com   then run again. Finished trials are already saved.");
      } else {
        console.log("\nSTOPPING EARLY: 3 API failures in a row. This usually means the free-tier rate/daily limit is used up.");
        console.log("Wait a while (or use another key / model with --model), then run again. Finished trials are already saved.");
      }
      aborted = true;
      break outer;
    }
    if (!(t === tasks[tasks.length - 1] && n === trials)) sleep(delay); // be gentle with the rate limit
  }
}

// ---- 3. summary
console.log("\nTask  Trials  Pass  Fail  Inconcl  API err  Avg s   Avg calls  Avg tokens");
console.log("----  ------  ----  ----  -------  -------  ------  ---------  ----------");
for (const t of tasks) {
  const rs = session.filter((r) => r.id === t.id);
  if (!rs.length) continue;
  const c = (s) => rs.filter((r) => r.verify_status === s).length;
  const avg = (k) => (rs.reduce((a, r) => a + r[k], 0) / rs.length).toFixed(1);
  console.log(`${t.id.padEnd(4)}  ${String(rs.length).padEnd(6)}  ${String(c("PASS")).padEnd(4)}  ${String(c("FAIL")).padEnd(4)}  ${String(c("INCONCLUSIVE")).padEnd(7)}  ${String(c("API_ERROR")).padEnd(7)}  ${avg("seconds").padEnd(6)}  ${avg("tool_calls").padEnd(9)}  ${Math.round(avg("total_tokens"))}`);
}
console.log(aborted ? "\nStopped early (see above). Partial results saved." : "\nDone. Now run: node benchmark/dashboard.mjs");