# Tatami

[![CI](https://github.com/a-trigueros/tatami/actions/workflows/ci.yml/badge.svg)](https://github.com/a-trigueros/tatami/actions/workflows/ci.yml)

[Worktrunk](https://github.com/max-sixty/worktrunk) pour
[jujutsu](https://jj-vcs.dev) : un wrapper de haut niveau autour de `jj` pour
gérer des **workspaces** — création, navigation, intégration, nettoyage — avec
hooks, aliases, approbations et intégration shell.

```text
tt switch feature          # cd dans le workspace `feature` (le wrapper fait le cd)
tt switch --create login   # crée le workspace + bookmark homonyme, puis cd
tt list                    # tableau des workspaces (statuts jj)
tt merge                   # intègre le workspace courant dans main (rebase + ff)
tt remove login            # oublie + met à la poubelle
```

## Installation

Prérequis : [jj](https://jj-vcs.dev) et une toolchain Rust récente. Les
contrats jj sont épinglés empiriquement sur **0.47.1** (voir `PLAN.md`) —
revalider avant toute montée de version de jj.

```text
cargo install --locked --git https://github.com/a-trigueros/tatami
# ou, depuis un clone :
cargo install --path .
```

## Intégration shell

```text
tatami config shell install       # zsh ou bash (détecté via $SHELL, --shell pour forcer)
exec zsh                          # recharger le shell interactif
```

Le wrapper `tt` intercepte `switch` et `merge --remove` (les deux impriment le
chemin de destination ; le wrapper fait le `cd`) et passe tout le reste au
binaire. Sans wrapper, `tt switch` imprime le chemin sans déplacer personne.

Sur macOS, `zsh -i -c` ne source pas `~/.zshrc` : tester dans un terminal
interactif, pas via `-c`.

## Configuration

`~/.config/tatami/config.toml`, sinon **fallback** sur
`~/.config/worktrunk/config.toml` — même schéma que worktrunk :

```toml
workspace-path = ".wt/{{ workspace | sanitize }}"

[post-start]
server = "npm run dev"

[aliases]
hello = "echo bonjour {{ workspace }} - {{ args }}"

[projects."github.com/owner/repo"]
pre-start = "cp .env.example .env"   # soumis à approbation
```

- variables : `{{ workspace }}`, `{{ workspace_path }}`, `{{ repo_path }}`,
  `{{ cwd }}`, `{{ args }}` (aliases) ; filtres : `sanitize` (seuls `/` et `\`
  → `-`), `sanitize_db`, `codename` ;
- `--format json` sur `tt list` pour le machine-readable ;
- hooks projet (`[[pre-start]]`…) soumis à **approbation** au premier run
  (`tt config approvals add` pour les pré-approuver, `--yes` pour contourner
  une invocation) ;
- `[step.copy-ignored]` : `tt step copy-ignored` copie les fichiers
  git-ignorés du workspace primaire vers le courant (`.worktreeinclude`
  filtre, `--force` écrase).

## État interne

`.jj/tatami/` dans le workspace hôte du stockage partagé (voir `PLAN.md`) :
workspace précédent (`tt switch -`), poubelle (`tt config state clear
--force`), logs des hooks détachés (`logs/`), approbations dans
`~/.config/tatami/approvals.toml`.

## Développement

```text
cargo test            # tier rapide + preuves sur vrai jj (le binaire doit être installé)
cargo clippy --all-targets
```

CI : fmt, clippy, tests complets avec vrai `jj` (Linux), tier rapide (macOS).
La version jj de référence est verrouillée dans `.github/workflows/ci.yml`.

`PLAN.md` documente les décisions, les contrats jj épinglés empiriquement et
l'avancement.
