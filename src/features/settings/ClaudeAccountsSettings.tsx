import { useEffect, useRef, useState, type ReactNode } from "react";
import type { ClaudeAccount, ClaudeAccountsPreferences, ClaudeProfile, CodexAccount, Preferences } from "../quota/types";
import { accountState, codexAccountState, type Tone } from "../quota/accountState";
import AddAccountFlow, { ABSOLUTE_PATH, ABSOLUTE_PATH_ERROR } from "../accounts/AddAccountFlow";
import "./claude-accounts.scss";


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
  /** Codex reuse: provider wording, preference key and per-account state. */
  provider?: "Claude" | "Codex";
  stateOf?: (account: A) => [string, Tone, string];
  planOf?: (account: A) => string | null | undefined;
  /** Per-account control shown before the connection state (reset, extra usage). */
  accessory?: (account: A) => ReactNode;
  /** Pointed at from the compact window: open the add flow. `nonce` re-arms a repeated request. */
  focus?: SettingsFocusRequest | null;
};

/** Where the Settings window was asked to land (see `settings_focus.rs`). */
export type SettingsFocusRequest = { accountId: string | null; add: boolean; nonce: number };

export { accountState, codexAccountState };

const pencil = <path d="M10.5 2.8 13.2 5.5 6 12.7 3 13l.3-3 7.2-7.2Z" />;
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

