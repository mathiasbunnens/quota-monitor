import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import type { QuotaRow } from "./providers";

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
  empty?: string;
  children?: ReactNode;
};

function level(remaining: number) {
  return remaining <= 20 ? "low" : remaining <= 40 ? "warning" : "healthy";
}

/** Model-specific rows (Fable) keep full opacity; a model tint and dot set them apart. */
function rowClass(row: QuotaRow) {
  return "provider-row" + (row.modelLabel ? " provider-row--model" : "");
}

function Row({ row, showReset = true }: { row: QuotaRow; showReset?: boolean }) {
  const known = row.remaining !== null;
  return <div className={rowClass(row)}>
    <div className="dashboard__quota-heading">
      <span>{row.modelLabel && <i className="provider-row__model-dot" aria-hidden="true" />}{row.label}</span>
      {known
        ? <strong>{row.exhausted ? "Épuisé" : <>{row.remaining} % <small>restants</small></>}</strong>
        : <strong className="provider-row__missing">{row.missing}</strong>}
    </div>
    <div className={"dashboard__progress dashboard__progress--" + (known ? level(row.remaining as number) : "low")}
      role="progressbar" aria-label={row.label} aria-valuemin={0} aria-valuemax={100}
      aria-valuenow={known ? (row.remaining as number) : undefined}
      aria-valuetext={known ? undefined : row.missing}>
      <span style={{ width: (known ? row.remaining : 0) + "%" }} />
    </div>
    {showReset && <p className="provider-row__reset">Réinitialisation : {row.resetText}</p>}
  </div>;
}

export default function ProviderCard({ name, badge, rows, status, error, compact, empty, children }: Props) {
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
    {shown.map((row) => <Row key={row.id} row={row} showReset={!compact} />)}
    {!rows.length && !error && <p className="provider-card__note">{empty ?? "Aucun quota à afficher."}</p>}
    {status && <p className="provider-card__note" role="status">{status}</p>}
    {hasDetails && open && <div id={detailsId} className="provider-card__details" role="region" aria-label={"Détails " + name}>
      {first && <p className="provider-row__reset">{first.label} · réinitialisation : {first.resetText}</p>}
      {rest.map((row) => <Row key={row.id} row={row} />)}
      {!rest.length && <p className="provider-card__note">Aucune autre limite communiquée.</p>}
    </div>}
    {children}
  </section>;
}
