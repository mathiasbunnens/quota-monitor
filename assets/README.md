# Application icon

`app-icon.png` is the approved, unmodified 1024×1024 bitmap from
`quota-monitor-V2.png`. It is the authoritative application-icon source, not
an SVG approximation. Preserve its colors and the small bright arc at the top.

SHA-256: `8ab76e2f3e93ff38b14b0ebdc9906b7fb11382cf2b418e2389487776f3d74309`

Regenerate the platform assets using the installed Tauri CLI:

    npm run icon:app

Tauri writes the desktop PNG, ICNS, ICO and mobile assets under
`src-tauri/icons/`. The existing `tauri.conf.json` bundle paths consume these
files without configuration changes. The older `src-tauri/icons/app-icon.svg`
is not the source for this approved bitmap.

The menu-bar template remains separate: `mascot-menu.png`, its SVG and
`npm run icon:tray` are unchanged. Never use the colored app tile as the tray
or macOS status-item template.
