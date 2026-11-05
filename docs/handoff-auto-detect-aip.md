# Handoff — profils `aip` : détection automatique et lecture des quotas (branche `feature/auto-detect-aip`)

**Direction courante (t_bde633ac)** : Quota Monitor détecte tout seul les profils du gestionnaire `aip` (<https://github.com/dqn/aip>, v0.11.0, rév. `3961fa0`) pour Claude et Codex, les montre sous leur nom aip et **lit leurs quotas** avec l'identifiant sauvegardé dans chaque profil (en mémoire, une requête HTTPS, jamais de CLI, jamais de rafraîchissement, jamais de bascule du compte actif). Base : `3258db7` (= 3.2.0). Rien n'est poussé, publié ni fusionné.

## Comportement livré
- Découverte à chaque démarrage/rafraîchissement via le socle commun (`account_sources::reconcile(all(..))`) ; aucun bouton d'import, mode Unique/Multi jamais changé par la découverte ; sans aip, résultat identique à 3.2.0 (tests `legacy_discover`).
- Profil aip complet = compte lisible (`Collect::Credential`) : affiché par défaut en Multi comme un dossier frère, nom/visibilité/exclusion/sélection Unique comme tout compte ; profil sans payload = « Non collectable » (`Blocked(Incomplete)`).
- Claude : `_current` replié dans le Principal (une seule collecte, par marqueur) ; les autres instantanés sont lus via `GET api.anthropic.com/api/oauth/usage` avec leur `accessToken` (5 h, 7 j, Fable, plan, extra usage) ; jeton expiré/révoqué → « Session expirée » avec la marche à suivre, sans quota fictif.
- Codex : chaque home aip est lu via `GET chatgpt.com/backend-api/wham/usage` avec son `access_token` + `ChatGPT-Account-Id` (fenêtres 5 h / 7 j, réserves, plan, crédits de reset lus, jamais consommés) ; copie du compte principal = « Même compte » ; jeton expiré → « Session expirée ».
- Jamais : trousseau, `aip`/`switch`/`login`/`logout`, lancement de CLI pour un profil aip, écriture dans les fichiers aip, rafraîchissement de jeton, redirection HTTP, jeton dans un log/message/préférence.

## Fichiers utiles
- Transport : `src-tauri/src/oauth_usage.rs` (chargement des identifiants, HTTP, projections, serveur de test local).
- Socle : `src-tauri/src/aip.rs`, `src-tauri/src/account_sources.rs`.
- Claude : `src-tauri/src/claude_accounts.rs`, `src-tauri/src/claude.rs` (`poll_profile`, `credential_snapshot`, plancher 120 s).
- Codex : `src-tauri/src/codex_accounts.rs` (`Discovered::credential`), `src-tauri/src/codex.rs` (`credential_plan`, `Transport`, `poll_credential`).
- UI : `src-tauri/src/account_menu.rs`, `src/features/quota/{accountState,types}.ts` (« Session expirée »).
- Détails, sources et preuves : `docs/aip-profiles.md` §10 (§1 format ; §3/§5/§7/§8 = ancienne direction, historique).

## Preuves
Synthétiques uniquement (dossiers temporaires, identifiants fictifs, serveur HTTP de bouclage, faux CLI). Aucune installation aip sur la machine de développement : **aucun test sur un compte réel, aucune requête vers Anthropic/OpenAI**. Tests : `cargo test --offline --locked --lib` 192/0/5, `node --test src/features/quota/*.test.cjs` 88/88, `npm run typecheck`, `npm run build`, `cargo fmt --check` (§10).

## Limites
- Claude dormant : lisible tant que l'`accessToken` de l'instantané est valide (quelques heures après le dernier `aip switch`, ou en continu si le tableau de bord d'aip, qui rafraîchit les instantanés, tourne). Rafraîchir depuis Quota Monitor impose une rotation + écriture sous le verrou d'aip : non fait, autorisation explicite requise (§10).
- Codex dormant : lisible tant que le JWT n'a pas expiré ; sinon « Session expirée » jusqu'au prochain `aip codex`.
- Validation native (app lancée, vraie installation aip) non faite.

## Suite éventuelle
1. Test manuel sur une vraie installation aip (build local, app installée quittée).
2. Décision : rafraîchissement des instantanés Claude dormants selon le protocole d'aip (voir §10) — uniquement sur autorisation explicite.
3. Avant une release : `cargo fmt` + `cargo test --locked` sur la branche de release (CI `check`).
