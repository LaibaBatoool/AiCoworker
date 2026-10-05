const ACTION_DESCRIPTIONS: Record<string, string> = {
  delete_file: "wants to permanently delete a file or folder",
  execute_command: "wants to run a terminal command",
  restore_snapshot: "wants to roll the entire workspace back to an earlier point",
};

interface PermissionDialogProps {
  toolName: string;
  args: unknown;
  onApprove: () => void;
  onDecline: () => void;
}

export default function PermissionDialog({ toolName, args, onApprove, onDecline }: PermissionDialogProps) {
  const description = ACTION_DESCRIPTIONS[toolName] ?? `wants to call ${toolName}`;
  const entries = args && typeof args === "object" ? Object.entries(args as Record<string, unknown>) : [];

  return (
    <div className="permission-overlay">
      <div className="permission-card">
        <div className="permission-kicker">Privileged action — backend-enforced</div>
        <h3 className="permission-title">The agent {description}.</h3>
        {entries.length > 0 && (
          <div className="permission-args">
            {entries.map(([key, value]) => (
              <div className="permission-row" key={key}>
                <span className="permission-row-key">{key}:</span>
                <span className="permission-row-value">{String(value)}</span>
              </div>
            ))}
          </div>
        )}
        <div className="permission-actions">
          <button className="btn-decline" onClick={onDecline}>
            Decline
          </button>
          <button className="btn-approve" onClick={onApprove}>
            Approve
          </button>
        </div>
      </div>
    </div>
  );
}