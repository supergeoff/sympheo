*** Settings ***
Documentation     Sympheo CLI surface checks. Verifies the binary's argument-handling
...               contract: ``--help`` succeeds, missing or invalid workflow paths
...               exit non-zero, and the printed error mentions the offending file.
...               No daemon is started here.
Library           Process
Library           String
Resource          resources/common.resource


*** Test Cases ***
Help Flag Prints Usage
    [Tags]    cli    smoke
    ${result}=    Run Process    ${SYMPHEO_BIN}    --help
    Should Be Equal As Integers    ${result.rc}    0    msg=--help should exit 0; stderr=${result.stderr}
    Should Contain    ${result.stdout}    Usage:
    Should Contain    ${result.stdout}    WORKFLOW_PATH
    Should Contain    ${result.stdout}    --port

Missing Workflow Path Exits Non-Zero
    [Documentation]    Without an arg sympheo defaults to ``WORKFLOW.md`` in cwd; absent it must fail.
    [Tags]    cli    error
    ${result}=    Run Process    ${SYMPHEO_BIN}    cwd=${EXECDIR}    stderr=STDOUT
    Should Not Be Equal As Integers    ${result.rc}    0    msg=expected non-zero rc when WORKFLOW.md is missing
    Should Contain    ${result.stdout}    MissingWorkflowFile

Invalid Workflow Path Reports It
    [Tags]    cli    error
    ${result}=    Run Process    ${SYMPHEO_BIN}    /nonexistent/WORKFLOW.md    stderr=STDOUT
    Should Not Be Equal As Integers    ${result.rc}    0
    Should Contain    ${result.stdout}    /nonexistent/WORKFLOW.md
    Should Contain    ${result.stdout}    No such file
