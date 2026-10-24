mod activity;
mod claude;
mod claude_cli;
mod codex;
mod indicator;
mod ntfy;
mod prefs;
#[cfg(target_os = "macos")]
mod settings_smoke;
#[cfg(target_os = "macos")]
pub use settings_smoke::settings_ui_smoke;
#[cfg(any(not(target_os = "macos"), test))]
mod tray_percentage;

/// Packaged executable smoke of the production transport, without opening UI or touching preferences.
pub fn claude_usage_smoke(directory: &str) -> Result<(), String> {
    let executable = claude_cli::discover().ok_or("Claude Code absent")?;
    let payload = claude_cli::read_usage(&executable, std::path::Path::new(directory))
        .map_err(str::to_string)?;
    if payload["rate_limits_available"] != true {
        return Err("Limites d’abonnement non disponibles".into());
    }
    let five = claude_cli::window(&payload["rate_limits"]["five_hour"]);
    let seven = claude_cli::window(&payload["rate_limits"]["seven_day"]);
    if five.is_none() && seven.is_none() {
        return Err("Aucune fenêtre reçue".into());
    }
    println!(
        "CLAUDE_USAGE_SMOKE_PASS {}",
        serde_json::json!({"fiveHour":five,"sevenDay":seven})
    );
    Ok(())
}

/// Entry for the `--claude-statusline-relay` helper mode (no UI is started).
pub fn claude_relay(args: &[String]) -> i32 {
    claude::relay_main(args)
}

/// True when the process was started as the Claude Code statusLine relay.
pub fn is_claude_relay(args: &[String]) -> bool {
    args.iter().any(|a| a == claude::RELAY_FLAG)
}

#[cfg(target_os = "macos")]
use chrono::{DateTime, Local};
use serde::Serialize;
use std::sync::atomic::AtomicBool;
#[cfg(target_os = "macos")]
use std::sync::OnceLock;
use std::sync::{atomic::Ordering, Mutex};
use tauri::{Emitter, Manager};

#[cfg(target_os = "macos")]
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    MainThreadOnly,
};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSApplication, NSBox, NSBoxType, NSColor, NSFont, NSImage, NSMenu, NSMenuDelegate, NSMenuItem,
    NSStatusBar, NSTextField, NSVariableStatusItemLength, NSView,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{
    MainThreadMarker, NSData, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

#[derive(Debug, Serialize, Clone)]
struct QuotaPayload {
    #[serde(skip)]
    used_percent: Option<f64>,
    model: String,
    remaining: u32,
    limit: u32,
    #[serde(rename = "resetAt", alias = "reset_at")]
    reset_at: String,
    #[serde(rename = "checkedAt", alias = "checked_at")]
    checked_at: String,
    confidence: String,
    period: String,
    #[serde(rename = "resetLabel", alias = "reset_label", default)]
    reset_label: Option<String>,
    source: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default, rename = "windowMinutes")]
    window_minutes: Option<i64>,
}

#[derive(Default)]
struct QuotaState(Mutex<Vec<QuotaPayload>>);

fn publish_quotas(app: &tauri::AppHandle) {
    ntfy::wake(app);
    render_native_menu(app);
    let _ = app.emit("quota-updated", ());
}

fn set_codex_snapshots(app: &tauri::AppHandle, snapshots: Option<Vec<QuotaPayload>>) {
    if let Ok(mut data) = app.state::<QuotaState>().0.lock() {
        *data = snapshots.unwrap_or_default();
    }
    publish_quotas(app);
}

static UPDATE_CHECK_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdateCheckMode {
    #[cfg(not(debug_assertions))]
    Automatic,
    Interactive,
}

struct UpdateCheckGuard;

impl Drop for UpdateCheckGuard {
    fn drop(&mut self) {
        UPDATE_CHECK_IN_PROGRESS.store(false, Ordering::Release);
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatus {
    phase: String,
    message: String,
    progress: Option<u8>,
    current_version: String,
    next_version: Option<String>,
}

#[derive(Default)]
struct UpdateState(Mutex<Option<UpdateStatus>>);

fn update_status(
    app: &tauri::AppHandle,
    phase: &str,
    message: impl Into<String>,
    progress: Option<u8>,
    next_version: Option<String>,
) {
    let status = UpdateStatus {
        phase: phase.into(),
        message: message.into(),
        progress,
        current_version: app.package_info().version.to_string(),
        next_version,
    };
    if let Ok(mut state) = app.state::<UpdateState>().0.lock() {
        *state = Some(status.clone());
    }
    if let Some(window) = app.get_webview_window("updater") {
        let _ = window.emit("update-status", status);
    }
}

fn show_update_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    let window = if let Some(window) = app.get_webview_window("updater") {
        window
    } else {
        tauri::WebviewWindowBuilder::new(
            app,
            "updater",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("Mise à jour de Quota Monitor")
        .inner_size(420.0, 260.0)
        .min_inner_size(360.0, 240.0)
        .resizable(false)
        .center()
        .build()?
    };
    window.show()?;
    window.set_focus()?;
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(|| {
        let mtm = MainThreadMarker::new().expect("update popup activation runs on main thread");
        NSApplication::sharedApplication(mtm).activate();
    });
    Ok(())
}

fn show_update_error(app: &tauri::AppHandle, mode: UpdateCheckMode, error: impl ToString) {
    let error = error.to_string();
    eprintln!("Quota Monitor update failed: {error}");
    if mode == UpdateCheckMode::Interactive {
        let _ = show_update_window(app);
        update_status(
            app,
            "error",
            format!("Vérifie ta connexion Internet puis réessaie.\n\nDétail : {error}"),
            None,
            None,
        );
    }
}

async fn check_for_available_update(app: tauri::AppHandle, mode: UpdateCheckMode) {
    use tauri_plugin_updater::UpdaterExt;

    if UPDATE_CHECK_IN_PROGRESS.swap(true, Ordering::AcqRel) {
        if mode == UpdateCheckMode::Interactive {
            let _ = show_update_window(&app);
        }
        return;
    }
    let _guard = UpdateCheckGuard;
    let current_version = app.package_info().version.to_string();
    if mode == UpdateCheckMode::Interactive {
        let _ = show_update_window(&app);
        update_status(
            &app,
            "checking",
            "Recherche d’une nouvelle version…",
            None,
            None,
        );
    }

    let update = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(error) => {
            show_update_error(&app, mode, error);
            return;
        }
    };

    match update {
        Ok(Some(update)) => {
            let next_version = update.version.clone();
            let _ = show_update_window(&app);
            update_status(
                &app,
                "available",
                format!(
                    "La version {current_version} est installée. Quota Monitor v{next_version} est disponible."
                ),
                None,
                Some(next_version),
            );
        }
        Ok(None) if mode == UpdateCheckMode::Interactive => update_status(
            &app,
            "up-to-date",
            format!("La version {current_version} est la dernière version disponible."),
            None,
            None,
        ),
        Ok(None) => {}
        Err(error) => show_update_error(&app, mode, error),
    }
}

async fn download_and_install_update(app: tauri::AppHandle) {
    use tauri_plugin_updater::UpdaterExt;

    if UPDATE_CHECK_IN_PROGRESS.swap(true, Ordering::AcqRel) {
        return;
    }
    let _guard = UpdateCheckGuard;
    let update = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(error) => {
            show_update_error(&app, UpdateCheckMode::Interactive, error);
            return;
        }
    };
    let update = match update {
        Ok(Some(update)) => update,
        Ok(None) => {
            update_status(
                &app,
                "up-to-date",
                "Aucune mise à jour n’est plus disponible.",
                None,
                None,
            );
            return;
        }
        Err(error) => {
            show_update_error(&app, UpdateCheckMode::Interactive, error);
            return;
        }
    };
    let next_version = update.version.clone();
    update_status(
        &app,
        "downloading",
        format!("Téléchargement de Quota Monitor v{next_version}…"),
        Some(0),
        Some(next_version.clone()),
    );
    let progress_app = app.clone();
    let progress_version = next_version.clone();
    let mut downloaded = 0_u64;
    let mut last_progress = 0_u8;
    let result = update
        .download_and_install(
            move |chunk_length, content_length| {
                downloaded = downloaded.saturating_add(chunk_length as u64);
                let progress = content_length
                    .filter(|total| *total > 0)
                    .map(|total| ((downloaded.saturating_mul(100) / total).min(100)) as u8)
                    .unwrap_or(last_progress);
                if progress != last_progress {
                    last_progress = progress;
                    update_status(
                        &progress_app,
                        "downloading",
                        format!("Téléchargement de Quota Monitor v{progress_version}…"),
                        Some(progress),
                        Some(progress_version.clone()),
                    );
                }
            },
            || {},
        )
        .await;
    match result {
        Ok(()) => update_status(
            &app,
            "ready",
            format!("Quota Monitor v{next_version} est installé et prêt à être relancé."),
            Some(100),
            Some(next_version),
        ),
        Err(error) => show_update_error(&app, UpdateCheckMode::Interactive, error),
    }
}

