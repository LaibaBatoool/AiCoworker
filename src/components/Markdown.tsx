// Small, safe Markdown renderer for the agent's answers.
// The model writes Markdown (**bold**, lists, tables, `code`); showing it raw looks broken.
// Safety: we never use dangerouslySetInnerHTML. The parser builds a tiny tree and React renders
// it as elements, so any HTML/script inside an answer is shown as plain text. Links are shown,
// not clickable (an answer may contain links from an attacker's email or web page).
// toHtml() renders the same tree for the exported HTML report (every text is escaped).
import type { ReactNode } from "react";
import "./markdown.css";

// ---------- tree ----------
type Inline =
  | { t: "text"; v: string }
  | { t: "code"; v: string }
  | { t: "b"; c: Inline[] }
  | { t: "i"; c: Inline[] }
  | { t: "link"; text: Inline[]; href: string };

interface Item { lines: string[]; children: Block[] }
type Block =
  | { t: "h"; level: number; text: string }
  | { t: "p"; lines: string[] }
  | { t: "code"; text: string }
  | { t: "list"; ordered: boolean; start: number; items: Item[] }
  | { t: "table"; head: string[]; rows: string[][]; align: ("left" | "center" | "right")[] }
  | { t: "quote"; lines: string[] }
  | { t: "hr" };

// ---------- block parser ----------
const LIST_RE = /^(\s*)([-*+•]|\d{1,3}[.)])\s+(.*)$/;
const indentOf = (l: string) => l.replace(/\t/g, "    ").match(/^\s*/)![0].length;
const isBlank = (l: string) => l.trim() === "";
const isFence = (l: string) => /^\s*(```|~~~)/.test(l);
const isHr = (l: string) => /^\s*([-*_])(\s*\1){2,}\s*$/.test(l);
const isHeading = (l: string) => /^\s{0,3}#{1,6}\s+\S/.test(l);
const isTableSep = (l: string | undefined) => !!l && /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(l);
const isTableStart = (lines: string[], i: number) => lines[i].includes("|") && isTableSep(lines[i + 1]);
const startsBlock = (lines: string[], i: number) =>
  isFence(lines[i]) || isHeading(lines[i]) || isHr(lines[i]) || LIST_RE.test(lines[i]) || /^\s*>/.test(lines[i]) || isTableStart(lines, i);

function cells(row: string): string[] {
  let r = row.trim();
  if (r.startsWith("|")) r = r.slice(1);
  if (r.endsWith("|") && !r.endsWith("\\|")) r = r.slice(0, -1);
  return r.split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, "|"));
}

function parseList(lines: string[], start: number, indent: number): [Block, number] {
  const first = lines[start].match(LIST_RE)!;
  const ordered = /\d/.test(first[2]);
  const list: Block & { t: "list" } = { t: "list", ordered, start: ordered ? parseInt(first[2], 10) : 1, items: [] };
  let i = start;
  while (i < lines.length) {
    const line = lines[i];
    if (isBlank(line)) {
      let j = i + 1;
      while (j < lines.length && isBlank(lines[j])) j++;
      const m = j < lines.length ? lines[j].match(LIST_RE) : null;
      if (m && indentOf(lines[j]) >= indent && /\d/.test(m[2]) === ordered) { i = j; continue; }
      if (j < lines.length && !m && indentOf(lines[j]) > indent && list.items.length) { i = j; continue; } // indented paragraph inside an item
      break;
    }
    const m = line.match(LIST_RE);
    const ind = indentOf(line);
    if (m && ind <= indent + 1 && ind >= indent) {
      if (/\d/.test(m[2]) !== ordered) break; // a different kind of list starts
      list.items.push({ lines: [m[3]], children: [] });
      i++;
    } else if (m && ind > indent + 1 && list.items.length) {
      const [sub, next] = parseList(lines, i, ind);
      list.items[list.items.length - 1].children.push(sub);
      i = next;
    } else if (m) {
      break; // less indented: belongs to an outer list
    } else if (list.items.length && (ind > indent || !startsBlock(lines, i))) {
      list.items[list.items.length - 1].lines.push(line.trim()); // continuation line of the item
      i++;
    } else {
      break;
    }
  }
  return [list, i];
}

export function parseMarkdown(src: string): Block[] {
  const lines = String(src ?? "").replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (isBlank(line)) { i++; continue; }
    if (isFence(line)) {
      const fence = line.trim().slice(0, 3);
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence)) body.push(lines[i++]);
      i++; // closing fence (or end)
      out.push({ t: "code", text: body.join("\n") });
      continue;
    }
    if (isHeading(line)) {
      const m = line.trim().match(/^(#{1,6})\s+(.*?)\s*#*\s*$/)!;
      out.push({ t: "h", level: m[1].length, text: m[2] });
      i++;
      continue;
    }
    if (isHr(line)) { out.push({ t: "hr" }); i++; continue; }
    if (isTableStart(lines, i)) {
      const head = cells(line);
      const align = cells(lines[i + 1]).map((c) => (/^:-+:$/.test(c) ? "center" : /-:$/.test(c) ? "right" : "left") as "left" | "center" | "right");
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && !isBlank(lines[i]) && lines[i].includes("|")) rows.push(cells(lines[i++]));
      out.push({ t: "table", head, rows, align });
      continue;
    }
    if (/^\s*>/.test(line)) {
      const q: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) q.push(lines[i++].replace(/^\s*>\s?/, ""));
      out.push({ t: "quote", lines: q });
      continue;
    }
    if (LIST_RE.test(line)) {
      const [list, next] = parseList(lines, i, indentOf(line));
      out.push(list);
      i = next;
      continue;
    }
    const p: string[] = [];
    while (i < lines.length && !isBlank(lines[i]) && (p.length === 0 || !startsBlock(lines, i))) p.push(lines[i++].trim());
    out.push({ t: "p", lines: p });
  }
  return out;
}

// ---------- inline parser ----------
const INLINE: { re: RegExp; make: (m: RegExpExecArray) => Inline }[] = [
  { re: /`([^`]+)`/, make: (m) => ({ t: "code", v: m[1] }) },
  { re: /\*\*(.+?)\*\*/, make: (m) => ({ t: "b", c: parseInline(m[1]) }) },
  { re: /__(.+?)__/, make: (m) => ({ t: "b", c: parseInline(m[1]) }) },
  { re: /\[([^\]]+)\]\(([^)\s]+)\)/, make: (m) => ({ t: "link", text: parseInline(m[1]), href: m[2] }) },
  { re: /(?<![\w*])\*(?!\s)([^*]+?)(?<!\s)\*(?![\w*])/, make: (m) => ({ t: "i", c: parseInline(m[1]) }) },
];

