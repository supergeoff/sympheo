# Sympheo end-to-end harness

A Robot Framework suite that provisions throwaway GitHub resources (a private
repo + Project v2 + issue), drives `sympheo` against them, and asserts the
issue lifecycle works end-to-end.

## Prerequisites

* `gh` CLI authenticated (`gh auth status` → green) and `GITHUB_TOKEN`
  exported. Token must include `repo`, `project`, `delete_repo` scopes.
* `python3` available on `PATH` (the harness creates its own venv under
  `tests/e2e/.venv`).
* `cargo` (the script builds `sympheo` in release mode).
* `MODE=claude` only: `ANTHROPIC_API_KEY` exported and `claude` CLI on PATH.
  Real API calls cost money; default mode is `mock`.

## Run

```sh
./scripts/e2e.sh                   # mock mode (default), no API spend
./scripts/e2e.sh --mode=claude     # real claude CLI
./scripts/e2e.sh --keep-on-failure # leave repo + project around on failure
./scripts/e2e.sh --suite=tests/e2e/suites/01_happy_path.robot
```

Reports land in `tests/e2e/reports/<UTC-timestamp>/{output.xml, log.html, report.html}`.
The sympheo daemon's stdout/stderr are copied next to the report under
`sympheo-logs/`, alongside the generated `WORKFLOW.md` and `script.yaml`.

## Layout

```
tests/e2e/
├── README.md
├── requirements.txt           Python deps (robotframework, robotframework-requests, PyYAML)
├── libraries/
│   └── github_project.py      gh CLI wrappers (status field-id resolution, item-edit)
├── resources/
│   ├── github.resource        provision/teardown the repo + project + issue
│   ├── sympheo.resource       spawn / wait-ready / kill the daemon
│   ├── workflow.resource      generate WORKFLOW.md + mock script.yaml fixtures
│   └── assertions.resource    /api/v1/state polling + workspace cleanup checks
├── suites/
│   └── 01_happy_path.robot    Todo -> In Progress -> Done lifecycle
└── reports/                   gitignored
```
