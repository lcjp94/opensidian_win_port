# windows-port — suivi du travail Windows

Base : le portage de Morglaf ([jpzk/opensidian#34](https://github.com/jpzk/opensidian/pull/34)), branche `Morglaf/opensidian:platform-windows`. Flux de travail et règles : `CLAUDE.md` à la racine.

## Lead du portage

- 2026-11-27 : nous prenons le lead proposé par Morglaf (réponse publiée sur #34). jpzk : remettre à jour sur `develop`, la prochaine version ; il validera la PR Windows juste après cette sortie. Morglaf : accès en écriture accordé à lcjp94 sur Morglaf/opensidian, il propose de tester sous Windows.
- 2026-11-28 : Morglaf/opensidian#1 à #12 fusionnées dans `platform-windows` (avance rapide, sans réécriture). Puis `develop` de l'amont fusionné dedans (pas de rebase, choix validé), plus un commit d'adaptations Windows :
  - l'API `vaultfs` de #250 (`read`, `read_fd`, `rel_of`, `CFG_MAX`/`CSS_MAX`, `WalkEntry.rel/len`) ;
  - les tickets de dépôt #280 (aléa via `vaultfs::os_random` au lieu de `/dev/urandom`, sans quoi aucun dépôt n'était enregistré sous Windows) ;
  - la clé de cache PDF, et les nouvelles questions de la fenêtre de confirmation (micro, périphérique, pièce jointe, plugin) ;
  - l'audit des lectures de #250.

  Windows : 627 + 34 tests. Linux : 736 tests, CI verte.
- 2026-11-28 : point d'étape publié sur #34. La CI Windows (#5) a construit, testé (627 + 34) et empaqueté cette pointe ; l'installateur est l'artefact `opensidian-windows-x86_64-unsigned` du run 46120610624 de `lcjp94/opensidian_win_port`. Demandé à Morglaf : faire viser `develop` à #34, et tester avec cet installateur. En attente.

Non couvert sous Windows : les tests de #250 qui créent des liens vers `/dev/zero`, des tubes nommés ou jouent sur les droits Unix (`read250_tests` et quelques autres) sont réservés à Unix. Le renforcement #250 n'a pas encore de test Windows à lui.

## PR en cours

| PR | Sujet | État |
|---|---|---|
| [jpzk/opensidian#34](https://github.com/jpzk/opensidian/pull/34) | Le portage Windows (branche de Morglaf, que nous menons) | à jour sur `develop` ; vise encore `main` |
| [jpzk/opensidian#263](https://github.com/jpzk/opensidian/pull/263) | PDF 3D : avis + lien vers le plan 2D | en relecture chez l'amont |
| [jpzk/opensidian#359](https://github.com/jpzk/opensidian/pull/359) | canevas : « Back to previous view » après « Zoom to selection » | ouverte |
| [jpzk/opensidian#360](https://github.com/jpzk/opensidian/pull/360) | canevas : glisser sur le fond déplace la vue, Maj+glisser sélectionne | ouverte |
| [jpzk/opensidian#361](https://github.com/jpzk/opensidian/pull/361) | `workspace.json` n'est plus réécrit à l'ouverture d'un coffre (attente de 17 s mesurée sous Windows) | ouverte |

Quand une PR est fusionnée chez Morglaf ou l'amont, la retirer de ce tableau. Morglaf/opensidian#1 à #12 sont fusionnées (2026-11-28) ; #13 (icône) est fermée.

## Depuis l'ancien portage

Toutes les pièces du tag `port-windows-v0.9.2` sont reportées, sauf l'icône en 7 tailles (logo lavande), essayée puis abandonnée le 2026-11-25 : l'icône de Morglaf est préférée. L'audit commit par commit des 30 commits avait trouvé deux oublis, le cadre PDF qui scintille (#12) et cette icône.

Vérifié à la main : le visualiseur d'image (une image ouverte dans son onglet) s'affiche, bien que `main.js` y construise `opensidian-img://localhost/…` en dur, sans `__osCustomSchemeUrl`. La raison pour laquelle cela marche n'est pas vérifiée.

Reste ouvert, non traité :
- La confirmation d'un programme affiche `Program: \\?\C:\Windows\System32\cmd.exe` (forme verbatim). Texte de sécurité : à décider à part.
- Les clés internes (confiance, secrets, taille de fenêtre, id de coffre) restent en forme `\\?\` : cohérentes entre elles, donc laissées telles quelles (#11).

## Version installée sur le poste

Branche locale `local/windows-svg` (non poussée), worktree `C:\cargo-target\wt\local-svg`, encore basée sur l'ancien `windows` (v0.9.6) + #262 + #263 + #359 + #360 + #361. Elle ne contient pas encore la mise à jour sur `develop`. Installateur : Inno Setup 6.7.3 (par utilisateur), `ISCC` comme dans `windows.yml`.

## Limites connues (à documenter dans la PR finale)

- Pas de sandbox sous Windows (Landlock est propre à Linux).
- `fs.chmod` des plugins : sans effet sous Windows.
- Le lancement d'un programme approuvé est moins strict que sous Linux (pas d'exec par descripteur), voir l'en-tête de `desk_proc/windows.rs`.

## Outils

- `contrib-ci.yml` : workflow de contrôle Linux pour une branche de contribution. À copier sous `.github/workflows/` sur une branche jetable `ci/<sujet>` poussée sur `origin`, jamais dans une PR.
- Mise à jour d'une branche sur `develop` : compiler aussitôt sous Windows (`cargo test --no-run`), car `develop` ajoute du code Unix qui fusionne sans conflit textuel.
