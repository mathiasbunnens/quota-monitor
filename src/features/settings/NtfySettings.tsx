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
        setMessage("Test accepté par le serveur. Vérifiez votre téléphone.");
      } else if (await onSave(draft)) {
        setMessage("Réglages enregistrés.");
      }
    } catch (cause) { setFailed(true); setMessage(String(cause)); }
    finally { setBusy(false); }
  };
  return <section className="prefs-group" aria-labelledby="ntfy-heading">
    <h2 id="ntfy-heading">Alertes téléphone · ntfy</h2>
    <p>Codex et Claude : alerte à 0 % restant. L’application doit rester ouverte et connectée.</p>
    <fieldset disabled={busy} className="ntfy-fields">
      <label><input type="checkbox" checked={draft.enabled}
        onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} /> Alertes automatiques</label>
      <label>Serveur ntfy<input type="url" value={draft.serverUrl} autoComplete="off" spellCheck={false}
        onChange={(e) => setDraft({ ...draft, serverUrl: e.target.value })} /></label>
      <label>Topic<input type="text" value={draft.topic} autoComplete="off" spellCheck={false}
        aria-describedby="ntfy-privacy" placeholder="Nom long et imprévisible" maxLength={128}
        onChange={(e) => setDraft({ ...draft, topic: e.target.value })} /></label>
      <div className="prefs-actions">
        <button type="button" className="primary" onClick={() => void act(false)}>Enregistrer</button>
        <button type="button" disabled={!draft.topic || !draft.serverUrl} onClick={() => void act(true)}>
          {busy ? "En cours…" : "Envoyer un test"}
        </button>
      </div>
    </fieldset>
    {message && <p className={failed ? "prefs-alert" : "prefs-state"} role={failed ? "alert" : "status"}>{message}</p>}
    <p id="ntfy-privacy" className="prefs-footnote">Topic public : son nom permet de lire et publier. Gardez-le secret ; préférez HTTPS.</p>
    <details className="prefs-disclosure">
      <summary>Configurer ntfy et comprendre les alertes</summary>
      <p>Sur le téléphone, installez ntfy, abonnez-vous à ce serveur et ce topic, puis autorisez les notifications
        dans ntfy et les réglages du téléphone. Aucune authentification n’est envoyée.</p>
      <p>Le test utilise les champs sans les enregistrer, même si les alertes sont désactivées.</p>
      <p>Une limite déjà épuisée à la première lecture ne déclenche pas d’alerte. Aucun rappel tant qu’elle reste
        épuisée, même après redémarrage. Les limites atteintes ensemble sont regroupées.</p>
      <p>Nouvelle alerte après réinitialisation ou retour sous 100 % consommé. Après une erreur réseau,
        aucun nouvel essai automatique avant ce réarmement.</p>
    </details>
  </section>;
}
