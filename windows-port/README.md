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
| [Morglaf/opensidian#10](https://github.com/Morglaf/opensidian/pull/10) | Plugins : `win32`, `exec()`/`shell: true` via `cmd.exe /d /s /c` en verbatim, la confirmation montre la ligne exacte | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#11](https://github.com/Morglaf/opensidian/pull/11) | Coffre passé en argument sans `\\?\`, un seul récent par coffre, nom du coffre coupé sur `\` ; aucune clé de config ne change | ouverte, fusionnée dans `windows` |
| [jpzk/opensidian#262](https://github.com/jpzk/opensidian/pull/262) | images SVG | acceptée par le mainteneur, à fusionner chez l'amont |
| [jpzk/opensidian#263](https://github.com/jpzk/opensidian/pull/263) | PDF 3D : avis + lien vers le plan 2D | en relecture chez l'amont |
| [jpzk/opensidian#359](https://github.com/jpzk/opensidian/pull/359) | canevas : « Back to previous view » après « Zoom to selection » | ouverte |
| [jpzk/opensidian#360](https://github.com/jpzk/opensidian/pull/360) | canevas : glisser sur le fond déplace la vue, Maj+glisser sélectionne | ouverte |

Quand une PR est fusionnée chez Morglaf ou l'amont, la retirer de ce tableau.

## À reporter depuis l'ancien portage

Toutes les pièces du tag `port-windows-v0.9.2` sont reportées (#5 à #11), le 2026-11-24.

Reste ouvert, non traité :
- La confirmation d'un programme affiche `Program: \\?\C:\Windows\System32\cmd.exe` (forme verbatim). Texte de sécurité : à décider à part.
- Les clés internes (confiance, secrets, taille de fenêtre, id de coffre) restent en forme `\\?\` : cohérentes entre elles, donc laissées telles quelles (#11).

## Version installée sur le poste

Branche locale `local/windows-svg` (non poussée), worktree `C:\cargo-target\wt\local-svg` : `windows` + #262 + #263 + #359 + #360, plus deux correctifs Windows qui manquent encore dans #262 et #263 (chemin du plan 2D en `/`, tests SVG sur l'adresse WebView2 et test du lien symbolique réservé à Unix). Installateur : Inno Setup 6.7.3 (par utilisateur), `ISCC` comme dans `windows.yml`. Signalé en commentaire sur #262 et #263 le 2026-11-24 (pour celle des deux, PR ou #34, qui arrive en second).

## Limites connues (à documenter dans la PR finale)

- Pas de sandbox sous Windows (Landlock est propre à Linux).
- `fs.chmod` des plugins : sans effet sous Windows.
- Le lancement d'un programme approuvé est moins strict que sous Linux (pas d'exec par descripteur), voir l'en-tête de `desk_proc/windows.rs`.

## Outils

- `contrib-ci.yml` : workflow de contrôle Linux pour une branche de contribution. À copier sous `.github/workflows/` sur une branche jetable `ci/<sujet>` poussée sur `origin`, jamais dans une PR.
