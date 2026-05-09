# Sympheo — Audit code vs SPEC.md (Draft v1)

Read-only audit produit en P0. Compare l'implémentation Rust de `/home/supergeoff/projects/sympheo` à la spécification `SPEC.md` (Draft v1, RFC 2119 normatif).

**Méthode** : pour chaque sous-point normatif (MUST/SHOULD/REQUIRED), localiser le code (file:line) ou noter `absent`. Statut ∈ `{conforme, partiel, absent, extension}`. Écart en 1 ligne factuelle. Aucune interprétation, aucune proposition de fix — la phase P1 traitera les corrections.

**Entrées** : `SPEC.md` 2488 lignes, code source Rust (~10 000 LoC), `WORKFLOW.md`, `opencode.json`, tests (`tests/` + tests embarqués).

**Convention de nommage du tableau** : `partiel` = présent mais ne couvre pas tous les sous-points normatifs ; `extension` = présent dans le code mais pas dans la spec.

---

## §4 Domain Model

### §4.1.1 Issue

| Champ normatif | file:line | Statut | Écart |
|---|---|---|---|
| `id` (string) | src/tracker/model.rs:13 | conforme | — |
| `identifier` (string) | src/tracker/model.rs:14 | conforme | — |
| `title` (string) | src/tracker/model.rs:15 | conforme | — |
| `description` (string\|null) | src/tracker/model.rs:16 | conforme | `Option<String>` |
| `priority` (int\|null) | src/tracker/model.rs:17 | conforme | `Option<i32>` |
| `state` (string) | src/tracker/model.rs:18 | conforme | — |
| `branch_name` (string\|null) | src/tracker/model.rs:19 | partiel | champ présent mais génération `<number>-<slug>` non implémentée côté GitHub adapter |
| `url` (string\|null) | src/tracker/model.rs:20 | conforme | — |
| `labels` (list\<string\>) lowercase | src/tracker/model.rs:21 + src/tracker/github.rs:301 | conforme | `.to_lowercase()` au remplissage |
| `blocked_by` (list\<BlockerRef\>) | src/tracker/model.rs:22 | partiel | structure correcte mais résolution toujours vide (`fetch_blocked_by` désactivé par défaut, query GraphQL `linkedItems` retirée commit a6cf8f1) |
| `created_at` / `updated_at` ISO-8601 \| null | src/tracker/model.rs:25-26 | conforme | `DateTime<Utc>` |

### §4.1.2 Workflow Definition

| Champ normatif | file:line | Statut | Écart |
|---|---|---|---|
| `config` (map) | src/tracker/model.rs:42 | conforme | `serde_json::Map` |
| `prompt_template` (string trimmed) | src/tracker/model.rs:43 | conforme | trim appliqué dans le parser |

### §4.1.3 Service Config (Typed View)

Tous les getters typés présents dans `src/config/typed.rs` (poll_interval_ms 127-131, workspace_root 134-145, active/terminal_states 101-125, max_concurrent_agents 171-175, cli timeouts 240-264, hooks 148-169). Conforme.

### §4.1.4 Workspace

| Champ normatif | file:line | Statut | Écart |
|---|---|---|---|
| `path` (absolute) | src/tracker/model.rs:139 | conforme | `PathBuf` |
| `workspace_key` (sanitized) | src/tracker/model.rs:140 | conforme | — |
| `created_now` (bool) | src/tracker/model.rs:141 | conforme | — |

### §4.1.5 Run Attempt

Tous les champs présents `src/tracker/model.rs:75-81`. `AttemptStatus` enum couvre les phases §7.2. Conforme.

### §4.1.6 Live Session

| Champ normatif | file:line | Statut | Écart |
|---|---|---|---|
| `session_id` | src/tracker/model.rs:56 | conforme | — |
| `agent_session_handle` | src/tracker/model.rs:57-58 | partiel | nommé `thread_id`/`turn_id` au lieu d'un seul `agent_session_handle` opaque ; sémantique légèrement divergente |
| `agent_pid` | src/tracker/model.rs:59 | conforme | `Option<u32>` |
| `last_agent_event` / `last_agent_timestamp` / `last_agent_message` | src/tracker/model.rs:60-62 | conforme | nommage `last_event`/`last_timestamp`/`last_message` |
| token counters (input/output/total + last_reported) | src/tracker/model.rs:63-68 | conforme | — |
| `turn_count` | src/tracker/model.rs:69 | conforme | — |

### §4.1.7 Retry Entry

| Champ normatif | file:line | Statut | Écart |
|---|---|---|---|
| `issue_id`, `identifier`, `attempt`, `error` | src/tracker/model.rs:131-134 | conforme | — |
| `due_at_ms` (monotonic ms) | src/orchestrator/retry.rs:133 | partiel | utilise `tokio::time::Instant` (handle de timer) au lieu d'un timestamp ms ; comportement équivalent fonctionnellement |
| `timer_handle` | — | absent | pas de handle séparé exposé dans la struct |

### §4.1.8 Orchestrator Runtime State

