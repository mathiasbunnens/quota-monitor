import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import "./update.scss";

type UpdatePhase =
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "ready"
  | "up-to-date"
  | "error";

export type UpdateStatus = {
  phase: UpdatePhase;
  message: string;
  progress: number | null;
  currentVersion: string;
  nextVersion: string | null;
};

export default function UpdatePopup() {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const root = useRef<HTMLElement>(null);

  useLayoutEffect(() => {
    const node = root.current;
    if (!node) return;
    let frame = 0;
    let lastHeight = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const height = Math.ceil(node.getBoundingClientRect().height);
        // Observe intrinsic content, not the viewport: screen clamping must not
        // trigger another resize request. Byte/progress events do not refit.
        if (height === lastHeight) return;
        lastHeight = height;
        void invoke("fit_update_window", {
          height,
          viewportHeight: window.innerHeight,
        }).catch(console.error);
      });
    });
    observer.observe(node);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let receivedEvent = false;
    const subscription = listen<UpdateStatus>("update-status", (event) => {
      receivedEvent = true;
      if (!disposed) {
        setStatus(event.payload);
        setError(null);
      }
    });
    void subscription
      .then(() => invoke<UpdateStatus>("get_update_status"))
      .then((value) => {
        if (!disposed && !receivedEvent) setStatus(value);
      })
      .catch((cause) => {
        if (!disposed) setError(String(cause));
      });
    return () => {
      disposed = true;
      void subscription.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, []);

  const run = async (command: UpdateCommand) => {
    setBusy(true);
    setError(null);
    try {
      await invoke(command);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <main className="update-popup" ref={root}>
      <UpdateContent
        status={status}
        busy={busy}
        error={error}
        onCommand={run}
      />
    </main>
  );
}

type UpdateCommand =
  | "start_update_install"
  | "restart_application"
  | "start_update_check"
  | "dismiss_update_window";

export function displayProgress(
  value: number | null | undefined,
): number | undefined {
  return value != null && Number.isFinite(value)
    ? Math.max(0, Math.min(100, Math.floor(value)))
    : undefined;
}

// Pure content is also used by the offline renderer tests (no updater/network).
export function UpdateContent({
  status,
  busy,
  error,
  onCommand,
}: {
  status: UpdateStatus | null;
  busy: boolean;
  error: string | null;
  onCommand: (command: UpdateCommand) => Promise<void>;
}) {
  const phase = status?.phase ?? "checking";
  const progress = displayProgress(status?.progress);
  const title =
    phase === "ready"
      ? "Mise à jour prête"
      : phase === "up-to-date"
        ? "Quota Monitor est à jour"
        : phase === "error"
          ? "Mise à jour impossible"
          : "Mise à jour de Quota Monitor";

  return (
    <>
      <header className="update-popup__header">
        <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
          <path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v4.2h-4.2" />
        </svg>
        <div>
          <h1>{title}</h1>
          <p className="update-popup__eyebrow">
            Quota Monitor{" "}
            {status?.currentVersion && `v${status.currentVersion}`}
          </p>
        </div>
      </header>
      <section
        className="update-popup__body"
        aria-label="État de la mise à jour"
      >
        <div className="update-popup__card">
          <p className="update-popup__message" role="status">
            {status?.message ?? "Recherche d’une nouvelle version…"}
          </p>
          {(phase === "downloading" ||
            phase === "installing" ||
            phase === "ready") && (
            <div className="update-popup__progress">
              <div className="update-popup__progress-heading">
                <span id="update-progress-label">
                  {phase === "installing"
                    ? "Vérification et installation"
                    : phase === "ready"
                      ? "Installation terminée"
                      : "Téléchargement"}
                </span>
                <span className="update-popup__percent">
                  {phase === "installing" || progress === undefined
                    ? "En cours…"
                    : `${progress} %`}
                </span>
              </div>
              <progress
                max={100}
                value={phase === "installing" ? undefined : progress}
                aria-labelledby="update-progress-label"
              />
            </div>
          )}
        </div>
        {error && (
          <p className="update-popup__error" role="alert">
            {error}
          </p>
        )}
        <div className="update-popup__actions">
          <button
            type="button"
            onClick={() => void onCommand("dismiss_update_window")}
          >
            Fermer
          </button>
          {phase === "available" && (
            <button
              type="button"
              className="update-popup__primary"
              disabled={busy}
              onClick={() => void onCommand("start_update_install")}
            >
              Télécharger et installer
            </button>
          )}
          {phase === "ready" && (
            <button
              type="button"
              className="update-popup__primary"
              disabled={busy}
              onClick={() => void onCommand("restart_application")}
            >
              Quitter et relancer
            </button>
          )}
          {phase === "error" && (
            <button
              type="button"
              className="update-popup__primary"
              disabled={busy}
              onClick={() => void onCommand("start_update_check")}
            >
              Réessayer
            </button>
          )}
        </div>
      </section>
    </>
  );
}
