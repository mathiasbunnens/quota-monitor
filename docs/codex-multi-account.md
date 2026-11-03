# Suivi de plusieurs comptes Codex

Même parcours que le multi-compte Claude (`docs/claude-multi-account.md`), mêmes composants et styles. Une « source » Codex est un dossier `CODEX_HOME` ; un profil TOML n'est pas une identité et n'est pas utilisé.

## Comportement

- **Compte principal** : inchangé. Le collecteur historique lance `codex app-server` sans injecter `CODEX_HOME` (la variable héritée de l'app ou `~/.codex`). Actions, resets, cadence, connexion depuis l'app restent les siens.
- **Dossiers supplémentaires** : découverte des dossiers `~/.codex-*` (noms et chemins seulement, aucun contenu lu), suivis automatiquement (sans étape « Suivre »), ajout manuel par le bouton « + » (sélecteur de dossier ou chemin absolu), renommage, « Supprimer <compte> » dans le détail (retire le suivi, ne supprime jamais de fichier). Au plus 8 dossiers supplémentaires suivis.
- **Détail d'un compte** (chevron › de la ligne) : se déplie sous la ligne et ne contient que le chemin du dossier (lecture seule, copiable, infobulle) et « Supprimer <compte> » (absent pour le principal). Réinitialisation, état, renommage et interrupteur restent sur la ligne.
- **Suppression** : le dossier est mémorisé dans `codexAccounts.excluded` (identités `profile:…`, absent des anciennes préférences = liste vide) ; les analyses suivantes ne le rajoutent pas. Le « + » (ou un chemin ajouté à la main) le réintègre. Aucun fichier n'est touché.
- **Visibilité d'un dossier détecté sans entrée enregistrée** : affiché en Multi, mis de côté en Unique (qui n'affiche que le compte choisi, donc le principal par défaut). Un compte unique reste donc inchangé ; une entrée déjà enregistrée garde son nom et sa visibilité. L'ordre est stable (principal, dossiers détectés triés, puis ajouts manuels).
- **Unique | Multi** et interrupteurs de visibilité comme Claude (Réglages → Fournisseurs → Codex). La visibilité ne change que l'affichage ; la collecte dépend du suivi et de l'activation de Codex.
- **Présentation** : le réglage unique Compacte/Classique (Affichage) s'applique à chaque carte de compte, dans le menu natif, la fenêtre compacte et le Dashboard React.

## Collecte par dossier suivi

- Un `codex app-server` enfant par dossier, `CODEX_HOME` = valeur brute suivie, posée **sur l'enfant seulement** (le CLI la canonicalise lui-même, sans repli vers le principal). L'environnement global n'est jamais modifié.
- Séquence : `initialize` → `account/read {"refreshToken":false}` → `account/rateLimits/read` seulement si le compte est de type ChatGPT. `account` nul : état « Non connecté », message avec la commande à lancer, aucun login lancé, aucun quota inventé.
- Snapshot, état d'actualisation, erreurs et cache distincts par compte (`AccountStore`), avec une **génération** par enregistrement : une réponse en vol d'un dossier retiré ou relancé est rejetée. Démarrages échelonnés (2 s) ; même cadence que le principal.
- Un dossier absent → « Indisponible » ; une erreur ou un enfant qui meurt ne touche que sa carte.
- **Doublon d'identité** : empreinte en mémoire (hash) de l'e-mail renvoyé par `account/read`, à défaut l'identifiant de compte ChatGPT. Jamais persistée, sérialisée ni affichée ; aucun token lu. Le second dossier porte « Même compte que « X » : quotas non additionnés. » et n'émet pas de notification.
- Aucune agrégation : chaque carte montre ses propres pourcentages.

## Préférences (`prefs.json`)

Nouvelle clé `codexAccounts` au même format que `claudeAccounts` : `{ mode: "single"|"multi", selectedId, profiles: [{ id, path, name, visible }] }`. Absente (fichiers 3.1.0 et builds Claude multi-compte) : Unique, principal seul, comportement historique. L'identifiant d'un dossier est dérivé de son chemin normalisé (comme Claude) : déplacer un dossier crée un nouveau compte suivi.

## Notifications

Clé de ledger `Codex:<id>:<période>` par compte ; les anciennes clés `Codex:<période>` du principal sont migrées une seule fois. Libellé « Codex · nom » quand plusieurs dossiers sont suivis. Les doublons d'identité sont exclus. Politique de déclenchement inchangée.

## Connecter un second compte (à faire par l'utilisateur)

Dans un terminal, sans quitter la session principale :

```sh
mkdir -p ~/.codex-travail
CODEX_HOME=~/.codex-travail codex login
# ou, sans navigateur local : CODEX_HOME=~/.codex-travail codex login --device-auth
```

`CODEX_HOME` est la variable documentée par OpenAI pour choisir le dossier de configuration/authentification de Codex. Ensuite : le dossier `.codex-travail` apparaît tout seul dans Réglages → Fournisseurs → Codex (« Non connecté » tant que le login n'est pas fait). L'app ne lance jamais ce login elle-même.

## Sécurité

Jamais de lecture/copie d'`auth.json`, de tokens ou du trousseau, jamais de `login`/`logout`, jamais de changement de la session Codex active. `refreshToken:false` ne garantit pas que le CLI n'écrit rien dans son propre dossier (journaux, bases SQLite) : le test runtime sur dossiers vides le montre.

## Fichiers

| Rôle | Fichiers |
| --- | --- |
| Découverte, suivi, validation | `src-tauri/src/codex_accounts.rs` |
| Enfants, génération, doublons, commandes | `src-tauri/src/codex.rs` |
| Prefs `codexAccounts`, réconciliation | `src-tauri/src/prefs.rs` |
| Cartes natives (`codex_account_panels`), fixtures | `src-tauri/src/lib.rs` |
| Notifications par compte | `src-tauri/src/ntfy.rs` |
| Plan React, types, Dashboard, Réglages | `src/features/quota/{providers,types}.ts`, `src/pages/Dashboard/Dashboard.tsx`, `src/features/settings/{Settings,ClaudeAccountsSettings}.tsx` |
| Tests | `src/features/quota/codexAccounts.test.cjs`, tests Rust `codex::tests`, `codex_accounts::tests`, `prefs::tests`, `ntfy::tests` |

## Preuves et limites

- Faux CLI (tests Rust) : un enfant par dossier, `CODEX_HOME` sur l'enfant seul, `refreshToken:false`, aucun login/logout, panne isolée, doublon détecté, environnement du processus inchangé.
- Runtime réel (test ignoré, opt-in) : deux `CODEX_HOME` vides sous `evidence/` → `account` nul pour chacun, aucune fuite du compte principal, délai dur et arrêt des processus.
- **Non prouvé** : deux vrais comptes authentifiés simultanément (aucune session utilisateur n'a été utilisée), backend keyring réel par dossier, comportement d'une future version du CLI.