Tous les champs présents `src/orchestrator/state.rs:25-33`. Conforme.

### §4.2 Identifiers and Normalization

| Règle normative | file:line | Statut | Écart |
|---|---|---|---|
| Workspace key : `[A-Za-z0-9._]`, autres → `-` | src/workspace/manager.rs:36-47 | partiel | autorise aussi `_` et `-`, remplaçant par `_` au lieu de `-` |
| GitHub branch_name `<number>-<slug>`, lowercase, runs non-`[a-z0-9]` → `-`, truncate 60 | src/tracker/github.rs | absent | non implémenté ; `Issue.branch_name` toujours `None` |
| Session ID format `<adapter>-<handle>-<turn>` (RECOMMENDED) | src/orchestrator/tick.rs:478 | partiel | format `{session_id}-{message_id}` |
| State comparison après lowercase | src/config/typed.rs:106 + filtres tick | conforme | — |

---

## §5 WORKFLOW.md Contract

### §5.1 Path Resolution

| Règle | file:line | Statut | Écart |
|---|---|---|---|
| Précédence : runtime explicit > cwd default | src/main.rs | partiel | parsing CLI args présent ; précédence non auditée à la ligne précise |
| Default `WORKFLOW.md` dans cwd | src/main.rs | conforme | — |
| Erreur `missing_workflow_file` typée | src/error.rs:5 | conforme | `SympheoError::MissingWorkflowFile` |

### §5.2 File Format

| Règle | file:line | Statut | Écart |
|---|---|---|---|
| Front matter `---` délimité | src/workflow/parser.rs:6-9 | conforme | — |
| YAML doit décoder en map ; non-map = erreur | src/workflow/parser.rs:12-14 | conforme | `WorkflowFrontMatterNotAMap` |
| Prompt body trimmed | src/workflow/parser.rs:17 | conforme | — |
| Front matter absent → config map vide | src/workflow/parser.rs:24-28 | conforme | — |

### §5.3 Front Matter Schema

Top-level keys conformes (tracker, polling, workspace, hooks, agent, cli) `src/config/typed.rs:30-52`. Voir aussi annexe extensions pour `daytona`, `skills`, `server`.

#### §5.3.5 `agent`

| Champ | Default spec | file:line | Statut |
|---|---|---|---|
| `max_concurrent_agents` | 10 | src/config/typed.rs:171-175 | conforme |
| `max_turns` | 20 | src/config/typed.rs:178-182 | conforme |
| `max_retry_backoff_ms` | 300000 | src/config/typed.rs:200-205 | conforme |
| `max_concurrent_agents_by_state` | `{}` | src/config/typed.rs:222-238 | conforme |

#### §5.3.6 `cli`

| Champ | Default spec | file:line | Statut |
|---|---|---|---|
| `command` | `opencode run` | src/config/typed.rs:240-244 | conforme |
| `args` | `[]` | src/config/typed.rs | absent | clé `cli.args` non lue |
| `env` | `{}` | src/config/typed.rs | absent | clé `cli.env` non lue |
| `turn_timeout_ms` | 3600000 | src/config/typed.rs:246-250 | conforme |
| `read_timeout_ms` | 5000 | src/config/typed.rs:253-257 | conforme |
| `stall_timeout_ms` | 300000 | src/config/typed.rs:260-263 | partiel | default 1800000 (30 min) au lieu de 300000 (5 min) |
| `options` | `{}` | src/config/typed.rs | absent | clé `cli.options` non lue |
| Sélection adapter par leading binary token | — | absent | sélection actuelle = boolean `daytona_enabled` (CLI hardcodée à opencode) |
| `cli_adapter_not_found` error | — | absent | type d'erreur non défini |

### §5.4 Prompt Template Contract

| Règle | file:line | Statut | Écart |
|---|---|---|---|
| Strict template engine (Liquid OK) | src/orchestrator/tick.rs:937-940 | conforme | `liquid::ParserBuilder::with_stdlib()` |
| Unknown variables MUST fail | src/orchestrator/tick.rs:924-933 | conforme | validation amont des variables racine |
| Unknown filters MUST fail | src/orchestrator/tick.rs | absent | aucune validation explicite des filtres |
| `issue` variable | src/orchestrator/tick.rs:943-950 | conforme | — |
| `attempt` integer\|null | src/orchestrator/tick.rs:951-953 | conforme | scalar optionnel |
| Fallback prompt si body vide | src/orchestrator/tick.rs:917-920 | conforme | — |

### §5.5 Workflow Validation Errors

| Erreur normative | file:line | Statut |
|---|---|---|
| `missing_workflow_file` | src/error.rs:5 | conforme |
| `workflow_parse_error` | src/error.rs:9 | conforme |
| `workflow_front_matter_not_a_map` | src/error.rs:12 | conforme |
| `template_parse_error` | src/error.rs:15 | conforme |
| `template_render_error` | src/error.rs:18 | conforme |
| `cli_adapter_not_found` | — | absent |

---

## §6 Configuration

### §6.1 Resolution Pipeline

