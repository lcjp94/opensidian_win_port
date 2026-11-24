# windows-port — suivi du travail Windows

Base : le portage de Morglaf ([jpzk/opensidian#34](https://github.com/jpzk/opensidian/pull/34), v0.9.6). Flux de travail et règles : `CLAUDE.md` à la racine.

## PR en cours

| PR | Sujet | État |
|---|---|---|
| [Morglaf/opensidian#1](https://github.com/Morglaf/opensidian/pull/1) | `.gitattributes` : LF dans les clones Windows (19 tests en échec → 0) | ouverte, fusionnée dans `windows` |
| [Morglaf/opensidian#2](https://github.com/Morglaf/opensidian/pull/2) | WinHTTP au lieu de `reqwest` + rustls (−41 paquets) | ouverte, fusionnée dans `windows` |
| [jpzk/opensidian#262](https://github.com/jpzk/opensidian/pull/262) | images SVG | acceptée par le mainteneur, à fusionner chez l'amont |
| [jpzk/opensidian#263](https://github.com/jpzk/opensidian/pull/263) | PDF 3D : avis + lien vers le plan 2D | en relecture chez l'amont |

Quand une PR est fusionnée chez Morglaf ou l'amont, la retirer de ce tableau.

## À reporter depuis l'ancien portage

Source : le tag `port-windows-v0.9.2` (`git show port-windows-v0.9.2:<chemin>`). Une ligne = une PR vers Morglaf, dans cet ordre de priorité. Rien de cette liste n'est encore vérifié sur la base v0.9.6.

1. **Noms Windows dans `vaultfs`** : refuser `:` (flux de données NTFS), les noms réservés (`CON`, `NUL`, `COM1`…, `LPT1`…), `< > " | ? *`, le point ou l'espace final, dans les chemins de l'app (`parts()` de `vaultfs/windows.rs`), pas seulement ceux des plugins. Source : `src-tauri/src/vaultfs_win.rs` et ses tests.
2. **CSP limitée à Windows** : `src-tauri/tauri.windows.conf.json` avec les sources `http://*.localhost`, et la CSP de base remise comme dans l'amont ; test qui lie les deux. Source : `tauri.windows.conf.json`, test `windows_conf_guard`.
3. **CI Windows** : job `windows-latest` (build + `cargo test --locked`) à côté de ceux de l'amont. Source : `.github/workflows/build.yml`.
4. **Installateur** : NSIS par utilisateur, non signé, `SHA256SUMS`, `icon.ico`, configuration séparée pour que `bundle.active` reste `false`. Source : `packaging/windows/`, `.github/workflows/build.yml`.
5. **Secrets des plugins** dans le Credential Manager (2560 octets max). Source : `src-tauri/src/secrets_win.rs`.
6. **Sélecteur de dossier natif** (IFileOpenDialog) pour choisir un coffre. Source : `src-tauri/src/pick_win.rs`, `ui/main.js` (`winBrowse`).
7. **Fenêtre** : glisser de la barre de titre, Aero Snap, bords de redimensionnement. La base a un chemin générique `start_dragging` : **vérifier d'abord** ce qu'il donne sous Windows.
8. **Plugins** : `process.platform === "win32"`, `os.platform()`/`os.type()`, `exec()` et `shell: true` via `cmd.exe /d /s /c` avec arguments passés tels quels (`windowsVerbatimArguments`). Source : `ui/plugins/node-desktop.js`, `desk_proc.rs` (`verbatim`) — à adapter au `desk_proc` v0.9.6 (consentement).
9. **Chemin canonique sans `\\?\`** pour le coffre passé en argument (sinon le sélecteur le refuse). Source : `plat::canon`. Vérifier si la base a déjà l'équivalent (`plain` dans `desk_proc/windows.rs`).

## Limites connues (à documenter dans la PR finale)

- Pas de sandbox sous Windows (Landlock est propre à Linux).
- `fs.chmod` des plugins : sans effet sous Windows.
- Le lancement d'un programme approuvé est moins strict que sous Linux (pas d'exec par descripteur), voir l'en-tête de `desk_proc/windows.rs`.

## Outils

- `contrib-ci.yml` : workflow de contrôle Linux pour une branche de contribution. À copier sous `.github/workflows/` sur une branche jetable `ci/<sujet>` poussée sur `origin`, jamais dans une PR.
