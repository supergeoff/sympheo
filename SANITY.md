# TODO — Mise en place qualité/stabilité repo Rust

## Contexte global

Mettre en place un pipeline de qualité/stabilité sur un repo Rust (single crate, extensible workspace plus tard) pour empêcher toute régression introduite par un agent IA ou un commit humain. Trois couches : pre-commit local via githooks standards, CI GitHub Actions bloquante, branch protection sur `main` via `gh` CLI.

Contraintes :
- Pas de dépendances externes à l'écosystème Rust standard (sauf `gh` CLI et actions GitHub officielles)
- Outils Rust : dernière version (`@latest`, jamais de pinning)
- Scripts portables bash (Linux/macOS/WSL)
- Pas de review humaine, mais admin enforcement actif
- Coverage en `lcov` + script bash maison pour seuil
- Le repo utilise [mise](https://mise.jdx.dev/) pour le tooling — utiliser si pertinent, sinon non

## Vocabulaire

Toutes les étapes utilisent des noms universels, agnostiques au langage. L'implémentation Rust est un détail interne :

- **format** (impl: `cargo fmt`)
- **lint** (impl: `cargo clippy`)
- **check** (impl: `cargo check`)
- **test** (impl: `cargo test`)
- **build** (impl: `cargo build --release`)
- **enforce-patterns** (impl: script bash de règles qualité globales)
- **coverage** (impl: `cargo llvm-cov` → lcov + script bash de seuil)

---

## Phase 0 — mise

Si le repo n'a pas déjà un `mise.toml` ou `.mise.toml`, en créer un à la racine déclarant au minimum :
- `rust = "latest"`

Si un fichier mise existe déjà, ajouter Rust en `latest` s'il n'y est pas, sans toucher au reste.

Cela donne aux contributeurs (humains et agents) une toolchain cohérente sans pinning explicite. La CI installera Rust via les actions standards, pas via mise.

---

## Phase 1 — Githooks locaux

### 1.1 Structure
- Créer `.githooks/` à la racine
- Ajouter dans le `README.md` une section "Setup" avec : `git config core.hooksPath .githooks` à exécuter après clone

### 1.2 Hook `pre-commit`
Créer `.githooks/pre-commit` (bash, exécutable, `#!/usr/bin/env bash` + `set -euo pipefail`). Étapes dans l'ordre, arrêt à la première erreur :

1. **format** : `cargo fmt --all -- --check`
2. **lint** : `cargo clippy --all-targets --all-features -- -D warnings`
3. **check** : `cargo check --all-targets --all-features`
4. **test** : `cargo test --all-features`
5. **enforce-patterns** : `bash .githooks/lib/enforce-patterns.sh`

Préfixer chaque étape avec `[pre-commit][<nom-étape>]` pour clarté. Compatible Linux/macOS/WSL, pas de bashisms exotiques.

### 1.3 Hook `commit-msg`
Créer `.githooks/commit-msg` (bash, exécutable) qui valide :

- Première ligne : `^(feat|fix|docs|style|refactor|test|chore|perf|build|ci|revert)(\([a-z0-9_-]+\))?!?: .+$`
- Première ligne ≤ 150 caractères
- Si body présent (lignes après ligne vide), aucune ligne du body > 150 caractères
- Ignorer lignes commençant par `#`
- Ignorer commits de merge auto (`Merge branch...`)
- Message d'erreur explicite avec le format attendu si rejet

### 1.4 Script `.githooks/lib/enforce-patterns.sh`
Script bash autonome et réutilisable (sourcé par pre-commit ET appelé par CI). Il porte l'ensemble des règles qualité globales qui ne sont couvertes par aucun autre outil. Conçu pour évoluer : nouvelles règles s'ajoutent ici.

Règles initiales :

- **`#[ignore]` sans justification** : tout `#[ignore]` doit avoir un commentaire `// Reason:` sur la ligne précédente ou suivante
- **`#[allow(...)]` sans justification** : toute forme `#[allow(...)]` doit avoir un commentaire `// Reason:` adjacent
- **Macros panicantes en code applicatif** : `todo!()`, `unimplemented!()` interdits hors `tests/` et hors blocs `#[cfg(test)]`
- **Tests sans assertion réelle** : fonctions `#[test]` qui ne contiennent ni `assert`, ni `assert_eq`, ni `assert_ne`, ni `panic`, ni `?`, ni retour `Result`, ni macro `assert*` custom

Pour chaque violation : afficher `fichier:ligne — <règle violée> — <extrait>`. Sortir 1 si au moins une violation. Scanner `src/` et `tests/` par défaut, paramétrable plus tard si besoin.

Le script doit être exécutable seul depuis la racine : `bash .githooks/lib/enforce-patterns.sh`.

### 1.5 Documentation README
Mettre à jour `README.md` :
- Commande de setup `git config core.hooksPath .githooks`
- Liste des règles enforced (résumé)
- Mention du bypass `git commit --no-verify` avec rappel que la CI re-validera

---

## Phase 2 — CI GitHub Actions

### 2.1 Workflow `.github/workflows/ci.yml`
Déclencheurs :
- `push` sur toute branche
- `pull_request` ciblant `main`

Runner : `ubuntu-latest`. Toolchain Rust : installer la dernière stable via l'action standard `dtolnay/rust-toolchain@stable` ou `actions/checkout` + `rustup update stable && rustup default stable`. Aucun pinning de version.

Cache : `actions/cache@latest` avec :
- `~/.cargo/registry`
- `~/.cargo/git`
- `target/`
- Clé basée sur hash de `Cargo.lock`

### 2.2 Jobs (tous bloquants, en parallèle)

Chaque job a un `name:` stable correspondant exactement au vocabulaire universel :

- **`format`** : `cargo fmt --all -- --check`
- **`lint`** : `cargo clippy --all-targets --all-features -- -D warnings`
- **`check`** : `cargo check --all-targets --all-features`
- **`test`** : `cargo test --all-features --workspace`
- **`build`** : `cargo build --release --all-features`
- **`enforce-patterns`** : `bash .githooks/lib/enforce-patterns.sh`
- **`coverage`** :
  - Installer l'outil de coverage Rust (`cargo install cargo-llvm-cov --locked`)
  - Générer : `cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info`
  - Vérifier : `bash scripts/check-coverage.sh lcov.info 80`
  - Uploader `lcov.info` comme artefact

Si `tarpaulin` ou tout autre outil de coverage existe (config, jobs, mentions dans `Cargo.toml`), le supprimer entièrement.

### 2.3 Script `scripts/check-coverage.sh`
Script bash universel, réutilisable Go/Python/autre :

- Args : `$1` = chemin lcov.info, `$2` = seuil minimum (entier ou flottant)
- Parser format LCOV : sommer `LF:` (lines found) et `LH:` (lines hit) sur tous les blocs
- Calculer `(LH / LF) * 100`, afficher avec 2 décimales
- Sortir 0 si `>= seuil`, 1 sinon avec message `Coverage X% < threshold Y%`
- `set -euo pipefail`, gérer fichier absent ou malformé
- Outils utilisés : `awk` et/ou `bc` (built-in Linux/macOS)

---

## Phase 3 — Branch protection via `gh` CLI

### 3.1 Script `scripts/setup-branch-protection.sh`
Script bash idempotent. Doit :

- Vérifier `gh auth status`
- Détecter `OWNER/REPO` via `gh repo view --json nameWithOwner`
- Appliquer via `gh api -X PUT repos/{owner}/{repo}/branches/main/protection` :
  - `required_status_checks.strict: true`
  - `required_status_checks.contexts: ["format", "lint", "check", "test", "build", "enforce-patterns", "coverage"]`
  - `enforce_admins: true`
  - `required_pull_request_reviews: null`
  - `restrictions: null`
  - `allow_force_pushes: false`
  - `allow_deletions: false`
  - `required_conversation_resolution: true`
- Activer la suppression auto des branches après merge : `gh api -X PATCH repos/{owner}/{repo} -f delete_branch_on_merge=true`
- Afficher la config finale via `gh api repos/{owner}/{repo}/branches/main/protection` pour vérification

---

## Phase 4 — Cohérence et validation

### 4.1 Nettoyage
- Supprimer toute trace de `tarpaulin` ou autre outil de coverage redondant
- Supprimer tout hook/CI/script existant qui ferait doublon avec ce setup

### 4.2 Fichiers de config
- `rustfmt.toml` minimal (laisser vide pour défauts, ou juste `edition = "2024"` si la crate est sur cette édition — à détecter dans `Cargo.toml`)
- `clippy.toml` vide pour permettre extension future

### 4.3 `.gitignore`
S'assurer que `target/`, `lcov.info`, `*.profraw` sont ignorés.

### 4.4 Test end-to-end
1. Activer les hooks localement : `git config core.hooksPath .githooks`
2. Faire un commit propre passant tous les hooks
3. Pousser une branche, ouvrir une PR
4. Vérifier que les 7 checks CI tournent
5. Vérifier qu'un push direct sur `main` est refusé
6. Vérifier qu'on ne peut pas merger sur `main` sans tous les checks verts

### 4.5 Test négatif (validation des garde-fous)
Sur une branche jetable, créer un commit par cas et vérifier l'échec correct :

- Mauvais formatting → `format` échoue
- Warning clippy → `lint` échoue
- Test sans assertion → `enforce-patterns` échoue
- `#[allow(dead_code)]` sans `// Reason:` → `enforce-patterns` échoue
- `#[ignore]` sans `// Reason:` → `enforce-patterns` échoue
- `todo!()` dans `src/` → `enforce-patterns` échoue
- Commit avec message non-conventional → hook `commit-msg` local échoue
- Couverture < 80% → `coverage` échoue

Documenter dans un fichier temporaire `VALIDATION.md` (à supprimer ensuite).

---

## Critères d'acceptation

- Tous les fichiers commités sur `main` via PR initiale
- `git config core.hooksPath .githooks` activé localement
- Branch protection visible dans Settings → Branches avec les 7 checks
- Aucune dépendance ajoutée au `Cargo.toml` non liée au code applicatif
- Outils requis hors-toolchain : `git`, `bash`, `gh` CLI, `awk`, `bc` (tous built-in ou installés via mise)
- `README.md` contient les instructions de setup complètes

---

## Notes pour l'agent qui implémente

- Procéder phase par phase, dans l'ordre
- Un commit conventional par phase (`chore(ci): setup githooks`, `chore(ci): add github actions workflow`, `chore(ci): apply branch protection`, etc.)
- Choisir l'option la plus minimaliste et standard si une décision n'est pas tranchée
- Aucune nouvelle dépendance Rust sans justification explicite
- Tester chaque hook localement avant de pousser
- Phase 3 (branch protection) en DERNIER, une fois la CI verte sur une PR de validation, sinon auto-blocage