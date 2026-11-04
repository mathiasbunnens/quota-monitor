# Faisabilité bornée : plusieurs comptes Codex

## Conclusion

**Faisable architecturalement avec un `CODEX_HOME` distinct par environnement de compte et un app-server enfant par environnement ; pas démontré en fonctionnement avec plusieurs comptes authentifiés.** Un profil TOML n'est pas une identité ChatGPT. Aucun basculement du compte global, copie de credentials ou accès direct au trousseau n'est nécessaire pour la stratégie proposée.

Cette étude n'implémente rien. Elle vérifie le code officiel du **tag `rust-v0.164.0`**, la version du binaire disponible et le collecteur local. Elle distingue les preuves statiques des validations encore nécessaires. Deux homes différents peuvent contenir le **même** compte : l'identité locale d'un environnement ne garantit pas une identité utilisateur différente.

## Périmètre et résultats exécutés

- Workspace : `/Users/mathiasbunnens/Downloads/quota-monitor/.worktrees/claude-multi-3.1`.
- Branche vérifiée : `feature/claude-multi-account-3.1`.
- HEAD vérifié : `716e3f017ec6d4e53662f7f5a706e5c81a68f387`.
- Binaire interrogé : `/Users/mathiasbunnens/.nvm/versions/node/v24.16.0/bin/codex` ; `--version` retourne **`codex-cli 0.164.0`**. Cette sortie ne certifie pas la reproductibilité du binaire depuis les sources.
- `git ls-remote --tags https://github.com/openai/codex.git '*0.164.0*'` a retourné, pour la version stable :

```text
fe3b29455488c2e189e43ad5526a107900c5e1b7 refs/tags/rust-v0.164.0
6aaac694803004a63c6f393d153c96d22521a6b9 refs/tags/rust-v0.164.0^{}
```

Les sources ci-dessous ont été récupérées directement depuis GitHub avec `curl -fLsS --max-time 25` ; l'arbre GitHub du commit a servi à retrouver les modules déplacés. Les extraits sont du code public, sans données d'authentification locales.

**Découverte locale limitée aux noms/chemins `~/.codex-*` : aucun dossier ou lien correspondant détecté.** La commande de contrôle, sans récursion ni ouverture de fichier :

```sh
for p in /Users/mathiasbunnens/.codex-*; do
  if test -d "$p"; then printf 'directory %s\n' "$p"
  elif test -L "$p"; then printf 'symlink %s\n' "$p"
  fi
done
```

Sa sortie est vide. Le glob non développé imprimé par un premier `printf` a été distingué d'un vrai chemin par ce contrôle. Ce constat ne concerne ni le home principal `~/.codex`, ni des chemins personnalisés ailleurs, ni des comptes potentiellement enregistrés au trousseau. Il ne signifie pas « aucun compte Codex ».

Aucune application normale n'a été lancée. Aucun `login`, `logout`, modèle, thread, tour, consommation de crédit de reset ou notification n'a été demandé. Aucun `auth.json`, token, contenu de keychain ou configuration privée de compte n'a été lu ou copié. Aucun environnement global n'a été modifié.

### Sonde app-server : non exécutée volontairement

La sonde avec deux homes vides était facultative. Elle n'a pas été lancée : la contrainte « aucun autre fichier » exclut ici de créer des homes et les fichiers d'état que le CLI pourrait y produire ; un lancement dans un home réel violerait le périmètre. **Il n'existe donc pas de résultat runtime `account: null` à revendiquer**, ni de validation de deux processus simultanés. Aucun enfant app-server n'a été créé par cette étude, donc aucun enfant à terminer.

