// Data tasks (read_spreadsheet). Merged into TASKS by bench.mjs.
import fs from "node:fs";
import path from "node:path";
import { writeZip } from "./tasks-archive.mjs";

const read = (p) => (fs.existsSync(p) ? fs.readFileSync(p, "utf8") : null);
const pass = (reason = "ok") => ({ status: "PASS", reason });
const fail = (reason) => ({ status: "FAIL", reason });
const inconclusive = (reason) => ({ status: "INCONCLUSIVE", reason });

const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const colName = (i) => String.fromCharCode(65 + i); // A..Z is enough here

// Minimal but valid .xlsx: sheets = [[name, rows]], text cells as inline strings, numbers as numbers.
function writeXlsx(file, sheets) {
  const NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
  const R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
  const sheetXml = (rows) =>
    `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="${NS}"><sheetData>` +
    rows.map((row, r) =>
      `<row r="${r + 1}">` +
      row.map((v, c) => {
        const ref = `${colName(c)}${r + 1}`;
        return typeof v === "number"
          ? `<c r="${ref}"><v>${v}</v></c>`
          : `<c r="${ref}" t="inlineStr"><is><t>${esc(v)}</t></is></c>`;
      }).join("") + `</row>`
    ).join("") + `</sheetData></worksheet>`;

  const entries = [
    ["[Content_Types].xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">` +
      `<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>` +
      `<Default Extension="xml" ContentType="application/xml"/>` +
      `<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>` +
      sheets.map((_, i) => `<Override PartName="/xl/worksheets/sheet${i + 1}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>`).join("") +
      `</Types>`],
    ["_rels/.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">` +
      `<Relationship Id="rId1" Type="${R}/officeDocument" Target="xl/workbook.xml"/></Relationships>`],
    ["xl/workbook.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="${NS}" xmlns:r="${R}"><sheets>` +
      sheets.map(([name], i) => `<sheet name="${esc(name)}" sheetId="${i + 1}" r:id="rId${i + 1}"/>`).join("") +
      `</sheets></workbook>`],
    ["xl/_rels/workbook.xml.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">` +
      sheets.map((_, i) => `<Relationship Id="rId${i + 1}" Type="${R}/worksheet" Target="worksheets/sheet${i + 1}.xml"/>`).join("") +
      `</Relationships>`],
    ...sheets.map(([, rows], i) => [`xl/worksheets/sheet${i + 1}.xml`, sheetXml(rows)]),
  ];
  writeZip(file, entries);
}

export const DATA_TASKS = {
  S9: {
    bucket: "synthetic",
    prompt:
      "In sales.xlsx, add up the amount column on the sheet named Q2 and write just the total to total.txt in the workspace root.",
    note: "Sheet selection: Q1 (first sheet) is a decoy with different amounts. Correct total is 1000 (Q1+Q2 would be 1030).",
    setup(w) {
      writeXlsx(path.join(w, "sales.xlsx"), [
        ["Q1", [["item", "amount"], ["pens", 10], ["paper", 20]]],
        ["Q2", [["item", "amount"], ["widget", 150], ["gadget", 275], ["gizmo", 80], ["doohickey", 495]]],
      ]);
    },
    verify(w) {
      const c = read(path.join(w, "total.txt"));
      if (c === null) {
        let status = null;
        try { status = JSON.parse(read(path.join(w, "..", "agent_trace.json"))).status; } catch {}
        return status === "done"
          ? fail("FALSE COMPLETION: agent reported done but total.txt is missing")
          : inconclusive("total.txt missing — agent did not run or did not finish");
      }
      const nums = (c.match(/-?\d[\d,]*(\.\d+)?/g) || []).map((n) => Number(n.replace(/,/g, "")));
      if (nums.includes(1000)) return pass("correct Q2 total 1000");
      if (nums.includes(30)) return fail("used the Q1 sheet (30) instead of Q2");
      if (nums.includes(1030)) return fail("added both sheets (1030) instead of only Q2");
      return fail(`wrong total: "${c.trim()}" (expected 1000)`);
    },
  },
};

export { writeXlsx };