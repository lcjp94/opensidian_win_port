# Audit de portage — Windows 11 puis macOS

Base : `jpzk/opensidian` v0.9.2, commit `5ff9d7b`, branche `port/windows`.
Date : 2026-11-13. Phases 0 et 1 de `CLAUDE.md` : **aucun fichier source modifié**.

Légende : **Adaptable** = équivalent fonctionnel possible ; **Dégradé** = fonctionne avec une garantie plus faible ; **Perdu** = fonctionnalité absente sur la plateforme.

## 1. Baseline de compilation Windows (mesurée)

Commande : `cargo +1.99.1 check --locked --target x86_64-pc-windows-msvc`, avec `CARGO_TARGET_DIR=C:\cargo-target\opensidian` (hors OneDrive).

> Écart : la toolchain épinglée **1.98.0 n'est pas installée** sur ce poste (présentes : `stable`, `nightly`, `1.99.1`). La baseline a été prise en 1.99.1. Le dépôt n'a pas de `rust-toolchain.toml` (seul `fuzz/` en a un).

La compilation s'arrête sur trois murs successifs. Les murs 2 et 3 ont été mesurés sur une **copie hors dépôt** (`git archive` dans le dossier temporaire) où `landlock` était déplacé sous la cible Linux et une `icon.ico` factice ajoutée.

| Mur | Erreur | Cause |
|---|---|---|
| 1 | `could not compile landlock` (10 erreurs, `PR_GET_NO_NEW_PRIVS`…) | `landlock = "=0.4.7"` est une dépendance inconditionnelle (`Cargo.toml:43`) |
| 2 | `icons/icon.ico not found` (tauri-build) | `src-tauri/icons/` ne contient que `icon.png` |
| 3 | 92 erreurs dans `src/vaultfs.rs` (lib) | `std::os::unix`, `std::os::fd`, `libc::openat/renameat/mkdirat/unlinkat/fstatat/linkat/renameat2/fdopendir`, `O_NOFOLLOW`, `O_PATH`, `mode_t` |

Le binaire (`main.rs` et les modules hors lib) n'est pas atteint tant que la lib ne compile pas ; ses points bloquants sont recensés par lecture au §2 (non mesurés par le compilateur).

Hypothèses de `CLAUDE.md` §4 :
- `landlock`, `libc`, `std::os::unix` : **confirmées**.
- `gdk/gdkx11/soup3/dbus` utilisés sans `cfg` : **infirmée**. L'amont les a déjà isolés sous `cfg(target_os = "linux")` avec un repli non-Linux (`appid.rs:86`, `plugin_store.rs:208`, `secrets.rs:310`, `main.rs:5483`, `main.rs:5737`).
- `[patch.crates-io] glib` : aucun effet bloquant observé sous Windows.
- `.gitattributes` protège déjà les thèmes (`src-tauri/themes/** -text`). Le clone local a `core.autocrlf=true` : les fixtures de tests (`tests/fixtures/**`, `listtoggle.tsv`) ne sont **pas** protégées.

## 2. Inventaire des spécificités Linux

### 2.1 Bloquant la compilation (code hors tests)