#[tauri::command]
fn get_update_status(app: tauri::AppHandle) -> UpdateStatus {
    app.state::<UpdateState>()
        .0
        .lock()
        .ok()
        .and_then(|status| status.clone())
        .unwrap_or_else(|| UpdateStatus {
            phase: "checking".into(),
            message: "Recherche d’une nouvelle version…".into(),
            progress: None,
            current_version: app.package_info().version.to_string(),
            next_version: None,
        })
}

#[tauri::command]
fn start_update_check(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(check_for_available_update(
        app,
        UpdateCheckMode::Interactive,
    ));
}

#[tauri::command]
fn start_update_install(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(download_and_install_update(app));
}

#[tauri::command]
fn dismiss_update_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("updater") {
        let _ = window.hide();
    }
}

#[tauri::command]
fn restart_application(app: tauri::AppHandle) {
    app.restart();
}

#[derive(Clone, Copy)]
#[cfg(target_os = "macos")]
struct NativeQuotaMenu {
    status_item: usize,
    status_button: usize,
    target: usize,
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct NativeMenuState(Mutex<Option<NativeQuotaMenu>>);

#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    struct QuotaMenuTarget;

    unsafe impl NSObjectProtocol for QuotaMenuTarget {}

    unsafe impl NSMenuDelegate for QuotaMenuTarget {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            MENU_OPEN.store(true, Ordering::Release);
            if let Some(app) = APP_HANDLE.get() { request_quota_refresh(app.clone()); }
        }

        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, _menu: &NSMenu) {
            MENU_OPEN.store(false, Ordering::Release);
        }
    }

    impl QuotaMenuTarget {
        #[unsafe(method(quitApp:))]
        fn quit_app(&self, _sender: Option<&AnyObject>) {
            if let Some(app) = APP_HANDLE.get() { app.exit(0); }
        }

        #[unsafe(method(connectAccount:))]
        fn connect_account(&self, _sender: Option<&AnyObject>) {
            let Some(app) = APP_HANDLE.get() else { return; };
            let Some(provider) = app.try_state::<codex::Provider>() else { return; };
            if provider.status().phase == "missing" {
                let _ = codex::open_codex_install(app.clone());
            } else {
                provider.login(false);
            }
        }

        #[unsafe(method(openSettings:))]
        fn open_settings_action(&self, _sender: Option<&AnyObject>) {
            if let Some(app) = APP_HANDLE.get().cloned() { open_settings_async(app); }
        }

        #[unsafe(method(checkForUpdates:))]
        fn check_for_updates(&self, _sender: Option<&AnyObject>) {
            if let Some(app) = APP_HANDLE.get().cloned() {
                tauri::async_runtime::spawn(check_for_available_update(
                    app,
                    UpdateCheckMode::Interactive,
                ));
            }
        }
    }
);

#[cfg(target_os = "macos")]
impl QuotaMenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

#[tauri::command]
fn get_platform() -> String {
    std::env::consts::OS.to_string()
}

#[tauri::command]
fn get_quota_snapshots(app: tauri::AppHandle) -> Vec<QuotaPayload> {
    if !prefs::current(&app).codex_enabled {
        return Vec::new();
    }
    let rows = app
        .state::<QuotaState>()
        .0
        .lock()
        .map(|data| data.clone())
        .unwrap_or_default();
    let plan = app
        .try_state::<codex::Provider>()
        .and_then(|provider| provider.status().plan_type);
    visible_quotas(rows, plan.as_deref())
}

fn visible_quotas(mut rows: Vec<QuotaPayload>, plan: Option<&str>) -> Vec<QuotaPayload> {
    let reserve = |p: &QuotaPayload| is_reserve(p);
    let five = |p: &QuotaPayload| {
        !reserve(p) && (p.period == "five-hour" || p.window_minutes == Some(300))
    };
    let weekly =
        |p: &QuotaPayload| !reserve(p) && (p.period == "weekly" || p.window_minutes == Some(10080));
    let five_active = rows.iter().any(|p| five(p) && p.remaining > 0);
    rows.sort_by_key(|p| quota_order(p, &five, &weekly, &reserve, five_active));
    if plan != Some("plus") {
        return rows;
    }
    let relevant: Vec<_> = rows
        .iter()
        .filter(|p| five(p) || weekly(p) || reserve(p))
        .cloned()
        .collect();
    if relevant.is_empty() {
        rows
    } else {
        relevant
    }
}

fn is_reserve(payload: &QuotaPayload) -> bool {
    let identity = format!("{} {}", payload.period, payload.model).to_ascii_lowercase();
    identity.contains("reserve")
        || identity.contains("luna")
        || identity.contains("base_model_inference")
}

fn quota_order(
    payload: &QuotaPayload,
    five: &impl Fn(&QuotaPayload) -> bool,
    weekly: &impl Fn(&QuotaPayload) -> bool,
    reserve: &impl Fn(&QuotaPayload) -> bool,
    five_active: bool,
) -> (u8, bool, String) {
    let rank = if five_active {
        if five(payload) {
            0
        } else if weekly(payload) {
            1
        } else if reserve(payload) {
            2
        } else {
            3
        }
    } else if reserve(payload) {
        0
    } else if weekly(payload) {
        1
    } else if five(payload) {
        2
    } else {
        3
    };
    (
        rank,
        !payload.period.starts_with("codex:codex:"),
        payload.period.clone(),
    )
}

#[tauri::command]
fn request_quota_refresh(app: tauri::AppHandle) {
    crate::claude::request_claude_refresh(app.clone());
    if !prefs::current(&app).codex_enabled {
        return;
    }
    if let Some(provider) = app.try_state::<codex::Provider>() {
        provider.refresh();
    }
}

/// Settings window as opened by the menu (shared with the settings smoke).
fn settings_window_builder(
    app: &tauri::AppHandle,
) -> tauri::WebviewWindowBuilder<'_, tauri::Wry, tauri::AppHandle> {
    tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App("index.html".into()))
        .title("Réglages de Quota Monitor")
        .inner_size(600.0, 520.0)
        .min_inner_size(520.0, 240.0)
        .center()
}

/// Application context; generated once so assets are embedded a single time.
fn app_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

fn show_settings_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    let window = if let Some(window) = app.get_webview_window("settings") {
        window
    } else {
        settings_window_builder(app).build()?
    };
    window.show()?;
    window.unminimize()?;
    window.set_focus()?;
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(|| {
        let mtm = MainThreadMarker::new().expect("settings activation runs on main thread");
        NSApplication::sharedApplication(mtm).activate();
    });
    Ok(())
}

/// Window creation must not run on the main thread inside a menu callback.
fn open_settings_async(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = show_settings_window(&app) {
            eprintln!("Quota Monitor settings unavailable: {error}");
        }
    });
}

#[tauri::command]
async fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    show_settings_window(&app).map_err(|e| e.to_string())
}

/// Fits the Settings window to its content height (logical px), within the screen.
#[tauri::command]
fn fit_settings_window(window: tauri::WebviewWindow, height: f64) -> Result<(), String> {
    if window.label() != "settings" || !height.is_finite() {
        return Ok(());
    }
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let width = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale)
        .width;
    let max = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| {
            m.work_area()
                .size
                .to_logical::<f64>(m.scale_factor())
                .height
                - 40.0
        })
        .unwrap_or(900.0);
    window
        .set_size(tauri::LogicalSize::new(
            width,
            height.clamp(240.0, max.max(240.0)),
        ))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_claude_usage(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url("https://claude.ai/settings/usage", None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_usage_details(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(
            "https://chatgpt.com/codex/cloud/settings/analytics#usage",
            None::<&str>,
        )
        .map_err(|e| e.to_string())
}

#[cfg(any(target_os = "macos", test))]
fn quota_color_transition(percentage: u32) -> (&'static str, &'static str, f64) {
    match percentage {
        0..=20 => ("red", "red", 0.0),
        21..=40 => ("red", "orange", f64::from(percentage - 20) / 20.0),
        41..=60 => ("orange", "green", f64::from(percentage - 40) / 20.0),
        _ => ("green", "green", 0.0),
    }
}

#[cfg(target_os = "macos")]
fn quota_color(percentage: u32, model: bool) -> Retained<NSColor> {
    let (from, to, fraction) = quota_color_transition(percentage);
    // Model-scoped rows swap only the healthy hue for teal; warning/low stay identical.
    let color = |name| match name {
        "red" => NSColor::systemRedColor(),
        "orange" => NSColor::systemOrangeColor(),
        _ if model => NSColor::systemTealColor(),
        _ => NSColor::systemGreenColor(),
    };
    let from_color = color(from);
    if from == to {
        return from_color;
    }
    from_color
        .blendedColorWithFraction_ofColor(fraction, &color(to))
        .unwrap_or(from_color)
}

// ---------------------------------------------------------------------------
// Menu-bar panel (macOS). The menu IS the dashboard: every limit of every
// active provider is visible without a second window. The panel model is pure
// so its wording and priorities are unit-tested; the AppKit view only lays it
// out. Inspired by the SublerAutomation popup: header + status capsule, one
// card per provider, uppercase section titles.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
const PANEL_WIDTH: f64 = 320.0;
#[cfg(target_os = "macos")]
const PANEL_HEADER_HEIGHT: f64 = 54.0;
#[cfg(target_os = "macos")]
const PANEL_ROW_HEIGHT: f64 = 48.0;
#[cfg(target_os = "macos")]
const PANEL_INNER_WIDTH: f64 = 276.0;

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Neutral,
    Good,
    Warning,
    Error,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq)]