| Étape | file:line | Statut |
|---|---|---|
| 1. Path selection | src/main.rs | partiel |
| 2. Parse YAML front matter | src/workflow/parser.rs:4-30 | conforme |
| 3. Defaults | src/config/typed.rs (multiple) | conforme |
| 4. `$VAR` indirection | src/config/resolver.rs | conforme |
| 5. Coerce + validate | src/config/typed.rs (multiple) | conforme |
| 6. Tracker adapter resolve | src/config/typed.rs:62-64 | partiel | accessor présent ; sélection `match` non centralisée |
| 7. CLI adapter resolve | src/config/typed.rs:240-244 | absent | pas de résolution par leading binary token |

### §6.2 Dynamic Reload (REQUIRED)

| Règle | file:line | Statut | Écart |
|---|---|---|---|
| Détection des changements WORKFLOW.md | src/main.rs (notify crate) | partiel | dépendance `notify = 8.2` présente ; chemin code à confirmer en P1 |
| Re-read + re-apply sans restart | src/orchestrator/tick.rs:51-56 | partiel | `reload_config()` existe ; couverture exhaustive (poll, concurrency, états, hooks, prompt) à valider |
| Reload invalide → keep last good + log | src/error.rs (multiple) | partiel | error types existent ; comportement « last known good » non vérifié à 100% |

### §6.3 Dispatch Preflight Validation

| Règle | file:line | Statut |
|---|---|---|
| Startup validation | src/config/typed.rs:340-372 (`validate_for_dispatch`) | conforme |
| Per-tick re-validation | src/orchestrator/tick.rs:73 | conforme |
| Tracker-specific validation déléguée | — | absent | pas de méthode `validate()` sur trait Tracker |
| CLI-specific validation déléguée | — | absent | pas de méthode `validate()` sur trait Cli |

---

## §7 Orchestration State Machine

### §7.1 Issue Orchestration States

5 états (Unclaimed, Claimed, Running, RetryQueued, Released) implicitement représentés via `state.running` HashMap, `state.claimed` HashSet, `state.retry_attempts` HashMap. Conforme.

### §7.2 Run Attempt Lifecycle (11 phases)

`AttemptStatus` enum `src/tracker/model.rs:115-125` couvre les 11 phases (PreparingWorkspace, BuildingPrompt, LaunchingAgentTurn, InitializingSession, StreamingTurn, Finishing, Succeeded, Failed, TimedOut, Stalled, CanceledByReconciliation). Conforme. Note : `LaunchingAgentTurn` nommé `LaunchingAgentProcess`.

### §7.3 Transition Triggers

| Trigger | file:line | Statut |
|---|---|---|
| Poll Tick | src/orchestrator/tick.rs:59 | conforme |
| Worker Exit (normal) | src/orchestrator/tick.rs:298-311 | conforme |
| Worker Exit (abnormal) | src/orchestrator/tick.rs:313-331 | conforme |
| Agent Update Event | src/orchestrator/tick.rs:467-540 | conforme |
| Retry Timer | src/orchestrator/tick.rs:374-462 | conforme |
| Reconciliation Refresh | src/orchestrator/tick.rs:218-258 | conforme |
| Stall Timeout | src/orchestrator/tick.rs:174-215 | conforme |

### §7.4 Idempotency

Conforme : single-authority state mutations dans `OrchestratorState`, checks claimed+running avant dispatch, reconciliation avant dispatch. Restart recovery tracker-driven (in-memory by design).

---

## §8 Polling, Scheduling, Reconciliation

### §8.1 Poll Loop

Séquence reconcile → validate → fetch → sort → dispatch implémentée `src/orchestrator/tick.rs:67-156`. Conforme.

### §8.2 Candidate Selection

| Règle | file:line | Statut | Écart |
|---|---|---|---|
| State in active_states & not in terminal_states | src/orchestrator/tick.rs:94-96 | conforme | — |
| Not in `running` / `claimed` | src/orchestrator/tick.rs:122 | conforme | — |
| Slots globaux disponibles | src/orchestrator/tick.rs:119 | conforme | — |
| Slots par état | src/orchestrator/tick.rs:126-132 | conforme | — |
| Blocker rule sur premier active state | src/orchestrator/tick.rs:100-104 | conforme | — |
| Sort priority asc, created_at asc, identifier tie-break | src/orchestrator/tick.rs:108-115 | conforme | — |

### §8.3 Concurrency Control

`available_slots()` `src/orchestrator/state.rs:92-95`, `count_running_by_state()` lowercase `src/orchestrator/state.rs:97-103`. Conforme.

### §8.4 Retry / Backoff

| Règle | file:line | Statut |
|---|---|---|
| Continuation retries 1000ms fixe | src/orchestrator/retry.rs:13-14 | conforme |
| Failure backoff `min(10000 * 2^(attempt-1), max_retry_backoff_ms)` | src/orchestrator/retry.rs:16-20 | conforme |

### §8.5 Active Run Reconciliation

