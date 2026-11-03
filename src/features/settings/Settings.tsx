import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import ExternalIcon from "../ui/ExternalIcon";
import CodexConnection from "../setup/CodexConnection";
import RefreshSettings from "../setup/RefreshSettings";
import NtfySettings from "./NtfySettings";
import ClaudeAccountsSettings, { CodexAccountsSettings } from "./ClaudeAccountsSettings";
import ResetCredits from "../quota/ResetCredits";
import type { ClaudeAccount, ClaudeLayout, ClaudeSnapshot, CodexAccount, CodexStatus, DisplayMode, IndicatorChoice, Preferences } from "../quota/types";
import { claudeRows, codexRows, layoutOf } from "../quota/providers";
import "../../pages/Dashboard/dashboard.scss";
import "./settings.scss";

type Tone = "good" | "warning" | "error" | "neutral";

const icons: Record<string, ReactNode> = {
  notifications: <><path d="M6 16V10a6 6 0 0 1 12 0v6l2 2H4l2-2M10 21h4" /></>,
  providers: <path d="M4 6.5h16M4 12h16M4 17.5h10" />,
  menubar: <><rect x="3" y="4.5" width="18" height="15" rx="2.5" /><path d="M3 9h18M14 6.8h4" /></>,
  refresh: <><path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3" /><path d="M19.5 4.5v4.2h-4.2" /></>,
};

const tabs = [
  { id: "providers", label: "Fournisseurs" },
  { id: "menubar", label: "Affichage" },
  { id: "refresh", label: "Actualisation" },
  { id: "notifications", label: "Notifications" },
] as const;
type TabId = (typeof tabs)[number]["id"];

const glyph = { fill: "none", stroke: "currentColor", strokeWidth: 1.5, strokeLinecap: "round", strokeLinejoin: "round" } as const;
const soft = { fill: "currentColor", opacity: 0.22 } as const;
const solid = { fill: "currentColor" } as const;
// Provider glyphs (monochrome, currentColor): Codex is a terminal agent,
// Claude a radiating mark. No new colour, no emoji.
const codexGlyph = <g {...glyph}><rect x="1.5" y="2.5" width="13" height="11" rx="2.5" /><path d="m4.5 6 2 2-2 2M8.5 10.5H11" /></g>;
const claudeGlyph = <g {...glyph}><path d="M8 2v12M2 8h12M3.8 3.8l8.4 8.4M12.2 3.8l-8.4 8.4" /></g>;

type Choice<T> = { value: T; label: string; icon: ReactNode; title?: string };

const indicatorOptions: Choice<IndicatorChoice>[] = [
  { value: "codex", label: "Codex", icon: <svg viewBox="0 0 16 16">{codexGlyph}</svg> },
  { value: "claude", label: "Claude", icon: <svg viewBox="0 0 16 16">{claudeGlyph}</svg> },
  { value: "both", label: "Les deux", icon: <svg viewBox="0 0 34 16">{codexGlyph}<g transform="translate(18 0)">{claudeGlyph}</g></svg> },
];

// Schematic pictograms (48 × 32): what the option puts on screen, no figures.
const displayModeOptions: Choice<DisplayMode>[] = [
  { value: "menubar", label: "Barre des menus", icon: <svg viewBox="0 0 48 32">
    <rect x="2" y="3" width="44" height="6" rx="1.5" {...soft} /><circle cx="38" cy="6" r="1.8" {...solid} />
    <g {...glyph}><rect x="27" y="11.5" width="17" height="17" rx="2.5" /><path d="M30.5 16.5h10M30.5 20.5h7M30.5 24.5h9" /></g>
  </svg> },
  { value: "app", label: "Application", icon: <svg viewBox="0 0 48 32">
    <g {...glyph}><rect x="9" y="2.5" width="30" height="19" rx="2.5" /><path d="M9 7.5h30M14 12.5h16M14 16.5h11" /></g>
    <rect x="8" y="25" width="32" height="5" rx="2.5" {...soft} /><rect x="21.5" y="26" width="5" height="3" rx="1" {...solid} />
  </svg> },
];

