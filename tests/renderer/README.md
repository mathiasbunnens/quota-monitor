# Offline updater renderer harness

Use the existing dependencies (Node 22+); no browser/runtime download is needed
when a local Playwright-capable browser is already available.

    npm run dev -- --host 127.0.0.1 --port 1437 --strictPort

Open `http://127.0.0.1:1437/tests/renderer/update.html`. This mounts the real
`UpdatePopup`, stylesheet, ResizeObserver and subscription logic. Only Tauri
IPC/events are mocked with the installed Tauri test API. No requests for an
update, install, restart or native window operation are executed.

In the browser console:

    await (await import('./checks.js')).checkUpdater()

Run at 420px in light mode and 360px in dark mode. The checks cover searching,
available, 0%, partial, unknown total, installing, 100% ready, a long error and
up-to-date. They assert bar/percentage alignment, no horizontal overflow or
resize loops, and install/retry/restart/close command dispatch plus rejected
IPC. Fit requests are recorded in `window.updateFixture.calls`; the browser
does not pretend to resize a native window. Resize the viewport to the recorded
height, or cap it to simulate a small screen and verify vertical scrolling.

Additional manual checks: enable reduced motion and inspect indeterminate
progress; use Tab/Enter on the buttons; scroll long errors to reach Close.
Do not use this harness to claim native window resizing or a real update was
exercised. The harness is outside `src/` and is not a production build entry.

Dependency-free test runner (uses existing React/TypeScript packages):

    node --test scripts/update-popup.test.mjs scripts/app-icon.test.mjs scripts/quota-providers.test.mjs
