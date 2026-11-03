import type { ClaudeAccount, ClaudeLayout, ClaudeSnapshot, Preferences, QuotaSnapshot } from "./types";

/** Same rule as Rust `prefs::parse`: an explicit value wins (unknown =
 *  compact); absent (older file) keeps that file's look — paired bars for a
 *  previous Multi build, otherwise the historical card. */
export function claudeLayoutOf(prefs: { claudeLayout?: string; claudeAccounts?: { mode?: string } } | null | undefined): ClaudeLayout {
  if (prefs?.claudeLayout === undefined) return prefs?.claudeAccounts?.mode === "multi" ? "compact" : "classic";
  return prefs.claudeLayout === "classic" ? "classic" : "compact";
}

export function codexLayoutOf(prefs: { codexLayout?: string } | null | undefined): ClaudeLayout {
  return prefs?.codexLayout === undefined || prefs.codexLayout === "classic" ? "classic" : "compact";
}

export type ClaudeCard = { key: string; name: string; snapshot: ClaudeSnapshot | null; account: ClaudeAccount | null };
/** Cards to draw and how, shared by every React view (mirrors the native
 *  `claude_account_panels`). The layout applies to one or several cards;
 *  only names/notes depend on the count. */
export type ClaudePlan = { cards: ClaudeCard[]; paired: boolean; named: boolean; emptyMulti: boolean };

export function claudePlan(prefs: Preferences | null, accounts: ClaudeAccount[], fallback: ClaudeSnapshot | null): ClaudePlan {
  const paired = claudeLayoutOf(prefs) === "compact";
  if (!prefs?.claudeEnabled) return { cards: [], paired, named: false, emptyMulti: false };
  const multiMode = prefs.claudeAccounts?.mode === "multi";
  const selected = accounts.find((account) => account.id === prefs.claudeAccounts?.selectedId)
    ?? accounts.find((account) => account.primary) ?? accounts[0];
  const shown = multiMode ? accounts.filter((account) => account.visible) : selected ? [selected] : [];
  const named = shown.length > 1;
  if (multiMode && !shown.length) return { cards: [], paired, named, emptyMulti: true };
  const cards: ClaudeCard[] = shown.length
    ? shown.map((account) => ({ key: account.id, name: named ? "Claude · " + account.name : "Claude", snapshot: account.snapshot, account }))
    : [{ key: "claude", name: "Claude", snapshot: fallback, account: null }];
  return { cards, paired, named, emptyMulti: false };
}

export function quotaPercentage(snapshot: QuotaSnapshot) {
  if (snapshot.limit <= 0) return 0;
  return Math.max(0, Math.min(100, (snapshot.remaining / snapshot.limit) * 100));
}

export function formatResetDate(value: string) {
  const date = new Date(value);
  if (!value || Number.isNaN(date.getTime())) return "non communiquée";
  return new Intl.DateTimeFormat("fr-FR", {
    weekday: "short",
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export function quotaLabel(snapshot: QuotaSnapshot) {
  return snapshot.label || ({
    "five-hour": "Limite 5 heures",
    weekly: "Limite hebdomadaire",
    "reserve-weekly": "Réserve Luna",
  } as Record<string, string>)[snapshot.period] || snapshot.period;
}

export type QuotaRow = {
  id: string;
  /** Slot in the paired block, only for a real window of the selected bucket. */
  pairSlot?: 0 | 1;
  label: string;
  modelLabel?: string;
  /** Remaining percentage, or null when the provider did not report it. */
  remaining: number | null;
  exhausted: boolean;
  /** Human text shown instead of a percentage when remaining is null. */
  missing: string;
  resetText: string;
  /** Structured reset (RFC 3339) for local countdowns, without collection. */
  resetAt?: string | null;
};

export function codexRows(snapshots: QuotaSnapshot[]): QuotaRow[] {
  const window = (s: QuotaSnapshot): { bucket: string; slot: 0 | 1 } | null => {
    if (/reserve|luna|base_model_inference/i.test(s.period)) return null;
    if (s.period === "five-hour" || s.period === "weekly") return { bucket: "legacy", slot: s.period === "five-hour" ? 0 : 1 };
    const match = /^(codex:.+):(primary|secondary)$/.exec(s.period);
    if (!match || (s.windowMinutes !== 300 && s.windowMinutes !== 10080)) return null;
    return { bucket: match[1] + ":" + s.model, slot: s.windowMinutes === 300 ? 0 : 1 };
  };
  const windows = snapshots.map(window);
  const bucket = windows.find((w) => w?.bucket.startsWith("codex:codex:"))?.bucket
    ?? windows.find((w) => w !== null)?.bucket;
  const selected = new Set<number>();
  for (const slot of [0, 1]) {
    const index = windows.findIndex((w) => w?.bucket === bucket && w?.slot === slot);
    if (index >= 0) selected.add(index);
  }
  return snapshots.map((snapshot, index) => {
    const remaining = Math.round(quotaPercentage(snapshot));
    return {
      id: snapshot.period, label: quotaLabel(snapshot), remaining, exhausted: remaining <= 0,
      ...(selected.has(index) ? { pairSlot: windows[index]!.slot } : {}),
      missing: "", resetText: formatResetDate(snapshot.resetAt), resetAt: snapshot.resetAt,
    };
  });
}

export function claudeRows(snapshot: ClaudeSnapshot, fableEnabled = true): QuotaRow[] {
  const metrics = snapshot.metrics.filter((metric) => fableEnabled || metric.id !== "claude:weekly_fable");
  const fable = metrics.find((metric) => metric.id === "claude:weekly_fable");
  // Compare raw values and a known reset, never rounded percentages or absent data.
  const duplicate = fable?.availability === "available" && fable.usedPercent !== null && fable.resetAt !== null
    && metrics.some((metric) => metric.id === "claude:seven_day" && metric.availability === "available"
      && metric.usedPercent === fable.usedPercent && metric.resetAt === fable.resetAt
      && metric.periodDurationSeconds === fable.periodDurationSeconds);
  const displayed = duplicate ? metrics.flatMap((metric) => metric.id === "claude:seven_day" ? [fable!]
    : metric.id === "claude:weekly_fable" ? [] : [metric]) : metrics;
  return displayed.map((metric) => {
    const available = metric.availability === "available" && metric.usedPercent !== null;
    return {
      id: metric.id, label: metric.id === "claude:weekly_fable" ? "Limite Fable" : metric.label,
      ...(metric.id === "claude:weekly_fable" ? { modelLabel: "Fable" } : {}),
      remaining: available ? Math.round(Math.max(0, Math.min(100, 100 - (metric.usedPercent as number)))) : null,
      exhausted: available && metric.exhausted,
      missing: metric.availability === "unavailable" ? "Réinitialisée · en attente" : "Non communiquée",
      resetText: metric.resetAt ? formatResetDate(metric.resetAt) : "non communiquée",
      resetAt: metric.resetAt,
    };
  });
}

export function claudeStatusText(snapshot: ClaudeSnapshot): string | null {
  switch (snapshot.state) {
    case "not_configured": return snapshot.message || "Claude Code introuvable.";
    case "waiting_for_data": return "En attente de Claude Code : les limites apparaissent après la première réponse d’une session.";
    case "stale": return snapshot.observedAt
      ? "Dernière observation : " + new Date(snapshot.observedAt).toLocaleTimeString("fr-FR") + ". Claude Code n’a rien envoyé récemment."
      : "Données anciennes.";
    case "error": return snapshot.message || "Les données de Claude Code sont illisibles.";
    default: return null;
  }
}
