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
  model: boolean;
  label: string;
  /** Percentage REMAINING (100 = full quota available), null when unknown. */
  remaining: number | null;
  value: string;
  reset: string;
  low: boolean;
  fill: CompactFill;
};

export type CompactProvider = {
  name: string;
  section: string;
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
  connection: { label: string; loggingIn: boolean } | null;
  version: string;
  /** Last updater state; null while no check has reported anything. */
  update: CompactUpdate | null;
};