struct PanelRow {
    /// Model-scoped row (Fable): smaller model name matching the fill, full opacity.
    model: bool,
    label: String,
    remaining: Option<u32>,
    value: String,
    reset: String,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq)]
struct PanelProvider {
    name: &'static str,
    badge: Option<String>,
    rows: Vec<PanelRow>,
    note: Option<(String, Tone)>,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq)]
struct PanelHeader {
    subtitle: String,
    status: &'static str,
    tone: Tone,
}

#[cfg(target_os = "macos")]
fn value_text(remaining: Option<u32>, exhausted: bool, missing: &str) -> String {
    match remaining {
        Some(0) => "Épuisé".into(),
        Some(_) if exhausted => "Épuisé".into(),
        Some(p) => format!("{p} %"),
        None => missing.into(),
    }
}

#[cfg(target_os = "macos")]
fn reset_phrase(rfc3339: &str, short_window: bool) -> Option<String> {
    DateTime::parse_from_rfc3339(rfc3339).ok().map(|date| {
        let local = date.with_timezone(&Local);
        if short_window {
            local.format("Réinitialisation à %Hh%M").to_string()
        } else {
            local
                .format("Réinitialisation le %d/%m à %Hh%M")
                .to_string()
        }
    })
}

/// "1h38", "7j2h", "42 min": time left before a window resets.
#[cfg(any(target_os = "macos", test))]
fn remaining_span(seconds: i64) -> String {
    let minutes = (seconds.max(0) + 59) / 60;
    let (days, hours, mins) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    match (days, hours) {
        (0, 0) => format!("{mins} min"),
        (0, h) => format!("{h}h{mins:02}"),
        (d, 0) => format!("{d}j"),
        (d, h) => format!("{d}j {h}h"),
    }
}

/// "Limite sur 1h38" while the reset is known and ahead, otherwise the window name.
#[cfg(target_os = "macos")]
fn limit_label(reset_at: Option<&str>, fallback: &str) -> String {
    reset_at
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.timestamp() - Local::now().timestamp())
        .filter(|left| *left > 0)
        .map(|left| format!("Limite sur {}", remaining_span(left)))
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(target_os = "macos")]
fn codex_panel(
    rows: &[QuotaPayload],
    phase: &str,
    message: &str,
    plan: Option<String>,
) -> PanelProvider {
    // Same vocabulary and order for every provider: 5 heures, hebdomadaire, réserve.
    let rank = |p: &QuotaPayload| -> u8 {
        if is_reserve(p) {
            2
        } else if p.period == "five-hour" || p.window_minutes == Some(300) {
            0
        } else if p.period == "weekly" || p.window_minutes == Some(10080) {
            1
        } else {
            3
        }
    };
    let mut ordered: Vec<&QuotaPayload> = rows.iter().collect();
    ordered.sort_by_key(|p| rank(p));
    let rows: Vec<PanelRow> = ordered
        .into_iter()
        .map(|p| {
            let short = rank(p) == 0;
            let remaining = p.remaining.min(100);
            PanelRow {
                model: false,
                label: match rank(p) {
                    0 => limit_label(Some(&p.reset_at), "Limite 5 heures"),
                    1 => limit_label(Some(&p.reset_at), "Limite hebdomadaire"),
                    2 => "Réserve".into(),
                    _ => quota_label(p),
                },
                remaining: Some(remaining),
                value: value_text(Some(remaining), false, ""),
                reset: reset_phrase(&p.reset_at, short)
                    .or_else(|| {
                        p.reset_label
                            .as_ref()
                            .map(|l| format!("Réinitialisation : {l}"))
                    })
                    .unwrap_or_else(|| "Réinitialisation non communiquée".into()),
            }
        })
        .collect();
    let note = match phase {
        "ready" if rows.is_empty() => {
            Some(("Aucune limite communiquée par Codex.".into(), Tone::Neutral))
        }
        "ready" => None,
        "error" => Some((
            non_empty(message, "Lecture de Codex impossible."),
            Tone::Error,
        )),
        _ => Some((non_empty(message, "Connexion à Codex…"), Tone::Warning)),
    };
    PanelProvider {
        name: "Codex",
        badge: plan,
        rows,
        note,
    }
}

#[cfg(target_os = "macos")]
fn claude_panel(snapshot: &claude::ClaudeSnapshot) -> PanelProvider {
    let rows: Vec<PanelRow> = claude::display_metrics(&snapshot.metrics)
        .iter()
        .map(|m| {
            let remaining = m
                .used_percent
                .filter(|_| m.availability == "available")
                .map(remaining_percent);
            let missing = if m.availability == "unavailable" {
                "En attente"
            } else {
                "—"
            };
            PanelRow {
                model: m.id == "claude:weekly_fable",
                label: if m.id == "claude:weekly_fable" {
                    "Limite Fable".into()
                } else if remaining.is_some() {
                    limit_label(m.reset_at.as_deref(), m.label)
                } else {
                    m.label.to_string()
                },
                remaining,
                value: value_text(remaining, m.exhausted, missing),
                reset: m
                    .reset_at
                    .as_deref()
                    .and_then(|at| reset_phrase(at, m.period_duration_seconds <= 18000))
                    .unwrap_or_else(|| "Réinitialisation non communiquée".into()),
            }
        })
        .collect();
    let note = match snapshot.state {
        "not_configured" | "error" | "unavailable" | "auth_required" => Some((
            non_empty(
                &snapshot.message,
                "Lecture impossible : vérifiez Claude Code.",
            ),
            Tone::Error,
        )),
        "waiting_for_data" => Some(("Lecture via Claude Code…".into(), Tone::Warning)),
        "stale" => Some((
            "Données anciennes, nouvelle lecture en cours.".into(),
            Tone::Warning,
        )),
        _ if rows.is_empty() => Some((
            "Aucune limite communiquée par Claude Code.".into(),
            Tone::Neutral,
        )),
        _ => None,
    };
    PanelProvider {
        name: "Claude",
        badge: snapshot.plan_type.clone(),
        rows,
        note,
    }
}

#[cfg(target_os = "macos")]
fn non_empty(text: &str, fallback: &str) -> String {
    if text.trim().is_empty() {
        fallback.into()
    } else {
        text.trim().into()
    }
}

#[cfg(target_os = "macos")]
fn panel_header(providers: &[PanelProvider], last_refresh: Option<DateTime<Local>>) -> PanelHeader {
    let tones: Vec<Tone> = providers
        .iter()
        .filter_map(|p| p.note.as_ref().map(|n| n.1))
        .collect();
    let (status, tone) = if providers.is_empty() {
        ("Inactif", Tone::Neutral)
    } else if tones.contains(&Tone::Error) {
        ("Erreur", Tone::Error)
    } else if tones.contains(&Tone::Warning) {
        ("Lecture…", Tone::Warning)
    } else {
        ("À jour", Tone::Good)
    };
    let subtitle = if providers.is_empty() {
        "Activez Codex ou Claude dans les Réglages".into()
    } else {
        last_refresh
            .map(|at| at.format("Actualisé à %Hh%M").to_string())
            .unwrap_or_else(|| "En attente des premières données".into())
    };
    PanelHeader {
        subtitle,
        status,
        tone,
    }
}

#[cfg(target_os = "macos")]
fn note_lines(text: &str) -> f64 {
    // ~46 characters per line at 11 pt across the card's inner width.
    (text.chars().count() as f64 / 46.0).ceil().clamp(1.0, 4.0)
}

#[cfg(target_os = "macos")]
fn card_height(provider: &PanelProvider) -> f64 {
    let rows = provider.rows.len() as f64 * PANEL_ROW_HEIGHT;
    let note = provider
        .note
        .as_ref()
        .map(|(text, _)| note_lines(text) * 14.0 + if provider.rows.is_empty() { 0.0 } else { 4.0 })
        .unwrap_or(0.0);
    16.0 + rows + note
}