Stall detection `src/orchestrator/tick.rs:174-215` + tracker refresh `src/orchestrator/tick.rs:218-258` avec les 3 cas (terminal/active/neither). Conforme.

### §8.6 Startup Terminal Workspace Cleanup

| Règle | file:line | Statut |
|---|---|---|
| Query terminal issues at startup | — | absent à confirmer en P1 (logique non visible dans les fichiers principaux audités) |
| Remove workspace dir per identifier | src/workspace/manager.rs:121-133 | partiel | `before_remove` hook + cleanup existent ; flow startup à câbler |
| Fetch failure logged + continue | — | absent à confirmer en P1 |

---

## §9 Workspace + Safety Invariants

### §9.1-9.2 Layout & Creation

| Règle | file:line | Statut |
|---|---|---|
| `<root>/<sanitized_identifier>` | src/workspace/manager.rs:49-51 | conforme |
| Sanitization → `[A-Za-z0-9._]` (autres `-`) | src/workspace/manager.rs:36-46 | partiel | autorise aussi `-` et `_`, remplaçant par `_` |
| Création si absent + flag `created_now` | src/workspace/manager.rs:60-66 | conforme |
| `after_create` conditionné à `created_now=true` | src/workspace/manager.rs:69-76 | conforme |

### §9.3 Workspace Population (OPTIONAL, implementation-defined)

`hooks.after_create: git clone ...` dans `WORKFLOW.md:26-27`. Compatible spec (extension implementation-defined). Pas de removal partial-dir on failure (pas explicitement géré).

### §9.4 Hooks

| Hook | Fatalité spec | file:line | Statut |
|---|---|---|---|
| `after_create` (fatal) | fatal | src/workspace/manager.rs:74-75 | conforme |
| `before_run` (fatal) | fatal | configurable via `hook_script("before_run")` | partiel | exécution dans worker à confirmer fatale |
| `after_run` (logged-ignored) | logged-ignored | src/config/typed.rs:148 | partiel | non-fatalité à confirmer |
| `before_remove` (logged-ignored) | logged-ignored | src/workspace/manager.rs:121-133 | conforme |
| `hooks.timeout_ms` (60000 default) | — | src/workspace/manager.rs:12 | conforme |
| Env vars `SYMPHEO_ISSUE_IDENTIFIER`/`_ID`/`_WORKSPACE_PATH` | RECOMMENDED | — | absent | hooks lancés sans ces variables exposées |

### §9.5 Safety Invariants

| Invariant | file:line | Statut |
|---|---|---|
| Inv 1 : `cwd == workspace_path` validé avant launch | src/agent/backend/local.rs:114-115, :141 | partiel | `validate_inside_root(workspace_path)` vérifie containment ; égalité `cwd == workspace_path` non explicitement testée |
| Inv 2 : `workspace_path` sous `workspace_root` | src/workspace/manager.rs:135-147 | conforme | `validate_inside_root()` |
| Inv 3 : `workspace_key` sanitized | src/workspace/manager.rs:36-46 | partiel | voir §9.2 |

---

## §10 CLI Adapter Contract

### §10.1 Adapter Identity / Selection

| Règle | file:line | Statut | Écart |
|---|---|---|---|
| Adapter declares `kind`, `binary_names`, `validate()` | — | absent | pas de trait `CliAdapter` ; `LocalBackend`/`DaytonaBackend` confondent backend env et CLI |
| Sélection par leading binary token de `cli.command` | src/agent/runner.rs:17-24 | absent | sélection actuelle = `daytona_enabled` flag |
| `cli_adapter_not_found` error | — | absent | type d'erreur non défini |

### §10.2 Lifecycle Operations

| Op normative | file:line | Statut | Écart |
|---|---|---|---|
| `start_session(workspace, cli_config) -> SessionContext` | src/agent/backend/local.rs | absent | absent comme méthode séparée ; logique inline dans `run_turn` |
| `run_turn(session, prompt, issue, turn, on_event) -> TurnResult` | src/agent/backend/local.rs:95-275 | partiel | signature `run_turn` existe mais ne reçoit pas de `SessionContext` typé ; `on_event` callback = `Sender<AgentEvent>` |
| `stop_session(session)` | — | absent | pas de méthode dédiée |
| First turn = full prompt | géré côté template | partiel | continuation gérée via `continuation_prompt()` mais découpage first/continuation à valider |
| Continuation = short guidance | src/config/typed.rs:266-271 | conforme | `continuation_prompt()` accessor |

### §10.3 Event Parsing / Normalization

| Règle | file:line | Statut | Écart |
|---|---|---|---|
| Event = `{event, timestamp, agent_pid, usage, rate_limits, ...}` | src/agent/parser.rs | partiel | `AgentEvent` enum sérialise event/payload ; champs spec non tous présents |
| 12 normalized event names | src/agent/parser.rs | partiel | events `StepStart`/`StepFinish`/`Text` etc. ; mapping vers `session_started`/`turn_completed`/etc. non normalisé |
| Stratégie de parsing documentée + version range CLI | — | absent | aucune doc de version OpenCode testée |

