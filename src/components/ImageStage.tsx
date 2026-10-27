// Live-view visuals for the picture tools (src-tauri/src/tools/images.rs):
//  - image_info: photo grid with real thumbnails, date, size and a 📍 badge for GPS
//  - resize_image: before → after for each new copy
//  - organize_photos: the new date folders with their photos
//  - remove_photo_location: photos with the 📍 crossed out
// Thumbnails come from the backend (image_thumbnail_tool) as small data: URLs.
// The result JSON in card.preview is cut at 4,000 chars, so parsing is tolerant.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./imagestage.css";

export interface ImageCard {
  name: string;
  args: any;
  status: "running" | "ok" | "error" | "waiting" | "declined";
  preview: string;
}

export const IMAGE_TOOLS = ["image_info", "resize_image", "organize_photos", "remove_photo_location"];

export function describeImage(name: string, a: any): string {
  const p = String(a?.relative_path ?? "pictures").replace(/\\/g, "/");
  switch (name) {
    case "image_info": return `Looking at ${p}`;
    case "resize_image": {
      const how = a?.percent ? `${a.percent}%` : a?.max_width || a?.max_height ? `max ${a?.max_width ?? "…"}×${a?.max_height ?? "…"}` : "";
      return `${how ? "Resizing" : "Converting"} ${p}${how ? ` (${how})` : ""}${a?.format ? ` → ${String(a.format).toUpperCase()}` : ""}`;
    }
    case "organize_photos": return `${a?.dry_run ? "Planning how to sort" : "Sorting"} ${p} by ${a?.by ?? "month"}`;
    case "remove_photo_location": return `Removing location data from ${p}`;
  }
  return name;
}

export function imageIcon(name: string): string {
  return ({ image_info: "🖼️", resize_image: "📐", organize_photos: "🗂️", remove_photo_location: "📍" } as Record<string, string>)[name] ?? "🖼️";
}

const thumbCache = new Map<string, string | null>();

function Thumb({ root, path, badge }: { root: string; path: string; badge?: React.ReactNode }) {
  const key = `${root}|${path}`;
  const [src, setSrc] = useState<string | null | undefined>(thumbCache.get(key));
  useEffect(() => {
    if (thumbCache.has(key)) { setSrc(thumbCache.get(key)); return; }
    let live = true;
    invoke<string>("image_thumbnail_tool", { workspaceRoot: root, relativePath: path, size: 160 })
      .then((s) => { thumbCache.set(key, s); if (live) setSrc(s); })
      .catch(() => { thumbCache.set(key, null); if (live) setSrc(null); });
    return () => { live = false; };
  }, [key, root, path]);
  return (
    <div className="im-thumb">
      {src ? <img src={src} alt="" /> : <span className="im-ph">{src === null ? "🖼️" : ""}</span>}
      {badge}
    </div>
  );
}

const name = (p: string) => p.split("/").pop() || p;
const kb = (n: number) => (n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);
function parse(preview: string): any { try { return JSON.parse(preview); } catch { return null; } }

function Banner({ card, what }: { card: ImageCard; what: string }) {
  if (card.status === "running") return <div className="im-banner">Working on {what}…</div>;
  if (card.status === "waiting") return <div className="im-banner im-wait">⏸ Waiting for your approval</div>;
  if (card.status === "declined") return <div className="im-banner">⊘ You declined — nothing changed</div>;
  if (card.status === "error") return <div className="im-banner im-error">{card.preview.replace(/^Error: /, "")}</div>;
  return null;
}

function Info({ card, root }: { card: ImageCard; root: string }) {
  const v = parse(card.preview);
  const imgs: any[] = v?.images ?? [...card.preview.matchAll(/"path":"([^"]+)"[^}]*?"has_location":(true|false)/g)].map((m) => ({ path: m[1], has_location: m[2] === "true" }));
  if (card.status !== "ok") return <div className="im-panel"><div className="im-head">🖼️ Pictures</div><Banner card={card} what="the pictures" /></div>;
  const loc = imgs.filter((i) => i.has_location).length;
  return (
    <div className="im-panel">
      <div className="im-head"><span>🖼️ {imgs.length} picture{imgs.length === 1 ? "" : "s"}</span>
        {loc > 0 ? <span className="im-warn-text">📍 {loc} show where they were taken</span> : <span className="im-ok">no location data</span>}</div>
      <div className="im-grid">
        {imgs.slice(0, 24).map((i, n) => (
          <div className="im-tile im-in" key={i.path} style={{ animationDelay: `${n * 50}ms` }}>
            <Thumb root={root} path={i.path} badge={i.has_location ? <span className="im-pin" title="contains GPS location">📍</span> : null} />
            <div className="im-name" title={i.path}>{name(i.path)}</div>
            <div className="im-meta">{i.width ? `${i.width}×${i.height}` : (i.format ?? "").toUpperCase()}{i.size_bytes ? ` · ${kb(i.size_bytes)}` : ""}</div>
            {(i.taken || i.camera) && <div className="im-meta">{[i.taken, i.camera].filter(Boolean).join(" · ")}</div>}
          </div>
        ))}
      </div>
      {imgs.length > 24 && <div className="im-more">+ {imgs.length - 24} more</div>}
    </div>
  );
}