#[cfg(target_os = "macos")]
fn panel_height(providers: &[PanelProvider]) -> f64 {
    PANEL_HEADER_HEIGHT
        + providers
            .iter()
            .map(|p| 20.0 + card_height(p) + 10.0)
            .sum::<f64>()
        + if providers.is_empty() { 0.0 } else { 2.0 }
}

#[cfg(target_os = "macos")]
fn tone_color(tone: Tone) -> Retained<NSColor> {
    match tone {
        Tone::Good => NSColor::systemGreenColor(),
        Tone::Warning => NSColor::systemOrangeColor(),
        Tone::Error => NSColor::systemRedColor(),
        Tone::Neutral => NSColor::secondaryLabelColor(),
    }
}

#[cfg(target_os = "macos")]
fn panel_label(
    mtm: MainThreadMarker,
    text: &str,
    font: &NSFont,
    color: &NSColor,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(font));
    label.setTextColor(Some(color));
    label.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
    label
}

#[cfg(target_os = "macos")]
fn style_model_label(label: &NSTextField, row: &PanelRow) {
    use objc2::AnyThread;
    use objc2_app_kit::{NSFontAttributeName, NSForegroundColorAttributeName};
    use objc2_foundation::{NSDictionary, NSMutableAttributedString, NSRange};

    let Some(prefix) = row.label.strip_suffix(" Fable") else {
        return;
    };
    let font = NSFont::systemFontOfSize(10.5);
    // Use the exact fill function, including its continuous warning transitions.
    let color = quota_color(row.remaining.unwrap_or(0), true);
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let values: [&AnyObject; 2] = [&font, &color];
    let attributes = NSDictionary::from_slices(&keys, &values);
    let text = NSMutableAttributedString::initWithAttributedString(
        NSMutableAttributedString::alloc(),
        &label.attributedStringValue(),
    );
    unsafe {
        text.addAttributes_range(
            &attributes,
            NSRange::new(prefix.encode_utf16().count() + 1, "Fable".len()),
        );
    }
    label.setAttributedStringValue(&text);
}

#[cfg(target_os = "macos")]
fn rounded_box(mtm: MainThreadMarker, color: &NSColor, radius: f64) -> Retained<NSBox> {
    let shape = NSBox::initWithFrame(mtm.alloc(), frame(0.0, 0.0, 1.0, 1.0));
    shape.setBoxType(NSBoxType::Custom);
    shape.setBorderWidth(0.0);
    shape.setCornerRadius(radius);
    shape.setFillColor(color);
    shape
}

/// Builds the menu panel view. Coordinates are written top-down and flipped.
#[cfg(target_os = "macos")]
fn make_panel_view(
    mtm: MainThreadMarker,
    header: &PanelHeader,
    providers: &[PanelProvider],
) -> Retained<NSView> {
    use objc2_app_kit::{NSFontWeightRegular, NSFontWeightSemibold, NSTextAlignment};
    let height = panel_height(providers);
    let root = NSView::initWithFrame(mtm.alloc(), frame(0.0, 0.0, PANEL_WIDTH, height));
    let place = |view: &NSView, x: f64, top: f64, w: f64, h: f64| {
        view.setFrame(frame(x, height - top - h, w, h));
        root.addSubview(view);
    };
    let (semibold, regular) = unsafe { (NSFontWeightSemibold, NSFontWeightRegular) };

    // Header: name, freshness, status capsule.
    let title = panel_label(
        mtm,
        "Quota Monitor",
        &NSFont::systemFontOfSize_weight(13.0, semibold),
        &NSColor::labelColor(),
    );
    place(&title, 16.0, 13.0, 190.0, 17.0);
    let subtitle = panel_label(
        mtm,
        &header.subtitle,
        &NSFont::systemFontOfSize(11.0),
        &NSColor::secondaryLabelColor(),
    );
    place(&subtitle, 16.0, 31.0, 210.0, 14.0);
    let tone = tone_color(header.tone);
    let capsule_width = header.status.chars().count() as f64 * 6.2 + 20.0;
    let capsule = rounded_box(mtm, &tone.colorWithAlphaComponent(0.15), 9.0);
    place(
        &capsule,
        PANEL_WIDTH - 16.0 - capsule_width,
        18.0,
        capsule_width,
        18.0,
    );
    let status = panel_label(
        mtm,
        header.status,
        &NSFont::systemFontOfSize_weight(10.0, semibold),
        &tone,
    );
    status.setAlignment(NSTextAlignment::Center);
    place(
        &status,
        PANEL_WIDTH - 16.0 - capsule_width,
        20.0,
        capsule_width,
        14.0,
    );

    let mut top = PANEL_HEADER_HEIGHT;
    for provider in providers {
        // Section title, like the uppercase captions of SublerAutomation.
        let section = panel_label(
            mtm,
            &provider.name.to_uppercase(),
            &NSFont::systemFontOfSize_weight(10.0, semibold),
            &NSColor::secondaryLabelColor(),
        );
        place(&section, 16.0, top + 2.0, 150.0, 14.0);
        if let Some(badge) = &provider.badge {
            let mut chars = badge.chars();
            let pretty = chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default();
            let badge = panel_label(
                mtm,
                &pretty,
                &NSFont::systemFontOfSize(10.0),
                &NSColor::secondaryLabelColor(),
            );
            badge.setAlignment(NSTextAlignment::Right);
            place(&badge, PANEL_WIDTH - 16.0 - 120.0, top + 2.0, 120.0, 14.0);
        }
        top += 20.0;

        let card_h = card_height(provider);
        let card = rounded_box(
            mtm,
            &NSColor::systemGrayColor().colorWithAlphaComponent(0.12),
            10.0,
        );
        place(&card, 10.0, top, PANEL_WIDTH - 20.0, card_h);

        let mut row_top = top + 8.0;
        for row in &provider.rows {
            // Composite the complete row once: track, fill, value, label and reset.
            let group = NSView::initWithFrame(
                mtm.alloc(),
                frame(
                    0.0,
                    height - row_top - PANEL_ROW_HEIGHT,
                    PANEL_WIDTH,
                    PANEL_ROW_HEIGHT,
                ),
            );
            root.addSubview(&group);
            let place = |view: &NSView, x: f64, top: f64, w: f64, h: f64| {
                view.setFrame(frame(x, PANEL_ROW_HEIGHT - (top - row_top) - h, w, h));
                group.addSubview(view);
            };
            let low = row.remaining.is_some_and(|p| p <= 20);
            let label = panel_label(
                mtm,
                &row.label,
                &NSFont::systemFontOfSize_weight(12.0, regular),
                &NSColor::labelColor(),
            );
            if row.model {
                style_model_label(&label, row);
            }
            place(&label, 20.0, row_top + 3.0, 180.0, 16.0);
            let value_color = if low {
                NSColor::systemRedColor()
            } else {
                NSColor::labelColor()
            };
            let value = panel_label(
                mtm,
                &row.value,
                &NSFont::monospacedDigitSystemFontOfSize_weight(13.0, semibold),
                &value_color,
            );
            value.setAlignment(NSTextAlignment::Right);
            place(
                &value,
                24.0 + PANEL_INNER_WIDTH - 96.0,
                row_top + 1.0,
                96.0,
                18.0,
            );

            let track = rounded_box(
                mtm,
                &NSColor::systemGrayColor().colorWithAlphaComponent(0.28),
                2.5,
            );
            place(&track, 22.0, row_top + 23.0, PANEL_INNER_WIDTH, 5.0);
            // A sub-radius fill renders as a pinched artefact: 0 % shows the track only.
            let fill_width =
                PANEL_INNER_WIDTH * f64::from(row.remaining.unwrap_or(0).min(100)) / 100.0;
            if fill_width >= 5.0 {
                let fill = rounded_box(
                    mtm,
                    &quota_color(row.remaining.unwrap_or(0), row.model),
                    2.5,
                );
                place(&fill, 22.0, row_top + 23.0, fill_width, 5.0);
            }
            let reset = panel_label(
                mtm,
                &row.reset,
                &NSFont::systemFontOfSize(10.5),
                &NSColor::secondaryLabelColor(),
            );
            place(&reset, 20.0, row_top + 31.0, PANEL_INNER_WIDTH, 13.0);
            row_top += PANEL_ROW_HEIGHT;
        }
        if let Some((text, tone)) = &provider.note {
            if !provider.rows.is_empty() {
                row_top += 4.0;
            }
            let note = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
            note.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            note.setTextColor(Some(&tone_color(*tone)));
            note.setSelectable(false);
            place(
                &note,
                20.0,
                row_top,
                PANEL_INNER_WIDTH,
                note_lines(text) * 14.0,
            );
        }
        top += card_h + 10.0;
    }
    root
}

#[cfg(target_os = "macos")]
fn latest_refresh(
    rows: &[QuotaPayload],
    claude_snap: Option<&claude::ClaudeSnapshot>,
) -> Option<DateTime<Local>> {
    let parse = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|d| d.with_timezone(&Local))
    };
    let codex = rows.iter().filter_map(|r| parse(&r.checked_at)).max();
    let claude = claude_snap
        .and_then(|s| s.observed_at.as_deref())
        .and_then(parse);
    codex.into_iter().chain(claude).max()
}

