/**
 * Height-follow logic of the standalone window, kept free of DOM and IPC so it
 * can be tested with a controlled clock.
 *
 * Rule: the window follows its content HEIGHT only; the width stays whatever
 * the user chose (the panel is centred up to 420 px). A manual height resize is
 * overridden at the next content change, never fought continuously: a size is
 * requested only when the measured content changes.
 */
export type FitRequest = { height: number; viewportHeight: number };

export type FitDeps = {
  request: (value: FitRequest) => void;
  setTimer: (callback: () => void, ms: number) => unknown;
  clearTimer: (handle: unknown) => void;
};

export const FIT_DEBOUNCE_MS = 40;

export function createFitScheduler(deps: FitDeps) {
  let handle: unknown = null;
  let lastContent = 0;
  return {
    /** Content measured by the observer; `viewport` is window.innerHeight. */
    measure(content: number, viewport: number) {
      const height = Math.ceil(content);
      if (!Number.isFinite(height) || height <= 0) return;
      // Our own window resizes change `viewport`, not `content`: ignored.
      if (height === lastContent) return;
      if (handle !== null) deps.clearTimer(handle);
      handle = deps.setTimer(() => {
        handle = null;
        lastContent = height;
        deps.request({ height, viewportHeight: viewport });
      }, FIT_DEBOUNCE_MS);
    },
    dispose() {
      if (handle !== null) deps.clearTimer(handle);
      handle = null;
    },
  };
}