export function parseInline(s: string): Inline[] {
  const out: Inline[] = [];
  let rest = s;
  while (rest) {
    let best: { idx: number; m: RegExpExecArray; make: (m: RegExpExecArray) => Inline } | null = null;
    for (const { re, make } of INLINE) {
      const m = re.exec(rest);
      if (m && (!best || m.index < best.idx)) best = { idx: m.index, m, make };
    }
    if (!best) { out.push({ t: "text", v: rest }); break; }
    if (best.idx > 0) out.push({ t: "text", v: rest.slice(0, best.idx) });
    out.push(best.make(best.m));
    rest = rest.slice(best.idx + best.m[0].length);
  }
  return out;
}

// ---------- React output ----------
function renderInline(nodes: Inline[]): ReactNode[] {
  return nodes.map((n, k) => {
    switch (n.t) {
      case "text": return n.v;
      case "code": return <code key={k}>{n.v}</code>;
      case "b": return <strong key={k}>{renderInline(n.c)}</strong>;
      case "i": return <em key={k}>{renderInline(n.c)}</em>;
      case "link": return <span key={k} className="md-link" title={n.href}>{renderInline(n.text)}<span className="md-href"> ({n.href})</span></span>;
    }
  });
}
const lines = (ls: string[]) => ls.map((l, k) => <span key={k}>{k > 0 && <br />}{renderInline(parseInline(l))}</span>);

function renderBlocks(blocks: Block[]): ReactNode[] {
  return blocks.map((b, k) => {
    switch (b.t) {
      case "h": {
        const H = (`h${Math.min(b.level + 2, 6)}`) as "h3"; // answers are inside a card: start small
        return <H key={k} className="md-h">{renderInline(parseInline(b.text))}</H>;
      }
      case "p": return <p key={k}>{lines(b.lines)}</p>;
      case "code": return <pre key={k} className="md-code"><code>{b.text}</code></pre>;
      case "hr": return <hr key={k} />;
      case "quote": return <blockquote key={k}>{lines(b.lines)}</blockquote>;
      case "list": {
        const items = b.items.map((it, j) => <li key={j}>{lines(it.lines)}{renderBlocks(it.children)}</li>);
        return b.ordered ? <ol key={k} start={b.start}>{items}</ol> : <ul key={k}>{items}</ul>;
      }
      case "table":
        return (
          <div key={k} className="md-table-wrap">
            <table>
              <thead><tr>{b.head.map((c, j) => <th key={j} style={{ textAlign: b.align[j] ?? "left" }}>{renderInline(parseInline(c))}</th>)}</tr></thead>
              <tbody>{b.rows.map((r, j) => (
                <tr key={j}>{b.head.map((_, x) => <td key={x} style={{ textAlign: b.align[x] ?? "left" }}>{renderInline(parseInline(r[x] ?? ""))}</td>)}</tr>
              ))}</tbody>
            </table>
          </div>
        );
    }
  });
}

