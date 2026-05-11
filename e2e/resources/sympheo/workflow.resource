*** Settings ***
Documentation     Generate ``WORKFLOW.md`` (and the mock ``script.yaml`` consumed by ``MockBackend``)
...               targeting the real ``supergeoff/sympheo`` repo + Project v2.
Library           OperatingSystem
Library           String
Resource          resources/common.resource


*** Keywords ***
Generate Mock Script
    [Arguments]    ${workflow_dir}
    [Documentation]    Writes the YAML script consumed by MockBackend (``src/agent/backend/mock.rs`` schema).
    ...                The 8s sleep keeps the worker visible in ``/api/v1/state`` long enough for the poll
    ...                to observe it (poll interval is 2s; without a sleep the whole turn finishes inside
    ...                a single tick).
    ${content}=    Catenate    SEPARATOR=\n
    ...    events:
    ...    \ \ - type: step_start
    ...    \ \ \ \ session_id: e2e-sess-1
    ...    \ \ \ \ message_id: e2e-msg-1
    ...    \ \ - type: text
    ...    \ \ \ \ message_id: e2e-msg-1
    ...    \ \ \ \ text: "mock turn output for e2e harness"
    ...    \ \ - type: sleep
    ...    \ \ \ \ delay_ms: 8000
    ...    \ \ - type: step_finish
    ...    \ \ \ \ message_id: e2e-msg-1
    ...    \ \ \ \ reason: stop
    ...    \ \ \ \ input_tokens: 5
    ...    \ \ \ \ output_tokens: 7
    ...    ${EMPTY}
    Create File    ${workflow_dir}/script.yaml    ${content}

Generate Workflow Md For Mock
    [Arguments]    ${workflow_dir}    ${repo_url}    ${owner}    ${repo_name}    ${project_number}    ${poll_ms}=2000
    Generate Mock Script    ${workflow_dir}
    ${slug}=    Set Variable    ${owner}/${repo_name}
    ${ws}=    Set Variable    ${workflow_dir}/workspace
    ${content}=    Catenate    SEPARATOR=\n
    ...    ---
    ...    tracker:
    ...    \ \ kind: github
    ...    \ \ api_key: $GITHUB_TOKEN
    ...    \ \ project_slug: ${slug}
    ...    \ \ project_number: ${project_number}
    ...    \ \ active_states: [Todo, In Progress]
    ...    \ \ terminal_states: [Done, Canceled]
    ...    cli:
    ...    \ \ command: mock-cli
    ...    \ \ options:
    ...    \ \ \ \ script: ${workflow_dir}/script.yaml
    ...    \ \ turn_timeout_ms: 30000
    ...    \ \ stall_timeout_ms: 15000
    ...    workspace:
    ...    \ \ root: ${ws}
    ...    \ \ repo_url: ${repo_url}
    ...    polling:
    ...    \ \ interval_ms: ${poll_ms}
    ...    agent:
    ...    \ \ max_concurrent_agents: 1
    ...    \ \ max_turns: 2
    ...    \ \ max_retry_attempts: 1
    ...    phases:
    ...    \ \ - name: build
    ...    \ \ \ \ state: In Progress
    ...    \ \ \ \ prompt: "Mock phase prompt for {{issue.identifier}}"
    ...    \ \ \ \ verifications:
    ...    \ \ \ \ \ \ - "true"
    ...    ---
    ...    Mock e2e prompt for issue {{issue.identifier}} — title: {{issue.title}}.
    ...    ${EMPTY}
    Create File    ${workflow_dir}/WORKFLOW.md    ${content}

Generate Hook Recorder Script
    [Arguments]    ${workflow_dir}    ${evidence_dir}
    [Documentation]    Writes ``hook.sh`` once: it takes the hook name as ``$1`` and dumps the
    ...                SYMPHEO_* env vars seen + ``pwd`` into ``${evidence_dir}/$1.env``. The
    ...                YAML hook bodies just call ``bash <path> <name>`` — keeps the WORKFLOW.md
    ...                free of multi-line shell content that Robot would have to escape twice.
    ${recorder}=    Catenate    SEPARATOR=\n
    ...    #!/usr/bin/env bash
    ...    set -euo pipefail
    ...    hook="$\{1:?hook name required}"
    ...    mkdir -p "${evidence_dir}"
    ...    {
    ...    \ \ echo "HOOK=$\{hook}"
    ...    \ \ echo "SYMPHEO_ISSUE_IDENTIFIER=$\{SYMPHEO_ISSUE_IDENTIFIER:-}"
    ...    \ \ echo "SYMPHEO_ISSUE_ID=$\{SYMPHEO_ISSUE_ID:-}"
    ...    \ \ echo "SYMPHEO_WORKSPACE_PATH=$\{SYMPHEO_WORKSPACE_PATH:-}"
    ...    \ \ echo "SYMPHEO_PHASE_NAME=$\{SYMPHEO_PHASE_NAME:-}"
    ...    \ \ echo "CWD=$(pwd)"
    ...    } > "${evidence_dir}/$\{hook}.env"
    ...    ${EMPTY}
    Create File    ${workflow_dir}/hook.sh    ${recorder}
    Run Process    chmod    +x    ${workflow_dir}/hook.sh