Pour une validation ultérieure explicitement autorisée, utiliser uniquement deux répertoires nouvellement créés sous `evidence/t_33379e53/feasibility`, définir `TMPDIR` et `CODEX_HOME` sur les enfants seulement, partir d'un environnement autorisé minimal sans variables d'authentification héritées, et choisir un stockage `ephemeral` pour ne pas solliciter le trousseau. Un tel override teste la mécanique de séparation, **pas le backend keyring réel**. Faire uniquement `initialize` → `initialized` → `account/read` avec `refreshToken:false`, n'imprimer que statut/null, imposer un délai dur et terminer/récolter le groupe de processus dans un `finally`. Ne pas requêter les quotas si le compte est null. Vérifier séparément que l'override est accepté par cette version et les règles système ; ne pas contourner une politique administrée.

## 1. `CODEX_HOME` et clé keyring : preuve versionnée

### Résolution du home : canonicalisation stricte de l'override

Source officielle [utils/home-dir/src/lib.rs, tag 0.164.0](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/utils/home-dir/src/lib.rs).

`find_codex_home()` ignore une valeur vide, sinon `find_codex_home_from_env()` vérifie les métadonnées du chemin, impose un répertoire existant puis le canonicalise. Un chemin absent, un fichier ou une erreur de canonicalisation provoque une erreur ; **pas de repli silencieux vers le compte principal**. Sans override, la fonction construit `home_dir()/.codex`, sans vérifier ici son existence.

Extraits innocents :

```rust
std::env::var("CODEX_HOME").ok().filter(|val| !val.is_empty());
// branche Some(val), après vérification metadata.is_dir():
let canonical = path.canonicalize().map_err(|err| { /* contexte d'erreur */ })?;
AbsolutePathBuf::from_absolute_path(canonical)
```

Les tests publiés dans ce module couvrent chemin absent, chemin fichier, canonicalisation et défaut utilisateur ; ils ont été **lus, pas exécutés** dans cette étude.

### Backend direct : service et clé dérivés du chemin

Source officielle [login/src/auth/storage.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/login/src/auth/storage.rs), fonctions `compute_store_key`, `DirectKeyringAuthStorage::load` et `create_keyring_auth_storage`.

```rust
const KEYRING_SERVICE: &str = "Codex Auth";
let canonical = codex_home
    .canonicalize()
    .unwrap_or_else(|_| codex_home.to_path_buf());
let path_str = canonical.to_string_lossy();
let mut hasher = Sha256::new();
hasher.update(path_str.as_bytes());
let digest = hasher.finalize();
let hex = format!("{digest:x}");
let truncated = hex.get(..16).unwrap_or(&hex);
Ok(format!("cli|{truncated}"))
```

**Réponse précise :** service `Codex Auth`, clé `cli|` suivie des 16 premiers caractères hexadécimaux du SHA-256 des octets de la représentation `to_string_lossy()` du chemin canonique. En cas d'échec de canonicalisation **dans ce helper**, le chemin fourni est utilisé tel quel. Ce repli local ne doit pas être confondu avec la validation stricte du `CODEX_HOME` explicite au démarrage. Un home valide passant par cette résolution sera déjà canonique.

Conséquences : deux alias/symlinks du même dossier ne représentent normalement pas deux slots ; deux dossiers canoniques distincts donnent des clés dérivées distinctes, sous réserve de collision du hash tronqué. Un renommage/déplacement change la clé attendue : ne pas promettre que les credentials keyring suivent le dossier. Ne jamais utiliser ce hash tronqué comme identifiant universel de personne, ni interroger soi-même le trousseau dans Quota Monitor.

### Attention : le backend `secrets` existe aussi dans cette version