/// The menu is being tracked: refresh its panel in place instead of swapping menus.
#[cfg(target_os = "macos")]
static MENU_OPEN: AtomicBool = AtomicBool::new(false);
/// Panel item of the menu currently attached to the status item (main thread only).
#[cfg(target_os = "macos")]
static PANEL_ITEM: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "macos")]
fn update_status_button(
    native: &NativeQuotaMenu,
    mtm: MainThreadMarker,
    indicator: &indicator::Indicator,
) {
    unsafe {
        let button = &*(native.status_button as *const objc2_app_kit::NSStatusBarButton);
        if indicator.stack.is_empty() {
            button.setImage(mascot_image(mtm).as_deref());
            button.setTitle(&NSString::from_str(&indicator.title));
        } else {
            button.setImage(Some(&stacked_status_image(mtm, &indicator.stack)));
            button.setTitle(&NSString::from_str(""));
        }
        button.setToolTip(Some(&NSString::from_str(&indicator.tooltip)));
    }
}

#[cfg(target_os = "macos")]
fn render_native_menu(app: &tauri::AppHandle) {
    let app = app.clone();
    let handle = app.clone();
    let _ = handle.run_on_main_thread(move || {
        let Some(native) = app
            .state::<NativeMenuState>()
            .0
            .lock()
            .ok()
            .and_then(|v| *v)
        else {
            return;
        };
        let snapshots = get_quota_snapshots(app.clone());
        let prefs = prefs::current(&app);
        let claude_snap = claude::snapshot(&app);
        let claude_visible = claude_is_visible(&claude_snap);
        let codex_status = app
            .try_state::<codex::Provider>()
            .map(|provider| provider.status());
        let connection_phase = codex_status
            .as_ref()
            .map(|status| status.phase.clone())
            .unwrap_or_else(|| "starting".into());
        let mtm = MainThreadMarker::new().expect("native menu on main thread");
        let mut providers = Vec::new();
        if prefs.codex_enabled {
            providers.push(codex_panel(
                &snapshots,
                &connection_phase,
                codex_status
                    .as_ref()
                    .map(|s| s.message.as_str())
                    .unwrap_or(""),
                codex_status.as_ref().and_then(|s| s.plan_type.clone()),
            ));
        }
        if claude_visible {
            providers.push(claude_panel(&claude_snap));
        }
        let header = panel_header(
            &providers,
            latest_refresh(&snapshots, claude_visible.then_some(&claude_snap)),
        );
        let indicator = indicator_for(&app, &snapshots, &claude_snap);
        let panel_view = make_panel_view(mtm, &header, &providers);
        let open_item = PANEL_ITEM.load(Ordering::Acquire);
        if MENU_OPEN.load(Ordering::Acquire) && open_item != 0 {
            // Live update while the user keeps the menu open.
            unsafe { (&*(open_item as *const NSMenuItem)).setView(Some(&panel_view)) };
            update_status_button(&native, mtm, &indicator);
            return;
        }
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        let target = unsafe { &*(native.target as *const QuotaMenuTarget) };
        menu.setDelegate(Some(ProtocolObject::from_ref(target)));
        let panel = NSMenuItem::new(mtm);
        panel.setView(Some(&panel_view));
        menu.addItem(&panel);
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        let add_action = |title: &str, action: objc2::runtime::Sel, key: &str, enabled: bool| {
            let item = NSMenuItem::new(mtm);
            item.setTitle(&NSString::from_str(title));
            item.setKeyEquivalent(&NSString::from_str(key));
            item.setEnabled(enabled);
            unsafe {
                item.setTarget(Some(target));
                item.setAction(Some(action));
            }
            menu.addItem(&item);
        };
        if prefs.codex_enabled && connection_phase != "ready" {
            let logging_in = connection_phase == "logging_in";
            add_action(
                if logging_in {
                    "Connexion en cours…"
                } else {
                    "Se connecter à Codex…"
                },
                objc2::sel!(connectAccount:),
                "",
                !logging_in,
            );
        }
        add_action("Réglages…", objc2::sel!(openSettings:), ",", true);
        add_action(
            "Vérifier les mises à jour…",
            objc2::sel!(checkForUpdates:),
            "",
            true,
        );
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        add_action("Quitter Quota Monitor", objc2::sel!(quitApp:), "q", true);
        menu.addItem(&make_version_item(
            mtm,
            &app.package_info().version.to_string(),
        ));
        unsafe {
            let item = &*(native.status_item as *const objc2_app_kit::NSStatusItem);
            item.setMenu(Some(&menu));
        }
        // The menu (retained by the status item) keeps the panel item alive.
        PANEL_ITEM.store(Retained::as_ptr(&panel) as usize, Ordering::Release);
        update_status_button(&native, mtm, &indicator);
    });
}

fn remaining_percent(used: f64) -> u32 {
    (100.0 - used).clamp(0.0, 100.0).round() as u32
}

fn claude_is_visible(snapshot: &claude::ClaudeSnapshot) -> bool {
    snapshot.enabled
}

#[cfg(any(not(target_os = "macos"), test))]
fn claude_status_line(state: &str) -> Option<&'static str> {
    match state {
        "waiting_for_data" => Some("Claude : en attente de Claude Code…"),
        "stale" => Some("Claude : données anciennes"),
        "not_configured" => Some("Claude : programme introuvable"),
        "error" | "unavailable" | "auth_required" => Some("Claude : erreur de lecture"),
        _ => None,
    }
}

fn short_codex_quota(rows: &[QuotaPayload]) -> Option<&QuotaPayload> {
    rows.iter().find(|row| {
        !is_reserve(row) && (row.window_minutes == Some(300) || row.period == "five-hour")
    })
}

fn codex_display(app: &tauri::AppHandle, rows: &[QuotaPayload]) -> Option<indicator::Display> {
    if !prefs::current(app).codex_enabled {
        return None;
    }
    if let Some(first) = short_codex_quota(rows) {
        return Some(indicator::Display::Percent(first.remaining.min(100)));
    }
    let failed = app
        .try_state::<codex::Provider>()
        .is_some_and(|p| p.status().phase == "error");
    Some(if failed {
        indicator::Display::Error
    } else {
        indicator::Display::Pending
    })
}

fn claude_display(snapshot: &claude::ClaudeSnapshot) -> Option<indicator::Display> {
    if !claude_is_visible(snapshot) {
        return None;
    }
    if matches!(
        snapshot.state,
        "not_configured" | "error" | "unavailable" | "auth_required"
    ) {
        return Some(indicator::Display::Error);
    }
    let short = snapshot
        .metrics
        .first()
        .filter(|m| m.availability == "available")
        .and_then(|m| m.used_percent);
    Some(match short {
        Some(used) => indicator::Display::Percent(remaining_percent(used)),
        None => indicator::Display::Pending,
    })
}

fn indicator_for(
    app: &tauri::AppHandle,
    rows: &[QuotaPayload],
    claude_snap: &claude::ClaudeSnapshot,
) -> indicator::Indicator {
    indicator::compute(
        &prefs::current(app).indicator,
        codex_display(app, rows),
        claude_display(claude_snap),
    )
}

fn quota_label(payload: &QuotaPayload) -> String {
    if is_reserve(payload) {
        return "Réserve".into();
    }
    payload
        .label
        .clone()
        .unwrap_or_else(|| match payload.period.as_str() {
            "five-hour" => "Limite 5 heures".into(),
            "weekly" => "Limite hebdomadaire".into(),
            "reserve-weekly" => "Réserve Luna".into(),
            _ => payload.period.clone(),
        })
}

#[cfg(not(target_os = "macos"))]
fn render_native_menu(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id("quota") {
        let snapshots = get_quota_snapshots(app.clone());
        let indicator = indicator_for(app, &snapshots, &claude::snapshot(app));
        let _ = tray.set_tooltip(Some(indicator.tooltip));
        if let Ok(menu) = desktop_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
        let _ = tray.set_icon(Some(tray_percentage::icon(indicator.icon_value)));
    }
}

