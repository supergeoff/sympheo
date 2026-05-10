*** Settings ***
Documentation     Full orchestration pipeline against the real ``claude`` CLI adapter.
...               Tagged ``claude`` so it is excluded from the default run — opt-in via
...               ``--include claude``. Requires ``ANTHROPIC_API_KEY`` exported and the
...               ``claude`` binary on ``PATH``. Real API calls; expect token spend.
Library           Process
Library           OperatingSystem
Library           Collections
Library           String
Library           libraries/github_project.py
Resource          resources/common.resource
Resource          resources/github/project.resource
Resource          resources/sympheo/daemon.resource
Resource          resources/sympheo/workflow.resource
Resource          resources/sympheo/state.resource

Suite Setup       Claude Pipeline Setup
Suite Teardown    Claude Pipeline Teardown

Force Tags        claude


*** Keywords ***
Claude Pipeline Setup
    Assert Sympheo Binary Exists
    Skip If    "%{ANTHROPIC_API_KEY=}"=="${EMPTY}"    ANTHROPIC_API_KEY not set; skipping claude pipeline
    Provision Test Issue
    Set Up Workflow Dir

Claude Pipeline Teardown
    Run Keyword And Ignore Error    Dump Last Sympheo State    ${PORT}
    Run Keyword And Ignore Error    Stop Sympheo Daemon
    Run Keyword And Ignore Error    Teardown Workflow Dir
    ${should_keep}=    Evaluate    "${KEEP_ON_FAILURE}"=="1" and "${SUITE STATUS}"=="FAIL"
    IF    ${should_keep}
        Log To Console    [teardown] keeping issue + branches (KEEP_ON_FAILURE=1, suite failed)
    ELSE
        Run Keyword And Ignore Error    Cleanup Test Issue
    END


*** Test Cases ***
Sympheo Drives Claude Issue Through Lifecycle
    [Documentation]    Issue Todo -> In Progress -> Done; orchestrator dispatches the real ``claude`` CLI.
    [Tags]    agent    happy-path
    Generate Workflow Md For Claude    ${WORKFLOW_DIR}    ${REPO_URL}    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}

    Start Sympheo Daemon    ${WORKFLOW_DIR}/WORKFLOW.md    ${PORT}
    Wait For Sympheo Ready    ${PORT}    timeout=30s

    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    In Progress    item_id=${ITEM_ID}
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Appear In Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=120s

    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Done    item_id=${ITEM_ID}
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Leave Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=600s
