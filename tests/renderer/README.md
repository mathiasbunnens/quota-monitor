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

## Notifications and menu template

Open `http://127.0.0.1:1437/tests/renderer/settings.html`, then run:

    await (await import('./notifications-checks.js')).checkNotifications()

This mounts the real Settings window, including its polling and intrinsic fit
observer. All IPC is intercepted; the fixture uses `.invalid` endpoints, never
reads real preferences and never sends notifications. Check at 480px light and
360px dark, with reduced motion. It exercises disabled/enabled drafts, test vs
save payloads, in-flight locking, rejected test/save, polling-safe drafts,
associated labels, visible privacy/threshold information, disclosure and overflow.
Use Tab/Enter on the help summary and scroll its expanded content in a short
viewport. `window.settingsFixture.fail('test_ntfy', 'long error…')` can exercise
long error wrapping; `fail(null)` clears it. Browser fit calls do not resize a
native window and are not evidence of native WebView sizing.

Run all structural/render tests with `node --test scripts/*.test.mjs`.
On macOS, an optional isolated native child verifies AppKit template rendering:

    swift tests/native/tray-template.swift src-tauri/icons/mascot-menu.png dist-local/evidence

It creates and removes only its own NSStatusItem, captures light/dark at 1x/2x,
and does not start Tauri, providers, an updater or the installed application.
This is an AppKit smoke test, not an installed application acceptance test.

The optical SVG/generator reconstruct the approved single-eye PNG mascot;
the canonical app artwork and bundle icons must remain untouched. `npm run
icon:tray` writes the 36px RGBA template used at 18pt by `mascot_image` in
`src-tauri/src/lib.rs`. Initial, single-provider and stacked-provider macOS
paths all use it; the stacked image also remains a template. Numeric text,
status indicators and Windows/Linux numeric tray rendering are unchanged.

## Compact standalone window

Open `http://127.0.0.1:1437/tests/renderer/compact.html`, then run:

    await (await import('./compact-checks.js')).checkCompact()

This mounts the real `CompactWindow` (macOS application mode) with synthetic
panel models shaped like `get_compact_panel`. All IPC is intercepted: no quota
provider, updater check, settings window or notification runs. The checks
cover the two top-right icon buttons (names, tooltips, focus, Cmd+,), their
dispatch to `open_settings` / `start_update_check`, in-flight locking and
rejected IPC, every updater state (busy/available/ready/error), row order and
fields against the model, notes, the Codex connection action and the empty
state. Run at 360px light, 300px dark with reduced motion and 420px. Keep the
tab visible: hidden pages throttle timers. Browser captures are not evidence of
native window, Dock or activation behaviour. `node --test
scripts/compact-window.test.mjs` covers the pure view without a browser.
