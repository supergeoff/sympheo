# Monitoring

Sympheo exposes a built-in HTTP dashboard and a REST API so you can observe what the orchestrator is doing in real time.

## Dashboard

When `server.port` is configured (or `--port` is passed on the CLI), Sympheo starts a web server.

Open your browser to:

```
http://localhost:<port>/
```

The dashboard auto-refreshes every 5 seconds and shows:

### Summary Cards

| Card | Meaning |
|------|---------|
| **Running** | Number of active agent sessions. |
| **Retrying** | Number of issues waiting in the retry queue. |
| **Tokens** | Total input + output tokens consumed across all sessions since startup. |
| **Runtime** | Total wall-clock seconds agents have been running since startup. |

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
| Last Message | Truncated text of the most recent message. |
| Tokens (in/out) | Token counters for the current session. |

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
      "error": "Agent exited with code 1"
    }
  ],
  "codex_totals": {
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

Response:

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
  "turn_count": 2,
  "retry_attempt": 0,
  "session": {
    "session_id": "sess-abc",
    "thread_id": "thread-1",
    "turn_id": "turn-2",
    "last_event": "step_finish",
    "last_message": "Refactored the parser module",
    "last_timestamp": "2024-01-15T09:20:00Z",
    "input_tokens": 15000,
    "output_tokens": 4200,
    "total_tokens": 19200,
    "turn_count": 2
  },
  "recent_events": [
    {
      "event": "step_finish",
      "at": "2024-01-15T09:20:00Z"
    }
  ]
}
```

If the issue is not currently running, the API returns `404 Not Found`.

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

While there is no dedicated metrics endpoint yet, the dashboard exposes aggregate token usage and runtime counters since startup. For external monitoring, you can poll `/api/v1/state` periodically and forward the `codex_totals` object to your metrics stack.
