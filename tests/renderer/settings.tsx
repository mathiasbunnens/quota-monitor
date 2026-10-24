// Browser-only synthetic preferences. No native command/network is executed.
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Settings from "../../src/features/settings/Settings";
import type { Preferences } from "../../src/features/quota/types";
import "../../src/styles/app.scss";

let prefs: Preferences = {
  codexEnabled: false, claudeEnabled: false, fableEnabled: true,
  indicator: "both", displayMode: "menubar", ntfy: { enabled: false, serverUrl: "https://ntfy.invalid", topic: "" },
};
const calls: { command: string; args: unknown }[] = [];
let failure: string | null = null;
let failureMessage = "Offline fixture: request failed";
let delay = 0;
mockIPC(async (command, args) => {
  calls.push({ command, args: structuredClone(args) });
  if (command === "test_ntfy" || command === "set_preferences") {
    await new Promise((resolve) => setTimeout(resolve, delay));
    if (failure === command) throw new Error(failureMessage);
  }
  if (command === "get_platform") return "macos";
  if (command === "get_preferences") return prefs;
  if (command === "get_codex_status") return { phase: "disabled", message: "Fixture", refreshSettings: { customSeconds: null } };
  if (command === "get_claude_snapshot") return { state: "disabled", metrics: [] };
  if (command === "set_preferences") { prefs = args!.prefs as Preferences; return prefs; }
  if (command === "test_ntfy" || command === "fit_settings_window") return null;
  throw new Error(`Unexpected fixture command: ${command}`);
}, { shouldMockEvents: true });
Object.assign(window, { settingsFixture: {
  calls, fail(command: string | null, message = "Offline fixture: request failed") { failure = command; failureMessage = message; },
  delay(ms: number) { delay = ms; },
  preferences() { return structuredClone(prefs); },
} });
createRoot(document.getElementById("root")!).render(<Settings />);
