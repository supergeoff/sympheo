*** Settings ***
Documentation     Provision and tear down GitHub Project v2 + repo + issue resources for sympheo e2e.
Library           OperatingSystem
Library           Process
Library           String
Library           Collections
Library           ../libraries/github_project.py


*** Keywords ***
Provision GitHub E2E Resources
    [Documentation]    Creates throwaway repo + Project v2 with statuses, adds an issue, populates suite vars.
    ${SHORT_UUID}=    Evaluate    __import__('uuid').uuid4().hex[:8]    modules=uuid
    ${TS}=            Evaluate    int(__import__('time').time())    modules=time
    ${OWNER}=         Get GH User
    ${REPO_NAME}=     Set Variable    sympheo-e2e-${SHORT_UUID}-${TS}
    Set Suite Variable    ${OWNER}
    Set Suite Variable    ${REPO_NAME}
    ${REPO_URL}=      Create Throwaway Repo    ${OWNER}    ${REPO_NAME}
    Set Suite Variable    ${REPO_URL}
    ${PROJECT_NUMBER}=    Create Project With Statuses    ${OWNER}    sympheo-e2e-${SHORT_UUID}
    Set Suite Variable    ${PROJECT_NUMBER}
    ${issue_meta}=    Add Test Issue
    ...    ${OWNER}    ${REPO_NAME}    ${PROJECT_NUMBER}
    ...    e2e: create hello.txt    Make a hello.txt with body "world"
    ${ISSUE_NODE_ID}=    Set Variable    ${issue_meta}[node_id]
    ${ISSUE_URL}=        Set Variable    ${issue_meta}[url]
    Set Suite Variable    ${ISSUE_NODE_ID}
    Set Suite Variable    ${ISSUE_URL}
    Log To Console    [provision] owner=${OWNER} repo=${REPO_NAME} project=${PROJECT_NUMBER} issue_node=${ISSUE_NODE_ID} url=${ISSUE_URL}

Teardown GitHub E2E Resources
    [Documentation]    Deletes the throwaway project and repo. Tolerant of partial state.
    ${proj_set}=    Get Variable Value    ${PROJECT_NUMBER}    ${EMPTY}
    ${repo_set}=    Get Variable Value    ${REPO_NAME}    ${EMPTY}
    ${owner_set}=    Get Variable Value    ${OWNER}    ${EMPTY}
    Run Keyword If    '${proj_set}'!='${EMPTY}' and '${owner_set}'!='${EMPTY}'
    ...    Run Keyword And Ignore Error    Delete Project    ${OWNER}    ${PROJECT_NUMBER}
    Run Keyword If    '${repo_set}'!='${EMPTY}' and '${owner_set}'!='${EMPTY}'
    ...    Run Keyword And Ignore Error    Delete Repo    ${OWNER}    ${REPO_NAME}
