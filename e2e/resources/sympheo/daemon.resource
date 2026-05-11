*** Settings ***
Documentation     Spawn / probe / kill the sympheo daemon. Uses ``curl`` via ``Process``
...               so the harness needs zero extra Python packages.
Library           OperatingSystem
Library           Process
Resource          resources/common.resource


*** Keywords ***
Assert Sympheo Binary Exists
    [Documentation]    Fail fast if the release binary is missing.
    ${exists}=    Run Keyword And Return Status    File Should Exist    ${SYMPHEO_BIN}
    IF    not ${exists}
        Fail    sympheo binary not found at ${SYMPHEO_BIN}; run `cargo build --release` first
    END
    Log To Console    [setup] sympheo binary at ${SYMPHEO_BIN}

Ensure Github Token In Env
    [Documentation]    Sympheo's tracker is an in-process reqwest client; it reads ``GITHUB_TOKEN``
    ...                from its own env, independently of the gh CLI. If the host has no valid
    ...                ``GITHUB_TOKEN`` exported (or it's stale), sympheo's startup fails with
    ...                ``missing tracker api key``. Always source the token from ``gh auth token``
    ...                so sympheo + gh agree on the same identity. The exported value is
    ...                suite-scoped; Robot does NOT log it.
    ${result}=    Run Process    gh    auth    token    stderr=STDOUT
    Should Be Equal As Integers    ${result.rc}    0    msg=gh auth token failed: ${result.stdout}
    ${tok}=    Strip String    ${result.stdout}
    Should Not Be Empty    ${tok}    msg=gh auth token returned empty
    Set Environment Variable    GITHUB_TOKEN    ${tok}
    ${len}=    Get Length    ${tok}
    Log To Console    [setup] GITHUB_TOKEN exported from `gh auth token` (length=${len})

Stage Claude OAuth Home
    [Documentation]    Build a minimal HOME-shaped directory under ``${WORKFLOW_DIR}/claude-home/``
    ...                containing only ``.claude/.credentials.json`` from the host. The WORKFLOW.md
    ...                generator then sets ``cli.env.HOME`` to this directory so claude — launched
    ...                inside sympheo's isolated workspace — can authenticate via OAuth without
    ...                exposing the rest of ``~/`` to ``--dangerously-skip-permissions``.
    File Should Exist    ${HOST_CLAUDE_CREDENTIALS}    msg=run `claude /login` once or export ANTHROPIC_API_KEY
    ${claude_home}=    Set Variable    ${WORKFLOW_DIR}/claude-home
    Create Directory    ${claude_home}/.claude
    Copy File    ${HOST_CLAUDE_CREDENTIALS}    ${claude_home}/.claude/.credentials.json
    Set Suite Variable    ${CLAUDE_HOME_DIR}    ${claude_home}
    Log To Console    [setup] staged claude OAuth home at ${claude_home}

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
    Run Keyword If    ${exists}    Run Keyword And Ignore Error    Remove Directory    ${OUTPUT_DIR}/sympheo-logs    recursive=True
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
