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

Generate Workflow Md For Claude Spec Phase
    [Arguments]    ${workflow_dir}    ${repo_url}    ${owner}    ${repo_name}    ${project_number}
    ...    ${project_id}    ${status_field_id}    ${in_progress_option_id}    ${poll_ms}=15000
    [Documentation]    Single ``Spec`` phase. Claude rewrites the issue body with a short
    ...                spec section, then moves the project item to ``In Progress``.
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
    ...    \ \ \ \ \ \ Write a brief 3-bullet specification for issue #{{issue.identifier}} ({{issue.title}}).
    ...    \ \ \ \ \ \ Update the issue body using:
    ...    \ \ \ \ \ \ \ \ gh issue edit {{issue.identifier}} --repo ${slug} --body '<original-body>\\n\\n## Spec\\n- ...\\n- ...\\n- ...'
    ...    \ \ \ \ \ \ Then move the project item to In Progress:
    ...    \ \ \ \ \ \ \ \ gh project item-edit --id ${ITEM_ID} --project-id ${project_id} --field-id ${status_field_id} --single-select-option-id ${in_progress_option_id}
    ...    \ \ \ \ \ \ Print "DONE_SPEC" and stop. Do not modify any code.
    ...    \ \ \ \ verifications: []
    ...    ---
    ...    {{ phase.prompt }}
    ...    ${EMPTY}
    Create File    ${workflow_dir}/WORKFLOW.md    ${content}

Generate Workflow Md For Claude Code Phase
    [Arguments]    ${workflow_dir}    ${repo_url}    ${owner}    ${repo_name}    ${project_number}    ${poll_ms}=15000
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
    ...    \ \ - name: build
    ...    \ \ \ \ state: In Progress
    ...    \ \ \ \ prompt: |
    ...    \ \ \ \ \ \ You are on a branch checked out from main for issue #{{issue.identifier}}.
    ...    \ \ \ \ \ \ Read the issue body for the exact task description and implement it.
    ...    \ \ \ \ \ \ Set up git auth + identity (GH_TOKEN is in your env):
    ...    \ \ \ \ \ \ \ \ gh auth setup-git
    ...    \ \ \ \ \ \ \ \ git config user.email sympheo-bot@noreply.local
    ...    \ \ \ \ \ \ \ \ git config user.name "sympheo-bot"
    ...    \ \ \ \ \ \ Stage and commit your change. Push the current branch to origin (use ``git push -u origin HEAD``).
    ...    \ \ \ \ \ \ Open a DRAFT pull request:
    ...    \ \ \ \ \ \ \ \ gh pr create --draft --repo ${slug} --base main --title "[e2e-test] {{issue.title}}" --body "Closes #{{issue.identifier}}"
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
