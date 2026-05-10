*** Settings ***
Documentation     Polling assertions against ``/api/v1/state`` and post-run filesystem checks.
...               Uses ``curl`` via the Process library instead of ``RequestsLibrary`` so the
...               harness needs zero extra Python packages.
Library           OperatingSystem
Library           Collections
Library           Process
Library           String


*** Keywords ***
Get Sympheo State
    [Arguments]    ${port}=18080
    ${result}=    Run Process    curl    -fsS    -m    5    http://127.0.0.1:${port}/api/v1/state
    Should Be Equal As Integers    ${result.rc}    0    msg=curl failed: ${result.stderr}
    ${parsed}=    Evaluate    __import__('json').loads(r"""${result.stdout}""")    modules=json
    RETURN    ${parsed}

Issue Is In Running
    [Arguments]    ${port}    ${issue_id}
    ${state}=    Get Sympheo State    ${port}
    ${ids}=    Evaluate    [r["issue_id"] for r in $state.get("running", [])]
    Should Contain    ${ids}    ${issue_id}    msg=issue ${issue_id} not in running map; running=${ids}

Issue Is Not In Running
    [Arguments]    ${port}    ${issue_id}
    ${state}=    Get Sympheo State    ${port}
    ${ids}=    Evaluate    [r["issue_id"] for r in $state.get("running", [])]
    Should Not Contain    ${ids}    ${issue_id}    msg=issue ${issue_id} still in running map; running=${ids}

Wait For Issue To Appear In Running
    [Arguments]    ${port}    ${issue_id}    ${timeout}=30s
    Wait Until Keyword Succeeds    ${timeout}    2s    Issue Is In Running    ${port}    ${issue_id}

Wait For Issue To Leave Running
    [Arguments]    ${port}    ${issue_id}    ${timeout}=60s
    Wait Until Keyword Succeeds    ${timeout}    2s    Issue Is Not In Running    ${port}    ${issue_id}

Dump Last Sympheo State
    [Documentation]    Best-effort dump of ``/api/v1/state`` to the Robot log; useful for failure forensics.
    [Arguments]    ${port}=18080
    ${ok}    ${state}=    Run Keyword And Ignore Error    Get Sympheo State    ${port}
    Run Keyword If    '${ok}'=='PASS'    Log    Last sympheo state: ${state}

Trigger Sympheo Refresh
    [Arguments]    ${port}=18080
    ${result}=    Run Process    curl    -fsS    -m    5    -X    POST    http://127.0.0.1:${port}/api/v1/refresh
    Should Be Equal As Integers    ${result.rc}    0    msg=curl POST refresh failed: ${result.stderr}
    RETURN    ${result.stdout}

Assert Workspace Cleaned
    [Arguments]    ${workspace_root}
    [Documentation]    After a Done transition the per-issue workspace dir should be removed.
    ${entries}=    List Directory    ${workspace_root}
    ${remaining}=    Evaluate    [e for e in $entries if e.startswith('SYMPHEO-')]
    Should Be Empty    ${remaining}    msg=expected no SYMPHEO-* dirs in ${workspace_root}, found ${remaining}