### §10.4 Approval / Tool Calls / User Input

| Règle | file:line | Statut |
|---|---|---|
| Implementation MUST document approval posture | — | absent |
| Approval requests don't stall indefinitely | src/agent/backend/local.rs:132 | partiel | `--dangerously-skip-permissions` passé à opencode (extension non spec) ; aucun fallback de timeout structuré pour user-input-required |
| Unsupported tool invocations don't hang | — | absent |

### §10.5 Error Mapping (11 catégories normatives)

| Catégorie | file:line | Statut |
|---|---|---|
| `cli_not_found` | — | absent |
| `invalid_workspace_cwd` | src/agent/backend/local.rs:114-115 | partiel | erreur générique au lieu de catégorisée |
| `session_start_failed` | — | absent |
| `turn_launch_failed` | src/agent/backend/local.rs:158 | partiel | `AgentRunnerError("spawn failed")` |
| `turn_read_timeout` | src/error.rs (AgentTurnTimeout) | partiel | nom différent |
| `turn_total_timeout` | src/error.rs (AgentTurnTimeout) | partiel | confondu avec read |
| `turn_cancelled` | — | absent |
| `turn_failed` | src/error.rs | partiel | mappé sur AgentRunnerError générique |
| `subprocess_exit` | src/error.rs (AgentProcessExit) | conforme | — |
| `output_parse_error` | — | absent |
| `user_input_required` | — | absent |

### §10.6 OpenCode Reference Adapter

| Règle | file:line | Statut |
|---|---|---|
| Default `cli.command = "opencode run"` | src/config/typed.rs:243 | conforme |
| Allocate session handle (UUIDv4 ou first-print) | — | partiel | reuse du `session_id` capturé ; pas de UUID alloué côté sympheo |
| `opencode run --session <handle> -- <prompt>` | src/agent/backend/local.rs:131-140 | partiel | format actuel : `bash -lc 'PROMPT=$(cat "..."); opencode "$PROMPT" --format json --dir "..." --dangerously-skip-permissions [--session ...]'` ; pas de `--` séparateur explicite |
| Parse stdout/log JSON pour final message, tool calls, tokens, rate_limits | src/agent/parser.rs | partiel | parse `StepStart`/`Text`/`StepFinish` ; tool call/result moins explicite |
| `cli.options.{model,permissions,mcp_servers}` recognized | — | absent | clé `cli.options` non lue |
| Unknown `cli.options` keys = log warning | — | absent | clé pas lue du tout |
| Documentation version OpenCode testée | — | absent |

### §10.7 Worker Algorithm

| Étape | file:line | Statut |
|---|---|---|
| 1. Create/reuse workspace | src/workspace/manager.rs:54-84 | conforme |
| 2. before_run hook | accesseur présent | partiel |
| 3. start_session | — | absent (pas de méthode) |
| 4. Per-turn loop | src/orchestrator/tick.rs (run_worker) | conforme |
| 5. stop_session | — | absent |
| 6. after_run hook | accesseur présent | partiel |
| 7. Exit normal | src/orchestrator/tick.rs:298-311 | conforme |

---

## §11 Tracker Adapter Contract

### §11.1 REQUIRED Operations

| Op | file:line | Statut |
|---|---|---|
| `validate(tracker_config)` | — | absent | pas de méthode trait ; validation faite dans `GithubTracker::new()` |
| `fetch_candidate_issues()` | src/tracker/mod.rs:10 ; src/tracker/github.rs:387-393 | conforme |
| `fetch_issues_by_states([])` empty no API call | src/tracker/github.rs:396-407 | conforme | early return ligne 397 |
| `fetch_issue_states_by_ids()` minimal + omit missing | src/tracker/github.rs:409-417 | conforme |

### §11.2 Normalization

| Règle | file:line | Statut |
|---|---|---|
| Labels lowercase | src/tracker/github.rs:301 | conforme |
| `priority` int\|null | src/tracker/github.rs:362 | partiel | toujours `null` (priority_field non implémenté) |
| `created_at`/`updated_at` ISO-8601 | src/tracker/github.rs:373-380 | conforme |
| `blocked_by` list of refs | src/tracker/github.rs:325-355 | partiel | structure correcte ; toujours vide (`fetch_blocked_by=false` par défaut, query GraphQL `linkedItems` retirée commit a6cf8f1) |
| `state` raw, lowercase comparison | src/tracker/github.rs:308-313 | conforme |

### §11.3 Error Categories (8)

| Catégorie | file:line | Statut |
|---|---|---|
| `unsupported_tracker_kind` | src/error.rs | partiel | nom différent |
| `missing_tracker_auth` | src/error.rs (`MissingTrackerApiKey`) | partiel |
| `missing_tracker_project_identity` | src/error.rs | partiel |
| `tracker_request_failed` | src/error.rs (`TrackerApiRequest`) | conforme |
| `tracker_status_error` | src/error.rs (`TrackerApiStatus`) | conforme |
| `tracker_graphql_errors` | src/tracker/github.rs:91-100 | partiel | détection inline mais pas de variant typé |
| `tracker_unknown_payload` | src/error.rs (`TrackerMalformedPayload`) | conforme |
| `tracker_pagination_error` | — | absent |

