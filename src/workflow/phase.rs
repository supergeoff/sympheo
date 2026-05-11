use crate::agent::cli::CliOptions;
use crate::config::resolver;
use crate::error::SympheoError;

// PRD-v2 §5.2.1 — entry of the `phases[]` block declared in WORKFLOW.md
// front matter. Each phase maps a tracker state to a prompt fragment
// (interpolated as `{{ phase.prompt }}` into the global template),
// post-turn verifications, and a per-phase `cli.options` override that
// is shallow-merged over the global `cli.options` at dispatch time.
#[derive(Debug, Clone, Default)]
pub struct Phase {
    pub name: String,
    pub state: String,
    pub prompt: String,
    pub verifications: Vec<String>,
    pub cli_options: CliOptions,
}

// PRD-v2 §5.2/§5.3 — owns the parsed `phases[]` block and the lookup
// + validation logic. Lives in `workflow::phase` so config and
// orchestrator depend on workflow for workflow data, not on tracker.
#[derive(Debug, Clone, Default)]
pub struct WorkflowSpec {
    phases: Vec<Phase>,
}

impl WorkflowSpec {
    // PRD-v2 §5.2 — parse the `phases[]` block from raw config front
    // matter. Returns an empty spec when the block is absent so callers
    // fall back to the global prompt template unchanged.
    //
    // SPEC §10.6: the per-phase override now lives under `phases[].cli.options`
    // (mirroring the global `cli.options` path). The legacy
    // `phases[].cli_options` shape is rejected with an explicit error.
    pub fn from_raw(
        raw: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Self, SympheoError> {
        let arr = match raw.get("phases").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return Ok(Self::default()),
        };
        let mut phases = Vec::with_capacity(arr.len());
        for v in arr {
            let Some(m) = v.as_object() else { continue };
            if m.contains_key("cli_options") {
                return Err(SympheoError::InvalidConfiguration(
                    "phases[].cli_options is renamed to phases[].cli.options (nested map mirroring the global cli.options)"
                        .into(),
                ));
            }
            let cli_options = match m.get("cli").and_then(|v| v.as_object()) {
                Some(cli_map) => {
                    let raw_options = cli_map
                        .get("options")
                        .cloned()
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                    CliOptions::parse(&raw_options)?
                }
                None => CliOptions::default(),
            };
            phases.push(Phase {
                name: resolver::get_string(m, "name").unwrap_or_default(),
                state: resolver::get_string(m, "state").unwrap_or_default(),
                prompt: resolver::get_string(m, "prompt").unwrap_or_default(),
                verifications: resolver::get_str_list(m, "verifications")
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect(),
                cli_options,
            });
        }
        Ok(Self { phases })
    }

    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    pub fn is_empty(&self) -> bool {
        self.phases.is_empty()
    }

    // PRD-v2 §5.3 — validation of the `phases[]` block:
    //   * required fields (name, state, prompt) present and non-empty
    //   * each phase.state belongs to active_states (case-insensitive)
    //   * no two phases declare the same state (case-insensitive)
    // active_states without a matching phase are NOT errors per §5.3
    // (warn at boot only); that warning is emitted by the orchestrator.
    pub fn validate(&self, active_states: &[String]) -> Result<(), SympheoError> {
        if self.phases.is_empty() {
            return Ok(());
        }
        let active: std::collections::HashSet<String> =
            active_states.iter().map(|s| s.to_lowercase()).collect();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for p in &self.phases {
            if p.name.trim().is_empty() {
                return Err(SympheoError::WorkflowPhaseMissingField("name".into()));
            }
            if p.state.trim().is_empty() {
                return Err(SympheoError::WorkflowPhaseMissingField("state".into()));
            }
            if p.prompt.trim().is_empty() {
                return Err(SympheoError::WorkflowPhaseMissingField("prompt".into()));
            }
            let state_lc = p.state.to_lowercase();
            if !active.contains(&state_lc) {
                return Err(SympheoError::WorkflowPhaseUnknownState(p.state.clone()));
            }
            if !seen.insert(state_lc) {
                return Err(SympheoError::WorkflowPhaseDuplicateState(p.state.clone()));
            }
        }
        Ok(())
    }

