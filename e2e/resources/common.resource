*** Settings ***
Documentation     Shared variables for the e2e harness.
...               Imported by every test suite. Anything project-specific
...               (owner, repo, project number) lives here so tests stay
...               readable and overrides are a single ``--variable`` away.


*** Variables ***
# GitHub target — the real ``supergeoff/sympheo`` repo + Project v2 #2.
${OWNER}             supergeoff
${REPO_NAME}         sympheo
${PROJECT_NUMBER}    2
${REPO_URL}          https://github.com/supergeoff/sympheo.git

# sympheo runtime knobs.
${PORT}              18080
${MODE}              mock
${KEEP_ON_FAILURE}   0

# Binary path — relative to the e2e/ root (where ``robot -A robot.args`` runs).
# ``${EXECDIR}`` is the directory robot was launched from.
${SYMPHEO_BIN}       ${EXECDIR}/../target/release/sympheo

# Source of the OAuth credentials file copied into a staged HOME so claude
# can authenticate inside the sympheo-isolated workspace. Override only if
# the install stores them elsewhere.
${HOST_CLAUDE_CREDENTIALS}    %{HOME}/.claude/.credentials.json

# Source dirs for opencode auth/config — same staging idea as claude. Both
# are copied verbatim into a per-suite fake HOME so opencode resolves
# ``~/.local/share/opencode`` and ``~/.config/opencode`` against that HOME.
${HOST_OPENCODE_DATA}         %{HOME}/.local/share/opencode
${HOST_OPENCODE_CONFIG}       %{HOME}/.config/opencode
