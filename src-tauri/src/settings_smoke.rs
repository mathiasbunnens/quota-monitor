//! `--settings-ui-smoke`: opens the real Settings window (hidden, unfocused) in
//! an isolated process and checks that the bundled React Settings page mounts.
//! Fails when the binary serves the dev URL instead of embedded assets (a plain
//! `cargo build` without a running `npm run dev`). Reads no preferences, starts
//! no provider, sends no notification.

use std::io::Write;
use std::time::Duration;

const MARKER: &str = r#"nav[role="tablist"][aria-label="Catégories de réglages"]"#;
const TIMEOUT: Duration = Duration::from_secs(15);

fn finish(pass: bool, detail: &str) -> ! {
    println!(
        "{} {detail}",
        if pass {
            "SETTINGS_UI_SMOKE_PASS"
        } else {
            "SETTINGS_UI_SMOKE_FAIL"
        }
    );
    let _ = std::io::stdout().flush();
    std::process::exit(if pass { 0 } else { 1 });
}

/// Called by the probe script from inside the Settings webview.
#[tauri::command]
fn settings_smoke_report(url: String, mounted: bool, tabs: u32) {
    let detail = serde_json::json!({ "url": url, "mounted": mounted, "tabs": tabs }).to_string();
    finish(mounted && tabs > 0, &detail);
}

pub fn settings_ui_smoke() {
    let probe = format!(
        r#"(() => {{
  const started = Date.now();
  const tick = () => {{
    const nav = document.querySelector({marker:?});
    const tabs = nav ? nav.querySelectorAll('[role="tab"]').length : 0;
    if (nav || Date.now() - started > 8000) {{
      window.__TAURI_INTERNALS__.invoke("settings_smoke_report", {{ url: location.href, mounted: !!nav, tabs }});
    }} else setTimeout(tick, 100);
  }};
  tick();
}})()"#,
        marker = MARKER
    );
    std::thread::spawn(|| {
        std::thread::sleep(TIMEOUT);
        finish(
            false,
            "timeout: Settings page never reported (dev URL or blocked IPC?)",
        );
    });
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            settings_smoke_report,
            crate::get_platform,
            crate::prefs::get_preferences,
            crate::fit_settings_window
        ])
        .setup(move |app| {
            app.handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory)?;
            let probe = probe.clone();
            crate::settings_window_builder(app.handle())
                .visible(false)
                .focused(false)
                .on_page_load(move |window, payload| {
                    eprintln!("settings page {:?}: {}", payload.event(), payload.url());
                    if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                        let _ = window.eval(&probe);
                    }
                })
                .build()?;
            Ok(())
        })
        .build(crate::app_context())
        .expect("settings smoke app")
        .run(|_, _| {});
}
