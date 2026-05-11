*** Settings ***
Documentation     Sympheo + opencode end-to-end "code phase" test.
...               Mirror of ``claude_code_phase.robot`` for the OpenCode reference adapter:
...               provisions a fresh ``[e2e-test]`` issue with a concrete, tiny code task in
...               the body, drops it in ``In Progress``, and verifies that opencode — driven by
...               sympheo — implements the change, pushes the orchestrator-created branch, and
...               opens a draft pull request that references the issue. Cleanup deletes the PR
...               + branch + issue.
...
...               Tagged ``opencode``; opt-in via ``--include opencode`` (excluded from the
...               default run). Requires whichever provider auth is staged in
...               ``~/.local/share/opencode/auth.json`` and ``GITHUB_TOKEN``. Real provider
...               + real PR; expect token spend.
Library           Process
Library           OperatingSystem
Library           Collections
Library           String
Library           DateTime
Library           libraries/github_project.py
Resource          resources/common.resource
Resource          resources/github/project.resource
Resource          resources/sympheo/daemon.resource
Resource          resources/sympheo/workflow.resource
Resource          resources/sympheo/state.resource

Suite Setup       Opencode Code Phase Setup
Suite Teardown    Opencode Code Phase Teardown

Force Tags        opencode


*** Variables ***
${MARKER_FILE_PREFIX}    e2e-marker-


*** Keywords ***
Opencode Code Phase Setup
    Assert Sympheo Binary Exists
    Ensure Github Token In Env
    Cleanup Stale E2E PRs
    Cleanup Stale E2E Issues
    Provision Test Issue
    Set Up Workflow Dir
    Stage Opencode Home
    ${meta}=    Get Project Status Metadata    ${OWNER}    ${PROJECT_NUMBER}
    Set Suite Variable    ${PROJECT_ID}             ${meta}[project_id]
    Set Suite Variable    ${STATUS_FIELD_ID}        ${meta}[status_field_id]
    ${options}=    Set Variable    ${meta}[options]
    Set Suite Variable    ${REVIEW_OPT_ID}          ${options}[review]
    ${stamp}=    Get Time    epoch
    Set Suite Variable    ${MARKER_FILE_NAME}    ${MARKER_FILE_PREFIX}${stamp}.txt
    Set Suite Variable    ${MARKER_CONTENT}      hello-from-opencode-${stamp}
    Set Test Issue Body With Code Task

Set Test Issue Body With Code Task
    [Documentation]    Replace the boilerplate body with a precise, machine-checkable instruction
    ...                so opencode has zero ambiguity about what to implement.
    ${task}=    Catenate    SEPARATOR=\n
    ...    [automated e2e — safe to ignore]
    ...    ${EMPTY}
    ...    Create a single new file at the repository root:
    ...    ${EMPTY}
    ...    - Path: `${MARKER_FILE_NAME}`
    ...    - Contents (single line, no trailing newline): `${MARKER_CONTENT}`
    ...    ${EMPTY}
    ...    That is the entire change. Do not edit any other file.
    ${result}=    Run Process    gh    issue    edit    ${ISSUE_NUMBER}
    ...    --repo    ${OWNER}/${REPO_NAME}    --body    ${task}
    Should Be Equal As Integers    ${result.rc}    0    msg=gh issue edit failed: ${result.stderr}

Opencode Code Phase Teardown
    Run Keyword And Ignore Error    Dump Last Sympheo State    ${PORT}
    Run Keyword And Ignore Error    Stop Sympheo Daemon
    Run Keyword And Ignore Error    Teardown Workflow Dir
    Run Keyword And Ignore Error    Cleanup Open PR For Issue
    ${should_keep}=    Evaluate    "${KEEP_ON_FAILURE}"=="1" and "${SUITE STATUS}"=="FAIL"
    IF    ${should_keep}
        Log To Console    [teardown] keeping issue + branches + PR (KEEP_ON_FAILURE=1, suite failed)
    ELSE
        Run Keyword And Ignore Error    Cleanup Test Issue
    END

Cleanup Open PR For Issue
    [Documentation]    Locate any open PR referencing our issue (regardless of head prefix) and
    ...                close + delete the branch via ``gh pr close --delete-branch``.
    ${have}=    Get Variable Value    ${ISSUE_NUMBER}    ${EMPTY}
    Return From Keyword If    '${have}'=='${EMPTY}'
    ${pr}=    Find Open Pr For Issue    ${OWNER}    ${REPO_NAME}    ${ISSUE_NUMBER}
    ${len}=    Get Length    ${pr}
    Return From Keyword If    ${len} == 0
    ${pr_number}=    Set Variable    ${pr}[number]
    Run Keyword And Ignore Error    Assert Safe To Close Pr    ${OWNER}    ${REPO_NAME}    ${pr_number}
    Run Keyword And Ignore Error    Close Pr    ${OWNER}    ${REPO_NAME}    ${pr_number}
    Log To Console    [teardown] closed PR #${pr_number} (head=${pr}[head_ref])

Wait For Pr To Be Opened For Issue
    [Arguments]    ${owner}    ${repo_name}    ${issue_number}    ${timeout}=15m
    Wait Until Keyword Succeeds    ${timeout}    30s    Pr Should Exist For Issue    ${owner}    ${repo_name}    ${issue_number}

Pr Should Exist For Issue
    [Arguments]    ${owner}    ${repo_name}    ${issue_number}
    ${pr}=    Find Open Pr For Issue    ${owner}    ${repo_name}    ${issue_number}
    ${len}=    Get Length    ${pr}
    Should Be True    ${len} > 0    msg=no open PR yet for issue #${issue_number}
    Set Suite Variable    ${OPEN_PR}    ${pr}


*** Test Cases ***
Opencode Implements Issue And Opens Draft PR
    [Documentation]    In Progress -> opencode writes the marker file, pushes branch, opens draft PR.
    [Tags]    agent    code-phase
    Generate Workflow Md For Opencode Code Phase    ${WORKFLOW_DIR}    ${REPO_URL}    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}
    ...    ${PROJECT_ID}    ${STATUS_FIELD_ID}    ${REVIEW_OPT_ID}
    ...    ${ISSUE_NUMBER}    ${ISSUE_TITLE}    ${ITEM_ID}

    # Move the freshly-created Todo issue to In Progress — kicks off sympheo.
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    In Progress    item_id=${ITEM_ID}

    Start Sympheo Daemon    ${WORKFLOW_DIR}/WORKFLOW.md    ${PORT}
    Wait For Sympheo Ready    ${PORT}    timeout=30s
    Trigger Sympheo Refresh    ${PORT}

    Wait For Issue To Appear In Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=120s

    # Opencode must implement the change, push, and open a draft PR. The PR
    # appearing on the remote is the verifiable signal — its existence
    # implies the branch was pushed (gh pr create requires it).
    Wait For Pr To Be Opened For Issue    ${OWNER}    ${REPO_NAME}    ${ISSUE_NUMBER}    timeout=15m

    Should Not Be Empty    ${OPEN_PR}[head_ref]
    Log To Console    [verify] PR opened: #${OPEN_PR}[number] head=${OPEN_PR}[head_ref] url=${OPEN_PR}[url]
