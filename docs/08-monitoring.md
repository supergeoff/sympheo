# Monitoring

Sympheo exposes a built-in HTTP dashboard and a REST API so you can observe what the orchestrator is doing in real time.

## Dashboard

The HTTP dashboard and JSON API are an OPTIONAL extension (SPEC §13.7). When `server.port` is configured (or `--port` is passed on the CLI), Sympheo starts a web server bound to loopback (`127.0.0.1`) by default.

Open your browser to:

```
http://localhost:<port>/
```

The dashboard auto-refreshes every 5 seconds via JavaScript. HTMX is loaded so per-row controls (kill switch, etc.) update without a full reload. The dashboard shows:

### Summary Cards

| Card | Meaning |
|------|---------|
| **Running** | Number of active agent sessions. |
| **Retrying** | Number of issues waiting in the retry queue. |
| **Tokens** | Total input + output tokens consumed across all sessions since startup. |
| **Runtime** | Total wall-clock seconds agents have been running since startup. |

### Ticket Summary

A visual breakdown of ticket activity on the board:

- **Counts per state** — small cards showing how many running issues are in each board column.
- **Recent Movements** — the last 10 issues sorted by when they most recently changed state, with a human-readable timestamp (e.g. `45s ago`).
- **Blocked or Delayed** — issues that are currently blocked by active dependencies, plus any issues sitting in the retry queue.

### Active Sessions Table

Each row represents one running issue:

