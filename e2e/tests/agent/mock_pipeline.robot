*** Settings ***
Documentation     Full orchestration pipeline against the mock CLI backend.
...               Drives one ``[e2e-test]`` issue through ``Todo`` → ``In Progress`` → ``Done``,
...               verifies the orchestrator picks it up on ``In Progress`` and releases it on
...               ``Done``, then cleans up the issue + project item + any ``sympheo/*`` branch.
...               Zero API spend; safe to run on every commit.
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

Suite Setup       Mock Pipeline Setup
Suite Teardown    Mock Pipeline Teardown


*** Keywords ***
Mock Pipeline Setup
    Assert Sympheo Binary Exists
    Provision Test Issue
    Set Up Workflow Dir

Mock Pipeline Teardown
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
Sympheo Drives Mock Issue Through Lifecycle
    [Documentation]    Issue Todo -> In Progress -> Done; orchestrator dispatches and releases it via the mock backend.
    [Tags]    agent    mock    happy-path
    Generate Workflow Md For Mock    ${WORKFLOW_DIR}    ${REPO_URL}    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}

    Start Sympheo Daemon    ${WORKFLOW_DIR}/WORKFLOW.md    ${PORT}
    Wait For Sympheo Ready    ${PORT}    timeout=30s

    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    In Progress    item_id=${ITEM_ID}
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Appear In Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=60s

    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Done    item_id=${ITEM_ID}
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Leave Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=90s