Generate Workflow Md For Hooks
    [Arguments]    ${workflow_dir}    ${owner}    ${repo_name}    ${project_number}    ${evidence_dir}    ${poll_ms}=2000
    [Documentation]    Mock-backed workflow that wires all four lifecycle hooks to a single
    ...                recorder script (one ``${hook_name}.env`` proof per hook in
    ...                ``${evidence_dir}``). The workflow deliberately OMITS
    ...                ``workspace.repo_url`` so the daemon falls back to ``after_create``
    ...                instead of cloning — that is the only path on which that hook fires.
    Generate Mock Script    ${workflow_dir}
    Generate Hook Recorder Script    ${workflow_dir}    ${evidence_dir}
    ${slug}=    Set Variable    ${owner}/${repo_name}
    ${ws}=    Set Variable    ${workflow_dir}/workspace
    ${recorder}=    Set Variable    ${workflow_dir}/hook.sh
    ${content}=    Catenate    SEPARATOR=\n
    ...    ---
    ...    tracker:
    ...    \ \ kind: github
    ...    \ \ api_key: $GITHUB_TOKEN
    ...    \ \ project_slug: ${slug}
    ...    \ \ project_number: ${project_number}
    ...    \ \ active_states: [In Progress]
    ...    \ \ terminal_states: [Done, Canceled]
    ...    cli:
    ...    \ \ command: mock-cli
    ...    \ \ options:
    ...    \ \ \ \ script: ${workflow_dir}/script.yaml
    ...    \ \ turn_timeout_ms: 30000
    ...    \ \ stall_timeout_ms: 15000
    ...    workspace:
    ...    \ \ root: ${ws}
    ...    polling:
    ...    \ \ interval_ms: ${poll_ms}
    ...    agent:
    ...    \ \ max_concurrent_agents: 1
    ...    \ \ max_turns: 1
    ...    \ \ max_retry_attempts: 0
    ...    hooks:
    ...    \ \ timeout_ms: 15000
    ...    \ \ after_create: bash ${recorder} after_create
    ...    \ \ before_run: bash ${recorder} before_run
    ...    \ \ after_run: bash ${recorder} after_run
    ...    \ \ before_remove: bash ${recorder} before_remove
    ...    phases:
    ...    \ \ - name: build
    ...    \ \ \ \ state: In Progress
    ...    \ \ \ \ prompt: "Hooks e2e mock phase for {{issue.identifier}}"
    ...    \ \ \ \ verifications: []
    ...    ---
    ...    Hooks e2e — issue {{issue.identifier}}.
    ...    ${EMPTY}
    Create File    ${workflow_dir}/WORKFLOW.md    ${content}