Sources officielles : [config/src/types.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/config/src/types.rs), [secrets/src/lib.rs#L184-L208](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/secrets/src/lib.rs#L184-L208), [secrets/src/local.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/secrets/src/local.rs).

`AuthKeyringBackendKind` distingue `Direct` et `Secrets`. Le défaut est `Direct` hors Windows et `Secrets` sous Windows ; une configuration peut sélectionner autrement. Pour `Secrets`, le payload est dans `$CODEX_HOME/secrets/codex_auth.age` et la clé de chiffrement est au service keyring **`codex`**, compte **`secrets|<16 hex SHA-256 du home>`**. `compute_keyring_account` utilise aussi canonicalisation puis repli sur le chemin fourni. Le namespace `CodexAuth` partage cette clé de home avec `ManagedSecrets` et `McpOAuth`, contrairement au namespace gateway qui ajoute un suffixe. Il serait donc faux d'affirmer que toutes les configurations utilisent le service `Codex Auth` ou la clé `cli|…`.

### Les deux sens du mot « fallback »

Dans [storage.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/login/src/auth/storage.rs) et [types.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/config/src/types.rs) :

| Mode | Comportement prouvé par le source |
| --- | --- |
| `file` (défaut de l'enum) | Stockage dans le home sélectionné, fichier `auth.json`. |
| `keyring` | Backend sélectionné ; une indisponibilité n'est pas un repli automatique `auto`. |
| `auto` | `load()` préfère keyring ; absence **ou erreur** entraîne une lecture fichier du même home ; `save()` se replie sur fichier en cas d'échec keyring. |
| `ephemeral` | Mémoire du processus, indexée par la clé dérivée du home. |

Nuance importante : `AutoAuthStorage::load_account_grant()` propage une erreur keyring au lieu d'autoriser un ancien fallback fichier ; ce n'est pas le même contrat que `load()`. Le repli de canonicalisation et le repli de backend sont deux mécanismes différents. Aucun ne constitue une recherche globale des credentials d'un autre home.

## 2. Profils TOML ≠ identité

Source versionnée décisive : [config/src/profile_toml.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/config/src/profile_toml.rs), structure `ConfigProfile` :

```rust
/// Collection of common configuration options that a user can define as a unit
/// in `config.toml`.
pub struct ConfigProfile {
    pub model: Option<String>,
    pub service_tier: Option<String>,
    pub model_provider: Option<String>,
    // approval_policy, sandbox_mode, reasoning, tools, features, etc.
}
```

Ce sont des réglages d'exécution ; cette structure n'offre pas une identité ChatGPT ni un `CODEX_HOME` propre par profil. Choisir un provider peut modifier le mode d'authentification, mais n'est pas un mécanisme fiable pour isoler plusieurs sessions ChatGPT. **Ne pas transformer `[profiles.*]` ou `--profile` en liste de comptes.**

Documentation officielle demandée : [Advanced Configuration](https://developers.openai.com/codex/config-advanced/), notamment la section « Config and state locations » et les couches de configuration projet. Autres références de contexte : [Config basics](https://developers.openai.com/codex/config-basic/), [Authentication](https://developers.openai.com/codex/auth/), [App Server](https://developers.openai.com/codex/app-server/). Ces pages sont évolutives (certaines redirigent actuellement vers `learn.chatgpt.com`) : leurs détails courants ne sont **pas** assimilés à une preuve du tag 0.164.0. Le verdict versionné ci-dessus repose sur GitHub.

## 3. Collecteur local : séparation manquante aujourd'hui

Inspection statique, sans exécuter l'application ni utiliser un vrai compte :

| Fichier / symbole | Constat |
| --- | --- |
| `src-tauri/src/codex.rs:302–357`, `Session::start` | Lance `app-server` sur stdio, stderr ignoré ; ajuste le `PATH` enfant pour Node ; `current_dir(std::env::temp_dir())` évite le cwd projet ; **ne définit pas `CODEX_HOME`**. Le home effectif reste hérité/default. |
| `codex.rs:289–300` | `Session::drop` ferme stdin et appelle `shared.kill_child()` ; mécanisme à conserver par enfant. |
| `codex.rs:366–396` | Requêtes avec identifiant, attente bornée à 30 secondes et vérification du stop. |
| `codex.rs:512,593` | Une seule `Option<Session>` dans la boucle ; `Provider`/`Shared` et leur statut ne portent pas de compte ciblé. |
| `codex.rs:691–711` | `account/read` avec `{"refreshToken":false}` ; quotas via `account/rateLimits/read` uniquement pour `chatgpt` / `chatgptAuthTokens`. Aucun appel modèle dans ce chemin. |
| `src-tauri/src/lib.rs:99–113` | `QuotaState(Mutex<Vec<QuotaPayload>>)` et `set_codex_snapshots` remplacent un seul ensemble de quotas ; manque une dimension environnement de compte. |
| `src-tauri/src/prefs.rs:36–48` | Activation Codex et comptes Claude, mais pas de `codex_accounts`. |
| `src-tauri/src/ntfy.rs:124–149` | Clé actuelle `Codex:<period>` et statut global `ready` ; ferait collision entre comptes sans migration. |

Extrait local central :

```rust
command.current_dir(std::env::temp_dir());
// plus loin, dans la boucle :
.call("account/read", json!({"refreshToken":false}))
// seulement pour les types ChatGPT reconnus :
.call("account/rateLimits/read", Value::Null)
```

Changer le cwd ne change donc pas le compte. Il faut passer `CODEX_HOME` **dans l'environnement de chaque enfant**, pas dans l'environnement global de l'application. Le principal doit garder la sémantique d'héritage actuelle ; ne pas imposer arbitrairement `~/.codex` si le processus parent sélectionne déjà un autre home.

**Lecture seule fonctionnelle ne signifie pas absence garantie d'écriture CLI.** `refreshToken:false` exprime la demande de lecture de compte, mais ne prouve pas que tout le démarrage, le routage workspace ou une lecture réseau de quotas ne provoque jamais de refresh ou d'état disque interne. Le [README app-server versionné](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/app-server/README.md#selected-workspace-routing) décrit notamment une découverte de routage au démarrage pour les connexions enregistrées. Ne pas promettre « zéro réseau » ou « zéro écriture credentials » sur la seule base de ce paramètre.

## 4. Stratégie future et fichiers concernés (proposition, non implémentée)

1. **Comptes — nouveau `src-tauri/src/codex_accounts.rs`.** Catalogue de métadonnées non secrètes : UUID stable de l'environnement, nom éditable, home de lancement original, chemin canonique pour déduplication, visible/activé, principal. Distinguer identité d'environnement et identité du compte distant. Détecter seulement les enfants directs `~/.codex-*` (noms et chemins), puis demander un ajout explicite ; aucun scan de credentials, aucune connexion implicite. Les dossiers absents/non lisibles deviennent des erreurs localisées, pas un fallback principal. Préserver la valeur originale pour le lancement tout en sachant que le CLI la canonicalise.
2. **Prefs — `src-tauri/src/prefs.rs`.** Ajouter `codex_accounts`, mode Unique/Multi, sélection stable et visibilité ; migrer une seule fois le principal historique. Ne rien changer aux comptes Claude ni aux réglages de présentation travaillés séparément. Le masquage visuel ne doit ni arrêter la collecte ni lancer un login. Le désarmement explicite du fournisseur arrête ses enfants.
3. **Workers — `src-tauri/src/codex.rs`.** Un worker / `Shared` / session / file de commandes par ID d'environnement. `Command::env("CODEX_HOME", home)` seulement pour les comptes explicitement ciblés ; principal en héritage compatible. Cwd réellement neutre, hors projet pour la production. Bornes sur nombre d'enfants, backoff, timeouts et décalage des polls. Arrêt + récolte des enfants au retrait, à la désactivation et à la fermeture. Toutes commandes sensibles existantes (connexion, annulation, reset) doivent cibler explicitement un ID et ne pas être diffusées à tous les workers.
4. **Snapshots et caches — `src-tauri/src/codex.rs`, `src-tauri/src/lib.rs`.** Nouveau `AccountSnapshot { account_id, status, rows, checked_at, generation }` ; map par ID au lieu d'un vecteur global. Cache cloisonné par fournisseur + ID + génération d'authentification + limite. Une erreur/connexion expirée affecte seulement le compte ciblé. Invalider les anciens résultats lors d'un changement de home, reconnexion ou changement d'identité distante ; rejeter une réponse en vol d'un worker retiré. Ne jamais interpréter null/erreur comme quota nul ou épuisé. Si persistance future : métadonnées minimales, jamais payload d'authentification.
5. **Ledger — `src-tauri/src/ntfy.rs`.** Passer de `Codex:<period>` à `Codex:<account_id>:<limit_id>:<window>`, avec migration historique vers le principal uniquement. Préserver la politique de notification existante, éviter le double signal entre comptes/alias et ne pas inclure un nom éditable dans la clé. L'état `ready` doit aussi être par compte.
6. **Exposition/présentation — `src-tauri/src/lib.rs` et consommateurs frontend/natifs à inventorier lors de l'implémentation.** Exposer les snapshots structurés et sélectionnés sans écraser l'état brut ; aucune refonte d'interface n'est autorisée par cette étude. Les chemins privés ne doivent pas devenir des télémétries ou identifiants exposés sans nécessité.

Ne pas réutiliser directement le schéma de hash keyring comme clé applicative persistante : il est tronqué, dépend du chemin et connaît plusieurs backends. Un UUID d'environnement préserve renommage affiché, ledger et préférences ; le chemin canonique sert à détecter les alias. Si le dossier est déplacé, l'utilisateur doit confirmer le nouveau lien et le CLI doit rester seul responsable de sa disponibilité d'authentification.

## 5. Limites et critères avant implémentation

### Établi

- Version annoncée du CLI local et correspondance du tag officiel vers le commit indiqués plus haut.
- Isolation du stockage par home dans les sources, avec distinction backend direct / secrets et canonicalisation / fallbacks.
- Profils de configuration distincts des environnements d'authentification.
- Collecteur local actuellement mono-session, avec contrat de lecture de quotas déjà utilisable comme base.
- Aucune entrée `~/.codex-*` détectée par la découverte bornée.

### Non prouvé ici

- Deux comptes réellement distincts, simultanément authentifiés, remontant chacun leurs quotas et leur plan.
- Accès macOS keychain effectif, permissions utilisateur, comportement des refreshs concurrents, rotation/expiration des sessions ou absence totale d'écritures CLI.
- Authentification via variables d'environnement, stockage administré, configurations custom, comportement Windows/Linux et versions autres que 0.164.0.
- Coût mémoire/CPU de plusieurs app-servers, attribution de l'activité globale Codex à un compte particulier et isolation après suppression/recréation ou déplacement d'un dossier.
- Disponibilité universelle des champs de quotas : conserver valeurs absentes et erreurs au lieu d'inventer des chiffres.

### Portes de validation recommandées

D'abord tests locaux avec un faux CLI : environnement principal hérité, override enfant, déduplication symlinks, dossier invalide, erreurs isolées, réponses tardives, arrêt des enfants, migration prefs et ledger, absence de commandes login/reset/modèle pendant le polling. Ensuite, si autorisé séparément, la sonde deux homes vides décrite plus haut. Enfin seulement, un essai opt-in avec deux environnements déjà authentifiés par l'utilisateur, sorties strictement réduites à statut/plan/disponibilité des quotas, sans copie ni lecture directe de credentials. Ce dernier essai est nécessaire avant d'annoncer le multi-compte comme fonctionnel.

## Incidents de collecte documentaire

Le chemin historique `codex-rs/core/src/auth/storage.rs` retourne HTTP 404 à ce tag ; le module réel est `codex-rs/login/src/auth/storage.rs`, retrouvé dans l'arbre officiel. Une tentative de commande Python inline a été bloquée par la politique d'approbation avant exécution ; les vérifications ont été réalisées par commandes directes. Une recherche de noms par l'outil générique dans le home a échoué ; le glob shell borné a fourni le contrôle final, sans lecture de contenu. Aucun de ces incidents ne justifie de substituer des résultats simulés. Aucun test de compilation ou test automatisé n'a été lancé : seul ce rapport est le livrable et aucun code produit n'a changé.
