import { getCurrentWindow } from "@tauri-apps/api/window";
import UpdatePopup from "./features/update/UpdatePopup";
import Settings from "./features/settings/Settings";
import CompactWindow from "./features/compact/CompactWindow";
import Dashboard from "./pages/Dashboard/Dashboard";

export default function App() {
  const label = getCurrentWindow().label;
  if (label === "updater") return <UpdatePopup />;
  if (label === "settings") return <Settings />;
  // macOS application mode: compact copy of the menu-bar panel.
  if (window.__QUOTA_MONITOR_VIEW__ === "compact") return <CompactWindow />;
  return <Dashboard />;
}
