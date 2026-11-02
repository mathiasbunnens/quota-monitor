import { useId, useRef, useState, type ReactNode } from "react";
import type { ClaudeAccount, ClaudeAccountsPreferences, ClaudeProfile, Preferences } from "../quota/types";
import "./claude-accounts.scss";

type Tone = "good" | "warning" | "error" | "neutral";

type Props = {
  prefs: Preferences;
  accounts: ClaudeAccount[];
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

const pencil = <path d="M10.5 2.8 13.2 5.5 6 12.7 3 13l.3-3 7.2-7.2Z" />;
const info = <><circle cx="8" cy="8" r="6" /><path d="M8 7.2v3.6M8 5.2v.1" /></>;
const chevron = <path d="M6 3.5 10.5 8 6 12.5" />;
const plus = <path d="M8 3.5v9M3.5 8h9" />;

function Glyph({ children }: { children: ReactNode }) {
  return <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true" className="claude-accounts__glyph">{children}</svg>;
}

/** Inline rename: the pencil swaps the name for a field (Enter saves, Escape cancels). */
export function AccountName({ account, disabled, onSave }: { account: ClaudeAccount; disabled: boolean; onSave: (name: string) => Promise<boolean> }) {
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

export default function ClaudeAccountsSettings({ prefs, accounts, busy, onUpdate, onAdd, onChooseFolder, actionError = null, advanced }: Props) {
  const [path, setPath] = useState("");
  const [pathError, setPathError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [details, setDetails] = useState<string | null>(null);
  const pathInput = useRef<HTMLInputElement>(null);
  const advancedId = useId();
  const settings: ClaudeAccountsPreferences = prefs.claudeAccounts ?? { mode: "single", selectedId: null, profiles: [] };
  const selected = accounts.find((account) => account.id === settings.selectedId) ?? accounts.find((account) => account.primary) ?? accounts[0];
  const multi = settings.mode === "multi";
  const several = accounts.length >= 2;
  // Stored choice, not the effective view (which is false while Claude is off).
  const storedVisible = (account: ClaudeAccount) => settings.profiles.find((profile) => profile.id === account.id)?.visible ?? account.primary;
  const saveProfile = (account: ClaudeAccount, patch: Partial<ClaudeProfile>) => {
    // Discovery metadata is not persisted until an explicit edit. Visibility
    // returned while Claude is off is false, not the user's stored choice.
    const stored = settings.profiles.find((profile) => profile.id === account.id);
    const profile: ClaudeProfile = { id: account.id, path: account.path, name: account.name, visible: stored?.visible ?? account.primary, ...patch };
    return onUpdate({ claudeAccounts: { ...settings, profiles: [...settings.profiles.filter((item) => item.id !== account.id), profile] } });
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
    {several && <div className="prefs-segmented" role="radiogroup" aria-label="Comptes Claude affichés">
      {([["single", "Unique"], ["multi", "Multi"]] as const).map(([mode, label]) => <label key={mode} className={settings.mode === mode ? "is-selected" : ""}>
        <input type="radio" name="claude-mode" checked={settings.mode === mode}
          onChange={() => void onUpdate({ claudeAccounts: { ...settings, mode } })} />{label}
      </label>)}
    </div>}

    {accounts.length > 0 && <ul className="claude-accounts__list" aria-label="Comptes Claude">
      {accounts.map((account) => {
        const [pill, tone, detail] = accountState(account);
        const detailId = "claude-detail-" + account.id;
        return <li className="claude-accounts__row" key={account.id}>
          <div className="claude-accounts__who">
            <AccountName key={account.id} account={account} disabled={busy} onSave={(name) => saveProfile(account, { name })} />
            {account.primary && <small>Principal</small>}
            {account.snapshot.planType && <span className="prefs-plan">{account.snapshot.planType}</span>}
          </div>
          <span className={"prefs-pill prefs-pill--" + tone} title={detail || undefined}>{pill}</span>
          {detail && <button type="button" className="claude-accounts__icon" aria-label={"Détail " + account.name}
            aria-expanded={details === account.id} aria-controls={details === account.id ? detailId : undefined}
            onClick={() => setDetails((value) => value === account.id ? null : account.id)}><Glyph>{info}</Glyph></button>}
          {multi
            ? <label className="prefs-switch"><input type="checkbox" role="switch" aria-label={"Afficher " + account.name}
              checked={storedVisible(account)}
              onChange={(event) => void saveProfile(account, { visible: event.target.checked })} /><span aria-hidden="true" /></label>
            : several && <label className="claude-accounts__pick"><input type="radio" name="claude-selected" aria-label={"Afficher " + account.name}
              checked={account.id === selected?.id}
              onChange={() => void onUpdate({ claudeAccounts: { ...settings, selectedId: account.id } })} /><span aria-hidden="true" /></label>}
          {details === account.id && detail && <p id={detailId} className="claude-accounts__detail">{detail}</p>}
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
        {accounts.map((account) => <div key={account.id}><dt>{account.name}</dt><dd><code>{account.path}</code></dd></div>)}
      </dl>}
      <form className="claude-accounts__manual" onSubmit={(event) => { event.preventDefault(); void submitPath(); }}>
        <input ref={pathInput} value={path} disabled={busy} placeholder="/Users/nom/.claude-travail" spellCheck={false}
          aria-label="Chemin absolu d’un profil Claude Code" aria-invalid={pathError ? true : undefined}
          onChange={(event) => setPath(event.target.value)} />
        <button type="submit" disabled={busy || !path}>Ajouter</button>
      </form>
      {pathError && <p role="alert" className="prefs-state prefs-state--error">{pathError}</p>}
      {advanced}
    </div>}
  </div>;
}
