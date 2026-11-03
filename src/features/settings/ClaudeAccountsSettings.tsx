import { useId, useRef, useState, type ReactNode } from "react";
import type { ClaudeAccount, ClaudeAccountsPreferences, ClaudeProfile, CodexAccount, Preferences } from "../quota/types";
import type { QuotaRow } from "../quota/providers";
import "./claude-accounts.scss";

type Tone = "good" | "warning" | "error" | "neutral";

/** What the shared account list needs from a Claude profile or a Codex home. */
type ListedAccount = ClaudeProfile & { primary: boolean; available: boolean; refreshing: boolean; tracked?: boolean };

type Props<A extends ListedAccount = ClaudeAccount> = {
  prefs: Preferences;
  accounts: A[];
  busy: boolean;
  onUpdate: (patch: Partial<Preferences>) => Promise<boolean>;
  /** Existing folder (picker or typed absolute path). */
  onAdd: (path: string) => Promise<boolean>;
  /** Native folder chooser; absent where unavailable (path entry remains). */
  onChooseFolder?: () => Promise<string | null>;
  /** Last add failure, shown under the Add action. */
  actionError?: string | null;
  /** Extra technical actions shown in the single secondary area. */
  advanced?: ReactNode;
  /** Codex reuse: provider wording, preference key and per-account state. */
  provider?: "Claude" | "Codex";
  stateOf?: (account: A) => [string, Tone, string];
  planOf?: (account: A) => string | null | undefined;
  /** Follow a detected, not yet followed folder (`tracked: false`). */
  onFollow?: (id: string) => Promise<boolean>;
  /** Stop following an additional folder (never deletes it). */
  onUntrack?: (id: string) => Promise<boolean>;
  /** Per-account control shown before the connection state (reset, extra usage). */
  accessory?: (account: A) => ReactNode;
  /** Windows of THIS account, listed in its detail. */
  rowsOf?: (account: A) => QuotaRow[];
  /** Extra detail content of THIS account (e.g. the Codex connection panel). */
  renderDetail?: (account: A) => ReactNode;
};

/** Connection state of one profile: [pill, tone, actionable detail]. */
export function accountState(account: ClaudeAccount): [string, Tone, string] {
  const s = account.snapshot;
  if (!account.available) return ["Indisponible", "neutral", "Dossier introuvable : vérifiez qu’il existe encore (disque, renommage)."];
  if (account.refreshing) return ["Actualisation…", "warning", ""];
  if (s.state === "disabled") return ["Désactivé", "neutral", ""];
  if (s.errorKind === "not_signed_in") return ["Non connecté", "warning", s.message || "Profil non connecté dans Claude Code."];
  switch (s.state) {
    case "ready": return ["Connecté", "good", ""];
    case "stale": return ["Connecté", "warning", "Dernières valeurs conservées, nouvelle lecture en attente."];
    case "waiting_for_data": return ["Lecture…", "warning", ""];
    case "not_configured": return ["Non installé", "neutral", s.message || "Claude Code introuvable."];
    default: return ["Erreur", "error", s.message || "Lecture impossible : vérifiez Claude Code."];
  }
}

/** Connection state of one Codex home: [pill, tone, actionable detail]. */
export function codexAccountState(account: CodexAccount): [string, Tone, string] {
  if (!account.tracked) return ["Détecté", "neutral", "Dossier détecté, non suivi : « Suivre » lit seulement ses quotas, sans connexion ni modification."];
  if (!account.available) return ["Indisponible", "neutral", "Dossier CODEX_HOME introuvable : vérifiez qu’il existe encore (disque, renommage)."];
  if (account.refreshing) return ["Actualisation…", "warning", ""];
  if (account.duplicateOf && account.phase === "ready") return ["Même compte", "warning", "Même compte que « " + account.duplicateOf + " » : quotas non additionnés."];
  switch (account.phase) {
    case "ready": return ["Connecté", "good", ""];
    case "signed_out": return ["Non connecté", "warning", account.message || "Dossier non connecté à Codex."];
    case "disabled": return ["Désactivé", "neutral", ""];
    case "missing": return ["Non installé", "neutral", account.message];
    case "error": return ["Erreur", "error", account.message || "Lecture impossible : vérifiez Codex CLI."];
    default: return ["Connexion…", "warning", ""];
  }
}

const pencil = <path d="M10.5 2.8 13.2 5.5 6 12.7 3 13l.3-3 7.2-7.2Z" />;
const chevron = <path d="M6 3.5 10.5 8 6 12.5" />;
const plus = <path d="M8 3.5v9M3.5 8h9" />;

function Glyph({ children }: { children: ReactNode }) {
  return <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true" className="claude-accounts__glyph">{children}</svg>;
}

