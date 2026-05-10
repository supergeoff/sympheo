# Sympheo end-to-end harness

A Robot Framework suite that drives `sympheo` against the **real**
`supergeoff/sympheo` repo and Project v2 #2.

The harness creates one issue prefixed with `[e2e-test]`, walks it
through `Todo` → `In Progress` → `Done`, verifies the orchestrator
dispatches and releases it correctly, then deletes the issue, removes
the project item, and tears down any `sympheo/*` branch the
orchestrator pushed during the run.

Every cleanup step is gated by safety pre-checks (issue title still
starts with `[e2e-test]`, node id still matches the one captured at
setup, branch name starts with `sympheo/`) so a typo cannot damage
real data on the repo.

## Prerequisites

* `gh` CLI authenticated (`gh auth status` → green) and
  `GITHUB_TOKEN` exported. Token must include `repo`, `project`,
  `delete_repo`, `workflow` scopes.
* `robot` on `PATH`. Installed via `mise.local.toml`
  (`pipx:robotframework`); `mise activate` puts it on `PATH`
  automatically. No project-local venv needed.
* `cargo` (build the binary first — the suite asserts it exists).
* `MODE=claude` only: `ANTHROPIC_API_KEY` exported and `claude` CLI
  on `PATH`. Real API calls cost money; default mode is `mock`.

## Build the binary

The suite asserts `target/release/sympheo` exists in Suite Setup;
build it first:

```sh
cargo build --release
```

## Run

All commands assume cwd is the project root.

```sh
# Run all e2e suites
robot --outputdir tests/e2e/reports tests/e2e/suites/

# Run a single suite
robot --outputdir tests/e2e/reports tests/e2e/suites/01_happy_path.robot

# Run by tag
robot --include happy-path --outputdir tests/e2e/reports tests/e2e/suites/

# Run a specific test
robot --test "Sympheo Drives Issue Through Lifecycle" --outputdir tests/e2e/reports tests/e2e/suites/

# Switch backend mode (mock by default; claude requires ANTHROPIC_API_KEY)
robot --variable MODE:claude --outputdir tests/e2e/reports tests/e2e/suites/

# Keep gh resources around when the suite fails (debugging)
robot --variable KEEP_ON_FAILURE:1 --outputdir tests/e2e/reports tests/e2e/suites/

# Append a UTC timestamp to output filenames (history)
robot --timestampoutputs --outputdir tests/e2e/reports tests/e2e/suites/

# View the latest report
xdg-open tests/e2e/reports/report.html
```

The HTML report lands in `tests/e2e/reports/report.html`. The
sympheo daemon's stdout/stderr, generated `WORKFLOW.md`, and (for
mock mode) `script.yaml` are copied next to it under
`sympheo-logs/`.

## Configuration knobs

Pass any of these with `--variable name:value`:

| Variable           | Default                                      | Purpose                                                |
|--------------------|----------------------------------------------|--------------------------------------------------------|
| `MODE`             | `mock`                                       | `mock` (no API spend) or `claude` (real CLI)           |
| `PORT`             | `18080`                                      | sympheo HTTP port                                      |
| `KEEP_ON_FAILURE`  | `0`                                          | `1` to keep the test issue + branch around on failure  |
| `OWNER`            | `supergeoff`                                 | repo owner                                             |
| `REPO_NAME`        | `sympheo`                                    | repo name                                              |
| `PROJECT_NUMBER`   | `2`                                          | Project v2 number                                      |
| `REPO_URL`         | `https://github.com/supergeoff/sympheo.git`  | repo URL passed to `WORKFLOW.md`                       |
| `SYMPHEO_BIN`      | `<suite>/../../../target/release/sympheo`    | binary path (absolute or relative to suite file)       |

## Layout

```
tests/e2e/
├── README.md
├── libraries/
│   └── github_project.py      gh CLI wrappers + safety pre-checks
├── resources/
│   ├── github.resource        create / cleanup the e2e issue + project item + branch
│   ├── sympheo.resource       spawn / wait-ready / kill the daemon (curl-based)
│   ├── workflow.resource      generate WORKFLOW.md + mock script.yaml
│   └── assertions.resource    /api/v1/state polling + workspace cleanup checks
├── suites/
│   └── 01_happy_path.robot    Todo -> In Progress -> Done lifecycle
└── reports/                   gitignored
```

## Safety guarantees

The harness operates against a real repo. The following invariants
are enforced in `libraries/github_project.py`:

* `assert_safe_to_delete_issue` — refuses to delete an issue whose
  title no longer starts with `[e2e-test]` or whose GraphQL node id
  doesn't match the one captured at setup.
* `assert_safe_to_delete_branch` — refuses to delete any branch
  whose name does not start with `sympheo/`.
* `assert_safe_to_remove_project_item` — refuses to remove a
  project item if its content URL no longer matches the issue this
  run created.
* Branches that pre-existed at the start of the run are recorded in
  a suite variable and explicitly excluded from the cleanup loop —
  only branches CREATED during this run are deleted.

If any pre-check fails, the cleanup keyword logs and skips that
specific step instead of damaging unrelated data.
