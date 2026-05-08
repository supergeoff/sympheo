# Cahier de Recette Fonctionnelle — Sympheo

> **Version:** 1.0  
> **Date:** 2026-05-08  
> **Produit:** Sympheo — Orchestrateur d'agents de codage  
> **Format:** Étapes procédurales numérotées (Préconditions → Actions → Résultats attendus)  
> **Cible:** Exécution automatisée par agent IA  
> **Backends couverts:** Local, Daytona  
> **Périmètre:** Tests end-to-end fonctionnels

---

## Table des matières

1. [Configuration & Démarrage](#1-configuration--démarrage)
2. [Polling & Récupération des Issues](#2-polling--récupération-des-issues)
3. [Filtrage & Sélection des Issues](#3-filtrage--sélection-des-issues)
4. [Dispatch Agent — Backend Local](#4-dispatch-agent--backend-local)
5. [Dispatch Agent — Backend Daytona](#5-dispatch-agent--backend-daytona)
6. [Session Multi-Tours](#6-session-multi-tours)
7. [Reconciliation & Cycle de Vie](#7-reconciliation--cycle-de-vie)
8. [Stall Detection](#8-stall-detection)
9. [Retry Logic](#9-retry-logic)
10. [Dashboard HTTP](#10-dashboard-http)
11. [Workspace Lifecycle & Hooks](#11-workspace-lifecycle--hooks)

---

## Conventions

- Chaque scénario possède un identifiant unique `SYM-XXX`.
- **Backend:** indique si le scénario s'applique à `Local`, `Daytona` ou `Les deux`.
- **Préconditions:** doivent être entièrement satisfaites avant d'exécuter les étapes.
- **Étapes:** numérotées, ordonnées, exécutées séquentiellement.
- **Résultats attendus:** assertions binaires (pass/fail) vérifiables automatiquement.
- Les valeurs entre chevrons `<...>` sont des paramètres à injecter selon l'environnement de test.

---

## 1. Configuration & Démarrage

### SYM-001 — Démarrage avec WORKFLOW.md valide (Local)

**Objectif :** Valider que Sympheo démarre correctement avec une configuration minimale valide en backend local.

**Backend :** Local

**Préconditions :**
- Le binaire `sympheo` est compilé et accessible dans `$PATH`.
- Un fichier `WORKFLOW.md` existe dans le répertoire de travail avec le contenu minimal suivant :
  ```yaml
  ---
  tracker:
    kind: github
    project_slug: <OWNER>/<REPO>
    project_number: <PROJECT_NUMBER>
    api_key: $GITHUB_API_KEY
  ---
  Do the work: {{ issue.title }}
  ```
- La variable d'environnement `GITHUB_API_KEY` est définie avec une clé API GitHub valide.
- Le projet GitHub `<OWNER>/<REPO>` existe et est accessible avec la clé API.
- Le binaire `opencode` est accessible dans `$PATH`.

**Étapes :**
1. Exécuter `sympheo ./WORKFLOW.md` dans le répertoire de travail.
2. Attendre 5 secondes.
3. Vérifier les logs de sortie (stdout/stderr).

**Résultats attendus :**
- Le log `startup validation passed` est présent.
- Aucun log d'erreur critique n'est émis au démarrage.
- Le processus reste actif (ne s'arrête pas avec un code de sortie non nul).
- Le log `orchestrator tick start` apparaît au moins une fois dans les 35 secondes suivant le démarrage (intervalle de polling par défaut : 30s).

---

### SYM-002 — Démarrage avec WORKFLOW.md valide (Daytona)

**Objectif :** Valider que Sympheo démarre correctement avec Daytona activé.

**Backend :** Daytona

**Préconditions :**
- Les préconditions de SYM-001 sont satisfaites.
- Le fichier `WORKFLOW.md` contient en plus la section Daytona :
  ```yaml
  daytona:
    enabled: true
    api_key: $DAYTONA_API_KEY
    api_url: https://api.daytona.io
  ```
- La variable d'environnement `DAYTONA_API_KEY` est définie avec une clé valide.

**Étapes :**
1. Exécuter `sympheo ./WORKFLOW.md`.
2. Attendre 5 secondes.
3. Vérifier les logs.

**Résultats attendus :**
- Le log `startup validation passed` est présent.
- Aucune erreur liée à Daytona n'est émise au démarrage.
- Le processus reste actif.

---

### SYM-003 — Échec de démarrage sans tracker.kind

**Objectif :** Valider que Sympheo refuse de démarrer si `tracker.kind` est manquant.

**Backend :** Les deux

**Préconditions :**
- Un fichier `WORKFLOW.md` invalide existe avec le contenu suivant :
  ```yaml
  ---
  tracker:
    project_slug: owner/repo
  ---
  prompt
  ```

**Étapes :**
1. Exécuter `sympheo ./WORKFLOW.md`.
2. Capturer le code de sortie du processus.

**Résultats attendus :**
- Le processus se termine avec le code de sortie `1`.
- Le log contient `startup validation failed`.
- Le log mentionne l'erreur `tracker.kind is required`.

---

### SYM-004 — Échec de démarrage sans tracker.api_key

**Objectif :** Valider que Sympheo refuse de démarrer si la clé API GitHub est manquante.

**Backend :** Les deux

**Préconditions :**
- Un fichier `WORKFLOW.md` existe avec `tracker.api_key` manquant ou vide.
- La variable d'environnement `GITHUB_API_KEY` n'est pas définie.

**Étapes :**
1. Exécuter `sympheo ./WORKFLOW.md`.

**Résultats attendus :**
- Le processus se termine avec le code de sortie `1`.
- Le log mentionne `MissingTrackerApiKey`.

---

### SYM-005 — Hot-reload de la configuration WORKFLOW.md

**Objectif :** Valider que la modification du fichier `WORKFLOW.md` recharge la configuration sans redémarrage.

**Backend :** Local

**Préconditions :**
- Sympheo est démarré avec un `WORKFLOW.md` valide et le processus est actif.
- Le log `workflow reloaded` n'a pas encore été émis.

**Étapes :**
1. Modifier le fichier `WORKFLOW.md` (par exemple, changer `poll_interval_ms` dans une section `polling`).
2. Enregistrer le fichier.
3. Attendre 1 seconde.
4. Vérifier les logs.

**Résultats attendus :**
- Le log `workflow reloaded` apparaît dans les 2 secondes suivant la sauvegarde.
- Aucune erreur de parsing n'est émise si le fichier reste valide.
- Le nouveau `poll_interval_ms` est pris en compte lors du prochain tick.

---

### SYM-006 — Résolution des variables d'environnement dans la configuration

**Objectif :** Valider que `$VAR` est interpolé dans les valeurs de configuration.

**Backend :** Les deux

**Préconditions :**
- `GITHUB_API_KEY=test123` est exportée dans l'environnement.
- Le `WORKFLOW.md` contient `api_key: $GITHUB_API_KEY`.

**Étapes :**
1. Exécuter `sympheo ./WORKFLOW.md`.
2. Vérifier que le démarrage réussit (validation passed).

**Résultats attendus :**
- Le démarrage réussit, prouvant que `$GITHUB_API_KEY` a été résolu en `test123`.

---

## 2. Polling & Récupération des Issues

### SYM-007 — Récupération des issues d'un GitHub ProjectV2 (Organisation)

**Objectif :** Valider que le polling récupère correctement les issues d'un projet organisation.

**Backend :** Les deux

**Préconditions :**
- Un projet GitHub ProjectV2 organisationnel existe avec au moins 1 issue ouverte.
- Le `project_slug` est au format `org/repo`.
- Le `project_number` correspond au numéro du projet.
- La clé API a les permissions `read:project` et `repo`.

**Étapes :**
1. Démarrer `sympheo` avec la configuration pointant vers ce projet.
2. Attendre le premier tick (maximum 35s).
3. Observer les logs.

**Résultats attendus :**
- Le log `orchestrator tick start` est présent.
- Aucune erreur `TrackerApiRequest` ou `TrackerApiStatus` n'est émise.
- Si des issues sont en état actif, le dispatch commence (logs `launching opencode agent`).

---

### SYM-008 — Récupération des issues d'un GitHub ProjectV2 (Utilisateur)

**Objectif :** Valider que le polling fonctionne avec un projet utilisateur (non organisation).

**Backend :** Les deux

**Préconditions :**
- Un projet GitHub ProjectV2 utilisateur existe avec au moins 1 issue.
- Le `project_slug` est au format `user/repo`.
- Le projet n'est pas rattaché à une organisation.

**Étapes :**
1. Démarrer `sympheo` avec cette configuration.
2. Attendre le premier tick.

**Résultats attendus :**
- Le polling réussit sans erreur.
- Les issues sont récupérées et traitées normalement.

---

### SYM-009 — Pagination des issues (>50 issues)

**Objectif :** Valider que le polling gère la pagination si le projet contient plus de 50 issues.

**Backend :** Les deux

**Préconditions :**
- Un projet GitHub ProjectV2 contient au moins 51 issues.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.

**Résultats attendus :**
- Aucune issue n'est perdue (toutes les issues actives sont candidates au dispatch).
- Aucune erreur de pagination n'est émise.

---

### SYM-010 — Extraction du champ Status personnalisé

**Objectif :** Valider que le champ `Status` du projet est correctement extrait et utilisé comme état de l'issue.

**Backend :** Les deux

**Préconditions :**
- Une issue dans le projet a le champ `Status` défini sur `In Progress`.
- `active_states` inclut `in progress` (valeur par défaut ou configurée).

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.

**Résultats attendus :**
- L'issue avec `Status = In Progress` est identifiée comme candidate.
- Si un slot est disponible, l'issue est dispatchée.

---

## 3. Filtrage & Sélection des Issues

### SYM-011 — Filtrage par états actifs

**Objectif :** Valider que seules les issues dans les états actifs sont dispatchées.

**Backend :** Les deux

**Préconditions :**
- Le projet contient 2 issues : une avec `Status = Todo`, une avec `Status = Done`.
- La configuration utilise les valeurs par défaut (`active_states` = Todo, In Progress ; `terminal_states` = Closed, Cancelled, etc.).
- Des slots d'agent sont disponibles.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.

**Résultats attendus :**
- L'issue `Todo` est dispatchée.
- L'issue `Done` n'est pas dispatchée.
- Le log mentionne uniquement l'issue `Todo` comme candidate.

---

### SYM-012 — Filtrage par états terminaux

**Objectif :** Valider que les issues dans un état terminal sont ignorées.

**Backend :** Les deux

**Préconditions :**
- Le projet contient une issue avec `Status = Closed`.
- `terminal_states` inclut `closed`.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.

**Résultats attendus :**
- L'issue `Closed` n'apparaît pas dans les logs de dispatch.
- Aucun worker n'est lancé pour cette issue.

---

### SYM-013 — Détection des issues bloquées

**Objectif :** Valider qu'une issue en état `todo` avec des dépendances non terminées est ignorée.

**Backend :** Les deux

**Préconditions :**
- Une issue `A` a `Status = Todo`.
- L'issue `A` a un champ `blocked_by` référençant l'issue `B`.
- L'issue `B` a `Status = In Progress` (non terminal).

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.

**Résultats attendus :**
- L'issue `A` n'est pas dispatchée.
- Aucun worker n'est lancé pour `A`.

---

### SYM-014 — Déblocage d'une issue

**Objectif :** Valider qu'une issue précédemment bloquée est dispatchée lorsque ses dépendances passent en état terminal.

**Backend :** Les deux

**Préconditions :**
- Les préconditions de SYM-013 sont satisfaites.
- Sympheo est en cours d'exécution.
- L'issue `B` est déplacée manuellement vers `Status = Done` (état terminal).

**Étapes :**
1. Modifier l'issue `B` sur GitHub pour la passer à `Done`.
2. Attendre le prochain tick (maximum 35s avec polling par défaut).

**Résultats attendus :**
- L'issue `A` devient candidate.
- L'issue `A` est dispatchée lors du prochain tick si un slot est disponible.

---

### SYM-015 — Tri par priorité, date de création, identifiant

**Objectif :** Valider l'ordre de dispatch des issues éligibles.

**Backend :** Les deux

**Préconditions :**
- Le projet contient 3 issues éligibles (`Todo` ou `In Progress`, non bloquées).
- Issue `C` : priorité 1, créée le 2026-01-01.
- Issue `B` : priorité 1, créée le 2026-01-02.
- Issue `A` : priorité 2, créée le 2026-01-01.
- Un seul slot d'agent est disponible (`max_concurrent_agents: 1`).

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.

**Résultats attendus :**
- L'issue `C` est dispatchée en premier (priorité la plus basse = la plus urgente, puis date la plus ancienne).
- L'issue `B` est en attente.
- L'issue `A` est en attente.

---

### SYM-016 — Limite de concurrence globale

**Objectif :** Valider que le nombre total d'agents concurrents ne dépasse pas `max_concurrent_agents`.

**Backend :** Les deux

**Préconditions :**
- `max_concurrent_agents: 2` dans la configuration.
- Le projet contient au moins 3 issues éligibles.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.

**Résultats attendus :**
- Exactement 2 workers sont lancés.
- Le 3ème issue reste en attente.
- Le log ne montre pas de 3ème dispatch simultané.

---

### SYM-017 — Limite de concurrence par état

**Objectif :** Valider que `max_concurrent_agents_by_state` est respectée.

**Backend :** Les deux

**Préconditions :**
- La configuration contient :
  ```yaml
  agent:
    max_concurrent_agents: 10
    max_concurrent_agents_by_state:
      "in progress": 1
  ```
- Le projet contient 2 issues avec `Status = In Progress` et 1 avec `Status = Todo`.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.

**Résultats attendus :**
- Un seul worker est lancé pour les issues `In Progress`.
- L'issue `Todo` peut également être dispatchée (dans la limite globale).
- Le 2ème `In Progress` reste en attente.

---

## 4. Dispatch Agent — Backend Local

### SYM-018 — Dispatch d'un agent local sur une issue

**Objectif :** Valider qu'un agent `opencode` est correctement lancé en local pour une issue.

**Backend :** Local

**Préconditions :**
- Une issue éligible existe dans le projet.
- `opencode` est installé et accessible.
- Le workspace root est configurable (par défaut dans `/tmp/sympheo_workspaces`).

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.
3. Observer les logs et le système de fichiers.

**Résultats attendus :**
- Le log `launching opencode agent (local backend)` apparaît.
- Un répertoire est créé sous le workspace root avec le nom sanitize de l'identifiant de l'issue.
- Le processus `opencode` est visible dans la liste des processus (`ps aux | grep opencode`).
- Le worker apparaît dans l'état `running` du dashboard (si activé).

---

### SYM-019 — Timeout d'un tour agent (Local)

**Objectif :** Valider qu'un tour agent est terminé si `codex_turn_timeout_ms` est dépassé.

**Backend :** Local

**Préconditions :**
- `codex_turn_timeout_ms: 5000` (5 secondes) dans la configuration.
- Une issue éligible existe.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch.
3. Simuler un blocage d'`opencode` (ou utiliser un mock qui ne répond pas).
4. Attendre 6 secondes.

**Résultats attendus :**
- Le log mentionne une erreur de timeout ou `AgentTurnTimeout`.
- Le processus `opencode` est tué.
- Le worker est marqué comme échoué et un retry est programmé.

---

### SYM-020 — Capture des événements JSON opencode (Local)

**Objectif :** Valider que les événements JSON ligne par ligne d'`opencode` sont correctement parsés.

**Backend :** Local

**Préconditions :**
- Une issue éligible existe.
- `opencode` est configuré pour émettre des événements JSON (`--format json`).

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch et la fin du premier tour.
3. Observer les logs de debug/info.

**Résultats attendus :**
- Les logs contiennent `step_start`, `step_finish` avec les IDs de session.
- Les tokens (`input_tokens`, `output_tokens`) sont accumulés dans l'état global.
- Le texte généré est stocké dans la session.

---

## 5. Dispatch Agent — Backend Daytona

### SYM-021 — Création d'un sandbox Daytona au premier dispatch

**Objectif :** Valider qu'un sandbox Daytona est créé lors du premier dispatch pour une issue.

**Backend :** Daytona

**Préconditions :**
- Daytona est activé dans la configuration avec une clé API valide.
- Le workspace pour l'issue n'existe pas encore (pas de fichier `.daytona_sandbox_id`).

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch d'une issue.
3. Vérifier la création du sandbox via l'API Daytona ou les logs.

**Résultats attendus :**
- Un sandbox est créé via l'API Daytona.
- L'ID du sandbox est écrit dans le fichier `<workspace>/.daytona_sandbox_id`.
- La commande `opencode` est exécutée à l'intérieur du sandbox.

---

### SYM-022 — Réutilisation du sandbox Daytona pour les tours suivants

**Objectif :** Valider que le même sandbox est réutilisé pour les tours suivants d'une même session.

**Backend :** Daytona

**Préconditions :**
- Une session est en cours sur une issue avec Daytona.
- Le fichier `.daytona_sandbox_id` existe dans le workspace.

**Étapes :**
1. Laisser le premier tour se terminer.
2. Attendre le début du tour 2 (continuation).
3. Vérifier l'API Daytona ou les logs.

**Résultats attendus :**
- Aucun nouveau sandbox n'est créé.
- Le même `sandbox_id` est utilisé pour le tour 2.
- Le paramètre `--session` est passé à `opencode` pour maintenir la continuité.

---

### SYM-023 — Timeout d'un tour agent (Daytona)

**Objectif :** Valider que le timeout de tour fonctionne aussi avec Daytona.

**Backend :** Daytona

**Préconditions :**
- `codex_turn_timeout_ms` est configuré à une valeur courte (ex: 5000ms).
- Une issue éligible existe.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch.
3. Bloquer la réponse du sandbox Daytona.
4. Attendre le dépassement du timeout.

**Résultats attendus :**
- L'erreur `AgentTurnTimeout` est émise.
- Le worker échoue et est replanifié en retry.

---

## 6. Session Multi-Tours

### SYM-024 — Rendu du template Liquid au premier tour

**Objectif :** Valider que le prompt du premier tour est rendu avec le template Liquid et les données de l'issue.

**Backend :** Les deux

**Préconditions :**
- Le `WORKFLOW.md` contient :
  ```
  ---
  tracker:
    ...
  ---
  Work on issue {{ issue.identifier }}: {{ issue.title }}
  ```
- Une issue `TEST-42` avec le titre `Fix login bug` est éligible.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch.
3. Intercepter ou observer la commande passée à `opencode`.

**Résultats attendus :**
- La commande `opencode` reçoit le prompt exact : `Work on issue TEST-42: Fix login bug`.
- Aucune erreur de template n'est émise.

---

### SYM-025 — Prompt de continuation aux tours suivants

**Objectif :** Valider que les tours N>1 utilisent le prompt de continuation hardcodé.

**Backend :** Les deux

**Préconditions :**
- Une session est en cours.
- `max_turns` est configuré à au moins 2.
- Le premier tour s'est terminé avec succès.

**Étapes :**
1. Attendre la fin du tour 1.
2. Observer le début du tour 2.

**Résultats attendus :**
- Le prompt du tour 2 est exactement : `Continue working on the current task. Review the conversation history and proceed with the next step.`
- Le paramètre `--session <session_id>` est passé pour maintenir le contexte.

---

### SYM-026 — Accumulation des tokens sur plusieurs tours

**Objectif :** Valider que les tokens sont cumulés dans l'état global à chaque tour.

**Backend :** Les deux

**Préconditions :**
- Une session s'exécute sur 2 tours.
- Chaque tour rapporte des tokens (input/output).

**Étapes :**
1. Laisser s'exécuter les 2 tours.
2. Interroger l'API `/api/v1/state` ou observer les logs.

**Résultats attendus :**
- `codex_totals.input_tokens` égale la somme des input tokens des 2 tours.
- `codex_totals.output_tokens` égale la somme des output tokens des 2 tours.
- `codex_totals.total_tokens` égale la somme totale.

---

### SYM-027 — Arrêt au maximum de tours atteint

**Objectif :** Valider que la session s'arrête après `max_turns` tours.

**Backend :** Les deux

**Préconditions :**
- `max_turns: 2` dans la configuration.
- Une issue éligible existe.

**Étapes :**
1. Démarrer `sympheo`.
2. Laisser la session s'exécuter.

**Résultats attendus :**
- Exactement 2 tours sont exécutés.
- Après le tour 2, le worker se termine normalement.
- Un retry immédiat est programmé (succès).

---

## 7. Reconciliation & Cycle de Vie

### SYM-028 — Cancellation quand une issue passe en état terminal

**Objectif :** Valider qu'un worker en cours est annulé si l'issue passe en état terminal.

**Backend :** Les deux

**Préconditions :**
- Une issue `X` est en cours de traitement (worker running).
- Sympheo est actif.

**Étapes :**
1. Sur GitHub, modifier l'issue `X` pour la passer à `Closed`.
2. Attendre le prochain tick (max 35s).
3. Observer les logs.

**Résultats attendus :**
- Le log indique que le worker est cancelled.
- Le flag `cancelled` passe à `true`.
- Le worker s'arrête.
- Le workspace est supprimé (cleanup).

---

### SYM-029 — Mise à jour de l'état d'une issue en cours

**Objectif :** Valider que l'état d'une issue en cours est rafraîchi lors de la reconciliation.

**Backend :** Les deux

**Préconditions :**
- Une issue `Y` est en cours avec `Status = In Progress`.
- Sympheo est actif.

**Étapes :**
1. Sur GitHub, modifier l'issue `Y` pour la passer à `In Review` (état actif).
2. Attendre le prochain tick.

**Résultats attendus :**
- L'état interne de l'issue dans `OrchestratorState` est mis à jour à `in review`.
- Le worker continue de s'exécuter (pas de cancellation).

---

### SYM-030 — Cleanup du workspace au passage en terminal

**Objectif :** Valider que le workspace est supprimé quand une issue devient terminale.

**Backend :** Les deux

**Préconditions :**
- Un worker est en cours pour l'issue `Z`.
- Le répertoire workspace existe sur le disque.

**Étapes :**
1. Passer l'issue `Z` à `Done` sur GitHub.
2. Attendre le prochain tick et la fin du worker.

**Résultats attendus :**
- Le répertoire workspace de l'issue `Z` n'existe plus.
- Si configuré, le hook `before_remove` est exécuté avant la suppression.

---

## 8. Stall Detection

### SYM-031 — Détection de stall et terminaison forcée

**Objectif :** Valider qu'un worker sans activité est tué après `codex_stall_timeout_ms`.

**Backend :** Les deux

**Préconditions :**
- `codex_stall_timeout_ms: 10000` (10 secondes).
- Une issue est dispatchée.
- L'agent ne produit aucun événement (simulation de blocage).

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch.
3. Attendre 15 secondes sans activité de l'agent.

**Résultats attendus :**
- Le log `stall detected, terminating` apparaît.
- Le worker est marqué comme échoué.
- Un retry est programmé avec backoff.

---

## 9. Retry Logic

### SYM-032 — Retry immédiat après succès

**Objectif :** Valider qu'un worker qui termine normalement est immédiatement replanifié.

**Backend :** Les deux

**Préconditions :**
- Une issue éligible existe.
- `max_turns: 1` (pour que le worker termine rapidement).

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre la fin du tour 1 et la sortie du worker.
3. Observer les logs.

**Résultats attendus :**
- Le log `worker exited normally` est présent.
- Un `RetryEntry` est créé avec un délai de 1 seconde.
- L'issue est à nouveau candidate au prochain cycle de retry.

---

### SYM-033 — Backoff exponentiel après échec

**Objectif :** Valider qu'un échec déclenche un retry avec délai croissant.

**Backend :** Les deux

**Préconditions :**
- Une issue éligible existe.
- L'agent est configuré pour échouer systématiquement (retourne `success: false`).
- `max_retry_backoff_ms: 30000` (30 secondes).

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier échec.
3. Noter le délai avant le retry.
4. Attendre le deuxième échec.
5. Noter le nouveau délai.

**Résultats attendus :**
- Le premier retry est programmé après ~10 secondes (base).
- Le deuxième retry est programmé après ~20 secondes.
- Le délai ne dépasse pas `max_retry_backoff_ms`.

---

### SYM-034 — Abandon d'un retry si l'issue devient terminale

**Objectif :** Valider qu'un retry en attente est annulé si l'issue passe en état terminal.

**Backend :** Les deux

**Préconditions :**
- Un retry est programmé pour l'issue `R` (suite à un échec).
- L'issue `R` est encore dans la file d'attente de retry.

**Étapes :**
1. Passer l'issue `R` à `Closed` sur GitHub.
2. Attendre que le retry devienne dû.

**Résultats attendus :**
- Le retry est retiré de la file.
- L'issue n'est pas redispatchée.
- Le log indique que l'issue est ignorée car terminale.

---

## 10. Dashboard HTTP

### SYM-035 — Démarrage du serveur HTTP

**Objectif :** Valider que le serveur HTTP démarre sur le port spécifié.

**Backend :** Les deux

**Préconditions :**
- Le port `8080` est libre sur `127.0.0.1`.

**Étapes :**
1. Exécuter `sympheo ./WORKFLOW.md --port 8080`.
2. Attendre 2 secondes.
3. Effectuer une requête HTTP `GET http://127.0.0.1:8080/`.

**Résultats attendus :**
- La réponse HTTP est `200 OK`.
- Le body HTML contient les compteurs `Running` et `Retrying`.

---

### SYM-036 — Endpoint API d'état global

**Objectif :** Valider que `/api/v1/state` retourne l'état complet de l'orchestrateur.

**Backend :** Les deux

**Préconditions :**
- Le serveur HTTP est actif sur le port 8080.
- Au moins un worker est en cours d'exécution.

**Étapes :**
1. Effectuer `GET http://127.0.0.1:8080/api/v1/state`.
2. Parser la réponse JSON.

**Résultats attendus :**
- Le JSON contient `generated_at`, `counts.running`, `counts.retrying`.
- Le tableau `running` contient au moins une entrée avec `issue_id`, `issue_identifier`, `turn_count`, `started_at`.
- Le tableau `retrying` est présent (même si vide).
- `codex_totals` contient `input_tokens`, `output_tokens`, `total_tokens`, `seconds_running`.

---

### SYM-037 — Endpoint API de détail d'une issue

**Objectif :** Valider que `/api/v1/:issue_identifier` retourne les détails d'une issue en cours.

**Backend :** Les deux

**Préconditions :**
- Un worker est en cours pour l'issue `TEST-42`.
- Le serveur HTTP est actif.

**Étapes :**
1. Effectuer `GET http://127.0.0.1:8080/api/v1/TEST-42`.
2. Parser la réponse JSON.

**Résultats attendus :**
- La réponse est `200 OK`.
- Le JSON contient `issue_identifier: "TEST-42"`, `status: "running"`, `turn_count`.

---

### SYM-038 — Endpoint API de détail pour une issue inexistante

**Objectif :** Valider que l'API retourne 404 pour une issue non en cours.

**Backend :** Les deux

**Préconditions :**
- Aucun worker n'est en cours pour l'issue `TEST-99`.
- Le serveur HTTP est actif.

**Étapes :**
1. Effectuer `GET http://127.0.0.1:8080/api/v1/TEST-99`.

**Résultats attendus :**
- La réponse est `404 Not Found`.

---

### SYM-039 — Endpoint de refresh

**Objectif :** Valider que `POST /api/v1/refresh` retourne une confirmation.

**Backend :** Les deux

**Préconditions :**
- Le serveur HTTP est actif.

**Étapes :**
1. Effectuer `POST http://127.0.0.1:8080/api/v1/refresh`.
2. Parser la réponse JSON.

**Résultats attendus :**
- La réponse est `200 OK`.
- Le JSON contient `queued: true`, `operations: ["poll", "reconcile"]`.

---

## 11. Workspace Lifecycle & Hooks

### SYM-040 — Création d'un workspace à la première exécution

**Objectif :** Valider qu'un workspace est créé lors du premier dispatch d'une issue.

**Backend :** Local

**Préconditions :**
- Le répertoire workspace pour l'issue n'existe pas.
- Une issue éligible existe.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch.
3. Vérifier l'existence du répertoire workspace.

**Résultats attendus :**
- Un répertoire est créé sous le workspace root.
- Le nom du répertoire correspond à `sanitize_identifier(issue.identifier)`.
- Si un hook `after_create` est configuré, il est exécuté.

---

### SYM-041 — Réutilisation d'un workspace existant

**Objectif :** Valider qu'un workspace existant est réutilisé pour un retry ou une nouvelle session.

**Backend :** Local

**Préconditions :**
- Un workspace existe déjà pour l'issue (contient potentiellement des fichiers).
- L'issue est dispatchée à nouveau (retry ou nouveau cycle).

**Étapes :**
1. Attendre le dispatch suivant pour la même issue.
2. Vérifier le workspace.

**Résultats attendus :**
- Le même répertoire est utilisé.
- Aucun nouveau répertoire n'est créé.
- Le hook `after_create` n'est pas réexécuté.

---

### SYM-042 — Exécution des hooks before_run et after_run

**Objectif :** Valider que les hooks sont exécutés aux bons moments du cycle de vie.

**Backend :** Local

**Préconditions :**
- La configuration contient :
  ```yaml
  hooks:
    before_run: "echo BEFORE_RUN > hook_log.txt"
    after_run: "echo AFTER_RUN >> hook_log.txt"
  ```
- Une issue éligible existe.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre la fin complète du worker (tous les tours).
3. Lire le fichier `hook_log.txt` dans le workspace.

**Résultats attendus :**
- Le fichier `hook_log.txt` contient `BEFORE_RUN` suivi de `AFTER_RUN`.
- `before_run` est exécuté avant le premier tour.
- `after_run` est exécuté après le dernier tour.

---

### SYM-043 — Hook before_run en échec

**Objectif :** Valider qu'un échec du hook `before_run` fait échouer le worker.

**Backend :** Local

**Préconditions :**
- La configuration contient `hooks.before_run: "exit 1"`.
- Une issue éligible existe.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch.

**Résultats attendus :**
- Le log mentionne `HookFailed`.
- Le worker échoue.
- Un retry est programmé.

---

### SYM-044 — Hook after_run en échec (non bloquant)

**Objectif :** Valider qu'un échec du hook `after_run` ne bloque pas la terminaison du worker.

**Backend :** Local

**Préconditions :**
- La configuration contient `hooks.after_run: "exit 1"`.
- Une issue éligible existe.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre la fin du worker.

**Résultats attendus :**
- Le log mentionne un warning `after_run hook failed`.
- Le worker se termine normalement malgré l'échec du hook.
- Un retry succès est programmé.

---

### SYM-045 — Sanitization des identifiants de workspace

**Objectif :** Valider que les caractères spéciaux dans les identifiants sont remplacés par des underscores.

**Backend :** Local

**Préconditions :**
- Une issue avec l'identifiant `feat/new_thing` est éligible.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch.
3. Lister le workspace root.

**Résultats attendus :**
- Un répertoire nommé `feat_new_thing` est créé (slash remplacé par underscore).

---

### SYM-046 — Validation du chemin workspace (anti-traversal)

**Objectif :** Valider qu'un chemin de workspace ne peut pas sortir du répertoire root.

**Backend :** Local

**Préconditions :**
- Le backend local est utilisé.

**Étapes :**
1. Tenter d'utiliser un workspace path malveillant (test unitaire ou injection).

**Résultats attendus :**
- L'appel à `validate_inside_root` retourne une erreur `WorkspaceError` avec le message `workspace path is outside root`.

---

### SYM-047 — Timeout des hooks

**Objectif :** Valider qu'un hook qui dépasse `hook_timeout_ms` est tué.

**Backend :** Local

**Préconditions :**
- La configuration contient `hooks.timeout_ms: 2000`.
- Le hook `before_run` exécute `sleep 10`.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le dispatch.

**Résultats attendus :**
- Le log mentionne que le hook a timeout après 2 secondes.
- Le processus du hook est tué.
- Le worker échoue.

---

## 12. Scénarios de bout en bout complets

### SYM-048 — Flux complet : Issue créée → Dispatch → Succès → Retry → Terminal → Cleanup

**Objectif :** Valider le cycle de vie complet d'une issue du début à la fin.

**Backend :** Local

**Préconditions :**
- Un projet GitHub avec une issue `E2E-1` en état `Todo`.
- `max_turns: 1`.
- `max_concurrent_agents: 1`.

**Étapes :**
1. Créer l'issue `E2E-1` avec `Status = Todo`.
2. Démarrer `sympheo`.
3. Attendre le dispatch et la fin du tour (succès).
4. Vérifier que le retry est programmé.
5. Sur GitHub, passer `E2E-1` à `Done`.
6. Attendre le prochain tick + retry.
7. Vérifier l'état final.

**Résultats attendus :**
- Étape 3 : worker lancé, tour exécuté, `worker exited normally`.
- Étape 4 : retry présent dans l'état.
- Étape 6 : le retry vérifie l'état, constate que c'est terminal, et ne redispatch pas.
- Étape 7 : le workspace est supprimé. L'issue n'apparaît plus dans `running` ni `retrying`.

---

### SYM-049 — Flux complet avec Daytona

**Objectif :** Valider le cycle de vie complet avec le backend Daytona.

**Backend :** Daytona

**Préconditions :**
- Les préconditions de SYM-048 sont satisfaites.
- Daytona est activé et configuré.

**Étapes :**
1. Suivre les étapes de SYM-048.
2. Vérifier la création et la réutilisation du sandbox.

**Résultats attendus :**
- Les mêmes résultats que SYM-048.
- Un sandbox Daytona est créé au premier dispatch.
- Le même sandbox est réutilisé pour le retry.
- Le sandbox est réutilisé via `.daytona_sandbox_id`.

---

### SYM-050 — Concurrence multiple avec différents états

**Objectif :** Valider le comportement avec plusieurs issues concurrentes dans différents états.

**Backend :** Local

**Préconditions :**
- `max_concurrent_agents: 3`.
- `max_concurrent_agents_by_state: { "in progress": 1 }`.
- 2 issues `In Progress` : `IP-1`, `IP-2`.
- 2 issues `Todo` : `TD-1`, `TD-2`.

**Étapes :**
1. Démarrer `sympheo`.
2. Attendre le premier tick.

**Résultats attendus :**
- `IP-1` est dispatchée.
- `IP-2` reste en attente (limite par état).
- `TD-1` est dispatchée.
- `TD-2` est dispatchée.
- Total : 3 workers actifs (maximum global).
