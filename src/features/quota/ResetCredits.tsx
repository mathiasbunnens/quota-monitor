import { invoke } from "@tauri-apps/api/core";
import { useEffect, useId, useRef, useState } from "react";

// The primary account keeps its historical key (an interrupted request survives
// the upgrade); every other account has its own, so a pending request is never
// replayed on another account.
const legacyPendingKey = "codex-pending-reset";
const pendingKeyOf = (accountId: string, primary: boolean) => primary ? legacyPendingKey : `${legacyPendingKey}:${accountId}`;
// One reset at a time across all accounts: no double click, no concurrent spend.
let redeeming = false;
export const redeemLock = {
  acquire() { if (redeeming) return false; redeeming = true; return true; },
  release() { redeeming = false; },
};
const outcomes: Record<string, string> = {
  reset: "Réinitialisation utilisée.",
  alreadyRedeemed: "Cette réinitialisation a déjà été appliquée.",
  nothingToReset: "Aucune limite éligible à réinitialiser pour le moment.",
  noCredit: "Aucune réinitialisation disponible sur ce compte.",
};

export function ResetIcon() {
  return <svg className="reset-icon" viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
    <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5v3.2H9.8" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
  </svg>;
}

type Props = {
  /** Account the reset is spent on: always sent, never inferred. */
  accountId: string;
  accountName: string;
  primary?: boolean;
  /** Connected, followed, not a duplicate: otherwise the caller renders nothing. */
  ready: boolean;
  count: number | null | undefined;
  onChange: () => void;
};

export default function ResetCredits({ accountId, accountName, primary = false, ready, count, onChange }: Props) {
  const pendingKey = pendingKeyOf(accountId, primary);
  const titleId = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState(() => localStorage.getItem(pendingKey));
  const [message, setMessage] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [done, setDone] = useState(false);
  useEffect(() => { if (confirming) dialog.current?.showModal(); }, [confirming]);
  const redeem = async () => {
    if (!redeemLock.acquire()) return;
    setBusy(true); setMessage("");
    try {
      // Persist before sending so a lost reply or restart cannot spend a second reset.
      const key = pending || crypto.randomUUID();
      localStorage.setItem(pendingKey, key); setPending(key);
      const { outcome, quotasRefreshed } = await invoke<{ outcome: string; quotasRefreshed: boolean }>("redeem_codex_reset", { idempotencyKey: key, accountId });
      if (!outcomes[outcome]) throw new Error("Réponse inconnue.");
      localStorage.removeItem(pendingKey); setPending(null);
      setMessage(outcomes[outcome] + (quotasRefreshed
        ? " Quotas actualisés."
        : " Impossible d’actualiser les quotas. Utilise Actualiser lorsque la connexion est rétablie ; ne consomme pas une autre réinitialisation pour cette tentative."));
      // The outcome is shown in the dialog, which stays open.
      setDone(true);
      onChange();
    } catch {
      setMessage("Résultat non confirmé. Vérifie la connexion et la version de Codex, puis réessaie la même demande pour éviter de consommer deux réinitialisations.");
    } finally { redeemLock.release(); setBusy(false); }
  };
  const dialogView = confirming && <dialog className="settings-panel" ref={dialog} onCancel={(event) => { if (busy) event.preventDefault(); else { setConfirming(false); setDone(false); } }} aria-labelledby={titleId}>
    <h2 id={titleId}>{pending ? "Reprendre la réinitialisation ?" : "Utiliser une réinitialisation ?"}</h2>
    <p>Cette action consomme une réinitialisation disponible sur le compte Codex « {accountName} », si une limite est éligible. Aucun achat n’est effectué.</p>
    {pending && <p>La demande précédente sera réutilisée pour éviter une double consommation. Conserve le même compte Codex pour cette tentative.</p>}
    {message && <p role="status">{message}</p>}
    <footer>
      <button disabled={busy} onClick={() => { setConfirming(false); setDone(false); }}>{done ? "Fermer" : "Annuler"}</button>
      {!done && <button className="primary" disabled={busy || !ready} onClick={() => void redeem()}>{busy ? "En cours…" : "Confirmer"}</button>}
    </footer>
  </dialog>;
  const usable = ready && (!!pending || (count != null && count > 0));
  const shown = !ready || count == null ? "—" : String(count);
  return <>
    <button type="button" className="reset-chip" disabled={!usable || busy}
      onClick={() => { setMessage(""); setDone(false); setConfirming(true); }}
      aria-label={`Réinitialisations disponibles : ${shown} — ${accountName}`}
      title={usable ? "Utiliser une réinitialisation" : "Aucune réinitialisation disponible"}>
      <ResetIcon /><span>{shown}</span>
    </button>
    {dialogView}
  </>;
}
