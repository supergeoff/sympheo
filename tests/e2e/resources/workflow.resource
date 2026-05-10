*** Settings ***
Documentation     Generate WORKFLOW.md (and mock script.yaml) fixtures pointing at the real ``supergeoff/sympheo`` repo + Project v2 #2.
Library           OperatingSystem
Library           String


*** Variables ***
${ACTIVE_STATES}      Todo,In Progress
${TERMINAL_STATES}    Done,Canceled


*** Keywords ***
Generate Mock Script
    [Arguments]    ${workflow_dir}
    [Documentation]    Writes the YAML script consumed by MockBackend (``src/agent/backend/mock.rs`` schema).
    ...    The 8s sleep keeps the worker visible in ``/api/v1/state`` long enough for the poll to observe it
    ...    (the test polls every 2s; without a sleep, the whole turn completes inside one tick).
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