function Resize({ card, root }: { card: ImageCard; root: string }) {
  const v = card.status === "ok" ? parse(card.preview) : null;
  const rows: any[] = v?.created ?? [];
  return (
    <div className="im-panel">
      <div className="im-head"><span>📐 {card.args?.format && !card.args?.max_width && !card.args?.max_height && !card.args?.percent ? "Converted copies" : "Resized copies"}</span>
        {card.status === "ok" && <span className="im-ok">originals untouched</span>}</div>
      {rows.length > 0 ? (
        <div className="im-rows">
          {rows.slice(0, 20).map((r, n) => (
            <div className="im-row im-in" key={r.to} style={{ animationDelay: `${n * 60}ms` }}>
              <Thumb root={root} path={r.to} />
              <div className="im-row-main">
                <div className="im-name">{name(r.from)} <span className="im-arrow">→</span> {r.to}</div>
                <div className="im-meta">{r.before} → <b>{r.after}</b> · {kb(r.size_bytes)}</div>
              </div>
            </div>
          ))}
        </div>
      ) : <Banner card={card} what="the pictures" />}
      {(v?.skipped ?? []).length > 0 && <div className="im-banner">Skipped: {v.skipped.join("; ")}</div>}
    </div>
  );
}

function Organize({ card, root }: { card: ImageCard; root: string }) {
  const v = card.status === "ok" ? parse(card.preview) : null;
  const moves: any[] = v?.moves ?? [];
  const groups = new Map<string, any[]>();
  for (const m of moves) {
    const folder = m.to.split("/").slice(0, -1).join("/");
    groups.set(folder, [...(groups.get(folder) ?? []), m]);
  }
  return (
    <div className="im-panel">
      <div className="im-head"><span>🗂️ {v?.dry_run ? "Plan (nothing moved yet)" : "Sorted by date"}</span>{v && <span className="im-muted">{v.note}</span>}</div>
      {moves.length === 0 ? <Banner card={card} what="the photos" /> : (
        <div className="im-folders">
          {[...groups.entries()].map(([folder, ms], n) => (
            <div className="im-folder im-in" key={folder} style={{ animationDelay: `${n * 90}ms` }}>
              <div className="im-folder-name">📁 {folder}<span className="im-muted"> · {ms.length}</span></div>
              <div className="im-strip">
                {ms.slice(0, 8).map((m) => (
                  <div key={m.to} title={`${m.from} → ${m.to} (${m.date_from})`}>
                    <Thumb root={root} path={v.dry_run ? m.from : m.to} badge={m.date_from !== "photo" ? <span className="im-pin im-filedate" title="no date taken in the photo; file date used">🕒</span> : null} />
                  </div>
                ))}
                {ms.length > 8 && <span className="im-muted">+{ms.length - 8}</span>}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Clean({ card, root }: { card: ImageCard; root: string }) {
  const v = card.status === "ok" ? parse(card.preview) : null;
  const rows: any[] = v?.cleaned ?? [];
  return (
    <div className="im-panel">
      <div className="im-head"><span>📍 {card.status === "ok" ? "Location removed" : "Removing location data"}</span>{v && <span className="im-ok">no quality loss</span>}</div>
      {rows.length > 0 ? (
        <div className="im-grid">
          {rows.slice(0, 24).map((r, n) => (
            <div className="im-tile im-in" key={r.path} style={{ animationDelay: `${n * 50}ms` }}>
              <Thumb root={root} path={r.path} badge={r.had_location ? <span className="im-pin im-pin-off">📍</span> : null} />
              <div className="im-name">{name(r.path)}</div>
              <div className="im-meta">{r.had_location ? "location removed" : "no location found"}{r.bytes_removed > 0 ? ` · −${kb(r.bytes_removed)}` : ""}</div>
            </div>
          ))}
        </div>
      ) : <Banner card={card} what="the pictures" />}
      {(v?.skipped ?? []).length > 0 && <div className="im-banner">Skipped: {v.skipped.join("; ")}</div>}
    </div>
  );
}

export default function ImageStage({ card, workspaceRoot }: { card: ImageCard; workspaceRoot: string }) {
  switch (card.name) {
    case "image_info": return <Info card={card} root={workspaceRoot} />;
    case "resize_image": return <Resize card={card} root={workspaceRoot} />;
    case "organize_photos": return <Organize card={card} root={workspaceRoot} />;
    case "remove_photo_location": return <Clean card={card} root={workspaceRoot} />;
  }
  return null;
}
