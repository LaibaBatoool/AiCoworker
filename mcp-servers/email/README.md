# Email MCP server for AI CoWorker

| Tool | What it does | Tier (set in mcp.json) |
|---|---|---|
| `list_inbox` | newest emails: sender, subject, date, unread/starred, attachment count | safe |
| `search_email` | find emails by sender, subject or body text | safe |
| `read_email` | full email (marks it read) | safe |
| `mark_email` | read / unread, star / unstar | mutating |
| `draft_email` | save an email or reply to Drafts **without sending** | mutating |
| `save_attachments` | download an email's attachments into the workspace (never overwrites) | mutating |
| `send_email` | send, or reply (threaded) with workspace files attached | **privileged** |
| `forward_email` | forward an email with its attachments, plus an optional note | **privileged** |
| `delete_email` | move an email to Trash (restorable) | **privileged** |

## Modes
- **mock** (default): a JSON mailbox, nothing ever leaves the computer. Seeded with demo emails on first use.
  File: `EMAIL_MOCK_FILE`, default `%APPDATA%\AiCoworker\mock-mailbox.json`. Delete that file to get fresh demo emails.
- **real**: IMAP (read, drafts, flags, trash) + SMTP (send). Run `npm install` in this folder once.
  Gmail: turn on 2-Step Verification, create an app password (Google Account → Security → App passwords),
  and put it in `mcp.json` (never in the workspace, never in chat). IMAP must be enabled in Gmail settings.

## mcp.json (`%APPDATA%\AiCoworker\mcp.json`)
```json
{
  "mcpServers": {
    "email": {
      "command": "node",
      "args": ["D:/path/to/AiCoworker/mcp-servers/email/server.mjs"],
      "env": { "EMAIL_MODE": "mock" },
      "tier": "privileged",
      "tool_tiers": {
        "list_inbox": "safe", "read_email": "safe", "search_email": "safe",
        "mark_email": "mutating", "draft_email": "mutating", "save_attachments": "mutating"
      }
    }
  }
}
```
Real mode: `"env": { "EMAIL_MODE": "real", "EMAIL_ADDRESS": "you@gmail.com", "EMAIL_APP_PASSWORD": "xxxx xxxx xxxx xxxx" }`
(optional: `EMAIL_IMAP_HOST`, `EMAIL_SMTP_HOST`, `EMAIL_SMTP_PORT`, `EMAIL_IMAP_PORT`). JSON allows no `//` comments.

## Safety
- Tiers come from AI CoWorker's config, never from this server. Anything not listed in `tool_tiers` is privileged.
- AI CoWorker's egress guard blocks secrets in the arguments *and* paths of secret-looking files (e.g. attaching `.env`).
- Outgoing: attachments only from inside the workspace (no links, no secret-looking files, 10 MB), ≤10 recipients,
  no line breaks in headers, ≤20 sends per hour.
- Incoming attachments: file names are sanitised (no `../`, no reserved Windows names), saved only inside the
  workspace, never overwrite, ≤20 files / 25 MB. Saving is "mutating", so the app snapshots first (undoable).
- Email content is untrusted: AI CoWorker wraps it as external data (spotlighting).
- Privacy: in real mode, email text the agent reads is sent to the cloud model. Use a local model for private mail.
- Known limit: the app password sits in plain text in `%APPDATA%\AiCoworker\mcp.json`.

## Test
`node mcp-servers/email/test.mjs` → `ALL 34 EMAIL TESTS PASSED` (mock mode, no account needed).