export default function ClaudeAccountsSettings<A extends ListedAccount = ClaudeAccount>({ prefs, accounts, busy, onUpdate, onAdd, onChooseFolder, actionError = null,
  provider = "Claude", stateOf, planOf, accessory, focus = null }: Props<A>) {
  const [path, setPath] = useState("");
  const [pathError, setPathError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const pathInput = useRef<HTMLInputElement>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const codex = provider === "Codex";
  const key = codex ? "codexAccounts" : "claudeAccounts";
  const slug = codex ? "codex" : "claude";
  // Visibility labels stay unique when both providers list a « Principal ».
  const show = (account: A) => "Afficher " + (codex ? "Codex " : "") + account.name;
  const state = stateOf ?? ((account: A) => accountState(account as unknown as ClaudeAccount));
  const plan = planOf ?? ((account: A) => (account as unknown as ClaudeAccount).snapshot?.planType);
  const settings: ClaudeAccountsPreferences = prefs[key] ?? { mode: "single", selectedId: null, profiles: [] };
  // Detected folders are followed automatically; `tracked` only guards an older backend answer.
  const listed = accounts.filter((account) => account.tracked !== false);
  const selected = listed.find((account) => account.id === settings.selectedId) ?? listed.find((account) => account.primary) ?? listed[0];
  const multi = settings.mode === "multi";
  const several = listed.length >= 2;
  // Stored choice, not the effective view (which is false while Claude is off).
  // An account added automatically (no saved entry yet) is shown in Multi.
  const defaultVisible = (account: A) => account.primary || multi;
  const storedVisible = (account: A) => settings.profiles.find((profile) => profile.id === account.id)?.visible ?? defaultVisible(account);
  const saveProfile = (account: A, patch: Partial<ClaudeProfile>) => {
    // Discovery metadata is not persisted until an explicit edit. Visibility
    // returned while Claude is off is false, not the user's stored choice.
    const stored = settings.profiles.find((profile) => profile.id === account.id);
    const profile: ClaudeProfile = { id: account.id, path: account.path, name: account.name, visible: stored?.visible ?? defaultVisible(account), ...patch };
    return onUpdate({ [key]: { ...settings, profiles: [...settings.profiles.filter((item) => item.id !== account.id), profile] } });
  };
  // The + opens the single add flow: folder chooser (macOS) and absolute path, in place.
  const add = () => {
    if (busy) return;
    setOpen(!open);
    setPathError(null);
    window.requestAnimationFrame(() => pathInput.current?.focus());
  };
  const close = () => { setOpen(false); setPathError(null); window.requestAnimationFrame(() => addButton.current?.focus()); };
  const choose = async () => {
    if (busy || !onChooseFolder) return;
    const chosen = await onChooseFolder();
    if (chosen && await onAdd(chosen)) setOpen(false);
  };
  const submitPath = async () => {
    if (busy) return;
    if (!ABSOLUTE_PATH.test(path)) { setPathError(ABSOLUTE_PATH_ERROR); return; }
    setPathError(null);
    if (await onAdd(path)) { setPath(""); setOpen(false); }
  };

  // Applied once per request; a request that arrives before the list has loaded waits for it.
  const applied = useRef<number | null>(null);
  useEffect(() => {
    if (!focus || applied.current === focus.nonce) return;
    if (focus.add) {
      setOpen(true);
      window.requestAnimationFrame(() => pathInput.current?.focus());
    } else return;
    applied.current = focus.nonce;
    root.current?.scrollIntoView?.({ block: "nearest" });
  }, [focus?.nonce, listed.length]);

  return <div className="claude-accounts" ref={root} aria-busy={busy || undefined}>
    {several && <div className="prefs-segmented" role="radiogroup" aria-label={"Comptes " + provider + " affichés"}>
      {([["single", "Unique"], ["multi", "Multi"]] as const).map(([mode, label]) => <label key={mode} className={settings.mode === mode ? "is-selected" : ""}>
        <input type="radio" name={slug + "-mode"} checked={settings.mode === mode}
          onChange={() => void onUpdate({ [key]: { ...settings, mode } })} />{label}
      </label>)}
    </div>}

    {accounts.length > 0 && <ul className="claude-accounts__list" aria-label={"Comptes " + provider}>
      {accounts.map((account) => {
        const [pill, tone, detail] = state(account);
        const tracked = account.tracked !== false;
        const planType = plan(account);
        return <li className="claude-accounts__row" key={account.id}>
          <div className="claude-accounts__who">
            <AccountName key={account.id} account={account} disabled={busy || !tracked} onSave={(name) => saveProfile(account, { name })} />
            {account.primary && account.name !== "Principal" && <small>Principal</small>}
            {planType && <span className="prefs-plan">{planType}</span>}
          </div>
          {accessory?.(account)}
          <span className={"prefs-pill prefs-pill--" + tone} title={detail || undefined}>{pill}</span>
          {(account as { phase?: string }).phase === "blocked" ? null : multi
            ? <label className="prefs-switch"><input type="checkbox" role="switch" aria-label={show(account)}
              checked={storedVisible(account)}
              onChange={(event) => void saveProfile(account, { visible: event.target.checked })} /><span aria-hidden="true" /></label>
            : several && <label className="claude-accounts__pick"><input type="radio" name={slug + "-selected"} aria-label={show(account)}
              checked={account.id === selected?.id}
              onChange={() => void onUpdate({ [key]: { ...settings, selectedId: account.id } })} /><span aria-hidden="true" /></label>}
        </li>;
      })}
    </ul>}

    <div className="claude-accounts__actions">
      <button type="button" ref={addButton} className="claude-accounts__icon claude-accounts__add" disabled={busy}
        aria-label={"Ajouter un compte " + provider} title={"Ajouter un compte " + provider} aria-expanded={open} onClick={add}>
        <Glyph>{plus}</Glyph>
      </button>
    </div>
    {actionError && <p role="alert" className="prefs-state prefs-state--error">{actionError}</p>}
    {open && <AddAccountFlow provider={provider} path={path} error={pathError} busy={busy} canChoose={!!onChooseFolder} inputRef={pathInput}
      onPath={setPath} onChoose={() => void choose()} onSubmit={() => void submitPath()} onClose={close} />}
  </div>;
}

type CodexProps = Omit<Props<CodexAccount>, "provider" | "stateOf" | "planOf">;
/** Codex homes: the same list, wording adapted, `codexAccounts` preferences. */
export function CodexAccountsSettings(props: CodexProps) {
  return <ClaudeAccountsSettings<CodexAccount> {...props} provider="Codex" stateOf={codexAccountState} planOf={(account) => account.planType} />;
}
