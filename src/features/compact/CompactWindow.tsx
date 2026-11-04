import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import ExternalIcon from "../ui/ExternalIcon";
import { createFitScheduler } from "./fitWindow";
import { CompactPanel } from "./CompactPanel";
import type { CompactPanelModel, ProviderKind } from "./types";
import "./compact.scss";

/** The menu-bar item refreshes on every open; the window does it on focus. */
const FOCUS_REFRESH_MS = 30_000;

/**
 * Standalone (Dock) window: the menu-bar panel, with "Réglages…" and
 * "Vérifier les mises à jour…" as two icon buttons in the top-right corner.
 * Both call the same backend commands as the existing settings/updater flows.
 */
export default function CompactWindow() {
  const [panel, setPanel] = useState<CompactPanelModel | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [readError, setReadError] = useState<string | null>(null);
  const [pending, setPending] = useState<"settings" | "update" | "connect" | null>(null);
  const [revision, setRevision] = useState(0);
  const lastRefresh = useRef(0);
  const reload = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    const quotas = listen("quota-updated", reload).catch(() => () => {});
    return () => { void quotas.then((unlisten) => unlisten()); };
  }, [reload]);

  useEffect(() => {
    let disposed = false;
    let timer: number;
    const poll = async () => {
      try {
        const next = await invoke<CompactPanelModel>("get_compact_panel");
        if (!disposed) { setPanel(next); setReadError(null); }
      } catch (cause) {
        if (!disposed) setReadError("Impossible de lire les quotas : " + String(cause));
      }
      if (!disposed) timer = window.setTimeout(() => void poll(), 2000);
    };
    void poll();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [revision]);

  useEffect(() => {
    const refresh = () => {
      const now = Date.now();
      if (now - lastRefresh.current < FOCUS_REFRESH_MS) return;
      lastRefresh.current = now;
      void invoke("request_quota_refresh").catch(() => undefined);
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, []);

  const busy = useRef(false);
  const run = useCallback(async (kind: "settings" | "update" | "connect", command: string) => {
    // One action at a time; a ref so double clicks before a re-render are ignored.
    if (busy.current) return;
    busy.current = true;
    setPending(kind); setError(null);
    try { await invoke(command); }
    catch (cause) { setError(String(cause)); }
    finally { busy.current = false; setPending(null); reload(); }
  }, [reload]);

  const openSettings = useCallback(() => void run("settings", "open_settings"), [run]);
  const checkUpdates = useCallback(() => void run("update", "start_update_check"), [run]);
  const connect = useCallback(() => void run("connect", "connect_codex_account"), [run]);

  // Provider ↗ and +: the same pages and Réglages focus as the app.
  const openUsage = useCallback((kind: ProviderKind) => {
    void invoke(kind === "codex" ? "open_usage_details" : "open_claude_usage").catch((cause) => setError(String(cause)));
  }, []);
  const addAccount = useCallback((kind: ProviderKind) => {
    void invoke("open_settings_focus", { provider: kind, add: true }).catch((cause) => setError(String(cause)));
  }, []);

  // Cmd+, opens Réglages, like the key equivalent of the menu item.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey && !event.ctrlKey && !event.altKey && event.key === ",") {
        event.preventDefault();
        openSettings();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openSettings]);

  // The window follows the panel height (see fitWindow.ts); width is the user's.
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const node = root.current;
    if (!node) return;
    const scheduler = createFitScheduler({
      request: (value) => void invoke("fit_main_window", value).catch(console.error),
      setTimer: (callback, ms) => window.setTimeout(callback, ms),
      clearTimer: (handle) => window.clearTimeout(handle as number),
    });
    const measure = () => scheduler.measure(node.getBoundingClientRect().height, window.innerHeight);
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    measure();
    return () => { observer.disconnect(); scheduler.dispose(); };
  }, []);

  return <div ref={root} className="compact-fit"><CompactPanel panel={panel} error={error ?? readError} pending={pending}
    onSettings={openSettings} onUpdate={checkUpdates} onConnect={connect}
    onUsage={openUsage} onAdd={addAccount} usageIcon={<ExternalIcon />} /></div>;
}
