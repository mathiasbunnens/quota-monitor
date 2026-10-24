import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { CompactPanel } from "./CompactPanel";
import type { CompactPanelModel } from "./types";
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

  return <CompactPanel panel={panel} error={error ?? readError} pending={pending}
    onSettings={openSettings} onUpdate={checkUpdates} onConnect={connect} />;
}
