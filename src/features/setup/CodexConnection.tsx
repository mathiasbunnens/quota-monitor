import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import type { CodexStatus } from "../quota/types";
import "./setup.scss";

/** `embedded`: inside the detail of the primary Codex account once connected —
 *  only what the primary needs (login, executable location), no intro or status text. */
type Props = { status: CodexStatus | null; onChange: () => void; embedded?: boolean };
export default function CodexConnection({ status, onChange, embedded = false }: Props) {
  const [error, setError] = useState<string | null>(null);
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const run = async (command: string, args?: Record<string, unknown>) => {
    setError(null); setBusy(true);
    try { await invoke(command, args); onChange(); }
    catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  return <section className={"connection-setup codex-connection" + (embedded ? " codex-connection--embedded" : "")}
    aria-labelledby={embedded ? undefined : "codex-title"} aria-label={embedded ? "Connexion Codex" : undefined}>
    {!embedded && <>
      <h2 id="codex-title" className="prefs-option__title">Connexion directe à Codex</h2>
      <p>Les quotas sont lus automatiquement depuis Codex. Aucune extension ni page ouverte n’est nécessaire après connexion.</p>
      <p role="status">{status?.message || "Recherche de Codex…"}</p>
    </>}
    {error && <p role="alert">{error}</p>}
    {status?.phase === "missing" && <button disabled={busy} onClick={() => void run("open_codex_install")}>Installer Codex — instructions officielles ↗</button>}
    {status?.executable && status.phase !== "logging_in" && <div className="codex-connection__actions">
      <button disabled={busy} onClick={() => void run("start_codex_login", { device: false })}>Se connecter avec ChatGPT</button>
      <button disabled={busy} onClick={() => void run("start_codex_login", { device: true })}>Connexion par code</button>
      {!embedded && <button disabled={busy} onClick={() => void run("request_quota_refresh")}>Réessayer / Actualiser</button>}
    </div>}
    {status?.phase === "logging_in" && <div>
      {status.userCode && <p>Code à saisir : <strong className="codex-connection__code">{status.userCode}</strong></p>}
      {status.authUrl && <>
        <button disabled={busy} onClick={() => void run("open_codex_login")}>Ouvrir la page de connexion ↗</button>
        {status.userCode && <p>Sur un autre appareil : <code>{status.authUrl}</code></p>}
      </>}
      <button disabled={busy} onClick={() => void run("cancel_codex_login")}>Annuler</button>
    </div>}
    <details>
      <summary>Emplacement de Codex</summary>
      <p>{status?.executable ? <>Détecté : <code>{status.executable}</code></> : "La détection est automatique après installation de Codex CLI."}</p>
      <form onSubmit={(event) => { event.preventDefault(); void run("set_codex_path", { path }); }}>
        <label htmlFor={embedded ? "codex-path-embedded" : "codex-path"}>Chemin complet vers codex (codex.exe sous Windows)</label>
        <input id={embedded ? "codex-path-embedded" : "codex-path"} value={path} onChange={(event) => setPath(event.target.value)} placeholder="Chemin de l’exécutable" />
        <button disabled={busy || !path.trim()} type="submit">Utiliser ce chemin</button>
        <button disabled={busy} type="button" onClick={() => { setPath(""); void run("set_codex_path", { path: "" }); }}>Détection automatique</button>
      </form>
    </details>

  </section>;
}
