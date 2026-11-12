# Tester Quota Monitor avec ai-profiles — fiche de test

Merci de tester cette version ! Elle affiche **tous vos profils ai-profiles** (Claude et Codex) dans Quota Monitor et doit les **garder connectés tout seuls**, même quand vous ne les utilisez pas. Pour cela, juste avant l'expiration de la connexion d'un profil, Quota Monitor lance l'outil officiel (`claude` ou `codex`) sur le dossier de ce profil. L'outil renouvelle alors la connexion lui-même. Quota Monitor ne crée **aucune** connexion à lui, n'envoie aucun message à l'IA et ne consomme pas votre quota.

Version de test : **3.2.1-test.aiprofiles.1**. Ce n'est pas une version officielle.

## 1. Avant de commencer

Il vous faut :

- un Mac avec **ai-profiles** installé ([bartekczyz/ai-profiles](https://github.com/bartekczyz/ai-profiles)) et au moins un profil déjà connecté ;
- **Claude Code 2.1.299 ou plus récent**. Pour vérifier, tapez dans le Terminal : `claude --version` ;
- **Codex 0.164.0 ou plus récent** (seulement si vous testez des profils Codex). Pour vérifier : `codex --version`.

Ce sont les versions sur lesquelles le fonctionnement a été vérifié. Les versions plus anciennes n'ont pas été testées : mettez-les à jour avant le test.

Si vous utilisez déjà Quota Monitor, **quittez-le** (icône dans la barre des menus › Quitter). La version de test remplace l'application habituelle et garde vos réglages.

## 2. Installer la version de test

1. Ouvrez le fichier `.dmg` reçu, puis glissez **Quota Monitor** dans **Applications** (acceptez de remplacer l'ancienne version).
2. Cette version de test n'est pas notarisée par Apple. Au premier lancement, macOS la bloque :
   - double-cliquez sur Quota Monitor dans Applications : un message dit qu'il ne peut pas vérifier l'app. Cliquez sur **Terminé** (ou **OK**) ;
   - ouvrez **Réglages Système › Confidentialité et sécurité**, descendez jusqu'à « Quota Monitor a été bloqué » et cliquez sur **Ouvrir quand même**. Confirmez avec votre mot de passe ;
   - sur un macOS plus ancien (avant 15), il suffit de faire **clic droit › Ouvrir** sur l'app, puis **Ouvrir**.
3. **Mises à jour désactivées** : cette version ne cherche pas de mise à jour. Si vous cliquez sur « Vérifier les mises à jour… », un message d'erreur s'affiche : c'est normal. N'installez pas d'autre version pendant le test.

### Pour avoir un journal (facultatif mais utile)

Ouvrez l'app de cette façon plutôt qu'avec un double-clic. Dans le Terminal, copiez cette ligne :

```sh
open -a "Quota Monitor" --stdout ~/Desktop/quota-monitor-test.log --stderr ~/Desktop/quota-monitor-test.log
```

Le journal s'écrit alors dans le fichier **`quota-monitor-test.log` sur votre Bureau**. Il ne contient que des messages d'erreur courts : aucun jeton, mot de passe ou identifiant secret n'y est écrit. Il peut rester vide, et c'est normal.

## 3. Étape 1 : un seul profil Claude

Commencez avec **un seul profil**, de préférence un profil **Claude secondaire**, c'est-à-dire pas celui que vous utilisez tous les jours.

**Important : n'utilisez pas ce profil dans un terminal pendant tout le test.** Le but est justement de voir si Quota Monitor le garde connecté pendant que vous ne vous en servez pas.

1. Lancez Quota Monitor.
2. Ouvrez **Réglages › Fournisseurs**. Vos profils ai-profiles s'affichent automatiquement, avec « · ai-profiles » à la fin de leur nom. Vous n'avez rien à ajouter.
3. Si besoin, choisissez **Multi** dans Claude pour voir plusieurs comptes en même temps.

### Ce qu'il faut regarder

- **Tous vos profils ai-profiles sont visibles** (Claude et Codex).
- Chaque profil affiche un état :
  - **Actif** : le profil est connecté et ses quotas s'affichent ;
  - **Renouvelé via claude** (ou **via codex**) : Quota Monitor vient de faire renouveler la connexion par l'outil officiel. C'est ce qu'on veut voir ;
  - **À reconnecter** : le renouvellement a été refusé (voir la section 4).
- **Après au moins 8 heures sans utiliser le profil** (une nuit, par exemple), le profil Claude doit afficher **« Renouvelé via claude »** et ses quotas doivent toujours s'afficher.
- Ouvrez ensuite **ai-profiles** : le même profil doit **toujours être connecté** là aussi. C'est le point le plus important du test.

Laissez Quota Monitor ouvert et le Mac allumé pendant ce temps (la mise en veille est acceptée).

## 4. Si un profil affiche « À reconnecter »

Reconnectez ce profil **dans ai-profiles**, comme d'habitude. **Il n'y a rien d'autre à faire** : Quota Monitor le voit tout seul et reprend le suivi. Ne cherchez pas de bouton de connexion dans Quota Monitor, il n'y en a pas.

Notez quand même l'heure et le profil concerné : c'est une information importante pour nous.

## 5. Étape 2 : Codex puis les autres profils

Si l'étape 1 s'est bien passée :

1. Faites la même chose avec **un profil Codex** : même règle, ne l'utilisez pas dans un terminal pendant le test. Vous devez voir **« Renouvelé via codex »**. Les connexions Codex durent plus longtemps : cela peut prendre plusieurs jours.
2. Ensuite, laissez tous vos profils en place et utilisez vos outils normalement. Vérifiez qu'aucun profil n'est déconnecté dans ai-profiles.

## 6. Ce qu'il faut nous renvoyer

1. **Captures d'écran** (Cmd + Maj + 4) :
   - **Réglages › Fournisseurs** ;
   - la **vue principale** (cliquez sur l'icône Quota Monitor dans la barre des menus).
   Faites-les au début du test, puis après 8 heures ou plus. Si un nom de profil ou une adresse e-mail est privé, vous pouvez le masquer.
2. Le **journal** `quota-monitor-test.log` de votre Bureau, si vous l'avez activé (section 2). Il ne contient aucun jeton.
3. Les **versions** : collez le résultat de ces commandes dans le Terminal :
   ```sh
   claude --version
   codex --version
   ```
   et la version d'**ai-profiles** (dans son menu « À propos »).
4. Un court résumé : quels profils sont passés à « Renouvelé », lesquels ont affiché « À reconnecter » (avec l'heure), et si un profil a été déconnecté dans ai-profiles.

**N'envoyez jamais** le contenu des dossiers ai-profiles, de fichiers `.credentials.json` ou `auth.json`, ni de capture du Trousseau.

## 7. Désinstaller ou revenir à la version normale

- **Revenir à la version officielle** : quittez Quota Monitor, téléchargez la dernière version sur [la page des versions](https://github.com/mathiasbunnens/quota-monitor/releases/latest), ouvrez le `.dmg` et glissez Quota Monitor dans Applications en remplaçant la version de test. Vos réglages sont conservés. Les noms donnés aux profils ai-profiles peuvent rester dans les réglages sans gêner.
- **Tout supprimer** : quittez Quota Monitor, mettez **Applications › Quota Monitor** à la Corbeille. Les réglages se trouvent dans `~/Library/Application Support/com.mathiasbunnens.chatgpt-quota-monitor` ; vous pouvez supprimer ce dossier pour un effacement complet.
- **Vos profils ai-profiles ne sont jamais modifiés par la désinstallation** : Quota Monitor ne stocke aucune connexion à lui.
