// Advanced setting: "Ask for Windows Hello before risky actions" (src-tauri/src/auth.rs).
// Turning it on OR off shows the Windows Hello prompt first (fingerprint, face or PIN),
// so someone else at the PC can't switch it off. The check itself happens in the backend.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface HelloStatus {
  required: boolean;
  availability: string;
}

const AVAIL: Record<string, string> = {
  Available: "Windows Hello is ready on this PC",
  NotConfiguredForUser: "Windows Hello isn't set up for your account (Settings → Accounts → Sign-in options)",
  DeviceNotPresent: "No fingerprint reader, camera or PIN found",
  DisabledByPolicy: "Windows Hello is turned off by a system policy",
  DeviceBusy: "Windows Hello is busy, try again",
};

export default function HelloToggle() {
  const [status, setStatus] = useState<HelloStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  useEffect(() => {
    invoke<HelloStatus>("hello_status_tool").then(setStatus).catch((e) => setMsg(String(e)));
  }, []);

  async function toggle() {
    if (!status) return;
    setBusy(true);
    setMsg(status.required ? "Confirm with Windows Hello to turn it off…" : "Confirm with Windows Hello to turn it on…");
    try {
      const s = await invoke<HelloStatus>("set_hello_required_tool", { required: !status.required });
      setStatus(s);
      setMsg(s.required ? "On: risky actions now need your fingerprint, face or PIN after Approve." : "Off.");
    } catch (e) {
      setMsg(`Not changed: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  const ready = status?.availability === "Available";
  return (
    <div className="hello-row">
      <div className="hello-text">
        <div className="hello-title">🔐 Windows Hello for risky actions {status?.required ? <span className="hello-on">ON</span> : <span className="hello-off">off</span>}</div>
        <div className="hello-sub">
          {status ? AVAIL[status.availability] ?? `Windows Hello: ${status.availability}` : "Checking Windows Hello…"}
        </div>
        {msg && <div className="hello-msg">{msg}</div>}
      </div>
      <button className="hello-btn" disabled={busy || !status || (!ready && !status.required)} onClick={toggle}>
        {busy ? "Waiting…" : status?.required ? "Turn off" : "Turn on"}
      </button>
    </div>
  );
}
