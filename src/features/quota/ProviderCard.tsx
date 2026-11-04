import type { ReactNode } from "react";
import type { QuotaRow } from "./providers";

/** Same rounded-up minute format as the native remaining_span helper.
 * Kept here (no value import) so the card stays a self-contained module. */
export function resetCountdown(resetAt: string | null | undefined, now: number): string {
  const left = resetAt ? Date.parse(resetAt) - now : NaN;
  if (!Number.isFinite(left) || left <= 0) return "—";
  const minutes = Math.ceil(left / 60000);
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor(minutes % 1440 / 60);
  const mins = minutes % 60;
  if (days) return `${days}j${hours ? ` ${hours}h` : ""}`;
  if (hours) return `${hours}h${String(mins).padStart(2, "0")}`;
  return `${mins} min`;
}

type Props = {
  /** Account name (the provider title lives in the group header). */
  name: string;
  /** Provider name, only for the accessible name of the card (« Quotas Codex · Travail »). */
  provider?: string;
  badge?: string | null;
  rows: QuotaRow[];
  /** Neutral status line (waiting, stale…). */
  status?: string | null;
  /** Visible error of an enabled account. */
  error?: string | null;
  /** Compacte presentation: the 5 h and 7 j windows paired (countdown above, reset below). */
  accountCompact?: boolean;
  /** Local clock (ms) for the visible countdowns of multi-profile cards. */
  now?: number;
  empty?: string;
  /** Connection state of this account (Connecté / Non connecté / Même compte…). */
  state?: { label: string; tone: "good" | "warning" | "error" | "neutral"; detail?: string };
  /** Account-level control of the header (reset chip), before the state. */
  action?: ReactNode;
  children?: ReactNode;
};

function level(remaining: number) {
  return remaining <= 20 ? "low" : remaining <= 40 ? "warning" : "healthy";
}

/** Multi-profile heading: live countdown (window name in title/aria) + value. */
function PairedHeading({ row, now }: { row: QuotaRow; now: number }) {
  const left = resetCountdown(row.resetAt, now);
  const label = row.modelLabel ? row.modelLabel + " · " + left : left;
  return <div className="dashboard__quota-heading">
    <span title={row.label + " · réinitialisation : " + row.resetText}
      aria-label={row.label + " · temps restant : " + left}>{label}</span>
    {row.remaining !== null
      ? <strong>{row.exhausted ? "Épuisé" : <>{row.remaining} % <small>restants</small></>}</strong>
      : <strong className="provider-row__missing">{row.missing}</strong>}
  </div>;
}

/** Same progressbar markup/classes as the Unique row; height comes from CSS. */
function PairedProgress({ row, className, slot }: { row: QuotaRow; className: string; slot: 1 | 2 }) {
  const known = row.remaining !== null;
  const state = known ? level(row.remaining as number) : "low";
  return <div className={"dashboard__progress dashboard__progress--" + state + " " + className
    + (row.modelLabel ? " provider-row--model provider-row--" + state : "")}
    style={{ gridRow: slot }}
    title={row.label + " · réinitialisation : " + row.resetText}
    role="progressbar" aria-label={row.label} aria-valuemin={0} aria-valuemax={100}
    aria-valuenow={known ? (row.remaining as number) : undefined}
    aria-valuetext={known ? undefined : row.missing}>
    <span style={{ width: (known ? row.remaining : 0) + "%" }} />
  </div>;
}

/** Short reset of a paired window: « Réinit. 03/11 19:00 » (local time, from the structured reset);
 *  without a usable reset the same wording as the Classique line (« non communiquée »). */
