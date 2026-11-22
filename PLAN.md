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
| `wt switch` | `tt switch [nom]` | cd vers la racine du workspace ; sans argument = picker ; `-` = précédent ; **`--create <nom>`** crée le workspace + bookmark homonyme puis switch (parité worktrunk : il n'existe pas de `wt create` autonome) ; détection stale inutile (voir §9) |
| `wt list` | `tt list` | statuts jj : `@` vide/divergent/conflict, retard vs bookmark principal, stale, présence disque ; `--format json` |
| `wt create` | — | **n'existe pas en amont** : worktrunk crée via `switch --create`. Destination par défaut : `../<repo>.<workspace>` (frère du repo, hors working copy) |
| `wt remove` | `tt remove <nom>` | garde-fous (commits non intégrés → refuser sans `--force`) ; `jj workspace forget` + rm du répertoire ; `--delete-bookmark` si intégré ; poubelle différée ; **protection spéciale workspace hôte** (§5) |
| `wt merge` | `tt merge [cible]` | décrire/squasher `nom@` → rebaser sur la cible si besoin → avancer le bookmark cible (ff) → `jj git push` éventuel → retirer le workspace |
| `wt rebase` | `tt rebase` | `jj rebase -d <cible>` des commits du workspace |
| `wt step …` | `tt step …` | primitives jj : `commit`, `squash`, `rebase`, `push`, `diff`, `for-each`, `prune`, **`copy-ignored`** (ne fait PAS partie de la création — les utilisateurs le câblent en hook post-start), `eval`, `rename` (nouveau). `promote`/`tether` : hors périmètre v1 |
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
- **Contrats épinglés empiriquement (jj 0.47.1)** : `jj workspace root` sort un
  chemin absolu **canonicalisé** (symlinks résolus) ; hors repo → exit 1 + stderr
  `Error: There is no jj repo in "."` ; le pointeur secondaire `.jj/repo` contient
  un chemin **relatif au dossier `.jj` du workspace** (ex. `../../main/.jj/repo`,
  sans newline) ; jj **tolère** les dossiers inconnus comme `.jj/tatami/`.
  La version jj de référence est verrouillée dans `.github/workflows/ci.yml`.

## 9. Avancement

### M0 — Fondations ✅ (2026-11-21)

- [x] Loader config : layering user (tatami → worktrunk), `TATAMI_CONFIG_PATH`,
      schéma lenient, fallback `workspace-path` → `worktree-path`
- [x] `tatami config show` (chemin résolu + source + workspace-path effectif)
- [x] Erreurs typées (`thiserror`), `warnings = "deny"`, rustfmt, clippy
- [x] Crate lib + bin (`src/lib.rs`, intégration tests via la lib)
- [x] Adaptateur jj : trait `Jj`, impl `CliJj` subprocess, `JjError` typé
- [x] Harness tests : `TempRepo` (vrai `jj git init`, config auteur isolée `JJ_CONFIG`)
- [x] État interne : `state::state_dir` (hôte + pointeur, chemins canoniques)
- [x] CI GitHub Actions : fmt, clippy, tests rapides (macOS), suite complète
      avec vrai jj (Linux musl)

### M1 — list + switch + shell ✅ (2026-11-21)

- [x] Adaptateur `workspaces()` : JSONL via `json({...})` dans `-T`
      (`WorkspaceInfo` : name, root, is_current, wc{change, description,
      empty, conflict, divergent})
- [x] `tt list` : table alignée (marqueur courant `*`, change, description,
      flags empty/conflict/divergent, racine + marque `(missing)` si absente
      du disque)
- [x] `state::WorkspaceState` (trait) + `FileState` (`.jj/tatami/
      previous_workspace`) — partagé entre workspaces via le pointeur,
      prouvé en intégration
- [x] `tt switch [nom|-]` : résolution, enregistrement du workspace quitté,
      no-op sur place sans toucher l'état ; picker numéroté sans argument
      (menu sur **stderr**, choix sur stdin, une tentative) ; stdout ne porte
      **que le chemin** (contrat du wrapper shell)
- [x] `tt config shell install/uninstall` zsh/bash : bloc marqué
      (`# >>> tatami shell integration >>>`) dans `~/.zshrc`/`~/.bashrc`,
      idempotent (ré-install = remplacement), uninstall ne retire que le bloc ;
      `tt` intercepte `switch` et fait le `cd`, tout le reste passe au binaire
- [x] Complétions : zsh `_tatami` dans `$XDG_CONFIG_HOME/tatami/completions`
      (fpath + compinit dans le bloc), bash dans
      `~/.local/share/bash-completion/completions/tatami` ; `tt config shell
      completion <zsh|bash>` pour un câblage manuel
- [x] Preuve end-to-end : vrai zsh **et** vrai bash sourcent le rc installé
      dans un HOME factice et font le `cd` (tests `shell_integration_should`,
      skip propre si l'interpréteur manque ; CI ubuntu installe zsh)
- [x] `Cli` déplacé en lib (`src/cli.rs`) — source unique binaire +
      complétions (`clap_complete`)

Notes épinglées (jj 0.47.1 + shells) :

- **Stale** : `jj workspace list` / `jj log` fonctionnent depuis un workspace
  stale (exit 0, vérifié) — seules les commandes qui snapshot échouent, et jj
  affiche lui-même `use jj workspace update-stale`. Donc **pas de gestion
  stale** dans `tt list`/`tt switch` ; un indicateur « stale » dans `tt list`
  reste une idée M5.
- Switch = le binaire imprime le chemin, **le shell fait le `cd`** — sans le
  wrapper, `tt switch` ne déplace personne (vérifié en smoke test).
- Picker minimal numéroté (une seule tentative, EOF/choix invalide = annulé).
  Un vrai picker interactif (fzf-like ou curseur) reste possible plus tard ;
  `switch.picker.pager` du schéma worktrunk s'appliquera à ce moment-là.
- **`zsh -i -c` ne source pas `~/.zshrc`** (vérifié) — les tests prouvent le
  contenu du bloc via `source` explicite ; le comportement des sessions
  zsh interactives réelles reste celui documenté par zsh.
- Complétions zsh `compinit` : le bloc appelle `compinit` sans dump dédié —
  si un conflit de dump apparaît chez un utilisateur, passer à
  `compinit -d …/tatami/zcompdump` (flag: cost = léger coût de startup
  dupliqué, when = premier retour utilisateur).

### M2 — create + remove ✅ (2026-11-22)

- [x] `tt switch --create <nom>` (parité worktrunk, pas de `tt create`
      autonome) : `jj workspace add --name` + bookmark homonyme (`jj bookmark
      create <nom> -r <nom>@`) ; `jj workspace add` exige que le parent de la
      destination existe (vérifié) → tatami crée les parents ; re-listing après
      création avant le switch
- [x] Résolution de destination : défaut `../<repo>.<workspace>` (parité
      worktrunk sourcée) ; template `workspace-path` (fallback
      `worktree-path`) avec expansion minimale `{{ workspace }}`/`{{ branch }}`
      (+ `| sanitize` : hors `[A-Za-z0-9._-]` → `-`), `~/…`, chemins absolus,
      relatifs résolus depuis la racine courante. `shortcut:` moteur complet
      (minijinja, tous filtres/variables) en M4
- [x] Noms validés (lettres/chiffres `-_/ .`, pas de `-`/`.` initial) — ils
      servent aussi de nom de bookmark
- [x] **jj exclut automatiquement les nested workspaces du snapshot** du
      workspace englobant (vérifié 0.47.1 : `jj st` propre avec `.wt/inner`
      existant) → template in-repo sûr sans configuration
- [x] `tt remove <nom>` : refus du workspace courant (tatami ne déplace
      personne), refus dur du workspace **hôte** (détecté via
      `state::hosts_storage` — `.jj/repo` dossier vs pointeur ; PLAN §5),
      `jj workspace forget` + répertoire déplacé dans
      `<state_dir>/trash/<nom>-<millis>` (**poubelle** : les fichiers ignorés
      type `.env` n'existent que là ; EXDEV = suppression en place), horloge
      injectée (`now_millis`), `--delete-bookmark` (jj sort 0 sur un bookmark
      absent → le booléen de suppression réelle se lit dans le stderr de jj,
      flag `shortcut:`) ; pas de `--force` anti-« dirty » : le working copy jj
      est toujours un commit — le contrôle d'intégration (commits non
      fusionnés) vient avec M3 où la résolution du bookmark principal sera de
      toute façon nécessaire
- [x] Hooks pre/post-start **exécutés** (slice faite) : formes string/table/
      pipeline typées (`CommandSpec` untagged, les clés des commandes non
      encore implémentées comme `pre-commit` restent unparsed) ; ordre sourcé
      worktrunk = pre-switch (bloquant, source) → création → pre-start
      (bloquant, nouveau workspace) → post-start (détaché) + post-switch
      (détaché, destination) ; `pre-remove` bloquant dans le répertoire condamné
      avant forget, `post-remove` détaché dans le répertoire appelant ;
      **rollback** de la création si pre-start échoue (forget + rm du répertoire
      neuf) ; switch sur place = post-switch seul (rien ne bouge)
      - `shortcut:` pre-* séquentiels (la vraie concurrence n'importe que
        pour les process longs = post-*), post-* **détachés** (spawn sans
        wait, pas de détection d'échec, sortie sur le terminal de l'appelant —
        routage vers `state/logs/` en M4), variables `{{ workspace }}` /
        `{{ workspace_path }}` seulement (minijinja complet M4), via `sh -c`
        (unix ; Windows plus tard)
      - **Config projet + approbations : FAIT** — `.config/tatami.toml` →
        fallback `.config/wt.toml` (le fichier tatami gagne, pas de mélange) ;
        hooks = user d'abord puis projet (accumulation sourcée) ; chaque
        commande de hook projet est soumise au gate **avant que la moindre
        commande projet ne tourne** ; clé = (racine hôte canonique = projet,
        phase, template brut — changer la commande ou déplacer le repo
        ré-arme l'approbation, comportement worktrunk) ; `~/.config/tatami/
        approvals.toml` (écriture toujours côté tatami), lecture avec
        fallback worktrunk ; `--yes` global contourne sans enregistrer ;
        EOF/prompt refusé = fail-closed (pre-* aborte, post-* saute)
        - le gate **possède** ses canaux IO (Box sur les handles std) et
          voyage séparé de `Hooks` — sinon deadlock : le lock interne de
          `Stdin` n'est **pas** réentrant (moralité : jamais `.lock()` tenu
          en travers d'un second lecteur du même flux)
        - raccourci : identité projet = chemin (l'identité remote
          `host/owner/repo` et la table `[projects]` du user config restent
          à typer — clés unparsed en attendant) ; pas encore de
          `tt config approvals add` (les CI passent par `--yes`)
      - question ouverte à sourcer avant la config projet : le statut exact de
        la clé `env` dans les tables de hooks worktrunk (variable
        d'environnement vs commande nommée)

### Prochaine étape

1. **M3 — merge + rebase + step** : workflow `tt merge` jj-isé (résolution du
   bookmark principal, squash/rebase/ff, push), `tt rebase`, `tt step`
   (commit, squash, rebase, push, diff, for-each, prune, `copy-ignored`)
2. **M4 — finition** : aliases, minijinja complet (variables/filtres hooks +
   `workspace-path`), `--format json`, `tt config state` (+ purge du trash),
   logs des hooks détachés, `tt config approvals add`, complétions étendues

### Contrats jj épinglés pour `tt list` (jj 0.47.1, docs templates v0.47.1)

- `jj workspace list` n'a **pas** de `--format` ; le contrat passe par `-T` :
  `json({...}) ++ "\n"` → JSONL parsé en `serde_json` (drift de version =
  `JjError::ParseOutput` explicite).
- `WorkspaceRef` : `name`, `current_workspace`, `root` (Option — None pour les
  workspaces créés avant jj 0.38), `target` (le commit de working copy, avec
  `.empty()/.conflict()/.divergent()/.description()/.change_id()`).
- Le mot-clé `working_copy` n'existe pas (c'est `target`) ; `path` non plus
  (c'est `root`).
- `description()` se termine par `\n` si non vide → `trim_end()` dans le
  template.
- Stale : pas de flag dans les templates — reporté à `tt switch` (réaction à
  l'erreur jj), pas à `tt list`.