/** Inline rename: the pencil swaps the name for a field (Enter saves, Escape cancels). */
export function AccountName({ account, disabled, onSave }: { account: { name: string }; disabled: boolean; onSave: (name: string) => Promise<boolean> }) {
  const [draft, setDraft] = useState<string | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  // Enter/Escape unmount the field, which also fires blur: settle only once.
  const settled = useRef(false);
  const stop = () => { settled.current = true; setDraft(null); window.requestAnimationFrame(() => button.current?.focus()); };
  const commit = async () => {
    if (settled.current) return;
    settled.current = true;
    const name = draft?.trim();
    if (name && name !== account.name && !(await onSave(name))) { settled.current = false; return; }
    stop();
  };
  if (draft !== null) return <input className="claude-accounts__rename" aria-label={"Nom du compte " + account.name} value={draft}
    maxLength={64} disabled={disabled} autoFocus spellCheck={false}
    onChange={(event) => setDraft(event.target.value)} onBlur={() => void commit()}
    onKeyDown={(event) => {
      if (event.key === "Enter") { event.preventDefault(); void commit(); }
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); stop(); }
    }} />;
  return <>
    <strong className="claude-accounts__label">{account.name}</strong>
    <button type="button" ref={button} className="claude-accounts__icon" disabled={disabled}
      aria-label={"Renommer " + account.name} title="Renommer" onClick={() => { settled.current = false; setDraft(account.name); }}><Glyph>{pencil}</Glyph></button>
  </>;
}

