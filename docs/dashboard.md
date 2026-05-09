# Sympheo dashboard

The HTTP server extension (`server.port` in `WORKFLOW.md` or `--port` CLI
flag) exposes both an HTML dashboard at `/` and a JSON API under
`/api/v1/`. SPEC §13.7 describes this as an OPTIONAL extension; the routes
below cover both the spec baseline and sympheo-specific operational
controls (also documented in `docs/extensions.md`).

## Spec baseline (§13.7.2)

| Method | Path | Purpose |
|---|---|---|
| GET | `/` | Server-rendered HTML dashboard |
| GET | `/api/v1/state` | Snapshot: counts, running, retrying, totals, rate limits |
| GET | `/api/v1/<issue_identifier>` | Single-issue runtime detail |
| POST | `/api/v1/refresh` | Trigger an immediate poll + reconcile cycle |

`<issue_identifier>` MUST be URL-encoded. For GitHub identifiers
containing `#`, this means `repo%2342` not `repo#42`.

## Operational control endpoints (extension)

These extend the §13.7 surface with the operator controls demanded by
CONTEXT.md: "il faut pouvoir kill l'avancement sur un ticket de sorte à
nettoyer le context et l'ensemble du travail sur la colone en cours".

### POST /api/v1/<issue_identifier>/cancel

Sets the running entry's `cancelled` atomic to `true`. The
`LocalBackend::run_turn` watchdog polls this flag every 1s and, when set,
issues `SIGKILL` to the entire CLI subprocess group via `killpg`. The
orchestrator then sees the worker exit and converts it to a retry per
§7.3 / §8.4 (failure-driven exponential backoff).

```bash
curl -X POST http://127.0.0.1:9090/api/v1/repo%2342/cancel
```

Response (200):
```json
{
  "cancelled": true,
  "issue_identifier": "repo#42",
  "issue_id": "I_kwDO...",
  "requested_at": "2026-05-09T12:34:56Z"
}
```

Returns 404 if no running entry matches the identifier.

### DELETE /api/v1/retry/<issue_identifier>

Removes an issue from the retry queue and releases its claim. Useful when
the operator has decided that retries are pointless (e.g. tracker state
needs manual fixing) and wants to free the slot for other work.

```bash
curl -X DELETE http://127.0.0.1:9090/api/v1/retry/repo%2342
```

Returns 404 if no retry entry matches the identifier.

## HTML dashboard

The dashboard is server-rendered HTML using Pico CSS. It auto-refreshes
every 5 seconds via JS. HTMX is loaded so that the per-row "Kill" buttons
on the Active Sessions table can POST to `/api/v1/<id>/cancel` without a
full page reload (`hx-post` + `hx-confirm`).

### What the operator sees

- **Last tick** — time since the orchestrator last completed a poll cycle.
- **Counts** — running, retrying, total tokens, aggregate runtime seconds.
- **Ticket Summary** — by-state counts (Todo / Spec / In Progress / etc.).
- **Recent Movements** — issues whose state changed recently.
- **Blocked or Delayed** — issues with non-terminal blockers, or in retry.
- **Active Sessions** — per-worker row with: identifier, state, session
  id, turn count, started time, last event name, **full last_message**
  (long messages collapse into a `<details>` element rather than being
  silently truncated, addressing CONTEXT.md "on vois trop peu de chose"),
  tokens in/out, and a Kill button.
- **Retry Queue** — pending retries with attempt count, error, and due
  time.

### What changed in P6

| Before | After |
|---|---|
| `last_message` truncated silently to 30 chars | Full message rendered; > 80 chars collapses into `<details>` summary |
| No way to interrupt a stuck or wasteful run | Kill button per active session (HTMX-style) |
| No way to drain a retry entry | DELETE `/api/v1/retry/<id>` |
| No HTMX | HTMX 2 loaded; existing 5s reload preserved as a fallback |

Future P6 follow-ups (not in this phase): per-worker SSE event stream,
fragment endpoints for partial dashboard refresh, detail view of an
individual worker.
