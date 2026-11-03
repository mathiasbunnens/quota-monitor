import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import type { CodexStatus } from "../quota/types";

/** Refresh frequency: one choice, applied immediately. */
export default function RefreshSettings({ status, onChange }: {
  status: CodexStatus | null; onChange: () => void;
}) {
  const saved = String(status?.refreshSettings.customSeconds ?? "auto");
  const [value, setValue] = useState(saved);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async (next: string) => {
    setValue(next); setSaving(true); setError(null);
    try {
      await invoke("set_refresh_settings", { settings: { customSeconds: next === "auto" ? null : Number(next) } });
      onChange();
    } catch (cause) { setError(String(cause)); setValue(saved); }
    finally { setSaving(false); }
  };
  return <div className="settings-form">
    <label htmlFor="refresh-frequency" className="prefs-option__title">Fréquence</label>
    <select id="refresh-frequency" value={value} disabled={saving} onChange={(event) => void save(event.target.value)}>
      <option value="auto">Automatique (selon l’activité)</option>
      <option value="30">Toutes les 30 secondes</option><option value="60">Toutes les minutes</option>
      <option value="120">Toutes les 2 minutes</option><option value="300">Toutes les 5 minutes</option>
      <option value="600">Toutes les 10 minutes</option>
    </select>
    <p>Actuellement toutes les {status?.refreshSeconds ?? 120} s, pour Codex et Claude.</p>
    {error && <p role="alert">{error}</p>}
  </div>;
}