Generate Workflow Md For Claude Spec Phase
    [Arguments]    ${workflow_dir}    ${repo_url}    ${owner}    ${repo_name}    ${project_number}
    ...    ${project_id}    ${status_field_id}    ${in_progress_option_id}
    ...    ${issue_number}    ${issue_title}    ${item_id}    ${poll_ms}=15000
    [Documentation]    Single ``Spec`` phase. Claude rewrites the issue body with a short
    ...                spec section, then moves the project item to ``In Progress``.
    ...                The phase prompt bakes literal values for issue/project ids — Liquid
    ...                rendering inside ``phase.prompt`` is NOT recursive in sympheo today,
    ...                so ``{{ issue.identifier }}`` placeholders inside the phase string
    ...                would reach claude verbatim.
    ${slug}=    Set Variable    ${owner}/${repo_name}
    ${ws}=    Set Variable    ${workflow_dir}/workspace
    ${content}=    Catenate    SEPARATOR=\n
    ...    ---
    ...    tracker:
    ...    \ \ kind: github
    ...    \ \ api_key: $GITHUB_TOKEN
    ...    \ \ project_slug: ${slug}
    ...    \ \ project_number: ${project_number}
    ...    \ \ active_states: [Spec]
    ...    \ \ terminal_states: [In Progress, Done, Canceled]
    ...    cli:
    ...    \ \ command: claude
    ...    \ \ env:
    ...    \ \ \ \ HOME: ${CLAUDE_HOME_DIR}
    ...    \ \ \ \ ANTHROPIC_API_KEY: $ANTHROPIC_API_KEY
    ...    \ \ \ \ GITHUB_TOKEN: $GITHUB_TOKEN
    ...    \ \ \ \ GH_TOKEN: $GITHUB_TOKEN
    ...    \ \ turn_timeout_ms: 600000
    ...    \ \ stall_timeout_ms: 180000
    ...    \ \ read_timeout_ms: 90000
    ...    workspace:
    ...    \ \ root: ${ws}
    ...    \ \ repo_url: ${repo_url}
    ...    polling:
    ...    \ \ interval_ms: ${poll_ms}
    ...    agent:
    ...    \ \ max_concurrent_agents: 1
    ...    \ \ max_turns: 3
    ...    \ \ max_retry_attempts: 1
    ...    phases:
    ...    \ \ - name: spec
    ...    \ \ \ \ state: Spec
    ...    \ \ \ \ prompt: |
    ...    \ \ \ \ \ \ Write a brief 3-bullet specification for issue #${issue_number}: "${issue_title}".
    ...    \ \ \ \ \ \ Read the current body first, then update it using:
    ...    \ \ \ \ \ \ \ \ gh issue edit ${issue_number} --repo ${slug} --body "$(gh issue view ${issue_number} --repo ${slug} --json body --jq .body)$(printf '\\n\\n## Spec\\n- bullet 1\\n- bullet 2\\n- bullet 3\\n')"
    ...    \ \ \ \ \ \ Then move the project item to In Progress:
    ...    \ \ \ \ \ \ \ \ gh project item-edit --id ${item_id} --project-id ${project_id} --field-id ${status_field_id} --single-select-option-id ${in_progress_option_id}
    ...    \ \ \ \ \ \ Print "DONE_SPEC" and stop. Do not modify any code.
    ...    \ \ \ \ verifications: []
    ...    ---
    ...    {{ phase.prompt }}
    ...    ${EMPTY}
    Create File    ${workflow_dir}/WORKFLOW.md    ${content}

Generate Workflow Md For Claude Code Phase
    [Arguments]    ${workflow_dir}    ${repo_url}    ${owner}    ${repo_name}    ${project_number}
    ...    ${project_id}    ${status_field_id}    ${review_option_id}
    ...    ${issue_number}    ${issue_title}    ${item_id}    ${poll_ms}=15000
    [Documentation]    Single ``In Progress`` phase. Claude implements the change described in
    ...                the issue body, commits, pushes the branch, and opens a draft PR.
    ${slug}=    Set Variable    ${owner}/${repo_name}
    ${ws}=    Set Variable    ${workflow_dir}/workspace
    ${content}=    Catenate    SEPARATOR=\n
    ...    ---
    ...    tracker:
    ...    \ \ kind: github
    ...    \ \ api_key: $GITHUB_TOKEN
    ...    \ \ project_slug: ${slug}
    ...    \ \ project_number: ${project_number}
    ...    \ \ active_states: [In Progress]
    ...    \ \ terminal_states: [Review, Done, Canceled]
    ...    cli:
    ...    \ \ command: claude
    ...    \ \ env:
    ...    \ \ \ \ HOME: ${CLAUDE_HOME_DIR}
    ...    \ \ \ \ ANTHROPIC_API_KEY: $ANTHROPIC_API_KEY
    ...    \ \ \ \ GITHUB_TOKEN: $GITHUB_TOKEN
    ...    \ \ \ \ GH_TOKEN: $GITHUB_TOKEN
    ...    \ \ turn_timeout_ms: 900000
    ...    \ \ stall_timeout_ms: 240000
    ...    \ \ read_timeout_ms: 90000
    ...    workspace:
    ...    \ \ root: ${ws}
    ...    \ \ repo_url: ${repo_url}
    ...    polling:
    ...    \ \ interval_ms: ${poll_ms}
    ...    agent:
    ...    \ \ max_concurrent_agents: 1
    ...    \ \ max_turns: 1
    ...    \ \ max_retry_attempts: 0
    ...    hooks:
    ...    \ \ timeout_ms: 60000
    ...    \ \ before_run: |
    ...    \ \ \ \ set -euo pipefail
    ...    \ \ \ \ branch="sympheo/$\{SYMPHEO_ISSUE_IDENTIFIER#\\#}"
    ...    \ \ \ \ git config --local user.email "sympheo-bot@noreply.local"
    ...    \ \ \ \ git config --local user.name "sympheo-bot"
    ...    \ \ \ \ git fetch origin --quiet
    ...    \ \ \ \ if git show-ref --verify --quiet "refs/heads/$\{branch}"; then
    ...    \ \ \ \ \ \ git checkout "$\{branch}"
    ...    \ \ \ \ else
    ...    \ \ \ \ \ \ git checkout -b "$\{branch}" origin/main
    ...    \ \ \ \ fi
    ...    phases:
    ...    \ \ - name: build
    ...    \ \ \ \ state: In Progress
    ...    \ \ \ \ prompt: |
    ...    \ \ \ \ \ \ A working branch has already been checked out from main by the before_run hook, and git identity is already configured locally — do not change branch.
    ...    \ \ \ \ \ \ Read the issue body (``gh issue view ${issue_number} --repo ${slug}``) for the exact task description and implement it.
    ...    \ \ \ \ \ \ Run ``gh auth setup-git`` once so git push can authenticate via GH_TOKEN.
    ...    \ \ \ \ \ \ Stage and commit your change, then push the current branch with ``git push -u origin HEAD``. Do NOT push to main directly; never run ``git checkout main`` or ``git push origin main``.
    ...    \ \ \ \ \ \ Open a DRAFT pull request:
    ...    \ \ \ \ \ \ \ \ gh pr create --draft --repo ${slug} --base main --title "[e2e-test] ${issue_title}" --body "Closes #${issue_number}"
    ...    \ \ \ \ \ \ Then move the project item out of In Progress to stop the orchestrator's poll loop:
    ...    \ \ \ \ \ \ \ \ gh project item-edit --id ${item_id} --project-id ${project_id} --field-id ${status_field_id} --single-select-option-id ${review_option_id}
    ...    \ \ \ \ \ \ Print "DONE_BUILD" and stop.
    ...    \ \ \ \ verifications: []
    ...    ---
    ...    {{ phase.prompt }}
    ...    ${EMPTY}
    Create File    ${workflow_dir}/WORKFLOW.md    ${content}

