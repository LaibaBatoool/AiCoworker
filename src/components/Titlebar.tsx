import { getCurrentWindow } from "@tauri-apps/api/window";

const appWindow = getCurrentWindow();

export default function Titlebar() {
  return (
    <div className="titlebar">
      <div className="titlebar-drag-area" data-tauri-drag-region>
        <div className="titlebar-brand-mark">AC</div>
        <span className="titlebar-title">AI CoWorker</span>
      </div>
      <div className="titlebar-controls">
        <button className="titlebar-btn" onClick={() => appWindow.minimize()} title="Minimize">
          <svg viewBox="0 0 14 14">
            <line x1="3" y1="7" x2="11" y2="7" stroke="currentColor" strokeWidth="1.2" />
          </svg>
        </button>
        <button className="titlebar-btn" onClick={() => appWindow.toggleMaximize()} title="Maximize">
          <svg viewBox="0 0 14 14">
            <rect x="3" y="3" width="8" height="8" stroke="currentColor" strokeWidth="1.2" fill="none" />
          </svg>
        </button>
        <button className="titlebar-btn titlebar-btn-close" onClick={() => appWindow.close()} title="Close">
          <svg viewBox="0 0 14 14">
            <path d="M3 3L11 11M11 3L3 11" stroke="currentColor" strokeWidth="1.2" />
          </svg>
        </button>
      </div>
    </div>
  );
}