### §11.4 GitHub Reference Adapter

| Règle | file:line | Statut | Écart |
|---|---|---|---|
| `kind=github` validation | src/config/typed.rs:340-348 | conforme | — |
| `org` REQUIRED | src/tracker/github.rs:40-45 | partiel | extrait de `project_slug` `owner/repo` ; pas de clé `org` propre |
| `project_number` positive int | src/tracker/github.rs:35-39 | partiel | présence vérifiée ; positivité non explicite |
| `status_field` REQUIRED + existence on project | src/tracker/github.rs:258 | absent | hardcodé `"Status"` ; pas de validation contre le project |
| `priority_field` OPTIONAL | — | absent | non implémenté |
| `endpoint` default + GH Enterprise support | src/config/typed.rs:66-76 | partiel | default `https://api.github.com` (sans `/graphql`) |
| `auth_token` default `$SYMPHEO_GITHUB_TOKEN` | src/config/typed.rs:78-83 | conforme |
| `active_states` default `["Todo","In Progress"]` lowercased | src/config/typed.rs:101-108 | conforme |
| `terminal_states` default | src/config/typed.rs:110-125 | conforme | inclut additionnels (closed/canceled/duplicate) |
| Identifier `<repo>#<number>` | src/tracker/github.rs:359 | partiel | format actuel = `<REPO>-<number>` (pas de `#`) |
| Branch_name `<number>-<slug>` truncate 60 | — | absent | non généré |
| Status field semantics (single-select option name) | src/tracker/github.rs:248-267 | conforme |
| Issue sans value status_field → state `""` (ignoré) | src/tracker/github.rs:307-314 | partiel | fallback sur github state au lieu de string vide |
| Project membership filter PRs/drafts | src/tracker/github.rs:269-280 | partiel | filter par repo, pas par type Issue/PR/Draft |
| Blockers `trackedInIssues` GraphQL | — | absent | retiré commit a6cf8f1 |
| Fallback `Blocked by #N` body parsing | — | absent |
| Auth `Bearer` + `Accept: application/vnd.github+json` | src/tracker/github.rs:47-54 | conforme |
| HTTP timeout 30000ms, page size 50 | src/tracker/github.rs:47-54, :226-244 | conforme |
| Validation `org`/`project_number`/`status_field`/`auth_token` | src/tracker/github.rs:28-70 | partiel | status_field non validé contre project |

### §11.5 Tracker Writes

Sympheo NE fait pas de mutation tracker (état/comment/PR) côté coeur ; toutes mutations déléguées à l'agent via tools. `move_issue_state`/`add_comment`/`update_issue_body`/`create_pull_request` du trait sont stubs `Ok(())` ou `UnsupportedTrackerKind`. Conforme (§11.5 explicit non-goal pour le coeur).

### §11.6 OPTIONAL `github_graphql` Tool Extension

Non implémenté (`absent`). Aucune surface MCP ou outil exposé à l'agent permettant un GraphQL passthrough authentifié.

---

## §12 Prompt Construction

| Règle | file:line | Statut |
|---|---|---|
| Strict variable checking | src/orchestrator/tick.rs:924-933 | conforme |
| Strict filter checking | — | absent | non vérifié |
| `issue` + `attempt` injectés | src/orchestrator/tick.rs:943-953 | conforme |
| Failure semantics : fail run + retry | src/orchestrator/retry.rs | conforme |

---

## §13 Logging / Observability

### §13.1 Required Context

`tracing` utilisé partout avec `issue_id`, `issue_identifier`, `session_id` dans les spans (ex : `src/agent/backend/local.rs:104-110`). Conforme.

### §13.3 Runtime Snapshot

`/api/v1/state` `src/server/mod.rs:347-433` retourne `running[]`, `retrying[]`, `agent_totals` (input/output/total/seconds_running), `rate_limits`. Conforme. Champs `turn_count` par run présent.

### §13.5 Token Accounting

Absolute totals préférés ; deltas `last_reported_input_tokens`/etc. tracés `src/tracker/model.rs:46-68`. Mode delta vs absolute non documenté formellement par adapter. Partiel.

### §13.7 OPTIONAL HTTP Server

| Route | file:line | Statut |
|---|---|---|
| `GET /` (HTML dashboard) | src/server/mod.rs:34-338 | conforme |
| `GET /api/v1/state` | src/server/mod.rs:347-433 | conforme |
| `GET /api/v1/<id>` | src/server/mod.rs:435-490 | conforme |
| `POST /api/v1/refresh` | src/server/mod.rs | conforme |
| Bind par défaut loopback | src/server/mod.rs:23 | conforme | `127.0.0.1` |
| `--port` CLI override `server.port` | src/main.rs | partiel |

---

## §14 Failure Model