Generate Workflow Md For Opencode Code Phase
    [Arguments]    ${workflow_dir}    ${repo_url}    ${owner}    ${repo_name}    ${project_number}
    ...    ${project_id}    ${status_field_id}    ${review_option_id}
    ...    ${issue_number}    ${issue_title}    ${item_id}
    ...    ${opencode_model}=openrouter/anthropic/claude-haiku-4.5    ${poll_ms}=15000
    [Documentation]    Mirror of ``Generate Workflow Md For Claude Code Phase`` but driven by
    ...                ``opencode run``. Same phase shape: implement, push branch, open draft PR,
    ...                move the project item to Review. The model is pinned in ``cli.command``
    ...                (``opencode run --model <id>``) — sympheo does not currently thread
    ...                ``cli.options.model`` into the opencode argv, and opencode's compiled-in
    ...                default points at a preview model that may have been retired.
    ...                ``OPENROUTER_API_KEY`` and ``ANTHROPIC_API_KEY`` are passed through so
    ...                opencode's env-based providers can authenticate inside the isolated
    ...                workspace.
    ${slug}=    Set Variable    ${owner}/${repo_name}
    ${ws}=    Set Variable    ${workflow_dir}/workspace
    ${content}=    Catenate    SEPARATOR=\n
    ...    ---
    ...    tracker:
    ...    \ \ kind: github
    ...    \ \ api_key: $GITHUB_TOKEN
    ...    \ \ project_slug: ${slug}
    ...    \ \ project_number: ${project_number}
    ...    \ \ active_states: [In Progress]
    ...    \ \ terminal_states: [Review, Done, Canceled]
    ...    cli:
    ...    \ \ command: opencode run --model ${opencode_model}
    ...    \ \ env:
    ...    \ \ \ \ HOME: ${OPENCODE_HOME_DIR}
    ...    \ \ \ \ GITHUB_TOKEN: $GITHUB_TOKEN
    ...    \ \ \ \ GH_TOKEN: $GITHUB_TOKEN
    ...    \ \ \ \ OPENROUTER_API_KEY: $OPENROUTER_API_KEY
    ...    \ \ \ \ ANTHROPIC_API_KEY: $ANTHROPIC_API_KEY
    ...    \ \ turn_timeout_ms: 900000
    ...    \ \ stall_timeout_ms: 240000
    ...    \ \ read_timeout_ms: 90000
    ...    workspace:
    ...    \ \ root: ${ws}
    ...    \ \ repo_url: ${repo_url}
    ...    polling:
    ...    \ \ interval_ms: ${poll_ms}
    ...    agent:
    ...    \ \ max_concurrent_agents: 1
    ...    \ \ max_turns: 1
    ...    \ \ max_retry_attempts: 0
    ...    hooks:
    ...    \ \ timeout_ms: 60000
    ...    \ \ before_run: |
    ...    \ \ \ \ set -euo pipefail
    ...    \ \ \ \ branch="sympheo/$\{SYMPHEO_ISSUE_IDENTIFIER#\\#}"
    ...    \ \ \ \ git config --local user.email "sympheo-bot@noreply.local"
    ...    \ \ \ \ git config --local user.name "sympheo-bot"
    ...    \ \ \ \ git fetch origin --quiet
    ...    \ \ \ \ if git show-ref --verify --quiet "refs/heads/$\{branch}"; then
    ...    \ \ \ \ \ \ git checkout "$\{branch}"
    ...    \ \ \ \ else
    ...    \ \ \ \ \ \ git checkout -b "$\{branch}" origin/main
    ...    \ \ \ \ fi
    ...    phases:
    ...    \ \ - name: build
    ...    \ \ \ \ state: In Progress
    ...    \ \ \ \ prompt: |
    ...    \ \ \ \ \ \ A working branch has already been checked out from main by the before_run hook, and git identity is already configured locally — do not change branch.
    ...    \ \ \ \ \ \ Read the issue body (``gh issue view ${issue_number} --repo ${slug}``) for the exact task description and implement it.
    ...    \ \ \ \ \ \ Run ``gh auth setup-git`` once so git push can authenticate via GH_TOKEN.
    ...    \ \ \ \ \ \ Stage and commit your change, then push the current branch with ``git push -u origin HEAD``. Do NOT push to main directly; never run ``git checkout main`` or ``git push origin main``.
    ...    \ \ \ \ \ \ Open a DRAFT pull request:
    ...    \ \ \ \ \ \ \ \ gh pr create --draft --repo ${slug} --base main --title "[e2e-test] ${issue_title}" --body "Closes #${issue_number}"
    ...    \ \ \ \ \ \ Then move the project item out of In Progress to stop the orchestrator's poll loop:
    ...    \ \ \ \ \ \ \ \ gh project item-edit --id ${item_id} --project-id ${project_id} --field-id ${status_field_id} --single-select-option-id ${review_option_id}
    ...    \ \ \ \ \ \ Print "DONE_BUILD" and stop.
    ...    \ \ \ \ verifications: []
    ...    ---
    ...    {{ phase.prompt }}
    ...    ${EMPTY}
    Create File    ${workflow_dir}/WORKFLOW.md    ${content}