export function pairedResetLabel(row: QuotaRow): string {
  const at = row.resetAt ? new Date(row.resetAt) : null;
  if (!at || Number.isNaN(at.getTime())) return "Réinit. " + row.resetText;
  const part = (options: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat("fr-FR", options).format(at);
  return "Réinit. " + part({ day: "2-digit", month: "2-digit" }) + " " + part({ hour: "2-digit", minute: "2-digit" });
}

/** Reset of each paired window, under its own bar column (5 h left, 7 j right):
 *  the same information as the « Réinitialisation » line of a Classique row. */
function PairedResets({ short, week }: { short?: QuotaRow; week?: QuotaRow }) {
  const line = (row?: QuotaRow) => row ? pairedResetLabel(row) : "";
  return <div className="provider-card__paired-resets">
    <span title={short ? short.label + " · réinitialisation : " + short.resetText : undefined}>{line(short)}</span>
    <span title={week ? week.label + " · réinitialisation : " + week.resetText : undefined}>{line(week)}</span>
  </div>;
}

/** 5 h row and the weekly row. Fable (and any other limit) is never a window of the pair. */
function pairedRows(rows: QuotaRow[]) {
  const short = rows.find((row) => row.pairSlot === 0 || row.id === "claude:five_hour" || row.id === "five_hour");
  const week = rows.find((row) => row.pairSlot === 1 || row.id === "claude:seven_day" || row.id === "seven_day");
  return { short, week, others: rows.filter((row) => row !== short && row !== week) };
}

/** Model-specific rows keep normal typography; only the model name gets the bar colour. */
function rowClass(row: QuotaRow) {
  return "provider-row" + (row.modelLabel ? " provider-row--model" : "");
}

function Row({ row }: { row: QuotaRow }) {
  const known = row.remaining !== null;
  const state = known ? level(row.remaining as number) : "low";
  const modelSuffix = row.modelLabel && row.label.endsWith(" " + row.modelLabel) ? row.modelLabel : null;
  return <div className={rowClass(row) + " provider-row--" + state}>
    <div className="dashboard__quota-heading">
      <span>{modelSuffix ? <>{row.label.slice(0, -modelSuffix.length)}<span className="provider-row__model-label">{modelSuffix}</span></> : row.label}</span>
      {known
        ? <strong>{row.exhausted ? "Épuisé" : <>{row.remaining} % <small>restants</small></>}</strong>
        : <strong className="provider-row__missing">{row.missing}</strong>}
    </div>
    <div className={"dashboard__progress dashboard__progress--" + state}
      role="progressbar" aria-label={row.label} aria-valuemin={0} aria-valuemax={100}
      aria-valuenow={known ? (row.remaining as number) : undefined}
      aria-valuetext={known ? undefined : row.missing}>
      <span style={{ width: (known ? row.remaining : 0) + "%" }} />
    </div>
    <p className="provider-row__reset">Réinitialisation : {row.resetText}</p>
  </div>;
}

export default function ProviderCard({ name, provider, badge, rows, status, error, accountCompact = false, now = Date.now(), empty, state, action, children }: Props) {
  const paired = pairedRows(rows);
  const title = <h3 className="provider-card__title"><span className="provider-card__name">{name}</span>{badge && <span className="plan-badge">{badge}</span>}</h3>;
  return <section className="provider-card" aria-label={"Quotas " + (provider ? provider + " · " : "") + name}>
    <header className="provider-card__header">
      {action
        ? <div className="provider-card__heading">{title}
          <span className="provider-card__action">{action}</span>
        </div>
        : title}
      {state && <div className="provider-card__status">
        <span className={"provider-card__pill provider-card__pill--" + state.tone} title={state.detail || undefined}>{state.label}</span>
      </div>}
    </header>
    {error && <p role="alert" className="provider-card__error">{error}</p>}
    {accountCompact ? <>
      <div className="provider-card__paired-values">
        {[paired.short, paired.week].map((row) => row && <PairedHeading key={row.id} row={row} now={now} />)}
      </div>
      <div className="provider-card__paired-bars">
        {paired.short && <PairedProgress row={paired.short} className="provider-card__window--short" slot={1} />}
        {paired.week && <PairedProgress row={paired.week} className="provider-card__window--week" slot={2} />}
      </div>
      {(paired.short || paired.week) && <PairedResets short={paired.short} week={paired.week} />}
      {/* Fable and any other limit stay visible: nothing is dropped to fit. */}
      {paired.others.map((row) => <Row key={row.id} row={row} />)}
    </> : rows.map((row) => <Row key={row.id} row={row} />)}
    {!rows.length && !error && <p className="provider-card__note">{empty ?? "Aucun quota à afficher."}</p>}
    {status && <p className="provider-card__note" role="status">{status}</p>}
    {children}
  </section>;
}
