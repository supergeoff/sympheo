# sympheo

A Rust-based orchestrator for running OpenCode agents in parallel on Daytona sandboxes.

## Overview

Sympheo dispatches coding agents to handle GitHub issues. It supports:
- **Local execution**: Run agents directly on the host machine
- **Daytona execution**: Run agents in isolated Daytona sandboxes  
- **Hybrid mode**: Mix local and Daytona execution based on labels or round-robin

## Quick Start

1. Clone the repository
2. Create a `WORKFLOW.md` with your configuration
3. Run `cargo run`

## Configuration

See `SPEC.md` for the full configuration schema.

## Testing

```bash
cargo test
```
