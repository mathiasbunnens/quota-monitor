export type QuotaSource = "codex";
export type QuotaPeriod = string;
export type QuotaSnapshot = {
  model: string;
  remaining: number;
  limit: number;
  resetAt: string;
  checkedAt: string;
  source: QuotaSource;
  confidence: "low" | "medium" | "high";
  period: QuotaPeriod;
  label?: string | null;
  windowMinutes?: number | null;
};
export type CodexStatus = {
  phase: "starting" | "disabled" | "missing" | "signed_out" | "ready" | "error" | "logging_in";
  message: string;
  executable: string | null;
  lastChecked: string | null;
  authUrl: string | null;
  userCode: string | null;
  planType: string | null;
  activeInstances: number | null;
  refreshSeconds: number;
  availableResets: number | null;
  refreshSettings: { customSeconds: number | null };
};

export type IndicatorChoice = "codex" | "claude" | "both";
export type DisplayMode = "menubar" | "app";
/** Presentation of the visible Claude account(s), one or several (Réglages → Affichage). */
export type ClaudeLayout = "compact" | "classic";
export type NtfyConfig = { enabled: boolean; serverUrl: string; topic: string };
export type ClaudeProfile = { id: string; path: string; name: string; visible: boolean };
export type ClaudeAccountsPreferences = {
  mode: "single" | "multi";
  selectedId: string | null;
  profiles: ClaudeProfile[];
};
export type ClaudeAccount = ClaudeProfile & {
  primary: boolean;
  available: boolean;
  refreshing: boolean;
  snapshot: ClaudeSnapshot;
};
/** One Codex environment (CODEX_HOME folder) from `get_codex_accounts`.
 *  `visible` already applies Unique/Multi and the enable switch. Never a
 *  token: `duplicateOf` only names an earlier home returning the same account. */
export type CodexAccount = ClaudeProfile & {
  primary: boolean;
  /** Followed (collected). A detected `~/.codex-*` folder is listed only. */
  tracked: boolean;
  available: boolean;
  refreshing: boolean;
  phase: CodexStatus["phase"] | "detected" | "unavailable";
  message: string;
  planType: string | null;
  lastChecked: string | null;
  rows: QuotaSnapshot[];
  duplicateOf: string | null;
};
export type Preferences = {
  claudeAccounts: ClaudeAccountsPreferences;
  /** Codex homes followed besides the primary. Absent in older files: Unique, primary only. */
  codexAccounts?: ClaudeAccountsPreferences;
  codexEnabled: boolean;
  claudeEnabled: boolean;
  fableEnabled: boolean;
  indicator: IndicatorChoice;
  displayMode: DisplayMode;
  ntfy: NtfyConfig;
  /** Single Compacte/Classique choice for Claude and Codex. Absent in older
   *  files: see `layoutOf` (same migration as Rust). */
  layout?: ClaudeLayout;
  /** Legacy keys, kept in sync by Rust for older builds; read-only here. */
  claudeLayout?: ClaudeLayout;
  codexLayout?: ClaudeLayout;
};

export type ClaudeMetric = {
  id: string;
  label: string;
  usedPercent: number | null;
  resetAt: string | null;
  availability: "available" | "not_reported" | "unavailable";
  periodDurationSeconds: number;
  exhausted: boolean;
};
export type ClaudeSnapshot = {
  provider: "claude";
  schemaVersion: number;
  enabled: boolean;
  configured: boolean;
  state: "disabled" | "not_configured" | "waiting_for_data" | "ready" | "stale" | "error";
  source: { kind: string; documentationUrl: string } | null;
  observedAt: string | null;
  providerFetchedAt: string | null;
  message: string;
  errorKind: string | null;
  metrics: ClaudeMetric[];
  capability: { refresh: "event_only" | "on_demand" | "none" };
  planType: string | null;
  extraUsage: { enabled: boolean; limitReached: boolean } | null;
};
