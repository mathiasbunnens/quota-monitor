import { useState } from "react";
import type { ClaudeAccount, ClaudeAccountsPreferences, ClaudeProfile, Preferences } from "../quota/types";
import "./claude-accounts.scss";

type Props = {
  prefs: Preferences;
  accounts: ClaudeAccount[];
  busy: boolean;
  onUpdate: (patch: Partial<Preferences>) => Promise<boolean>;
  onAdd: (path: string) => Promise<boolean>;
};

function ProfileName({ account, disabled, onSave }: { account: ClaudeAccount; disabled: boolean; onSave: (name: string) => Promise<boolean> }) {
  const [name, setName] = useState(account.name);
  return <form className="claude-accounts__rename" onSubmit={(event) => { event.preventDefault(); if (name.trim()) void onSave(name.trim()); }}>
    <input aria-label={"Nom du compte " + account.name} value={name} disabled={disabled} onChange={(event) => setName(event.target.value)} />
    <button type="submit" disabled={disabled || !name.trim() || name.trim() === account.name}>Renommer</button>
  </form>;
}

export default function ClaudeAccountsSettings({ prefs, accounts, busy, onUpdate, onAdd }: Props) {
  const [path, setPath] = useState("");
  const [pathError, setPathError] = useState<string | null>(null);
  const settings: ClaudeAccountsPreferences = prefs.claudeAccounts ?? { mode: "single", selectedId: null, profiles: [] };
  const selected = accounts.find((account) => account.id === settings.selectedId) ?? accounts.find((account) => account.primary) ?? accounts[0];
  const multi = settings.mode === "multi";
  const saveProfile = (account: ClaudeAccount, patch: Partial<ClaudeProfile>) => {
    // Discovery metadata is not persisted until an explicit edit. Visibility
    // returned while Claude is off is false, not the user's stored choice.
    const stored = settings.profiles.find((profile) => profile.id === account.id);
    const profile: ClaudeProfile = { id: account.id, path: account.path, name: account.name, visible: stored?.visible ?? account.primary, ...patch };
    return onUpdate({ claudeAccounts: { ...settings, profiles: [...settings.profiles.filter((item) => item.id !== account.id), profile] } });
  };
  return <div className="claude-accounts">
    {accounts.length >= 2 && <>
      <div className="prefs-segmented" role="radiogroup" aria-label="Affichage des comptes Claude">
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
      <p className="prefs-footnote">L’affichage ne change pas la session Claude Code. Tous les comptes sont actualisés lorsque Claude est activé.</p>
    </>}
    <details className="prefs-disclosure">
      <summary>Comptes Claude{accounts.length ? ` (${accounts.length})` : ""}</summary>
      {accounts.map((account) => <div className="claude-accounts__profile" key={account.id}>
        <div className="claude-accounts__heading">
          <strong>{account.name}{account.primary && <small> · Principal</small>}</strong>
          {multi && <label className="prefs-switch"><input type="checkbox" role="switch" aria-label={"Afficher " + account.name}
            checked={settings.profiles.find((profile) => profile.id === account.id)?.visible ?? account.primary} disabled={busy}
            onChange={(event) => void saveProfile(account, { visible: event.target.checked })} /><span aria-hidden="true" /></label>}
        </div>
        <code>{account.path}</code>
        <p className="prefs-footnote">{!account.available ? "Dossier indisponible" : account.refreshing ? "Actualisation…" : account.snapshot.state === "ready" ? "Connecté" : account.snapshot.message || "En attente de données"}</p>
        <ProfileName key={account.id + account.name} account={account} disabled={busy} onSave={(name) => saveProfile(account, { name })} />
      </div>)}
      <form className="claude-accounts__add" onSubmit={async (event) => {
        event.preventDefault();
        if (busy) return;
        if (!/^(\/|[A-Za-z]:[\\/]|\\\\)/.test(path)) { setPathError("Indiquez un chemin absolu vers un dossier existant."); return; }
        setPathError(null);
        if (await onAdd(path)) setPath("");
      }}>
        <label>Chemin absolu du dossier de configuration Claude
          <input value={path} disabled={busy} placeholder="/Users/nom/.claude-travail" onChange={(event) => setPath(event.target.value)} />
        </label>
        <button type="submit" disabled={busy || !path}>Ajouter un compte</button>
        {pathError && <p role="alert">{pathError}</p>}
      </form>
    </details>
  </div>;
}