export default function Markdown({ text, className }: { text: string; className?: string }) {
  return <div className={`md ${className ?? ""}`}>{renderBlocks(parseMarkdown(text))}</div>;
}

// ---------- HTML output (exported report) ----------
const esc = (v: unknown) =>
  String(v ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c] as string));

function inlineHtml(nodes: Inline[]): string {
  return nodes.map((n) => {
    switch (n.t) {
      case "text": return esc(n.v);
      case "code": return `<code>${esc(n.v)}</code>`;
      case "b": return `<strong>${inlineHtml(n.c)}</strong>`;
      case "i": return `<em>${inlineHtml(n.c)}</em>`;
      case "link": return `${inlineHtml(n.text)} <span class="md-href">(${esc(n.href)})</span>`;
    }
  }).join("");
}
const linesHtml = (ls: string[]) => ls.map((l) => inlineHtml(parseInline(l))).join("<br>");

function blocksHtml(blocks: Block[]): string {
  return blocks.map((b) => {
    switch (b.t) {
      case "h": return `<h${Math.min(b.level + 2, 6)} class="md-h">${inlineHtml(parseInline(b.text))}</h${Math.min(b.level + 2, 6)}>`;
      case "p": return `<p>${linesHtml(b.lines)}</p>`;
      case "code": return `<pre class="md-code"><code>${esc(b.text)}</code></pre>`;
      case "hr": return "<hr>";
      case "quote": return `<blockquote>${linesHtml(b.lines)}</blockquote>`;
      case "list": {
        const items = b.items.map((it) => `<li>${linesHtml(it.lines)}${blocksHtml(it.children)}</li>`).join("");
        return b.ordered ? `<ol start="${b.start}">${items}</ol>` : `<ul>${items}</ul>`;
      }
      case "table":
        return `<div class="md-table-wrap"><table><thead><tr>${b.head.map((c, j) => `<th style="text-align:${b.align[j] ?? "left"}">${inlineHtml(parseInline(c))}</th>`).join("")}</tr></thead><tbody>${
          b.rows.map((r) => `<tr>${b.head.map((_, x) => `<td style="text-align:${b.align[x] ?? "left"}">${inlineHtml(parseInline(r[x] ?? ""))}</td>`).join("")}</tr>`).join("")
        }</tbody></table></div>`;
    }
  }).join("");
}

export function markdownToHtml(text: string): string {
  return `<div class="md">${blocksHtml(parseMarkdown(text))}</div>`;
}

/** CSS for markdownToHtml inside the (light) exported report. */
export const MARKDOWN_REPORT_CSS = `
  .md { white-space: normal; line-height: 1.6; }
  .md p { margin: 0 0 8px; } .md p:last-child { margin-bottom: 0; }
  .md ul, .md ol { margin: 4px 0 8px; padding-left: 22px; } .md ul { list-style: disc; } .md ol { list-style: decimal; }
  .md li { padding: 2px 0; } .md li > ul, .md li > ol { margin: 2px 0; }
  .md code { font: 12.5px ui-monospace, Consolas, monospace; background: #f3f0fb; padding: 1px 5px; border-radius: 4px; }
  .md .md-code { background: #f6f4fd; border: 1px solid var(--line); border-radius: 8px; padding: 10px 12px; margin: 6px 0 10px; }
  .md .md-code code { background: none; padding: 0; }
  .md .md-h { font-size: 14.5px; margin: 12px 0 6px; } .md blockquote { margin: 6px 0; padding-left: 10px; border-left: 3px solid var(--line); color: var(--muted); }
  .md .md-href { color: var(--muted); font-size: 12px; } .md hr { border: 0; border-top: 1px solid var(--line); margin: 10px 0; }
  .md .md-table-wrap { overflow-x: auto; margin: 6px 0 10px; } .md table { border-radius: 8px; }
`;