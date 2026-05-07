# Backends for Agent Execution

This module provides pluggable backends for executing agent turns.

- `local` (default): runs the agent command directly on the host machine using a subprocess.
- `daytona`: dispatches the agent command inside a Daytona sandbox via the Daytona REST API.
