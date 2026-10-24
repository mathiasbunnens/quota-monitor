# Régression : panneau informatif du menu macOS sélectionnable

## Cause établie et portée

Le menu macOS est un `NSMenu` AppKit, pas une page HTML. Le panneau contenant
« Actualisé à… » est la vue personnalisée du premier `NSMenuItem`.
`NSMenuItem::new` laisse cet élément activé ; `setView` ne le désactive pas.
Comme le menu utilise `setAutoenablesItems(false)`, l'absence d'action ne le
rend pas automatiquement informatif. À l'inverse, l'élément de version était
déjà explicitement désactivé.

Le correctif appelle `panel.setEnabled(false)` à sa construction. Le remplacement
de sa vue pendant un rafraîchissement conserve cet état. Le cycle existant reste
inchangé : mise à jour en place pendant le suivi, reconstruction une fois fermé.
Aucun séparateur, raccourci, action, logo template ou réglage de notification ne
change. L'extraction du constructeur permet de tester le vrai menu sans démarrer
Tauri ni accéder aux préférences ou fournisseurs.

## Boucles de vérification

Depuis la racine du dépôt :

```sh
cargo run --manifest-path src-tauri/Cargo.toml --locked -- --quota-ui-smoke "$TMPDIR/quota-menu-smoke"
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo check --manifest-path src-tauri/Cargo.toml --locked
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
swift tests/native/menu-hover.swift
swift tests/native/menu-hover.swift --legacy
```

Le dernier appel doit sortir avec le code 1 : il reproduit la configuration
antérieure, et non un échec du correctif. L'appel sans `--legacy` doit sortir à 0.
Le test Swift nécessite une session graphique macOS et l'autorisation existante
de publier des événements ; sinon il sort à 77 sans demander cette autorisation.
Il déplace le pointeur sur son propre menu puis le restaure, sans cliquer sur
l'application installée. Ne pas manipuler la souris pendant cette vérification.
`--capture <répertoire scratch>` capture uniquement la fenêtre du menu enfant.
Les libellés et les actions Swift sont des fixtures explicites sans réseau.

Le smoke Rust s'exécute sur le thread principal et utilise le constructeur de
production. Avant la désactivation, il échoue sur « informational panel must not
be selectable ». Après :

- 12 constructions : quatre états (deux fournisseurs, erreur, attente, inactif)
  croisés avec trois états de connexion ;
- 48 remplacements de vue entre ces états, y compris les changements de hauteur ;
- éléments informatifs désactivés, deux séparateurs natifs, aucune action invisible,
  actions Réglages/Vérifier/Quitter et raccourcis conservés ;
- huit rendus du panneau clair/sombre et deux rendus d'indicateur empilé.

Le test Swift vérifie `menu.highlightedItem` par vrais événements de déplacement
sur le menu enfant, avec un contrôle positif avant les contrôles négatifs. Il
couvre attente/erreur/actualisé, clair/sombre, remplacement de vue de même taille
pendant le suivi, survol des deux actions et invocation de Réglages par Cmd+,.
La variante antérieure sélectionne le panneau sur 12 contrôles ; la variante
corrigée ne le sélectionne pas et conserve les vraies actions.

## Limites à garder explicites

Vérifié sur macOS 26.6.2 (25G83), AppKit 2685.7, avec objc2-app-kit 0.3.2,
objc2 0.6.4 et Tauri 2.11.6 du lockfile. Les tests Rust existants donnent
71 réussites et deux tests réels explicitement ignorés. Les avertissements de
code mort de Claude sont préexistants. Aucun fichier frontal ne change.

La sélection native indue est reproduite et corrigée ; les captures de la
fixture ne montrent pas le rectangle bleu exact décrit par l'utilisateur.
Elles ne constituent donc pas une preuve visuelle de résolution de son symptôme
sur l'application installée. Demander un court enregistrement du menu réel,
avec version de l'application/macOS et état des fournisseurs, pour confirmer
que le défaut visuel correspond bien à cette sélection.

Aucune installation, aucun lancement/arrêt de l'application utilisateur, aucun
accès à ses quotas/identifiants/préférences et aucune publication ne sont requis.
Le redimensionnement de vues pendant le suivi, déconseillé par Apple, n'est pas
modifié ici ; les transitions de hauteur sont vérifiées structurellement, pas
comme garantie de rendu interactif sur toutes les versions macOS. VoiceOver n'a
pas été exercé manuellement ; les labels et actions natifs sont conservés.

Références :

- https://developer.apple.com/documentation/appkit/nsmenuitem/isenabled
- https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/MenuList/Articles/ViewsInMenuItems.html
- API locale versionnée : objc2-app-kit-0.3.2/src/generated/NSMenuItem.rs