export default function ClaudeAccountsSettings<A extends ListedAccount = ClaudeAccount>({ prefs, accounts, busy, onUpdate, onAdd, onChooseFolder, actionError = null, advanced,
  provider = "Claude", stateOf, planOf, onFollow, onUntrack, accessory, rowsOf, renderDetail }: Props<A>) {
  const [path, setPath] = useState("");
  const [pathError, setPathError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [details, setDetails] = useState<string | null>(null);
  const pathInput = useRef<HTMLInputElement>(null);
  const advancedId = useId();
  const codex = provider === "Codex";
  const key = codex ? "codexAccounts" : "claudeAccounts";
  const slug = codex ? "codex" : "claude";
  // Visibility labels stay unique when both providers list a « Principal ».
  const show = (account: A) => "Afficher " + (codex ? "Codex " : "") + account.name;
  const detailLabel = (account: A) => "Détail " + (codex ? "Codex " : "") + account.name;
  const state = stateOf ?? ((account: A) => accountState(account as unknown as ClaudeAccount));
  const plan = planOf ?? ((account: A) => (account as unknown as ClaudeAccount).snapshot?.planType);
  const settings: ClaudeAccountsPreferences = prefs[key] ?? { mode: "single", selectedId: null, profiles: [] };
  // Only followed accounts can be displayed; a detected folder is listed to be followed.
  const listed = accounts.filter((account) => account.tracked !== false);
  const selected = listed.find((account) => account.id === settings.selectedId) ?? listed.find((account) => account.primary) ?? listed[0];
  const multi = settings.mode === "multi";
  const several = listed.length >= 2;
  // Stored choice, not the effective view (which is false while Claude is off).
  const storedVisible = (account: A) => settings.profiles.find((profile) => profile.id === account.id)?.visible ?? account.primary;
  const saveProfile = (account: A, patch: Partial<ClaudeProfile>) => {
    // Discovery metadata is not persisted until an explicit edit. Visibility
    // returned while Claude is off is false, not the user's stored choice.
    const stored = settings.profiles.find((profile) => profile.id === account.id);
    const profile: ClaudeProfile = { id: account.id, path: account.path, name: account.name, visible: stored?.visible ?? account.primary, ...patch };
    return onUpdate({ [key]: { ...settings, profiles: [...settings.profiles.filter((item) => item.id !== account.id), profile] } });
  };
  // « Ajouter » opens the folder chooser directly; without one, the path field.
  const add = async () => {
    if (busy) return;
    if (onChooseFolder) {
      const chosen = await onChooseFolder();
      if (chosen) await onAdd(chosen);
      return;
    }
    setOpen(true);
    window.requestAnimationFrame(() => pathInput.current?.focus());
  };
  const submitPath = async () => {
    if (busy) return;
    if (!/^(\/|[A-Za-z]:[\\/]|\\\\)/.test(path)) { setPathError("Chemin absolu requis (il commence par /)."); return; }
    setPathError(null);
    if (await onAdd(path)) setPath("");
  };

  return <div className="claude-accounts" aria-busy={busy || undefined}>
    {several && <div className="prefs-segmented" role="radiogroup" aria-label={"Comptes " + provider + " affichés"}>
      {([["single", "Unique"], ["multi", "Multi"]] as const).map(([mode, label]) => <label key={mode} className={settings.mode === mode ? "is-selected" : ""}>
        <input type="radio" name={slug + "-mode"} checked={settings.mode === mode}
          onChange={() => void onUpdate({ [key]: { ...settings, mode } })} />{label}
      </label>)}
    </div>}

    {accounts.length > 0 && <ul className="claude-accounts__list" aria-label={"Comptes " + provider}>
      {accounts.map((account) => {
        const [pill, tone, detail] = state(account);
        const detailId = slug + "-detail-" + account.id;
        const tracked = account.tracked !== false;
        const planType = plan(account);
        const isOpen = details === account.id;
        const windows = rowsOf?.(account) ?? [];
        const extra = renderDetail?.(account);
        return <li className="claude-accounts__row" key={account.id}>
          <div className="claude-accounts__who">
            <AccountName key={account.id} account={account} disabled={busy || !tracked} onSave={(name) => saveProfile(account, { name })} />
            {account.primary && <small>Principal</small>}
            {planType && <span className="prefs-plan">{planType}</span>}
          </div>
          {accessory?.(account)}
          <span className={"prefs-pill prefs-pill--" + tone} title={detail || undefined}>{pill}</span>
          <button type="button" className="claude-accounts__icon" aria-label={detailLabel(account)}
            aria-expanded={isOpen} aria-controls={isOpen ? detailId : undefined}
            onClick={() => setDetails((value) => value === account.id ? null : account.id)}><Glyph>{chevron}</Glyph></button>
          {!tracked
            ? onFollow && <button type="button" className="prefs-link" disabled={busy} aria-label={"Suivre " + account.name}
              onClick={() => void onFollow(account.id)}>Suivre</button>
            : multi
            ? <label className="prefs-switch"><input type="checkbox" role="switch" aria-label={show(account)}
              checked={storedVisible(account)}
              onChange={(event) => void saveProfile(account, { visible: event.target.checked })} /><span aria-hidden="true" /></label>
            : several && <label className="claude-accounts__pick"><input type="radio" name={slug + "-selected"} aria-label={show(account)}
              checked={account.id === selected?.id}
              onChange={() => void onUpdate({ [key]: { ...settings, selectedId: account.id } })} /><span aria-hidden="true" /></label>}
          {isOpen && <div id={detailId} className="claude-accounts__detail" role="group" aria-label={detailLabel(account)}>
            {detail && <p>{detail}</p>}
            {windows.length > 0 && <ul className="claude-accounts__windows">
              {windows.map((window) => <li key={window.id}>
                <span>{window.label}</span>
                <strong>{window.remaining !== null ? (window.exhausted ? "Épuisé" : window.remaining + " % restants") : window.missing}</strong>
                <small>Réinitialisation : {window.resetText}</small>
              </li>)}
            </ul>}
            {extra}
          </div>}
        </li>;
      })}
    </ul>}

    <div className="claude-accounts__actions">
      <button type="button" className="claude-accounts__add" disabled={busy} onClick={() => void add()}>
        <Glyph>{plus}</Glyph>Ajouter un compte
      </button>
      <button type="button" className="prefs-more" aria-label="Chemins et options avancées" title="Chemins et options avancées"
        aria-expanded={open} aria-controls={open ? advancedId : undefined} onClick={() => setOpen((value) => !value)}>
        <Glyph>{chevron}</Glyph>
      </button>
    </div>
    {actionError && <p role="alert" className="prefs-state prefs-state--error">{actionError}</p>}

    {open && <div id={advancedId} className="prefs-advanced claude-accounts__advanced" role="group" aria-label="Chemins et options avancées">
      {accounts.length > 0 && <dl className="claude-accounts__paths">
        {accounts.map((account) => <div key={account.id}><dt>{account.name}</dt><dd><code>{account.path}</code>
          {onUntrack && account.tracked !== false && !account.primary && <> <button type="button" className="prefs-link" disabled={busy}
            aria-label={"Ne plus suivre " + account.name} title="Le dossier et son contenu ne sont pas modifiés"
            onClick={() => void onUntrack(account.id)}>Ne plus suivre</button></>}
        </dd></div>)}
      </dl>}
      <form className="claude-accounts__manual" onSubmit={(event) => { event.preventDefault(); void submitPath(); }}>
        <input ref={pathInput} value={path} disabled={busy} placeholder={codex ? "/Users/nom/.codex-travail" : "/Users/nom/.claude-travail"} spellCheck={false}
          aria-label={codex ? "Chemin absolu d’un dossier CODEX_HOME" : "Chemin absolu d’un profil Claude Code"} aria-invalid={pathError ? true : undefined}
          onChange={(event) => setPath(event.target.value)} />
        <button type="submit" disabled={busy || !path}>Ajouter</button>
      </form>
      {pathError && <p role="alert" className="prefs-state prefs-state--error">{pathError}</p>}
      {advanced}
    </div>}
  </div>;
}

type CodexProps = Omit<Props<CodexAccount>, "provider" | "stateOf" | "planOf">;
/** Codex homes: the same list, wording adapted, `codexAccounts` preferences. */
export function CodexAccountsSettings(props: CodexProps) {
  return <ClaudeAccountsSettings<CodexAccount> {...props} provider="Codex" stateOf={codexAccountState} planOf={(account) => account.planType} />;
}
