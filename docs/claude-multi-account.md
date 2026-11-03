# Claude multi-comptes — portage sur Quota Monitor 3.1.0

Branche `feature/claude-multi-account-3.1`, base `1bcc2e5` (= `v3.1.0` = `origin/main`). Portage de `2d298ad` (multi-comptes) et `6a6a395` (décompte visible), repris hunk par hunk dans l'architecture 3.1.0 (`panel_model()`, fenêtre compacte, Réglages à onglets, Fable). Aucun fichier d'icône, de mascotte, d'updater, de `tauri.conf.json`, `Cargo.*` ou `package*.json` modifié ; version 3.1.0 conservée.

## Comportement

- Consultation uniquement : aucun login, changement de session ou accès direct aux identifiants OAuth/trousseau.
- Profils : `CLAUDE_CONFIG_DIR` absolu, sinon `~/.claude` ; détection `~/.claude-*` ; ajout manuel absolu ; déduplication canonique/symlinks. Un dossier indisponible reste représenté sans bloquer les autres.
- Collecte sérialisée `initialize` / `get_usage` ; `CLAUDE_CONFIG_DIR` défini uniquement sur le processus enfant. Cache, erreur, rafraîchissement et ticket par identité canonique ; anti-chevauchement et résultats périmés rejetés. Les changements d'affichage ne relancent pas la collecte.
- `prefs.json` 3.1.0 (mode d'affichage, Fable, ntfy, actualisation) se charge sans perte ; `claudeAccounts` absent → Unique, principal (test `v310_preferences_file_loads_without_loss_and_defaults_to_single_primary`).
- Réglages (onglet Fournisseurs) : Unique|Multi dès deux profils, renommage, visibilité (affichage seul), ajout manuel.
- Un seul compte visible : carte « Claude » sans nom de compte ni note ajoutée ; sa présentation suit le choix Compacte/Classique (voir plus bas). Classique = carte 3.1.0 exacte.
- Multi : une carte « Claude · {profil} » par compte visible, dans `panel_model()` donc **identique dans le menu natif et la fenêtre compacte**. Bloc apparié de 40 pt : 5 h à l'épaisseur Unique mesurée sur 3.1.0 (menu AppKit 5 pt rayon 2,5 ; fenêtre compacte 5 px ; Dashboard React 7 px rayon 8), 7 j à 3 pt/px, écart 2. Entête « décompte · valeur » (3h30, 42 min, 3j 8h) calculé depuis `resetAt` et l'horloge locale, redessiné à la minute sans collecte. Aucune agrégation, couleurs et thèmes existants.
- Fable : préférence 3.1.0 appliquée à chaque compte (vue d'affichage) ; libellé « Limite Fable » ; si la fenêtre hebdomadaire est coalescée dans Fable, la barre 7 j est préfixée « Fable · ». Rien n'est masqué pour faire tenir la carte (Fable et usage supplémentaire restent visibles).
- Notifications : **politique 3.1.0 conservée** (alerte au retour à 100 % restant, une fois par cycle). Appliquée à tous les profils (visibles ou non), clé stable `Claude:{id profil}:{métrique}`, migration unique des anciennes clés du principal (Fable inclus), sans doublon ; nom du compte dans le message dès deux profils. Alertes Fable muettes si Fable désactivé.

## Fichiers

| Périmètre | Fichiers |
| --- | --- |
| Profils, collecte, préférences, notifications | `src-tauri/src/{claude_accounts.rs,claude_cli.rs,claude.rs,prefs.rs,ntfy.rs}` |
| `panel_model`, menu AppKit, compact, tray, fixtures natives | `src-tauri/src/lib.rs`, `src-tauri/src/menu_fixture.rs` |
| Cartes et contrats React | `src/features/quota/{types.ts,providers.ts,ProviderCard.tsx}` |
| Fenêtre compacte | `src/features/compact/{types.ts,CompactPanel.tsx,compact.scss}` |
| Réglages | `src/features/settings/{Settings.tsx,ClaudeAccountsSettings.tsx,claude-accounts.scss,settings.scss}` |
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

## Réglages simplifiés et présentation Compacte | Classique

Contrat en vigueur depuis t_ad995d23 (remplace celui de 644cca6, où un seul compte gardait toujours la carte historique).

Deux réglages indépendants :

- **Fournisseurs → Claude** choisit *quels* comptes sont affichés (`Unique | Multi`, dès deux profils).
- **Affichage → Présentation Claude** choisit *comment* : `Compacte` (barres 5 h / 7 j jumelées, décomptes) ou `Classique` (lignes séparées historiques : 5 h, hebdomadaire, Fable, pourcentages, réinitialisations). **S'applique à tous les cas** : Unique, Multi avec un seul visible, plusieurs visibles. Jamais d'agrégation.

### Préférence `claudeLayout`

- Règle partagée : Rust `prefs::parse` / `claude_account_panels` (menu AppKit et fenêtre compacte/Dock via `panel_model()`) et React `claudeLayoutOf` / `claudePlan` (Dashboard). Plus aucune condition sur le nombre de comptes.
- Choix enregistré explicite = prioritaire sur le mode et le nombre de comptes ; valeur inconnue → `compact`, autres préférences conservées.
- Clé absente (ancien fichier) : on garde l'aspect que ce fichier avait déjà — `compact` pour un ancien Multi (barres jumelées), sinon `classic` (carte 3.1.0). Pas de fichier = `classic`. La valeur migrée est réécrite comme clé propre.
- Une seule carte : nom « Claude », pas de note d'usage supplémentaire ; plusieurs : « Claude · {profil} ». Zéro compte visible : aucune carte inventée, message d'état vide ; le choix reste modifiable et enregistré.
- Changer le style n'appelle que `set_preferences` : aucune collecte, ni mode, ni visibilité, ni cache/session modifiés.

### Affichage

- Trois groupes, pictogrammes + libellés courts, aucun paragraphe : Mode d'affichage (mini barre des menus avec icône / petite fenêtre + Dock), Afficher dans la barre des menus (glyphes Codex / Claude / les deux, monochromes `currentColor`, masqué en mode Application), Présentation Claude (miniatures : double barre serrée / lignes espacées, sans chiffres).
- Mêmes `prefs-segmented` (variante `prefs-choices`), couleurs et thèmes ; vrais boutons radio : Tab, flèches, focus visible, nom accessible explicite. Les radios restent actives pendant l'enregistrement (garde `mutation`), le focus n'est donc pas perdu.

### Fournisseurs → Claude

- Une seule liste (profils détectés inclus) : nom, crayon de renommage en ligne (Entrée enregistre, Échap annule, focus rendu au crayon), Principal, abonnement, pastille d'état ; interrupteur Afficher en Multi, sélection ronde en Unique. Détail d'erreur via le bouton ⓘ (et infobulle), pas de paragraphe permanent.
- **« Ajouter un compte »** toujours présent (0, 1 ou n profils) : ouvre directement le sélecteur de dossier macOS (`NSOpenPanel`) ; sans sélecteur (Windows/Linux), ouvre la zone secondaire avec le champ de chemin.
- Un seul accès secondaire (chevron « Chemins et options avancées ») : chemins des profils, ajout par chemin absolu, Actualiser, Retirer l'ancien relais. Plus de panneau d'ajout, de disclosure imbriquée ni de « Gérer le compte ».
- Backend inchangé : `add_claude_profile` (déduplication canonique, erreurs concises « déjà affiché », « introuvable », « indisponible », chemin brut conservé), passage en Multi après un ajout volontaire. Codex inchangé.

## Correctif : fausse barre bleue en haut du menu

Symptôme : bande bleue au-dessus du panneau de quotas en restant sur « Réglages… » ou en passant à « Vérifier les mises à jour… ».

- Cause reproduite : pendant le suivi du menu, chaque actualisation (ouverture, résultat de collecte, tic de décompte à la minute) remplaçait la vue du panneau par `setView`. AppKit reconstruit alors la ligne et replace le fond de sélection de l'action surlignée (`NSRootMenuWindowBackgroundView`) en haut du panneau ; la ligne « Réglages… » perd aussi son texte.
- Correctif (`refresh_open_panel`) : on garde la vue du panneau et on y déplace le nouveau contenu. Si la hauteur change pendant qu'une action est surlignée (cas qui décalait aussi la sélection), ce redimensionnement attend : prochaine actualisation sans surlignage, fermeture du menu (`menuDidClose:` redessine) ou réouverture. Les valeurs à hauteur égale (décomptes, pourcentages) restent mises à jour en direct ; seul un changement de structure (ex. Claude « Lecture… » → quotas) peut rester affiché à l'ancienne hauteur tant qu'une action reste surlignée.
- Fixture interactive `--menu-tracking-fixture <dir>` (`menu_fixture.rs`) : vrai menu de production ouvert en suivi AppKit, données synthétiques, flèches clavier envoyées au seul processus, actualisations même hauteur / autre hauteur / rafale, clair et sombre, variante `legacy` (`setView`) et `production`. Verdict calculé depuis la position du fond de sélection par rapport à la ligne surlignée.

## Vérifications t_ad995d23

- RED sur 644cca6 : `layout_choice_applies_to_a_single_visible_account` et `claude_layout_migrates_normalises_and_persists_independently` (Rust), 19 tests Node (présentation mono, migration, Réglages simplifiés). GREEN : `cargo test --offline` 118 réussis / 4 ignorés, Node 69 réussis, `npm run typecheck`, `npm run build`, `cargo fmt --check` OK.
- `--quota-ui-smoke` : 82 PASS ; mono Compacte = 5/5/3/3 pt (jumelé), mono Classique = 4 × 5 pt ; le rafraîchissement en direct garde la même vue (assertion d'identité).
- `--menu-tracking-fixture` : `legacy` → STRAY_BAND_OVER_PANEL sur toutes les actualisations avec action surlignée (clair et sombre) ; `production` → 0 bande, sélection toujours sur sa ligne, redimensionnements différés seulement sous surlignage.
- Fixtures React headless (IPC simulé, captures avant/après 480 px clair et 360 px sombre) : Affichage 58 → 24 mots, Claude Multi 71 → 32 ; aucun débordement ; clavier réel (CDP) : flèches sur Présentation et Mode, renommage Entrée/Échap, interrupteur Espace, sélection Unique, ⓘ, Ajouter → sélecteur → `add_claude_profile`, chemin relatif refusé, erreur backend affichée ; Dashboard mono Compacte jumelé / Classique historique.

### Limites

- Le survol à la souris n'a pas été reproduit par la fixture (les mouvements souris envoyés au processus ne surlignent pas) : la preuve utilise le surlignage clavier, même fond de sélection AppKit. Validation finale du symptôme exact par l'utilisateur sur l'app.
- Fixtures ≠ application installée ; `NSOpenPanel` non ouvert en test. Windows/Linux non exécutés nativement. Pas de suppression de profil (non demandé).

## Extension Codex — t_33379e53

- **Affichage → Présentation Codex** et **Présentation Claude** sont indépendantes, avec les mêmes miniatures Compacte/Classique. `codexLayout` absent → `classic` (ancien rendu conservé), choix explicite prioritaire, valeur inconnue → `compact`. Aucun changement de `claudeLayout`, des comptes ou de leur collecte. Préférences effectives inspectées sans modification : Claude Compacte, clé Codex absente.
- Codex Compacte s'applique dès un compte : menu AppKit, fenêtre compacte React via le même `panel_model`, Dashboard. Épaisseurs haut/bas : 5/3 pt (AppKit), 5/3 px (compact), 7/3 px (Dashboard, épaisseur historique propre à cette vue) ; écart 2. Décomptes locaux, pourcentages restants et plan conservés ; date exacte dans les infobulles/Détails. Classique reste historique.
- Le CLI 0.164.0 fournit de vraies clés `codex:{rateLimitId}:{primary|secondary}` et `windowMinutes`. On sélectionne les fenêtres 300/10080 minutes d'un **même bucket et modèle** (priorité au bucket `codex`, sinon premier éligible). Pas d'agrégation. Fenêtre absente = aucune barre ni valeur inventée ; semaine seule = barre fine. Autres buckets, durées et réserves restent des lignes complètes, avec leurs valeurs/resets. `pairSlot` transmet cette appartenance explicite au renderer compact ; aucun appariement arbitraire des deux premières lignes Codex.
- Auth/transport `codex.rs`, reset/reconnexion, refresh, notifications, tray/icônes, updater et correctif `refresh_open_panel` inchangés. Changer la présentation n'appelle que `set_preferences`.
- Vérifié : RED/GREEN Node et Rust (mono, migration, IDs réels, buckets croisés, semaine seule, extras, erreurs, épuisé), 73 tests Node réussis, 121 Rust réussis / 4 ignorés ; typecheck, build Vite, compilation native offline et fmt OK. Fixture AppKit : 116 marqueurs PASS, mesures 5/5/3/3 avec écart 2 ; 19 scènes navigateur headless, clavier flèches/persistance IPC, décompte local sans collecte, aucun débordement ni exception. Captures avant 716e3f0/après et clair/sombre examinées. JSON AppKit de la fixture utilisé directement par le renderer compact, sans simulation d'un compte réel.
- Faisabilité multi-Codex **documentée, non implémentée** : voir [codex-multi-account-feasibility.md](codex-multi-account-feasibility.md). Isolation par `CODEX_HOME` établie dans le source officiel tagué, y compris les deux backends keyring ; pas de test de deux comptes authentifiés. Aucun login/logout, credentials, lecture keychain ou appel modèle. Aucune application normale lancée ; fixtures isolées seulement.
- Preuves non commitées : `evidence/t_33379e53/`. Les builds locaux restent dans `dist-local/3.1.0-<sha>-arm64/`, jamais installés automatiquement ; signature ad hoc, sans notarisation. Quitter l'instance installée avant un essai volontaire (même identité/données).

### Complément demandé : titres Réglages

Les quatre onglets partagent désormais `prefs-group__title` (13 px, graisse 600, casse française normale, interligne 1,45) pour les titres de groupe et les légendes ; `prefs-option__title` / les libellés utilisent 12 px / 500. Le titre de page reste celui de la fenêtre native « Réglages » (région React nommée, pas de titre visuel doublonné). Espacement de groupe commun de 10 px, mêmes tokens, icônes et contrôles. Anciennes règles locales `#ntfy-heading`, `h2` fournisseur/connexion et `h3` Fable supprimées. « Actualisation des quotas » distingue le groupe de l'option « Fréquence ».

RED navigateur : titre Actualisation absent et titre Notifications à 700 au lieu de 600. GREEN : 16 captures comparables (4 onglets × clair/sombre × avant/après 029dff0), titres tous à 13/600 et au même alignement gauche 35 px sur viewport 480 px, sans clipping ni débordement ; captures examinées. Les 73 tests Node, typecheck et build Vite passent après ce complément. Aucun transport, réglage d'actualisation ou envoi de notification modifié.
