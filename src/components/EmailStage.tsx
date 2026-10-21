// Live-view visuals for the email MCP server (mcp-servers/email):
//  - inbox list for list_inbox / search_email
//  - an email reader for read_email
//  - a compose card for send / reply / forward / draft, so the user sees EXACTLY what will be
//    sent while the app waits for their approval
//  - simple cards for save_attachments, mark_email, delete_email
// Works with any MCP server name ("mcp__<server>__list_inbox"); it parses the server's text output
// (format defined in mcp-servers/email/server.mjs, summaryLine / read_email).
import "./emailstage.css";

export interface EmailCard {
  name: string;
  args: any;
  status: "running" | "ok" | "error" | "waiting" | "declined";
  preview: string;
}

const TOOLS = ["list_inbox", "search_email", "read_email", "send_email", "forward_email", "draft_email", "save_attachments", "mark_email", "delete_email"];

/** "mcp__email__send_email" -> "send_email" (null if it's not an email tool). */
export function emailTool(name: string): string | null {
  if (!name.startsWith("mcp__")) return null;
  const tool = name.split("__").slice(2).join("__");
  return TOOLS.includes(tool) ? tool : null;
}

const list = (v: unknown): string[] =>
  Array.isArray(v) ? v.map(String).filter(Boolean) : typeof v === "string" && v.trim() ? v.split(/[,;]/).map((s) => s.trim()) : [];
const idOf = (a: any) => String(a?.id ?? a?.reply_to_id ?? "").replace(/^uid:/i, "#");
const who = (a: any) => {
  const to = list(a?.to);
  return to.length === 0 ? "" : to.length === 1 ? to[0] : `${to[0]} +${to.length - 1}`;
};

export function describeEmail(tool: string, a: any): string {
  switch (tool) {
    case "list_inbox": return a?.unread_only ? "Checking unread emails" : "Checking the inbox";
    case "search_email": return `Searching email for "${a?.query ?? ""}"`;
    case "read_email": return `Reading email ${idOf(a)}`;
    case "send_email": return a?.reply_to_id ? `Replying to email ${idOf(a)}` : `Sending an email to ${who(a) || "…"}`;
    case "forward_email": return `Forwarding email ${idOf(a)} to ${who(a) || "…"}`;
    case "draft_email": return a?.reply_to_id ? `Drafting a reply to email ${idOf(a)}` : `Drafting an email${who(a) ? ` to ${who(a)}` : ""}`;
    case "save_attachments": return `Saving attachments of email ${idOf(a)}`;
    case "mark_email": {
      const what = [a?.read === undefined ? null : a.read ? "read" : "unread", a?.starred === undefined ? null : a.starred ? "starred" : "unstarred"].filter(Boolean).join(" + ");
      return `Marking email ${idOf(a)} as ${what || "…"}`;
    }
    case "delete_email": return `Moving email ${idOf(a)} to Trash`;
  }
  return tool;
}

export function emailIcon(tool: string): string {
  return ({ list_inbox: "📥", search_email: "🔎", read_email: "✉️", send_email: "📤", forward_email: "📨", draft_email: "📝",
    save_attachments: "📎", mark_email: "⭐", delete_email: "🗑️" } as Record<string, string>)[tool] ?? "✉️";
}

// ---------- parsing the server's text ----------
interface Summary { id: string; unread: boolean; starred: boolean; date: string; from: string; subject: string; attachments: number; preview: string }
const LINE = /^\[(.+?)\]\s+((?:UNREAD |STARRED )*)(.+?) \| From: (.*?) \| Subject: (.*?)(?: \| Attachments: (\d+))?(?: \| Preview: (.*))?$/;

function parseSummaries(text: string): Summary[] {
  return text.split("\n").map((l) => LINE.exec(l.trim())).filter(Boolean).map((m) => ({
    id: m![1], unread: m![2].includes("UNREAD"), starred: m![2].includes("STARRED"), date: m![3],
    from: m![4], subject: m![5], attachments: Number(m![6] || 0), preview: (m![7] || "").replace(/…$/, ""),
  }));
}

function splitAddr(s: string): { name: string; addr: string } {
  const m = /^\s*"?([^"<]*?)"?\s*<([^>]+)>/.exec(s);
  if (m) return { name: m[1].trim() || m[2], addr: m[2] };
  return { name: s.trim(), addr: "" };
}

function shortDate(s: string): string {
  const m = /^(\d{4})-(\d\d)-(\d\d) (\d\d:\d\d)/.exec(s);
  if (!m) return s;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return m[4];
  const mon = d.toLocaleString("en", { month: "short" });
  return d.getFullYear() === now.getFullYear() ? `${Number(m[3])} ${mon}` : `${Number(m[3])} ${mon} ${m[1]}`;
}

