# Sympheo end-to-end harness

Robot Framework suites that drive `sympheo` against the real
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

## Layout

```
e2e/
├── robot.args                argument file consumed by `robot -A`
├── requirements.txt
├── tests/                    *** Test Cases *** only — one suite per feature
│   ├── github/
│   │   └── issue_management.robot   gh wrappers + project transitions
│   ├── cli/
│   │   └── surface.robot            sympheo binary CLI (--help, errors)
│   └── agent/
│       ├── mock_pipeline.robot       orchestration with mock-cli (default)
│       ├── claude_pipeline.robot     full Todo→In Progress→Done lifecycle, claude-driven (opt-in)
│       ├── claude_spec_phase.robot   Spec phase: claude rewrites issue body + transitions to In Progress (opt-in)
│       └── claude_code_phase.robot   In Progress phase: claude implements + pushes branch + opens draft PR (opt-in)
├── resources/                *** Keywords *** — composable, no test cases
│   ├── common.resource
│   ├── github/project.resource
│   └── sympheo/{daemon,workflow,state}.resource
├── libraries/
│   └── github_project.py     gh CLI wrappers + safety pre-checks
├── data/                     fixtures (empty)
└── results/                  generated; gitignored
```

## Prerequisites

* `gh` CLI authenticated (`gh auth status` → green) and
  `GITHUB_TOKEN` exported. Token must include `repo`, `project`,
  `delete_repo`, `workflow` scopes.
* `robot` on `PATH`. Installed via `mise.local.toml`
  (`pipx:robotframework`); `mise activate` puts it on `PATH`.
  Alternatively: `pip install -r e2e/requirements.txt`.
* `cargo build --release` — every agent suite asserts the binary at
  `target/release/sympheo` exists.
* For the `claude` suite: `ANTHROPIC_API_KEY` exported and the
  `claude` CLI on `PATH`. Skipped automatically if `ANTHROPIC_API_KEY`
  is unset.

## Run

All commands assume cwd is the `e2e/` directory. Robot's `--pythonpath`
is set to `.`, so resources/libraries import from there regardless of
suite depth.

```sh
cd e2e

# Default: every suite except the `claude` one (free, fast)
robot -A robot.args

# Same as above, explicit
robot --pythonpath . --outputdir results --exclude claude tests/

# Run a single feature
robot --pythonpath . --outputdir results tests/github/
robot --pythonpath . --outputdir results tests/cli/
robot --pythonpath . --outputdir results tests/agent/mock_pipeline.robot

# Filter by tag
robot --pythonpath . --outputdir results --include happy-path tests/

# Run the claude suites (real API spend)
robot --pythonpath . --outputdir results --include claude tests/

# Run a specific claude scenario
robot --pythonpath . --outputdir results tests/agent/claude_spec_phase.robot
robot --pythonpath . --outputdir results tests/agent/claude_code_phase.robot

# Keep the test issue + branches when the run fails (debugging)
robot --pythonpath . --outputdir results --variable KEEP_ON_FAILURE:1 tests/

# Append a UTC timestamp so each run keeps its own report files
robot --pythonpath . --outputdir results --timestampoutputs tests/

# Open the latest report
xdg-open results/report.html
```

Reports land in `e2e/results/` (`output.xml`, `log.html`, `report.html`).
For agent suites, the sympheo daemon's stdout/stderr, generated
`WORKFLOW.md`, and (mock mode) `script.yaml` are copied next to the
report under `sympheo-logs/`.

## Configuration knobs

Pass any of these with `--variable name:value`:

| Variable           | Default                                      | Purpose                                               |
|--------------------|----------------------------------------------|-------------------------------------------------------|
| `MODE`             | `mock`                                       | `mock` (no API spend) or `claude` (real CLI)          |
| `PORT`             | `18080`                                      | sympheo HTTP port                                     |
| `KEEP_ON_FAILURE`  | `0`                                          | `1` keeps the test issue + branches around on failure |
| `OWNER`            | `supergeoff`                                 | repo owner                                            |
| `REPO_NAME`        | `sympheo`                                    | repo name                                             |
| `PROJECT_NUMBER`   | `2`                                          | Project v2 number                                     |
| `REPO_URL`         | `https://github.com/supergeoff/sympheo.git`  | repo URL passed to `WORKFLOW.md`                      |
| `SYMPHEO_BIN`      | `${EXECDIR}/../target/release/sympheo`       | binary path                                           |

## Safety guarantees

The harness operates against a real repo. The following invariants
are enforced in `libraries/github_project.py`:

* `assert_safe_to_delete_issue` — refuses to delete an issue whose
  title no longer starts with `[e2e-test]` or whose GraphQL node id
  doesn't match the one captured at setup.
* `assert_safe_to_delete_branch` — refuses to delete any branch
  whose name does not start with `sympheo/`.
* `assert_safe_to_remove_project_item` — refuses to remove a project
  item if its content URL no longer matches the issue this run
  created.
* Branches that pre-existed at the start of the run are recorded in
  a suite variable and explicitly excluded from the cleanup loop —
  only branches CREATED during this run are deleted.

If any pre-check fails, the cleanup keyword logs and skips that step
instead of damaging unrelated data.
