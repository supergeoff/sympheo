*** Settings ***
Documentation     Sympheo e2e happy path against the REAL ``supergeoff/sympheo`` repo + Project v2 #2.
...
...               Drives one ``[e2e-test]``-prefixed issue through ``Todo`` → ``In Progress`` → ``Done``,
...               verifies the orchestrator picks it up on ``In Progress`` and releases it on ``Done``,
...               then deletes the issue + project item + any orchestrator-pushed ``sympheo/*`` branch.
...               Cleanup is gated by safety pre-checks in ``libraries/github_project.py``.

Library           Process
Library           OperatingSystem
Library           Collections
Library           String
Library           ../libraries/github_project.py
Resource          ../resources/github.resource
Resource          ../resources/sympheo.resource
Resource          ../resources/workflow.resource
Resource          ../resources/assertions.resource

Suite Setup       Suite Setup Steps
Suite Teardown    Suite Cleanup


*** Variables ***
${MODE}              mock
${PORT}              18080
${KEEP_ON_FAILURE}   0
# Default the binary path relative to the suite file. ``${EXECDIR}`` is the
# directory robot was invoked from; ``${CURDIR}`` is the suite file's dir,
# which is what we actually want when callers cd into the project root and
# run ``robot tests/e2e/suites/``. Using ``${CURDIR}`` makes the default
# stable regardless of cwd.
${SYMPHEO_BIN}       ${CURDIR}/../../../target/release/sympheo


*** Keywords ***
Suite Setup Steps
    Assert Sympheo Binary Exists
    Provision GitHub E2E Resources
    Set Up Workflow Dir

Assert Sympheo Binary Exists
    [Documentation]    Fail fast in Suite Setup if the release binary is missing.
    ${exists}=    Run Keyword And Return Status    File Should Exist    ${SYMPHEO_BIN}
    IF    not ${exists}
        Fail    sympheo binary not found at ${SYMPHEO_BIN}; run `cargo build --release` first
    END
    Log To Console    [setup] sympheo binary at ${SYMPHEO_BIN}

Suite Cleanup
    [Documentation]    Tolerant teardown: dump state, stop daemon, copy logs, remove gh resources.
    Run Keyword And Ignore Error    Dump Last Sympheo State    ${PORT}
    Run Keyword And Ignore Error    Stop Sympheo Daemon
    Run Keyword And Ignore Error    Teardown Workflow Dir
    ${should_keep}=    Evaluate    "${KEEP_ON_FAILURE}"=="1" and "${SUITE STATUS}"=="FAIL"
    IF    ${should_keep}
        Log To Console    [teardown] keeping issue + branches (KEEP_ON_FAILURE=1, suite failed)
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
    ...    Generate Workflow Md For Mock      ${WORKFLOW_DIR}    ${REPO_URL}    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}

    Start Sympheo Daemon    ${WORKFLOW_DIR}/WORKFLOW.md    ${PORT}
    Wait For Sympheo Ready    ${PORT}    timeout=30s

    # Move issue to "In Progress" — sympheo should observe it on the next poll and dispatch.
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    In Progress    item_id=${ITEM_ID}
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Appear In Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=60s

    # Move to Done — terminal — sympheo should drop it from running and clean up.
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Done    item_id=${ITEM_ID}
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Leave Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=90s
