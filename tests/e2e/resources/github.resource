*** Settings ***
Documentation     Setup and teardown for the e2e harness against the REAL ``supergeoff/sympheo`` repo and Project v2 #2.
...               This NEVER provisions throwaway resources. It creates one issue (prefixed
...               with ``[e2e-test]``) on the existing repo, drives it, then deletes the
...               issue + project item + any remote ``sympheo/*`` branch the orchestrator
...               pushed. Every cleanup step is wrapped in a safety pre-check so a typo'd
...               variable cannot damage real user data.
Library           OperatingSystem
Library           Process
Library           String
Library           Collections
Library           ../libraries/github_project.py


*** Variables ***
${OWNER}              supergeoff
${REPO_NAME}          sympheo
${PROJECT_NUMBER}     2
${REPO_URL}           https://github.com/supergeoff/sympheo.git


*** Keywords ***
Provision GitHub E2E Resources
    [Documentation]    Creates one ``[e2e-test]``-prefixed issue on the real repo and adds it to project ${PROJECT_NUMBER}.
    Set Suite Variable    ${OWNER}
    Set Suite Variable    ${REPO_NAME}
    Set Suite Variable    ${PROJECT_NUMBER}
    Set Suite Variable    ${REPO_URL}
    ${pre_branches}=    List Sympheo Branches    ${OWNER}    ${REPO_NAME}
    Set Suite Variable    ${PRE_RUN_SYMPHEO_BRANCHES}    ${pre_branches}
    Log To Console    [provision] pre-run sympheo branches=${pre_branches}
    ${meta}=    Create Test Issue    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}
    Set Suite Variable    ${ISSUE_NODE_ID}    ${meta}[node_id]
    Set Suite Variable    ${ISSUE_URL}        ${meta}[url]
    Set Suite Variable    ${ISSUE_NUMBER}     ${meta}[number]
    Set Suite Variable    ${ISSUE_TITLE}      ${meta}[title]
    Set Suite Variable    ${PROJECT_ID}       ${meta}[project_id]
    Set Suite Variable    ${ITEM_ID}          ${meta}[item_id]
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Todo    item_id=${ITEM_ID}
    Log To Console    [provision] issue=#${ISSUE_NUMBER} node=${ISSUE_NODE_ID} item=${ITEM_ID}

Teardown GitHub E2E Resources
    [Documentation]    Removes the project item and deletes the test issue and any
    ...                ``sympheo/*`` branch pushed during the run. Each step is independently
    ...                fault-tolerant. NEVER touches a branch that did not start with ``sympheo/``
    ...                or an issue whose title is no longer prefixed with ``[e2e-test]``.
    ${have_item}=     Get Variable Value    ${ITEM_ID}    ${EMPTY}
    ${have_issue}=    Get Variable Value    ${ISSUE_NODE_ID}    ${EMPTY}
    ${have_number}=   Get Variable Value    ${ISSUE_NUMBER}    ${EMPTY}

    # Defensive: move the item to a terminal state so the orchestrator stops
    # observing it if the test failed mid-flight.
    Run Keyword If    '${have_item}'!='${EMPTY}'
    ...    Run Keyword And Ignore Error
    ...    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Canceled    item_id=${ITEM_ID}

    # Remove project item (with safety check that it still maps to our URL).
    Run Keyword If    '${have_item}'!='${EMPTY}'
    ...    Run Keyword And Ignore Error
    ...    Safe Remove Project Item    ${OWNER}    ${PROJECT_NUMBER}    ${PROJECT_ID}    ${ITEM_ID}    ${ISSUE_URL}

    # Best-effort: delete any remote sympheo/* branch that was pushed for THIS issue.
    Run Keyword If    '${have_number}'!='${EMPTY}'
    ...    Run Keyword And Ignore Error
    ...    Cleanup Sympheo Branches For Issue    ${OWNER}    ${REPO_NAME}    ${ISSUE_NUMBER}

    # Delete the test issue itself, gated on title-prefix + node-id checks.
    Run Keyword If    '${have_issue}'!='${EMPTY}' and '${have_number}'!='${EMPTY}'
    ...    Run Keyword And Ignore Error
    ...    Safe Delete Test Issue    ${OWNER}    ${REPO_NAME}    ${ISSUE_NUMBER}    ${ISSUE_NODE_ID}

Safe Remove Project Item
    [Arguments]    ${owner}    ${project_number}    ${project_id}    ${item_id}    ${issue_url}
    Assert Safe To Remove Project Item    ${owner}    ${project_number}    ${item_id}    ${issue_url}
    Remove Project Item    ${owner}    ${project_number}    ${project_id}    ${item_id}

Safe Delete Test Issue
    [Arguments]    ${owner}    ${repo_name}    ${issue_number}    ${issue_node_id}
    Assert Safe To Delete Issue    ${owner}    ${repo_name}    ${issue_number}    ${issue_node_id}
    Delete Issue Via Graphql    ${issue_node_id}

Cleanup Sympheo Branches For Issue
    [Documentation]    Find any ``sympheo/*`` branch the orchestrator created during this run and delete it.
    ...                Branches that pre-existed (captured in ``${PRE_RUN_SYMPHEO_BRANCHES}``) are NEVER touched.
    [Arguments]    ${owner}    ${repo_name}    ${issue_number}
    ${current}=    List Sympheo Branches    ${owner}    ${repo_name}
    ${pre}=        Get Variable Value    ${PRE_RUN_SYMPHEO_BRANCHES}    ${EMPTY}
    ${pre_set}=    Run Keyword If    '${pre}'=='${EMPTY}'    Create List
    ...    ELSE    Set Variable    ${pre}
    ${created}=    Evaluate    [b for b in $current if b not in $pre_set]
    Log To Console    [teardown] sympheo/* branches created during run: ${created}
    FOR    ${b}    IN    @{created}
        Run Keyword And Ignore Error    Delete Remote Branch If Safe    ${owner}    ${repo_name}    ${b}
    END