Generate Workflow Md For Claude
    [Arguments]    ${workflow_dir}    ${repo_url}    ${owner}    ${repo_name}    ${project_number}    ${poll_ms}=5000
    ${slug}=    Set Variable    ${owner}/${repo_name}
    ${ws}=    Set Variable    ${workflow_dir}/workspace
    ${content}=    Catenate    SEPARATOR=\n
    ...    ---
    ...    tracker:
    ...    \ \ kind: github
    ...    \ \ api_key: $GITHUB_TOKEN
    ...    \ \ project_slug: ${slug}
    ...    \ \ project_number: ${project_number}
    ...    \ \ active_states: [Todo, In Progress]
    ...    \ \ terminal_states: [Done, Canceled]
    ...    cli:
    ...    \ \ command: claude
    ...    \ \ env:
    ...    \ \ \ \ ANTHROPIC_API_KEY: $ANTHROPIC_API_KEY
    ...    \ \ turn_timeout_ms: 600000
    ...    \ \ stall_timeout_ms: 120000
    ...    \ \ read_timeout_ms: 90000
    ...    workspace:
    ...    \ \ root: ${ws}
    ...    \ \ repo_url: ${repo_url}
    ...    polling:
    ...    \ \ interval_ms: ${poll_ms}
    ...    agent:
    ...    \ \ max_concurrent_agents: 1
    ...    \ \ max_turns: 5
    ...    \ \ max_retry_attempts: 1
    ...    phases:
    ...    \ \ - name: implement
    ...    \ \ \ \ state: In Progress
    ...    \ \ \ \ prompt: "Implement issue {{issue.identifier}}: {{issue.title}}. Commit and push."
    ...    \ \ \ \ verifications:
    ...    \ \ \ \ \ \ - "git status"
    ...    ---
    ...    You are working on issue {{issue.identifier}} — {{issue.title}}.
    ...    ${EMPTY}
    Create File    ${workflow_dir}/WORKFLOW.md    ${content}
