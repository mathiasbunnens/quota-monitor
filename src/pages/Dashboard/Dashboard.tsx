import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import ResetCredits from "../../features/quota/ResetCredits";
import ProviderCard from "../../features/quota/ProviderCard";
import ProviderGroup from "../../features/quota/ProviderGroup";
import AddAccountButton from "../../features/accounts/AddAccountButton";
import { accountState, claudeSnapshotState, codexAccountState, codexPhaseState, type AccountState } from "../../features/quota/accountState";
import { useCallback, useEffect, useState } from "react";
import ExternalIcon from "../../features/ui/ExternalIcon";
import CodexConnection from "../../features/setup/CodexConnection";
import { claudePlan, claudeRows, claudeStatusText, codexPlan, codexRows, type CodexCard } from "../../features/quota/providers";
import type { ClaudeAccount, ClaudeSnapshot, CodexAccount, CodexStatus, Preferences, QuotaSnapshot } from "../../features/quota/types";
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
  // Every Codex home (primary + followed); [] with an older backend → historical card.
  const [codexAccounts, setCodexAccounts] = useState<CodexAccount[]>([]);
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
      const [p, s, c, k, a, x] = await Promise.allSettled([
        invoke<Preferences>("get_preferences"),
        invoke<QuotaSnapshot[]>("get_quota_snapshots"),
        invoke<CodexStatus>("get_codex_status"),
        invoke<ClaudeSnapshot>("get_claude_snapshot"),
        invoke<ClaudeAccount[]>("get_claude_accounts"),
        invoke<CodexAccount[]>("get_codex_accounts"),
      ]);
      if (disposed) return;
      if (p.status === "fulfilled") setPrefs(p.value);
      if (s.status === "fulfilled" && c.status === "fulfilled") {
        setSnapshots(s.value); setStatus(c.value); setCodexError(null);
      } else setCodexError("Impossible de lire l’état de Codex.");
      if (k.status === "fulfilled") setClaude(k.value);
      if (a.status === "fulfilled") setAccounts(a.value);
      if (x.status === "fulfilled" && Array.isArray(x.value)) setCodexAccounts(x.value);
      setClaudeError(a.status === "rejected" || k.status === "rejected" ? "Impossible de lire l’état de Claude." : null);
      timer = window.setTimeout(() => void poll(), 2000);
    };
    void poll();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [revision]);

  const codexOn = prefs?.codexEnabled ?? true;
  const claudeOn = !!prefs?.claudeEnabled;
  // Shared with the native menu/compact window: the saved Compacte/Classique
  // choice applies to one or several cards; only names/notes follow the count.
  const plan = claudePlan(prefs, accounts, claude);
  const codex = codexPlan(prefs, codexAccounts, snapshots);
  const named = plan.named || codex.named;
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
  // Provider title arrow: the provider's detailed usage page.
  const openUsage = async (command: "open_usage_details" | "open_claude_usage") => {
    setError(null);
    try { await invoke(command); }
    catch (cause) { setError(String(cause)); }
  };
  const codexUsage = { label: "Codex : usage détaillé sur chatgpt.com", onOpen: () => void openUsage("open_usage_details"), icon: <ExternalIcon /> };
  const claudeUsage = { label: "Claude : usage détaillé sur claude.ai", onOpen: () => void openUsage("open_claude_usage"), icon: <ExternalIcon /> };
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
    {/* One group per provider: header « Codex ↗ » once, its account cards, its +.
        Side by side only when each provider shows a single card; otherwise stacked full width. */}
    <div className={"provider-grid" + (both && !named ? " provider-grid--two" : "")}>
      {codexOn && <ProviderGroup id="codex-group" title="Codex" usage={codexUsage} columns={codex.paired && codex.cards.length > 1}
        footer={<AddAccountButton provider="Codex" onChange={reload} />}>
        {codex.cards.map((card) => <CodexAccountCard key={card.key} card={card} named={codex.named} paired={codex.paired}
          compact={codex.paired || (!named && both)} now={now} status={status} codexStatus={codexStatus} codexFailure={codexFailure} onChange={reload} />)}
        {codex.emptyMulti && <p className="provider-card__note" role="status">Aucun compte Codex visible. Choisissez les comptes dans les Réglages.</p>}
      </ProviderGroup>}
      {claudeOn && <ProviderGroup id="claude-group" title="Claude" usage={claudeUsage} columns={plan.paired && plan.cards.length > 1}
        footer={<AddAccountButton provider="Claude" onChange={reload} />}>
        {plan.cards.map(({ key, snapshot, account }) => {
          const state: AccountState = account ? accountState(account) : snapshot ? claudeSnapshotState(snapshot) : ["Lecture…", "warning", ""];
          return <ProviderCard key={key} provider="Claude" name={account?.name ?? "Principal"} state={detailState(state)}
            detail={account ? { path: account.path, label: "Détail Claude " + account.name } : undefined}
            badge={snapshot?.planType} rows={snapshot ? claudeRows(snapshot, fableEnabled) : []}
            compact={plan.paired || (!plan.named && both)} accountCompact={plan.paired} now={now}
            status={plan.named && account ? (!account.available ? "Dossier indisponible." : account.refreshing ? "Actualisation…" : claudeStatusText(account.snapshot))
              : snapshot ? claudeStatusText(snapshot) : "Lecture…"}
            error={claudeError} empty="Aucune limite communiquée par Claude Code.">
            {plan.named && snapshot?.extraUsage?.enabled && <p className="provider-card__note">{snapshot.extraUsage.limitReached ? "Extra · plafond atteint" : "Usage supplémentaire activé"}</p>}
          </ProviderCard>;
        })}
        {plan.emptyMulti && <p className="provider-card__note" role="status">Aucun compte Claude visible. Choisissez les comptes dans les Réglages.</p>}
      </ProviderGroup>}
    </div>
    <footer className="dashboard__footer">
      <span>{codexOn && snapshots[0]?.checkedAt ? "Codex vérifié : " + new Date(snapshots[0].checkedAt).toLocaleTimeString("fr-FR") : ""}</span>
    </footer>
    {codexOn && <p className="dashboard__strategy">Actualisation Codex {status?.refreshSettings.customSeconds ? "personnalisée" : "dynamique"} · toutes les {status?.refreshSeconds || 120} s{claudeOn ? " · Claude : mis à jour par Claude Code" : ""}</p>}
    {codexOn && !isConnected && (showSetup || needsSetup) && <CodexConnection status={status} onChange={reload} />}
  </main>;
}

