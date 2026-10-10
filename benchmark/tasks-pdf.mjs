// PDF page tasks (merge_pdfs / extract_pdf_pages). Merged into TASKS by bench.mjs.
// Includes a tiny PDF writer (one text label per page) and a reader that
// follows Root -> Pages -> Kids -> Contents to recover the page ORDER.
import fs from "node:fs";
import path from "node:path";

const read = (p) => (fs.existsSync(p) ? fs.readFileSync(p, "latin1") : null);
const pass = (reason = "ok") => ({ status: "PASS", reason });
const fail = (reason) => ({ status: "FAIL", reason });
const inconclusive = (reason) => ({ status: "INCONCLUSIVE", reason });

// ---- writer: one page per label, each page shows its label
function writePdf(file, labels) {
  const objs = []; // index i -> object number i+1
  const add = (body) => { objs.push(body); return objs.length; };
  const catalog = add(null), pages = add(null);
  const font = add("<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>");
  const kids = [];
  for (const label of labels) {
    const content = `BT /F1 24 Tf 100 700 Td (${label}) Tj ET`;
    const c = add(`<< /Length ${content.length} >>\nstream\n${content}\nendstream`);
    kids.push(add(`<< /Type /Page /Parent ${pages} 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 ${font} 0 R >> >> /Contents ${c} 0 R >>`));
  }
  objs[catalog - 1] = `<< /Type /Catalog /Pages ${pages} 0 R >>`;
  objs[pages - 1] = `<< /Type /Pages /Kids [${kids.map((k) => `${k} 0 R`).join(" ")}] /Count ${kids.length} >>`;
  let out = "%PDF-1.4\n";
  const offsets = [];
  objs.forEach((body, i) => { offsets.push(out.length); out += `${i + 1} 0 obj\n${body}\nendobj\n`; });
  const xref = out.length;
  out += `xref\n0 ${objs.length + 1}\n0000000000 65535 f \n` + offsets.map((o) => `${String(o).padStart(10, "0")} 00000 n \n`).join("");
  out += `trailer\n<< /Size ${objs.length + 1} /Root ${catalog} 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, out, "latin1");
}

// ---- reader: page labels in page order, or { error } if the file can't be followed
function pdfPageLabels(file) {
  const raw = read(file);
  if (raw === null) return { error: "missing" };
  const objs = {};
  for (const m of raw.matchAll(/(\d+)\s+(\d+)\s+obj\b([\s\S]*?)\bendobj/g)) objs[m[1]] = m[3];
  const root = (raw.match(/\/Root\s+(\d+)\s+\d+\s+R/g) || []).pop()?.match(/(\d+)/)[1];
  if (!root || !objs[root]) return { error: "no readable /Root (compressed cross-reference or object streams?)" };
  const ref = (txt, key) => txt?.match(new RegExp(`/${key}\\s+(\\d+)\\s+\\d+\\s+R`))?.[1];
  const labels = [];
  const walk = (id, depth = 0) => {
    const o = objs[id];
    if (!o || depth > 32) throw new Error(`object ${id} not readable`);
    if (/\/Type\s*\/Pages\b/.test(o)) {
      const kids = o.match(/\/Kids\s*\[([^\]]*)\]/)?.[1] || "";
      for (const k of kids.matchAll(/(\d+)\s+\d+\s+R/g)) walk(k[1], depth + 1);
      return;
    }
    const arr = o.match(/\/Contents\s*\[([^\]]*)\]/)?.[1];
    const ids = arr ? [...arr.matchAll(/(\d+)\s+\d+\s+R/g)].map((m) => m[1]) : [ref(o, "Contents")];
    let text = "";
    for (const cid of ids) {
      const c = objs[cid];
      if (!c) throw new Error(`content ${cid} missing`);
      if (/\/Filter/.test(c)) throw new Error("content stream is compressed");
      text += c;
    }
    labels.push((text.match(/\(([^)]*)\)\s*Tj/) || [])[1] ?? "?");
  };
  try { walk(ref(objs[root], "Pages")); } catch (e) { return { error: e.message }; }
  return { labels };
}

function falseCompletion(w, file) {
  let status = null;
  try { status = JSON.parse(read(path.join(w, "..", "agent_trace.json"))).status; } catch {}
  return status === "done"
    ? fail(`FALSE COMPLETION: agent reported done but ${file} is missing`)
    : inconclusive(`${file} missing — agent did not run or did not finish`);
}

const PART = (p, n) => Array.from({ length: n }, (_, i) => `PART${p}-PAGE${i + 1}`);

export const PDF_TASKS = {
  S10: {
    bucket: "synthetic",
    prompt: "Merge part1.pdf, part2.pdf and part3.pdf, in that order, into a new file called combined.pdf in the workspace root.",
    setup(w) {
      writePdf(path.join(w, "part3.pdf"), PART(3, 1)); // created first on purpose: order must come from the prompt
      writePdf(path.join(w, "part1.pdf"), PART(1, 2));
      writePdf(path.join(w, "part2.pdf"), PART(2, 3));
    },
    verify(w) {
      const f = path.join(w, "combined.pdf");
      if (!fs.existsSync(f)) return falseCompletion(w, "combined.pdf");
      const r = pdfPageLabels(f);
      if (r.error) return inconclusive(`combined.pdf exists but couldn't be checked automatically (${r.error}) — open it and check by hand`);
      const want = [...PART(1, 2), ...PART(2, 3), ...PART(3, 1)];
      return JSON.stringify(r.labels) === JSON.stringify(want)
        ? pass("6 pages in the right order")
        : fail(`wrong pages/order: ${r.labels.join(", ")}`);
    },
  },

  S11: {
    bucket: "synthetic",
    prompt: "report.pdf has 6 pages. Create a new file summary.pdf in the workspace root that contains only pages 2 to 4 of report.pdf. Do not change report.pdf.",
    setup(w) {
      writePdf(path.join(w, "report.pdf"), Array.from({ length: 6 }, (_, i) => `REPORT-PAGE${i + 1}`));
    },
    verify(w) {
      const src = pdfPageLabels(path.join(w, "report.pdf"));
      if (src.error || src.labels.length !== 6) return fail("report.pdf was changed or damaged");
      const f = path.join(w, "summary.pdf");
      if (!fs.existsSync(f)) return falseCompletion(w, "summary.pdf");
      const r = pdfPageLabels(f);
      if (r.error) return inconclusive(`summary.pdf exists but couldn't be checked automatically (${r.error}) — open it and check by hand`);
      const want = ["REPORT-PAGE2", "REPORT-PAGE3", "REPORT-PAGE4"];
      return JSON.stringify(r.labels) === JSON.stringify(want)
        ? pass("pages 2-4 extracted, report.pdf untouched")
        : fail(`wrong pages: ${r.labels.join(", ")}`);
    },
  },
};

export { writePdf, pdfPageLabels };