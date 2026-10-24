// Pure layout of the menu-bar panel for the standalone window. No IPC here:
// CompactWindow supplies the model and the real handlers (tested with mocks).
import type { CompactFill, CompactPanelModel, CompactRow, CompactUpdate, GaugeColor } from "./types";

export type UpdateIndicator = {
  state: "idle" | "busy" | "available" | "ready" | "error";
  /** State sentence, announced and used as accessible description. */
  description: string;
};

/** Updater state as shown by the update icon (null = no check reported yet). */
export function updateIndicator(update: CompactUpdate | null): UpdateIndicator {
  const next = update?.nextVersion ? ` v${update.nextVersion}` : "";
  switch (update?.phase) {
    case "checking":
      return { state: "busy", description: "Recherche de mises à jour en cours…" };
    case "downloading": {
      const progress = typeof update.progress === "number" && Number.isFinite(update.progress)
        ? ` (${Math.max(0, Math.min(100, Math.floor(update.progress)))} %)` : "";
      return { state: "busy", description: `Téléchargement de Quota Monitor${next} en cours${progress}…` };
    }
    case "installing":
      return { state: "busy", description: `Installation de Quota Monitor${next} en cours…` };
    case "available":
      return { state: "available", description: `Mise à jour disponible : Quota Monitor${next}.` };
    case "ready":
      return { state: "ready", description: `Quota Monitor${next} est installé : relancez l’application.` };
    case "error":
      return { state: "error", description: "La dernière vérification des mises à jour a échoué." };
    case "up-to-date":
      return { state: "idle", description: "Quota Monitor est à jour." };
    default:
      return { state: "idle", description: "Aucune vérification en cours." };
  }
}

const colorVar = (color: GaugeColor, model: boolean) =>
  color === "green" && model ? "var(--compact-teal)" : `var(--compact-${color})`;

/** CSS colour of a gauge fill: same thresholds and blend as the native menu. */
export function fillColor(fill: CompactFill, model: boolean): string {
  const from = colorVar(fill.from, model);
  if (fill.from === fill.to || fill.fraction <= 0) return from;
  const to = colorVar(fill.to, model);
  const share = Math.round(Math.min(1, fill.fraction) * 1000) / 10;
  return `color-mix(in srgb, ${from} ${100 - share}%, ${to} ${share}%)`;
}

function RowLabel({ row }: { row: CompactRow }) {
  // Model-scoped rows colour only the model name with the exact fill colour.
  if (row.model && row.label.endsWith(" Fable")) {
    return <span className="compact-row__label">{row.label.slice(0, -" Fable".length)}{" "}
      <span style={{ color: fillColor(row.fill, true) }}>Fable</span></span>;
  }
  return <span className="compact-row__label">{row.label}</span>;
}

function GearIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  </svg>;
}

/** Same circular arrow as the updater window (UpdatePopup). */
function UpdateIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
    <path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v4.2h-4.2" />
  </svg>;
}

export type CompactPanelProps = {
  panel: CompactPanelModel | null;
  error: string | null;
  pending: "settings" | "update" | "connect" | null;
  onSettings: () => void;
  onUpdate: () => void;
  onConnect: () => void;
};

export function CompactPanel({ panel, error, pending, onSettings, onUpdate, onConnect }: CompactPanelProps) {
  const update = updateIndicator(panel?.update ?? null);
  const updateTitle = update.state === "idle"
    ? "Vérifier les mises à jour…"
    : `Vérifier les mises à jour… — ${update.description}`;
  const header = panel?.header;
  return <main className="compact" aria-labelledby="compact-title">
    <header className="compact__header">
      <div className="compact__titles">
        <h1 id="compact-title">{header?.title ?? "Quota Monitor"}</h1>
        <p>{header?.subtitle ?? "En attente des premières données"}</p>
      </div>
      {header && <span className={`compact__capsule compact-tone--${header.tone}`} role="status">
        <span className="compact-sr">État : </span>{header.status}</span>}
      <div className="compact__actions" role="group" aria-label="Actions de Quota Monitor">
        <button type="button" className="compact-icon" onClick={onSettings}
          aria-label="Réglages" title="Réglages… (⌘,)" aria-keyshortcuts="Meta+Comma"
          aria-disabled={pending === "settings" || undefined}>
          <GearIcon />
        </button>
        <button type="button" className={`compact-icon compact-icon--update is-${update.state}`} onClick={onUpdate}
          aria-label="Vérifier les mises à jour" title={updateTitle}
          aria-describedby="compact-update-state" aria-busy={update.state === "busy" || pending === "update"}
          aria-disabled={pending === "update" || undefined}>
          <UpdateIcon />
          {(update.state === "available" || update.state === "ready" || update.state === "error") &&
            <span className="compact-icon__badge" aria-hidden="true" />}
        </button>
        <span id="compact-update-state" className="compact-sr" aria-live="polite">
          {update.state === "idle" ? "" : update.description}
        </span>
      </div>
    </header>
    {error && <p className="compact__alert" role="alert">{error}</p>}
    {panel?.providers.map((provider) => <section key={provider.name} className="compact-provider"
      aria-label={provider.badge ? `${provider.name} (${provider.badge})` : provider.name}>
      <div className="compact-provider__head" aria-hidden="true">
        <span>{provider.section}</span>
        {provider.badge && <span className="compact-provider__badge">{provider.badge}</span>}
      </div>
      <div className="compact-card">
        {provider.rows.map((row, index) => <div key={`${row.label}-${index}`} className="compact-row">
          <div className="compact-row__line">
            <RowLabel row={row} />
            <span className={"compact-row__value" + (row.low ? " is-low" : "")}>{row.value}</span>
          </div>
          <div className="compact-row__track" role="meter" aria-label={`${row.label} : quota restant`}
            aria-valuemin={0} aria-valuemax={100} aria-valuenow={row.remaining ?? undefined}
            aria-valuetext={row.remaining === null ? row.value : `${row.value} restant`}>
            {row.fill.visible && <span className="compact-row__fill"
              style={{ width: `${row.fill.ratio * 100}%`, background: fillColor(row.fill, row.model) }} />}
          </div>
          <span className="compact-row__reset">{row.reset}</span>
        </div>)}
        {provider.note && <p className={`compact-card__note compact-tone--${provider.note.tone}`
          + (provider.rows.length ? " has-rows" : "")}>{provider.note.text}</p>}
      </div>
    </section>)}
    {panel?.connection && <div className="compact__connect">
      <button type="button" onClick={onConnect} disabled={panel.connection.loggingIn || pending === "connect"}>
        {panel.connection.label}
      </button>
    </div>}
    {panel && <p className="compact__version">Quota Monitor v{panel.version}</p>}
  </main>;
}
