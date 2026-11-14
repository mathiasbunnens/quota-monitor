# Comptes bartekczyz/ai-profiles

## Référence et découverte

Support macOS du manager **[bartekczyz/ai-profiles](https://github.com/bartekczyz/ai-profiles)**, distinct de `dqn/aip`. Format inspecté à la révision **26bf0e8b25a1908474ccf822200bb76698ae06cb** :

- `apps/ai-profiles/src-tauri/src/{profiles.rs,paths.rs,launchers/cli.rs}` : métadonnées, dossiers et wrappers ;
- `usage/{credentials.rs,token_account.rs,quota.rs,refresh.rs,codex.rs}` et `accounts.rs` : credentials, identité, usage et renouvellement.

Le fichier `~/Library/Application Support/ai-profiles/profiles.json` contient `{"profiles":[…]}`. Champs utilisés : `id`, `name`, `app` (`claude` par défaut, `codex` explicitement), `surfaces.cli`. Les champs inconnus ne sont pas réécrits. Les identifiants de dossier sont validés, les noms bornés et le fichier limité à 1 Mio. Une entrée CLI utilise le **vrai** dossier `profiles/<id>/cli-config`. Une entrée bureau uniquement reste listée, non collectable, sans inventer de home CLI ni de quota. Un dossier CLI sans connexion est découvert, puis affiche son erreur de connexion réelle.

Les comptes apparaissent sans import ni ajout manuel, avec le suffixe initial `· ai-profiles`. Les noms personnalisés, exclusions, visibilités et choix Unique/Multi restent prioritaires ; la découverte ne change jamais le mode. Identités de dossier canoniques pour la déduplication, chemins bruts du manager pour l'accès aux credentials : les alias manuels/symlinks ne doivent pas contourner le transport protégé. Le compte principal conserve son environnement et son collecteur historiques, même s'il désigne le même dossier ; il ne devient pas un second compte. Les protections aip restent distinctes. Codex partage la borne existante de huit lectures de credentials supplémentaires entre aip et ai-profiles (les homes déjà sauvegardés conservent leur traitement historique).

## Collecte réelle et transport

La découverte ne lit aucun secret. La collecte lit seulement en mémoire les credentials du compte concerné ; aucune commande de connexion, déconnexion, switch ou import n'est exécutée.

**Claude** : `.credentials.json` si présent ; sinon, sur macOS, le service Keychain `Claude Code-credentials-<sha256(raw_cli_config_dir)[:8]>`, compte `$USER`, via `/usr/bin/security find-generic-password`. Pas de repli vers le service principal non suffixé. Une valeur brute différente peut désigner un autre service : le chemin enregistré dans les préférences n'écrase donc pas le chemin brut dérivé du manager. Un fichier corrompu ne déclenche pas de repli Keychain. Lecture Keychain bornée à 64 Kio et cinq secondes, erreurs expurgées.

Avant d'afficher un quota, `GET https://api.anthropic.com/api/oauth/profile` doit confirmer `/account/uuid` contre `.claude.json` → `oauthAccount.accountUuid`. Cette vérification est **fail-closed**, plus stricte que l'upstream : identité absente, incohérente ou devenue indisponible = quotas masqués. Elle protège notamment les collisions du namespace Keychain de 32 bits. Puis lecture de `/api/oauth/usage` avec le même bearer et `anthropic-beta: oauth-2025-04-20`. L'identité locale et le bearer sont relus après la réponse ; un changement pendant la collecte invalide le résultat. Pour une erreur d'identité, même les anciennes métriques et le plan sont supprimés.

**Codex** : lecture du vrai `cli-config/auth.json` puis `GET https://chatgpt.com/backend-api/wham/usage` avec bearer et `ChatGPT-Account-Id`. L'app-server officiel n'est lancé sur ces homes que pour le renouvellement (section suivante), jamais pour collecter. Relecture de la connexion (bearer **et account_id**) après HTTP ; un remplacement concurrent invalide la réponse. Les empreintes d'identité et normalisations existantes restent utilisées pour les vues Codex et la déduplication des comptes authentifiés.

Les bases HTTPS sont fixes en production, redirections désactivées, délais bornés ; les bases loopback ne servent qu'aux tests synthétiques. Réutilisation des parseurs existants pour pourcentages, resets, plan, Fable et erreurs. Minimum 120 s entre lectures API par compte ; pauses supplémentaires sur limitation HTTP. Quota Monitor n'écrit aucun fichier d'auth, metadata, marqueur actif ni entrée Keychain. Aucun reset payant n'est consommé.

## Suivi continu : renouvellement par l'outil officiel (option A)

Quota Monitor ne détient **aucune connexion propre** et n'appelle **jamais** un endpoint OAuth token/refresh pour ces profils (`oauth_refresh` n'est pas atteignable depuis `Transport::ManagedCredential`). Une 2e connexion OAuth sur le même compte risquerait de déconnecter le profil (rotation des refresh tokens : `refresh_token_reused`). Quand le jeton d'accès du profil arrive à **4 min** de son expiration (ou est expiré), Quota Monitor lance l'outil officiel **sur le dossier de ce profil**, qui renouvelle et écrit lui-même la connexion, puis relit :