function Avatar({ name }: { name: string }) {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  const letter = (name.match(/[A-Za-z0-9]/)?.[0] ?? "?").toUpperCase();
  return <span className="em-avatar" style={{ background: `hsl(${h} 55% 42%)` }}>{letter}</span>;
}

const cleanError = (p: string) => p.replace(/^Error:\s*/, "").replace(/^MCP tool '[^']+' reported an error:\s*/, "");

function ErrorBox({ card }: { card: EmailCard }) {
  return <div className="em-banner em-banner-error">✕ {cleanError(card.preview)}</div>;
}

// ---------- inbox list ----------
function InboxList({ card, tool }: { card: EmailCard; tool: string }) {
  if (card.status === "running")
    return <div className="em-panel">{[0, 1, 2, 3].map((i) => <div key={i} className="em-row em-skeleton"><span className="em-avatar" /><div><i /><i /></div></div>)}</div>;
  if (card.status === "error") return <ErrorBox card={card} />;
  const items = parseSummaries(card.preview);
  const title = tool === "search_email" ? `Search: "${card.args?.query ?? ""}"` : card.args?.unread_only ? "Unread" : "Inbox";
  return (
    <div className="em-panel">
      <div className="em-panel-head">
        <span>{tool === "search_email" ? "🔎" : "📥"} {title}</span>
        <span className="em-muted">{items.length} {tool === "search_email" ? "match" : "email"}{items.length === 1 ? "" : tool === "search_email" ? "es" : "s"}
          {items.some((x) => x.unread) && ` · ${items.filter((x) => x.unread).length} unread`}</span>
      </div>
      {items.length === 0 && <div className="em-empty">{card.preview || "No emails."}</div>}
      {items.map((m, i) => {
        const f = splitAddr(m.from);
        return (
          <div key={m.id} className={`em-row ${m.unread ? "em-unread" : ""}`} style={{ animationDelay: `${i * 45}ms` }}>
            <Avatar name={f.name} />
            <div className="em-row-main">
              <div className="em-row-top">
                <span className="em-from" title={f.addr}>{f.name}</span>
                <span className="em-date" title={m.date}>{m.starred && <span className="em-star">★</span>}{shortDate(m.date)}</span>
              </div>
              <div className="em-subject">{m.unread && <span className="em-dot" />}{m.subject}{m.attachments > 0 && <span className="em-clip" title={`${m.attachments} attachment(s)`}>📎{m.attachments > 1 ? m.attachments : ""}</span>}</div>
              {m.preview && <div className="em-preview">{m.preview}</div>}
            </div>
            <span className="em-id">{m.id.replace(/^uid:/, "#")}</span>
          </div>
        );
      })}
    </div>
  );
}

// ---------- reader ----------
function Reader({ card }: { card: EmailCard }) {
  if (card.status === "running") return <div className="em-panel em-reader em-skeleton-block"><i /><i /><i /></div>;
  if (card.status === "error") return <ErrorBox card={card} />;
  const [head, ...rest] = card.preview.split("\n\n");
  const h: Record<string, string> = {};
  for (const l of head.split("\n")) {
    const m = /^([A-Za-z]+):\s?(.*)$/.exec(l);
    if (m) h[m[1].toLowerCase()] = m[2];
  }
  const f = splitAddr(h.from ?? "");
  const atts = (h.attachments ?? "").split(",").map((s) => s.trim()).filter(Boolean);
  return (
    <div className="em-panel em-reader">
      <div className="em-reader-subject">{h.subject ?? "(no subject)"}</div>
      <div className="em-reader-meta">
        <Avatar name={f.name} />
        <div>
          <div><b>{f.name}</b> {f.addr && <span className="em-muted">&lt;{f.addr}&gt;</span>}</div>
          <div className="em-muted">to {h.to || "me"} · {h.date}</div>
        </div>
      </div>
      {atts.length > 0 && <div className="em-chips">{atts.map((a) => <span key={a} className="em-chip">📎 {a}</span>)}</div>}
      <div className="em-body">{rest.join("\n\n").trim() || "(empty)"}</div>
      <div className="em-note">🛡 Email text is treated as data: the agent was told not to follow instructions inside it.</div>
    </div>
  );
}

