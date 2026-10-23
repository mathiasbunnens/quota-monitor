import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import ExternalIcon from "../ui/ExternalIcon";
import CodexConnection from "../setup/CodexConnection";
import RefreshSettings from "../setup/RefreshSettings";
import NtfySettings from "./NtfySettings";
import ResetCredits from "../quota/ResetCredits";
import type { ClaudeSnapshot, CodexStatus, IndicatorChoice, Preferences } from "../quota/types";
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
  { id: "menubar", label: "Barre des menus" },
  { id: "refresh", label: "Actualisation" },
  { id: "notifications", label: "Notifications" },
] as const;
type TabId = (typeof tabs)[number]["id"];

const indicatorOptions: { value: IndicatorChoice; label: string }[] = [
  { value: "codex", label: "Codex" },
  { value: "claude", label: "Claude" },
  { value: "both", label: "Les deux" },
];

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

function Pill({ tone, children }: { tone: Tone; children: ReactNode }) {
  return <span className={"prefs-pill prefs-pill--" + tone}>{children}</span>;
}

function UsageLink({ label, command }: { label: string; command: string }) {
  return <button type="button" className="prefs-link prefs-external" onClick={() => void invoke(command).catch(() => undefined)}>
    {label} <ExternalIcon />
  </button>;
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

function More({ label, open, onToggle }: { label: string; open: boolean; onToggle: () => void }) {
  return <button type="button" className="prefs-more" aria-label={label} title={label} aria-expanded={open} onClick={onToggle}>
    <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" /></svg>
  </button>;
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
  const [codexMore, setCodexMore] = useState(false);
  const [claudeMore, setClaudeMore] = useState(false);
  useFitWindow(root, [tab, prefs !== null].join());
  const reload = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    const quotas = listen("quota-updated", reload).catch(() => () => {});
    return () => { void quotas.then((unlisten) => unlisten()); };
  }, [reload]);

  useEffect(() => {
    let disposed = false;
    let timer: number;
    const poll = async () => {
      const [p, s, k] = await Promise.allSettled([
        invoke<Preferences>("get_preferences"),
        invoke<CodexStatus>("get_codex_status"),
        invoke<ClaudeSnapshot>("get_claude_snapshot"),
      ]);
      if (disposed) return;
      if (p.status === "fulfilled") setPrefs(p.value);
      if (s.status === "fulfilled") setStatus(s.value);
      if (k.status === "fulfilled") setClaude(k.value);
      timer = window.setTimeout(() => void poll(), 2000);
    };
    void poll();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [revision]);

  const update = async (patch: Partial<Preferences>) => {
    if (!prefs) return false;
    setError(null); setBusy(true);
    try { setPrefs(await invoke<Preferences>("set_preferences", { prefs: { ...prefs, ...patch } })); reload(); return true; }
    catch (cause) { setError(String(cause)); return false; }
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
  const [codexPill, codexTone, codexMessage] = codexState(prefs.codexEnabled, status);
  const [claudePill, claudeTone, claudeMessage]: [string, Tone, string] = !prefs.claudeEnabled
    ? claudeStates.disabled
    : claude ? claudeStates[claude.state] : ["Lecture…", "warning", "Lecture…"];
  const codexReady = status?.phase === "ready";
  const claudeReady = claude?.state === "ready";
  const extra = claude?.extraUsage;

  return <main className="prefs" ref={root}>
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
            <div><h2 id="codex-heading">Codex</h2>{status?.planType && prefs.codexEnabled && <span className="prefs-plan">{status.planType}</span>}</div>
            {prefs.codexEnabled && codexReady && <ResetCredits compact status={status} onChange={reload} />}
            <Pill tone={codexTone}>{codexPill}</Pill>
            <Switch label="Activer Codex" checked={prefs.codexEnabled} disabled={busy}
              onChange={(value) => void update({ codexEnabled: value })} />
          </header>
          {!prefs.codexEnabled && <p className="prefs-state">{codexMessage}</p>}
          {prefs.codexEnabled && codexReady && <div className="prefs-links">
            <UsageLink label="Usage détaillé sur chatgpt.com" command="open_usage_details" />
            <More label="Compte Codex" open={codexMore} onToggle={() => setCodexMore((v) => !v)} />
          </div>}
          {prefs.codexEnabled && codexReady && codexMore && <CodexConnection status={status} onChange={reload} />}
          {prefs.codexEnabled && !codexReady && <CodexConnection status={status} onChange={reload} />}
        </section>

        <section className="prefs-group" aria-labelledby="claude-heading">
          <header className="prefs-group__head">
            <div><h2 id="claude-heading">Claude</h2>{claude?.planType && prefs.claudeEnabled && <span className="prefs-plan">{claude.planType}</span>}</div>
            {prefs.claudeEnabled && extra?.enabled && <button type="button" className="reset-chip"
              onClick={() => void invoke("open_claude_usage").catch(() => undefined)}
              title="Usage supplémentaire payant activé" aria-label="Usage supplémentaire Claude">
              <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M8 3.5v9M3.5 8h9" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" /></svg><span>{extra.limitReached ? "Plafond" : "Extra"}</span>
            </button>}
            <Pill tone={claudeTone}>{claudePill}</Pill>
            <Switch label="Activer Claude" checked={prefs.claudeEnabled} disabled={busy}
              onChange={(value) => void update({ claudeEnabled: value })} />
          </header>
          {(!prefs.claudeEnabled || !claudeReady) && <p className={"prefs-state prefs-state--" + claudeTone} role="status">{claudeMessage}</p>}
          <div className="prefs-group__head prefs-fable">
            <div><h3>Quota Fable</h3><p className="prefs-footnote">Afficher le quota hebdomadaire et ses alertes, si Claude Code le communique. Ne modifie pas votre forfait Anthropic.</p></div>
            <Switch label="Afficher le quota Fable et ses alertes" checked={prefs.fableEnabled} disabled={busy || !prefs.claudeEnabled}
              onChange={(value) => void update({ fableEnabled: value })} />
          </div>
          {prefs.claudeEnabled && <div className="prefs-links">
            <UsageLink label="Usage détaillé sur claude.ai" command="open_claude_usage" />
            <More label="Avancé" open={claudeMore} onToggle={() => setClaudeMore((v) => !v)} />
          </div>}
          {prefs.claudeEnabled && claudeMore && <div className="prefs-advanced">
              <p className="prefs-footnote">Lu via Claude Code (<code>claude auth login</code>), sans jeton ni requête de modèle.</p>
              <div className="prefs-actions">
                <button type="button" disabled={busy} onClick={() => void invoke("request_claude_refresh").then(reload)}>Actualiser</button>
                <button type="button" className="prefs-link" disabled={busy} onClick={() => void relay("uninstall_claude_relay")}>Retirer l’ancien relais</button>
              </div>
          </div>}
        </section>
      </div>}

      {tab === "menubar" && <div role="tabpanel" id="panel-menubar" aria-labelledby="tab-menubar">
        <fieldset className="prefs-group" disabled={busy}>
          <legend className="prefs-group__legend">Afficher dans la barre des menus</legend>
          <div className="prefs-segmented" role="radiogroup" aria-label="Indicateur de la barre des menus">
            {indicatorOptions.map((option) => <label key={option.value} className={prefs.indicator === option.value ? "is-selected" : ""}>
              <input type="radio" name="indicator" value={option.value} checked={prefs.indicator === option.value}
                onChange={() => void update({ indicator: option.value })} />{option.label}
            </label>)}
          </div>
          <p>Quota restant sur la limite 5 heures.</p>
        </fieldset>
      </div>}

      {tab === "refresh" && <div role="tabpanel" id="panel-refresh" aria-labelledby="tab-refresh">
        <section className="prefs-group">
          <RefreshSettings key={String(status?.refreshSettings.customSeconds ?? "auto")} status={status} onChange={reload} />
        </section>
      </div>}
    </div>
  </main>;
}
