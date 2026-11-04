// Pure layout of the menu-bar panel for the standalone window. No IPC here:
// CompactWindow supplies the model and the real handlers (tested with mocks).
import type { ReactNode } from "react";
import type { CompactFill, CompactPanelModel, CompactProvider, CompactRow, CompactUpdate, GaugeColor, ProviderKind } from "./types";

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

/** Same text as the native compact_heading: countdown · value (+ Fable name). */
export function pairHeading(row: CompactRow): string {
  const text = row.countdown && row.countdown !== "—" ? `${row.countdown} · ${row.value}` : row.value;
  return row.model ? `Fable · ${text}` : text;
}

/** Same short reset line as the native paired block: « Réinit. à 14h30 ». */
export function pairReset(row: CompactRow): string {
  return row.reset.replace("Réinitialisation", "Réinit.");
}

/** Compacte block: 5 h + 7 j and an optional third fine Fable track. */
function Pair({ rows, fable }: { rows: CompactRow[]; fable?: CompactRow }) {
  const week = rows.find((row, index) => (row.pairSlot ?? index) === 1);
  const windows = ["Limite 5 heures", "Limite hebdomadaire"];
  return <div className={"compact-pair" + (fable ? " has-fable" : "")}>
    <div className="compact-pair__values">
      {rows.map((row, index) => <span key={index} className={row.low ? "is-low" : undefined}
        style={{ marginLeft: row.pairSlot === 1 ? "auto" : undefined }}
        title={`${windows[row.pairSlot ?? index]} · ${row.label} · ${row.reset}`}>{pairHeading(row)}</span>)}
    </div>
    <div className="compact-pair__bars">
      {rows.map((row, index) => {
        const slot = row.pairSlot ?? index;
        const week = slot === 1;
        // A fill narrower than its own track height only shows the track.
        const visible = row.fill.ratio * 276 >= (week ? 3 : 5);
        return <div key={index} className={"compact-row__track compact-pair__track--" + (week ? "week" : "short")}
          style={{ gridRow: slot + 1 }}
          role="meter" aria-label={`${windows[slot]} : ${row.label} · ${row.remaining === null ? row.value : `${row.value} restant`}`}
          aria-valuemin={0} aria-valuemax={100} aria-valuenow={row.remaining ?? undefined}
          aria-valuetext={row.remaining === null ? row.value : `${row.value} restant`}>
          {visible && <span className="compact-row__fill"
            style={{ width: `${row.fill.ratio * 100}%`, background: fillColor(row.fill, row.model) }} />}
        </div>;
      })}
      {fable && <div className="compact-row__track compact-pair__track--week compact-pair__track--fable"
        role="meter" aria-label={`${fable.label} : quota restant`} aria-valuemin={0} aria-valuemax={100}
        aria-valuenow={fable.remaining ?? undefined} aria-valuetext={fable.value} title={`${fable.label} · ${fable.reset}`}>
        {fable.fill.ratio * 276 >= 3 && <span className="compact-row__fill" style={{ width: `${fable.fill.ratio * 100}%`, background: fillColor(fable.fill, true) }} />}
      </div>}
    </div>
    <div className="compact-pair__resets">
      {rows.map((row, index) => <span key={index} style={{ marginLeft: row.pairSlot === 1 ? "auto" : undefined }}
        title={`${windows[row.pairSlot ?? index]} · ${row.label} · ${row.reset}`}>{pairReset(row)}</span>)}
    </div>
    {fable && <div className="compact-pair__fable-caption">
      <span style={{ color: fillColor(fable.fill, true) }} title={`${pairHeading(fable)} · ${fable.reset}`}>Fable</span>
      {fable.reset !== week?.reset && <span>{pairReset(fable)}</span>}
    </div>}
  </div>;
}

