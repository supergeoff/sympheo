*** Settings ***
Documentation     Sympheo e2e happy path: GitHub Project issue Todo -> In Progress -> Done.
...               Verifies the orchestrator picks up the issue when moved to "In Progress",
...               drives the worker to completion, and releases it once moved to "Done".

Library           Process
Library           OperatingSystem
Library           RequestsLibrary
Library           Collections
Library           String
Library           ../libraries/github_project.py
Resource          ../resources/github.resource
Resource          ../resources/sympheo.resource
Resource          ../resources/workflow.resource
Resource          ../resources/assertions.resource

Suite Setup       Run Keywords
...               Provision GitHub E2E Resources    AND
...               Set Up Workflow Dir
Suite Teardown    Suite Cleanup


*** Variables ***
${MODE}              mock
${PORT}              18080
${KEEP_ON_FAILURE}   0


*** Keywords ***
Suite Cleanup
    [Documentation]    Tolerant teardown: dump state, stop daemon, copy logs, optionally keep gh resources.
    Run Keyword And Ignore Error    Dump Last Sympheo State    ${PORT}
    Run Keyword And Ignore Error    Stop Sympheo Daemon
    Run Keyword And Ignore Error    Teardown Workflow Dir
    ${should_keep}=    Evaluate    "${KEEP_ON_FAILURE}"=="1" and "${SUITE STATUS}"=="FAIL"
    IF    ${should_keep}
        Log To Console    [teardown] keeping repo ${OWNER}/${REPO_NAME} and project ${PROJECT_NUMBER} (KEEP_ON_FAILURE=1, suite failed)
    ELSE
        Run Keyword And Ignore Error    Teardown GitHub E2E Resources
    END


*** Test Cases ***
Sympheo Drives Issue Through Lifecycle
    [Documentation]    Issue Todo -> In Progress -> Done; orchestrator dispatches and releases it.
    [Tags]    happy-path    ${MODE}
    Run Keyword If    '${MODE}'=='claude'
    ...    Generate Workflow Md For Claude    ${WORKFLOW_DIR}    ${REPO_URL}    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}
    ...    ELSE
    ...    Generate Workflow Md For Mock    ${WORKFLOW_DIR}    ${REPO_URL}    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}

    Start Sympheo Daemon    ${WORKFLOW_DIR}/WORKFLOW.md    ${PORT}
    Wait For Sympheo Ready    ${PORT}    timeout=30s

    # Move issue to "In Progress" — sympheo should observe it on the next poll and dispatch.
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    In Progress
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Appear In Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=60s

    # Move to Done — terminal — sympheo should drop it from running and clean up.
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Done
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Leave Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=90s
