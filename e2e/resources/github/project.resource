*** Settings ***
Documentation     Provision and tear down a single ``[e2e-test]`` issue on the real
...               ``supergeoff/sympheo`` repo + Project v2. Every cleanup step is
...               gated by safety pre-checks in ``libraries/github_project.py``:
...               title prefix, node id match, project membership, branch prefix.
Library           OperatingSystem
Library           Process
Library           String
Library           Collections
Library           libraries/github_project.py
Resource          resources/common.resource


*** Keywords ***
Cleanup Stale E2E PRs
    [Documentation]    Close any open ``[e2e-test]``-prefixed PR left over from prior runs and
    ...                delete its branch. Best-effort: each close is independent.
    ${stale}=    List Stale E2E PRs    ${OWNER}    ${REPO_NAME}
    Log To Console    [pre-clean] stale e2e PRs: ${stale}
    FOR    ${pr}    IN    @{stale}
        Run Keyword And Ignore Error    Close Pr    ${OWNER}    ${REPO_NAME}    ${pr}[number]
    END

Cleanup Stale E2E Issues
    [Documentation]    Sweep any open ``[e2e-test]``-prefixed issues left over from prior runs.
    ...                Without this, sympheo would dispatch them on top of our fresh issue,
    ...                which both defeats isolation and burns through the GraphQL rate budget.
    ...                Best-effort: each delete is independently fault-tolerant.
    ${stale}=    List Stale E2E Issues    ${OWNER}    ${REPO_NAME}
    Log To Console    [pre-clean] stale e2e issues: ${stale}
    FOR    ${it}    IN    @{stale}
        Run Keyword And Ignore Error    Safe Delete Test Issue    ${OWNER}    ${REPO_NAME}    ${it}[number]    ${it}[node_id]
    END

Provision Test Issue
    [Documentation]    Creates one ``[e2e-test]``-prefixed issue and adds it to the project.
    ...                Captures the suite-scoped state needed for cleanup.
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

Cleanup Test Issue
    [Documentation]    Removes the project item, deletes the test issue, and
    ...                deletes any ``sympheo/*`` branch pushed during the run.
    ...                Each step is independently fault-tolerant.
    ${have_item}=     Get Variable Value    ${ITEM_ID}    ${EMPTY}
    ${have_issue}=    Get Variable Value    ${ISSUE_NODE_ID}    ${EMPTY}
    ${have_number}=   Get Variable Value    ${ISSUE_NUMBER}    ${EMPTY}

    IF    '${have_item}'!='${EMPTY}'
        Run Keyword And Ignore Error
        ...    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Canceled    item_id=${ITEM_ID}
    END

    IF    '${have_item}'!='${EMPTY}'
        Run Keyword And Ignore Error    Safe Remove Project Item
        ...    ${OWNER}    ${PROJECT_NUMBER}    ${PROJECT_ID}    ${ITEM_ID}    ${ISSUE_URL}
    END

    IF    '${have_number}'!='${EMPTY}'
        Run Keyword And Ignore Error    Cleanup Sympheo Branches For Issue
        ...    ${OWNER}    ${REPO_NAME}    ${ISSUE_NUMBER}
    END

    IF    '${have_issue}'!='${EMPTY}' and '${have_number}'!='${EMPTY}'
        Run Keyword And Ignore Error    Safe Delete Test Issue
        ...    ${OWNER}    ${REPO_NAME}    ${ISSUE_NUMBER}    ${ISSUE_NODE_ID}
    END

Safe Remove Project Item
    [Arguments]    ${owner}    ${project_number}    ${project_id}    ${item_id}    ${issue_url}
    Assert Safe To Remove Project Item    ${owner}    ${project_number}    ${item_id}    ${issue_url}
    Remove Project Item    ${owner}    ${project_number}    ${project_id}    ${item_id}

Safe Delete Test Issue
    [Arguments]    ${owner}    ${repo_name}    ${issue_number}    ${issue_node_id}
    Assert Safe To Delete Issue    ${owner}    ${repo_name}    ${issue_number}    ${issue_node_id}
    Delete Issue Via Graphql    ${issue_node_id}

Cleanup Sympheo Branches For Issue
    [Documentation]    Find any ``sympheo/*`` branch the orchestrator pushed during this run and delete it.
    ...                Branches captured in ``${PRE_RUN_SYMPHEO_BRANCHES}`` at setup are NEVER touched.
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
