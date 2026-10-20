# Email MCP server for AI CoWorker

Tools: `list_inbox`, `read_email`, `search_email` (read-only) and `send_email` (send or reply, with workspace attachments).

## Modes
- **mock** (default): a JSON mailbox, nothing ever leaves the computer. Seeded with demo emails on first use.
  File: `EMAIL_MOCK_FILE`, default `%APPDATA%\AiCoworker\mock-mailbox.json`.
- **real**: IMAP (read) + SMTP (send). Run `npm install` in this folder once.
  Gmail: turn on 2-Step Verification, create an app password (Google Account → Security → App passwords),
  and put it in `mcp.json` (never in the workspace, never in chat).

## mcp.json (`%APPDATA%\AiCoworker\mcp.json`)
```json
{
  "mcpServers": {
    "email": {
      "command": "node",
      "args": ["D:/path/to/AiCoworker/mcp-servers/email/server.mjs"],
      "env": { "EMAIL_MODE": "mock" },
      "tier": "privileged",
      "tool_tiers": { "list_inbox": "safe", "read_email": "safe", "search_email": "safe" }
    }
  }
}
```
Real mode: `"env": { "EMAIL_MODE": "real", "EMAIL_ADDRESS": "you@gmail.com", "EMAIL_APP_PASSWORD": "xxxx xxxx xxxx xxxx" }`
(optional: `EMAIL_IMAP_HOST`, `EMAIL_SMTP_HOST`, `EMAIL_SMTP_PORT`, `EMAIL_IMAP_PORT`).

## Safety
- Tiers come from AI CoWorker's config, never from this server: `send_email` must stay **privileged** (you approve every send).
- AI CoWorker's egress guard blocks secrets in the arguments *and* paths of secret-looking files (e.g. attaching `.env`).
- This server also refuses attachments outside the workspace, links, secret-looking files, more than 10 recipients,
  line breaks in headers, and more than 20 sends per hour.
- Email content is untrusted: AI CoWorker wraps it as external data (spotlighting).
- Known limit: the app password sits in plain text in `%APPDATA%\AiCoworker\mcp.json`.

## Test
`node mcp-servers/email/test.mjs` → `ALL 19 EMAIL TESTS PASSED` (mock mode, no account needed).