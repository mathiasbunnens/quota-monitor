// Browser-only, explicitly synthetic compact-window fixture. Never imported by
// the app entry. All IPC is intercepted: no quota provider, credential,
// updater, settings window or notification is touched.
import { createRoot } from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import CompactWindow from "../../src/features/compact/CompactWindow";
import type { CompactPanelModel, CompactRow, CompactUpdate } from "../../src/features/compact/types";
import "../../src/styles/app.scss";

const color = (remaining: number): Pick<CompactRow["fill"], "from" | "to" | "fraction"> =>
  remaining <= 20 ? { from: "red", to: "red", fraction: 0 }
    : remaining <= 40 ? { from: "red", to: "orange", fraction: (remaining - 20) / 20 }
      : remaining <= 60 ? { from: "orange", to: "green", fraction: (remaining - 40) / 20 }
        : { from: "green", to: "green", fraction: 0 };
const row = (label: string, remaining: number | null, reset: string, model = false, value?: string): CompactRow => ({
  model, label, remaining, reset,
  value: value ?? (remaining === null ? "—" : remaining === 0 ? "Épuisé" : `${remaining} %`),
  low: remaining !== null && remaining <= 20,
  fill: { ...color(remaining ?? 0), visible: (remaining ?? 0) * 276 / 100 >= 5, ratio: (remaining ?? 0) / 100 },
});

const codex = {
  name: "Codex", section: "CODEX", badge: "Plus", note: null,
  rows: [
    row("Limite sur 1h35", 63, "Réinitialisation à 15h40"),
    row("Limite sur 4j 2h", 85, "Réinitialisation le 29/10 à 09h12"),
    row("Réserve", 0, "Réinitialisation le 29/10 à 09h12"),
  ],
};
const claude = {
  name: "Claude", section: "CLAUDE", badge: "Pro", note: null,
  rows: [
    row("Limite sur 1h35", 12, "Réinitialisation à 15h40"),
    row("Limite Fable", 37, "Réinitialisation le 29/10 à 09h12", true),
  ],
};
const header = { title: "Quota Monitor", subtitle: "Actualisé à 14h05", status: "À jour", tone: "good" as const };
const base = { connection: null, version: "fixture", update: null };

export const scenarios: Record<string, CompactPanelModel> = {
  "two-providers": { ...base, header, providers: [codex, claude] },
  "codex-error": {
    ...base, header: { ...header, status: "Erreur", tone: "error" },
    connection: { label: "Se connecter à Codex…", loggingIn: false },
    providers: [{ ...codex, badge: null, rows: [], note: {
      text: "Codex a refusé la lecture des quotas : reconnectez votre compte. " +
        "Message synthétique volontairement long pour vérifier le retour à la ligne.", tone: "error" } }],
  },
  "claude-waiting": {
    ...base, header: { ...header, subtitle: "En attente des premières données", status: "Lecture…", tone: "warning" },
    providers: [{ ...claude, rows: [], note: { text: "Lecture via Claude Code…", tone: "warning" } }],
  },
  "logging-in": {
    ...base, header: { ...header, status: "Lecture…", tone: "warning" },
    connection: { label: "Connexion en cours…", loggingIn: true },
    providers: [{ ...codex, badge: null, rows: [], note: { text: "Connexion à Codex…", tone: "warning" } }],
  },
  none: {
    ...base, header: { title: "Quota Monitor", subtitle: "Activez Codex ou Claude dans les Réglages", status: "Inactif", tone: "neutral" },
    providers: [],
  },
};

let panel: CompactPanelModel = structuredClone(scenarios["two-providers"]);
const calls: { command: string; args: unknown }[] = [];
let failure: string | null = null;
let delay = 0;
let gate: Promise<void> | null = null;
let release = () => {};
mockIPC(async (command, args) => {
  calls.push({ command, args: structuredClone(args) });
  if (["open_settings", "start_update_check", "connect_codex_account"].includes(command)) {
    await new Promise((resolve) => setTimeout(resolve, delay));
    if (gate) await gate;
    if (failure === command) throw new Error(`Offline fixture: ${command} failed`);
    return null;
  }
  if (failure === command) throw new Error(`Offline fixture: ${command} failed`);
  if (command === "get_compact_panel") return structuredClone(panel);
  if (command === "request_quota_refresh") return null;
  throw new Error(`Unexpected fixture command: ${command}`);
}, { shouldMockEvents: true });

const settle = () => new Promise((resolve) => setTimeout(resolve, 60));
Object.assign(window, { compactFixture: {
  calls, scenarios,
  async scenario(name: string) { panel = structuredClone(scenarios[name]); await emit("quota-updated"); await settle(); },
  async update(next: Partial<CompactUpdate> | null) {
    panel = { ...panel, update: next === null ? null : {
      phase: "checking", message: "", progress: null, currentVersion: "fixture", nextVersion: "fixture-next", ...next } };
    await emit("quota-updated"); await settle();
  },
  fail(command: string | null) { failure = command; },
  delay(ms: number) { delay = ms; },
  /** Holds action commands in flight until `release()` (timer-independent). */
  hold() { gate = new Promise((resolve) => { release = () => { gate = null; resolve(); }; }); },
  release() { release(); },
} });
createRoot(document.getElementById("root")!).render(<CompactWindow />);