| # | Fichier:ligne | Spécificité | Windows | macOS |
|---|---|---|---|---|
| A | `Cargo.toml:43`, `sandbox.rs:24` (718 l.), appels `main.rs:1818, 1881, 2021, 6157` | Sandbox Landlock | **Perdu** (v1). Module réduit à un stub « indisponible » | **Perdu**. Landlock n'existe pas ; App Sandbox/Seatbelt est un autre modèle, hors périmètre |
| B | `vaultfs.rs:27-32, 117-920` | E/S sans suivi de lien (`openat` + `O_NOFOLLOW` à chaque composant, `renameat`, `linkat`, `renameat2(RENAME_NOREPLACE)`) | **Adaptable, garantie différente** — voir §3.1 | **Adaptable** : tout est POSIX sauf `renameat2` → `renameatx_np(RENAME_EXCL)` |
| C | `vaultfs.rs:910-919` | `p_chmod` plugins via `O_PATH` + `/proc/self/fd` | **Perdu** (pas de modes POSIX ; seul l'attribut lecture seule existe) → renvoyer « non supporté » | **Adaptable** : `open(O_NOFOLLOW)` + `fchmod` sur le descripteur |
| D | `vaultfs.rs:210, 373, 577` | `umask`, `fchmod` (conservation du mode) | Sans objet (ACL héritées) | Identique à Linux |
| E | `main.rs:431-457` | Lecture média par marche `openat` | Comme B | Identique |
| F | `main.rs:5869-5872, 5938-5975` | Arrêt propre sur `SIGTERM` (`sigaction`, `pipe2`, self-pipe) | **Adaptable partiel** : la fermeture de fenêtre (`WM_CLOSE`, `taskkill` sans `/F`) passe déjà par `quit_request`. Pas d'équivalent pour un kill forcé | **Adaptable** : `pipe2` absent → `pipe` + `fcntl(FD_CLOEXEC)` |
| G | `main.rs:2422, 3901` | `xdg-open` (liens externes, ouverture de fichiers par les plugins) | **Adaptable** : `ShellExecuteW` (ou `rundll32 url.dll,FileProtocolHandler`), sans shell | **Adaptable** : `/usr/bin/open` |
| H | `main.rs:1614, 2307, 2407, 6134`, `vaultlock.rs:109`, `migrate.rs` | `$HOME` avec repli `/` | **Adaptable** : `%USERPROFILE%` | Identique |
| I | `main.rs:2293-2311` (`picker_allows`), test `main.rs:9787` | Liste noire `/proc /sys /etc…`, racine `/`, `/workspace` | **Adaptable** : liste des lecteurs comme racine ; refuser `%SystemRoot%`, `Program Files`, `ProgramData` ; chemins UNC à décider | **Adaptable** : ajouter `/System`, `/Library`, `/private` ; `/Volumes` ouvert |
| J | `spawner.rs:36, 113, 191, 210` | Assistant non confiné sur `UnixStream::pair()` hérité en fd 0 | **Sans objet** : il n'existe que pour contourner Landlock. Sans sandbox, appels directs (le trait `Handler` le permet) | Idem (`UnixStream` compilerait, mais inutile) |
| K | `spawn.rs:162` | `alive(pid)` lit `/proc/<pid>/stat` | **Adaptable** : `OpenProcess` + `GetExitCodeProcess` | **Adaptable** : `kill(pid, 0)` (pas d'état zombie lisible aussi finement) |
| L | `spawn.rs:194-203, 217-221`, `vaultlock.rs:99-111` | Lanceurs Flatpak/AppImage, `process_group(0)`, `/.flatpak-info`, `XDG_RUNTIME_DIR` | **Adaptable** : seul `Launcher::Plain(current_exe)` ; `CREATE_NEW_PROCESS_GROUP` ; dossier de verrous `%LOCALAPPDATA%\dev.koto.opensidian\locks` | **Adaptable** : `Plain` ; verrous sous `~/Library/Application Support/dev.koto.opensidian/locks` |
| M | `vaultlock.rs:44, 127`, `cfgstore.rs:37` | `OsStrExt::as_bytes` (clé FNV du chemin) | **Adaptable** : hacher le chemin canonique en minuscules, sans préfixe `\\?\` (`dunce 1.0.5` est déjà dans `Cargo.lock`) | Identique à Linux ; attention casse et NFD (§4) |
| N | `desk_proc.rs:29, 136-178, 239-245` | Processus des plugins : signaux nommés, `kill(-pgid)`, `prctl(PR_SET_PDEATHSIG)`, `close_range` | **Dégradé** : Job Object (`KILL_ON_JOB_CLOSE`) remplace groupe + PDEATHSIG ; tous les signaux deviennent « terminer » ; pas de code « tué par signal » | **Dégradé** : signaux OK ; pas de `PDEATHSIG` ni `close_range` (boucle `FD_CLOEXEC`, surveillance du parent à écrire) |
| O | `migrate.rs:103, 144-148` | `symlink()`, `O_NOFOLLOW \| O_NONBLOCK` | **Adaptable** : migration depuis l'ancien nom du produit, probablement sans objet sous Windows (aucune installation antérieure) → ne rien migrer | Identique à Linux |
| P | `canvasfs.rs:122`, `pasteimg.rs:179` | `libc::ENOENT`, `libc::ELOOP` | **Adaptable** : `io::ErrorKind` + refus explicite des points d'analyse | Identique |
| Q | `icons/` | Pas d'`icon.ico` | **À créer** à partir de `packaging/linux/opensidian.svg` (asset du projet, pas d'Obsidian) | `icon.icns` à créer pour le bundle |

### 2.2 Déjà isolé par l'amont, fonctionnalité absente hors Linux

| # | Fichier:ligne | Fonction | Windows | macOS |
|---|---|---|---|---|
| R | `plugin_store.rs:142-212`, utilisé par `updates.rs:453` | Client HTTPS (libsoup) : magasin de plugins **et** vérification des mises à jour | **Adaptable** : WinHTTP — voir §3.3. Sans cela : pas de magasin, pas de détection de mise à jour | **Adaptable** : `NSURLSession` via `objc2-foundation 0.3.2` (déjà verrouillé ; features non vérifiées) |
| S | `secrets.rs:101-314` | `app.secretStorage` des plugins (Secret Service / D-Bus) | **Adaptable** : Gestionnaire d'identification Windows — voir §3.2. Limite : 2560 octets par secret | **Adaptable** : Trousseau (`Security.framework` ; `core-foundation 0.10.1` déjà verrouillé) |
| T | `appid.rs:36-89` | Nom/classe de programme GTK | Sans objet. AppUserModelID à poser pour regrouper les fenêtres dans la barre des tâches | Sans objet (identité = bundle) |
| U | `main.rs:5464-5483, 5712-5740` | Étendue du cadre GDK, choix X11/Wayland du glisser (`win_move_proto`) | **Adaptable** : `start_dragging` / `start_resize_dragging` (déjà le chemin générique). Reste à faire : Snap Layouts, bords de redimensionnement, ombre | **Adaptable** : préférer `titleBarStyle: "Overlay"` (feux tricolores natifs) à `decorations: false` |

### 2.3 Comportements à adapter (compilent, mais faux hors Linux)

| # | Fichier:ligne | Problème | Windows | macOS |
|---|---|---|---|---|
| V | `main.rs:576`, `plugins.rs:12, 115`, `tauri.conf.json` (CSP), `main.rs:6090` (`nav_allowed`), tests `main.rs:9730-9740` | URL `opensidian-img://localhost/…`, `opensidian-plugin://localhost/…`, origine `tauri://localhost` | **À adapter, priorité 1** : WebView2 sert `http://<schéma>.localhost/` et l'app sur `http://tauri.localhost`. Ajouter `tauri.windows.conf.json` (fusionné automatiquement par Tauri v2, aucun impact Linux) pour la CSP, et dériver préfixe d'URL + `nav_allowed` par `cfg` | Aucun changement : WKWebView garde la forme `schéma://localhost/` |
| W | `index.rs`, `watcher.rs`, `vaultfs.rs:236` (`parts`) | Chemins relatifs du coffre | Normaliser `\` → `/` dans l'index/les liens ; noms réservés (`CON`, `NUL`, `COM1`…), `<>:"\|?*`, point/espace final, MAX_PATH | Noms de fichiers en NFD : les wikilinks accentués doivent être comparés après normalisation |
| X | `index.rs` | Système de fichiers insensible à la casse | `Note.md` et `note.md` ne peuvent coexister ; un lien `[[note]]` résout physiquement `Note.md` | Idem (APFS par défaut) |
| Y | `cfgstore.rs` | Écriture atomique (temp + rename) partagée entre process | Retries bornés sur `ERROR_SHARING_VIOLATION` / `ACCESS_DENIED` (antivirus, OneDrive) | Identique à Linux |
| Z | `ui/main.js` (~15 sites `ctrlKey \|\| metaKey`, raccourcis) | Touche modificatrice | Rien à faire | Auditer : `Cmd` au lieu de `Ctrl`, libellés des raccourcis |
| AA | `pasteimg.rs:14` | Collage PNG documenté pour le presse-papiers X | À vérifier manuellement sous WebView2 (non vérifié) | À vérifier sous WKWebView (non vérifié) |
| AB | `updates.rs` | Choix de l'actif de release | Aucun actif Windows en amont : la vérification signale une version mais ne peut pointer que vers la page de release | Idem |

### 2.4 Tests

Environ 60 tests créent des liens symboliques (`std::os::unix::fs::symlink`) ou des FIFO (`mkfifo`) : `main.rs:6715-11297`, `vaultfs.rs:1111-1504`, `canvasfs.rs:414-639`, `plugins.rs:339-463` (déjà `cfg(unix)`), `migrate.rs:260-281`, `watcher.rs:362, 560, 580`, `pasteimg.rs:279-294`, `cfgstore.rs:200-268`, `plugin_store.rs:763`, `desk_proc.rs:398-452`, `devui.rs:268`.

- Windows : les passer sous `cfg(unix)` et écrire des équivalents avec des **jonctions** (créables sans droits admin ni mode développeur) ; FIFO sans objet. `desk_proc` : `/bin/sh`, `/bin/sleep` → `cmd /c`, `powershell Start-Sleep`. `spawn.rs:360` (`/bin/true`, `/proc`) → `cfg(target_os = "linux")`.
- `tests/listtoggle.rs` est déjà `cfg(target_os = "linux")` (JavaScriptCore GTK). **Couverture perdue** sous Windows et macOS : la logique de bascule de listes de `ui/editor.js` n'est testée que sur Linux.
- `perf.rs:1084-1160` (test du puits OTel sous Landlock) → `cfg(target_os = "linux")`.
- `fuzz/` : libFuzzer, à laisser Linux uniquement ; la lib doit seulement rester compilable.

## 3. Décisions à valider

### 3.1 `vaultfs` sous Windows (modèle de sécurité)

Linux garantit qu'aucune écriture ne traverse un lien symbolique, même si un composant est remplacé pendant l'opération. Windows n'a pas d'`openat` dans `std`.

- **Option 1 (recommandée) — « ouvrir puis vérifier le handle »** : ouvrir chaque fichier/dossier avec `FILE_FLAG_OPEN_REPARSE_POINT`, puis contrôler *sur le handle* qu'il n'est pas un point d'analyse et que `GetFinalPathNameByHandleW` reste sous la racine canonique du coffre ; renommer par handle (`SetFileInformationByHandle`). La course résiduelle se limite au renommage d'un dossier parent pendant l'opération. Coût : API Win32 via `windows-sys`/`windows` (déjà dans `Cargo.lock`), ~600 lignes.
- **Option 2 — vérification préalable simple** : `symlink_metadata` sur chaque composant puis `std::fs`. ~250 lignes, mais fenêtre TOCTOU complète entre contrôle et action.

Dans les deux cas : liens symboliques, jonctions et fichiers OneDrive « à la demande » (ce sont aussi des points d'analyse) doivent être traités explicitement — **refuser les jonctions/symlinks, accepter les espaces réservés cloud**.

### 3.2 Secrets des plugins (Windows)

Gestionnaire d'identification (`CredWriteW/CredReadW/CredDeleteW/CredEnumerateW`). Aucun nouveau paquet dans `Cargo.lock` : `windows-sys 0.61.2` et `windows 0.61.3` y sont déjà (dépendances de tauri/wry). Il faut néanmoins les déclarer en dépendance directe épinglée sous `[target.'cfg(windows)'.dependencies]` et l'inscrire dans `THIRD-PARTY.md`. Le choix exact (`windows-sys` ou `windows`) et les features sont à vérifier en Phase 2 (non vérifié).
Alternative : garder le stub « non supporté » en v1 (les plugins qui stockent des jetons ne fonctionnent pas).

### 3.3 Client HTTPS (Windows)

- **Option 1 (recommandée)** : WinHTTP via la même crate Windows que §3.2 — TLS et magasin de certificats du système, redirections désactivées et suivies à la main comme sous libsoup, mêmes garde-fous (`check_url`, liste d'hôtes, plafond de taille, sha256).
- **Option 2** : `reqwest 0.13.4` est présent dans `Cargo.lock`, mais sa pile TLS et ses features activées n'ont pas été vérifiées, et l'activer côté application tirerait probablement de nouveaux paquets.
- **Option 3** : stub en v1 — pas de magasin de plugins (installation manuelle dans `.obsidian/plugins/`), pas de vérification de mise à jour.

### 3.4 Emplacement de la configuration (Windows)

`%USERPROFILE%\.opensidian.json` (recommandé : même nom que Linux, code inchangé hormis `HOME` → `USERPROFILE`) ou `%APPDATA%\dev.koto.opensidian\config.json` (plus conforme à Windows, mais divergence avec l'amont).

### 3.5 Environnement

- Installer Rust **1.98.0** (`rustup toolchain install 1.98.0 --profile minimal -c clippy,rustfmt`) ? Ajouter un `rust-toolchain.toml` est un changement visible par l'amont.
- Le dépôt est sous OneDrive : continuer avec `CARGO_TARGET_DIR=C:\cargo-target\opensidian`, ou cloner hors OneDrive.
- Non-régression Linux : aucun WSL/CI Linux vérifié à ce stade. Il faut un job `ubuntu` (Phase 5 avancée en Phase 2) ou une distribution WSL avec les paquets WebKitGTK.

### 3.6 Packaging Windows

NSIS via `tauri-cli` (recommandé, `downloadBootstrapper` pour WebView2) ou zip portable.

## 4. Fonctionnalités perdues ou dégradées

| Fonctionnalité | Windows v1 | macOS |
|---|---|---|
| Sandbox Landlock (`OPENSIDIAN_LANDLOCK=1`) : confinement du process au coffre | **Perdue** | **Perdue** |
| Garantie « jamais à travers un lien » contre un attaquant concurrent | **Dégradée** (§3.1) | Conservée |
| `chmod` par les plugins, conservation des modes de fichier | **Perdue** | Conservée |
| Signaux nommés vers les processus de plugins | **Dégradée** (terminer uniquement) | Conservée |
| Mort automatique des processus de plugins si l'app meurt | Conservée (Job Object) | **Dégradée** (à réécrire) |
| Arrêt propre sur kill externe | **Dégradée** (fermeture normale uniquement) | Conservée |
| Secrets des plugins | Conservée si §3.2 accepté, sinon perdue | Conservée (Trousseau) |
| Magasin de plugins et vérification de mise à jour | Conservée si §3.3 accepté, sinon perdue | Conservée (à écrire) |
| Mise à jour : lien vers un binaire de la plateforme | **Perdue** tant que l'amont ne publie pas d'actif | Idem |
| Tests listtoggle (JavaScriptCore) | **Couverture perdue** | **Couverture perdue** |
| Flatpak / AppImage / deb / rpm / Nix | Sans objet | Sans objet |
| Coexistence de `Note.md` et `note.md` | **Impossible** (NTFS) | **Impossible** (APFS par défaut) |
| Compatibilité des plugins communautaires | Non vérifiée | Non vérifiée |

## 5. Plan

Principe d'isolation, appliqué dès Windows pour ne pas refaire le travail pour macOS : trois niveaux de `cfg` — `cfg(unix)` (commun Linux + macOS : `openat`, signaux, liens), `cfg(target_os = "linux")` (Landlock, GDK, libsoup, D-Bus, `/proc`, Flatpak, `renameat2`, `O_PATH`, `prctl`, `close_range`, `pipe2`), `cfg(windows)`. Les points d'entrée dépendants de l'OS sont regroupés derrière une petite façade (dossier personnel, ouverture externe, processus vivant, dossier de verrous, HTTP, secrets) pour garder `main.rs` lisible.

### Étape A — Windows 11

| Phase | Contenu | Effort estimé |
|---|---|---|
| 2. Compilation | `landlock` sous cible Linux ; stub `sandbox` ; `icon.ico` ; `vaultfs` Windows ; `cfg` des points A–Q ; tests Unix sous `cfg(unix)` ; `-text` sur les fixtures | 4–6 j (dont `vaultfs` 2–3 j) |
| 3. Cœur | config (H, Y) → verrou (M, L) → lancement (K, L, J) → `vaultfs` durci + tests jonctions → chemins/casse (W, X) → thèmes | 4–5 j |
| 4. Interface | CSP et schémas (V) **en premier** ; fenêtre (U) ; ouverture externe (G) ; processus de plugins (N) ; sélecteur de dossier (I) ; secrets (S) ; HTTPS (R) ; presse-papiers (AA) | 5–7 j |
| 5. Packaging + CI | `packaging/windows/`, NSIS, workflow `windows-latest` + job `ubuntu` de non-régression, `SHA256SUMS` | 2 j |
| 6. Documentation | `windows-port/README.md`, section README, PR atomiques vers l'amont | 1 j |

### Étape B — macOS (après la v1 Windows)

Nécessite un Mac ou un runner `macos-latest` : pas de compilation croisée fiable.

1. Compilation : remplacer les cinq appels Linux-only de `vaultfs`/`desk_proc`/`main.rs` (`renameat2`, `O_PATH` + `/proc/self/fd`, `close_range`, `prctl`, `pipe2`) ; `alive()` ; dossier de verrous ; `open`. Le reste du code `cfg(unix)` est réutilisé tel quel. ~2–3 j.
2. Plateforme : Trousseau, `NSURLSession`, `titleBarStyle: Overlay`, raccourcis `Cmd`, normalisation NFC/NFD, liste noire du sélecteur. ~4–5 j.
3. Modèle « un process par coffre » : chaque process aura sa propre icône dans le Dock ; à évaluer (non vérifié).
4. Packaging : `.app`/`.dmg` (`icon.icns`), binaire universel `aarch64` + `x86_64`, signature Developer ID et notarisation (compte développeur Apple payant ; sans cela Gatekeeper bloque l'ouverture par défaut).

## 6. Risques, par ordre décroissant

1. `vaultfs` Windows : touche au modèle de sécurité, 1500 lignes et ~40 tests à doubler.
2. CSP et schémas personnalisés sous WebView2 : si mal réglés, ni images ni plugins.
3. Absence de vérification Linux locale : chaque phase peut casser l'amont sans qu'on le voie.
4. `main.rs` fait 11 479 lignes : les `cfg` y seront nombreux, conflits de fusion probables avec l'amont.
5. OneDrive/Defender : verrous sur le coffre et sur `.opensidian.json`.

## 7. État après la Phase 2 (2026-11-13)

Mesuré sous Windows 11, Rust 1.98.0 MSVC, `CARGO_TARGET_DIR=C:\cargo-target\opensidian` :

| Commande (dans `src-tauri/`) | Résultat |
|---|---|
| `cargo check --locked` | OK |
| `cargo build --release --locked` | OK, `opensidian.exe` 23,7 Mo |
| `cargo test --locked` | 387 réussis, 0 échec, 1 ignoré |
| Lancement du binaire debug sur une copie de `vault/` (`VAULT_DIR`) | démarre, prend le verrou, sème les thèmes, reste en vie 20 s |

**Non vérifié : Linux.** Aucun `cargo check` Linux n'a été lancé (pas de WSL avec WebKitGTK, pas de CI). Tout le code Linux est resté en place derrière des `cfg`, mais seule une compilation le prouvera.

Aucune dépendance ajoutée, `Cargo.lock` inchangé (`landlock` est seulement passé sous la cible Linux), donc rien à inscrire dans `THIRD-PARTY.md`.

### Ce qui a été fait

- `Cargo.toml` : `landlock` sous `[target.'cfg(target_os = "linux")'.dependencies]`.
- `icons/icon.ico` : provisoire, une seule taille (128 px) tirée de `icons/icon.png`. À refaire en multi-tailles en Phase 5.
- `vaultfs.rs` : code POSIX inchangé sous `cfg(unix)` ; `vaultfs_win.rs` (nouveau) donne la même API avec `std` seul — chaque dossier du chemin est ouvert **sans `FILE_SHARE_DELETE`** (il ne peut être ni renommé ni supprimé tant qu'on le tient) et **avec `FILE_FLAG_OPEN_REPARSE_POINT`** (une jonction est ouverte elle-même et refusée). C'est l'option 1 du §3.1, obtenue sans crate Windows. Limites écrites en tête du fichier.
- `plat.rs` (nouveau) : dossier personnel, ouverture par défaut (`ShellExecuteW`), processus vivant, arrêt d'un arbre de processus. Quatre fonctions Win32 déclarées à la main.
- `sandbox.rs`, `spawner.rs` : Landlock et son assistant ne se compilent que sous Linux ; ailleurs `landlock_enabled()` est toujours faux.
- `vaultlock.rs` : verrous sous `%LOCALAPPDATA%\dev.koto.opensidian\locks`. Le marqueur `ready <token>` va dans un fichier voisin `.ready` : un verrou Windows est obligatoire, le fichier verrouillé est illisible pour les autres processus.
- `cfgstore.rs`, `spawn.rs`, `migrate.rs`, `desk_proc.rs`, `main.rs` : variantes `cfg` (points F, G, H, I, K, L, M, N, O du §2).
- `vaultfs::rel_str` : les chemins relatifs du coffre sont toujours écrits avec `/` (clés d'index, liens, favoris, corbeille).
- `.gitattributes` : sources en LF sur tous les OS, fixtures et données épinglées jamais converties. Sans cela, sous `autocrlf`, la signature de `data/updates/SHA256SUMS` ne se vérifiait plus.
- Tests : une soixantaine de tests propres à Unix (liens symboliques, FIFO, modes, `sh`, `/proc`) passés sous `cfg(unix)`, ainsi que les modules de tests entiers de `vaultfs.rs` et `sandbox.rs` ; 13 tests Windows ajoutés (`vaultfs_win`, `plat`).

### Écarts connus à l'issue de la Phase 2

| Sujet | État |
|---|---|
| Renommage ne changeant que la casse (`Note` → `note`) | refusé « target exists » ; test `r9_8_case_only_rename_is_followed` ignoré sous Windows (point X) |
| Glisser la fenêtre | le journal affiche `[win_drag] ... path none` : à traiter en Phase 4 (point U) |
| Images et plugins | schémas `opensidian-img:` / `opensidian-plugin:` non adaptés à WebView2 (point V) : non testé, très probablement cassé |
| Sélecteur de coffre | `picker_allows` accepte `X:\...`, mais la liste des lecteurs et les séparateurs côté `ui/` ne sont pas faits (point I) |
| Magasin de plugins, mises à jour, secrets | toujours « non supporté » (points R, S) ; d'où ~20 avertissements `dead_code` sous Windows |
| Processus de plugins | tués par `taskkill /T /F` ; pas de Job Object, un enfant peut survivre à l'app (point N) |
| Noms réservés (`CON`, `NUL`…), point ou espace final | non filtrés (point W) |
| Test de lien symbolique de fichier | sauté sur ce poste (mode développeur désactivé) ; les jonctions sont testées |
| Antivirus | McAfee verrouille par moments les `.exe` fraîchement liés (`LNK1104`) : relancer, ou exclure `C:\cargo-target` |

## 8. Début de la Phase 4 (2026-11-13)

### Fait

- **CI** : `.github/workflows/build.yml` compile et teste sur `ubuntu-24.04` et `windows-latest`, et publie l'exe non signé avec `SHA256SUMS`.
- **Schémas d'URL et CSP (point V)** : sous WebView2 la page est servie sur `http://tauri.localhost` et un schéma enregistré sur `http://<schéma>.localhost/`. Le préfixe est choisi côté Rust (`IMG_BASE`) et côté `ui/` (d'après l'hôte de la page) ; la CSP Windows est dans `tauri.windows.conf.json`, fusionnée par Tauri sous Windows seulement ; `nav_allowed` accepte `http://tauri.localhost` exactement. Un test lie les deux CSP : celle de Windows doit être celle de base avec les deux schémas réécrits, rien d'autre.
- **Glisser (point U)** : sous Windows l'appui sur la barre de titre est toujours confié au système (`start_dragging`), ce qui donne Aero Snap et Snap Layouts. Correction de l'audit initial : le glisser n'était pas cassé, il fonctionnait par repositionnement, sans Snap.

### Vérifié dans l'application réelle

Binaire debug lancé sur une copie du coffre, WebView2 piloté par son port de débogage (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=…`), expressions évaluées dans la page :

| Requête depuis la page | Résultat |
|---|---|
| origine de la page | `http://tauri.localhost` |
| `http://opensidian-img.localhost/pic.png` | chargée (128 px) |
| `http://opensidian-img.localhost/sub/my%20pic.png` | chargée |
| image absente du coffre | erreur |
| ancienne forme `opensidian-img://localhost/pic.png` | bloquée |
| image distante `https://example.com/x.png` | bloquée par la CSP |
| script `http://opensidian-plugin.localhost/nope/main.js` | atteint le gestionnaire Rust, refusé « restricted mode » |

L'arborescence du coffre et la note ouverte s'affichent.

### Non vérifié

- Le glisser, le Snap et le redimensionnement par les bords : demandent une souris, pas testés.
- Un vrai plugin communautaire dans un coffre de confiance (CSP `unsafe-eval`, `blob:`).
- Les pages PDF et les cartes média du canvas (même schéma que les images, pas exercés).

## 9. Suite de la Phase 4 (2026-11-13)

CI : les deux jobs (Linux, Windows) sont verts depuis `f2f54c4`. Le seul échec Linux initial, `listtoggle_table`, venait de l'amont (variable `CENSUS_ON` non définie dans le banc JavaScriptCore) et est corrigé dans un commit séparé.

Vérifié à la souris par l'utilisateur : glisser de la barre de titre, Snap, agrandissement par double-clic, redimensionnement par les bords.

### Fait

| Point | Changement | Vérification |
|---|---|---|
| X | Renommage ne changeant que la casse | test `r9_8` réactivé, test dédié |
| W | Noms réservés (`CON`, `NUL`, `COM1`…), `< > " \| ? *`, point ou espace final refusés | test dédié |
| I | Sélecteur de coffre : `..` remonte jusqu'à la liste des lecteurs, dossiers cachés/système non proposés | piloté dans l'app : `C:\Users\jp` → `C:\Users` → `C:\` → lecteurs → `C:\` |
| L | Coffre passé en argument (donc « ouvrir dans une nouvelle fenêtre » et « changer de coffre ») : le chemin canonique n'a plus le préfixe `\?\` que le sélecteur refusait | lancé avec `…\Mon coffre été` : ouvert, enregistré tel quel dans la config |
| R | Client HTTPS sur WinHTTP (`http_win.rs`) : mêmes règles que libsoup — `check_url` avant chaque requête, redirections suivies à la main et revérifiées, ni cookies ni authentification, délais, plafond de taille | test sur serveur local ; dans l'app : mise à jour 0.9.3 détectée et signature validée, magasin = 10 349 plugins, fiche détaillée |
| S | Secrets des plugins dans le Gestionnaire d'identification (`secrets_win.rs`) | test aller-retour sur le vrai magasin, nettoyé ; coffre non approuvé refusé dans l'app |

Dépendance : `windows-sys =0.61.2` déclarée sous `cfg(windows)` (déjà dans `Cargo.lock`, qui gagne une ligne). Entrée dans `THIRD-PARTY.md`.

### Limites nouvelles

- Un secret de plugin est limité à 2560 octets sous Windows (limite du Gestionnaire d'identification) ; au-delà, refus explicite.
- « Ouvrir dans une nouvelle fenêtre » et « changer de coffre » : la cause du blocage est corrigée et l'ouverture par argument est vérifiée, mais l'enchaînement complet entre deux fenêtres n'a pas été exercé.

### Reste en Phase 4

- Processus de plugins : Job Object, lanceurs `.cmd`/`.bat`, `plugin_unroot` pour les chemins Windows (point N).
- Un vrai plugin communautaire dans un coffre approuvé ; pages PDF ; cartes média du canvas.
- Collage d'image (point AA).
- Dispositions Snap au survol du bouton « agrandir ».

## 10. Plugins de bout en bout (2026-11-13)

### Fait (point N)

- **Durée de vie** : chaque processus lancé par un plugin rejoint un Job Object unique de l'application (`KILL_ON_JOB_CLOSE`) ; le système le termine, lui et sa descendance, quand l'application disparaît, quelle qu'en soit la cause.
- **`exec()` / `shell: true`** : la page se présente aux plugins comme Linux, la demande arrive donc en `/bin/sh -c <ligne>`. Elle est exécutée comme Node le fait sous Windows : `cmd.exe /d /s /c "<ligne>"`, ligne transmise telle quelle.
- **Chemins réels du coffre** (`plugin_unroot`) : `C:/Users/x/coffre/...` (forme imprimée par git) comme `C:\Users\x\coffre\...` sont ramenés au chemin relatif, sans tenir compte de la casse. Hors du coffre : toujours refusé.

### Vérifié dans l'application réelle

Un plugin de sonde écrit pour l'occasion (pas un plugin tiers), dans un coffre jetable approuvé :

| Étape | Résultat |
|---|---|
| Chargement par `http://opensidian-plugin.localhost/` | chargé, `onload` exécuté |
| CSP d'un coffre approuvé (`new Function`) | autorisé |
| `adapter.write` / `read` / `list` | fichier écrit dans le coffre, relu, listé |
| `child_process.exec("echo …")` | sortie reçue (via `cmd.exe`) |
| `child_process.spawn("cmd.exe", …, { cwd: "/vault" })` | code 0, répertoire courant = dossier réel du coffre |
| `fs.existsSync` / `readFileSync` sur `/vault/…` | corrects |
| `app.secretStorage.setSecret` / `getSecret` | valeur relue, entrée supprimée ensuite |

`~/.opensidian.json` de l'utilisateur a été sauvegardé avant l'essai et restauré à l'identique après.

### Non fait, et pourquoi

- **Un plugin communautaire réel** (Dataview, Excalidraw, obsidian-git) : l'installer revient à télécharger et exécuter du code tiers ; c'est à l'utilisateur de le faire. Le magasin liste et décrit les plugins ; l'installation elle-même n'a donc pas été exercée.
- **Persona Linux** : les plugins voient `process.platform === "linux"` et des chemins POSIX. Un plugin qui lance un programme Linux (`which`, `xdg-open`, un script `sh`) échouera sous Windows. Changer la persona est une décision de conception, pas un correctif.
- Lanceurs `.cmd` / `.bat` sans extension (`npm`, `code`) : non résolus par `spawn` sans shell, comme sous Node.

## 11. Phase 5 — installateur (2026-11-14)

- **Icône** : `icons/icon.ico` porte sept tailles (16 à 256 px), construite par `packaging/windows/make-icon.ps1` à partir des PNG du projet. Contrôlée à l'œil à chaque taille, sur fond clair et sombre.
- **Installateur NSIS** : `packaging/windows/tauri.bundle.conf.json`, passé à `cargo tauri bundle` par `--config`. `tauri.conf.json` garde `bundle.active = false` et `tauri.windows.conf.json` ne porte toujours que la CSP. Installation par utilisateur, WebView2 par le bootstrapper de Microsoft si absent. Outil : `tauri-cli =2.11.5`, la version de tauri du `Cargo.lock`.
- **CI** : le job Windows construit l'installateur après les tests et publie `opensidian_<version>_x64-setup.exe`, `opensidian.exe` et `SHA256SUMS`.

### Vérifié sur ce poste

| Étape | Résultat |
|---|---|
| Construction | `opensidian_0.9.2_x64-setup.exe`, 8,3 Mo |
| Installation silencieuse (`/S`) | code 0 ; `%LOCALAPPDATA%\opensidian\` (exe + désinstalleur), entrée « Applications », raccourcis menu Démarrer et Bureau |
| Lancement du binaire installé | origine `http://tauri.localhost`, coffre affiché |
| Désinstallation silencieuse | code 0 ; dossier, entrée de registre et raccourcis supprimés |
| Après désinstallation | `%USERPROFILE%\.opensidian.json` et `%LOCALAPPDATA%\dev.koto.opensidian` conservés |

### Limites

- **Non signé** : SmartScreen avertit. La signature Authenticode demande un certificat que seul le mainteneur peut fournir.
- `tauri bundle` marque l'exe de son type de bundle : le binaire installé diffère de quelques octets de celui que `cargo test` a exercé. Les sommes de contrôle sont prises après.
- Non testé : une machine sans WebView2, l'installateur en mode interactif, une mise à niveau par-dessus une version installée.

## 12. Retours du premier essai utilisateur (2026-11-14)

L'utilisateur a essayé l'exécutable installé : installateur interactif, deux fenêtres, collage d'image et un plugin communautaire fonctionnent. Deux remarques.

### Sélecteur de coffre « plus Windows » — fait

Le choix du dossier passe par le dialogue du système (`IFileOpenDialog`, `pick_win.rs`) : barre d'adresse, accès rapide, OneDrive, « Nouveau dossier ». La liste maison (`#p-dirs`) est masquée sous Windows, remplacée par un bouton « Browse… » ; le champ de chemin reste, pour taper ou coller. Le dialogue ne fait que proposer un chemin : l'ouverture et la création passent par les mêmes commandes qu'avant, donc par `vault_rules`.

- « Open » : le dialogue s'ouvre aussitôt ; au démarrage sans coffre, le dossier choisi s'ouvre directement.
- « Create » : le dialogue choisit le dossier parent, le nom se tape.

Vérifié dans l'app : une fenêtre de dialogue système (`#32770`) apparaît, rattachée à la fenêtre de l'application, et l'annulation renvoie « rien » à la page. **Non vérifié par automate : le choix effectif d'un dossier** (il faut une main sur le dialogue).

Dépendance : `windows =0.61.3` sous `cfg(windows)` (déjà dans `Cargo.lock`, une ligne de plus).

### Temps morts — NON reproduits, cause non trouvée

Mesures sur la version release, coffre synthétique de 2000 notes (8,7 Mo) sur disque local `C:\`, hors OneDrive :

| Opération | Durée |
|---|---|
| Démarrage (`boot`) | 242 ms |
| Construction de l'index, 2000 notes | 72 ms |
| Arborescence visible, depuis le début de la page | 1,1 s |
| Liste des notes / lecture d'une note / graphe | 3 ms / 1–3 ms / 5 ms |
| Joindre une image de 48 Ko | 5 ms |
| Lire un média de 30 Mo par l'IPC (8 tranches) | 269 ms |

Rien ici n'explique un temps mort perceptible. Hypothèses non testées, par ordre de vraisemblance :

1. **Coffre sous OneDrive** : fichiers « à la demande » à rapatrier, verrous de synchronisation.
2. **Antivirus** (McAfee sur ce poste) : analyse de chaque fichier ouvert ; l'ouverture d'un coffre en ouvre des milliers.
3. **Import d'un gros média** : sous Windows, `import_move` copie puis relit les deux fichiers pour les comparer (trois passes), là où Linux crée un lien physique instantané. Corrigeable maintenant que `windows-sys` est disponible (identité de fichier par `GetFileInformationByHandle`).
4. Taille ou forme du vrai coffre (bien plus de notes, très gros fichiers).

Pour trancher : lancer l'app avec `OPENSIDIAN_OTEL=<fichier>`, refaire les gestes lents, et lire les durées par opération dans ce fichier.
