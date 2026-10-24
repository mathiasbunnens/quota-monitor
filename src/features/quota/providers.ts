import type { ClaudeSnapshot, QuotaSnapshot } from "./types";

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
  label: string;
  modelLabel?: string;
  /** Remaining percentage, or null when the provider did not report it. */
  remaining: number | null;
  exhausted: boolean;
  /** Human text shown instead of a percentage when remaining is null. */
  missing: string;
  resetText: string;
};

export function codexRows(snapshots: QuotaSnapshot[]): QuotaRow[] {
  return snapshots.map((snapshot) => {
    const remaining = Math.round(quotaPercentage(snapshot));
    return {
      id: snapshot.period, label: quotaLabel(snapshot), remaining, exhausted: remaining <= 0,
      missing: "", resetText: formatResetDate(snapshot.resetAt),
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