    // Look up the Phase whose state matches `issue_state` case-insensitively.
    pub fn phase_for_state(&self, issue_state: &str) -> Option<&Phase> {
        let target = issue_state.to_lowercase();
        self.phases
            .iter()
            .find(|p| p.state.to_lowercase() == target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::cli::Permission;

    fn raw_with_phases(phases: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        let mut raw = serde_json::Map::new();
        raw.insert("phases".into(), phases);
        raw
    }

    #[test]
    fn from_raw_empty_when_block_absent() {
        let spec = WorkflowSpec::from_raw(&serde_json::Map::new()).unwrap();
        assert!(spec.is_empty());
    }

    #[test]
    fn from_raw_parses_full_entry() {
        let raw = raw_with_phases(serde_json::json!([
            {
                "name": "build",
                "state": "In Progress",
                "prompt": "Implement the LLD",
                "verifications": ["cargo fmt --all -- --check", "cargo test"],
                "cli": {
                    "options": {
                        "model": "sonnet",
                        "permission": "acceptEdits",
                        "additional_args": ["--verbose"]
                    }
                }
            }
        ]));
        let spec = WorkflowSpec::from_raw(&raw).unwrap();
        let phases = spec.phases();
        assert_eq!(phases.len(), 1);
        let p = &phases[0];
        assert_eq!(p.name, "build");
        assert_eq!(p.state, "In Progress");
        assert_eq!(p.prompt, "Implement the LLD");
        assert_eq!(p.verifications.len(), 2);
        assert_eq!(p.cli_options.model.as_deref(), Some("sonnet"));
        assert_eq!(p.cli_options.permission, Some(Permission::AcceptEdits));
        assert_eq!(p.cli_options.additional_args, vec!["--verbose"]);
    }

    #[test]
    fn from_raw_phase_without_cli_block_defaults_to_empty_options() {
        let raw = raw_with_phases(serde_json::json!([
            { "name": "spec", "state": "Spec", "prompt": "go" }
        ]));
        let spec = WorkflowSpec::from_raw(&raw).unwrap();
        let p = &spec.phases()[0];
        assert!(p.cli_options.model.is_none());
        assert!(p.cli_options.permission.is_none());
        assert!(p.cli_options.additional_args.is_empty());
    }

    #[test]
    fn from_raw_rejects_legacy_cli_options_key() {
        let raw = raw_with_phases(serde_json::json!([
            {
                "name": "x", "state": "Spec", "prompt": "p",
                "cli_options": { "model": "sonnet" }
            }
        ]));
        let err = WorkflowSpec::from_raw(&raw).unwrap_err();
        assert!(
            matches!(err, SympheoError::InvalidConfiguration(s) if s.contains("cli_options") && s.contains("cli.options"))
        );
    }

    #[test]
    fn from_raw_propagates_legacy_permission_mode_error() {
        let raw = raw_with_phases(serde_json::json!([
            {
                "name": "x", "state": "Spec", "prompt": "p",
                "cli": { "options": { "permission_mode": "plan" } }
            }
        ]));
        let err = WorkflowSpec::from_raw(&raw).unwrap_err();
        assert!(
            matches!(err, SympheoError::InvalidConfiguration(s) if s.contains("permission_mode"))
        );
    }

    #[test]
    fn from_raw_drops_empty_string_verifications() {
        let raw = raw_with_phases(serde_json::json!([
            {
                "name": "spec",
                "state": "Spec",
                "prompt": "p",
                "verifications": ["cargo check", "  ", ""]
            }
        ]));
        let spec = WorkflowSpec::from_raw(&raw).unwrap();
        assert_eq!(
            spec.phases()[0].verifications,
            vec!["cargo check".to_string()]
        );
    }

    #[test]
    fn validate_ok_when_absent() {
        let spec = WorkflowSpec::default();
        assert!(spec.validate(&["spec".into()]).is_ok());
    }

    #[test]
    fn validate_ok_full() {
        let raw = raw_with_phases(serde_json::json!([
            { "name": "spec", "state": "Spec", "prompt": "go" },
            { "name": "build", "state": "In Progress", "prompt": "go" }
        ]));
        let spec = WorkflowSpec::from_raw(&raw).unwrap();
        let active = vec!["spec".into(), "in progress".into()];
        assert!(spec.validate(&active).is_ok());
    }

    #[test]
    fn validate_unknown_state_errors() {
        let raw = raw_with_phases(serde_json::json!([
            { "name": "x", "state": "NotInActive", "prompt": "p" }
        ]));
        let spec = WorkflowSpec::from_raw(&raw).unwrap();
        assert!(matches!(
            spec.validate(&["spec".into()]),
            Err(SympheoError::WorkflowPhaseUnknownState(_))
        ));
    }

    #[test]
    fn validate_duplicate_state_errors() {
        let raw = raw_with_phases(serde_json::json!([
            { "name": "a", "state": "Spec", "prompt": "p" },
            { "name": "b", "state": "spec", "prompt": "p" }
        ]));
        let spec = WorkflowSpec::from_raw(&raw).unwrap();
        assert!(matches!(
            spec.validate(&["spec".into()]),
            Err(SympheoError::WorkflowPhaseDuplicateState(_))
        ));
    }

    #[test]
    fn validate_missing_name_errors() {
        let raw = raw_with_phases(serde_json::json!([
            { "name": "", "state": "Spec", "prompt": "p" }
        ]));
        let spec = WorkflowSpec::from_raw(&raw).unwrap();
        assert!(matches!(
            spec.validate(&["spec".into()]),
            Err(SympheoError::WorkflowPhaseMissingField(_))
        ));
    }

    #[test]
    fn validate_missing_prompt_errors() {
        let raw = raw_with_phases(serde_json::json!([
            { "name": "spec", "state": "Spec", "prompt": "" }
        ]));
        let spec = WorkflowSpec::from_raw(&raw).unwrap();
        assert!(matches!(
            spec.validate(&["spec".into()]),
            Err(SympheoError::WorkflowPhaseMissingField(_))
        ));
    }

    #[test]
    fn phase_for_state_case_insensitive() {
        let raw = raw_with_phases(serde_json::json!([
            { "name": "build", "state": "In Progress", "prompt": "p" }
        ]));
        let spec = WorkflowSpec::from_raw(&raw).unwrap();
        let p = spec.phase_for_state("in progress").unwrap();
        assert_eq!(p.name, "build");
        assert!(spec.phase_for_state("Done").is_none());
    }
}
