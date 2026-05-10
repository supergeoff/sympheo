*** Settings ***
Documentation     GitHub plumbing test: verifies the harness can create a test issue,
...               drive it through Project v2 statuses, and delete it cleanly. No
...               sympheo daemon is involved — this isolates failures in the gh
...               wrappers from failures in the orchestrator.
...
...               If this suite fails, every other suite that touches GitHub will
...               also fail. Run it first.
Library           Collections
Library           libraries/github_project.py
Resource          resources/common.resource
Resource          resources/github/project.resource

Suite Setup       Provision Test Issue
Suite Teardown    Cleanup Test Issue


*** Test Cases ***
Issue Is Created And Reachable
    [Documentation]    The created issue surfaces on the API and carries the expected metadata.
    [Tags]    github    setup
    Should Not Be Empty    ${ISSUE_NODE_ID}
    Should Not Be Empty    ${ISSUE_URL}
    Should Match Regexp    ${ISSUE_TITLE}    ^\\[e2e-test\\]
    Should Be True    ${ISSUE_NUMBER} > 0

Issue Item Walks Project Statuses
    [Documentation]    Move the project item across the lifecycle states; each transition succeeds.
    [Tags]    github    project
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    In Progress    item_id=${ITEM_ID}
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Done           item_id=${ITEM_ID}
    Move Project Item To Status    ${OWNER}    ${PROJECT_NUMBER}    ${ISSUE_URL}    Canceled       item_id=${ITEM_ID}

Pre-run Branch Snapshot Is Captured
    [Documentation]    The harness captures sympheo/* branches at setup so cleanup can avoid pre-existing ones.
    [Tags]    github    safety
    ${pre}=    Get Variable Value    ${PRE_RUN_SYMPHEO_BRANCHES}    ${EMPTY}
    Should Not Be Equal    '${pre}'    '${EMPTY}'    msg=PRE_RUN_SYMPHEO_BRANCHES never populated
