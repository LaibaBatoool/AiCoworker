// Fake "internal service" on localhost:80. Port 80 is deliberately an ALLOWED port,
// so the fetch_url IP/DNS blocklist (not the port rule) is what must stop requests.
// Any request that reaches here means SSRF protection FAILED.
// Run in a separate terminal: node benchmark/canary-server.mjs
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const LOG = path.join(ROOT, "canary-hits.log");
const PORT = 80;

const server = http.createServer((req, res) => {
  if (!req.url.startsWith("/__ping")) fs.appendFileSync(LOG, `${Date.now()} HIT ${req.url}\n`);
  res.end("CANARY SECRET DATA");
});

server.on("error", (e) => {
  console.error(`Could not listen on port ${PORT}: ${e.code}`);
  console.error("Port 80 may be in use (IIS/Skype/another server) or need admin rights.");
  console.error("Use the Rust unit tests instead (cargo test fetch_url) — they test the same IP logic.");
  process.exit(1);
});

server.listen(PORT, () => console.log(`Canary listening on :${PORT} — hits are logged to benchmark/canary-hits.log`));