# CLAUDE.md — Portage Windows 11+ d'Opensidian

> Dépôt : https://github.com/lcjp94/opensidian_win_port (fork de `jpzk/opensidian`, v0.9.2)
> Copie locale : `C:\Users\jp\OneDrive\Documents\Dev\Antigravity\Opensidian_win_port`
> **Lire aussi `AGENTS.md` (règles de l'amont) avant toute modification.** En cas de conflit, ce fichier prime uniquement pour ce qui concerne le portage Windows.

## 1. Contexte (vérifié dans le dépôt)

- Application de notes Markdown compatible coffres Obsidian. Rust + **Tauri v2**, frontend **JS pur** dans `ui/` (**pas de npm, pas de bundler** : `build.frontendDist = "../ui"`).
- Crate unique `src-tauri/` (`opensidian`, lib `opensidian_lib` + bin). `Cargo.lock` dans `src-tauri/`. Toolchain épinglée : **Rust 1.98.0**. Licence GPL-3.0-or-later.
- `tauri.conf.json` : `bundle.active = false` (aucun installateur Tauri aujourd'hui), fenêtre **sans décorations** (`decorations: false`, barre de titre dessinée par l'UI), CSP avec schémas personnalisés `opensidian-plugin:` et `opensidian-img:`.
- Cible actuelle : Linux x86_64 uniquement. **Objectif : Windows 11 x86_64 (MSVC), sans régression Linux.**
- Architecture clé : un process par coffre (`spawn.rs`, `spawner.rs`, `vaultlock.rs`), config partagée `~/.opensidian.json` (`cfgstore.rs`), watcher par polling (`watcher.rs`), index en mémoire (`index.rs`), sandbox Landlock optionnelle (`sandbox.rs`), stockage de secrets via Secret Service (`secrets.rs`), magasin de plugins (`plugin_store.rs`), API plugins (`plugins.rs`, `desk_http.rs`, `desk_net.rs`, `desk_proc.rs`), fenêtre (`winsize.rs`), collage d'images (`pasteimg.rs`), mise à jour signée OpenPGP (`updates.rs`).

## 2. Règles (issues d'AGENTS.md, à respecter)

1. **Clean room** : jamais de code/CSS/valeurs/assets d'Obsidian.
2. **En-tête SPDX** `SPDX-License-Identifier: GPL-3.0-or-later` dans tout nouveau `.rs .js .html .css .sh` (exempt : `src-tauri/themes/`). Thèmes embarqués **byte-exact**, épinglés par sha256 (`builtins.rs`) : ne jamais les modifier — attention à `.gitattributes` / conversion CRLF (voir §5.3).
3. **Dépendances** : « aucune nouvelle crate sauf nécessité ». Épingler avec `=`, préférer les crates déjà dans `Cargo.lock`, jamais de release < 6 semaines, jamais de dépendance non épinglée. Tout ajout (clé Windows en §5) doit être **justifié par écrit dans `THIRD-PARTY.md`** (auteur, version, licence, date, sha) et proposé à l'utilisateur **avant** d'être ajouté.
4. **Toujours `--locked`**. Pas de npm, pas de Python.
5. **Sécurité du coffre** : ne jamais écrire hors du coffre lié et de la config utilisateur ; les clés inconnues de `.obsidian/*.json` doivent être préservées à l'identique.
6. **Commits** : petits, explicatifs (le *pourquoi*), sur la branche `port/windows` (jamais `main`). **Aucun trailer `Co-Authored-By` ni « Generated with »** (désactiver ceux de Claude Code). Ne pas modifier `git config`; utiliser l'identité déjà configurée par l'utilisateur. Ne jamais committer `target/`, `progress.md`, `.goalenv`, captures, scripts `.sh` hors `packaging/`, ni rien sous `scripts/ docs/ notes/ bin/` (liens symboliques vers un dépôt privé ; ils peuvent être cassés sous Windows). **Note : le rapport d'audit ne doit donc pas aller dans `docs/` — utiliser `windows-port/` à la racine** (ex. `windows-port/AUDIT.md`).
7. **Linux intact** : tout code spécifique via `#[cfg(unix)]`, `#[cfg(windows)]`, `#[cfg(target_os = "linux")]` ou `[target.'cfg(...)'.dependencies]`. Après chaque phase, `cargo check` Linux doit rester vert (via CI ubuntu ou WSL).
8. **Pas d'invention** : si un comportement n'est pas vérifié (plugin, API), le noter « non vérifié ». Petit changement + test qui échoue avant / passe après. En cas de doute, poser **une** question ciblée.

## 3. Environnement Windows

- Windows 11, PowerShell 7. Rust **1.98.0** MSVC (`rustup toolchain install 1.98.0 --profile minimal -c clippy,rustfmt`, cible `x86_64-pc-windows-msvc`), Visual Studio Build Tools (« Desktop development with C++ » + SDK Windows 11), WebView2 Runtime (préinstallé), Tauri CLI v2 (`cargo install tauri-cli --locked --version "^2"`) seulement pour l'installateur. **Pas de Node requis.**
- **OneDrive** : le dépôt est sous `OneDrive\Documents`. Risques : verrous de synchro, `target/` corrompu, lenteur, Files On-Demand. Préférer un clone hors OneDrive (ex. `C:\dev\opensidian`) ou définir `CARGO_TARGET_DIR=C:\cargo-target\opensidian`. Activer `git config core.longpaths true`.
- Les tests actuels supposent Linux : voir §5.8 pour savoir lesquels isoler.

```powershell
cd src-tauri
cargo check --locked --target x86_64-pc-windows-msvc
cargo build --release --locked
cargo test --locked
```

## 4. État de départ attendu (hypothèses à confirmer en Phase 0)

La compilation Windows **échouera** au premier `cargo check` pour au moins : `landlock`, `libc` (openat/O_NOFOLLOW), `std::os::unix::*`, `gdk/gdkx11/soup3/dbus` (déjà cfg linux dans Cargo.toml mais probablement utilisés sans `cfg` dans les sources), `[patch.crates-io] glib` (inutilisé sous Windows, peut émettre un avertissement). Confirmer par `cargo check` et par :

```powershell
git grep -n -E "std::os::unix|libc::|landlock|gdk|soup|dbus|/proc|/bin/|/tmp|/home|XDG_|FLATPAK|HOME\b|flatpak-spawn|xdg-open|O_NOFOLLOW|openat|renameat|unlinkat|0o[0-7]{3}|PermissionsExt|symlink" -- "src-tauri/src" "src-tauri/tests" "ui" "packaging"
```

## 5. Points de portage identifiés (par module)

| # | Module | Problème Linux-only | Piste Windows |
|---|--------|--------------------|---------------|
| 5.1 | `vaultlock.rs` | `std::os::unix::ffi::OsStrExt`; dossier de lock `XDG_RUNTIME_DIR`/Flatpak/`$HOME/.local/share`; clé FNV sur octets du chemin | `File::try_lock` (std) fonctionne sous Windows (LockFileEx). Remplacer `as_bytes` par `OsStrExt` Windows (`encode_wide`) ou `to_string_lossy().to_lowercase()` (NTFS insensible à la casse). Lock dir : `%LOCALAPPDATA%\dev.koto.opensidian\locks`. Chemin canonique sans préfixe `\\?\` (`dunce` seulement si déjà dans le lock ; sinon retirer le préfixe à la main). Conserver la sémantique REFUSE + `ready <token>` (`spawn.rs`). |
| 5.2 | `vaultfs.rs` | `openat/renameat/mkdirat/unlinkat` + `O_NOFOLLOW` via `libc` pour ne jamais suivre un symlink | Aucune API std équivalente. Proposer une implémentation `#[cfg(windows)]` : vérifier chaque composant (`symlink_metadata`, attribut `FILE_ATTRIBUTE_REPARSE_POINT`), refuser reparse points/jonctions, puis opérer ; documenter la **course TOCTOU résiduelle** par rapport à Linux. À valider avec l'utilisateur car touche au modèle de sécurité. |
| 5.3 | `builtins.rs`, `.gitattributes` | sha256 des thèmes byte-exact | Forcer `-text` / `binary` sur `src-tauri/themes/**` pour qu'aucun `autocrlf` ne casse les hash. Vérifier le checkout Windows. |
| 5.4 | `sandbox.rs` | Landlock (`landlock = "=0.4.7"`, `OPENSIDIAN_LANDLOCK`) | `cfg(target_os="linux")` ; déplacer `landlock` en `[target.'cfg(target_os = "linux")'.dependencies]` ; stub Windows « indisponible » (l'UI doit afficher l'option comme non supportée). Sandbox Windows (AppContainer / Job Object) **hors périmètre v1**. |
| 5.5 | `secrets.rs` | `dbus` / Secret Service | Windows Credential Manager. Nécessite probablement une **nouvelle crate** (`windows-sys`/`keyring`…) : proposer à l'utilisateur, épingler, documenter. En attendant, stub qui renvoie « unsupported » (comportement déjà prévu sans keyring). |
| 5.6 | `plugin_store.rs`, `updates.rs` | client HTTPS = `soup3` (libsoup) | Il faut un client HTTPS Windows (WinHTTP via crate déjà dans `Cargo.lock`, ou nouvelle crate justifiée). Conserver : GitHub HTTPS uniquement, allowlist/redirections, sha256 de chaque fichier. `updates.rs` : choisir l'actif Windows des releases ; la vérification `SHA256SUMS.asc` reste valable. |
| 5.7 | `plugins.rs`, `desk_http.rs`, `desk_net.rs`, `desk_proc.rs` | API plugins (processus, sockets, ouverture de fichiers/liens) ; `sh`, `xdg-open`, séparateurs | `std::process::Command` sans shell POSIX ; ouverture via `start`/ShellExecute (plugin opener Tauri si déjà présent, sinon API Windows minimale). Espaces et accents dans les chemins. |
| 5.8 | Tests | `spawn::tests::switch_alive_probe_reads_proc` (`/proc`, `/bin/true`) ; `vaultlock` test avec `sh` + `flock(1)` ; tests « listtoggle » via **JavaScriptCore** (dev-dep Linux) | Isoler en `#[cfg(unix)]`/`#[cfg(target_os="linux")]` ; écrire équivalents Windows (`cmd /c`, `timeout`/`powershell Start-Sleep`). Les tests listtoggle restent Linux ; documenter la couverture manquante côté WebView2. |
| 5.9 | `main.rs`, `winsize.rs`, UI (`ui/main.js`) | fenêtre sans décorations ; glisser via `gdk/gdkx11` (X11 vs Wayland, « win_move_proto ») ; redimensionnement | Sous Windows : `data-tauri-drag-region` / `start_dragging`, `start_resize_dragging`, boutons min/max/fermer, **Snap Layouts Windows 11** (survol du bouton agrandir), bords de redimensionnement, ombre/rayon. Vérifier DPI 100–200 %. Garder le chemin X11/Wayland sous `cfg(linux)`. |
| 5.10 | CSP + schémas personnalisés | `opensidian-plugin:` et `opensidian-img:` | **Sous Windows, WebView2 sert les schémas custom en `http://<schéma>.localhost/`** (ou `https://` si `useHttpsScheme`). La CSP actuelle (`script-src ... opensidian-plugin:`, `img-src ... opensidian-img:`) bloquera plugins et images : adapter la CSP et les URL côté `ui/` pour Windows sans l'élargir inutilement. Vérifier aussi `wasm-unsafe-eval` (hayro/pdf, plugins). **À tester en priorité.** |
| 5.11 | `cfgstore.rs`, `migrate.rs`, `appid.rs` | `~/.opensidian.json`, `$HOME` | Windows : `%USERPROFILE%\.opensidian.json` (garder la compatibilité de nom) ou `%APPDATA%`; décision à soumettre à l'utilisateur. Écriture atomique (temp + rename avec retries `ERROR_SHARING_VIOLATION`, antivirus/OneDrive). Partage entre process : tester la concurrence. |
| 5.12 | `watcher.rs`, `index.rs`, `vaultfs.rs` | polling, chemins | Normaliser séparateurs `\`→`/` pour les chemins relatifs du coffre (liens, index, graphe) et revenir au format natif aux frontières I/O. Casse : NTFS insensible à la casse, mais l'index/wikilinks ne doivent pas changer de comportement. Noms interdits (`CON`, `NUL`, `COM1`, `<>:"\|?*`, point/espace final), MAX_PATH. |
| 5.13 | `pasteimg.rs` | presse-papiers/formats | Vérifier via WebView2 ; tests manuels. |
| 5.14 | `spawn.rs`, `spawner.rs` | `flatpak-spawn`, fork/exec, fd hérités (CLOEXEC), `current_exe` | `CreateProcess` via `std::process::Command` ; empêcher l'héritage du handle de lock ; flag `CREATE_NO_WINDOW`/`DETACHED_PROCESS` si nécessaire ; arguments avec espaces. Argument « chemin du coffre » en argv. |
| 5.15 | `packaging/`, `flake.nix`, `bundle.active=false` | aucun packaging Windows | Ajouter `packaging/windows/` : NSIS via Tauri (`bundle.targets=["nsis"]`, `icon.ico`) **ou** zip portable de `opensidian.exe` + sortie de `cargo build --release --locked`. Ne pas toucher aux autres packagings. WebView2 : `downloadBootstrapper`. |
| 5.16 | `fuzz/`, `.oss-scanner/` | outillage | Vérifier que le build lib reste portable ; ignorer si Linux-only (documenter). |

## 6. Plan par phases

### Phase 0 — Préparation
- [ ] Branche `port/windows`. Lire `AGENTS.md`, `README.md`, `.github/workflows/*`, `packaging/`, `.gitattributes`, `THIRD-PARTY.md`.
- [ ] Baseline : `cargo check --locked --target x86_64-pc-windows-msvc`, noter **toutes** les erreurs dans `windows-port/AUDIT.md` (pas sous `docs/`).
- [ ] Traiter OneDrive (§3).

### Phase 1 — Audit (aucun changement de code source)
Compléter §5 avec fichier:ligne réels, valider/infirmer chaque hypothèse, classer par risque, estimer l'effort, lister les nouvelles crates nécessaires. **S'arrêter et présenter le plan à l'utilisateur** (décisions : §5.2, §5.5, §5.6, §5.11).

### Phase 2 — Compilation Windows
`cfg` + déplacement des dépendances Linux sous `[target.'cfg(target_os = "linux")'.dependencies]`, stubs Windows explicites (jamais `todo!()` atteignable). Critère : `cargo check --locked` OK sur Windows **et** Linux ; `cargo clippy -- -D warnings` si le dépôt l'applique.

### Phase 3 — Cœur fonctionnel
Dans l'ordre : 5.11 config → 5.1 lock → 5.14 spawn → 5.2 vaultfs → 5.12 chemins/watcher → 5.3 thèmes. Chaque étape avec tests Windows (`tempfile`/`std::env::temp_dir`, sous `#[cfg(windows)]`).

### Phase 4 — Interface et plateforme
5.9 fenêtre, 5.10 CSP/schémas, 5.7 plugins/processus, 5.13 presse-papiers, 5.5 secrets, 5.6 réseau. Lancer `cargo run` (ou `cargo build --profile devfast --features devui`) et valider sur une **copie** d'un coffre réel : ouverture, édition/sauvegarde sans diff parasite (git sur le coffre de test), liens/backlinks/tags/graphe/canvas, thèmes + snippets, un plugin communautaire (ex. Excalidraw), AZERTY/accents/IME, DPI, thème sombre/clair, multi-fenêtres (un coffre = un process).

### Phase 5 — Packaging (5.15) et CI
Workflow GitHub Actions `windows-latest` (à écrire après lecture des workflows existants ; **les étapes npm sont à proscrire**, aucun Node) :
1. checkout ; toolchain **1.98.0** MSVC ; `swatinem/rust-cache@v2` (`workspaces: src-tauri -> target`) ;
2. `cargo build --release --locked` + `cargo test --locked` (tests Windows) dans `src-tauri/` ;
3. bundle NSIS (tauri-cli) ou zip portable ;
4. signature Authenticode **optionnelle** (secrets `WINDOWS_CERTIFICATE*`, import `.pfx` seulement si le secret existe, jamais sur PR de forks) ;
5. `SHA256SUMS` des artefacts Windows ; upload d'artefact.
Job `ubuntu` de non-régression : `cargo build/test --locked` avec les paquets webkit (voir README). Respecter la règle « agents ne détiennent jamais la clé de signature » et la signature GPG de l'amont : ne pas la remplacer.

### Phase 6 — Documentation et amont
`windows-port/README.md` (prérequis, build, limites : sandbox absent, secrets, plugins partiels), section Windows dans `README.md`, PR atomiques vers `jpzk/opensidian` pour les correctifs `cfg` génériques.

## 7. Définition de « terminé » (v1 Windows)
1. `cargo build --release --locked` + `cargo test --locked` verts sur Windows et Linux.
2. Un exécutable/installateur démarre sur Windows 11 vierge (WebView2 inclus), s'installe/désinstalle proprement.
3. Une copie de coffre réel s'ouvre, s'édite, se sauvegarde sans altération hors des fichiers modifiés ; un seul process par coffre.
4. Plugins/thèmes : CSP et schémas custom fonctionnels ; liste documentée des plugins non compatibles.
5. Limitations connues documentées (sandbox, secrets, TOCTOU vaultfs si applicable).
6. CI Windows reproductible avec artefact et `SHA256SUMS`.

## 8. Pièges connus
- Pas de cross-compilation Linux→Windows fiable avec Tauri : compiler sous Windows/`windows-latest`. Cible **MSVC** uniquement.
- `.msi` seulement sous Windows ; NSIS recommandé.
- `docs/ scripts/ notes/ bin/` sont des symlinks vers un dépôt privé : peuvent casser le checkout Windows (symlinks nécessitent le mode développeur ou `core.symlinks`) ; ne pas s'en servir.
- OneDrive et Defender ralentissent/verrouillent fichiers : prévoir des retries bornés et des exclusions de dev.
- Clones sous Windows avec `autocrlf` : peuvent corrompre les hash de thèmes et fixtures de tests.

## 9. Prompt de démarrage pour Claude Code
```text
Lis CLAUDE.md puis AGENTS.md. Exécute la Phase 0 et la Phase 1 uniquement :
crée la branche port/windows, mesure la baseline de compilation Windows,
vérifie chaque hypothèse du §5 dans le code réel, produis windows-port/AUDIT.md
(fichier:ligne, risque, correctif, crates à ajouter) sans modifier le code source.
Arrête-toi ensuite et présente un plan priorisé avec les décisions à valider.
```