5 classes (Workflow, Workspace, Agent, Tracker, Observability) couvertes par `SympheoError` enum `src/error.rs` + recovery behavior conforme (§14.2 : skip dispatch / convert to retry / keep service alive). État in-memory by design (§14.3) — aucune persistance retry timer / running session inter-restart, conforme.

---

## §15 Security & Operational Safety

| Règle | file:line | Statut |
|---|---|---|
| §15.1 Trust boundary (no artifact validation) | by design | conforme |
| §15.2 Workspace under root | src/workspace/manager.rs:135-147 | conforme |
| §15.2 cwd = workspace_path | src/agent/backend/local.rs:141 | partiel | containment OK, égalité explicite à durcir |
| §15.2 Sanitized identifiers | src/workspace/manager.rs:36-47 | partiel | autorise `-` et `_` |
| §15.3 `$VAR` indirection | src/config/resolver.rs | conforme |
| §15.3 No token logging | by design | conforme |
| §15.4 Hook timeout REQUIRED | src/workspace/manager.rs:12 | conforme |
| §15.5 Harness hardening (opt-in) | hooks + daytona | conforme | par design |

---

## §16 Reference Algorithms

| Sous-section | file:line | Statut | Écart |
|---|---|---|---|
| 16.1 Service Startup | src/main.rs + src/orchestrator/mod.rs | partiel | startup terminal cleanup §8.6 manque |
| 16.2 Poll-and-Dispatch Tick | src/orchestrator/tick.rs:59-156 | conforme | — |
| 16.3 Reconcile Active Runs | src/orchestrator/tick.rs:67, :174-258 | conforme | — |
| 16.4 Dispatch One Issue | src/orchestrator/tick.rs:117-156 | conforme | — |
| 16.5 Worker Attempt | src/orchestrator/tick.rs (run_worker) + src/agent/backend/local.rs | partiel | start_session/stop_session non explicites |
| 16.6 Worker Exit + Retry | src/orchestrator/tick.rs:298-331 + src/orchestrator/retry.rs | conforme | — |

---

## §17 Test Matrix

### §17.1 Workflow/config parsing
Tests YAML, defaults, validation, $VAR, ~ expansion, per-state concurrency, prompt rendering présents (`tests/integration_test.rs` + tests embarqués). Conforme partiellement — strict filter checking non testé.

### §17.2 Workspace + safety invariants
Sanitization, create/reuse, hooks (after_create, before_remove), root containment testés. **Manquant** : env vars `SYMPHEO_*` exposées aux hooks (test absent car feature absente).

### §17.3 Tracker Adapter Contract Conformance
GitHub adapter testé (`tests/github_tracker_test.rs` ~508 lignes) : validation, normalization, pagination. **Manquant** : tracker_pagination_error mapping, status_field validation contre project.

### §17.4 GitHub Reference Adapter
Validation, status field extraction, normalization couverts. **Manquant** : identifier `<repo>#<number>` (format actuel `<REPO>-<num>`), branch_name generation, blockers `trackedInIssues`, body fallback.

### §17.5 Orchestrator dispatch/reconciliation/retry
Backoff, retry queue, reconciliation testés (`tests/orchestrator_test.rs` ~765 lignes). **Manquant** : sort order priority+created assert, snapshot timeout/unavailable.

### §17.6 CLI Adapter Contract Conformance
Backend creation + session mgmt + run_turn testés. **Manquant** : `validate()` séparé, adapter selection par leading binary token, normalized event names assertions.

### §17.7 OpenCode Reference Adapter
Default cli.command testé. **Manquant** : version range OpenCode documenté+testé, unknown options warning, session resumption explicit, full vs continuation prompt.

### §17.8 Observability
Structured logging vérifié implicitement. **Manquant** : assertion explicite des context fields requis (issue_id/identifier/session_id) dans tous les logs concernés.

### §17.9 CLI/host lifecycle
Args parsing + exit codes testés. Conforme.

### §17.10 Real Integration Profile (RECOMMENDED)
**Absent**. Pas de smoke test live GitHub (tout mocké), pas de smoke test live opencode, pas de gate CI/flag.

---

## Annexe A. Extensions et Comportements Hors Spec

Ces éléments sont présents dans le code mais pas dans la spec stricte. La spec autorise les extensions documentées (§5.3 « Extensions SHOULD document their field schema »).

