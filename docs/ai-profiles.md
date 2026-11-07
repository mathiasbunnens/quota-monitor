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

**Codex** : lecture du vrai `cli-config/auth.json` puis `GET https://chatgpt.com/backend-api/wham/usage` avec bearer et `ChatGPT-Account-Id`. Pas d'app-server sur ces homes, car il peut renouveler un grant importé/partagé. Relecture de la connexion (bearer **et account_id**) après HTTP ; un remplacement concurrent invalide la réponse. Les empreintes d'identité et normalisations existantes restent utilisées pour les vues Codex et la déduplication des comptes authentifiés.

Les bases HTTPS sont fixes en production, redirections désactivées, délais bornés ; les bases loopback ne servent qu'aux tests synthétiques. Réutilisation des parseurs existants pour pourcentages, resets, plan, Fable et erreurs. Minimum 120 s entre lectures API par compte ; pauses supplémentaires sur limitation HTTP. Aucun fichier d'auth, metadata, marqueur actif ni entrée Keychain n'est écrit. Aucun reset payant n'est consommé.

## Suivi continu : limite d'authentification explicite

Le suivi est **continu tant que le propriétaire renouvelle sa session**, pas un renouvellement autonome de profils dormants.

À chaque cycle, Quota Monitor recharge les credentials, refuse un access token localement expiré (avec la marge de sécurité commune), puis reprend automatiquement dès que le CLI/manager publie une nouvelle connexion valide. Expiration et 401 donnent une session expirée sans métriques ; connexion absente/incomplète, identité non vérifiable, 403, 429, timeout et erreur serveur restent explicites, jamais un faux zéro. Pour ai-profiles, aucune ancienne métrique Claude n'est conservée après une erreur, même transitoire : sans vérification d'identité réussie à ce cycle, elle pourrait appartenir à une connexion remplacée entre-temps. Les autres collecteurs conservent leurs règles historiques.

**Pourquoi ne pas appeler `oauth_refresh` ?** L'upstream Claude renouvelle par une vraie session PTY avec prompt et écriture du credential propriétaire, sans protocole de verrouillage interprocessus partagé avec un collecteur externe. Codex peut porter un grant importé/partagé, sans l'autorisation d'ownership d'aip. Faire tourner ces refresh tokens indépendamment peut révoquer la session source. L'adaptateur n'utilise donc ni la rotation aip, ni un verrou privé prétendument compatible, ni une copie dans un home artificiel. Un compte dormant expiré nécessite un renouvellement par son CLI/manager ; Quota Monitor ne peut pas le maintenir connecté seul en sécurité.

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
