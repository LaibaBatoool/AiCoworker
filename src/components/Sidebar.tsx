interface SidebarProps {
  activeView: "cowork" | "debug";
  onChangeView: (view: "cowork" | "debug") => void;
}

export default function Sidebar({ activeView, onChangeView }: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="sidebar-brand">
        <div className="sidebar-brand-mark">AC</div>
        <div className="sidebar-brand-label">COWORKER</div>
      </div>
      <nav className="sidebar-nav">
        <button
          className={`sidebar-item ${activeView === "cowork" ? "active" : ""}`}
          onClick={() => onChangeView("cowork")}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
            <path d="M12 2l2.4 7.2L22 12l-7.6 2.8L12 22l-2.4-7.2L2 12l7.6-2.8L12 2z" strokeLinejoin="round" />
          </svg>
          Cowork
        </button>
        <button
          className={`sidebar-item ${activeView === "debug" ? "active" : ""}`}
          onClick={() => onChangeView("debug")}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
            <path d="M14.7 6.3a4 4 0 00-5.4 5.4L3 18v3h3l6.3-6.3a4 4 0 005.4-5.4l-2.1 2.1-2-2 2.1-2.1z" strokeLinejoin="round" />
          </svg>
          Debug Tools
        </button>
      </nav>
    </aside>
  );
}