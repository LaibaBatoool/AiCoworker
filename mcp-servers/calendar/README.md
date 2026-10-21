# Calendar MCP server for AI CoWorker

Local-first calendar: events live in a file on your computer and nothing is sent anywhere.
To put events into Google Calendar, Outlook or Apple Calendar, the agent exports a standard `.ics`
file (reminders included) that you import with one click. No dependencies (`npm install` not needed).

| Tool | What it does | Tier (set in mcp.json) |
|---|---|---|
| `list_events` | events in a date range (default: next 7 days); always starts with today's date | safe |
| `search_events` | find events by title, place, notes or attendee | safe |
| `find_free_time` | free slots on a day (default 09:00–18:00, ≥ 60 min) | safe |
| `add_event` | add an event (warns about overlaps; default length 1 hour) | mutating |
| `update_event` | change only the given fields (moving keeps the length) | mutating |
| `import_ics` | add events from a `.ics` file in the workspace (e.g. an invitation saved from an email) | mutating |
| `export_ics` | write events to a `.ics` file in the workspace (never overwrites) | mutating |
| `delete_event` | delete an event | **privileged** |

Times are local, written `YYYY-MM-DD HH:MM` (24-hour); a date alone means an all-day event.
Attendees are only stored — this server never sends invitations.

## mcp.json (`%APPDATA%\AiCoworker\mcp.json`, inside `"mcpServers"`)
```json
"calendar": {
  "command": "node",
  "args": ["D:/path/to/AiCoworker/mcp-servers/calendar/server.mjs"],
  "tier": "privileged",
  "tool_tiers": {
    "list_events": "safe", "search_events": "safe", "find_free_time": "safe",
    "add_event": "mutating", "update_event": "mutating", "import_ics": "mutating", "export_ics": "mutating"
  }
}
```
Events are stored in `%APPDATA%\AiCoworker\calendar.json` (override with `"env": { "CALENDAR_FILE": "..." }`).

## Safety
- Tiers come from AI CoWorker's config, never from this server. Anything not listed is privileged (delete).
- `.ics` files only inside the workspace (no absolute paths, `..`, links, `.git`); exports never overwrite;
  imports are capped at 1 MB / 200 events and skip duplicates. Imported text is untrusted data.
- Writing an `.ics` is "mutating", so the app snapshots the workspace first (undoable).

## Limits
- Reminders are stored and exported in the `.ics` (your calendar app shows them); the server itself doesn't pop up notifications.
- Recurring events (RRULE) are not supported yet; time zones in imported files (TZID) are read as local time.
- Real Google Calendar sync needs Google sign-in (OAuth) — planned.

## Test
`node mcp-servers/calendar/test.mjs` → `ALL 35 CALENDAR TESTS PASSED`.