| Column | Description |
|--------|-------------|
| Issue | Issue identifier (e.g., `#42`). |
| State | Current board column. |
| Session | Agent session ID. |
| Turns | How many turns have been executed for this issue. |
| Started | Timestamp when the current turn began. |
| Last Event | The most recent event type from the agent stream. |
| Last Message | Full text of the most recent message. Long messages collapse into a `<details>` element instead of being silently truncated. |
| Tokens (in/out) | Token counters for the current session. |
| Kill | HTMX-driven button that cancels the active turn (see [`POST /api/v1/<id>/cancel`](#post-apiv1issue_identifiercancel)). |

### Retry Queue Table

Shows issues that failed and are waiting to be retried:

| Column | Description |
|--------|-------------|
| Issue | Issue identifier. |
| Attempt | Retry attempt number. |
| Error | Truncated error message from the last failure. |
| Due In | Countdown until the next retry attempt. |

## REST API

All endpoints return JSON except `/`, which returns HTML.

### `GET /api/v1/state`

Returns the full orchestrator state.

```json
{
  "generated_at": "2024-01-15T09:30:00Z",
  "counts": {
    "running": 3,
    "retrying": 1
  },
  "running": [
    {
      "issue_id": "123456",
      "issue_identifier": "#42",
      "state": "in progress",
      "session_id": "sess-abc",
      "turn_count": 2,
      "started_at": "2024-01-15T09:15:00Z",
      "last_event": "step_finish",
      "last_message": "Refactored the parser module",
      "tokens": {
        "input_tokens": 15000,
        "output_tokens": 4200,
        "total_tokens": 19200
      }
    }
  ],
  "retrying": [
    {
      "issue_id": "123457",
      "issue_identifier": "#43",
      "attempt": 2,
      "due_at": "2024-01-15T09:31:00Z",
      "error": "Agent exited with code 1"
    }
  ],
  "summary": {
    "by_state": {
      "in progress": 2,
      "todo": 1
    },
    "recent_changes": [
      {
        "identifier": "#42",
        "state": "in progress",
        "last_state_change_at": "2024-01-15T09:25:00Z"
      }
    ],
    "blocked": [
      {
        "identifier": "#44",
        "state": "todo",
        "blocked_by": ["#45"]
      }
    ],
    "delayed": [
      {
        "identifier": "#43",
        "attempt": 2,
        "error": "Agent exited with code 1"
      }
    ]
  },
  "agent_totals": {
    "input_tokens": 45000,
    "output_tokens": 12000,
    "total_tokens": 57000,
    "seconds_running": 1800
  },
  "rate_limits": {}
}
```

### `POST /api/v1/refresh`

Triggers an immediate poll and reconciliation cycle, bypassing the normal timer.

```bash
curl -X POST http://localhost:9090/api/v1/refresh
```

Response (`202 Accepted` — the work is queued and runs asynchronously):

```json
{
  "queued": true,
  "coalesced": false,
  "requested_at": "2024-01-15T09:30:00Z",
  "operations": ["poll", "reconcile"]
}
```

### `GET /api/v1/{issue_identifier}`

Returns detailed state for a single running issue.

```bash
curl http://localhost:9090/api/v1/%2342
```

Response:

```json
{
  "issue_identifier": "#42",
  "issue_id": "123456",
  "status": "running",
  "started_at": "2024-01-15T09:15:00Z",
  "attempts": {
    "restart_count": 0,
    "current_retry_attempt": 0
  },
  "running": {
    "session_id": "sess-abc",
    "turn_count": 2,
    "state": "in progress",
    "started_at": "2024-01-15T09:15:00Z",
    "last_event": "step_finish",
    "last_message": "Refactored the parser module",
    "last_event_at": "2024-01-15T09:20:00Z",
    "tokens": {
      "input_tokens": 15000,
      "output_tokens": 4200,
      "total_tokens": 19200
    },
    "thread_id": "thread-1",
    "turn_id": "turn-2"
  },
  "retry": null,
  "recent_events": [
    {
      "at": "2024-01-15T09:20:00Z",
      "event": "step_finish",
      "message": "Refactored the parser module"
    }
  ],
  "last_error": null,
  "tracked": {}
}
```

If the issue is not currently running, the API returns `404 Not Found` with the JSON error envelope:

```json
{ "error": { "code": "issue_not_found", "message": "..." } }
```

### Error responses

All `/api/v1/*` error responses use the SPEC §13.7.2 envelope:

```json
{ "error": { "code": "<machine-readable code>", "message": "<human-readable description>" } }
```

Defined codes:

| Status | Code | When |
|--------|------|------|
| `404 Not Found` | `issue_not_found` | `GET /api/v1/<id>`, `POST /api/v1/<id>/cancel`, `DELETE /api/v1/retry/<id>` when no matching entry. |
| `405 Method Not Allowed` | `method_not_allowed` | An unsupported method is used on a defined route (e.g. `GET /api/v1/refresh`). |

### `POST /api/v1/<issue_identifier>/cancel`

Operational kill switch. Sets the worker's `cancelled` atomic; the local backend's watchdog sends `SIGKILL` to the entire CLI process group within ~1s. The orchestrator converts the worker exit into a retry per SPEC §7.3 and §8.4.

```bash
curl -X POST http://localhost:9090/api/v1/repo%2342/cancel
```

Returns `404` if no running entry matches the identifier. On success:

```json
{
  "cancelled": true,
  "issue_identifier": "repo#42",
  "issue_id": "I_kwDO...",
  "requested_at": "2026-05-09T12:34:56Z"
}
```

### `DELETE /api/v1/retry/<issue_identifier>`

Removes an issue from the retry queue and releases its claim. Useful when retries are pointless (the tracker state needs a manual fix) and you want the slot freed.

```bash
curl -X DELETE http://localhost:9090/api/v1/retry/repo%2342
```

Returns `404` if no retry entry matches the identifier.

## Logs and Tracing

Sympheo uses the `tracing` ecosystem. By default it logs at `INFO` level in human-readable format.

You can adjust the log level via the `RUST_LOG` environment variable:

```bash
RUST_LOG=sympheo=debug cargo run
```

For structured JSON logging (useful in production):

```bash
RUST_LOG=info cargo run -- --port 9090 2>&1 | jq .
```

> Note: JSON formatting may require enabling the `json` feature in tracing-subscriber if not already enabled in your build.

### Useful Log Patterns

| What to look for | Log level | Example message |
|------------------|-----------|-----------------|
| New issue detected | INFO | `issue detected` |
| Agent launched | INFO | `launching agent` |
| Agent finished | INFO | `turn finished` |
| Retry scheduled | WARN | `scheduling retry` |
| Workspace cleaned | INFO | `workspace removed` |
| Config reloaded | INFO | `workflow reloaded` |
| Validation error | ERROR | `startup validation failed` |

## Metrics

While there is no dedicated metrics endpoint yet, the dashboard exposes aggregate token usage and runtime counters since startup. For external monitoring, you can poll `/api/v1/state` periodically and forward the `agent_totals` object to your metrics stack.
