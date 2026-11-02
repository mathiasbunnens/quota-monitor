import { useRef, useState } from "react";
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
  /** Profile already detected (~/.claude-*). */
  onFollow?: (id: string) => Promise<boolean>;
  /** Native folder chooser; absent where unavailable (path entry remains). */
  onChooseFolder?: () => Promise<string | null>;
  /** Last add/follow failure, shown next to the action that caused it. */
  actionError?: string | null;
};

/** Connection state of one profile: [pill, tone, actionable detail]. */
export function accountState(account: ClaudeAccount): [string, Tone, string] {
  const s = account.snapshot;
  if (!account.available) return ["Indisponible", "neutral", "Dossier introuvable : vérifiez qu’il existe encore (disque, renommage)."];
  if (account.refreshing) return ["Actualisation…", "warning", ""];
  if (s.state === "disabled") return ["Désactivé", "neutral", "Activez Claude pour lire ses quotas."];
  if (s.errorKind === "not_signed_in") return ["Non connecté", "warning", s.message || "Profil non connecté dans Claude Code."];
  switch (s.state) {
    case "ready": return ["Connecté", "good", ""];
    case "stale": return ["Connecté", "warning", "Dernières valeurs conservées, nouvelle lecture en attente."];
    case "waiting_for_data": return ["Lecture…", "warning", ""];
    case "not_configured": return ["Non installé", "neutral", s.message || "Claude Code introuvable."];
    default: return ["Erreur", "error", s.message || "Lecture impossible : vérifiez Claude Code."];
  }
}

function StatePill({ account }: { account: ClaudeAccount }) {
  const [label, tone] = accountState(account);
  return <span className={"prefs-pill prefs-pill--" + tone}>{label}</span>;
}

function ProfileName({ account, disabled, onSave }: { account: ClaudeAccount; disabled: boolean; onSave: (name: string) => Promise<boolean> }) {
  const [name, setName] = useState(account.name);
  return <form className="claude-accounts__rename" onSubmit={(event) => { event.preventDefault(); if (name.trim()) void onSave(name.trim()); }}>
    <input aria-label={"Nom du compte " + account.name} value={name} maxLength={64} disabled={disabled} onChange={(event) => setName(event.target.value)} />
    <button type="submit" disabled={disabled || !name.trim() || name.trim() === account.name}>Renommer</button>
  </form>;
}

function Identity({ account }: { account: ClaudeAccount }) {
  return <span className="claude-accounts__name"><strong>{account.name}</strong>
    {account.primary && <small> · Principal</small>}
    {account.snapshot.planType && <span className="prefs-plan"> {account.snapshot.planType}</span>}
  </span>;
}