#[cfg(target_os = "macos")]
fn frame(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

#[cfg(target_os = "macos")]
fn demo_payload(
    label: &str,
    period: &str,
    remaining: u32,
    reset_at: &str,
    minutes: Option<i64>,
) -> QuotaPayload {
    QuotaPayload {
        used_percent: None,
        model: "codex".into(),
        remaining,
        limit: 100,
        reset_at: reset_at.into(),
        checked_at: Local::now().to_rfc3339(),
        confidence: "demo".into(),
        period: period.into(),
        reset_label: None,
        source: "demo".into(),
        label: Some(label.into()),
        window_minutes: minutes,
    }
}

/// Off-screen render of the real menu panel with demonstration data, in light
/// and dark appearances. Never shows a window (positioned at -20000).
#[cfg(target_os = "macos")]
pub fn quota_ui_smoke(directory: &str) {
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
        NSBackingStoreType, NSBitmapImageFileType, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::NSDictionary;
    let mtm = MainThreadMarker::new().expect("main thread");
    let _app = NSApplication::sharedApplication(mtm);
    std::fs::create_dir_all(directory).unwrap();
    let soon = (Local::now() + chrono::Duration::minutes(95)).to_rfc3339();
    let week = (Local::now() + chrono::Duration::days(4)).to_rfc3339();
    let codex_rows = vec![
        demo_payload("Limite 5 heures", "five-hour", 63, &soon, Some(300)),
        demo_payload("Limite hebdomadaire", "weekly", 85, &week, Some(10080)),
        demo_payload("Réserve", "reserve-weekly", 0, &week, None),
    ];
    // Stacked menu-bar indicator (two providers), rendered on light and dark bars.
    for (mode, background) in [
        ("light", NSColor::whiteColor()),
        ("dark", NSColor::blackColor()),
    ] {
        let lines = [("Codex", "0%".to_string()), ("Claude", "73%".to_string())];
        let image = stacked_status_image(mtm, &lines);
        let size = image.size();
        let canvas = NSView::initWithFrame(mtm.alloc(), frame(0.0, 0.0, size.width + 12.0, 24.0));
        let bg = rounded_box(mtm, &background, 0.0);
        bg.setFrame(canvas.bounds());
        canvas.addSubview(&bg);
        let view = objc2_app_kit::NSImageView::imageViewWithImage(&image, mtm);
        view.setFrame(frame(6.0, 1.0, size.width, size.height));
        view.setContentTintColor(Some(&*if mode == "dark" {
            NSColor::whiteColor()
        } else {
            NSColor::blackColor()
        }));
        canvas.addSubview(&view);
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                frame(-20000.0, -20000.0, size.width + 12.0, 24.0),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setContentView(Some(&canvas));
        canvas.displayIfNeeded();
        let bitmap = canvas
            .bitmapImageRepForCachingDisplayInRect(canvas.bounds())
            .expect("bitmap");
        canvas.cacheDisplayInRect_toBitmapImageRep(canvas.bounds(), &bitmap);
        let png = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }
        .unwrap();
        assert!(png.writeToFile_atomically(
            &NSString::from_str(&format!("{directory}/status-stacked-{mode}.png")),
            true
        ));
        println!("NATIVE_STATUS_PASS {mode}: {}x{}", size.width, size.height);
    }
    // Exercise the production projection, including the duplicate weekly window.
    let claude = claude_panel(&claude::usage_snapshot(
        &serde_json::json!({"subscription_type":"pro", "rate_limits":{
            "five_hour":{"utilization":88,"resets_at":soon},
            "seven_day":{"utilization":37,"resets_at":week},
            "model_scoped":[{"display_name":"Fable","utilization":37,"resets_at":week}]
        }}),
        Local::now().timestamp(),
    ));
    assert_eq!(claude.rows.len(), 2);
    assert_eq!(claude.rows[1].label, "Limite Fable");
    let scenarios: Vec<(&str, Vec<PanelProvider>)> = vec![
        (
            "two-providers",
            vec![
                codex_panel(&codex_rows, "ready", "", Some("plus".into())),
                claude.clone(),
            ],
        ),
        (
            "codex-error",
            vec![codex_panel(
                &[],
                "error",
                "Codex a refusé la lecture des quotas : reconnectez votre compte.",
                None,
            )],
        ),
        (
            "claude-waiting",
            vec![PanelProvider {
                rows: vec![],
                note: Some(("Lecture via Claude Code…".into(), Tone::Warning)),
                ..claude
            }],
        ),
        ("none", vec![]),
    ];
    for (name, providers) in scenarios {
        let header = panel_header(&providers, (!providers.is_empty()).then(Local::now));
        for (mode, appearance) in [
            ("light", unsafe { NSAppearanceNameAqua }),
            ("dark", unsafe { NSAppearanceNameDarkAqua }),
        ] {
            let panel = make_panel_view(mtm, &header, &providers);
            let bounds = panel.bounds();
            assert_eq!(bounds.size.width, PANEL_WIDTH);
            assert_eq!(bounds.size.height, panel_height(&providers));
            let background = rounded_box(mtm, &NSColor::windowBackgroundColor(), 0.0);
            background.setFrame(bounds);
            let content = NSView::initWithFrame(mtm.alloc(), bounds);
            content.addSubview(&background);
            content.addSubview(&panel);
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    mtm.alloc(),
                    frame(-20000.0, -20000.0, bounds.size.width, bounds.size.height),
                    NSWindowStyleMask::Borderless,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            unsafe { window.setReleasedWhenClosed(false) };
            window.setAppearance(NSAppearance::appearanceNamed(appearance).as_deref());
            window.setContentView(Some(&content));
            content.layoutSubtreeIfNeeded();
            content.displayIfNeeded();
            let bitmap = content
                .bitmapImageRepForCachingDisplayInRect(bounds)
                .expect("bitmap");
            content.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
            let png = unsafe {
                bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }
            .unwrap();
            let path = format!("{directory}/menu-{name}-{mode}.png");
            assert!(png.writeToFile_atomically(&NSString::from_str(&path), true));
            let pdf = content.dataWithPDFInsideRect(bounds);
            assert!(pdf.writeToFile_atomically(
                &NSString::from_str(&format!("{directory}/menu-{name}-{mode}.pdf")),
                true
            ));
            println!(
                "NATIVE_LAYOUT_PASS {name} {mode}: {}x{}",
                bounds.size.width, bounds.size.height
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn mascot_image(mtm: MainThreadMarker) -> Option<Retained<NSImage>> {
    let bytes = include_bytes!("../icons/mascot-menu.png");
    let data = unsafe { NSData::dataWithBytes_length(bytes.as_ptr().cast(), bytes.len()) };
    let icon = NSImage::initWithData(mtm.alloc(), &data)?;
    icon.setSize(NSSize::new(18.0, 18.0));
    icon.setTemplate(true);
    Some(icon)
}

/// Two providers: mascot + two stacked lines ("Codex 0%" over "Claude 73%"),
/// full names, right-aligned values. Template image so it follows the menu
/// bar appearance; drawn lazily at the screen's scale.
#[cfg(target_os = "macos")]
fn stacked_status_image(
    mtm: MainThreadMarker,
    lines: &[(&'static str, String)],
) -> Retained<NSImage> {
    use objc2_app_kit::{
        NSFontAttributeName, NSFontWeightMedium, NSFontWeightSemibold,
        NSForegroundColorAttributeName, NSStringDrawing,
    };
    use objc2_foundation::{NSAttributedStringKey, NSDictionary};
    let attrs = |weight| {
        let font = NSFont::monospacedDigitSystemFontOfSize_weight(9.5, weight);
        let color = NSColor::blackColor();
        let keys: [&NSAttributedStringKey; 2] =
            unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
        let values: [&AnyObject; 2] = [&font, &color];
        NSDictionary::from_slices(&keys, &values)
    };
    let (name_attrs, value_attrs) =
        unsafe { (attrs(NSFontWeightMedium), attrs(NSFontWeightSemibold)) };
    let texts: Vec<(Retained<NSString>, Retained<NSString>)> = lines
        .iter()
        .map(|(name, value)| (NSString::from_str(name), NSString::from_str(value)))
        .collect();
    let width_of = |text: &NSString, a: &NSDictionary<NSAttributedStringKey, AnyObject>| unsafe {
        text.sizeWithAttributes(Some(a)).width.ceil()
    };
    let name_w = texts
        .iter()
        .map(|(n, _)| width_of(n, &name_attrs))
        .fold(0.0, f64::max);
    let value_w = texts
        .iter()
        .map(|(_, v)| width_of(v, &value_attrs))
        .fold(0.0, f64::max);
    let icon = mascot_image(mtm);
    let text_x = if icon.is_some() { 21.0 } else { 0.0 };
    let size = NSSize::new(text_x + name_w + 4.0 + value_w, 22.0);
    let handler = block2::RcBlock::new(move |_rect: NSRect| -> objc2::runtime::Bool {
        if let Some(icon) = &icon {
            icon.drawInRect(frame(0.0, 2.0, 18.0, 18.0));
        }
        for (index, (name, value)) in texts.iter().enumerate() {
            let y = if index == 0 { 11.0 } else { 0.5 };
            unsafe {
                name.drawAtPoint_withAttributes(NSPoint::new(text_x, y), Some(&name_attrs));
                let vw = value.sizeWithAttributes(Some(&value_attrs)).width;
                value.drawAtPoint_withAttributes(
                    NSPoint::new(size.width - vw, y),
                    Some(&value_attrs),
                );
            }
        }
        objc2::runtime::Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(size, false, &handler);
    image.setTemplate(true);
    image
}

#[cfg(target_os = "macos")]
fn make_version_item(mtm: MainThreadMarker, version: &str) -> Retained<NSMenuItem> {
    let container = NSView::initWithFrame(mtm.alloc(), frame(0.0, 0.0, 280.0, 24.0));
    let label = NSTextField::labelWithString(
        &NSString::from_str(&format!("Quota Monitor v{version}")),
        mtm,
    );
    label.setFrame(frame(16.0, 5.0, 248.0, 14.0));
    label.setFont(Some(&NSFont::systemFontOfSize(10.0)));
    label.setTextColor(Some(&NSColor::tertiaryLabelColor()));
    label.setAlignment(objc2_app_kit::NSTextAlignment::Center);
    container.addSubview(&label);

    let item = NSMenuItem::new(mtm);
    item.setView(Some(&container));
    item.setEnabled(false);
    item
}

#[cfg(target_os = "macos")]
fn build_native_menu(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let mtm = MainThreadMarker::new().expect("menu setup on main thread");
    let target = QuotaMenuTarget::new(mtm);
    let status_item =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    let button = status_item.button(mtm).expect("status item button");
    button.setImage(mascot_image(mtm).as_deref());
    *app.state::<NativeMenuState>().0.lock().unwrap() = Some(NativeQuotaMenu {
        status_item: Retained::as_ptr(&status_item) as usize,
        status_button: Retained::as_ptr(&button) as usize,
        target: Retained::as_ptr(&target) as usize,
    });
    // The status item and its target live for the lifetime of this menu-bar app.
    std::mem::forget(status_item);
    std::mem::forget(target);
    render_native_menu(app.handle());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn show_dashboard(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        request_quota_refresh(app.clone());
    }
}

#[cfg(not(target_os = "macos"))]
fn desktop_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    let menu = Menu::new(app)?;
    let rows = get_quota_snapshots(app.clone());
    let prefs = prefs::current(app);
    let claude_snap = claude::snapshot(app);
    if !prefs.codex_enabled && !claude_is_visible(&claude_snap) {
        menu.append(&MenuItem::new(
            app,
            "Aucun fournisseur actif",
            false,
            None::<&str>,
        )?)?;
    }
    if prefs.codex_enabled && rows.is_empty() {
        menu.append(&MenuItem::new(
            app,
            "Codex · Quotas indisponibles",
            false,
            None::<&str>,
        )?)?;
    }
    for row in rows {
        menu.append(&MenuItem::new(
            app,
            format!(
                "Codex · {} · {} % restants",
                quota_label(&row),
                row.remaining.min(100)
            ),
            false,
            None::<&str>,
        )?)?;
    }
    if claude_is_visible(&claude_snap) {
        if claude_snap.metrics.is_empty() {
            menu.append(&MenuItem::new(
                app,
                format!("Claude · {}", claude_snap.message),
                false,
                None::<&str>,
            )?)?;
        }
        for m in &claude_snap.metrics {
            let text = match (m.availability, m.used_percent) {
                ("available", Some(_)) if m.exhausted => "épuisé".to_string(),
                ("available", Some(u)) => format!("{} % restants", remaining_percent(u)),
                ("unavailable", _) => "réinitialisée · en attente".to_string(),
                _ => "non communiquée".to_string(),
            };
            menu.append(&MenuItem::new(
                app,
                format!("Claude · {} · {text}", m.label),
                false,
                None::<&str>,
            )?)?;
        }
        if let Some(line) = claude_status_line(claude_snap.state) {
            menu.append(&MenuItem::new(app, line, false, None::<&str>)?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    for (id, title) in [
        ("show", "Afficher les quotas"),
        ("refresh", "Actualiser les quotas"),
        ("settings", "Réglages…"),
        ("usage", "Voir les détails sur Codex…"),
        ("update", "Vérifier les mises à jour…"),
        ("quit", "Quitter"),
    ] {
        menu.append(&MenuItem::with_id(app, id, title, true, None::<&str>)?)?;
    }
    menu.append(&MenuItem::new(
        app,
        format!("Quota Monitor v{}", app.package_info().version),
        false,
        None::<&str>,
    )?)?;
    Ok(menu)
}

#[cfg(not(target_os = "macos"))]
fn build_desktop(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let window = app
        .get_webview_window("main")
        .expect("main window was created");
    let menu = desktop_menu(app.handle())?;
    let tray = TrayIconBuilder::with_id("quota")
        .icon(tray_percentage::icon(None))
        .tooltip("Quota Monitor · En attente des quotas")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_dashboard(app),
            "settings" => open_settings_async(app.clone()),
            "refresh" => {
                request_quota_refresh(app.clone());
            }
            "usage" => {
                let _ = open_usage_details(app.clone());
            }
            "update" => {
                tauri::async_runtime::spawn(check_for_available_update(
                    app.clone(),
                    UpdateCheckMode::Interactive,
                ));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_dashboard(tray.app_handle());
            }
        });
    #[cfg(target_os = "windows")]
    let tray = tray.show_menu_on_left_click(false);
    if let Err(error) = tray.build(app) {
        eprintln!("Quota Monitor tray unavailable: {error}");
        // A hidden startup must remain recoverable if tray creation fails.
        let _ = window.show();
    } else {
        // Linux may report success without a visible tray host. Closing its window
        // therefore exits normally; minimizing keeps the bridge running.
        #[cfg(target_os = "windows")]
        window.on_window_event({
            let window = window.clone();
            move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if window.hide().is_ok() {
                        api.prevent_close();
                    }
                }
            }
        });
    }
    #[cfg(not(target_os = "windows"))]
    let _ = window;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(QuotaState::default())
        .manage(UpdateState::default())
        .manage(prefs::PrefsState::default())
        .invoke_handler(tauri::generate_handler![
            ntfy::test_ntfy,
            get_platform,
            get_quota_snapshots,
            request_quota_refresh,
            open_settings,
            prefs::get_preferences,
            prefs::set_preferences,
            claude::get_claude_snapshot,
            claude::request_claude_refresh,
            claude::install_claude_relay,
            claude::uninstall_claude_relay,
            open_usage_details,
            open_claude_usage,
            fit_settings_window,
            get_update_status,
            start_update_check,
            start_update_install,
            dismiss_update_window,
            restart_application,
            codex::get_codex_status,
            codex::set_refresh_settings,
            codex::redeem_codex_reset,
            codex::start_codex_login,
            codex::cancel_codex_login,
            codex::set_codex_path,
            codex::open_codex_install,
            codex::open_codex_login
        ])
        .setup(|app| {
            *app.state::<prefs::PrefsState>().0.lock().unwrap() = prefs::load(app.handle());
            ntfy::start(app.handle().clone());
            #[cfg(target_os = "macos")]
            let _ = APP_HANDLE.set(app.handle().clone());

            #[cfg(not(debug_assertions))]
            tauri::async_runtime::spawn(check_for_available_update(
                app.handle().clone(),
                UpdateCheckMode::Automatic,
            ));

            #[cfg(not(target_os = "macos"))]
            let _window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Quota Monitor")
            .inner_size(500.0, 620.0)
            .min_inner_size(360.0, 420.0)
            .visible(
                !(cfg!(target_os = "windows")
                    && std::env::args_os().any(|arg| arg == "--autostart")),
            )
            .build()?;
            #[cfg(target_os = "macos")]
            app.handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory)?;

            #[cfg(target_os = "macos")]
            {
                app.manage(NativeMenuState::default());
                build_native_menu(app)?;
            }

            #[cfg(not(target_os = "macos"))]
            build_desktop(app)?;

            app.manage(codex::start(app.handle().clone()));
            claude::start_watcher(app.handle().clone());
            Ok(())
        })
        .build(app_context())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Closing the Settings/updater window must not quit the menu-bar app.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            if let tauri::RunEvent::Exit = event {
                if let Some(provider) = app.try_state::<codex::Provider>() {
                    provider.stop();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicator_uses_short_window_even_when_reserve_is_first() {
        let rows = codex::normalize(&serde_json::json!({"rateLimitsByLimitId": {
            "codex": {"primary":{"usedPercent":100,"windowDurationMins":300},"secondary":{"usedPercent":10,"windowDurationMins":10080}},
            "base_model_inference":{"primary":{"usedPercent":30,"windowDurationMins":10080}}
        }})).unwrap();
        let rows = visible_quotas(rows, Some("plus"));
        assert!(is_reserve(&rows[0]));
        assert_eq!(short_codex_quota(&rows).map(|r| r.remaining), Some(0));
    }

    #[test]
    fn weekly_only_does_not_impersonate_short_quota() {
        let rows = codex::normalize(&serde_json::json!({"rateLimits": {
            "primary":{"usedPercent":12,"windowDurationMins":10080}
        }}))
        .unwrap();
        assert!(short_codex_quota(&rows).is_none());
    }

    #[test]
    fn plus_switches_from_five_hour_to_reserve() {
        let mut rows = codex::normalize(&serde_json::json!({"rateLimitsByLimitId": {
            "codex": {"primary": {"usedPercent":20,"windowDurationMins":300}, "secondary":{"usedPercent":10,"windowDurationMins":10080}},
            "base_model_inference": {"primary":{"usedPercent":30,"windowDurationMins":10080}}
        }})).unwrap();
        let selected = visible_quotas(rows.clone(), Some("plus"));
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].window_minutes, Some(300));
        assert_eq!(selected[1].window_minutes, Some(10080));
        assert!(selected[2].period.contains("base_model_inference"));
        assert_eq!(visible_quotas(rows.clone(), Some("pro")).len(), 3);
        rows.iter_mut()
            .find(|row| row.window_minutes == Some(300))
            .unwrap()
            .remaining = 0;
        let selected = visible_quotas(rows.clone(), Some("plus"));
        assert_eq!(selected.len(), 3);
        assert!(selected[0].period.contains("base_model_inference"));
        assert_eq!(selected[1].window_minutes, Some(10080));
        assert_eq!(selected[2].window_minutes, Some(300));
        rows.iter_mut()
            .find(|row| row.window_minutes == Some(300))
            .unwrap()
            .remaining = 1;
        assert_eq!(visible_quotas(rows, Some("plus"))[0].remaining, 1);
    }
    #[test]
    fn missing_reserve_is_not_fabricated() {
        let rows = codex::normalize(&serde_json::json!({"rateLimits":{"primary":{"usedPercent":100,"windowDurationMins":300}}})).unwrap();
        assert_eq!(visible_quotas(rows, Some("plus")).len(), 1);
    }

    #[test]
    fn plus_keeps_both_windows_without_reserve() {
        let rows = codex::normalize(&serde_json::json!({"rateLimits":{
            "primary":{"usedPercent":4,"windowDurationMins":300},
            "secondary":{"usedPercent":35,"windowDurationMins":10080}
        }}))
        .unwrap();
        let selected = visible_quotas(rows, Some("plus"));
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].window_minutes, Some(300));
        assert_eq!(selected[1].window_minutes, Some(10080));
    }

    #[test]
    fn quota_color_blends_between_thresholds() {
        assert_eq!(quota_color_transition(20), ("red", "red", 0.0));
        assert_eq!(quota_color_transition(30), ("red", "orange", 0.5));
        assert_eq!(quota_color_transition(40), ("red", "orange", 1.0));
        assert_eq!(quota_color_transition(50), ("orange", "green", 0.5));
        assert_eq!(quota_color_transition(60), ("orange", "green", 1.0));
        assert_eq!(quota_color_transition(80), ("green", "green", 0.0));
    }

    #[cfg(target_os = "macos")]
    fn payload(period: &str, remaining: u32, minutes: Option<i64>) -> QuotaPayload {
        QuotaPayload {
            used_percent: None,
            model: "codex".into(),
            remaining,
            limit: 100,
            reset_at: (Local::now() + chrono::Duration::hours(2)).to_rfc3339(),
            checked_at: "2026-10-14T23:00:00+02:00".into(),
            confidence: "test".into(),
            period: period.into(),
            reset_label: None,
            source: "test".into(),
            label: None,
            window_minutes: minutes,
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_fable_label_is_exact_with_known_and_unknown_resets() {
        let now = Local::now().timestamp();
        let reset = (Local::now() + chrono::Duration::hours(30)).to_rfc3339();
        let snap = claude::usage_snapshot(
            &serde_json::json!({"rate_limits":{
                "five_hour":{"utilization":10,"resets_at":reset},
                "seven_day":{"utilization":20,"resets_at":reset},
                "model_scoped":[{"display_name":"Fable","utilization":30,"resets_at":reset}]
            }}),
            now,
        );
        let panel = claude_panel(&snap);
        assert_eq!(panel.rows.len(), 3);
        assert!(!panel.rows[0].label.contains("Fable"));
        assert!(!panel.rows[1].label.contains("Fable"));
        assert!(!panel.rows[0].model);
        assert!(!panel.rows[1].model);
        assert!(panel.rows[2].model);
        assert_eq!(panel.rows[2].label, "Limite Fable");
        // A countdown is not a new 30h quota window: duration remains weekly.
        assert_eq!(snap.metrics[2].period_duration_seconds, 604800);
        let unknown = claude::usage_snapshot(
            &serde_json::json!({"rate_limits":{
                "model_scoped":[{"display_name":"Fable","utilization":30}]
            }}),
            now,
        );
        assert_eq!(claude_panel(&unknown).rows[2].label, "Limite Fable");
        let mut duplicate = snap.clone();
        duplicate.metrics[1].used_percent = duplicate.metrics[2].used_percent;
        let merged = claude_panel(&duplicate);
        assert_eq!(merged.rows.len(), 2);
        assert_eq!(merged.rows[1].label, "Limite Fable");
        assert_eq!(merged.rows[1].value, "70 %");
        assert!(merged.rows[1].model);
        assert!(merged.rows[1].reset.contains("Réinitialisation"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn panel_shows_every_codex_limit_without_a_details_window() {
        // Codex may deliver weekly/reserve first: the panel always shows 5 h, then 7 days.
        let rows = [
            payload("reserve-weekly", 0, None),
            payload("weekly", 85, Some(10080)),
            payload("five-hour", 63, Some(300)),
        ];
        let panel = codex_panel(&rows, "ready", "", Some("plus".into()));
        assert_eq!(panel.rows.len(), 3);
        let labels: Vec<_> = panel.rows.iter().map(|r| r.label.as_str()).collect();
        assert!(
            labels[0].starts_with("Limite sur ") && labels[1].starts_with("Limite sur "),
            "{labels:?}"
        );
        assert_eq!(labels[2], "Réserve");
        // Unknown or past reset: the window name stays.
        assert_eq!(
            limit_label(Some("2000-01-01T00:00:00Z"), "Limite 5 heures"),
            "Limite 5 heures"
        );
        assert_eq!(
            limit_label(None, "Limite hebdomadaire"),
            "Limite hebdomadaire"
        );
        assert_eq!(panel.rows[0].value, "63 %");
        assert!(panel.rows[0].reset.starts_with("Réinitialisation à "));
        assert!(panel.rows[1].reset.starts_with("Réinitialisation le "));
        assert_eq!(panel.rows[2].value, "Épuisé");
        assert_eq!(panel.note, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn panel_header_surfaces_the_worst_provider_state() {
        let ok = codex_panel(&[payload("five-hour", 50, Some(300))], "ready", "", None);
        let failing = codex_panel(&[], "error", "Compte déconnecté", None);
        let waiting = codex_panel(&[], "starting", "", None);
        assert_eq!(panel_header(&[ok.clone()], None).status, "À jour");
        assert_eq!(
            panel_header(&[ok.clone(), waiting.clone()], None).status,
            "Lecture…"
        );
        assert_eq!(
            panel_header(&[waiting, failing.clone()], None).status,
            "Erreur"
        );
        assert_eq!(
            failing.note,
            Some(("Compte déconnecté".into(), Tone::Error))
        );
        assert_eq!(panel_header(&[], None).status, "Inactif");
        assert_eq!(
            panel_header(&[ok], None).subtitle,
            "En attente des premières données"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn panel_height_grows_with_rows_and_notes() {
        let one = codex_panel(&[payload("five-hour", 50, Some(300))], "ready", "", None);
        let two = codex_panel(
            &[
                payload("five-hour", 50, Some(300)),
                payload("weekly", 80, Some(10080)),
            ],
            "ready",
            "",
            None,
        );
        assert_eq!(
            panel_height(&[two.clone()]) - panel_height(&[one]),
            PANEL_ROW_HEIGHT
        );
        assert!(panel_height(&[two.clone(), two]) > PANEL_HEADER_HEIGHT * 2.0);
    }

    #[test]
    fn enabled_claude_remains_visible_when_binary_is_missing() {
        let missing = claude::compute(true, false, claude::FileState::Missing, 1_700_000_000);
        assert_eq!(missing.state, "not_configured");
        assert!(claude_is_visible(&missing));
        let off = claude::compute(false, false, claude::FileState::Missing, 1_700_000_000);
        assert!(!claude_is_visible(&off));
    }

    #[test]
    fn remaining_span_is_compact() {
        assert_eq!(remaining_span(98 * 60), "1h38");
        assert_eq!(remaining_span((5 * 24 + 12) * 3600), "5j 12h");
        assert_eq!(remaining_span(7 * 24 * 3600), "7j");
        assert_eq!(remaining_span(42 * 60 - 30), "42 min");
        assert_eq!(remaining_span(3600), "1h00");
    }
}
