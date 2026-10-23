import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import ResetCredits from "../../features/quota/ResetCredits";
import ProviderCard from "../../features/quota/ProviderCard";
import { useCallback, useEffect, useState } from "react";
import CodexConnection from "../../features/setup/CodexConnection";
import { claudeRows, claudeStatusText, codexRows } from "../../features/quota/providers";
import type { ClaudeSnapshot, CodexStatus, Preferences, QuotaSnapshot } from "../../features/quota/types";
import "./dashboard.scss";

export default function Dashboard() {
  const [snapshots, setSnapshots] = useState<QuotaSnapshot[]>([]);
  const [status, setStatus] = useState<CodexStatus | null>(null);
  const [claude, setClaude] = useState<ClaudeSnapshot | null>(null);
  const [prefs, setPrefs] = useState<Preferences | null>(null);
  const [showSetup, setShowSetup] = useState(false);
  // Errors are tracked per provider so one failing source never blanks the other.
  const [codexError, setCodexError] = useState<string | null>(null);
  const [claudeError, setClaudeError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [revision, setRevision] = useState(0);
  const reload = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    const quotas = listen("quota-updated", reload).catch(() => () => {});
    return () => { void quotas.then((unlisten) => unlisten()); };
  }, [reload]);

  useEffect(() => {
    let disposed = false;
    void invoke<string>("get_platform").then((platform) => {
      if (!disposed) document.documentElement.dataset.platform = platform;
    }).catch(() => undefined);
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    // `disposed` flips on every reload: an answer that was in flight when a
    // provider got disabled can never bring its section back.
    let disposed = false;
    let timer: number;
    const poll = async () => {
      const [p, s, c, k] = await Promise.allSettled([
        invoke<Preferences>("get_preferences"),
        invoke<QuotaSnapshot[]>("get_quota_snapshots"),
        invoke<CodexStatus>("get_codex_status"),
        invoke<ClaudeSnapshot>("get_claude_snapshot"),
      ]);
      if (disposed) return;
      if (p.status === "fulfilled") setPrefs(p.value);
      if (s.status === "fulfilled" && c.status === "fulfilled") {
        setSnapshots(s.value); setStatus(c.value); setCodexError(null);
      } else setCodexError("Impossible de lire l’état de Codex.");
      if (k.status === "fulfilled") { setClaude(k.value); setClaudeError(null); }
      else setClaudeError("Impossible de lire l’état de Claude.");
      timer = window.setTimeout(() => void poll(), 2000);
    };
    void poll();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [revision]);

  const codexOn = prefs?.codexEnabled ?? true;
  const claudeOn = !!prefs?.claudeEnabled && !!claude && claude.enabled;

  const refresh = async () => {
    setRefreshing(true); setError(null);
    const jobs: Promise<unknown>[] = [];
    if (codexOn) jobs.push(invoke("request_quota_refresh"));
    if (claudeOn) jobs.push(invoke("request_claude_refresh"));
    const results = await Promise.allSettled(jobs);
    const failed = results.find((r) => r.status === "rejected") as PromiseRejectedResult | undefined;
    if (failed) setError(String(failed.reason));
    reload(); setRefreshing(false);
  };
  const openDetails = async () => {
    setError(null);
    try { await invoke("open_usage_details"); }
    catch (cause) { setError(String(cause)); }
  };
  const openSettings = async () => {
    try { await invoke("open_settings"); }
    catch (cause) { setError(String(cause)); }
  };

  const isConnected = status?.phase === "ready";
  const needsSetup = snapshots.length === 0 && status?.phase !== "ready";
  const both = codexOn && claudeOn;
  const none = !codexOn && !claudeOn && prefs !== null;
  const codexStatus = status && status.phase !== "ready" && status.phase !== "error" ? status.message : null;
  const codexFailure = codexError ?? (status?.phase === "error" ? status.message : null);

  return <main className="dashboard">
    <header className="dashboard__header">
      <div><h1>Quota Monitor</h1></div>
      <div className="dashboard__actions">
        <button onClick={() => void refresh()} disabled={refreshing || none}>{refreshing ? "Actualisation…" : "Actualiser"}</button>
        <button onClick={() => void openSettings()}>Réglages…</button>
        {codexOn && !isConnected && <button onClick={() => setShowSetup((value) => !value)} aria-expanded={showSetup}>{showSetup ? "Masquer la connexion" : "Connexion à Codex"}</button>}
      </div>
    </header>
    {error && <p role="alert">{error}</p>}
    {none && <p className="dashboard__source" role="status">Aucun fournisseur actif. Active Codex ou Claude dans les Réglages.</p>}
    <div className={"provider-grid" + (both ? " provider-grid--two" : "")}>
      {codexOn && <ProviderCard name="Codex" badge={status?.planType} rows={codexRows(snapshots)} compact={both}
        status={codexStatus} error={codexFailure}
        empty={status?.phase === "ready" ? "Aucun quota à afficher. La réserve peut ne pas être communiquée par Codex." : "Quotas indisponibles."} />}
      {claudeOn && claude && <ProviderCard name="Claude" rows={claudeRows(claude, prefs?.fableEnabled ?? true)} compact={both}
        status={claudeStatusText(claude)} error={claudeError} empty="Aucune limite communiquée par Claude Code." />}
    </div>
    {codexOn && <ResetCredits status={status} onChange={reload} />}
    <footer className="dashboard__footer">
      <span>{codexOn && snapshots[0]?.checkedAt ? "Codex vérifié : " + new Date(snapshots[0].checkedAt).toLocaleTimeString("fr-FR") : ""}</span>
      {codexOn && <button onClick={() => void openDetails()}>Voir les détails sur Codex ↗</button>}
    </footer>
    {codexOn && <p className="dashboard__strategy">Actualisation Codex {status?.refreshSettings.customSeconds ? "personnalisée" : "dynamique"} · toutes les {status?.refreshSeconds || 120} s{claudeOn ? " · Claude : mis à jour par Claude Code" : ""}</p>}
    {codexOn && !isConnected && (showSetup || needsSetup) && <CodexConnection status={status} onChange={reload} />}
  </main>;
}
