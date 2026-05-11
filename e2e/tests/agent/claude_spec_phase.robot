*** Settings ***
Documentation     Sympheo + claude end-to-end "spec phase" test.
...               Provisions a fresh ``[e2e-test]`` issue, drops it in the ``Spec`` status,
...               and verifies that claude — driven by sympheo with a ``Spec``-only phase —
...               (a) rewrites the issue body with a spec section and (b) transitions the
...               project item to ``In Progress``. Tagged ``claude`` so it is excluded from
...               the default run.
...
...               Requires ``ANTHROPIC_API_KEY`` and ``GITHUB_TOKEN`` exported. Real API
...               calls; expect token spend.
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

Suite Setup       Spec Phase Setup
Suite Teardown    Spec Phase Teardown

Force Tags        claude


*** Variables ***
${SPEC_BODY_MARKER}    ## Spec


*** Keywords ***
Spec Phase Setup
    Assert Sympheo Binary Exists
    Ensure Github Token In Env
    Cleanup Stale E2E PRs
    Cleanup Stale E2E Issues
    Provision Test Issue
    Set Up Workflow Dir
    Stage Claude OAuth Home
    ${meta}=    Get Project Status Metadata    ${OWNER}    ${PROJECT_NUMBER}
    Set Suite Variable    ${PROJECT_ID}             ${meta}[project_id]
    Set Suite Variable    ${STATUS_FIELD_ID}        ${meta}[status_field_id]
    ${options}=    Set Variable    ${meta}[options]
    Set Suite Variable    ${IN_PROGRESS_OPT_ID}     ${options}[in progress]
    Log To Console    [setup] project=${PROJECT_ID} field=${STATUS_FIELD_ID} in_progress=${IN_PROGRESS_OPT_ID}

Spec Phase Teardown
    Run Keyword And Ignore Error    Dump Last Sympheo State    ${PORT}
    Run Keyword And Ignore Error    Stop Sympheo Daemon
    Run Keyword And Ignore Error    Teardown Workflow Dir
    ${should_keep}=    Evaluate    "${KEEP_ON_FAILURE}"=="1" and "${SUITE STATUS}"=="FAIL"
    IF    ${should_keep}
        Log To Console    [teardown] keeping issue + branches (KEEP_ON_FAILURE=1, suite failed)
    ELSE
        Run Keyword And Ignore Error    Cleanup Test Issue
    END

Wait For Issue Body To Contain
    [Arguments]    ${owner}    ${repo_name}    ${issue_number}    ${needle}    ${timeout}=10m
    Wait Until Keyword Succeeds    ${timeout}    30s    Issue Body Should Contain    ${owner}    ${repo_name}    ${issue_number}    ${needle}

Issue Body Should Contain
    [Arguments]    ${owner}    ${repo_name}    ${issue_number}    ${needle}
    ${body}=    Get Issue Body    ${owner}    ${repo_name}    ${issue_number}
    Should Contain    ${body}    ${needle}    msg=issue #${issue_number} body still missing '${needle}'

Wait For Project Item Status
    [Arguments]    ${owner}    ${project_number}    ${item_id}    ${expected_status}    ${timeout}=10m
    Wait Until Keyword Succeeds    ${timeout}    30s    Project Item Status Should Equal    ${owner}    ${project_number}    ${item_id}    ${expected_status}

Project Item Status Should Equal
    [Arguments]    ${owner}    ${project_number}    ${item_id}    ${expected_status}
    ${actual}=    Get Project Item Status    ${owner}    ${project_number}    ${item_id}
    Should Be Equal    ${actual}    ${expected_status}    msg=item ${item_id} status='${actual}' (expected '${expected_status}')


*** Test Cases ***
Claude Writes Spec And Transitions To In Progress
    [Documentation]    Spec status -> claude writes spec body + moves to In Progress.
    [Tags]    agent    spec-phase
    Generate Workflow Md For Claude Spec Phase    ${WORKFLOW_DIR}    ${REPO_URL}    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}
    ...    ${PROJECT_ID}    ${STATUS_FIELD_ID}    ${IN_PROGRESS_OPT_ID}
    ...    ${ISSUE_NUMBER}    ${ISSUE_TITLE}    ${ITEM_ID}

    # Move the freshly-created Todo issue to Spec — kicks off sympheo.
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Spec    item_id=${ITEM_ID}

    Start Sympheo Daemon    ${WORKFLOW_DIR}/WORKFLOW.md    ${PORT}
    Wait For Sympheo Ready    ${PORT}    timeout=30s
    Trigger Sympheo Refresh    ${PORT}

    # Sympheo should dispatch claude on the Spec issue.
    Wait For Issue To Appear In Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=120s

    # Claude is expected to: (1) rewrite the body with a "## Spec" section,
    # (2) move the project item to "In Progress". Both can take several
    # minutes depending on Anthropic load + claude turn count.
    Wait For Issue Body To Contain    ${OWNER}    ${REPO_NAME}    ${ISSUE_NUMBER}    ${SPEC_BODY_MARKER}    timeout=10m
    Wait For Project Item Status    ${OWNER}    ${PROJECT_NUMBER}    ${ITEM_ID}    In Progress    timeout=10m

    # Once it has moved to In Progress (a terminal state for THIS workflow),
    # sympheo should drop the issue from /running.
    Trigger Sympheo Refresh    ${PORT}
    Wait For Issue To Leave Running    ${PORT}    ${ISSUE_NODE_ID}    timeout=120s
