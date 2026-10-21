# Notes & to-do MCP server for AI CoWorker

Local-first notes and to-do list: everything lives in one file on your computer, nothing is sent anywhere.
No dependencies (`npm install` not needed).

| Tool | What it does | Tier (set in mcp.json) |
|---|---|---|
| `list_tasks` | open tasks sorted by due date (overdue flagged), or done / all; filter by tag or due date | safe |
| `list_notes` | notes, newest first, with a preview; search text or tag | safe |
| `read_note` | one note in full | safe |
| `add_task` | add one task (title, due date, priority, notes, tags); warns about duplicates | mutating |
| `update_task` | tick it done / reopen, or change any field | mutating |
| `add_note` | save a note (Markdown is fine) | mutating |
| `update_note` | rename, retag, replace the text, or append to it | mutating |
| `export_markdown` | write the to-do list (as checkboxes) and/or notes to a `.md` file in the workspace (never overwrites) | mutating |
| `delete_item` | delete a note (`n…`) or a task (`t…`) | **privileged** |

Dates are local: `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`. Every task list starts with today's date.

## mcp.json (`%APPDATA%\AiCoworker\mcp.json`, inside `"mcpServers"`)
```json
"notes": {
  "command": "node",
  "args": ["D:/path/to/AiCoworker/mcp-servers/notes/server.mjs"],
  "tier": "privileged",
  "tool_tiers": {
    "list_tasks": "safe", "list_notes": "safe", "read_note": "safe",
    "add_task": "mutating", "update_task": "mutating", "add_note": "mutating",
    "update_note": "mutating", "export_markdown": "mutating"
  }
}
```
Data file: `%APPDATA%\AiCoworker\notes.json` (override with `"env": { "NOTES_FILE": "..." }`).

## Safety
- Tiers come from AI CoWorker's config, never from this server; anything not listed (delete) is privileged.
- Export only inside the workspace (no absolute paths, `..`, `.git`), never overwrites; it is "mutating", so the app snapshots first.
- Ticking a task done is an update, not a delete, so finishing work never needs an approval and nothing is lost.

## Test
`node mcp-servers/notes/test.mjs` → `ALL 32 NOTES TESTS PASSED`.