| Fournisseur | Commande (version vérifiée) | Pourquoi sans quota |
|---|---|---|
| Claude | `claude -p --input-format stream-json --output-format stream-json …` avec `CLAUDE_CONFIG_DIR=<cli-config>`, requêtes de contrôle `initialize` + `get_usage` (Claude Code **2.1.299**) | `get_usage` = `GET /api/oauth/usage` après `checkAndRefreshOAuthTokenIfNeeded` (+ retry 401) ; aucun message utilisateur, aucun appel modèle. C'est déjà le transport du compte principal. |
| Codex | `codex app-server` avec `CODEX_HOME=<cli-config>`, `initialize` + `getAuthStatus {includeToken:true, refreshToken:<force>}` (codex-cli **0.164.0**, `rust-v0.164.0`) | `getAuthStatus` charge l'auth de ce home ; `auth()` renouvelle 5 min avant expiration, `refreshToken:true` force (`refresh_token_if_requested`). Aucun thread/tour, aucun appel modèle. Le jeton renvoyé n'est testé qu'en présence puis oublié. |

Sources : chaînes du binaire Claude Code installé et sondes sur dossier jetable (`get_usage` sans credential → `rate_limits_available:false`, sans appel modèle) ; code `codex-rs` du tag `rust-v0.164.0` (`app-server/src/request_processors/account_processor.rs`, `login/src/auth/manager.rs`) et sonde `getAuthStatus` sur `CODEX_HOME` jetable. Aucune commande `login`, navigateur ou prompt interactif n'est lancée (stdin réservé au protocole JSON, délai borné ; le processus est tué au-delà).

Planificateur (`official_refresh.rs`), par profil : un seul renouvellement à la fois (drapeau `busy`, aucun verrou tenu pendant l'exécution) ; le succès n'est accepté que si la relecture montre une **nouvelle** connexion hors marge (un outil qui n'a pas joint le fournisseur renvoie l'ancien jeton = échec) ; échec/timeout → backoff 60 s, 120 s… plafonné à 30 min (« Actif », avertissement) ; refus (Claude `rate_limits_available:false`, Codex `authToken:null`, connexion disparue) → **« À reconnecter »** sans quota, verrouillé tant que la connexion ne change pas : jamais de boucle. Dès que l'utilisateur reconnecte le profil via ai-profiles (nouveau jeton), le suivi reprend seul. Sur 401 d'un jeton non expiré localement, un seul renouvellement forcé, jamais deux rotations par cycle.

**Concurrence avec un terminal ouvert sur le même profil.** Le verrou de Quota Monitor empêche deux renouvellements simultanés de son côté. Côté CLI : Codex 0.164 relit `auth.json` avant de rafraîchir et adopte un jeton déjà renouvelé (rechargement gardé) ; Claude Code 2.1.299 contient `checkAndRefreshOAuthTokenIfNeeded` et `clearOAuthTokenCache` (relecture des credentials avant refresh présumée d'après ces noms, comportement exact non prouvé). Un CLI ouvert et l'outil lancé par Quota Monitor restent néanmoins deux processus du **même outil officiel** sans verrou interprocessus garanti par le fournisseur : c'est le même risque que deux terminaux ouverts sur ce profil, pas une connexion supplémentaire. Non validé sur comptes réels.

UI : Réglages → Fournisseurs et vue principale affichent par profil « Actif », « Renouvelé via claude|codex » ou « À reconnecter » (indice : reconnecter le profil dans ai-profiles). Les profils aip et le principal ne sont pas concernés.

Le collecteur aip existant garde son propre protocole de renouvellement/verrous et ses marqueurs d'ownership, inchangés.

## Vérification reproductible

```sh
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --lib ai_profiles -- --nocapture
# Régressions ciblées, séparément :
# account_sources::tests, claude_accounts::tests, codex_accounts::tests,
# claude::tests, codex::tests, oauth_usage::tests, oauth_refresh::tests
cargo check --offline --locked --manifest-path src-tauri/Cargo.toml --lib
git diff --check
```

Tests synthétiques : fixtures profiles.json, découverte/réconciliation et préférences, chemins bruts/alias/principal, slot Keychain exact injecté (jamais le trousseau réel), collision/identité manquante, collecteurs Claude/Codex sur plusieurs cycles, expiration puis renouvellement atomique par un faux propriétaire, relecture concurrente pendant HTTP, 401/403/429/500, réponse malformée et timeout. Les tests fake-CLI existants vérifient l'environnement hérité du principal, les variables exactes des homes ordinaires et l'absence de lancement sur les profils de managers. Les tests de renouvellement aip restent exercés. Les tests unitaires ne peuvent pas appeler la nouvelle lecture Keychain native (`cfg(test)` renvoie NoToken).

L'inspection réelle limitée aux métadonnées sur la machine de développement n'a trouvé ni racine ai-profiles ni profiles.json. **Aucun compte réel, credential réel, accès Keychain natif ou appel fournisseur réel n'a donc été validé pour cet adaptateur.** Le compte rendu durable sépare ces constats des simulations HTTP et contient commandes, résultats et SHA.

Aucun changement frontend, version, release ou installation. Une **build de test**, puis validation sur profils réels, restent nécessaires avant de tester cette fonctionnalité dans l'application installée. Ne pas lancer une seconde instance partageant ses préférences/auth avec `/Applications/Quota Monitor.app`.
