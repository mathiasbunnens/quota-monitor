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
- Portage initial : aucun vrai compte interrogé. Correctif de connexion ci-dessous : seule une lecture `get_usage` du principal a été exécutée explicitement, sans appel de modèle ni lecture d'identifiants. Aucune notification/ntfy réelle, application normale jamais lancée.

## Correctif : session du compte principal

Le multi-comptes forçait `CLAUDE_CONFIG_DIR=~/.claude` même pour le principal. Sur macOS, cette présence explicite sélectionne une autre session Claude Code : la même lecture renvoyait `subscription_type: null` et `rate_limits_available: false`, alors que sans la variable elle renvoyait `max` et `true`.

- Principal : hériter exactement de l'environnement de l'app, comme en 3.1.0. Ne pas ajouter la variable si elle est absente ; si elle existe, ne pas réécrire sa valeur.
- Autres profils : définir la variable seulement sur l'enfant avec le chemin brut découvert/saisi, sans canonicalisation (identité/cache/déduplication restent canoniques). Le chemin sauvegardé par les Réglages reste brut.
- Réponse sans session : état `not_signed_in`, aucune barre/quota inventé, et indication de connexion propre au profil. Les autres comptes fonctionnent indépendamment.
- Test de régression observé en échec avant le correctif : le faux CLI recevait le chemin du principal au lieu de `<unset>` ; après correction, les deux cas absent/hérité passent, ainsi que les chemins supplémentaires exacts et la déduplication du principal.
- Vérifications du correctif : `cargo test --offline` 109 réussis, 4 ignorés (deux intégrations historiques, un helper de sous-processus exécuté par son test parent, une sonde réelle opt-in) ; 45 tests Node affectés réussis ; `npm run build` OK. Une exécution explicite de `live_primary_collector_is_signed_in --ignored --nocapture` passe avec seulement `subtype=success subscription_type="max" rate_limits_available=true` imprimés.
- Fixture navigateur du profil non connecté inspectée en clair/sombre : indication lisible, aucun débordement horizontal, compte voisin inchangé. Aucune modification de styles.
- Les preuves (`evidence/`) ne sont pas commitées.

## Comptes Claude plus clairs et présentation Compacte | Classique

Deux réglages indépendants :

- **Fournisseurs → Claude** choisit *quels* comptes sont affichés (`Unique | Multi`, dès deux profils).
- **Affichage → Présentation des comptes Claude** choisit *comment* plusieurs comptes visibles sont dessinés : `Compacte` (barres 5 h / 7 j jumelées, décomptes) ou `Classique` (une section par compte avec les lignes séparées historiques : 5 h, hebdomadaire, Fable, pourcentages, réinitialisations, nom et abonnement). Jamais d'agrégation. Ce choix n'est pas nommé « Multi ».

### Ajout d'un compte

- 0 ou 1 profil : bouton **« Suivre un autre compte Claude »** visible immédiatement (nom, abonnement, état du compte affichés ; gestion détaillée repliée).
- Panneau d'ajout : aide « profil existant » (aucun compte créé, aucune connexion, session inchangée) ; **profils détectés** d'abord (ceux qui ne sont pas déjà affichés, donc sans doublon) ; **« Choisir un dossier… »** (macOS : `NSOpenPanel` natif, un seul dossier, fichiers cachés visibles, création de dossier interdite, retour du chemin seulement — aucune lecture de contenu, aucun nouveau droit Tauri) ; **chemin absolu** en option avancée (ouverte d'office là où aucun sélecteur n'existe, Windows/Linux).
- Backend `claude_accounts::follow` (commandes `follow_claude_profile` et `add_claude_profile`) : déduplication par identité canonique (alias/symlink, dossier détecté choisi via le sélecteur), erreurs actionnables (« déjà affiché », « introuvable : vérifiez le chemin ou utilisez Choisir un dossier… », « indisponible »). Chemin brut conservé pour le lancement (correctif 72696ec intact).
- Ajout volontaire : passage en **Multi**, ce qui était affiché (le principal quand on passe de 1 à 2) **et** le nouveau compte visibles ; en quittant Unique, les autres profils restent masqués explicitement. Sans aucun autre profil utilisable (pas de `~/.claude`), le compte ajouté devient le compte Unique affiché. La découverte au démarrage ne passe jamais par ce chemin et ne change pas le mode.
- Multi : liste directement visible (nom modifiable, abonnement, état Connecté / Non connecté / Indisponible / Erreur, interrupteur Afficher), puis « Ajouter un compte Claude ». Unique : sélection du compte affiché, gestion repliée (« Gérer les comptes Claude (n) ») contenant l'ajout.
- Clavier : CTA atteignable par Tab, Entrée ouvre le panneau, Échap/Annuler le ferme et rend le focus au bouton ; `aria-busy`, actions désactivées pendant une opération ; erreurs `role="alert"` dans le panneau.

### Préférence de présentation

- `prefs.json` : clé `claudeLayout` (`compact` | `classic`). Absente → `compact` (comportement antérieur : multi jumelé, mono historique) ; valeur inconnue normalisée sans perdre les autres préférences (`parse` et `set_preferences`).
- Un seul compte visible : rendu historique exact quel que soit le choix (tests d'égalité Rust, captures natives identiques au pixel, HTML React identique) ; Affichage conserve la valeur et l'explique.
- Appliquée à `panel_model()` (menu AppKit et fenêtre compacte/Dock via `compact: !classic`) et au Dashboard React (Classique : sections pleine largeur empilées). Changer le style n'appelle que `set_preferences` : `collection_changed` reste faux, ni mode, ni visibilité, ni cache/session ne changent, aucune collecte.

### Vérifications

- RED observés : `claude_layout_migrates_normalises_and_persists_independently` (valeur inconnue non normalisée) avant correction ; 12 tests React nouveaux en échec sur les sources de 72696ec. Les tests `follow` ont été écrits avec la fonction (RED = absence de l'API).
- `cargo test --offline` : 118 réussis, 4 ignorés ; Node : 68 réussis ; `npm run build` OK ; `cargo fmt --check` OK.
- `--quota-ui-smoke` natif : nouveaux scénarios `*-classic` (clair/sombre) ; épaisseurs Classique multi = 8 × 5 pt, Compacte inchangée (5/5/3/3), mono Classique identique au pixel au mono Compacte.
- Fixtures React (headless, IPC simulé) : 36 captures Réglages/Dashboard 480/360/640/420 px, clair/sombre, aucun débordement horizontal ; parcours clavier vérifié ; changement de style sans aucun appel de collecte.

### Limites

- Sélecteur natif uniquement sur macOS ; Windows/Linux : saisie de chemin, rendu React non exécuté nativement sur ces systèmes.
- Fixtures ≠ test de l'application installée ; le sélecteur `NSOpenPanel` n'a pas été ouvert pendant les tests (interaction utilisateur requise).
- Pas de suppression de profil (non demandé). La surbrillance bleue au survol du menu est un bug distinct, non traité ici.
