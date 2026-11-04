// Serialised menu-bar panel model (`get_compact_panel` in src-tauri/src/lib.rs).
// The backend computes every label, value, order and gauge colour exactly as
// for the native menu; the compact window only lays it out.

export type CompactTone = "neutral" | "good" | "warning" | "error";
export type GaugeColor = "red" | "orange" | "green";

export type CompactFill = {
  from: GaugeColor;
  to: GaugeColor;
  /** Blend towards `to`, 0..=1 (NSColor blendedColorWithFraction). */
  fraction: number;
  /** False when the fill would be narrower than the gauge radius. */
  visible: boolean;
  /** Fill width relative to the track, 0..=1. */
  ratio: number;
};

export type CompactRow = {
  /** Explicit pair membership from panel_model, never inferred from order. */
  pairSlot?: 0 | 1 | null;
  model: boolean;
  label: string;
  /** Percentage REMAINING (100 = full quota available), null when unknown. */
  remaining: number | null;
  value: string;
  reset: string;
  /** Time left before the reset ("3h30", "42 min", "3j 8h"); "—" when unknown. */
  countdown: string;
  low: boolean;
  fill: CompactFill;
};

export type ProviderKind = "codex" | "claude";

export type CompactProvider = {
  /** Groups the cards under one provider header; absent in older models. */
  kind?: ProviderKind;
  /** Account of this card: routes its chevron to its detail in Réglages. */
  accountId?: string | null;
  /** Account name, the card title under the provider header. */
  accountName?: string | null;
  /** Connection state of that account (Connecté, Non connecté, Même compte…). */
  state?: { label: string; tone: CompactTone } | null;
  name: string;
  section: string;
  /** Claude multi-account card: 5 h and 7 j paired, like the menu. */
  compact: boolean;
  badge: string | null;
  rows: CompactRow[];
  note: { text: string; tone: CompactTone } | null;
};

export type UpdatePhase =
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "ready"
  | "up-to-date"
  | "error";

export type CompactUpdate = {
  phase: UpdatePhase;
  message: string;
  progress: number | null;
  currentVersion: string;
  nextVersion: string | null;
};

export type CompactPanelModel = {
  header: { title: string; subtitle: string; status: string; tone: CompactTone };
  providers: CompactProvider[];
  /** Enabled providers: each gets its header (↗) and its + even without a card. */
  addable?: ProviderKind[];
  connection: { label: string; loggingIn: boolean } | null;
  version: string;
  /** Last updater state; null while no check has reported anything. */
  update: CompactUpdate | null;
};
