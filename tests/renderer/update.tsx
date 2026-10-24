// Browser-only, explicitly synthetic fixtures. Never imported by the app entry.

import { createRoot } from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import UpdatePopup, {
  type UpdateStatus,
} from "../../src/features/update/UpdatePopup";
import "../../src/styles/app.scss";

const calls: { command: string; height?: number }[] = [];
let status: UpdateStatus = {
  phase: "checking",
  progress: null,
  currentVersion: "test-current",
  nextVersion: "test-next",
  message: "Recherche d’une nouvelle version…",
};
let failCommand: string | null = null;
mockIPC(
  (command, args) => {
    calls.push({ command, height: args?.height as number | undefined });
    if (command === failCommand)
      throw new Error("Offline fixture: IPC failure");
    if (command === "get_update_status") return status;
    // Fit calls are measured, not executed: the test runner supplies a viewport
    // bounded by a simulated screen. No native install or restart can occur.
    return null;
  },
  { shouldMockEvents: true },
);

Object.assign(window, {
  updateFixture: {
    calls,
    async set(next: Partial<UpdateStatus>) {
      status = { ...status, ...next };
      await emit("update-status", status);
      await new Promise((resolve) => setTimeout(resolve, 100));
    },
    fail(command: string | null) {
      failCommand = command;
    },
  },
});
createRoot(document.getElementById("root")!).render(<UpdatePopup />);
