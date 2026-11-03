import { useEffect, useId, useRef, useState, type ReactNode } from "react";
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
  name: string;
  badge?: string | null;
  rows: QuotaRow[];
  /** Neutral status line (waiting, stale…). */
  status?: string | null;
  /** Visible error of an enabled account. */
  error?: string | null;
  /** Two providers side by side: short quota only, the rest on hover/click/keyboard. */
  compact: boolean;
  /** Multi-profile layout: keep both primary windows visible. */
  accountCompact?: boolean;
  /** Local clock (ms) for the visible countdowns of multi-profile cards. */
  now?: number;
  empty?: string;
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
function PairedProgress({ row, className }: { row: QuotaRow; className: string }) {
  const known = row.remaining !== null;
  const state = known ? level(row.remaining as number) : "low";
  return <div className={"dashboard__progress dashboard__progress--" + state + " " + className
    + (row.modelLabel ? " provider-row--model provider-row--" + state : "")}
    title={row.label + " · réinitialisation : " + row.resetText}
    role="progressbar" aria-label={row.label} aria-valuemin={0} aria-valuemax={100}
    aria-valuenow={known ? (row.remaining as number) : undefined}
    aria-valuetext={known ? undefined : row.missing}>
    <span style={{ width: (known ? row.remaining : 0) + "%" }} />
  </div>;
}

/** 5 h row and the weekly row (or the weekly window coalesced into Fable). */
function pairedRows(rows: QuotaRow[]) {
  const short = rows.find((row) => row.pairSlot === 0 || row.id === "claude:five_hour" || row.id === "five_hour");
  const week = rows.find((row) => row.pairSlot === 1 || row.id === "claude:seven_day" || row.id === "seven_day")
    ?? rows.find((row, index) => index === 1 && row.id === "claude:weekly_fable");
  return { short, week, others: rows.filter((row) => row !== short && row !== week) };
}

/** Model-specific rows keep normal typography; only the model name gets the bar colour. */
function rowClass(row: QuotaRow) {
  return "provider-row" + (row.modelLabel ? " provider-row--model" : "");
}

function Row({ row, showReset = true }: { row: QuotaRow; showReset?: boolean }) {
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
    {showReset && <p className="provider-row__reset">Réinitialisation : {row.resetText}</p>}
  </div>;
}

export default function ProviderCard({ name, badge, rows, status, error, compact, accountCompact = false, now = Date.now(), empty, children }: Props) {
  const detailsId = useId();
  const root = useRef<HTMLElement>(null);
  const [hover, setHover] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [focused, setFocused] = useState(false);
  const open = compact && (hover || pinned || focused);

  useEffect(() => {
    if (!pinned) return;
    const outside = (event: PointerEvent) => {
      if (root.current && !root.current.contains(event.target as Node)) setPinned(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [pinned]);

  const [first, ...rest] = rows;
  const paired = pairedRows(rows);
  const shown = compact ? (first ? [first] : []) : rows;
  const hasDetails = compact && rows.length > 0;
  return <section ref={root} className={"provider-card" + (compact ? " provider-card--compact" : "")}
    aria-label={"Quotas " + name}
    onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
    onFocus={() => setFocused(true)}
    onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false); }}
    onKeyDown={(event) => { if (event.key === "Escape") { setPinned(false); setHover(false); setFocused(false); } }}>
    <header className="provider-card__header">
      <h2>{name}{badge && <span className="plan-badge">{badge}</span>}</h2>
      {hasDetails && <button type="button" aria-expanded={open} aria-controls={detailsId}
        onClick={() => setPinned((value) => !value)}>Détails</button>}
    </header>
    {error && <p role="alert" className="provider-card__error">{error}</p>}
    {accountCompact ? <>
      <div className="provider-card__paired-values">
        {[paired.short, paired.week].map((row) => row && <PairedHeading key={row.id} row={row} now={now} />)}
      </div>
      <div className="provider-card__paired-bars">
        {paired.short && <PairedProgress row={paired.short} className="provider-card__window--short" />}
        {paired.week && <PairedProgress row={paired.week} className="provider-card__window--week" />}
      </div>
      {/* Fable and any other limit stay visible: nothing is dropped to fit. */}
      {paired.others.map((row) => <Row key={row.id} row={row} />)}
    </> : shown.map((row) => <Row key={row.id} row={row} showReset={!compact} />)}
    {!rows.length && !error && <p className="provider-card__note">{empty ?? "Aucun quota à afficher."}</p>}
    {status && <p className="provider-card__note" role="status">{status}</p>}
    {hasDetails && open && <div id={detailsId} className="provider-card__details" role="region" aria-label={"Détails " + name}>
      {!accountCompact && first && <p className="provider-row__reset">{first.label} · réinitialisation : {first.resetText}</p>}
      {(accountCompact ? rows : rest).map((row) => <Row key={row.id} row={row} />)}
      {!accountCompact && !rest.length && <p className="provider-card__note">Aucune autre limite communiquée.</p>}
    </div>}
    {children}
  </section>;
}
