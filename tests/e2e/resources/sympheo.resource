*** Settings ***
Documentation     Spawn / kill / probe the sympheo daemon. Uses ``curl`` via the Process library
...               instead of ``RequestsLibrary`` so the harness only needs the global
...               ``robot`` install and zero extra Python packages.
Library           OperatingSystem
Library           Process


*** Keywords ***
Set Up Workflow Dir
    [Documentation]    Create a temp dir for WORKFLOW.md, workspace, and logs. Stored as suite vars.
    ${tmp}=    Evaluate    __import__('tempfile').mkdtemp(prefix='sympheo-e2e-')    modules=tempfile
    Set Suite Variable    ${WORKFLOW_DIR}    ${tmp}
    Create Directory    ${tmp}/workspace
    Create Directory    ${tmp}/logs
    Set Suite Variable    ${SYMPHEO_LOG}    ${tmp}/logs
    Log To Console    [setup] workflow_dir=${tmp}

Teardown Workflow Dir
    [Documentation]    Best-effort removal of the temp workflow dir. Always copies logs to ``${OUTPUT_DIR}`` first.
    ${dir}=    Get Variable Value    ${WORKFLOW_DIR}    ${EMPTY}
    Run Keyword If    '${dir}'!='${EMPTY}'    Copy Logs To Report Dir    ${dir}
    Run Keyword If    '${dir}'!='${EMPTY}'    Run Keyword And Ignore Error    Remove Directory    ${dir}    recursive=True

Copy Logs To Report Dir
    [Arguments]    ${dir}
    ${log_src}=    Set Variable    ${dir}/logs
    ${exists}=    Run Keyword And Return Status    Directory Should Exist    ${log_src}
    Run Keyword If    ${exists}    Copy Directory    ${log_src}    ${OUTPUT_DIR}/sympheo-logs
    ${wf}=    Set Variable    ${dir}/WORKFLOW.md
    ${wf_exists}=    Run Keyword And Return Status    File Should Exist    ${wf}
    Run Keyword If    ${wf_exists}    Copy File    ${wf}    ${OUTPUT_DIR}/WORKFLOW.md
    ${sc}=    Set Variable    ${dir}/script.yaml
    ${sc_exists}=    Run Keyword And Return Status    File Should Exist    ${sc}
    Run Keyword If    ${sc_exists}    Copy File    ${sc}    ${OUTPUT_DIR}/script.yaml

Start Sympheo Daemon
    [Arguments]    ${workflow_path}    ${port}=18080
    [Documentation]    Kills any stale sympheo bound to ``${port}`` (a previous failed run that didn't get torn down) before starting fresh.
    Run Keyword And Ignore Error    Kill Stale Sympheo On Port    ${port}
    ${stdout}=    Set Variable    ${SYMPHEO_LOG}/stdout.log
    ${stderr}=    Set Variable    ${SYMPHEO_LOG}/stderr.log
    ${result}=    Start Process    ${SYMPHEO_BIN}    ${workflow_path}    --port    ${port}
    ...    stdout=${stdout}    stderr=${stderr}
    ...    env:RUST_LOG=info,sympheo=debug
    Set Suite Variable    ${SYMPHEO_HANDLE}    ${result}
    Log To Console    [sympheo] started pid=${result.pid} log=${stdout}

Kill Stale Sympheo On Port
    [Arguments]    ${port}
    ${cmd}=    Set Variable    pids=$(lsof -ti :${port} 2>/dev/null || true); if [ -n "$pids" ]; then echo "killing stale: $pids" >&2; kill -9 $pids; sleep 1; fi
    Run Process    bash    -lc    ${cmd}

Wait For Sympheo Ready
    [Arguments]    ${port}=18080    ${timeout}=30s
    Wait Until Keyword Succeeds    ${timeout}    1s    Sympheo State Endpoint Responds    ${port}

Sympheo State Endpoint Responds
    [Arguments]    ${port}
    ${result}=    Run Process    curl    -fsS    -m    5    http://127.0.0.1:${port}/api/v1/state
    Should Be Equal As Integers    ${result.rc}    0    msg=curl failed: ${result.stderr}
    RETURN    ${result.stdout}

Stop Sympheo Daemon
    ${handle}=    Get Variable Value    ${SYMPHEO_HANDLE}    ${None}
    Run Keyword If    $handle is not None    Run Keyword And Ignore Error    Terminate Process    ${SYMPHEO_HANDLE}    kill=True