// Compacte: 5 h and 7 j bars stacked tight. Classique: separated rows, each
// with its label, value and reset line.
const claudeLayoutOptions: Choice<ClaudeLayout>[] = [
  { value: "compact", label: "Compacte", icon: <svg viewBox="0 0 48 32">
    <path d="M6 9h14M34 9h8" {...glyph} />
    <rect x="6" y="14" width="36" height="4" rx="2" {...soft} /><rect x="6" y="14" width="25" height="4" rx="2" {...solid} />
    <rect x="6" y="19.5" width="36" height="2" rx="1" {...soft} /><rect x="6" y="19.5" width="31" height="2" rx="1" {...solid} />
  </svg> },
  { value: "classic", label: "Classique", icon: <svg viewBox="0 0 48 32">
    <path d="M6 3.5h12M36 3.5h6M6 22.5h12M36 22.5h6" {...glyph} />
    <rect x="6" y="7" width="36" height="3" rx="1.5" {...soft} /><rect x="6" y="7" width="25" height="3" rx="1.5" {...solid} />
    <path d="M6 14h20" {...glyph} opacity="0.45" />
    <rect x="6" y="26" width="36" height="3" rx="1.5" {...soft} /><rect x="6" y="26" width="31" height="3" rx="1.5" {...solid} />
  </svg> },
];

/** Radio group drawn as the existing segmented control, with an icon per
 *  option (`tiles`: pictogram above the label). Native radios keep arrow keys. */
function Choices<T extends string>({ legend, label, name, options, value, tiles = false, onChange }: {
  legend: string; label: string; name: string; options: Choice<T>[]; value: T; tiles?: boolean; onChange: (value: T) => void;
}) {
  return <>
    <legend className="prefs-group__title prefs-group__legend">{legend}</legend>
    <div className={"prefs-segmented prefs-choices" + (tiles ? " prefs-choices--tiles" : "")} role="radiogroup" aria-label={label}>
      {options.map((option) => <label key={option.value} className={value === option.value ? "is-selected" : ""} title={option.title}>
        <input type="radio" name={name} value={option.value} checked={value === option.value} onChange={() => onChange(option.value)} />
        <span className="prefs-choices__icon" aria-hidden="true">{option.icon}</span>
        <span>{option.label}</span>
      </label>)}
    </div>
  </>;
}

const claudeStates: Record<ClaudeSnapshot["state"], [string, Tone, string]> = {
  disabled: ["Désactivé", "neutral", "Désactivé : aucune lecture."],
  not_configured: ["Non installé", "neutral", "Claude Code non installé : installez-le puis lancez claude auth login."],
  waiting_for_data: ["Lecture…", "warning", "Lecture des quotas via Claude Code…"],
  ready: ["Connecté", "good", "Connecté."],
  stale: ["Ancien", "warning", "Dernières données anciennes."],
  error: ["Erreur", "error", "Lecture impossible : vérifiez la connexion et la version de Claude Code."],
};

function codexState(enabled: boolean, status: CodexStatus | null): [string, Tone, string] {
  if (!enabled) return ["Désactivé", "neutral", "Désactivé : aucune connexion ni sondage."];
  const message = status?.message || "Connexion à Codex…";
  switch (status?.phase) {
    case "ready": return ["Connecté", "good", message];
    case "error": return ["Erreur", "error", message];
    case "missing": return ["Non installé", "neutral", message];
    case "signed_out": return ["Déconnecté", "warning", message];
    default: return ["Connexion…", "warning", message];
  }
}

/** Provider title that doubles as the detailed-usage link: the existing
 *  external arrow after the name. Plain text while the provider is off. */
function usageTitle(title: string, label: string, command: string, enabled: boolean) {
  if (!enabled) return title;
  return <button type="button" className="prefs-title-link" aria-label={label} title="Usage détaillé"
    onClick={() => void invoke(command).catch(() => undefined)}>{title} <ExternalIcon /></button>;
}

/** Fits the Settings window to its content (per tab), then corrects any
 *  remaining overflow measured by the real webview. */
function useFitWindow(content: RefObject<HTMLElement | null>, key: unknown) {
  useLayoutEffect(() => {
    const node = content.current;
    if (!node) return;
    let frame = 0;
    let wanted = 0;
    let corrections = 0;
    const request = (height: number) => {
      wanted = height;
      void invoke("fit_settings_window", { height }).catch((cause) => console.error("fit_settings_window", cause));
    };
    const fit = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        corrections = 0;
        const height = Math.ceil(node.getBoundingClientRect().height);
        if (Math.abs(height - window.innerHeight) > 1) request(height);
      });
    };
    // After the window resized: if the page still scrolls, grow by the overflow (bounded).
    const check = () => {
      const overflow = document.documentElement.scrollHeight - window.innerHeight;
      if (overflow > 1 && corrections < 3 && wanted > 0) { corrections += 1; request(wanted + overflow); }
    };
    const observer = new ResizeObserver(fit);
    observer.observe(node);
    window.addEventListener("resize", check);
    fit();
    return () => { observer.disconnect(); window.removeEventListener("resize", check); cancelAnimationFrame(frame); };
  }, [content, key]);
}

