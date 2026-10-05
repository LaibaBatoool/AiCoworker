type OrbState = "idle" | "thinking" | "waiting" | "done" | "error";

interface StatusOrbProps {
  state: OrbState;
  label: string;
}

export default function StatusOrb({ state, label }: StatusOrbProps) {
  return (
    <div className="status-row">
      <div className={`status-orb ${state}`} />
      <span>{label}</span>
    </div>
  );
}

export type { OrbState };