function Row({ row }: { row: CompactRow }) {
  return <div className="compact-row">
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
  </div>;
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

const PROVIDER_TITLES: Record<ProviderKind, string> = { codex: "Codex", claude: "Claude" };

/** Cards grouped under their provider, in model order; an enabled provider
 *  without a card still gets its (empty) group for its header and its +. */
export type CompactGroup = { kind: ProviderKind; providers: CompactProvider[]; addable: boolean };
export function compactGroups(panel: CompactPanelModel): CompactGroup[] {
  const groups: CompactGroup[] = [];
  const addable = panel.addable ?? [];
  for (const provider of panel.providers) {
    if (!provider.kind) continue;
    let group = groups.find((item) => item.kind === provider.kind);
    if (!group) { group = { kind: provider.kind, providers: [], addable: addable.includes(provider.kind) }; groups.push(group); }
    group.providers.push(provider);
  }
  for (const kind of addable) if (!groups.some((group) => group.kind === kind)) groups.push({ kind, providers: [], addable: true });
  return groups;
}

function PlusIcon() {
  return <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path d="M8 3.5v9M3.5 8h9" /></svg>;
}

export type CompactPanelProps = {
  panel: CompactPanelModel | null;
  error: string | null;
  pending: "settings" | "update" | "connect" | null;
  onSettings: () => void;
  onUpdate: () => void;
  onConnect: () => void;
  /** « CODEX ↗ »: detailed usage of the provider. */
  onUsage?: (kind: ProviderKind) => void;
  /** + of a provider: its add-account flow (Réglages → Fournisseurs). */
  onAdd?: (kind: ProviderKind) => void;
  /** The app's external arrow (ExternalIcon), passed in to keep this view import-free. */
  usageIcon?: ReactNode;
};

function Card({ provider }: { provider: CompactProvider }) {
  const pair = pairRowsOf(provider);
  const fable = pair.length ? provider.rows.find((row) => row.model && row.label.endsWith(" Fable")) : undefined;
  return <>
    {pair.length > 0 && <Pair rows={pair} fable={fable} />}
    {provider.rows.filter((row) => !pair.includes(row) && row !== fable)
      .map((row, index) => <Row key={`${row.label}-${index}`} row={row} />)}
    {provider.note && <p className={`compact-card__note compact-tone--${provider.note.tone}`
      + (provider.rows.length ? " has-rows" : "")}>{provider.note.text}</p>}
  </>;
}

function pairRowsOf(provider: CompactProvider) {
  if (!provider.compact) return [];
  // Compatibility for historical Claude fixture models without pairSlot.
  if (provider.name !== "Codex" && provider.rows.every((r) => r.pairSlot === undefined)) {
    const windows = provider.rows.filter((r) => !r.model).slice(0, 2);
    return windows.length >= 2 ? windows : [];
  }
  return provider.rows.filter((r) => r.pairSlot === 0 || r.pairSlot === 1).sort((a, b) => a.pairSlot! - b.pairSlot!);
}

export function CompactPanel({ panel, error, pending, onSettings, onUpdate, onConnect, onUsage, onAdd, usageIcon }: CompactPanelProps) {
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
    {panel && compactGroups(panel).map((group) => {
      const title = PROVIDER_TITLES[group.kind];
      return <section key={group.kind} className="compact-group" aria-label={title}>
        <h2 className="compact-group__head">
          <button type="button" className="compact-group__usage" aria-label={`${title} : usage détaillé`} title="Usage détaillé"
            onClick={() => onUsage?.(group.kind)}>{title.toUpperCase()}{usageIcon ?? " ↗"}</button>
        </h2>
        {group.providers.map((provider, index) => {
          const name = provider.accountName ?? provider.section;
          return <section key={`${provider.name}-${index}`} className="compact-provider"
            aria-label={provider.badge ? `${provider.name} (${provider.badge})` : provider.name}>
            <div className="compact-provider__head">
              <span className="compact-provider__name">{name}</span>
              {provider.badge && <span className="compact-provider__badge">{provider.badge}</span>}
              {provider.state && <span className={`compact-provider__state compact-tone--${provider.state.tone}`}>{provider.state.label}</span>}
            </div>
            <div className="compact-card"><Card provider={provider} /></div>
          </section>;
        })}
        {group.addable && <div className="compact-group__footer">
          <button type="button" className="compact-group__add" aria-label={`Ajouter un compte ${title}`} title={`Ajouter un compte ${title}`}
            onClick={() => onAdd?.(group.kind)}><PlusIcon /></button>
        </div>}
      </section>;
    })}
    {panel?.providers.filter((provider) => !provider.kind).map((provider, providerIndex) => <section key={`${provider.name}-${providerIndex}`} className="compact-provider"
      aria-label={provider.badge ? `${provider.name} (${provider.badge})` : provider.name}>
      <div className="compact-provider__head compact-provider__head--section" aria-hidden="true">
        <span>{provider.section}</span>
        {provider.badge && <span className="compact-provider__badge">{provider.badge}</span>}
      </div>
      <div className="compact-card"><Card provider={provider} /></div>
    </section>)}
    {panel?.connection && <div className="compact__connect">
      <button type="button" onClick={onConnect} disabled={panel.connection.loggingIn || pending === "connect"}>
        {panel.connection.label}
      </button>
    </div>}
    {panel && <p className="compact__version">Quota Monitor v{panel.version}</p>}
  </main>;
}
