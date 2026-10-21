// Live-view visuals for the calendar MCP server (mcp-servers/calendar):
//  - agenda grouped by day for list_events / search_events / import / export
//  - an event card for add_event / update_event (before → after) / delete_event
//  - a day bar with free and busy time for find_free_time
// Parses the server's text output (format defined in mcp-servers/calendar/server.mjs, line()).
import "./calendarstage.css";

export interface CalCard {
  name: string;
  args: any;
  status: "running" | "ok" | "error" | "waiting" | "declined";
  preview: string;
}

const TOOLS = ["list_events", "search_events", "find_free_time", "add_event", "update_event", "delete_event", "export_ics", "import_ics"];

/** "mcp__calendar__add_event" -> "add_event" (null if it's not a calendar tool). */
export function calTool(name: string): string | null {
  if (!name.startsWith("mcp__")) return null;
  const tool = name.split("__").slice(2).join("__");
  return TOOLS.includes(tool) ? tool : null;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
function niceDate(ymd: string, withYear = false): string {
  const m = /^(\d{4})-(\d\d)-(\d\d)/.exec(ymd || "");
  if (!m) return ymd || "";
  const d = new Date(+m[1], +m[2] - 1, +m[3]);
  return `${DAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}${withYear || d.getFullYear() !== new Date().getFullYear() ? ` ${d.getFullYear()}` : ""}`;
}
const whenOf = (a: any) => {
  const s = String(a?.start ?? "");
  if (!s) return "";
  const time = /[ T](\d{1,2}:\d{2})/.exec(s)?.[1];
  const end = /[ T](\d{1,2}:\d{2})/.exec(String(a?.end ?? ""))?.[1];
  return time ? `${niceDate(s)}, ${time}${end ? `–${end}` : a?.duration_minutes ? ` (${a.duration_minutes} min)` : ""}` : `${niceDate(s)} (all day)`;
};

export function describeCal(tool: string, a: any): string {
  switch (tool) {
    case "list_events": return a?.from ? `Checking the calendar from ${niceDate(a.from)}` : "Checking the calendar";
    case "search_events": return `Searching the calendar for "${a?.query ?? ""}"`;
    case "find_free_time": return `Finding free time on ${niceDate(a?.date ?? "")}`;
    case "add_event": return `Adding "${a?.title ?? "event"}"${whenOf(a) ? ` — ${whenOf(a)}` : ""}`;
    case "update_event": return `Changing event ${a?.id ?? ""}${whenOf(a) ? ` → ${whenOf(a)}` : ""}`;
    case "delete_event": return `Deleting event ${a?.id ?? ""}`;
    case "export_ics": return `Exporting events to ${a?.path || "calendar-export.ics"}`;
    case "import_ics": return `Importing events from ${a?.path ?? "a .ics file"}`;
  }
  return tool;
}

export function calIcon(tool: string): string {
  return ({ list_events: "📅", search_events: "🔎", find_free_time: "🕒", add_event: "➕", update_event: "✏️", delete_event: "🗑️",
    export_ics: "📤", import_ics: "📥" } as Record<string, string>)[tool] ?? "📅";
}

// ---------- parsing ----------
interface Ev { id: string; date: string; start?: string; end?: string; title: string; location?: string; reminder?: string; with?: string; notes?: string }
const LINE = /^\[(e\d+)\] (\d{4}-\d\d-\d\d) (?:(\d\d:\d\d)–(\d\d:\d\d)|all day) \| (.*?)(?: \| Location: (.*?))?(?: \| Reminder: (\d+) min)?(?: \| With: (.*?))?(?: \| Notes: (.*))?$/;
function parseLine(l: string): Ev | null {
  const m = LINE.exec(l.trim());
  return m ? { id: m[1], date: m[2], start: m[3], end: m[4], title: m[5], location: m[6], reminder: m[7], with: m[8], notes: m[9] } : null;
}
const parseAll = (t: string) => t.split("\n").map((l) => parseLine(l.replace(/^(Before|After):\s*/, ""))).filter(Boolean) as Ev[];
const cleanError = (p: string) => p.replace(/^Error:\s*/, "").replace(/^MCP tool '[^']+' reported an error:\s*/, "");

function Banner({ card, verb }: { card: CalCard; verb: string }) {
  if (card.status === "waiting") return <div className="cal-banner cal-wait">⏸ Waiting for your approval — nothing has changed yet.</div>;
  if (card.status === "declined") return <div className="cal-banner cal-declined">⊘ You declined — the calendar was not changed.</div>;
  if (card.status === "error") return <div className="cal-banner cal-error">✕ {cleanError(card.preview)}</div>;
  if (card.status === "running") return <div className="cal-banner">{verb}…</div>;
  return null;
}

// ---------- pieces ----------
function Tile({ date }: { date: string }) {
  const m = /^(\d{4})-(\d\d)-(\d\d)/.exec(date || "");
  if (!m) return <div className="cal-tile"><span>—</span><b>?</b></div>;
  const d = new Date(+m[1], +m[2] - 1, +m[3]);
  return <div className="cal-tile"><span>{MONTHS[d.getMonth()]}</span><b>{d.getDate()}</b><i>{DAYS[d.getDay()]}</i></div>;
}

function EventRow({ e }: { e: Ev }) {
  return (
    <div className="cal-row">
      <div className="cal-time">{e.start ? <>{e.start}<span>{e.end}</span></> : <em>all day</em>}</div>
      <div className="cal-row-main">
        <div className="cal-title">{e.title}</div>
        <div className="cal-meta">
          {e.location && <span>📍 {e.location}</span>}
          {e.reminder && <span>🔔 {e.reminder} min before</span>}
          {e.with && <span>👥 {e.with}</span>}
        </div>
      </div>
      <span className="cal-id">{e.id}</span>
    </div>
  );
}

function Agenda({ events, title, sub }: { events: Ev[]; title: string; sub?: string }) {
  const days = new Map<string, Ev[]>();
  for (const e of events) days.set(e.date, [...(days.get(e.date) ?? []), e]);
  const today = (() => { const d = new Date(); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`; })();
  return (
    <div className="cal-panel">
      <div className="cal-head"><span>{title}</span>{sub && <span className="cal-muted">{sub}</span>}</div>
      {events.length === 0 && <div className="cal-empty">No events.</div>}
      {[...days.entries()].map(([d, evs]) => (
        <div key={d} className="cal-day">
          <div className={`cal-day-label ${d === today ? "cal-today" : ""}`}>{d === today ? "Today · " : ""}{niceDate(d, true)}</div>
          {evs.map((e) => <EventRow key={e.id + e.date} e={e} />)}
        </div>
      ))}
    </div>
  );
}

function EventCard({ card, tool }: { card: CalCard; tool: string }) {
  const a = card.args ?? {};
  const evs = parseAll(card.preview);
  const after = card.status === "ok" ? evs[evs.length - 1] : undefined;
  const before = tool === "update_event" && card.status === "ok" && evs.length > 1 ? evs[0] : undefined;
  // before it ran, show what the agent ASKED for (from the arguments)
  const startTime = /[ T](\d{1,2}:\d{2})/.exec(String(a.start ?? ""))?.[1];
  const endTime = /[ T](\d{1,2}:\d{2})/.exec(String(a.end ?? ""))?.[1];
  const e: Ev = after ?? {
    id: a.id ?? "new", date: String(a.start ?? "").slice(0, 10), start: startTime, end: endTime,
    title: a.title ?? (tool === "add_event" ? "(no title)" : `event ${a.id ?? ""}`), location: a.location,
    reminder: a.reminder_minutes != null ? String(a.reminder_minutes) : undefined, with: Array.isArray(a.attendees) ? a.attendees.join(", ") : undefined,
  };
  const heads = card.preview.split("\n").filter((l) => /^(Heads up|Note):/.test(l));
  const verb = tool === "add_event" ? "Adding" : tool === "update_event" ? "Updating" : "Deleting";
  return (
    <div className={`cal-panel cal-event ${tool === "delete_event" && card.status === "ok" ? "cal-deleted" : ""} ${card.status === "waiting" ? "cal-panel-wait" : ""}`}>
      <div className="cal-head">
        <span>{calIcon(tool)} {tool === "add_event" ? "New event" : tool === "update_event" ? "Change event" : "Delete event"}</span>
        {card.status === "ok" && <span className="cal-ok">{tool === "add_event" ? "✓ added" : tool === "update_event" ? "✓ updated" : "✓ deleted"}</span>}
      </div>
      <Banner card={card} verb={verb} />
      {!/^\d{4}-\d\d-\d\d$/.test(e.date) && card.status !== "error" && (
        <div className="cal-empty">{tool === "delete_event" ? `Event ${a.id ?? ""} will be removed from your calendar.` : `Changing event ${a.id ?? ""}…`}</div>
      )}
      {/^\d{4}-\d\d-\d\d$/.test(e.date) && card.status !== "error" && (
        <div className="cal-event-body">
          <Tile date={e.date} />
          <div>
            <div className="cal-event-title">{e.title}</div>
            <div className="cal-event-when">{e.date ? niceDate(e.date, true) : ""}{e.start ? ` · ${e.start}${e.end ? `–${e.end}` : a.duration_minutes ? ` · ${a.duration_minutes} min` : ""}` : e.date ? " · all day" : ""}</div>
            <div className="cal-meta">
              {e.location && <span>📍 {e.location}</span>}
              {e.reminder && <span>🔔 {e.reminder} min before</span>}
              {e.with && <span>👥 {e.with}</span>}
            </div>
          </div>
        </div>
      )}
      {before && <div className="cal-before">was: {niceDate(before.date, true)}{before.start ? ` · ${before.start}–${before.end}` : " · all day"} · {before.title}</div>}
      {heads.map((h) => <div key={h} className="cal-banner cal-wait">{h}</div>)}
    </div>
  );
}

function FreeTime({ card }: { card: CalCard }) {
  if (card.status !== "ok") return <div className="cal-panel"><Banner card={card} verb="Looking" /></div>;
  const range = /between (\d\d:\d\d) and (\d\d:\d\d)/.exec(card.preview);
  const toMin = (s: string) => +s.slice(0, 2) * 60 + +s.slice(3, 5);
  const [s, e] = range ? [toMin(range[1]), toMin(range[2])] : [540, 1080];
  const free = [...card.preview.matchAll(/^- (\d\d:\d\d)–(\d\d:\d\d)$/gm)].map((m) => [toMin(m[1]), toMin(m[2])]);
  const pct = (x: number) => `${((x - s) / Math.max(1, e - s)) * 100}%`;
  const hours: number[] = [];
  for (let h = Math.ceil(s / 60); h <= Math.floor(e / 60); h++) hours.push(h);
  return (
    <div className="cal-panel">
      <div className="cal-head"><span>🕒 {niceDate(card.args?.date ?? "", true)}</span><span className="cal-muted">slots of ≥ {card.args?.duration_minutes ?? 60} min</span></div>
      <div className="cal-bar">
        {free.map(([a, b], i) => <div key={i} className="cal-free" style={{ left: pct(a), width: `calc(${pct(b)} - ${pct(a)})` }} />)}
      </div>
      <div className="cal-hours">{hours.map((h) => <span key={h} style={{ left: pct(h * 60) }}>{String(h).padStart(2, "0")}</span>)}</div>
      <div className="cal-legend"><span><i className="cal-sw-free" /> free</span><span><i className="cal-sw-busy" /> busy</span></div>
      <div className="cal-chips">
        {free.length ? free.map(([a, b], i) => <span key={i} className="cal-chip">{`${String(Math.floor(a / 60)).padStart(2, "0")}:${String(a % 60).padStart(2, "0")}–${String(Math.floor(b / 60)).padStart(2, "0")}:${String(b % 60).padStart(2, "0")}`}</span>)
          : <span className="cal-muted">No free slot that long.</span>}
      </div>
    </div>
  );
}

export default function CalendarStage({ card }: { card: CalCard }) {
  const tool = calTool(card.name) ?? "";
  const a = card.args ?? {};
  if (tool === "find_free_time") return <FreeTime card={card} />;
  if (tool === "add_event" || tool === "update_event" || tool === "delete_event") return <EventCard card={card} tool={tool} />;
  if (card.status !== "ok") return <div className="cal-panel"><div className="cal-head"><span>{calIcon(tool)} {describeCal(tool, a)}</span></div><Banner card={card} verb="Working" /></div>;
  const first = card.preview.split("\n").find((l) => /event\(s\)/.test(l)) ?? "";
  const title =
    tool === "search_events" ? `🔎 "${a.query ?? ""}"` :
    tool === "export_ics" ? `📤 ${/to (.+?\.ics)/.exec(card.preview)?.[1] ?? "calendar-export.ics"}` :
    tool === "import_ics" ? `📥 from ${a.path ?? ""}` : "📅 Calendar";
  const sub = tool === "list_events" || tool === "search_events" ? first.replace(/[:.]$/, "").replace(/ match ".*"$/, " found") : `${parseAll(card.preview).length} event(s)`;
  return <Agenda events={parseAll(card.preview)} title={title} sub={sub} />;
}