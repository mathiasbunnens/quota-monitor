import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import type { NtfyConfig } from "../quota/types";

export default function NtfySettings({ config, onSave }: {
  config: NtfyConfig; onSave: (value: NtfyConfig) => Promise<boolean>;
}) {
  // A local draft is deliberately independent of the parent’s periodic polling.
  const [draft, setDraft] = useState(config);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [failed, setFailed] = useState(false);
  const act = async (test: boolean) => {
    setBusy(true); setMessage(""); setFailed(false);
    try {
      if (test) {
        await invoke("test_ntfy", { config: draft });
        setMessage("Notification test acceptée par le serveur. Vérifiez votre téléphone.");
      } else if (await onSave(draft)) {
        setMessage("Réglages enregistrés.");
      }
    } catch (cause) { setFailed(true); setMessage(String(cause)); }
    finally { setBusy(false); }
  };
  return <section className="prefs-group" aria-labelledby="ntfy-heading">
    <h2 id="ntfy-heading">Notifications téléphone · ntfy</h2>
    <p>Alerte uniquement au passage d’une limite Codex ou Claude à 100 % réellement consommé (0 % restant).
      Une limite déjà épuisée à sa première observation ne déclenche pas d’alerte.</p>
    <fieldset disabled={busy} className="ntfy-fields">
      <label><input type="checkbox" checked={draft.enabled}
        onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} /> Activer les alertes automatiques</label>
      <label>URL du serveur ntfy<input type="url" value={draft.serverUrl} autoComplete="off" spellCheck={false}
        onChange={(e) => setDraft({ ...draft, serverUrl: e.target.value })} /></label>
      <label>Topic<input type="text" value={draft.topic} autoComplete="off" spellCheck={false}
        placeholder="Un nom long, aléatoire et imprévisible" maxLength={128}
        onChange={(e) => setDraft({ ...draft, topic: e.target.value })} /></label>
      <div className="prefs-actions">
        <button type="button" className="primary" onClick={() => void act(false)}>Enregistrer</button>
        <button type="button" disabled={!draft.topic || !draft.serverUrl} onClick={() => void act(true)}>
          {busy ? "En cours…" : "Envoyer un test"}
        </button>
      </div>
    </fieldset>
    {message && <p className={failed ? "prefs-alert" : "prefs-state"} role={failed ? "alert" : "status"}>{message}</p>}
    <p>Installez ntfy sur votre téléphone puis abonnez-vous au même topic sur le même serveur.
      Autorisez les notifications dans l’application et les réglages du téléphone.</p>
    <p>Sans authentification, le topic est public : toute personne qui connaît son nom peut lire ou publier.
      Choisissez un nom imprévisible et ne le partagez pas. HTTPS est recommandé ; aucune authentification n’est envoyée.</p>
    <p className="prefs-footnote">Le test utilise les champs ci-dessus sans les enregistrer, même si les alertes sont désactivées.
      L’application doit rester ouverte et connectée. Aucun rappel tant que la limite reste épuisée, même après redémarrage.
      Les limites atteintes au même rafraîchissement sont regroupées. Réarmement après réinitialisation ou retour réel sous 100 % ;
      en cas d’erreur réseau, pas de nouvel essai automatique avant réarmement.</p>
  </section>;
}