| Extension | file:line | Décision recommandée pour P1 |
|---|---|---|
| `daytona:` front matter (cloud sandbox backend) | src/config/typed.rs:54-339 + src/agent/backend/daytona.rs | documenter comme extension (Appendix-style), aligner sur le futur trait `Executor` |
| `skills:` mapping front matter (state → SKILL.md path) | src/config/typed.rs:58, :374-376 + src/skills/* | documenter comme extension OU baker dans le prompt body via Liquid |
| `workspace.git_reset_strategy` (default "stash") | src/config/typed.rs:157-162 + src/git/local.rs | documenter comme extension implementation-defined |
| `agent.max_turns_per_state` (map per-state) | src/config/typed.rs:185-198 | documenter comme extension §5.3.5 (forward compat) |
| `agent.max_retry_attempts` (cap total retries) | src/config/typed.rs:207-212 | documenter comme extension §5.3.5 |
| `cli.continuation_prompt` (custom continuation guidance) | src/config/typed.rs:266-271 | conforme à §10.6 esprit ; documenter |
| `tracker.fetch_blocked_by` flag | src/config/typed.rs:95-99 | documenter comme extension §5.3.1 |
| `server.port` HTTP extension | src/config/typed.rs:214-220 | conforme §13.7 (extension documentée) |
| `--dangerously-skip-permissions` passé à opencode | src/agent/backend/local.rs:132 | déplacer dans `cli.options` (§5.3.6) avec doc explicite ; conforme §10.6 esprit |
| `probe_opencode` pre-flight (timeout 10s systématique) | src/agent/backend/local.rs:33-90 | supprimer ; non spec ; couvert par `cli.turn_timeout_ms` |
| `bash -lc 'PROMPT=$(cat ...); ...'` injection prompt par fichier | src/agent/backend/local.rs:124-140 | conforme §10.6 (workaround ARG_MAX), à documenter |
| `LiveSession.thread_id`/`turn_id` au lieu d'`agent_session_handle` unique | src/tracker/model.rs:57-58 | renommer/consolider en P1 pour conformité §4.1.6 |
| `Issue.identifier` format `<REPO>-<number>` au lieu de `<repo>#<number>` | src/tracker/github.rs:359 | aligner sur §11.4.2 en P1 |

---

## Annexe B. Synthèse Couverture par Section

| Section | Couverture | Verdict global |
|---|---|---|
| §4 Domain model | ~95% | conforme (renommages mineurs) |
| §5 WORKFLOW.md | ~85% | conforme avec écarts (cli.args/env/options non lus, cli_adapter_not_found absent) |
| §6 Config | ~80% | conforme avec écarts (résolution adapter par binary, dynamic reload partiel) |
| §7 State machine | ~95% | conforme |
| §8 Polling/scheduling | ~95% | conforme (startup terminal cleanup à confirmer) |
| §9 Workspace + safety | ~75% | partiel (sanitization stricte, env vars hooks, invariant 1 explicite) |
| §10 CLI Adapter Contract | ~40% | partiel important — pas de trait adapter, pas de selection par binary, error mapping incomplet |
| §10.6 OpenCode adapter | ~50% | partiel — flow présent, séparation start/run/stop manquante, options non lues |
| §11 Tracker contract | ~70% | partiel (validate trait, error categories, blockers absents) |
| §11.4 GitHub adapter | ~70% | partiel (identifier format, branch_name, blockers, status_field validation) |
| §12 Prompt construction | ~80% | conforme avec strict filter check absent |
| §13 Logging + observability | ~95% | conforme |
| §14 Failure model | ~100% | conforme |
| §15 Security | ~85% | conforme avec invariants à durcir |
| §16 Reference algorithms | ~90% | conforme |
| §17 Tests | ~75% (core) / 0% (real integration) | partiel |
| Appendix A SSH | 0% | absent (non implémenté ; Daytona est un backend différent, pas SSH) |
| Appendix B Linear | 0% | absent (référence dans config defaults uniquement) |

**Conformité globale estimée** : ~75% sur le contrat core (sans les extensions), ~90% si on accepte les extensions actuelles documentées comme telles.

---

## Annexe C. Recommandations pour P1 (alignment refactor)

P1 doit traiter par ordre de criticité :

1. **§10 CLI Adapter Contract** : créer le trait `CliAdapter` (validate / start_session / run_turn / stop_session), sélection par leading binary token, error mapping normalisé, stub `PiAdapter`.
2. **§11 Tracker Adapter Contract** : ajouter `validate()` au trait, mapper les 8 error categories.
3. **§11.4 GitHub adapter** : aligner identifier `<repo>#<number>`, générer `branch_name`, restaurer blockers via `trackedInIssues` (ou fallback body parsing) avec doc explicite, valider status_field contre project.
4. **§9.4 Hooks env vars** : exposer `SYMPHEO_ISSUE_IDENTIFIER` / `SYMPHEO_ISSUE_ID` / `SYMPHEO_WORKSPACE_PATH`.
5. **§9.5 Safety Invariant 1** : assertion explicite `cwd == workspace_path` avant launch.
6. **§9.2 Sanitization** : restreindre à `[A-Za-z0-9._]` strict, remplacer par `-` (et non `_`).
7. **§5.3.6 cli.{args,env,options}** : lire ces clés dans la config et les passer au trait `CliAdapter`.
8. **§8.6 Startup terminal workspace cleanup** : confirmer la présence et compléter si absent.
9. **§5.5 `cli_adapter_not_found`** : ajouter le variant typé.
10. **Extensions** : documenter ou supprimer selon Annexe A (`probe_opencode` à supprimer, autres à documenter dans `docs/extensions.md` à créer).

P1 n'introduit pas de nouvelle feature ; il aligne strictement.
