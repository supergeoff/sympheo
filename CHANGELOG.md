# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Local backend stderr capture** ([#SYMPHEO-108](https://github.com/supergeoff/sympheo/issues/108))
  - The local backend now reads the opencode agent's stderr stream line-by-line.
  - Each non-empty line is logged at `WARN` level with the target `opencode::stderr`.
  - Log entries include the `issue_id` field so operators can correlate diagnostic output with the ticket it belongs to.
  - The stderr reader task is automatically cleaned up when the agent turn completes or times out, preventing stream leaks.

## [0.1.0] - 2024-01-15

### Added

- Initial release of Sympheo — autonomous orchestrator for GitHub project boards.
- Polling-based tracker for GitHub Projects.
- Local and Daytona execution backends.
- Stage-specific agent skills (Architect, Tech Lead, Code Reviewer, Test Expert, Doc Expert).
- Built-in HTTP dashboard and REST API.
- Workspace lifecycle management with hooks.
- Retry queue with exponential backoff.

[Unreleased]: https://github.com/supergeoff/sympheo/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/supergeoff/sympheo/releases/tag/v0.1.0
