import type { ClaudeAccount, ClaudeSnapshot, CodexAccount, CodexStatus } from "./types";

export type Tone = "good" | "warning" | "error" | "neutral";
/** [label shown, tone, actionable detail] — one source for Settings rows and Dashboard cards. */
export type AccountState = [string, Tone, string];

/** Connection state of a Claude profile from its snapshot. */
export function claudeSnapshotState(s: ClaudeSnapshot, available = true, refreshing = false): AccountState {
  if (!available) return ["Indisponible", "neutral", "Dossier introuvable : vérifiez qu’il existe encore (disque, renommage)."];
  if (refreshing) return ["Actualisation…", "warning", ""];
  if (s.state === "disabled") return ["Désactivé", "neutral", ""];
  if (s.errorKind === "not_signed_in") return ["Non connecté", "warning", s.message || "Profil non connecté dans Claude Code."];
  // aip snapshot whose saved token no longer works: actionable, never a quota.
  if (s.errorKind === "session_expired") return ["Session expirée", "warning", s.message || "Session du profil aip expirée : réactivez-le dans aip pour la renouveler."];
  if (s.errorKind === "profile_blocked") return ["Non collectable", "neutral", s.message || "Profil aip incomplet, rien à lire."];
  switch (s.state) {
    case "ready": return ["Connecté", "good", ""];
    case "stale": return ["Connecté", "warning", "Dernières valeurs conservées, nouvelle lecture en attente."];
    case "waiting_for_data": return ["Lecture…", "warning", ""];
    case "not_configured": return ["Non installé", "neutral", s.message || "Claude Code introuvable."];
    default: return ["Erreur", "error", s.message || "Lecture impossible : vérifiez Claude Code."];
  }
}

export function accountState(account: ClaudeAccount): AccountState {
  return claudeSnapshotState(account.snapshot, account.available, account.refreshing);
}

/** State of the primary Codex collector (no account list: older backend). */
export function codexPhaseState(phase: CodexStatus["phase"] | "unavailable" | "blocked" | "expired" | undefined, message = ""): AccountState {
  switch (phase) {
    case "ready": return ["Connecté", "good", ""];
    case "signed_out": return ["Non connecté", "warning", message || "Dossier non connecté à Codex."];
    case "disabled": return ["Désactivé", "neutral", ""];
    case "missing": return ["Non installé", "neutral", message];
    case "error": return ["Erreur", "error", message || "Lecture impossible : vérifiez Codex CLI."];
    case "expired": return ["Session expirée", "warning", message || "Session du profil aip expirée : relancez-le avec aip codex pour la renouveler."];
    case "blocked": return ["Non collectable", "neutral", message || "Profil aip incomplet, rien à lire."];
    default: return ["Connexion…", "warning", ""];
  }
}

/** Connection state of one Codex home. */
export function codexAccountState(account: CodexAccount): AccountState {
  if (!account.available) return ["Indisponible", "neutral", "Dossier CODEX_HOME introuvable : vérifiez qu’il existe encore (disque, renommage)."];
  if (account.refreshing) return ["Actualisation…", "warning", ""];
  if (account.duplicateOf && account.phase === "ready") return ["Même compte", "warning", "Même compte que « " + account.duplicateOf + " » : quotas non additionnés."];
  return codexPhaseState(account.phase, account.message);
}