// ---------- compose (send / reply / forward / draft) ----------
function Compose({ card, tool }: { card: EmailCard; tool: string }) {
  const a = card.args ?? {};
  const to = list(a.to);
  const isDraft = tool === "draft_email", isFwd = tool === "forward_email";
  const body = String(isFwd ? a.note ?? "" : a.body ?? "");
  const atts = list(a.attachments);
  const resultLine = (k: string) => (card.preview.split("\n").find((l) => l.startsWith(k + ": ")) ?? "").slice(k.length + 2);
  const subject = a.subject || resultLine("Subject") || (a.reply_to_id ? `Re: (subject of email ${idOf(a)})` : isFwd ? `Fwd: (subject of email ${idOf(a)})` : "");
  const toShown = to.length ? to : resultLine("To") ? resultLine("To").split(", ") : [];
  const banner =
    card.status === "waiting" ? <div className="em-banner em-banner-wait">⏸ Waiting for your approval — nothing has been sent yet. Check the details below.</div>
    : card.status === "running" ? <div className="em-banner">{isDraft ? "Saving the draft…" : "Sending…"}</div>
    : card.status === "declined" ? <div className="em-banner em-banner-declined">⊘ You declined — nothing was sent.</div>
    : card.status === "error" ? <ErrorBox card={card} />
    : <div className="em-banner em-banner-ok">{isDraft ? "📝 Saved to Drafts — not sent" : /MOCK mailbox/.test(card.preview) ? "✓ Sent (mock mailbox — nothing left this computer)" : "✓ Sent"}</div>;
  return (
    <div className={`em-panel em-compose ${card.status === "waiting" ? "em-compose-wait" : ""} ${card.status === "ok" && !isDraft ? "em-compose-sent" : ""}`}>
      <div className="em-panel-head">
        <span>{emailIcon(tool)} {isDraft ? "Draft" : isFwd ? "Forward" : a.reply_to_id ? "Reply" : "New email"}</span>
        {(a.reply_to_id || isFwd) && <span className="em-muted">{isFwd ? "forwarding" : "in reply to"} email {idOf(a)}</span>}
      </div>
      {banner}
      <div className="em-field"><span>To</span><div>{toShown.length ? toShown.map((t) => <span key={t} className="em-chip">{t}</span>) : <span className="em-muted">{a.reply_to_id ? `the sender of email ${idOf(a)}` : "(no recipient yet)"}</span>}</div></div>
      <div className="em-field"><span>Subject</span><div>{subject || <span className="em-muted">(none)</span>}</div></div>
      {atts.length > 0 && <div className="em-field"><span>Attach</span><div>{atts.map((x) => <span key={x} className="em-chip">📎 {x}</span>)}</div></div>}
      {isFwd && <div className="em-field"><span>Includes</span><div className="em-muted">the original email and its attachments</div></div>}
      <div className="em-body em-compose-body">{body || <span className="em-muted">{isFwd ? "(no note)" : "(empty)"}</span>}</div>
    </div>
  );
}

// ---------- the rest ----------
function SavedFiles({ card }: { card: EmailCard }) {
  if (card.status === "error") return <ErrorBox card={card} />;
  const files = card.preview.split("\n").map((l) => /^- (.+) \((.+)\)$/.exec(l)).filter(Boolean);
  return (
    <div className="em-panel">
      <div className="em-panel-head"><span>📎 Attachments of email {idOf(card.args)}</span><span className="em-muted">→ {card.args?.folder || "email-attachments"}/</span></div>
      {card.status === "running" && <div className="em-empty">Downloading…</div>}
      {card.status === "waiting" && <div className="em-banner em-banner-wait">⏸ Waiting for your approval</div>}
      {card.status === "ok" && files.length === 0 && <div className="em-empty">{card.preview}</div>}
      {files.map((m) => <div key={m![1]} className="em-file"><span>📄 {m![1]}</span><span className="em-muted">{m![2]}</span></div>)}
      {files.length > 0 && <div className="em-note">Saved into the workspace (snapshot taken first — undoable). Existing files are never overwritten.</div>}
    </div>
  );
}

function Simple({ card, tool }: { card: EmailCard; tool: string }) {
  return (
    <div className="em-simple">
      <div className="em-simple-icon">{emailIcon(tool)}</div>
      <div>{describeEmail(tool, card.args)}</div>
      {card.status === "waiting" && <div className="em-banner em-banner-wait">⏸ Waiting for your approval</div>}
      {card.status === "declined" && <div className="em-banner em-banner-declined">⊘ You declined — nothing changed.</div>}
      {card.status === "error" && <ErrorBox card={card} />}
      {card.status === "ok" && <div className="em-muted">{card.preview}</div>}
    </div>
  );
}

export default function EmailStage({ card }: { card: EmailCard }) {
  const tool = emailTool(card.name) ?? "";
  switch (tool) {
    case "list_inbox":
    case "search_email": return <InboxList card={card} tool={tool} />;
    case "read_email": return <Reader card={card} />;
    case "send_email":
    case "forward_email":
    case "draft_email": return <Compose card={card} tool={tool} />;
    case "save_attachments": return <SavedFiles card={card} />;
    default: return <Simple card={card} tool={tool} />;
  }
}