const detailState = ([label, tone, detail]: AccountState) => ({ label, tone, detail });

/** One Codex account card under the « Codex ↗ » header. The historical card
 *  (primary alone, or no account list) is fed by the primary collector; an
 *  additional home shows its own rows, state and duplicate note, never a sum. */
function CodexAccountCard({ card, named, paired, compact, now, status, codexStatus, codexFailure, onChange }: {
  card: CodexCard; named: boolean; paired: boolean; compact: boolean; now: number;
  status: CodexStatus | null; codexStatus: string | null; codexFailure: string | null; onChange: () => void;
}) {
  const account = card.account;
  // A reset belongs to one connected, followed, non-duplicate account: shown on its card.
  const credits = account && account.tracked && account.phase === "ready" && !account.duplicateOf
    ? <ResetCredits accountId={account.id} accountName={account.name} primary={account.primary}
      ready={account.available} count={account.availableResets} onChange={onChange} />
    : null;
  if (!account || (!named && account.primary)) {
    return <ProviderCard provider="Codex" name={account?.name ?? "Principal"} badge={status?.planType} rows={codexRows(card.rows)} compact={compact}
      accountCompact={paired} now={now} status={codexStatus} error={codexFailure} action={credits}
      state={detailState(account ? codexAccountState(account) : codexPhaseState(status?.phase, status?.message))}
      detail={account ? { path: account.path, label: "Détail Codex " + account.name } : undefined}
      empty={status?.phase === "ready" ? "Aucun quota à afficher. La réserve peut ne pas être communiquée par Codex." : "Quotas indisponibles."}></ProviderCard>;
  }
  const ready = account.phase === "ready";
  const note = !account.available ? "Dossier indisponible."
    : account.refreshing ? "Actualisation…"
    : !ready && account.phase !== "error" ? account.message : null;
  return <ProviderCard provider="Codex" name={account.name} badge={account.planType} rows={codexRows(card.rows)} compact={compact}
    accountCompact={paired} now={now} status={note} error={account.phase === "error" ? account.message : null} action={credits}
    state={detailState(codexAccountState(account))} detail={{ path: account.path, label: "Détail Codex " + account.name }}
    empty={ready ? "Aucun quota à afficher pour ce compte." : "Quotas indisponibles."}>
    {account.duplicateOf && <p className="provider-card__note">Même compte que « {account.duplicateOf} » : quotas non additionnés.</p>}
  </ProviderCard>;
}
