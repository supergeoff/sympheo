*** Settings ***
Documentation     Sympheo workspace-hooks conformance e2e.
...               Drives one ``[e2e-test]`` issue through ``Todo`` → ``In Progress`` → ``Done``
...               with a mock-cli backend (zero token spend) and verifies that all four
...               documented lifecycle hooks — ``after_create``, ``before_run``, ``after_run``,
...               ``before_remove`` — fire with the documented SYMPHEO_* env vars
...               (``SYMPHEO_ISSUE_IDENTIFIER``, ``SYMPHEO_ISSUE_ID``, ``SYMPHEO_WORKSPACE_PATH``;
...               plus ``SYMPHEO_PHASE_NAME`` for the two run-bracket hooks). Each hook script
...               appends its observation to a file under ``${EVIDENCE_DIR}``; the test reads
...               those files back after the lifecycle completes.
...
...               This suite is tagged ``mock`` + ``hooks``; it is part of the default run.
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

Suite Setup       Hooks Pipeline Setup
Suite Teardown    Hooks Pipeline Teardown


*** Keywords ***
Hooks Pipeline Setup
    Assert Sympheo Binary Exists
    Ensure Github Token In Env
    Cleanup Stale E2E Issues
    Provision Test Issue
    Set Up Workflow Dir
    ${evidence}=    Set Variable    ${WORKFLOW_DIR}/hook-evidence
    Create Directory    ${evidence}
    Set Suite Variable    ${EVIDENCE_DIR}    ${evidence}

Hooks Pipeline Teardown
    Run Keyword And Ignore Error    Copy Evidence To Report Dir
    Run Keyword And Ignore Error    Dump Last Sympheo State    ${PORT}
    Run Keyword And Ignore Error    Stop Sympheo Daemon
    Run Keyword And Ignore Error    Teardown Workflow Dir
    ${should_keep}=    Evaluate    "${KEEP_ON_FAILURE}"=="1" and "${SUITE STATUS}"=="FAIL"
    IF    ${should_keep}
        Log To Console    [teardown] keeping issue + branches (KEEP_ON_FAILURE=1, suite failed)
    ELSE
        Run Keyword And Ignore Error    Cleanup Test Issue
    END

Copy Evidence To Report Dir
    [Documentation]    Persist the hook-evidence dir alongside Robot's output so failures are inspectable.
    ${have}=    Get Variable Value    ${EVIDENCE_DIR}    ${EMPTY}
    Return From Keyword If    '${have}'=='${EMPTY}'
    ${exists}=    Run Keyword And Return Status    Directory Should Exist    ${EVIDENCE_DIR}
    Return From Keyword If    not ${exists}
    Run Keyword And Ignore Error    Remove Directory    ${OUTPUT_DIR}/hook-evidence    recursive=True
    Copy Directory    ${EVIDENCE_DIR}    ${OUTPUT_DIR}/hook-evidence

Read Hook Env File
    [Arguments]    ${name}
    ${path}=    Set Variable    ${EVIDENCE_DIR}/${name}.env
    File Should Exist    ${path}    msg=${name} hook did not run (proof file missing at ${path})
    ${raw}=    Get File    ${path}
    ${dict}=    Evaluate    dict(line.split('=', 1) for line in $raw.strip().splitlines() if '=' in line)
    RETURN    ${dict}

Assert Sympheo Env In Hook
    [Arguments]    ${name}    ${expect_phase}=${EMPTY}
    ${env}=    Read Hook Env File    ${name}
    Should Be Equal    ${env}[HOOK]    ${name}    msg=hook env file labels wrong hook name
    Should Be Equal As Strings    ${env}[SYMPHEO_ISSUE_ID]    ${ISSUE_NODE_ID}
    ${expected_identifier}=    Set Variable    ${REPO_NAME}#${ISSUE_NUMBER}
    Should Be Equal As Strings    ${env}[SYMPHEO_ISSUE_IDENTIFIER]    ${expected_identifier}
    Should Not Be Empty    ${env}[SYMPHEO_WORKSPACE_PATH]
    Should Match Regexp    ${env}[SYMPHEO_WORKSPACE_PATH]    ${WORKFLOW_DIR}/workspace/.*
    IF    '${expect_phase}'!='${EMPTY}'
        Should Be Equal As Strings    ${env}[SYMPHEO_PHASE_NAME]    ${expect_phase}
    ELSE
        Should Be Equal As Strings    ${env}[SYMPHEO_PHASE_NAME]    ${EMPTY}
    END

Wait For Hook Evidence
    [Arguments]    ${name}    ${timeout}=60s
    Wait Until Keyword Succeeds    ${timeout}    2s    File Should Exist    ${EVIDENCE_DIR}/${name}.env


*** Test Cases ***
Sympheo Fires Four Lifecycle Hooks With Documented Env
    [Documentation]    Drives the issue Todo -> In Progress -> Done and asserts each of the four
    ...                hooks left a proof file with the documented SYMPHEO_* env vars.
    [Tags]    mock    hooks    happy-path
    Generate Workflow Md For Hooks    ${WORKFLOW_DIR}    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}    ${EVIDENCE_DIR}

    Start Sympheo Daemon    ${WORKFLOW_DIR}/WORKFLOW.md    ${PORT}
    Wait For Sympheo Ready    ${PORT}    timeout=30s

    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    In Progress    item_id=${ITEM_ID}
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Appear In Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=60s

    # after_create + before_run should fire as soon as the worker starts.
    Wait For Hook Evidence    after_create    timeout=60s
    Wait For Hook Evidence    before_run    timeout=60s

    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Done    item_id=${ITEM_ID}
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Leave Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=90s

    # after_run + before_remove fire as the worker tears down.
    Wait For Hook Evidence    after_run    timeout=60s
    Wait For Hook Evidence    before_remove    timeout=60s

    # after_create / before_remove are dispatched outside a phase context; the
    # daemon does not (currently) supply SYMPHEO_PHASE_NAME for those. The two
    # run-bracket hooks DO see the active phase ("build" here).
    Assert Sympheo Env In Hook    after_create
    Assert Sympheo Env In Hook    before_run      expect_phase=build
    Assert Sympheo Env In Hook    after_run       expect_phase=build
    Assert Sympheo Env In Hook    before_remove

    Log To Console    [verify] all four lifecycle hooks fired with documented env
