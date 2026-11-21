# Tatami — plan de développement

> Suivi de projet. Dernière mise à jour : 2026-11-21.

## 1. Objet

Tatami est un portage de [worktrunk](https://github.com/max-sixty/worktrunk) pour
[jujutsu](https://jj-vcs.dev) : un wrapper de haut niveau autour de `jj` pour gérer
des **workspaces** (l'équivalent jj des worktrees git), avec la même philosophie
d'ergonomie (workflow de merge, hooks, intégration shell, picker).

Fichier de configuration : **schéma strictement identique** à worktrunk.
Résolution : `~/.config/tatami/config.toml`, sinon fallback
`~/.config/worktrunk/config.toml`.

## 2. Décisions arrêtées (2026-11-21)

| # | Décision | Choix |
|---|----------|-------|
| 1 | Nommage | binaire `tatami`, wrapper shell `tt` |
| 2 | État interne repo-global | `.jj/tatami/` dans le workspace **hôte** du stockage partagé (voir §5) |
| 3 | Config projet | `.config/tatami.toml`, sinon fallback `.config/wt.toml` |
| 4 | Clé d'emplacement | `workspace-path` (nouveau), fallback legacy `worktree-path` si absente |
| 5 | Repos colocalisés (jj+git) | dans le périmètre v1 |
| 6 | Création workspace | bookmark du même nom créé par défaut (`jj bookmark create <nom> -r @`) |
| 7 | Env `WORKTRUNK_*` | non acceptée ; tatami n'expose que `TATAMI_*` (même convention d'imbrication `__`) |

## 3. Surface CLI cible (mapping `wt` → `tt`)

| Worktrunk | Tatami | Adaptation jj |
|---|---|---|
| `wt switch` | `tt switch [nom]` | cd vers la racine du workspace ; sans argument = picker ; `-` = précédent ; `--create` → `tt create` ; détection stale → proposer `jj workspace update-stale` |
| `wt list` | `tt list` | statuts jj : `@` vide/divergent/conflict, retard vs bookmark principal, stale, présence disque ; `--format json` |
| `wt create` | `tt create <nom>` | `jj workspace add` + bookmark homonyme + copie des ignorés + hooks post-start |
| `wt remove` | `tt remove <nom>` | garde-fous (commits non intégrés → refuser sans `--force`) ; `jj workspace forget` + rm du répertoire ; `--delete-bookmark` si intégré ; poubelle différée ; **protection spéciale workspace hôte** (§5) |
| `wt merge` | `tt merge [cible]` | décrire/squasher `nom@` → rebaser sur la cible si besoin → avancer le bookmark cible (ff) → `jj git push` éventuel → retirer le workspace |
| `wt rebase` | `tt rebase` | `jj rebase -d <cible>` des commits du workspace |
| `wt step …` | `tt step …` | primitives jj : `commit`, `squash`, `rebase`, `push`, `diff`, `for-each`, `prune`, `copy-ignored`, `eval`, `rename` (nouveau). `promote`/`tether` : hors périmètre v1 |
| `wt hook …` | `tt hook …` | événements identiques (`pre/post` × `switch/start/commit/merge/remove`) ; approbation premier run dans `~/.config/tatami/approvals.toml` (fallback worktrunk, même règle que la config) |
| `wt config …` | `tt config …` | `create/show/shell install\|uninstall/state` |

Globales : `-C <path>`, `--config`, `--config-set`, `-v/-vv`, `-y/--yes`, `--no-hooks`.

## 4. Compatibilité configuration (schéma worktrunk)

Faits vérifiés (source : worktrunk/DeepWiki, 2026-11-21) :

- Hooks = **clés top-level** kebab-case (`pre-start = "npm install"`), **pas** de
  table `[hooks]`.
- Formes de valeur identiques pour hooks et aliases :
  1. string (une commande) ;
  2. table nommée (commandes concurrentes) ;
  3. `[[cle]]` répétés (pipeline ordonné, concurrent dans l'étape, échec = arrêt).
- Sections : `[switch] cd=true`, `[switch.picker] pager`, `[merge]
  squash/commit/rebase/remove/verify/ff = true`, `[remove] delete-branch=true`,
  `[commit] stage="all"`, `[step.copy-ignored] exclude=[]`,
  `[projects."host/owner/repo"]` (overrides scalaires ; hooks/aliases s'accumulent,
  user avant projet, hooks projet soumis à approbation).
- Env : `WORKTRUNK_A__B` → tatami : `TATAMI_A__B` (décision #7 : pas d'alias `WORKTRUNK_`).

Règles tatami :

- Parse **lenient** (clés inconnues ignorées) : un config worktrunk partagé reste
  chargeable pendant que tatami implémente les sections progressivement.
- `workspace-path` gagne sur `worktree-path` ; l'expansion de variables
  (`{{ repo_path }}`, `{{ owner }}`, `{{ workspace }}`, filtres `sanitize`,
  `sanitize_db`, `codename(2)`, `~`) est faite par un moteur unique (minijinja)
  en M4 — point d'attention validé.
- Un fichier tatami **présent mais invalide** est une erreur, pas un fallback
  silencieux vers worktrunk.

## 5. État interne (décision #2)

Emplacement : `.jj/tatami/` dans le **workspace hôte** du stockage partagé jj.

Faits jj vérifiés :

- Le stockage partagé vit physiquement dans le `.jj/repo` du workspace initial ;
  les workspaces secondaires ont un `.jj/repo` **fichier** contenant un chemin
  relatif vers lui (ex. `../../main/.jj/repo`).
- `jj workspace forget` (même sur `default`) ne détruit pas le stockage ; c'est le
  `rm -rf` du répertoire hôte qui le fait.

Conséquences :

- Résolution de l'état : si `.jj/repo` est un dossier → on est l'hôte ; sinon lire
  le pointeur, résoudre, remonter de deux niveaux.
  `shortcut: le format du fichier pointeur .jj/repo est un détail privé de jj —
  revalider à chaque upgrade jj notable (quand : lecture du pointeur qui échoue
  ou jj ≥ version majeure).`
- `tt remove` sur le workspace hôte doit **avertir fortement** : supprimer ce
  répertoire détruit le stockage partagé (tous les workspaces meurent), pas
  seulement l'état tatami. Implémenté en M2.
- État concerné : workspace précédent (switch `-`), cache branche/bookmark
  principal, logs de hooks, poubelle, marqueurs/vars.

## 6. Architecture

```
src/
  main.rs            # wiring clap → dispatch
  cli/               # définition clap (une commande = un module) — à partir de M1
  cmd/               # un module par commande, orchestration only
  config/            # schéma serde + layering (user tatami < user worktrunk < env < --config-set)
  jj/                # ADAPTATEUR : le seul module qui invoque `jj` (subprocess, templates -T)
  state/             # état interne (voir §5)
  hook/              # exécution hooks + approvals (M2)
  template/          # minijinja, variables/filtres partagés (M4)
  style/             # rendu table/json/couleurs
```

Principes :

- Adaptateur jj derrière un trait (`Jj`) : fake pour les tests rapides, tests
  d'intégration sur **vrai jj** dans des repos temporaires (tier « preuve »).
- Erreurs typées (`thiserror`), pas de bool de succès ; les erreurs jj
  remontent avec contexte (commande, stderr).
- Une commande = une slice verticale (cli + cmd + jj/state), ordre d'exécution
  visible à la lecture.
- Qualité : `[lints.rust] warnings = "deny"`, rustfmt, clippy, CI GitHub Actions
  (build + tests + intégration jj), tests rapides partout / intégration étiquetée.
- Tests nommés comme connaissance métier (`mod load_user_config_should { … }`),
  pas `test1`/`should_work`.
- Un seul crate tant que ça tient (AHA).

## 7. Roadmap

Chaque jalon livrera un binaire utilisable.

- **M0 — Fondations**
  - Loader config avec layering user (tatami → worktrunk) + `TATAMI_CONFIG_PATH`,
    schéma lenient, `tt config show`.
  - Erreurs typées, lints deny, harness tests (repo jj temporaire) — jj vient après.
  - Sortie : `tt config show` affiche le chemin résolu, la source (tatami /
    fallback worktrunk) et le `workspace-path` effectif ; CI verte.
- **M1 — list + switch + shell** : `tt list` (table de statut), `tt switch`
  (picker, `-`), `tt config shell install` (zsh/bash), état « précédent », stale.
- **M2 — create + remove** : `tt create` (bookmark homonyme, copie des ignorés),
  hooks pre/post-start avec approbation ; `tt remove` (garde-fous, forget + rm,
  poubelle, protection workspace hôte §5).
- **M3 — merge + rebase + step** : workflow `tt merge` jj-isé, `tt rebase`,
  primitives `tt step` (commit, squash, rebase, push, diff, for-each, prune,
  copy-ignored).
- **M4 — Finition** : aliases, minijinja + expansion complète, `--format json`,
  `tt config state`, logs, `--config-set`, complétions, README/docs, fish/nushell.
- **M5 — Au-delà du portage** : `tt rename` (jj natif), intégration op-log,
  statut conflits/divergence riche, forge (pr:/mr:) plus tard.

## 8. Points d'attention jj (mémo technique)

- **Stale** : réécrire `nom@` depuis un autre workspace rend le premier obsolète ;
  `tt switch` doit détecter et proposer `jj workspace update-stale`.
- Pas de branche à poser par workspace : un commit `nom@` par workspace,
  bookmarks partagés au niveau repo.
- « Propre/sale » n'existe pas tel quel : métriques utiles = `@` vide,
  divergent, conflict, retard vs principal.
- Colocated (périmètre v1) : `.git` présent à côté de `.jj` ; l'état reste dans
  `.jj/tatami/` (uniforme), `jj` reste la seule interface.
- Suppression : `jj workspace forget` ne touche pas au disque ; tatami fait
  forget + rm. Cas spécial workspace hôte → §5.
