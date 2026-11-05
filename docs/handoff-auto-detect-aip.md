# Handoff — profils `aip` : détection, lecture des quotas et renouvellement des sessions (branche `feature/auto-detect-aip`)

**Direction courante (t_f8f5b8bc, autorisation de l'utilisateur : « vas-y, tu peux lancer ça pour que ça reste actif »)** : Quota Monitor détecte tout seul les profils du gestionnaire `aip` (<https://github.com/dqn/aip>, v0.11.0, rév. `3961fa0`, toujours tête de `main` le 2026-11-05) pour Claude et Codex, les montre sous leur nom aip, **lit leurs quotas** avec l'identifiant sauvegardé dans chaque profil et **renouvelle cet identifiant avant expiration** selon le protocole du fournisseur (Claude : protocole d'aip, même `credentials.json.lock` ; Codex : protocole du CLI, protection d'aip contre les grants importés reprise), en réécrivant **seulement les jetons** dans le fichier du profil. Jamais de bascule du compte actif, jamais de CLI pour un profil aip, jamais de trousseau. Base : `3258db7` (= 3.2.0). Rien n'est poussé, publié ni fusionné.

## Comportement livré
- Découverte à chaque démarrage/rafraîchissement via le socle commun (`account_sources::reconcile(all(..))`) ; aucun bouton d'import, mode Unique/Multi jamais changé par la découverte ; sans aip, résultat identique à 3.2.0 (tests `legacy_discover`).
- Profil aip complet = compte lisible (`Collect::Credential`) : affiché par défaut en Multi comme un dossier frère ; profil sans payload = « Non collectable » (`Blocked(Incomplete)`).
- Claude : `_current` replié dans le Principal (CLI, jamais lu ni renouvelé par ce chemin, comme aip) ; les autres instantanés sont lus via `GET api.anthropic.com/api/oauth/usage` avec leur `accessToken`, **renouvelés à 5 min de l'expiration** (`POST platform.claude.com/v1/oauth/token`, sous `flock` de `credentials.json.lock`, relecture sous verrou, écriture atomique 0600) et réécrits ; refus du fournisseur → « Session expirée » avec la marche à suivre, sans quota fictif.
- Codex : chaque home aip est lu via `GET chatgpt.com/backend-api/wham/usage` ; un home **qui possède son grant** (`aip codex login` fait : empreinte absente de `aip-imported-refresh.sha256`) est renouvelé à ≥ 10 min de l'expiration (`POST auth.openai.com/oauth/token`, JSON du CLI, relecture avant refresh, `tokens.*` + `last_refresh` réécrits) ; un home dont le grant est **encore importé** du compte principal (ou partagé avec un autre profil / le home principal) n'est jamais tourné : lisible tant que valide, puis « Session expirée … `aip codex login` ».
- Suivi autonome par le polling existant (plancher 120 s par profil, backoff, 429 respecté) : aucun processus externe, une requête de renouvellement par expiration.
- Jamais : secret affiché/journalisé, redirection HTTP, base autre que HTTPS figée, `_current`/`_order`/marqueurs modifiés, `aip switch`/`login`/`logout`, suppression de profil, rotation d'un grant partagé.

## Fichiers utiles
- Renouvellement : `src-tauri/src/oauth_refresh.rs` (protocoles, verrou, écriture atomique, état « rotation non sauvegardée », tests dont la simulation inter-processus).
- Transport : `src-tauri/src/oauth_usage.rs` (chargement, HTTP, projections, serveur de test local avec corps des requêtes).
- Socle : `src-tauri/src/aip.rs`, `src-tauri/src/account_sources.rs`.
- Claude : `src-tauri/src/claude_accounts.rs`, `src-tauri/src/claude.rs` (`poll_profile` → `oauth_refresh::read_claude_profile`, `credential_error`).
- Codex : `src-tauri/src/codex_accounts.rs`, `src-tauri/src/codex.rs` (`poll_credential` → `oauth_refresh::read_codex_home`, `ExtraWorker::primary_home`).
- UI : `src-tauri/src/account_menu.rs`, `src/features/quota/{accountState,types}.ts` (« Session expirée » ; `credential_unsaved` tombe dans « Erreur » avec son message).
- Détails, sources et preuves : `docs/aip-profiles.md` §11 (renouvellement), §10 (lecture), §1 (format) ; §3/§5/§7/§8 = historique.

## Preuves
Synthétiques uniquement (dossiers temporaires, identifiants fictifs, serveur HTTP de bouclage, faux CLI, simulation inter-processus du verrou par ré-exécution du binaire de test). Aucune installation aip ni compte réel sur la machine de développement : **aucune requête vers Anthropic/OpenAI, aucune preuve sur un vrai profil**. Tests : RED prouvé (anciens appels rétablis → aucun POST), puis `cargo test --offline --locked --lib` 208/0/5, `npm run typecheck`, `npm run build`, `rustfmt --check` sur les fichiers touchés (§11).

## Limites
- Codex : la classe « grant importé, pas de `aip codex login` » n'est pas renouvelable sans interaction de l'utilisateur (sinon déconnexion de la session d'origine) ; course résiduelle théorique avec un CLI en pleine requête dans sa fenêtre de 5 min (le CLI n'a pas de verrou inter-processus), évitée en fonctionnement normal par la marge de 10 min et la relecture du CLI.
- Claude : deux instantanés partageant le même refresh token ne sont renouvelés ni l'un ni l'autre (refaire `aip save claude`).
- Hors unix : pas de renouvellement (pas de verrou compatible), lecture inchangée.
- Validation native (app lancée sur une vraie installation aip) non faite.

## Suite éventuelle
1. Test manuel sur une vraie installation aip (build local, app installée quittée) : vérifier qu'un profil dormant reste « Connecté » au-delà de l'expiration et que `aip` continue de fonctionner.
2. Avant une release : `cargo fmt` + `cargo test --locked` sur la branche de release (CI `check`).
