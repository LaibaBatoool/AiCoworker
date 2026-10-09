// AI CoWorker — one-command benchmark runner (v2: API-error aware).
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

const sleep = (s) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, s * 1000);
const bench = (args) => spawnSync(process.execPath, [BENCH, ...args], { encoding: "utf8" });
const isApiFailure = (res) => ["error", "crash", "timeout"].includes(res.status);

function listTasks() {
  const out = bench(["list"]).stdout || "";
  const tasks = [];
  for (const line of out.split(/\r?\n/)) {
    const m = line.match(/^(\S+)\s+\[([^\]]+)\]\s+(.*)$/);
    if (m) {
      const [bucket, taskConfirm] = m[2].split(":"); // e.g. "adversarial:deny"
      tasks.push({ id: m[1], bucket, confirm: taskConfirm || null, prompt: m[3] });
    }
  }
  return tasks;
}

function loadRuns() {
  try { return JSON.parse(fs.readFileSync(RUNS, "utf8")); } catch { return []; }
}

function runAgent(t) {
  bench(["setup", t.id]); // fresh workspace for every attempt
  const ws = path.join(ROOT, "workspaces", t.id, "ws");
  // a task can pin its own confirmation policy (A9/A9b: careful user = deny)
  const r = spawnSync(EXE, [ws, model, String(timeout), t.confirm || confirm, t.prompt], {
    cwd: TAURI,
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

// ---- 1b. network pre-flight: fail fast instead of burning ~7 minutes of retries
try {
  await dns.lookup("api.groq.com");
} catch (e) {
  console.error(`\nCan't reach api.groq.com (DNS lookup failed: ${e.code}). This is a network problem on this PC, not the agent.`);
  console.error("Check:  Resolve-DnsName api.groq.com   and   Test-NetConnection api.groq.com -Port 443");
  console.error("Nothing was run or recorded.");
  process.exit(1);
}

// ---- 2. choose tasks
let tasks = listTasks();
if (only.length) tasks = tasks.filter((t) => only.includes(t.id));
if (!tasks.length) { console.error("No matching tasks."); process.exit(1); }

console.log(`\nRunning ${tasks.length} task(s) x ${trials} trial(s) | model ${model} | confirmations: ${confirm}\n`);

const session = [];
const all = loadRuns();
let apiErrorStreak = 0;
let aborted = false;

outer:
for (const t of tasks) {
  for (let n = 1; n <= trials; n++) {
    const label = `[${t.id} ${n}/${trials}${t.confirm ? " confirm:" + t.confirm : ""}]`;
    // run the real agent, retrying when the API itself failed
    let res = runAgent(t);
    let attempt = 0;
    while (isApiFailure(res) && attempt < retries) {
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
      agent_status: res.status, agent_reason: res.reason || "",
      seconds: Number((res.seconds || 0).toFixed(1)),
      model_turns: res.model_turns || 0, tool_calls: res.tool_calls || 0, tool_errors: res.tool_errors || 0,
      confirmations: res.confirmations || 0, tools: res.tools || {},
      verify_status: verifyStatus, verify_reason: verifyReason,
    };
    session.push(rec);
    all.push(rec);
    fs.writeFileSync(RUNS, JSON.stringify(all, null, 2)); // saved after every trial

    console.log(`${label} agent:${rec.agent_status} | ${rec.seconds}s | ${rec.tool_calls} calls | ${rec.tool_errors} tool errors | ${rec.confirmations} confirms | ${verifyStatus}${verifyStatus === "PASS" ? "" : " — " + verifyReason}`);

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
console.log("\nTask  Trials  Pass  Fail  Inconcl  API err  Avg s   Avg calls");
console.log("----  ------  ----  ----  -------  -------  ------  ---------");
for (const t of tasks) {
  const rs = session.filter((r) => r.id === t.id);
  if (!rs.length) continue;
  const c = (s) => rs.filter((r) => r.verify_status === s).length;
  const avg = (k) => (rs.reduce((a, r) => a + r[k], 0) / rs.length).toFixed(1);
  console.log(`${t.id.padEnd(4)}  ${String(rs.length).padEnd(6)}  ${String(c("PASS")).padEnd(4)}  ${String(c("FAIL")).padEnd(4)}  ${String(c("INCONCLUSIVE")).padEnd(7)}  ${String(c("API_ERROR")).padEnd(7)}  ${avg("seconds").padEnd(6)}  ${avg("tool_calls")}`);
}
console.log(aborted ? "\nStopped early (see above). Partial results saved." : "\nDone. Now run: node benchmark/dashboard.mjs");