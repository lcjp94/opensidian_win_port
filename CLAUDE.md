# CLAUDE.md — Opensidian sous Windows 11

> **Lire aussi `AGENTS.md` (règles de l'amont) avant toute modification.** En cas de conflit, ce fichier prime uniquement pour ce qui concerne Windows.

## 1. Base de travail (depuis le 2026-11-24)

- **Base : le portage de Morglaf**, PR [jpzk/opensidian#34](https://github.com/jpzk/opensidian/pull/34), branche `platform-windows` de `Morglaf/opensidian`, sur **v0.9.6**. Le mainteneur (jpzk) le relit ; Morglaf a accepté nos contributions et nous a proposé de mener le chantier.
- **Branche de travail : `windows`** (ce dépôt, `lcjp94/opensidian_win_port`) = `morglaf/platform-windows` + nos PR encore ouvertes chez Morglaf, fusionnées + ce fichier et `windows-port/`.
- **Ancien portage** (v0.9.2, branche `port/windows`) : archivé sous le tag `port-windows-v0.9.2`. C'est la source des pièces à reporter (`windows-port/README.md`, « À reporter »). Son audit complet : `git show port-windows-v0.9.2:windows-port/AUDIT.md`.
- Application de notes Markdown compatible coffres Obsidian. Rust + **Tauri v2**, frontend **JS pur** dans `ui/` (**pas de npm, pas de bundler**). Crate unique `src-tauri/`. Toolchain **Rust 1.98.0**. Licence GPL-3.0-or-later.

## 2. Dépôts et remotes

| Remote | Dépôt | Rôle |
|---|---|---|
| `origin` | `lcjp94/opensidian_win_port` | ce dépôt : branche `windows`, CI de contrôle `ci/*`, tag d'archive |
| `fork` | `lcjp94/opensidian` | fork de jpzk : têtes des PR (`contrib/*`) |
| `morglaf` | `Morglaf/opensidian` | la base : `platform-windows` |
| `upstream` | `jpzk/opensidian` | l'amont : `develop` (contributions), `main` (versions) |

## 3. Flux de travail

1. **Une contribution = une branche `contrib/<sujet>` partant de `morglaf/platform-windows`** (pas de `windows`), petite et indépendante. Commits explicatifs (le *pourquoi*).
2. Tester sous Windows (§5), puis sous Linux via la CI de contrôle : pousser la branche + `windows-port/contrib-ci.yml` sous `.github/workflows/` sur `origin` en `ci/<sujet>` (branche jetable, jamais dans une PR).
3. Pousser `contrib/<sujet>` sur `fork`, ouvrir la PR vers `Morglaf/opensidian:platform-windows` (`gh pr create --repo Morglaf/opensidian --base platform-windows --head lcjp94:contrib/<sujet>`). Description : pourquoi, ce qui change, ce qui est vérifié et ce qui ne l'est pas, mention « written with Claude Code and reviewed and tested by me ».
4. Fusionner la branche dans `windows` (`git merge --no-ff`) pour continuer à travailler avec.
5. **Quand Morglaf met à jour sa branche** : `git fetch morglaf` puis `git merge morglaf/platform-windows` dans `windows`. Une fois une de nos PR fusionnée chez lui, sa branche la contient déjà : rien à faire.
6. **Correctif générique** (pas propre à Windows) : PR vers `jpzk/opensidian:develop`, branche partant de `upstream/develop`.
7. Ne rien publier (commentaire, PR, ticket) au nom de l'utilisateur sans son accord explicite.

## 4. Règles (issues d'AGENTS.md, à respecter)

1. **Clean room** : jamais de code/CSS/valeurs/assets d'Obsidian.
2. **En-tête SPDX** `SPDX-License-Identifier: GPL-3.0-or-later` dans tout nouveau `.rs .js .html .css .sh` (exempt : `src-tauri/themes/`). Thèmes embarqués **byte-exact**, épinglés par sha256 (`builtins.rs`).
3. **Dépendances** : aucune nouvelle crate sauf nécessité. Épingler avec `=`, préférer les crates déjà dans `Cargo.lock`, jamais de release < 6 semaines. Vérifier que `Cargo.lock` ne gagne aucun paquet (comparer le nombre de `name = ` avant/après). Tout ajout : justifié dans le commentaire `Cargo.toml` et proposé à l'utilisateur **avant**.
4. **Toujours `--locked`**. Pas de npm, pas de Python.
5. **Sécurité du coffre** : ne jamais écrire hors du coffre lié et de la config utilisateur ; les clés inconnues de `.obsidian/*.json` sont préservées à l'identique.
6. **Commits** : petits, explicatifs, jamais sur `main`. **Aucun trailer `Co-Authored-By` ni « Generated with »** dans les commits. Ne pas modifier `git config` ; utiliser l'identité déjà configurée. Ne jamais committer `target/`, `progress.md`, `.goalenv`, des captures. Sous `scripts/ docs/ notes/ bin/` : ne toucher qu'aux fichiers que l'amont suit déjà (le reste y est privé, voir AGENTS.md).
7. **Linux et macOS intacts** : tout code spécifique via `#[cfg(windows)]`, `#[cfg(unix)]`, `#[cfg(target_os = "…")]` ou `[target.'cfg(...)'.dependencies]`. Après chaque changement, Linux doit rester vert (CI de contrôle).
8. **Pas d'invention** : un comportement non vérifié est noté « non vérifié ». Petit changement + test qui échoue avant / passe après. En cas de doute, poser **une** question ciblée.

## 5. Environnement Windows

- Windows 11, PowerShell. Rust **1.98.0** MSVC, Visual Studio Build Tools, WebView2 (préinstallé). Tauri CLI v2 seulement pour un installateur. **Pas de Node.**
- Dépôt sous **OneDrive** : compiler hors OneDrive avec `CARGO_TARGET_DIR=C:\cargo-target\opensidian`. Les copies de travail annexes vont dans `C:\cargo-target\wt\`.
- Git for Windows met `core.autocrlf=true` : le `.gitattributes` de la base garde LF. Après un changement de `.gitattributes`, recharger : `git rm -r --cached . ; git reset --hard`.
- L'antivirus (McAfee) verrouille parfois l'exécutable au lien : erreur `LNK1104`. Relancer la commande.
- Pour lancer l'app sur un coffre de test : `VAULT_DIR=<coffre>`, `WEBVIEW2_USER_DATA_FOLDER=<profil jetable>` ; la config est lue dans `%USERPROFILE%\.opensidian.json` (la sauvegarder avant un essai et la restaurer après).

```powershell
cd src-tauri
$env:CARGO_TARGET_DIR = "C:\cargo-target\opensidian"
cargo build --release --locked
cargo test --locked
```

## 6. Pièges connus

- WebView2 sert les schémas personnalisés en `http://<schéma>.localhost/` : CSP et URL côté `ui/` en tiennent compte.
- `cargo clippy --all-targets` s'arrête sur `consent.rs` (`while !state.done`), code de la base, signalé à Morglaf dans sa PR #2.
- Les tests `listtoggle` / `procstdio` utilisent JavaScriptCore : Linux seulement.
