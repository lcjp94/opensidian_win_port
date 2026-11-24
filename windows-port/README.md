# windows-port — suivi du travail Windows

Base : le portage de Morglaf ([jpzk/opensidian#34](https://github.com/jpzk/opensidian/pull/34), v0.9.6). Flux de travail et règles : `CLAUDE.md` à la racine.

## PR en cours

| PR | Sujet | État |
|---|---|---|
| [Morglaf/opensidian#1](https://github.com/Morglaf/opensidian/pull/1) | `.gitattributes` : LF dans les clones Windows (19 tests en échec → 0) | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#2](https://github.com/Morglaf/opensidian/pull/2) | WinHTTP au lieu de `reqwest` + rustls (−41 paquets) | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#3](https://github.com/Morglaf/opensidian/pull/3) | `vaultfs` : refus des noms que Windows ne gère pas (`CON`, `:`, point final…) | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#4](https://github.com/Morglaf/opensidian/pull/4) | CSP élargie sous Windows seulement ; « Render media » coupé retire aussi l'origine WebView2 | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#5](https://github.com/Morglaf/opensidian/pull/5) | CI Windows (`windows.yml`) : build + `cargo test` sur `windows-2025-vs2026`, durcie comme `macos.yml` | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#6](https://github.com/Morglaf/opensidian/pull/6) | Installateur par utilisateur (Inno Setup, sans bundler Tauri), construit et testé dans `windows.yml` ; s'appuie sur #5 | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#7](https://github.com/Morglaf/opensidian/pull/7) | `app.secretStorage` dans le Credential Manager (2560 octets max) ; s'appuie sur #2 | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#8](https://github.com/Morglaf/opensidian/pull/8) | Choix du coffre dans le dialogue de dossier Windows (IFileOpenDialog), crate `windows` déjà dans le lock | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#9](https://github.com/Morglaf/opensidian/pull/9) | Barre de titre : le glisser passe à Windows (Aero Snap, Snap Layouts, agrandir en haut) | ouverte, fusionnée dans `windows` |
| [jpzk/opensidian#262](https://github.com/jpzk/opensidian/pull/262) | images SVG | acceptée par le mainteneur, à fusionner chez l'amont |
| [jpzk/opensidian#263](https://github.com/jpzk/opensidian/pull/263) | PDF 3D : avis + lien vers le plan 2D | en relecture chez l'amont |

Quand une PR est fusionnée chez Morglaf ou l'amont, la retirer de ce tableau.

## À reporter depuis l'ancien portage

Source : le tag `port-windows-v0.9.2` (`git show port-windows-v0.9.2:<chemin>`). Une ligne = une PR vers Morglaf, dans cet ordre de priorité. Rien de cette liste n'est encore vérifié sur la base v0.9.6.

1. **Plugins** : `process.platform === "win32"`, `os.platform()`/`os.type()`, `exec()` et `shell: true` via `cmd.exe /d /s /c` avec arguments passés tels quels (`windowsVerbatimArguments`). Source : `ui/plugins/node-desktop.js`, `desk_proc.rs` (`verbatim`) — à adapter au `desk_proc` v0.9.6 (consentement).
2. **Chemin canonique sans `\\?\`** pour le coffre passé en argument (sinon le sélecteur le refuse). Source : `plat::canon`. Vérifier si la base a déjà l'équivalent (`plain` dans `desk_proc/windows.rs`). Attention : la clé du coffre (`plugins::trust_key`, avec `\\?\`) nomme aussi les secrets (#7) et les entrées de `~/.opensidian.json` ; si elle change, migrer l'existant. Vu en testant #8 : la liste des coffres récents montre le même coffre deux fois (`\\?\D:\…` et `D:\…`), et son nom affiche le chemin entier (`base()` de `ui/main.js` ne coupe que sur `/`).

## Limites connues (à documenter dans la PR finale)

- Pas de sandbox sous Windows (Landlock est propre à Linux).
- `fs.chmod` des plugins : sans effet sous Windows.
- Le lancement d'un programme approuvé est moins strict que sous Linux (pas d'exec par descripteur), voir l'en-tête de `desk_proc/windows.rs`.

## Outils

- `contrib-ci.yml` : workflow de contrôle Linux pour une branche de contribution. À copier sous `.github/workflows/` sur une branche jetable `ci/<sujet>` poussée sur `origin`, jamais dans une PR.