function Switch({ label, checked, disabled, onChange }: {
  label: string; checked: boolean; disabled: boolean; onChange: (value: boolean) => void;
}) {
  return <label className="prefs-switch">
    <input type="checkbox" role="switch" aria-label={label} checked={checked} disabled={disabled}
      onChange={(event) => onChange(event.target.checked)} />
    <span aria-hidden="true" />
  </label>;
}

export default function Settings() {
  const [tab, setTab] = useState<TabId>("providers");
  const [prefs, setPrefs] = useState<Preferences | null>(null);
  const [status, setStatus] = useState<CodexStatus | null>(null);
  const [claude, setClaude] = useState<ClaudeSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const tabRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const root = useRef<HTMLElement>(null);
  const [accounts, setAccounts] = useState<ClaudeAccount[]>([]);
  const [accountsError, setAccountsError] = useState<string | null>(null);
  const mutation = useRef(false);
  const [canChooseMode, setCanChooseMode] = useState(false);
  const [accountActionError, setAccountActionError] = useState<string | null>(null);
  // Codex homes (primary + followed + detected), same flow as Claude.
  const [codexAccounts, setCodexAccounts] = useState<CodexAccount[]>([]);
  const [codexAccountsError, setCodexAccountsError] = useState<string | null>(null);
  const [codexActionError, setCodexActionError] = useState<string | null>(null);
  useFitWindow(root, [tab, prefs !== null].join());
  const reload = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    let disposed = false;
    void invoke<string>("get_platform").then((platform) => {
      if (!disposed) setCanChooseMode(platform === "macos");
    }).catch(() => undefined);
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    const quotas = listen("quota-updated", reload).catch(() => () => {});
    return () => { void quotas.then((unlisten) => unlisten()); };
  }, [reload]);

  useEffect(() => {
    let disposed = false;
    let timer: number;
    const poll = async () => {
      const [p, s, k, a, x] = await Promise.allSettled([
        invoke<Preferences>("get_preferences"),
        invoke<CodexStatus>("get_codex_status"),
        invoke<ClaudeSnapshot>("get_claude_snapshot"),
        invoke<ClaudeAccount[]>("get_claude_accounts"),
        invoke<CodexAccount[]>("get_codex_accounts"),
      ]);
      if (disposed) return;
      if (p.status === "fulfilled" && !mutation.current) setPrefs(p.value);
      if (a.status === "fulfilled") setAccounts(a.value);
      setAccountsError(a.status === "rejected" ? "Impossible de découvrir les comptes Claude. Nouvelle tentative automatique…" : null);
      if (x.status === "fulfilled" && Array.isArray(x.value)) setCodexAccounts(x.value);
      setCodexAccountsError(x.status === "rejected" ? "Impossible de découvrir les comptes Codex. Nouvelle tentative automatique…" : null);
      if (s.status === "fulfilled") setStatus(s.value);
      if (k.status === "fulfilled") setClaude(k.value);
      timer = window.setTimeout(() => void poll(), 2000);
    };
    void poll();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [revision]);

  const update = async (patch: Partial<Preferences>) => {
    if (!prefs || mutation.current) return false;
    mutation.current = true;
    setError(null); setBusy(true);
    try { setPrefs(await invoke<Preferences>("set_preferences", { prefs: { ...prefs, ...patch } })); reload(); return true; }
    catch (cause) { setError(String(cause)); return false; }
    finally { mutation.current = false; setBusy(false); }
  };
  // Explicit add (folder chooser or absolute path): the backend validates and
  // deduplicates, then switches to Multi. Errors stay next to the action.
  const addProfile = async (path: string) => {
    if (mutation.current) return false;
    mutation.current = true; setAccountActionError(null); setBusy(true);
    try { setPrefs(await invoke<Preferences>("add_claude_profile", { path })); reload(); return true; }
    catch (cause) { setAccountActionError(String(cause).replace(/^Error: /, "")); return false; }
    finally { mutation.current = false; setBusy(false); }
  };
  const chooseFolder = async () => {
    if (mutation.current) return null;
    setAccountActionError(null); setBusy(true);
    try { return await invoke<string | null>("choose_claude_profile_folder"); }
    catch (cause) { setAccountActionError(String(cause).replace(/^Error: /, "")); return null; }
    finally { setBusy(false); }
  };
  // Codex homes: explicit add/follow/untrack commands, never login/logout.
  // The backend validates and returns the canonical preferences.
  const codexAction = async (command: "add_codex_profile" | "follow_codex_profile" | "untrack_codex_profile", payload: Record<string, string>) => {
    if (mutation.current) return false;
    mutation.current = true; setCodexActionError(null); setBusy(true);
    try { setPrefs(await invoke<Preferences>(command, payload)); reload(); return true; }
    catch (cause) { setCodexActionError(String(cause).replace(/^Error: /, "")); return false; }
    finally { mutation.current = false; setBusy(false); }
  };
  const chooseCodexFolder = async () => {
    if (mutation.current) return null;
    setCodexActionError(null); setBusy(true);
    try { return await invoke<string | null>("choose_codex_profile_folder"); }
    catch (cause) { setCodexActionError(String(cause).replace(/^Error: /, "")); return null; }
    finally { setBusy(false); }
  };
  const relay = async (command: "install_claude_relay" | "uninstall_claude_relay") => {
    setError(null); setBusy(true);
    try { setClaude(await invoke<ClaudeSnapshot>(command)); reload(); }
    catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };

  const onTabKey = (event: KeyboardEvent, index: number) => {
    const move = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : event.key === "Home" ? -index : event.key === "End" ? tabs.length - 1 - index : 0;
    if (!move) return;
    event.preventDefault();
    const next = tabs[(index + move + tabs.length) % tabs.length];
    setTab(next.id); tabRefs.current[next.id]?.focus();
  };

  if (!prefs) return <main className="prefs"><p className="prefs-loading" role="status">Chargement…</p></main>;
  const [, , codexMessage] = codexState(prefs.codexEnabled, status);
  const [, claudeTone, claudeMessage]: [string, Tone, string] = !prefs.claudeEnabled
    ? claudeStates.disabled
    : claude ? claudeStates[claude.state] : ["Lecture…", "warning", "Lecture…"];
  const codexReady = status?.phase === "ready";
  const claudeReady = claude?.state === "ready";
  const layout = layoutOf(prefs);

  return <main className="prefs" ref={root} aria-label="Réglages">
    <nav role="tablist" aria-label="Catégories de réglages" className="prefs-toolbar">
      {tabs.map((item, index) => <button key={item.id} type="button" role="tab" id={"tab-" + item.id}
        ref={(node) => { tabRefs.current[item.id] = node; }}
        aria-selected={tab === item.id} aria-controls={"panel-" + item.id}
        tabIndex={tab === item.id ? 0 : -1} onClick={() => setTab(item.id)} onKeyDown={(event) => onTabKey(event, index)}>
        <svg viewBox="0 0 24 24" aria-hidden="true">{icons[item.id]}</svg>
        <span>{item.label}</span>
      </button>)}
    </nav>

    <div className="prefs-body">
      {error && <p className="prefs-alert" role="alert">{error}</p>}
      {tab === "notifications" && <div role="tabpanel" id="panel-notifications" aria-labelledby="tab-notifications">
        <NtfySettings config={prefs.ntfy} onSave={(ntfy) => update({ ntfy })} />
      </div>}

      {tab === "providers" && <div role="tabpanel" id="panel-providers" aria-labelledby="tab-providers">
        <section className="prefs-group" aria-labelledby="codex-heading">
          <header className="prefs-group__head">
            <div><h2 id="codex-heading" className="prefs-group__title">{usageTitle("Codex", "Codex : usage détaillé sur chatgpt.com", "open_usage_details", prefs.codexEnabled)}</h2></div>
            <Switch label="Activer Codex" checked={prefs.codexEnabled} disabled={busy}
              onChange={(value) => void update({ codexEnabled: value })} />
          </header>
          {!prefs.codexEnabled && <p className="prefs-state">{codexMessage}</p>}
          {prefs.codexEnabled && !codexReady && <CodexConnection status={status} onChange={reload} />}
          <CodexAccountsSettings prefs={prefs} accounts={codexAccounts} busy={busy} onUpdate={update}
            // Reset and detail act on the account of their own row (never the primary by default).
            accessory={(account) => account.tracked && account.phase === "ready" && !account.duplicateOf
              ? <ResetCredits accountId={account.id} accountName={account.name} primary={account.primary}
                ready={account.available} count={account.availableResets} onChange={reload} />
              : null}
            rowsOf={(account) => codexRows(account.rows)}
            renderDetail={(account) => account.primary && codexReady ? <CodexConnection status={status} onChange={reload} /> : null}
            onAdd={(path) => codexAction("add_codex_profile", { path })}
            onFollow={(id) => codexAction("follow_codex_profile", { id })}
            onUntrack={(id) => codexAction("untrack_codex_profile", { id })}
            onChooseFolder={canChooseMode ? chooseCodexFolder : undefined} actionError={codexActionError}
            advanced={<p className="prefs-footnote">Un dossier <code>CODEX_HOME</code> par compte, lu en lecture seule par Codex CLI. Connexion dans un terminal : <code>CODEX_HOME=… codex login</code>.</p>} />
          {codexAccountsError && <p className="prefs-state prefs-state--error" role="alert">{codexAccountsError}</p>}
        </section>

        <section className="prefs-group" aria-labelledby="claude-heading">
          <header className="prefs-group__head">
            <div><h2 id="claude-heading" className="prefs-group__title">{usageTitle("Claude", "Claude : usage détaillé sur claude.ai", "open_claude_usage", prefs.claudeEnabled)}</h2></div>
            <Switch label="Activer Claude" checked={prefs.claudeEnabled} disabled={busy}
              onChange={(value) => void update({ claudeEnabled: value })} />
          </header>
          {(!prefs.claudeEnabled || !claudeReady) && <p className={"prefs-state prefs-state--" + claudeTone} role="status">{claudeMessage}</p>}
          <div className="prefs-group__head prefs-fable">
            <div><h3 className="prefs-option__title">Quota Fable</h3></div>
            <Switch label="Afficher le quota Fable et ses alertes" checked={prefs.fableEnabled} disabled={busy || !prefs.claudeEnabled}
              onChange={(value) => void update({ fableEnabled: value })} />
          </div>
          <ClaudeAccountsSettings prefs={prefs} accounts={accounts} busy={busy} onUpdate={update} onAdd={addProfile}
            // Paid extra usage is reported per account: it moves onto its row.
            accessory={(account) => prefs.claudeEnabled && account.snapshot.extraUsage?.enabled
              ? <button type="button" className="reset-chip"
                onClick={() => void invoke("open_claude_usage").catch(() => undefined)}
                title="Usage supplémentaire payant activé" aria-label={"Usage supplémentaire Claude : " + account.name}>
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M8 3.5v9M3.5 8h9" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" /></svg><span>{account.snapshot.extraUsage.limitReached ? "Plafond" : "Extra"}</span>
              </button>
              : null}
            rowsOf={(account) => claudeRows(account.snapshot, prefs.fableEnabled)}
            onChooseFolder={canChooseMode ? chooseFolder : undefined} actionError={accountActionError}
            advanced={<>
              <p className="prefs-footnote">Lu via Claude Code (<code>claude auth login</code>), sans jeton ni requête de modèle.</p>
              <div className="prefs-actions">
                {prefs.claudeEnabled && <button type="button" disabled={busy} onClick={() => void invoke("request_claude_refresh").then(reload)}>Actualiser</button>}
                <button type="button" className="prefs-link" disabled={busy} onClick={() => void relay("uninstall_claude_relay")}>Retirer l’ancien relais</button>
              </div>
            </>} />
          {accountsError && <p className="prefs-state prefs-state--error" role="alert">{accountsError}</p>}
        </section>
      </div>}

      {tab === "menubar" && <div role="tabpanel" id="panel-menubar" aria-labelledby="tab-menubar">
        {canChooseMode && <fieldset className="prefs-group" aria-busy={busy || undefined}>
          <Choices legend="Mode d’affichage" label="Mode d’affichage" name="displayMode" tiles options={displayModeOptions}
            value={prefs.displayMode} onChange={(displayMode) => void update({ displayMode })} />
        </fieldset>}
        {(!canChooseMode || prefs.displayMode !== "app") && <fieldset className="prefs-group" aria-busy={busy || undefined}>
          <Choices legend="Afficher dans la barre des menus" label="Indicateur de la barre des menus (quota 5 heures)" name="indicator"
            options={indicatorOptions} value={prefs.indicator} onChange={(indicator) => void update({ indicator })} />
        </fieldset>}
        {/* One presentation for both providers, without quota collection. */}
        <fieldset className="prefs-group" aria-busy={busy || undefined}>
          <Choices legend="Présentation" label="Présentation" name="layout" tiles
            options={claudeLayoutOptions} value={layout} onChange={(layout) => void update({ layout })} />
        </fieldset>
      </div>}

      {tab === "refresh" && <div role="tabpanel" id="panel-refresh" aria-labelledby="tab-refresh">
        <section className="prefs-group" aria-labelledby="refresh-heading">
          <h2 id="refresh-heading" className="prefs-group__title">Actualisation des quotas</h2>
          <RefreshSettings key={String(status?.refreshSettings.customSeconds ?? "auto")} status={status} onChange={reload} />
        </section>
      </div>}
    </div>
  </main>;
}
