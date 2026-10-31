# Claude multi-comptes — portage sur Quota Monitor 3.1.0

Branche `feature/claude-multi-account-3.1`, base `1bcc2e5` (= `v3.1.0` = `origin/main`). Portage de `2d298ad` (multi-comptes) et `6a6a395` (décompte visible), repris hunk par hunk dans l'architecture 3.1.0 (`panel_model()`, fenêtre compacte, Réglages à onglets, Fable). Aucun fichier d'icône, de mascotte, d'updater, de `tauri.conf.json`, `Cargo.*` ou `package*.json` modifié ; version 3.1.0 conservée.

## Comportement

- Consultation uniquement : aucun login, changement de session ou accès direct aux identifiants OAuth/trousseau.
- Profils : `CLAUDE_CONFIG_DIR` absolu, sinon `~/.claude` ; détection `~/.claude-*` ; ajout manuel absolu ; déduplication canonique/symlinks. Un dossier indisponible reste représenté sans bloquer les autres.
- Collecte sérialisée `initialize` / `get_usage` ; `CLAUDE_CONFIG_DIR` défini uniquement sur le processus enfant. Cache, erreur, rafraîchissement et ticket par identité canonique ; anti-chevauchement et résultats périmés rejetés. Les changements d'affichage ne relancent pas la collecte.
- `prefs.json` 3.1.0 (mode d'affichage, Fable, ntfy, actualisation) se charge sans perte ; `claudeAccounts` absent → Unique, principal (test `v310_preferences_file_loads_without_loss_and_defaults_to_single_primary`).
- Réglages (onglet Fournisseurs) : Unique|Multi dès deux profils, renommage, visibilité (affichage seul), ajout manuel.
- **Un seul compte visible = carte et lignes 3.1.0 exactes** (menu natif, fenêtre compacte, Dashboard, tray Windows/Linux) : pas de nom de compte, pas d'appariement.
- Multi : une carte « Claude · {profil} » par compte visible, dans `panel_model()` donc **identique dans le menu natif et la fenêtre compacte**. Bloc apparié de 40 pt : 5 h à l'épaisseur Unique mesurée sur 3.1.0 (menu AppKit 5 pt rayon 2,5 ; fenêtre compacte 5 px ; Dashboard React 7 px rayon 8), 7 j à 3 pt/px, écart 2. Entête « décompte · valeur » (3h30, 42 min, 3j 8h) calculé depuis `resetAt` et l'horloge locale, redessiné à la minute sans collecte. Aucune agrégation, couleurs et thèmes existants.
- Fable : préférence 3.1.0 appliquée à chaque compte (vue d'affichage) ; libellé « Limite Fable » ; si la fenêtre hebdomadaire est coalescée dans Fable, la barre 7 j est préfixée « Fable · ». Rien n'est masqué pour faire tenir la carte (Fable et usage supplémentaire restent visibles).
- Notifications : **politique 3.1.0 conservée** (alerte au retour à 100 % restant, une fois par cycle). Appliquée à tous les profils (visibles ou non), clé stable `Claude:{id profil}:{métrique}`, migration unique des anciennes clés du principal (Fable inclus), sans doublon ; nom du compte dans le message dès deux profils. Alertes Fable muettes si Fable désactivé.

## Fichiers

| Périmètre | Fichiers |
| --- | --- |
| Profils, collecte, préférences, notifications | `src-tauri/src/{claude_accounts.rs,claude_cli.rs,claude.rs,prefs.rs,ntfy.rs}` |
| `panel_model`, menu AppKit, compact, tray, fixtures natives | `src-tauri/src/lib.rs` |
| Cartes et contrats React | `src/features/quota/{types.ts,providers.ts,ProviderCard.tsx}` |
| Fenêtre compacte | `src/features/compact/{types.ts,CompactPanel.tsx,compact.scss}` |
| Réglages | `src/features/settings/{Settings.tsx,ClaudeAccountsSettings.tsx,claude-accounts.scss}` |
| Dashboard | `src/pages/Dashboard/{Dashboard.tsx,dashboard.scss}` |
| Tests et fixtures | `src/features/quota/claudeProfiles.test.cjs`, `scripts/{compact-window,quota-providers}.test.mjs`, `scripts/claude-renderer-fixture.mjs`, `tests/renderer/compact.tsx` |

## Vérifications exécutées

- `cargo test --offline` (src-tauri) : **106 réussis, 0 échec, 2 ignorés** (intégrations aux vrais comptes).
- Node 24.16.0 : `node --test scripts/*.test.mjs src/features/quota/claudeProfiles.test.cjs` : **55 réussis, 0 échec** ; `npm run build` (tsc + Vite) OK.
- `--quota-ui-smoke` natif : 53 PASS, 0 échec (épaisseurs mesurées 5/5 Unique, 5/3 Multi).
- Non-régression : smoke natif 3.1.0 (`1bcc2e5`) et port lancés dans la même minute → **10/10 captures communes identiques au pixel** (deux fournisseurs, Codex en erreur, Claude en attente, aucun fournisseur, icône de barre empilée ; clair/sombre).
- Fixtures React (IPC simulé) : Dashboard, fenêtre compacte et Réglages, clair/sombre ; Multi, Fable, erreurs/épuisé/manquant, un seul visible, Unique, aucun compte ; aucun débordement horizontal.

## Limites

- Windows/Linux : rendu React et helper tray testés sur macOS uniquement, pas d'exécution native.
- Aucun vrai compte interrogé, aucune notification/ntfy réelle, application normale jamais lancée.
- Les preuves (`evidence/`) ne sont pas commitées.
