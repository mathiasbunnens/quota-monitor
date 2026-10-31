import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import ResetCredits from "../../features/quota/ResetCredits";
import ProviderCard from "../../features/quota/ProviderCard";
import { useCallback, useEffect, useState } from "react";
import CodexConnection from "../../features/setup/CodexConnection";
import { claudeRows, claudeStatusText, codexRows } from "../../features/quota/providers";
import type { ClaudeAccount, ClaudeSnapshot, CodexStatus, Preferences, QuotaSnapshot } from "../../features/quota/types";
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
  const [accounts, setAccounts] = useState<ClaudeAccount[]>([]);
  const [now, setNow] = useState(Date.now);
  const reload = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    // Presentation only: never reload snapshots or ask a provider to collect.
    const timer = window.setInterval(() => setNow(Date.now()), 60000);
    return () => window.clearInterval(timer);
  }, []);

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
      const [p, s, c, k, a] = await Promise.allSettled([
        invoke<Preferences>("get_preferences"),
        invoke<QuotaSnapshot[]>("get_quota_snapshots"),
        invoke<CodexStatus>("get_codex_status"),
        invoke<ClaudeSnapshot>("get_claude_snapshot"),
        invoke<ClaudeAccount[]>("get_claude_accounts"),
      ]);
      if (disposed) return;
      if (p.status === "fulfilled") setPrefs(p.value);
      if (s.status === "fulfilled" && c.status === "fulfilled") {
        setSnapshots(s.value); setStatus(c.value); setCodexError(null);
      } else setCodexError("Impossible de lire l’état de Codex.");
      if (k.status === "fulfilled") setClaude(k.value);
      if (a.status === "fulfilled") setAccounts(a.value);
      setClaudeError(a.status === "rejected" || k.status === "rejected" ? "Impossible de lire l’état de Claude." : null);
      timer = window.setTimeout(() => void poll(), 2000);
    };
    void poll();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [revision]);

  const codexOn = prefs?.codexEnabled ?? true;
  const claudeOn = !!prefs?.claudeEnabled;
  const multiMode = prefs?.claudeAccounts?.mode === "multi";
  const selected = accounts.find((account) => account.id === prefs?.claudeAccounts?.selectedId)
    ?? accounts.find((account) => account.primary) ?? accounts[0];
  const shownAccounts = multiMode ? accounts.filter((account) => account.visible) : selected ? [selected] : [];
  const multi = shownAccounts.length > 1;
  const singleSnapshot = multiMode ? shownAccounts[0]?.snapshot : selected?.snapshot ?? claude;
  const fableEnabled = prefs?.fableEnabled ?? true;

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
    <div className={"provider-grid" + (both || (claudeOn && multi) ? " provider-grid--two" : "") + (claudeOn && multi ? " provider-grid--accounts" : "")}>
      {codexOn && <ProviderCard name="Codex" badge={status?.planType} rows={codexRows(snapshots)} compact={both}
        status={codexStatus} error={codexFailure}
        empty={status?.phase === "ready" ? "Aucun quota à afficher. La réserve peut ne pas être communiquée par Codex." : "Quotas indisponibles."} />}
      {claudeOn && multi && shownAccounts.map((account) => <ProviderCard key={account.id} name={"Claude · " + account.name}
        badge={account.snapshot.planType} rows={claudeRows(account.snapshot, fableEnabled)} compact accountCompact now={now}
        status={!account.available ? "Dossier indisponible." : account.refreshing ? "Actualisation…" : claudeStatusText(account.snapshot)}
        error={claudeError} empty="Aucune limite communiquée par Claude Code.">
        {account.snapshot.extraUsage?.enabled && <p className="provider-card__note">{account.snapshot.extraUsage.limitReached ? "Extra · plafond atteint" : "Usage supplémentaire activé"}</p>}
      </ProviderCard>)}
      {claudeOn && multiMode && !shownAccounts.length && <p className="provider-card__note" role="status">Aucun compte Claude visible. Choisissez les comptes dans les Réglages.</p>}
      {/* One visible account: the v3.1.0 Claude card, unchanged. */}
      {claudeOn && !multi && (!multiMode || shownAccounts.length > 0) && <ProviderCard name="Claude" rows={singleSnapshot ? claudeRows(singleSnapshot, fableEnabled) : []} compact={both}
        status={singleSnapshot ? claudeStatusText(singleSnapshot) : "Lecture…"} error={claudeError} empty="Aucune limite communiquée par Claude Code." />}
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
