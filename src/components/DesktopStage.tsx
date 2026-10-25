// Live-view visuals for the desktop tools (src-tauri/src/tools/desktop.rs):
//  - system_info: RAM / CPU / disk / battery meters
//  - list_processes: apps as memory bars;  list_windows: open windows
//  - close_app, clipboard_read / clipboard_write, take_screenshot: one card each
// The result arrives as JSON in card.preview (cut at 4,000 chars), so every
// parser falls back gracefully when the JSON is incomplete.
import "./desktopstage.css";

export interface DesktopCard {
  name: string;
  args: any;
  status: "running" | "ok" | "error" | "waiting" | "declined";
  preview: string;
}

export const DESKTOP_TOOLS = ["system_info", "list_processes", "list_windows", "close_app", "clipboard_read", "clipboard_write", "take_screenshot"];

export function describeDesktop(name: string, a: any): string {
  switch (name) {
    case "system_info": return "Checking this computer (RAM, disk, battery)";
    case "list_processes": return "Checking what is running";
    case "list_windows": return "Looking at the open windows";
    case "close_app": return `${a?.force ? "Force-closing" : "Closing"} ${a?.app ?? "an app"}`;
    case "clipboard_read": return "Reading the clipboard";
    case "clipboard_write": return "Copying text to the clipboard";
    case "take_screenshot": return "Taking a screenshot";
  }
  return name;
}

export function desktopIcon(name: string): string {
  const m: Record<string, string> = {
    system_info: "💻", list_processes: "📈", list_windows: "🪟", close_app: "✖️",
    clipboard_read: "📋", clipboard_write: "📋", take_screenshot: "📸",
  };
  return m[name] ?? "💻";
}

function parse(preview: string): any {
  try { return JSON.parse(preview); } catch { return null; }
}

function Meter({ label, pct, text, warn }: { label: string; pct: number; text: string; warn?: number }) {
  const p = Math.max(0, Math.min(100, pct || 0));
  const level = p >= (warn ?? 85) ? "dk-hot" : p >= 60 ? "dk-warm" : "";
  return (
    <div className="dk-meter">
      <div className="dk-meter-top"><span>{label}</span><span className="dk-muted">{text}</span></div>
      <div className="dk-bar"><div className={level} style={{ width: `${p}%` }} /></div>
    </div>
  );
}

function Waiting({ card, what }: { card: DesktopCard; what: string }) {
  if (card.status === "waiting") return <div className="dk-banner dk-wait">⏸ Waiting for your approval to {what}</div>;
  if (card.status === "declined") return <div className="dk-banner">⊘ You declined — nothing was done</div>;
  if (card.status === "error") return <div className="dk-banner dk-error">{card.preview.replace(/^Error: /, "")}</div>;
  if (card.status === "running") return <div className="dk-banner">Working…</div>;
  return null;
}

function SystemInfo({ card }: { card: DesktopCard }) {
  const v = parse(card.preview);
  if (!v) return <Waiting card={card} what="check the system" />;
  const ramUsed = (v.ram_total_gb ?? 0) - (v.ram_free_gb ?? 0);
  return (
    <div className="dk-panel">
      <div className="dk-head"><span>💻 This computer</span><span className="dk-muted">{v.os}</span></div>
      <div className="dk-body">
        <Meter label="Memory (RAM)" pct={v.ram_used_percent} text={`${ramUsed.toFixed(1)} of ${v.ram_total_gb} GB used`} />
        <Meter label="CPU load" pct={v.cpu_load_percent} text={`${v.cpu_load_percent ?? "?"}% · ${v.logical_cores ?? "?"} cores`} />
        {(v.disks ?? []).map((d: any) => (
          <Meter key={d.drive} label={`Disk ${d.drive}`} pct={d.total_gb ? ((d.total_gb - d.free_gb) / d.total_gb) * 100 : 0} warn={90}
            text={`${d.free_gb} GB free of ${d.total_gb} GB`} />
        ))}
        <div className="dk-facts">
          {v.battery_percent != null && <span>🔋 {v.battery_percent}%{v.plugged_in ? " · plugged in" : " · on battery"}</span>}
          {v.uptime_hours != null && <span>⏱ on for {v.uptime_hours} h</span>}
          {v.cpu && <span className="dk-muted">{v.cpu}</span>}
        </div>
      </div>
    </div>
  );
}