export default function ClaudeAccountsSettings({ prefs, accounts, busy, onUpdate, onAdd, onFollow, onChooseFolder, actionError = null }: Props) {
  const [path, setPath] = useState("");
  const [pathError, setPathError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const settings: ClaudeAccountsPreferences = prefs.claudeAccounts ?? { mode: "single", selectedId: null, profiles: [] };
  const selected = accounts.find((account) => account.id === settings.selectedId) ?? accounts.find((account) => account.primary) ?? accounts[0];
  const multi = settings.mode === "multi";
  const several = accounts.length >= 2;
  // Stored choice, not the effective view (which is false while Claude is off).
  const storedVisible = (account: ClaudeAccount) => settings.profiles.find((profile) => profile.id === account.id)?.visible ?? account.primary;
  const displayed = (account: ClaudeAccount) => multi ? storedVisible(account) : account.id === selected?.id;
  const candidates = accounts.filter((account) => !displayed(account));
  const saveProfile = (account: ClaudeAccount, patch: Partial<ClaudeProfile>) => {
    // Discovery metadata is not persisted until an explicit edit. Visibility
    // returned while Claude is off is false, not the user's stored choice.
    const stored = settings.profiles.find((profile) => profile.id === account.id);
    const profile: ClaudeProfile = { id: account.id, path: account.path, name: account.name, visible: stored?.visible ?? account.primary, ...patch };
    return onUpdate({ claudeAccounts: { ...settings, profiles: [...settings.profiles.filter((item) => item.id !== account.id), profile] } });
  };
  const done = (ok: boolean) => { if (ok) { setAdding(false); setPath(""); setPathError(null); } return ok; };
  const ctaRef = useRef<HTMLButtonElement>(null);
  // Closing the panel (Escape/Annuler) returns focus to the action that opened it.
  const close = () => { setAdding(false); window.requestAnimationFrame(() => ctaRef.current?.focus()); };
  const chooseFolder = async () => {
    if (busy || !onChooseFolder) return;
    const chosen = await onChooseFolder();
    if (chosen) done(await onAdd(chosen));
  };
  const cta = <button type="button" ref={ctaRef} className="claude-accounts__cta" aria-expanded={adding} aria-controls={adding ? "claude-accounts-add" : undefined}
    disabled={busy} onClick={() => setAdding((value) => !value)}>
    {several ? "Ajouter un compte Claude" : "Suivre un autre compte Claude"}
  </button>;
  const addPanel = adding && <div id="claude-accounts-add" className="claude-accounts__panel" role="group" aria-labelledby="claude-accounts-add-title"
    aria-busy={busy} onKeyDown={(event) => { if (event.key === "Escape") { event.stopPropagation(); close(); } }}>
    <h3 id="claude-accounts-add-title">Suivre un autre compte Claude</h3>
    <p className="prefs-footnote">Associe un <strong>profil existant</strong> de Claude Code, déjà connecté sur cet ordinateur (son dossier de configuration, par exemple <code>~/.claude-travail</code>). Quota Monitor lit seulement ses quotas : aucun compte créé, aucune connexion, la session Claude Code ne change pas.</p>
    <h4>Profils détectés</h4>
    {candidates.length ? <ul className="claude-accounts__candidates" aria-label="Profils Claude détectés">
      {candidates.map((account) => <li key={account.id}>
        <div><Identity account={account} /><code>{account.path}</code></div>
        <StatePill account={account} />
        <button type="button" disabled={busy || !account.available || !onFollow} aria-label={"Suivre " + account.name}
          onClick={async () => { if (onFollow) done(await onFollow(account.id)); }}>Suivre</button>
      </li>)}
    </ul> : <p className="prefs-footnote">Aucun autre profil détecté (dossiers <code>~/.claude-*</code>).</p>}
    <div className="prefs-actions">
      {onChooseFolder && <button type="button" disabled={busy} onClick={() => void chooseFolder()}>Choisir un dossier…</button>}
      <button type="button" className="prefs-link" disabled={busy} onClick={close}>Annuler</button>
    </div>
    <details className="prefs-disclosure" open={!onChooseFolder || undefined}>
      <summary>Option avancée : chemin absolu</summary>
      <form className="claude-accounts__add" onSubmit={async (event) => {
        event.preventDefault();
        if (busy) return;
        if (!/^(\/|[A-Za-z]:[\\/]|\\\\)/.test(path)) { setPathError("Indiquez un chemin absolu vers un dossier de profil existant (il commence par /)."); return; }
        setPathError(null);
        done(await onAdd(path));
      }}>
        <label>Chemin absolu du dossier de configuration Claude
          <input value={path} disabled={busy} placeholder="/Users/nom/.claude-travail" spellCheck={false}
            aria-invalid={pathError ? true : undefined} onChange={(event) => setPath(event.target.value)} />
        </label>
        <button type="submit" disabled={busy || !path}>Suivre ce dossier</button>
        {pathError && <p role="alert" className="prefs-state prefs-state--error">{pathError}</p>}
      </form>
    </details>
    {actionError && <p role="alert" className="prefs-state prefs-state--error">{actionError}</p>}
  </div>;

  return <div className="claude-accounts" aria-busy={busy || undefined}>
    {several && <>
      <div className="prefs-segmented" role="radiogroup" aria-label="Comptes Claude affichés">
        {([['single', 'Unique'], ['multi', 'Multi']] as const).map(([mode, label]) => <label key={mode} className={settings.mode === mode ? "is-selected" : ""}>
          <input type="radio" name="claude-mode" checked={settings.mode === mode} disabled={busy}
            onChange={() => void onUpdate({ claudeAccounts: { ...settings, mode } })} />{label}
        </label>)}
      </div>
      {!multi && <label>Compte Claude affiché
        <select value={selected?.id ?? ""} disabled={busy} onChange={(event) => void onUpdate({ claudeAccounts: { ...settings, selectedId: event.target.value } })}>
          {accounts.map((account) => <option key={account.id} value={account.id}>{account.name}{account.primary ? " · Principal" : ""}</option>)}
        </select>
      </label>}
    </>}

    {multi && <>
      <ul className="claude-accounts__list" aria-label="Comptes Claude">
        {accounts.map((account) => <li className="claude-accounts__profile" key={account.id}>
          <div className="claude-accounts__heading">
            <Identity account={account} />
            <StatePill account={account} />
            <label className="prefs-switch"><input type="checkbox" role="switch" aria-label={"Afficher " + account.name}
              checked={storedVisible(account)} disabled={busy}
              onChange={(event) => void saveProfile(account, { visible: event.target.checked })} /><span aria-hidden="true" /></label>
          </div>
          {accountState(account)[2] && <p className="prefs-footnote">{accountState(account)[2]}</p>}
          <ProfileName key={account.id + account.name} account={account} disabled={busy} onSave={(name) => saveProfile(account, { name })} />
        </li>)}
      </ul>
      {cta}{addPanel}
    </>}

    {!multi && !several && selected && <div className="claude-accounts__profile">
      <div className="claude-accounts__heading"><Identity account={selected} /><StatePill account={selected} /></div>
      {accountState(selected)[2] && <p className="prefs-footnote">{accountState(selected)[2]}</p>}
    </div>}
    {!multi && !several && <>{cta}{addPanel}</>}

    {!multi && accounts.length > 0 && <details className="prefs-disclosure">
      <summary>{several ? `Gérer les comptes Claude (${accounts.length})` : "Gérer le compte Claude"}</summary>
      {accounts.map((account) => <div className="claude-accounts__profile" key={account.id}>
        <div className="claude-accounts__heading"><Identity account={account} /><StatePill account={account} /></div>
        <code>{account.path}</code>
        {accountState(account)[2] && <p className="prefs-footnote">{accountState(account)[2]}</p>}
        <ProfileName key={account.id + account.name} account={account} disabled={busy} onSave={(name) => saveProfile(account, { name })} />
      </div>)}
      {several && <div className="claude-accounts__more">{cta}{addPanel}</div>}
    </details>}

    {several && <p className="prefs-footnote">L’affichage ne change pas la session Claude Code. Tous les comptes sont actualisés lorsque Claude est activé ; leur présentation se règle dans Affichage.</p>}
  </div>;
}
