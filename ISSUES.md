# Plan de Remediation — Test E2E Sympheo (Issue #118)

> **Date** : 2026-05-08  
> **Contexte** : Test end-to-end d'un ticket créé dans le projet GitHub #2 (`supergeoff/sympheo`).  
> **Méthode** : `cargo run -- --port 9090` avec tracker GitHub actif et backend `opencode` local.

---

## 1. Résumé exécutif

L'orchestrateur Sympheo présente une **architecture fondamentalement solide** (streaming d'événements, gestion des tokens par delta, skills mappés par colonne, isolation des workspaces). Cependant, **quatre bugs bloquants** et plusieurs imperfections empêchent un cycle E2E complet de se dérouler de manière fiable :

| # | Problème | Criticité | Empêche le workflow E2E ? |
|---|----------|-----------|---------------------------|
| 1 | Port serveur ignoré dans `WORKFLOW.md` | 🔴 Critique | Non, mais UX cassée |
| 2 | Polling interval reset (ticks ultra-rapides) | 🔴 Critique | Oui (saturation) |
| 3 | Fuites de processus `opencode` (stall detection) | 🔴 Critique | Oui (tokens gaspillés, slots bloqués) |
| 4 | Échec systématique du skill `doc` / shell escaping | 🔴 Critique | Oui (ticket #108 bloqué en boucle) |
| 5 | Absence de skill `todo` | 🟡 Majeur | Oui (ticket #118 ne démarre pas) |
| 6 | `total_tokens` toujours à 0 dans l'API | 🟡 Majeur | Non, mais monitoring aveugle |
| 7 | Concurrence polluée par les retries agressifs | 🟡 Majeur | Oui (latence sur les nouveaux tickets) |

---

## 2. Problèmes détaillés

### ISSUE-001 — Port serveur ignoré dans `WORKFLOW.md`

- **Statut** : 🔴 Ouvert
- **Criticité** : Critique
- **Catégorie** : Configuration / CLI

#### Description
La clé `server.port: 9090` définie dans le YAML front matter de `WORKFLOW.md` n'est **jamais lue** au démarrage. Le serveur Axum ne démarre que si l'argument `--port` est explicitement passé en ligne de commande (`cargo run -- --port 9090`).

#### Impact
- L'utilisateur configure le port dans le fichier de workflow mais le dashboard reste inaccessible.
- Confusion entre la configuration déclarative et les arguments CLI.

#### Root Cause
Dans `src/main.rs:127` :
```rust
if let Some(port) = cli.port {
    tokio::spawn(async move {
        sympheo::server::start_server(port, state_clone).await
    });
}
```
Aucun fallback n'existe sur `config.server_port()`.

#### Remediation
1. Lire `server.port` depuis la config YAML parsed (`ServiceConfig`).
2. Utiliser la logique : `cli.port.or(config.server_port())`.
3. Si un port est résolu, démarrer le serveur.
4. Ajouter un log `INFO` indiquant la source du port (CLI vs config).

#### Fichiers concernés
- `src/main.rs`
- `src/config/typed.rs` (ajouter `server_port()` si absent)

---

### ISSUE-002 — Polling interval reset (ticks ultra-rapides)

- **Statut** : 🔴 Ouvert
- **Criticité** : Critique
- **Catégorie** : Orchestrateur / Main Loop

#### Description
L'orchestrateur effectue des ticks environ **toutes les 1.5 secondes** au lieu des 30 000 ms configurées (`polling.interval_ms: 30000`).

#### Impact
- Saturation CPU de la boucle principale.
- Appels répétés et inutiles à l'API GraphQL GitHub (risque de rate limit).
- Retries exponentiels déclenchés bien plus vite que prévu.
- Consommation excessive de tokens API car les agents sont relancés en boucle.

#### Root Cause
Dans `src/main.rs:185-200` :
```rust
loop {
    tokio::select! {
        _ = interval.tick() => {},
        _ = notify.notified() => { ... },
    }
    let cfg = orchestrator.config.read().await.clone();
    interval = tokio::time::interval(tokio::time::Duration::from_millis(cfg.poll_interval_ms()));
    // ^^^ L'interval est recréé à chaque itération, resettant le timer
}
```
La recréation de `tokio::time::Interval` à chaque loop annule la périodicité.

#### Remediation
1. **Ne pas recréer** l'`interval` dans la boucle.
2. Si le `poll_interval_ms` change (hot reload), utiliser `interval.reset()` ou recréer l'interval **uniquement** quand la valeur a effectivement changé.
3. Ajouter un log de debug indiquant l'interval actuellement actif.

#### Fichiers concernés
- `src/main.rs`

---

### ISSUE-003 — Fuites de processus `opencode` (stall detection)

- **Statut** : 🔴 Ouvert
- **Criticité** : Critique
- **Catégorie** : Agent Backend / Local Backend

#### Description
Lorsqu'un agent dépasse le `stall_timeout_ms` (5 minutes) sans émettre d'événement, l'orchestrateur le déclare "stalled" et le reschedule en retry. Cependant, le **processus `opencode` sous-jacent n'est pas tué de manière fiable**, laissant des processus orphelins consommer des ressources et des tokens.

#### Impact
- **Fuites de processus** : observation de 2 à 3 processus `opencode run` simultanés pour le même ticket (#118).
- **Coût API** : chaque processus orphelin continue d'appeler les LLM en arrière-plan.
- **Pollution des workspaces** : risque de conflits de fichiers si deux agents modifient le même workspace.

#### Root Cause
Dans `src/agent/backend/local.rs:139-151` :
```rust
if read_result.is_err() {
    drop(event_tx);
    let _ = child.kill().await;
    let _ = stderr_handle.abort();
    return Err(SympheoError::AgentTurnTimeout);
}
let _ = child.kill().await;
let _ = stderr_handle.abort();
let _ = timeout(Duration::from_secs(5), child.wait()).await;
```
- `child.kill().await` envoie SIGKILL au processus `bash`, mais `opencode` (et ses éventuels processus enfants `zsh`, `find`, etc.) peut survivre si ce n'est pas le leader du process group.
- Le orchestrateur (`tick.rs`) qui détecte le stall à un niveau supérieur ne semble pas propager le kill au `AgentBackend`.

#### Remediation
1. **Process Group Kill** : lancer `opencode` dans un nouveau process group (`setsid` ou `Command::new("setsid")`) et tuer tout le groupe avec `killpg`.
2. **Cleanup explicite au niveau orchestrateur** : quand un worker est retiré pour "stalled", appeler explicitement `runner.cleanup_workspace()` **et** tuer le processus actif.
3. **Timeout de garde** : ajouter un `tokio::time::timeout` au niveau du `run_turn` entier dans l'orchestrateur, et forcer le `drop` du backend si dépassé.
4. **Zombie reaper** : s'assurer que `child.wait()` est bien appelé après `kill()` (actuellement c'est fait avec un timeout de 5s, mais peut être insuffisant).

#### Fichiers concernés
- `src/agent/backend/local.rs`
- `src/orchestrator/tick.rs`
- `src/agent/runner.rs`

---

### ISSUE-004 — Échec systématique du skill `doc` / shell escaping

- **Statut** : 🔴 Ouvert
- **Criticité** : Critique
- **Catégorie** : Agent Backend / Local Backend

#### Description
Le ticket #108 (colonne **Doc**, skill `skills/doc/SKILL.md`) échoue systématiquement à chaque attempt. Le `stderr` d'opencode affiche son **message d'aide complet** au lieu de traiter le prompt, ce qui indique que la commande shell est mal formée.

#### Impact
- Ticket #108 bloqué indéfiniment en retry (attempt 1→5 observé, puis probablement au-delà).
- Occupation permanente d'un slot de concurrence (`max_concurrent_agents: 5`).
- Le workflow Doc → Done ne peut pas être validé E2E.

#### Root Cause
Hypothèses classées par probabilité :

1. **Shell escaping incomplet** : `shell_escape()` dans `local.rs:169` n'échappe pas les apostrophes (`'`). Le prompt est injecté dans `bash -lc '...'`. Si le skill `doc` (ou le template) contient un `'`, la commande shell est cassée.
2. **Prompt trop long / ARG_MAX** : le skill `doc` fait ~110 lignes + le template ~79 lignes. Bien que la limite Linux soit ~2MB, `opencode` (Yargs) pourrait mal parser un argument positionnel extrêmement long.
3. **Interaction `--session`** : sur retry, `--session <id>` est ajouté. Si l'`id` de session contient des caractères spéciaux, cela pourrait perturber le parsing.

#### Remediation
1. **Renforcer `shell_escape`** : échapper aussi `'` en `\'` et vérifier d'autres caractères spéciaux (`;`, `|`, `&`, `<`, `>`, `(`, `)`, `*`, `?`, `[`, `]`, `\n`).
2. **Alternative : passer le prompt via stdin ou fichier** : au lieu de `opencode run "<prompt>"`, utiliser `echo "$PROMPT" | opencode run --format json --dir ... --dangerously-skip-permissions` ou écrire le prompt dans un fichier temporaire et passer `--file /tmp/prompt.txt`.
3. **Ajouter du logging de la commande exacte** (niveau DEBUG) avant `cmd.spawn()` pour faciliter le diagnostic futur.
4. **Tester unitairement** `local_backend_run_turn` avec le skill `doc` réel en mock pour reproduire l'erreur.

#### Fichiers concernés
- `src/agent/backend/local.rs`
- `skills/doc/SKILL.md` (vérifier les caractères spéciaux)
- `src/orchestrator/tick.rs` (construction du prompt)

---

### ISSUE-005 — Absence de skill `todo`

- **Statut** : 🟡 Ouvert
- **Criticité** : Majeur
- **Catégorie** : Skills / Workflow

#### Description
La colonne `Todo` n'a **aucun skill mappé** dans `WORKFLOW.md` :
```yaml
skills:
  mapping:
    spec: ./skills/spec/SKILL.md
    in progress: ./skills/build/SKILL.md
    review: ./skills/review/SKILL.md
    test: ./skills/test/SKILL.md
    doc: ./skills/doc/SKILL.md
```
L'agent qui reçoit un ticket en `Todo` n'a donc que le template Liquid générique, sans instructions spécifiques sur ce qu'il doit faire (analyser l'issue, produire une spec, déplacer le ticket vers `Spec`).

#### Impact
- Le ticket #118 est resté en `Todo` pendant toute la durée du test.
- L'agent tourne mais n'a pas de "call to action" clair pour avancer le workflow.
- Le pipeline E2E est bloqué à la première étape.

#### Root Cause
Omission dans la configuration des skills.

#### Remediation
1. **Créer `skills/todo/SKILL.md`** avec des instructions claires :
   - Analyser l'issue et le codebase.
   - Produire une spécification technique détaillée (LLD).
   - Déplacer le ticket vers la colonne **Spec** via `gh project item-edit`.
   - Ne PAS écrire de code d'implémentation à ce stade.
2. **Mapper le skill** dans `WORKFLOW.md` :
   ```yaml
   skills:
     mapping:
       todo: ./skills/todo/SKILL.md
       spec: ./skills/spec/SKILL.md
       ...
   ```
3. **Optionnel : skill `default`** : ajouter un skill `default` comme fallback pour toute colonne non mappée, avec des instructions génériques de progression.

#### Fichiers concernés
- `WORKFLOW.md`
- `skills/todo/SKILL.md` (à créer)

---

### ISSUE-006 — `total_tokens` toujours à 0 dans l'API state

- **Statut** : 🟡 Ouvert
- **Criticité** : Majeur
- **Catégorie** : API / Dashboard

#### Description
L'endpoint `/api/v1/state` retourne constamment :
```json
"codex_totals": {
  "input_tokens": 0,
  "output_tokens": 0,
  "total_tokens": 0,
  "seconds_running": ...
}
```
Même lorsque des turns réussis (`step_finish` avec `reason=tool-calls`) sont observés dans les logs.

#### Impact
- Le dashboard affiche "Tokens: 0" en permanence.
- Impossible de monitorer la consommation réelle et le coût API.

#### Root Cause
- L'événement `TokenUsage` n'est peut-être pas émis par `opencode` dans cette version, ou le format JSON diffère du parser attendu.
- Ou bien le `TokenInfo` est contenu dans `step_finish.part.tokens` mais pas émis comme événement `TokenUsage` séparé.

#### Remediation
1. **Vérifier le contrat d'événements** : dans `src/agent/parser.rs`, s'assurer que le parser gère bien le champ `tokens` à l'intérieur de `step_finish`.
2. **Fallback sur `step_finish.tokens`** : si aucun événement `TokenUsage` n'est reçu, extraire les tokens directement du `step_finish` pour mettre à jour les totaux.
3. **Ajouter des logs DEBUG** dans le consumer d'événements (`tick.rs:496-536`) pour tracer ce qui est reçu.

#### Fichiers concernés
- `src/agent/parser.rs`
- `src/orchestrator/tick.rs`
- `src/server/mod.rs` (dashboard HTML)

---

### ISSUE-007 — Concurrence polluée par les retries agressifs

- **Statut** : 🟡 Ouvert
- **Criticité** : Majeur
- **Catégorie** : Orchestrateur / Dispatch

#### Description
Le ticket #108 (en échec perpétuel) est relancé à chaque tick rapide (voir ISSUE-002), occupant un slot de concurrence. Cela retarde le traitement des autres tickets éligibles.

#### Impact
- Latence accrue pour les nouveaux tickets (comme #118).
- Risque de starvation si plusieurs tickets sont en retry.

#### Root Cause
- Combinaison de ISSUE-002 (ticks rapides) et du fait que les retries ne sont pas décomptés séparément des nouveaux dispatches.

#### Remediation
1. **Séparer les limites** : `max_concurrent_agents` devrait idéalement distinguer "nouveaux tickets" et "retries", ou au moins garantir qu'un ticket en retry n'empêche pas le dispatch de nouveaux tickets.
2. **Circuit breaker** : après N échecs consécutifs (ex: 5) sur un même ticket avec la même erreur, le mettre en pause plus longtemps ou le déplacer manuellement vers `Cancelled`.
3. **Corriger ISSUE-002** résoudra déjà en grande partie ce symptôme.

#### Fichiers concernés
- `src/orchestrator/tick.rs`
- `src/config/typed.rs`

---

## 3. Plan d'action priorisé

| Priorité | Issue | Tâche | Estimation |
|----------|-------|-------|------------|
| P0 | ISSUE-002 | Corriger le reset de l'intervalle de polling | ~30 min |
| P0 | ISSUE-003 | Implémenter le process group kill pour opencode | ~1h |
| P0 | ISSUE-004 | Renforcer le shell escaping ou passer le prompt via fichier | ~1h |
| P1 | ISSUE-001 | Lire `server.port` depuis la config en fallback CLI | ~20 min |
| P1 | ISSUE-005 | Créer le skill `todo` et le mapper | ~30 min |
| P1 | ISSUE-006 | Corriger le comptage des tokens (fallback `step_finish`) | ~30 min |
| P2 | ISSUE-007 | Ajouter un circuit breaker / limiter les retries | ~1h |
| P2 | — | Ré-exécuter le test E2E complet avec le ticket #118 | ~2h (dépend des LLM) |

---

## 4. Notes pour le prochain test E2E

Avant de relancer un cycle E2E complet :
1. **Fermer ou déplacer le ticket #108** en `Done` ou `Cancelled` pour ne pas polluer le test.
2. **Vérifier** que `GITHUB_TOKEN` a les scopes `project`, `repo`, et `workflow`.
3. **S'assurer** qu'aucun processus `opencode` orphelin ne tourne (`pkill -f "opencode run"`).
4. **Nettoyer** les workspaces : `rm -rf ~/sympheo_workspaces/*`.
5. **Lancer** avec `cargo run -- --port 9090` (jusqu'à correction d'ISSUE-001).
6. **Surveiller** les logs en temps réel : `tail -f /tmp/sympheo_e2e.log`.
7. **Objectifs de validation** :
   - [ ] Ticket #118 (ou un nouveau) passe de `Todo` → `Spec` en < 10 min.
   - [ ] Une branche est créée dans le workspace.
   - [ ] La colonne `Spec` produit un fichier de spec détaillé.
   - [ ] La colonne `In Progress` implémente avec TDD (tests failing → passing).
   - [ ] La colonne `Review` détecte et corrige des problèmes.
   - [ ] La colonne `Test` valide `cargo test` et `cargo tarpaulin`.
   - [ ] La colonne `Doc` met à jour `README.md`, `docs/`, et inline docs.
   - [ ] Le ticket atteint `Done` et une PR est ouverte.