function Processes({ card }: { card: DesktopCard }) {
  const rows: any[] | null = parse(card.preview);
  if (!Array.isArray(rows)) return card.status === "ok" ? <pre className="dk-pre">{card.preview}</pre> : <Waiting card={card} what="list processes" />;
  const max = Math.max(1, ...rows.map((r) => r.memory_mb || 0));
  return (
    <div className="dk-panel">
      <div className="dk-head"><span>📈 Biggest apps by memory</span><span className="dk-muted">{rows.length} shown</span></div>
      <div className="dk-body">
        {rows.map((r, i) => (
          <div className="dk-proc dk-in" key={r.name + i} style={{ animationDelay: `${i * 40}ms` }}>
            <div className="dk-proc-name">{r.name}{r.processes > 1 && <span className="dk-muted"> ×{r.processes}</span>}
              {r.window_title && <div className="dk-proc-title">{r.window_title}</div>}</div>
            <div className="dk-proc-bar"><div style={{ width: `${(r.memory_mb / max) * 100}%` }} /></div>
            <div className="dk-proc-mb">{r.memory_mb >= 1024 ? `${(r.memory_mb / 1024).toFixed(1)} GB` : `${Math.round(r.memory_mb)} MB`}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

function Windows({ card }: { card: DesktopCard }) {
  const rows: any[] | null = parse(card.preview);
  if (!Array.isArray(rows)) return card.status === "ok" ? <pre className="dk-pre">{card.preview}</pre> : <Waiting card={card} what="list windows" />;
  return (
    <div className="dk-panel">
      <div className="dk-head"><span>🪟 Open windows</span><span className="dk-muted">{rows.length}</span></div>
      <div className="dk-wins">
        {rows.length === 0 && <div className="dk-muted">No open windows.</div>}
        {rows.map((w, i) => (
          <div className="dk-win dk-in" key={w.pid} style={{ animationDelay: `${i * 40}ms` }}>
            <div className="dk-win-bar"><span /><span /><span /></div>
            <div className="dk-win-title">{w.title}</div>
            <div className="dk-muted">{w.name} · pid {w.pid}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

function CloseApp({ card }: { card: DesktopCard }) {
  const rows: any[] | null = card.status === "ok" ? parse(card.preview) : null;
  return (
    <div className="dk-simple">
      <div className={`dk-simple-icon ${card.status === "ok" ? "dk-poof" : ""}`}>🪟</div>
      <div><b>{card.args?.app}</b>{card.args?.force && <span className="dk-hot-text"> · force (unsaved work is lost)</span>}</div>
      {Array.isArray(rows) ? rows.map((r) => <div key={r.pid} className="dk-muted">{r.name} (pid {r.pid}): {r.result}</div>) : <Waiting card={card} what={`close ${card.args?.app ?? "this app"}`} />}
    </div>
  );
}

function Clipboard({ card }: { card: DesktopCard }) {
  const reading = card.name === "clipboard_read";
  const v = reading && card.status === "ok" ? parse(card.preview) : null;
  const text: string = reading ? v?.text ?? "" : String(card.args?.text ?? "");
  const files: string[] = v?.files ?? [];
  const show = !reading || card.status === "ok";
  return (
    <div className="dk-panel">
      <div className="dk-head"><span>📋 {reading ? "Your clipboard" : "Copying to your clipboard"}</span>
        {card.status === "ok" && <span className="dk-ok">{reading ? "read" : "✓ ready to paste (Ctrl+V)"}</span>}</div>
      {show && (text || files.length > 0) ? (
        <div className="dk-clip">
          {text && <pre>{text.length > 1500 ? text.slice(0, 1500) + "…" : text}</pre>}
          {files.map((f) => <div key={f} className="dk-muted">📄 {f}</div>)}
        </div>
      ) : show && card.status === "ok" ? <div className="dk-banner">The clipboard is empty.</div> : null}
      <Waiting card={card} what={reading ? "read your clipboard (it may hold passwords)" : "copy this"} />
    </div>
  );
}

function Screenshot({ card }: { card: DesktopCard }) {
  const v = card.status === "ok" ? parse(card.preview) : null;
  const saved = v?.saved_to ?? card.preview.match(/"saved_to":"([^"]+)"/)?.[1];
  const text: string = v?.screen_text ?? "";
  return (
    <div className="dk-panel">
      <div className="dk-head"><span>📸 Screenshot</span>{saved && <span className="dk-muted">{saved}{v?.width ? ` · ${v.width}×${v.height}` : ""}</span>}</div>
      {card.status === "ok" ? (
        <>
          <div className="dk-flash" />
          <div className="dk-sub">Text on the screen (OCR)</div>
          <pre className="dk-pre">{text ? (text.length > 2500 ? text.slice(0, 2500) + "…" : text) : "No readable text found."}</pre>
        </>
      ) : card.status === "running" ? <div className="dk-banner">📸 Capturing in {card.args?.delay_seconds ?? 3} s — bring the right window to the front…</div>
        : <Waiting card={card} what="capture your screen" />}
    </div>
  );
}

export default function DesktopStage({ card }: { card: DesktopCard }) {
  switch (card.name) {
    case "system_info": return <SystemInfo card={card} />;
    case "list_processes": return <Processes card={card} />;
    case "list_windows": return <Windows card={card} />;
    case "close_app": return <CloseApp card={card} />;
    case "clipboard_read":
    case "clipboard_write": return <Clipboard card={card} />;
    case "take_screenshot": return <Screenshot card={card} />;
  }
  return null;
}
