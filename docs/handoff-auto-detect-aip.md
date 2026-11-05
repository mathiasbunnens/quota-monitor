# Handoff — détection automatique des profils `aip` (branche `feature/auto-detect-aip`)

**Direction courante** : Quota Monitor détecte tout seul les profils du gestionnaire `aip` (<https://github.com/dqn/aip>, v0.11.0, rév. `3961fa0`) pour Claude et Codex, les montre sous leur nom aip et ne lit jamais un profil dont la sécurité n'est pas prouvée. Base : `3258db7` (= 3.2.0). Rien n'est poussé, publié ni fusionné.

## Comportement livré
- Découverte à chaque démarrage/rafraîchissement via le socle commun (`account_sources::reconcile(all(..))`) ; aucun bouton d'import, mode Unique/Multi jamais changé par la découverte ; sans aip, résultat identique à 3.2.0 (tests `legacy_discover`).
- Claude : `_current` replié dans le Principal (une seule collecte) ; les autres profils = « Non collectable » (instantané du trousseau, lecture impossible sans `aip switch`).
- Codex : profils aip = « Non collectable » (grant OAuth indépendant invérifiable sans lire `auth.json`) ; une entrée enregistrée en 3.2.0 (ajout manuel sous `~/.codex/profiles` ou lien `~/.codex-*`) garde sa collecte 3.2.0.
- Doublons (liens, autres orthographes, chemin enregistré) = une entrée ; identité = chemin canonique ; renommage aip = nouvelle identité ; dossiers absents/corrompus ignorés sans erreur.
- Jamais : lecture de secrets, trousseau, `aip`/`switch`/`login`/`logout`, modification de l'environnement, des fichiers aip ou du compte actif.

## Fichiers utiles
- Socle : `src-tauri/src/aip.rs`, `src-tauri/src/account_sources.rs`.
- Claude : `src-tauri/src/claude_accounts.rs`, `src-tauri/src/claude.rs` (`collection_plan`, gardes `poll_profile`/`collect`).
- Codex : `src-tauri/src/codex_accounts.rs`, `src-tauri/src/codex.rs` (`collection_plan`, `account_view`).
- UI : `src-tauri/src/account_menu.rs`, `src-tauri/src/lib.rs` (`codex_panel`), `src/features/quota/{accountState,types}.ts`, `src/features/settings/ClaudeAccountsSettings.tsx`.
- Détails, sources et preuves : `docs/aip-profiles.md` (§1 format, §5 décision produit, §7 Claude, §8 Codex, §9 validation).

## Preuves
Synthétiques uniquement (dossiers temporaires, sentinelles, faux CLI). Aucune installation aip sur la machine de développement : **aucun test réel aip**. Tests : `cargo test --offline --locked --lib` 183/0/5, `npm run typecheck`, `npm run build`, `cargo fmt --check` (§9).

## Limites
- Quotas des profils aip non actifs (Claude) et de tous les profils aip Codex illisibles par conception.
- Alias aip du Principal Claude non affiché dans l'UI.
- Repli `_current` Claude faux si le trousseau a été changé hors aip (seul l'alias est faux, aucune collecte en plus).
- Validation native (app lancée, vraie installation aip) non faite.

## Suite éventuelle
1. Décision produit §5 pour Codex : garder bloqué / opt-in par profil après `aip codex login` / collecte automatique. L'opt-in se branche en mettant `Discovered::blocker = None` et `launch` = chemin brut.
2. Test manuel sur une vraie installation aip (build local, app installée quittée).
3. Avant une release : `cargo fmt` + `cargo test --locked` sur la branche de release (CI `check`).
