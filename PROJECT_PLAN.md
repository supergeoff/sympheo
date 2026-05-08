# Sympheo — Plan Projet Complet

> **Rôle :** PM/PO + Architecte Technique  
> **Date :** 2026-05-08  
> **Objectif :** Guider des agents IA dans l'implémentation de 4 workstreams critiques (1 prioritaire + 3 fonctionnels) sur le codebase Rust Sympheo.

---

## Table des matières

1. [Analyse du Codebase](#1-analyse-du-codebase)
2. [Matrice de couverture SPEC.md](#2-matrice-de-couverture-specmd)
3. [Workstream 0 — Core Compliance & SPEC Conformance (PRIORITAIRE)](#3-workstream-0--core-compliance--spec-conformance-prioritaire)
4. [Workstream 1 — Backend Daytona (Terminer)](#4-workstream-1--backend-daytona-terminer)
5. [Workstream 2 — Dashboard HTML + Pico CSS](#5-workstream-2--dashboard-html--pico-css)
6. [Workstream 3 — Skill Mapping par Colonne Workflow](#6-workstream-3--skill-mapping-par-colonne-workflow)
7. [Considérations Transverses](#7-considérations-transverses)
8. [Dépendances et Ordre d'exécution](#8-dépendances-et-ordre-dexécution)

---

## 1. Analyse du Codebase

### 1.1 Vue d'ensemble

**Sympheo** est un orchestrateur Rust (async/tokio) qui fait le pont entre un **issue tracker** (actuellement GitHub Projects) et des **agents de codage** (OpenCode/Codex). Le service lit un fichier `WORKFLOW.md` (YAML front matter + template Liquid), poll le tracker, crée des workspaces par issue, et lance des sessions agent.

**Stack technique :**
- Langage : Rust (edition 2021)
- Async runtime : Tokio (full features)
- HTTP serveur : Axum 0.8
- Templating : Liquid 0.26
- Parsing YAML : serde_yaml 0.9
- HTTP client : reqwest 0.12
- File watcher : notify 7.0
- Logging : tracing + tracing-subscriber
- CLI : clap 4.5

### 1.2 Architecture des modules

```
src/
├── main.rs              # Point d'entrée CLI, bootstrap, boucle principale
├── lib.rs               # Déclarations de modules publiques
├── error.rs             # Enum centralisée SympheoError
├── agent/
│   ├── mod.rs           # Exports backend/parser/runner
│   ├── backend/
│   │   ├── mod.rs       # Trait AgentBackend
│   │   ├── local.rs     # Backend local (subprocess bash + opencode run)
│   │   └── daytona.rs   # Backend Daytona (sandbox API + opencode run) ⚠️ INCOMPLET
│   ├── parser.rs        # Parsing JSON line protocol OpenCode (step_start/text/step_finish)
│   └── runner.rs        # AgentRunner (sélection backend local vs daytona)
├── config/
│   ├── mod.rs           # Exports
│   ├── resolver.rs      # Résolution $VAR, ~, chemins relatifs
│   └── typed.rs         # ServiceConfig — getters typés avec défauts
├── orchestrator/
│   ├── mod.rs           # Exports
│   ├── state.rs         # OrchestratorState, RunningEntry, TokenTotals
│   ├── retry.rs         # Logique de backoff exponentiel + continuation retry
│   └── tick.rs          # Orchestrator principal (tick, reconcile, spawn_worker, run_worker)
├── tracker/
│   ├── mod.rs           # Trait IssueTracker
│   ├── model.rs         # Issue, BlockerRef, LiveSession, RetryEntry, WorkflowDefinition...
│   └── github.rs        # Implémentation GitHub Projects via GraphQL ⚠️ NON CONFORME SPEC
├── server/
│   └── mod.rs           # Serveur HTTP Axum (dashboard + API REST basique)
├── workflow/
│   ├── mod.rs           # Exports
│   ├── loader.rs        # WorkflowLoader (lecture fichier)
│   └── parser.rs        # Parse YAML front matter + body markdown
└── workspace/
    ├── mod.rs           # Exports
    └── manager.rs       # WorkspaceManager (création, hooks, sanitization)
```

### 1.3 Comportement actuel (très simplifié)

1. **Démarrage** (`main.rs`) :
   - Charge `WORKFLOW.md`
   - Crée `ServiceConfig`
   - Valide la config (exige `tracker.kind == "github"`)
   - Nettoie les workspaces des issues en état terminal
   - Démarre le serveur HTTP si `--port` fourni
   - Démarre le watcher de fichier `WORKFLOW.md`
   - Lance la boucle de polling

2. **Tick orchestrateur** (`orchestrator/tick.rs`) :
   - Reconciliation : stall detection + refresh états tracker
   - Validation preflight
   - Fetch candidates
   - Sort (priority → created_at → identifier)
   - Dispatch avec contraintes de concurrence (global + par état)

3. **Worker** (`run_worker` dans `tick.rs`) :
   - Crée/réutilise workspace
   - Lance hook `before_run`
   - Tour 1 : render prompt Liquid complet
   - Tours N>1 : prompt hardcodé "Continue working..."
   - Appelle `AgentRunner::run_turn`
   - Met à jour l'état live (tokens, session_id, turn_count)
   - Refresh état issue, break si non-actif ou max_turns atteint
   - Hook `after_run`

4. **Backend Local** (`agent/backend/local.rs`) :
   - Spawn `bash -lc "opencode run <prompt> --format json ..."`
   - Parse stdout ligne par ligne (JSON)
   - Capture StepStart → Text → StepFinish
   - Kill le process après StepFinish
   - Timeout basé sur `codex.turn_timeout_ms`

5. **Backend Daytona** (`agent/backend/daytona.rs`) :
   - Lit la config `daytona.*` depuis ServiceConfig
   - Crée un sandbox via POST `/api/sandbox`
   - Persiste l'ID sandbox dans `.daytona_sandbox_id` dans le workspace
   - Exécute `opencode run` via l'API execute de Daytona
   - Parse le résultat de la même façon que le backend local
   - **Ne parle PAS le protocole Codex app-server**
   - **Ne gère pas les sessions/threads/turns de manière stateful**

---

## 2. Matrice de couverture SPEC.md

La specification (`SPEC.md`, 2169 lignes, Draft v1) définit un orchestrateur générique initialement conçu pour Linear. Notre implémentation actuelle utilise GitHub et OpenCode. Voici l'analyse de conformité :

### 2.1 ✅ Fonctionnalités couvertes

| Section SPEC | Description | État | Fichiers |
|---|---|---|---|
| 5.1-5.2 | Workflow loader (YAML front matter + markdown body) | ✅ | `workflow/loader.rs`, `workflow/parser.rs` |
| 5.3-5.4 | Front matter schema basique, prompt template Liquid | ✅ | `config/typed.rs`, `orchestrator/tick.rs` |
| 5.5 | Erreurs workflow (missing, parse, front_matter_not_map) | ✅ | `error.rs` |
| 6.1 | Résolution config ($VAR, ~, chemins relatifs) | ✅ | `config/resolver.rs` |
| 6.2 | Dynamic reload via file watcher | ✅ | `main.rs` (notify watcher) |
| 6.3 | Dispatch preflight validation | ✅ | `config/typed.rs::validate_for_dispatch()` |
| 7.1 | Orchestration states (Unclaimed, Claimed, Running, RetryQueued, Released) | ✅ | `orchestrator/state.rs`, `tick.rs` |
| 7.2-7.3 | Run attempt lifecycle + transitions | ⚠️ Partiel | Manque certains statuts détaillés (PreparingWorkspace, etc.) |
| 7.4 | Idempotency et duplicate dispatch prevention | ✅ | `tick.rs` (claimed + running checks) |
| 8.1-8.4 | Polling loop, candidate selection, sorting, concurrency, retry backoff | ✅ | `orchestrator/tick.rs`, `retry.rs` |
| 8.5 | Reconciliation (stall + tracker state refresh) | ✅ | `tick.rs::reconcile()` |
| 8.6 | Startup terminal workspace cleanup | ✅ | `main.rs` |
| 9.1-9.3 | Workspace layout, creation, reuse | ✅ | `workspace/manager.rs` |
| 9.4 | Workspace hooks (after_create, before_run, after_run, before_remove) | ✅ | `workspace/manager.rs` |
| 9.5 | Safety invariants (cwd, root containment, sanitization) | ✅ | `workspace/manager.rs` |
| 12.1-12.3 | Prompt rendering (issue + attempt, strict Liquid) | ✅ | `orchestrator/tick.rs::build_prompt()` |
| 13.1-13.2 | Structured logging | ✅ | tracing partout |
| 13.3 | Runtime snapshot (state interne) | ✅ | `orchestrator/state.rs` |
| 13.7 | HTTP server extension | ⚠️ Basique | `server/mod.rs` — endpoints présents mais dashboard minimal |
| 14.1-14.4 | Failure model, recovery, restart behavior | ✅ | `tick.rs`, `retry.rs` |
| 17.1-17.4 | Tests core (config, workspace, tracker, orchestrator) | ⚠️ Partiel | `tests/integration_test.rs` — couverture minime |

### 2.2 ❌ Écarts majeurs (hors scope des 3 workstreams)

| Section SPEC | Problème | Impact |
|---|---|---|
| 5.3.1 | `tracker.kind` attend `"linear"`, on a `"github"` | Non-conformité spec (mais correspond au besoin utilisateur) |
| 11.x | Tracker client est GitHub pas Linear | L'implémentation actuelle est GitHub-centric |
| 10.x | Agent runner utilise `opencode run` au lieu de `codex app-server` | Non-conformité protocol app-server. Cependant c'est le choix d'utiliser OpenCode. |
| 17.5 | App-server client tests | Absents — difficile à tester sans vrai Codex |
| 18.2 | `linear_graphql` tool extension | Non implémenté |

### 2.3 ⚠️ Problèmes spécifiques aux 3 workstreams

**Workstream 1 — Daytona :**
- Le backend Daytona exécute `opencode run` comme une commande one-shot via l'API execute. Il ne maintient pas de session app-server vivante.
- Pas de gestion thread/turn selon le protocole Codex app-server.
- Pas de streaming events vers l'orchestrator pendant l'exécution.
- Pas de gestion des policies (approval, sandbox).
- Le `delete_sandbox` existe mais n'est jamais appelé.

**Workstream 2 — Dashboard :**
- Le dashboard (`/`) retourne un HTML brut sans style : `<html><body><h1>Symphony</h1>...`
- Ne consomme pas l'API JSON pour s'enrichir.
- Manque : auto-refresh, tables détaillées, métriques, indicateurs d'erreur.
- L'API `/api/v1/state` manque des champs (last_event, last_message, tokens par session).
- L'API `/api/v1/:issue_identifier` manque la plupart des champs suggérés par la spec.

**Workstream 3 — Skill Mapping :**
- Aucun concept de "skill" dans le codebase.
- Le prompt template est global, pas de surcharge par état/column.
- Aucun dossier `skills/` n'existe.
- La spec ne mentionne pas les skills — c'est une **feature métier spécifique au besoin utilisateur**.

---

## 3. Workstream 0 — Core Compliance & SPEC Conformance (PRIORITAIRE)

> **🔴 Ce workstream est bloquant et doit être traité en premier.**  
> Il corrige les écarts entre l'implémentation actuelle et le contrat SPEC.md qui affectent la fiabilité, l'observabilité et la conformité du système.

### 3.1 Analyse des écarts core identifiés

| # | Écart | SPEC | Fichier(s) concerné(s) | Sévérité |
|---|---|---|---|---|
| 0.1 | `RunAttempt` / `AttemptStatus` jamais instanciés | §7.2 — 11 statuts de lifecycle | `src/tracker/model.rs`, `src/orchestrator/tick.rs` | Moyenne |
| 0.2 | Liquid template **pas en strict mode** — variables manquantes rendues comme `""` | §5.4 — "Unknown variables MUST fail rendering" | `src/orchestrator/tick.rs::build_prompt()` | **Haute** |
| 0.3 | Token accounting **naïf** (`+=` brut) — risque de double comptage | §13.5 — "track deltas relative to last reported totals" | `src/orchestrator/tick.rs::run_worker()` | **Haute** |
| 0.4 | Rate limits **jamais mis à jour** depuis les events agent | §13.5 — "Track the latest rate-limit payload" | `src/orchestrator/state.rs`, `src/agent/parser.rs` | Moyenne |
| 0.5 | Prompt de continuation **hardcodé en anglais** | §7.1 — "Continuation turns SHOULD send only continuation guidance" | `src/orchestrator/tick.rs::run_worker()` | Moyenne |
| 0.6 | Retry queue sans **vrai timer handle** | §4.1.7 — `timer_handle` requis | `src/tracker/model.rs`, `src/orchestrator/tick.rs` | Faible |
| 0.7 | Events agent incomplets — seuls 3/13 events sont parsés | §10.4 — ~13 events requis | `src/agent/parser.rs` | **Haute** |
| 0.8 | Defaults config non conformes (`codex.command`, `tracker.endpoint`) | §6.4 | `src/config/typed.rs` | Faible |
| 0.9 | Tests core insuffisants (dispatch sort, reconciliation, stall, backoff) | §17.1–17.4 | `tests/integration_test.rs` | Moyenne |

### 3.2 Découpage des tâches

#### Tâche 0.1 : Activer le strict mode Liquid + tests

**Fichiers :**
- Modifier : `src/orchestrator/tick.rs`
- Créer : `tests/liquid_strict_test.rs`

**Détails :**
Le parser Liquid par défaut remplace les variables inconnues par `""`. Pour être strict, on doit vérifier les variables **avant** le render, ou utiliser une option stricte.

```rust
// Option 1 : validation manuelle des variables utilisées dans le template
fn validate_template_vars(template_str: &str, available_vars: &[&str]) -> Result<(), SympheoError> {
    // Regex pour extraire {{ var.name }} et {% if var.name %}
    // Vérifier que chaque variable existe dans available_vars
}

// Option 2 : patch liquid pour échouer sur missing
// Liquid ne supporte pas nativement un mode strict. 
// Solution pragmatique : wrapper le render et détecter si des {{ ... }} subsistent dans l'output.
fn render_strict(...) -> Result<String, SympheoError> {
    let rendered = template.render(&globals).map_err(...)?;
    // Vérifier s'il reste des patterns {{ ... }} non remplacés
    if UNRESOLVED_RE.is_match(&rendered) {
        return Err(SympheoError::TemplateRenderError("unresolved variable".into()));
    }
    Ok(rendered)
}
```

**Critère d'acceptation :** Un template avec `{{ issue.unknown_field }}` doit produire une `TemplateRenderError`.

---

#### Tâche 0.2 : Corriger le token accounting (delta-based)

**Fichiers :**
- Modifier : `src/orchestrator/tick.rs`
- Modifier : `src/orchestrator/state.rs`
- Modifier : `src/tracker/model.rs`

**Détails :**
Dans `LiveSession`, les champs `last_reported_*` existent mais ne sont pas utilisés pour calculer un delta.

```rust
// Dans run_worker, après un turn_result avec tokens :
if let Some(ref tokens) = turn_result.tokens {
    let mut st = state.write().await;
    if let Some(entry) = st.running.get_mut(&issue.id) {
        if let Some(ref mut sess) = entry.session {
            // Calculer le delta par rapport au dernier total rapporté
            let delta_input = tokens.input.saturating_sub(sess.last_reported_input_tokens);
            let delta_output = tokens.output.saturating_sub(sess.last_reported_output_tokens);
            let delta_total = tokens.total.saturating_sub(sess.last_reported_total_tokens);
            
            st.codex_totals.input_tokens += delta_input;
            st.codex_totals.output_tokens += delta_output;
            st.codex_totals.total_tokens += delta_total;
            
            sess.last_reported_input_tokens = tokens.input;
            sess.last_reported_output_tokens = tokens.output;
            sess.last_reported_total_tokens = tokens.total;
            
            sess.input_tokens = tokens.input;
            sess.output_tokens = tokens.output;
            sess.total_tokens = tokens.total;
        }
    }
}
```

**Critère d'acceptation :** Si deux turns consécutifs rapportent les mêmes totaux (1000 input, 500 output), le deuxième n'ajoute rien aux totaux globaux.

---

#### Tâche 0.3 : Parser tous les events agent requis par le SPEC

**Fichiers :**
- Modifier : `src/agent/parser.rs`
- Modifier : `src/orchestrator/tick.rs`

**Détails :**
Ajouter les events manquants dans `OpencodeEvent` (ou une struct générique si le format diffère) :

```rust
pub enum AgentEvent {
    SessionStarted { session_id: String, thread_id: String },
    TurnCompleted { session_id: String, turn_id: String, tokens: Option<TokenInfo> },
    TurnFailed { session_id: String, reason: String },
    TurnCancelled { session_id: String },
    TurnInputRequired { session_id: String },
    ApprovalAutoApproved { session_id: String, kind: String },
    Notification { session_id: String, message: String },
    RateLimit { payload: serde_json::Value },
    TokenUsage { input: u64, output: u64, total: u64 },
    StepStart { ... },   // existant
    Text { ... },        // existant
    StepFinish { ... },  // existant
    Malformed { raw: String },
    Other,
}
```

Chaque event doit être forwardé à l'orchestrator via un callback ou un channel. Dans `run_worker`, après `run_turn`, parser ligne par ligne et :
- `RateLimit` → mettre à jour `st.codex_rate_limits`
- `Notification` / `TurnFailed` / etc. → mettre à jour `session.last_event` et `session.last_message`
- `TokenUsage` → mettre à jour les compteurs (avec delta)

**Critère d'acceptation :** Un event `RateLimit` dans le stdout de l'agent est parsé et reflété dans l'état orchestrateur.

---

#### Tâche 0.4 : Rendre le prompt de continuation configurable

**Fichiers :**
- Modifier : `src/config/typed.rs`
- Modifier : `src/orchestrator/tick.rs`

**Détails :**
Ajouter une config optionnelle dans le front matter :

```yaml
agent:
  continuation_prompt: |
    Continue le travail sur la tâche actuelle. Passe en revue l'historique et avance sur la prochaine étape.
```

```rust
// ServiceConfig
pub fn continuation_prompt(&self) -> String {
    self.agent()
        .and_then(|m| resolver::get_string(m, "continuation_prompt"))
        .unwrap_or_else(|| "Continue working on the current task. Review the conversation history and proceed with the next step.".into())
}
```

Dans `run_worker` :
```rust
let prompt = if turn_number == 1 {
    build_prompt(config, &issue, attempt)?
} else {
    config.continuation_prompt()
};
```

**Critère d'acceptation :** Le prompt de tour 2+ peut être customisé dans `WORKFLOW.md`.

---

#### Tâche 0.5 : Tracker les statuts `RunAttempt` pendant le worker

**Fichiers :**
- Modifier : `src/orchestrator/tick.rs`
- Modifier : `src/tracker/model.rs`

**Détails :**
Créer et mettre à jour un `RunAttempt` au fil de l'eau dans `run_worker` :

```rust
let mut attempt_record = RunAttempt {
    issue_id: issue.id.clone(),
    issue_identifier: issue.identifier.clone(),
    attempt,
    workspace_path: workspace.path.clone(),
    started_at: Utc::now(),
    status: AttemptStatus::PreparingWorkspace,
    error: None,
};

// Après chaque étape :
attempt_record.status = AttemptStatus::BuildingPrompt;
// ... log / stocker dans l'état si besoin

// En cas d'erreur :
attempt_record.status = AttemptStatus::Failed;
attempt_record.error = Some(e.to_string());
```

Pour ne pas alourdir l'état en mémoire, on peut logger ces transitions avec `tracing::info!(status = ?attempt_record.status, ...)` plutôt que de les stocker durablement.

**Critère d'acceptation :** Les logs du worker montrent les transitions de statut : `PreparingWorkspace` → `BuildingPrompt` → `LaunchingAgentProcess` → `StreamingTurn` → `Finishing`.

---

#### Tâche 0.6 : Aligner les defaults config avec le SPEC

**Fichiers :**
- Modifier : `src/config/typed.rs`

**Détails :**
| Champ | Changement |
|---|---|
| `tracker_endpoint` | Quand `kind == "linear"` → `https://api.linear.app/graphql`. Garder GitHub pour `kind == "github"`. |
| `codex_command` | Default `"codex app-server"` au lieu de `"opencode run"`. Le `WORKFLOW.md` de l'utilisateur peut override. |

**Note :** Ce changement est **rupteur** pour l'installation actuelle (qui utilise `opencode run`). Il faut mettre à jour le `WORKFLOW.md` du repo pour préserver le comportement actuel :
```yaml
codex:
  command: opencode run
```

**Critère d'acceptation :** `cargo test` passe après mise à jour du `WORKFLOW.md` exemple.

---

#### Tâche 0.7 : Tests core manquants

**Fichiers :**
- Créer/Modifier : `tests/integration_test.rs`

**Détails :**
Ajouter les tests suivants :

1. **Dispatch sort order** : 3 issues avec priorities différentes → vérifier l'ordre de dispatch.
2. **Reconciliation terminal** : simuler un running entry + tracker retournant terminal → vérifier que le worker est stoppé et le workspace nettoyé.
3. **Stall detection** : simuler un running entry avec `last_timestamp` ancien → vérifier que `reconcile()` le termine.
4. **Retry backoff** : vérifier la formule `min(10000 * 2^(attempt-1), max_backoff)`.
5. **Strict Liquid** : template avec variable inconnue → doit échouer.
6. **Token delta** : deux updates avec les mêmes totaux → le total global n'augmente pas.

**Critère d'acceptation :** `cargo test` couvre ≥80% des cas critiques listés dans SPEC §17.1–17.4.

### 3.3 Critères d'acceptation du Workstream 0

- [ ] Le render Liquid échoue sur les variables inconnues (strict mode).
- [ ] Le token accounting utilise des deltas (pas de double comptage).
- [ ] Les rate limits sont extraits des events agent et stockés dans l'état.
- [ ] Le prompt de continuation est configurable via `WORKFLOW.md`.
- [ ] Le worker logge les transitions de statut `AttemptStatus`.
- [ ] Les defaults config sont alignés avec SPEC (ou documentés comme déviations explicites).
- [ ] Les tests couvrent dispatch sort, reconciliation, stall, backoff, strict Liquid, token delta.
- [ ] `cargo test` passe à 100%.
- [ ] `cargo clippy` passe sans warning nouveau.

---

## 4. Workstream 1 — Backend Daytona (Terminer)

### 3.1 Analyse de l'état actuel

**Fichier :** `src/agent/backend/daytona.rs` (326 lignes)

**Ce qui marche :**
- Création de sandbox Daytona via REST API
- Persistance de l'ID sandbox sur disque local
- Exécution de commande via l'API execute de Daytona
- Parsing du résultat JSON OpenCode (même logique que local)

**Ce qui ne marche pas / manque :**
1. **Pas de protocole app-server** : on exécute `opencode run` en one-shot. Pour Codex, il faudrait lancer `codex app-server`, lui parler en JSON-RPC over stdio, créer une thread, démarrer des turns.
2. **Pas de gestion de session/thread/turn** : le `session_id` passé à `run_turn` est injecté dans la CLI mais pas utilisé pour reprendre une conversation dans le sandbox.
3. **Pas de streaming events** : l'API execute de Daytona retourne un résultat final. On ne reçoit pas les events intermédiaires (token usage, approvals, etc.) pendant l'exécution.
4. **Pas de cleanup sandbox** : les sandboxes sont créés mais jamais supprimés.
5. **Workspace path mismatch** : on passe le chemin local du workspace alors que dans le sandbox Daytona, le workspace est probablement monté à un chemin différent (ex: `/workspace`).
6. **Pas de health check / start sandbox** : le sandbox est créé mais on ne vérifie pas qu'il est bien `started` avant d'exécuter.

### 3.2 Architecture cible

Le backend Daytona doit gérer un **sandbox lifecycle** complet :

```
run_turn(issue, prompt, session_id, workspace_path)
  ├── validate_inside_root(workspace_path)
  ├── read_sandbox_id(workspace_path) → existant ?
  │     ├── OUI → vérifier état du sandbox (GET /api/sandbox/{id})
  │     │         ├── Si stopped/unknown → recréer
  │     │         └── Si running → réutiliser
  │     └── NON → create_sandbox() + write_sandbox_id()
  ├── S'assurer que le sandbox est started
  ├── Si pas de session_id → start_new_session(sandbox_id, prompt, issue)
  │     └── POST process/execute : codex app-server start-session ...
  │         └── Récupérer thread_id
  ├── Si session_id → continue_turn(sandbox_id, thread_id, prompt)
  │     └── POST process/execute : codex app-server turn ...
  ├── Parser le résultat pour extraire events + tokens
  └── Retourner TurnResult
```

**Contrainte technique majeure :** L'API execute de Daytona est request/response. Elle ne supporte pas le streaming stdio temps réel. Pour obtenir les events en temps réel, il faut soit :
- **Option A** : Exécuter `codex app-server` comme daemon dans le sandbox, exposer un port, et parler via HTTP/WebSocket au sandbox.
- **Option B** : Utiliser l'API execute pour lancer `codex app-server` en background, rediriger stdout vers un fichier, puis "tail" ce fichier via des commands execute successives.
- **Option C** : Utiliser le mode one-shot (`opencode run`) pour chaque turn, en passant `--session <thread_id>` pour la continuité. C'est l'approche actuelle mais enrichie.

**Recommandation (plus pragmatique pour Daytona) :**

Puisque Daytona est un environnement isolé où l'agent s'exécute, et que l'API execute est le seul canal, l'**Option C améliorée** est la plus réaliste :
- Chaque `run_turn` fait un `execute_command` avec `opencode run` (ou `codex` CLI).
- Le `--session` est utilisé pour la continuité.
- Pour le streaming, on parse le stdout complet retourné par l'API execute.
- On ajoute la gestion du lifecycle sandbox (start, health, cleanup).

Cependant, le SPEC.md section 10.2 parle de `codex app-server`. Si l'utilisateur veut être conforme SPEC, il faut implémenter le protocole app-server. Mais dans le contexte Daytona, cela nécessite de lancer l'app-server comme processus persistant dans le sandbox et de communiquer avec.

**Décision architecturale recommandée :**

Le backend Daytona doit implémenter deux modes (sélectionnable via config) :
1. **Mode "one-shot" (actuel, à améliorer)** : Compatible `opencode run`. Usage : exécution simple via CLI.
2. **Mode "app-server" (à implémenter)** : Lance `codex app-server` en daemon dans le sandbox, communique via un port forward ou fichier.

Pour ce projet, on implémente le **mode one-shot amélioré** car c'est ce qui correspond au toolchain actuel (OpenCode). Les améliorations sont :
- Gestion complète du lifecycle sandbox (create → start → health → delete).
- Support `--session` pour les continuation turns.
- Cleanup des sandboxes terminés.
- Synchronisation workspace local → sandbox (git clone dans le sandbox si nouveau).
- Meilleure gestion des erreurs réseau et retry.

### 3.3 Découpage des tâches

#### Tâche 1.1 : Refactor DaytonaConfig pour supporter les modes

**Fichiers :**
- Modifier : `src/config/typed.rs`
- Modifier : `src/agent/backend/daytona.rs`

**Détails :**
Ajouter un champ `mode` dans la config Daytona (défaut : `"oneshot"`).

```rust
// Dans DaytonaConfig
pub mode: DaytonaMode, // Oneshot | AppServer

pub enum DaytonaMode {
    Oneshot,
    AppServer,
}
```

Ajouter les getters dans `ServiceConfig` :
```rust
pub fn daytona_mode(&self) -> String {
    self.daytona()
        .and_then(|m| resolver::get_string(m, "mode"))
        .unwrap_or_else(|| "oneshot".to_string())
}
```

#### Tâche 1.2 : Implémenter le lifecycle complet sandbox

**Fichiers :**
- Modifier : `src/agent/backend/daytona.rs`

**Détails :**
Ajouter les méthodes manquantes :

```rust
async fn start_sandbox(&self, sandbox_id: &str) -> Result<(), SympheoError>
async fn get_sandbox_state(&self, sandbox_id: &str) -> Result<String, SympheoError>
async fn ensure_sandbox_running(&self, workspace_path: &Path) -> Result<String, SympheoError>
```

La méthode `ensure_sandbox_running` doit :
1. Lire l'ID sandbox depuis le fichier `.daytona_sandbox_id`
2. Si existe → `get_sandbox_state` → si `"running"` retourner l'ID, sinon `start_sandbox`
3. Si n'existe pas → `create_sandbox` → `write_sandbox_id` → retourner l'ID

**API Daytona à utiliser :**
- GET `{api_url}/api/sandbox/{id}` — pour l'état
- POST `{api_url}/api/sandbox/{id}/start` — pour démarrer
- DELETE `{api_url}/api/sandbox/{id}` — pour supprimer (déjà implémenté)

#### Tâche 1.3 : Synchronisation workspace vers sandbox

**Fichiers :**
- Modifier : `src/agent/backend/daytona.rs`

**Détails :**
Avant de lancer un turn dans un nouveau sandbox, cloner le repo dans le sandbox.

L'API execute permet d'exécuter des commandes. Pour un nouveau workspace :
1. Vérifier si `/workspace` contient déjà un repo (via `ls -la /workspace`)
2. Si vide → exécuter `git clone <repo_url> /workspace` dans le sandbox
3. Le repo URL peut être passé via la config `daytona.repo_url` ou déduit des hooks.

Alternative plus simple : utiliser le volume workspace de Daytona si disponible.

**Note :** Cette étape est nécessaire car le workspace local et le sandbox Daytona sont deux environnements distincts. Le `workspace_manager` crée un dossier local mais Daytona a son propre filesystem.

#### Tâche 1.4 : Gestion des continuation turns avec session_id

**Fichiers :**
- Modifier : `src/agent/backend/daytona.rs`

**Détails :**
Le code actuel injecte `--session <session_id>` dans la commande si fourni. Vérifier que cette logique fonctionne correctement avec OpenCode.

La commande construite actuellement :
```bash
opencode run "<prompt>" --format json --dir <workspace> --dangerously-skip-permissions [--session <sid>]
```

**Améliorations :**
- Ajouter un timeout explicite pour la commande execute.
- Gérer le cas où le sandbox est supprimé entre deux turns (recréer + re-cloner).
- Logger clairement chaque étape (create/start/execute).

#### Tâche 1.5 : Cleanup automatique des sandboxes

**Fichiers :**
- Modifier : `src/agent/backend/daytona.rs`
- Modifier : `src/workspace/manager.rs` (optionnel)

**Détails :**
Implémenter le cleanup dans deux cas :
1. **Après run success + issue terminal** : dans `run_worker`, après la boucle de turns, si l'issue est terminée, appeler `delete_sandbox`.
2. **Startup cleanup** : dans `main.rs`, lors du cleanup des workspaces terminaux, aussi supprimer les sandboxes associés.

Pour le cas 2, il faut pouvoir lister les sandboxes. Alternative : lors du `remove_workspace`, lire `.daytona_sandbox_id` et appeler `delete_sandbox`.

#### Tâche 1.6 : Gestion robuste des erreurs Daytona

**Fichiers :**
- Modifier : `src/agent/backend/daytona.rs`
- Modifier : `src/error.rs` (si besoin d'erreurs spécifiques)

**Détails :**
Ajouter une stratégie de retry pour les erreurs transitoires de l'API Daytona (5xx, timeout réseau).

```rust
// Exemple de retry avec backoff simple
async fn create_sandbox_with_retry(&self, retries: u32) -> Result<DaytonaSandbox, SympheoError>
```

Gérer spécifiquement :
- Sandbox déjà existant (conflit)
- Sandbox en état "error" (recréer)
- Rate limiting (429)

#### Tâche 1.7 : Tests unitaires et d'intégration Daytona

**Fichiers :**
- Créer : `tests/daytona_backend_test.rs`

**Détails :**
- Mock du client reqwest pour tester la logique sans vrai Daytona.
- Tester le cycle create → start → execute → delete.
- Tester la reprise de session (lecture `.daytona_sandbox_id`).
- Tester les cas d'erreur (sandbox non démarrable, API down).

### 3.4 Critères d'acceptation

- [ ] Un sandbox Daytona est créé, démarré, et réutilisé pour tous les turns d'une même issue.
- [ ] Le sandbox est supprimé lorsque l'issue passe en état terminal.
- [ ] La commande agent supporte `--session` pour les continuation turns.
- [ ] Le workspace est correctement initialisé dans le sandbox (repo cloné).
- [ ] Les erreurs API Daytona sont loguées et gérées avec retry.
- [ ] Les tests passent (`cargo test`).
- [ ] Pas de régression sur le backend local.

---

## 5. Workstream 2 — Dashboard HTML + Pico CSS

### 4.1 Analyse de l'état actuel

**Fichier :** `src/server/mod.rs` (120 lignes)

**Dashboard actuel (`/`) :**
```rust
async fn dashboard(State(state): State<SharedState>) -> (StatusCode, String) {
    let st = state.read().await;
    let html = format!(
        "<html><body><h1>Symphony</h1><p>Running: {}</p><p>Retrying: {}</p></body></html>",
        st.running.len(),
        st.retry_attempts.len()
    );
    (StatusCode::OK, html)
}
```

**API actuelle :**
- `GET /api/v1/state` — retourne counts, running[], retrying[], codex_totals, rate_limits
- `GET /api/v1/:issue_identifier` — retourne info basique sur une issue running
- `POST /api/v1/refresh` — retourne un dummy JSON (pas connecté à l'orchestrateur)

**Écarts SPEC (section 13.7) :**
- Le dashboard ne montre pas : les sessions actives détaillées, les retry delays, la consommation tokens, les recent events, les health indicators.
- L'API `/api/v1/:issue_identifier` manque : workspace path, attempts, logs, recent_events, last_error.
- Le refresh endpoint ne déclenche pas réellement un poll.

### 4.2 Architecture cible

**Design system :** [Pico CSS](https://picocss.com) — framework CSS classless, léger, responsive. Inclusion via CDN.

**Pages :**
1. **Dashboard principal (`/`)** :
   - Header avec titre et statut global
   - Cards : Running, Retrying, Total Tokens, Runtime
   - Table des sessions actives (identifier, state, session_id, turn_count, started_at, last_event, tokens)
   - Table des retries (identifier, attempt, due_in, error)
   - Barre d'état / health (dernier tick, erreurs)
   - Auto-refresh toutes les 5 secondes

2. **API JSON** (enrichie pour alimenter le dashboard et potentiellement un futur frontend SPA)

### 4.3 Découpage des tâches

#### Tâche 2.1 : Enrichir l'API JSON avec les champs manquants

**Fichiers :**
- Modifier : `src/server/mod.rs`
- Modifier : `src/orchestrator/state.rs` (ajouter `last_tick_at` si utile)

**Détails :**

**`/api/v1/state` — ajouter dans chaque running entry :**
```json
{
  "last_event": "turn_completed",
  "last_message": "Working on tests...",
  "last_event_at": "2026-05-08T10:00:00Z",
  "tokens": {
    "input_tokens": 1200,
    "output_tokens": 800,
    "total_tokens": 2000
  }
}
```

**`/api/v1/:issue_identifier` — enrichir pour retourner :**
```json
{
  "issue_identifier": "SYM-42",
  "status": "running",
  "workspace": { "path": "/tmp/sympheo_workspaces/SYM-42" },
  "attempts": { "restart_count": 0, "current_retry_attempt": null },
  "running": { /* détails session */ },
  "retry": null,
  "recent_events": [ /* 10 derniers events */ ],
  "last_error": null
}
```

**`/api/v1/refresh` — connecter à l'orchestrateur :**
- Nécessite d'exposer une méthode `trigger_refresh()` sur `Orchestrator`.
- Utiliser un `tokio::sync::Notify` ou un channel pour signaler au main loop de faire un tick immédiat.

#### Tâche 2.2 : Implémenter le dashboard HTML avec Pico CSS

**Fichiers :**
- Modifier : `src/server/mod.rs`

**Détails :**
Remplacer la fonction `dashboard` par un template HTML complet avec Pico CSS.

```html
<!DOCTYPE html>
<html lang="en" data-theme="dark">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Sympheo Dashboard</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/@picocss/pico@2/css/pico.min.css">
  <style>
    /* custom styles minimes */
    .status-dot { width: 12px; height: 12px; border-radius: 50%; display: inline-block; }
    .status-running { background: var(--pico-color-green-500); }
    .status-retrying { background: var(--pico-color-amber-500); }
    .status-error { background: var(--pico-color-red-500); }
  </style>
</head>
<body>
  <main class="container">
    <h1>🔧 Sympheo Orchestrator</h1>
    <div class="grid">
      <article><h3>Running</h3><p class="display">{}</p></article>
      <article><h3>Retrying</h3><p class="display">{}</p></article>
      <article><h3>Tokens</h3><p class="display">{}</p></article>
      <article><h3>Runtime</h3><p class="display">{}s</p></article>
    </div>
    <h2>Active Sessions</h2>
    <table>
      <thead><tr><th>Issue</th><th>State</th><th>Session</th><th>Turns</th><th>Started</th><th>Last Event</th></tr></thead>
      <tbody>...</tbody>
    </table>
    <h2>Retry Queue</h2>
    ...
  </main>
  <script>
    setInterval(() => location.reload(), 5000);
  </script>
</body>
</html>
```

**Approche recommandée :** Utiliser une **fonction helper** `render_dashboard(state: &OrchestratorState) -> String` qui génère le HTML. Cela garde la logique dans le fichier Rust sans dépendre de moteur de template externe.

Alternative : si le fichier devient trop gros, créer `src/server/dashboard.rs` avec la fonction de rendering.

#### Tâche 2.3 : Connecter le refresh endpoint

**Fichiers :**
- Modifier : `src/server/mod.rs`
- Modifier : `src/orchestrator/tick.rs`
- Modifier : `src/main.rs`

**Détails :**
1. Ajouter un `tokio::sync::mpsc::Sender<()>` dans `Orchestrator` (ou un `Notify`).
2. Le endpoint `POST /api/v1/refresh` envoie un signal sur ce channel.
3. Dans `main.rs`, la boucle principale écoute soit l'interval, soit le signal refresh.

```rust
// Dans Orchestrator
pub refresh_tx: tokio::sync::mpsc::Sender<()>,

// Dans main loop
loop {
    tokio::select! {
        _ = interval.tick() => {},
        _ = refresh_rx.recv() => { info!("manual refresh triggered"); },
    }
    orchestrator.tick().await;
    orchestrator.process_retries().await;
}
```

#### Tâche 2.4 : Tests du serveur

**Fichiers :**
- Créer : `tests/server_test.rs`

**Détails :**
- Test que `/` retourne du HTML avec les balises attendues.
- Test que `/api/v1/state` retourne le JSON attendu.
- Test que `/api/v1/refresh` retourne 202 et déclenche bien un signal.

### 4.4 Critères d'acceptation

- [ ] Le dashboard affiche les sessions actives dans un tableau HTML stylé avec Pico CSS.
- [ ] Le dashboard affiche la file de retry.
- [ ] Le dashboard affiche les métriques agrégées (tokens, runtime).
- [ ] Auto-refresh toutes les 5 secondes.
- [ ] L'API JSON est enrichie avec last_event, last_message, tokens par session.
- [ ] `POST /api/v1/refresh` déclenche un poll immédiat.
- [ ] Le design est responsive et utilise Pico CSS (inclusion CDN).
- [ ] Tests passent (`cargo test`).

---

## 6. Workstream 3 — Skill Mapping par Colonne Workflow

### 5.1 Analyse du besoin

**Contexte :** Le `WORKFLOW.md` définit un prompt template global. Cependant, les instructions pour un agent devraient varier selon la **colonne/state** de l'issue dans le projet tracker :
- **Todo** : "Analyse l'issue, propose une solution, crée une branche."
- **In Progress** : "Implémente la solution, écris les tests, fais passer le CI."
- **Review** : "Relis le code, corrige les retours, met à jour la documentation."
- **Done** : (généralement pas dispatché)

**Concept de Skill :** Un fichier texte/markdown contenant des instructions spécifiques à une colonne. Stocké dans un dossier `skills/` à côté de `WORKFLOW.md`.

**Mapping :** La config WORKFLOW.md déclare quelle skill utiliser pour chaque état tracker.

### 5.2 Architecture cible

**Structure de dossier :**
```
project/
├── WORKFLOW.md
└── skills/
    ├── todo.md
    ├── in_progress.md
    └── review.md
```

**Extension du WORKFLOW.md front matter :**
```yaml
skills:
  mapping:
    todo: skills/todo.md
    in progress: skills/in_progress.md
    review: skills/review.md
  default: skills/default.md   # fallback si pas de mapping
```

**Intégration dans le prompt :**
Le prompt final pour un agent est composé de :
1. **Prompt template principal** (corps du WORKFLOW.md)
2. **Skill instructions** (contenu du fichier skill correspondant à l'état actuel de l'issue)
3. **Variables** : `issue`, `attempt`, `skill` (nom de la skill)

Ordre de composition recommandé :
```
{skill_instructions}

---

{prompt_template}
```

Ou injecté via une nouvelle variable Liquid `{{ skill_instructions }}` dans le template.

### 5.3 Découpage des tâches

#### Tâche 5.1 : Ajouter le modèle de données Skill

**Fichiers :**
- Créer : `src/skills/mod.rs`
- Créer : `src/skills/loader.rs`
- Créer : `src/skills/mapper.rs`
- Modifier : `src/lib.rs` (ajouter `pub mod skills;`)

**Détails :**

**`src/skills/mod.rs` :**
```rust
pub mod loader;
pub mod mapper;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct SkillMapping {
    pub by_state: HashMap<String, String>, // state_lowercase -> path relative
    pub default: Option<String>,
}
```

**`src/skills/loader.rs` :**
```rust
pub fn load_skill(path: &Path) -> Result<Skill, SympheoError>
pub fn load_skills(mapping: &SkillMapping, base_dir: &Path) -> Result<HashMap<String, Skill>, SympheoError>
```

**`src/skills/mapper.rs` :**
```rust
impl SkillMapping {
    pub fn from_config(config: &ServiceConfig) -> Self
    pub fn resolve_skill(&self, issue_state: &str) -> Option<&str> // retourne le path
}
```

#### Tâche 5.2 : Étendre ServiceConfig pour la config skills

**Fichiers :**
- Modifier : `src/config/typed.rs`

**Détails :**
Ajouter la section `skills` dans le parsing front matter :

```rust
fn skills(&self) -> Option<&serde_yaml::Mapping> { ... }

pub fn skill_mapping(&self) -> SkillMapping { ... }
```

La config YAML attendue :
```yaml
skills:
  mapping:
    todo: ./skills/todo.md
    in progress: ./skills/in_progress.md
  default: ./skills/default.md
```

#### Tâche 5.3 : Intégrer le skill loader dans le workflow loader

**Fichiers :**
- Modifier : `src/workflow/loader.rs`
- Modifier : `src/main.rs`

**Détails :**
Lors du chargement du workflow, charger aussi les skills.

```rust
// Dans main.rs
let skill_mapping = config.skill_mapping();
let skills = load_skills(&skill_mapping, &workflow_dir)?;
```

Stocker les skills dans `Orchestrator` pour qu'ils soient accessibles pendant le worker run.

#### Tâche 5.4 : Modifier le rendu du prompt pour injecter les skills

**Fichiers :**
- Modifier : `src/orchestrator/tick.rs`
- Modifier : `src/agent/runner.rs` (si besoin de passer les skills)

**Détails :**
Modifier `build_prompt` pour accepter les skill instructions :

```rust
fn build_prompt(
    config: &ServiceConfig,
    issue: &Issue,
    attempt: Option<u32>,
    skill_instructions: Option<&str>,
) -> Result<String, SympheoError> {
    let template_str = ...;
    let skill_part = skill_instructions.unwrap_or("");
    
    // Option A : préfixer
    let full_prompt = if skill_part.is_empty() {
        template_str.clone()
    } else {
        format!("{}\n\n---\n\n{}", skill_part, template_str)
    };
    
    // Puis render avec Liquid
    // Ajouter la variable `skill_instructions` au contexte Liquid
    ...
}
```

Dans `run_worker`, avant de construire le prompt, déterminer la skill à utiliser :
```rust
let skill_content = skills.get(&issue.state.to_lowercase())
    .or_else(|| skills.get("default"))
    .map(|s| s.content.as_str());
```

#### Tâche 5.5 : Gestion du reload dynamique des skills

**Fichiers :**
- Modifier : `src/main.rs`

**Détails :**
Le watcher de `WORKFLOW.md` doit aussi surveiller le dossier `skills/` (si configuré).

Alternative simple : lors du reload du workflow, recharger aussi les skills.

```rust
// Dans le watcher loop
let new_skills = load_skills(&new_config.skill_mapping(), &workflow_dir)?;
// Stocker dans l'orchestrateur (nécessite d'ajouter un champ skills à Orchestrator)
```

#### Tâche 5.6 : Tests

**Fichiers :**
- Créer : `tests/skills_test.rs`
- Créer : `tests/fixtures/skills/todo.md`
- Créer : `tests/fixtures/skills/in_progress.md`

**Détails :**
- Test parsing de `SkillMapping` depuis YAML.
- Test chargement de skill depuis fichier.
- Test rendu prompt avec skill injecté.
- Test fallback default skill.
- Test skill manquant (pas d'erreur, juste pas d'instructions supplémentaires).

### 5.4 Critères d'acceptation

- [ ] Le dossier `skills/` peut contenir des fichiers `.md` par état tracker.
- [ ] Le `WORKFLOW.md` supporte une section `skills.mapping` pour mapper états → fichiers.
- [ ] Le prompt envoyé à l'agent inclut les instructions de la skill correspondante.
- [ ] Si aucune skill n'est mappée pour un état, le prompt template standard est utilisé.
- [ ] Le reload du workflow recharge aussi les skills.
- [ ] Les skills sont accessibles comme variable Liquid `{{ skill_instructions }}`.
- [ ] Tests passent (`cargo test`).

---

## 7. Considérations Transverses

### 6.1 Tests

La couverture de test actuelle est minime (`tests/integration_test.rs`, ~150 lignes). Chaque workstream doit ajouter ses tests.

**Stratégie de test :**
- **Unit tests** : dans chaque module avec `#[cfg(test)]`.
- **Integration tests** : dans `tests/` pour les scénarios end-to-end.
- **Mocks** : Utiliser des traits (`IssueTracker`, `AgentBackend`) pour mocker les dépendances externes.

### 6.2 Documentation

- Mettre à jour `README.md` avec les nouvelles features (Daytona mode, dashboard, skills).
- Documenter le format des fichiers skill dans le README.
- Mettre à jour le `WORKFLOW.md` exemple pour montrer la section `skills`.

### 6.3 Compatibilité et non-régression

- Le backend **local** ne doit pas être modifié (ou très peu).
- Le dashboard doit rester **optionnel** (démarré seulement si `--port`).
- Les skills doivent être **optionnels** (pas de skill = comportement actuel).
- La config Daytona doit avoir des **valeurs par défaut** raisonnables.

### 6.4 Sécurité

- Ne pas loguer les `api_key` Daytona ou tracker.
- Valider les chemins de skill (restent dans le répertoire du projet).
- Le dashboard HTTP bind `127.0.0.1` par défaut (déjà le cas).

---

## 8. Dépendances et Ordre d'exécution

### 8.1 Ordre recommandé pour les agents IA

```
Phase 0 — Fondations & Conformité (BLOQUANT, à faire en premier)
└── Workstream 0 : Core Compliance & SPEC Conformance
    └── (Tâches 0.1 → 0.2 → 0.3 → 0.4 → 0.5 → 0.6 → 0.7)

Phase 1 — Features parallélisables (indépendantes entre elles)
├── Workstream 2 : Dashboard HTML + Pico CSS
│   └── (Tâches 2.1 → 2.2 → 2.3 → 2.4)
│
└── Workstream 3 : Skill Mapping
    └── (Tâches 5.1 → 5.2 → 5.3 → 5.4 → 5.5 → 5.6)

Phase 2 — Backend complexe (dépend de la stabilité du core)
└── Workstream 1 : Backend Daytona
    └── (Tâches 1.1 → 1.2 → 1.3 → 1.4 → 1.5 → 1.6 → 1.7)
```

**Justification :**
- **Workstream 0 est bloquant** : il corrige des bugs de comptage, de conformité et d'observabilité. Si on construit Daytona ou les skills par-dessus un core non conforme, on réplique les erreurs.
- Le **dashboard** et les **skills** sont des features relativement isolées qui touchent des modules différents. Ils peuvent être développés en parallèle.
- Le **backend Daytona** est le plus complexe et risqué. Il faut une base stable (config, orchestrator, parser events) avant de l'attaquer.

### 8.2 Dépendances entre tâches

| Tâche | Dépend de | Justification |
|---|---|---|
| 0.2 (token delta) | 0.3 (events parsés) | Besoin des events `TokenUsage` pour mettre à jour les compteurs correctement |
| 0.4 (continuation config) | 0.6 (defaults alignés) | Le default du prompt doit être cohérent avec la config |
| 1.2 (sandbox lifecycle) | 1.1 (config mode) | Besoin du mode pour savoir quel lifecycle appliquer |
| 1.4 (continuation turns) | 1.2 (sandbox lifecycle) | Besoin d'un sandbox running stable |
| 1.5 (cleanup) | 1.2 (sandbox lifecycle) | Besoin de delete_sandbox appelable |
| 2.3 (refresh connecté) | 2.1 (API enrichie) | Le refresh sert à mettre à jour les données |
| 5.4 (prompt avec skill) | 5.1, 5.2, 5.3 | Besoin du modèle + config + loader |
| 5.5 (reload skills) | 5.4 | Besoin que les skills soient intégrés pour les recharger |

### 8.3 Estimation (ordre de grandeur)

| Workstream | Tâches | Complexité | Estimation |
|---|---|---|---|
| **0 — Core Compliance** | 7 | Moyenne/Élevée | 2-3 jours |
| 2 — Dashboard | 4 | Moyenne | 1-2 jours |
| 3 — Skills | 6 | Faible/Moyenne | 1-2 jours |
| 1 — Daytona | 7 | Élevée | 3-5 jours |

**Total estimé :** 7-12 jours de développement agentique (hors review, test intégration, documentation).

---

## 8. Résumé des fichiers à créer / modifier

### Fichiers à créer
```
src/skills/mod.rs
src/skills/loader.rs
src/skills/mapper.rs
tests/daytona_backend_test.rs
tests/server_test.rs
tests/skills_test.rs
tests/fixtures/skills/todo.md
tests/fixtures/skills/in_progress.md
tests/liquid_strict_test.rs        # Workstream 0 — strict mode Liquid
tests/token_accounting_test.rs     # Workstream 0 — delta accounting
tests/agent_events_test.rs         # Workstream 0 — events parser
```

### Fichiers à modifier
```
src/main.rs                  # Watcher skills, refresh signal, startup cleanup sandboxes
src/lib.rs                   # Ajout mod skills
src/error.rs                 # Erreurs Daytona/Skills si nécessaire
src/config/typed.rs          # Daytona mode, skill mapping, defaults SPEC
src/config/resolver.rs       # (peut-être) helper list/map
src/agent/backend/daytona.rs # Lifecycle sandbox, sync, cleanup, retry
src/agent/backend/local.rs   # (peut-être) ajout trait method si besoin
src/agent/parser.rs          # Workstream 0 — events agent complets
src/agent/runner.rs          # Passage skills au backend
src/orchestrator/tick.rs     # Build prompt avec skills, refresh signal, token delta, strict Liquid, attempt status
src/orchestrator/state.rs    # (peut-être) ajout last_tick
src/server/mod.rs            # Dashboard HTML Pico, API enrichie, refresh connecté
src/tracker/model.rs         # Workstream 0 — RunAttempt / LiveSession updates
src/workspace/manager.rs     # (peut-être) helper pour daytona cleanup
README.md                    # Documentation nouvelles features
WORKFLOW.md                  # Exemple section skills
```

---

## 9. Checklist de validation finale

Avant de considérer le projet terminé :

- [ ] `cargo build` passe sans warning.
- [ ] `cargo test` passe à 100%.
- [ ] `cargo clippy` passe.
- [ ] Le backend local fonctionne toujours (test de non-régression).
- [ ] **Workstream 0** : Liquid strict mode actif, token accounting delta-based, events agent parsés.
- [ ] Le dashboard est accessible et affiche les données.
- [ ] Les skills sont chargés et injectés dans les prompts.
- [ ] Le backend Daytona crée, réutilise, et nettoie les sandboxes.
- [ ] La documentation (`README.md`, `WORKFLOW.md`) est à jour.

---

*Plan généré le 2026-05-08. Ce document est la source de vérité pour le découpage et l'ordonnancement des travaux d'implémentation.*
