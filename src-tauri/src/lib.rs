#[cfg(target_os = "macos")]
mod account_menu;
mod activity;
mod claude;
mod claude_accounts;
mod claude_cli;
mod codex;
mod codex_accounts;
mod display_mode;
mod indicator;
#[cfg(target_os = "macos")]
mod menu_fixture;
#[cfg(target_os = "macos")]
pub use menu_fixture::menu_tracking_fixture;
mod ntfy;
mod prefs;
mod settings_focus;
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
        .inner_size(420.0, 180.0)
        .min_inner_size(360.0, 120.0)
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
    update_status(
        &app,
        "checking",
        "Préparation du téléchargement…",
        None,
        None,
    );
    let update = match app.updater() {
        // Refresh the release metadata using the existing signed updater.
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
        None,
        Some(next_version.clone()),
    );
    let progress_app = app.clone();
    let progress_version = next_version.clone();
    let mut downloaded = 0_u64;
    let mut last_progress = None;
    let install_app = app.clone();
    let install_version = next_version.clone();
    let result = update
        .download_and_install(
            move |chunk_length, content_length| {
                downloaded = downloaded.saturating_add(chunk_length as u64);
                let progress = download_progress(downloaded, content_length);
                if progress != last_progress {
                    last_progress = progress;
                    update_status(
                        &progress_app,
                        "downloading",
                        format!("Téléchargement de Quota Monitor v{progress_version}…"),
                        progress,
                        Some(progress_version.clone()),
                    );
                }
            },
            move || {
                update_status(
                    &install_app,
                    "installing",
                    "Téléchargement reçu. Vérification et installation…",
                    None,
                    Some(install_version.clone()),
                )
            },
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

// Reserve 100% for the successful download/verification/installation result.
fn download_progress(downloaded: u64, total: Option<u64>) -> Option<u8> {
    total
        .filter(|total| *total > 0)
        .map(|total| ((u128::from(downloaded) * 100 / u128::from(total)).min(99)) as u8)
}

#[cfg(test)]
mod update_progress_tests {
    use super::download_progress;

    #[test]
    fn byte_progress_is_bounded_and_unknown_totals_stay_indeterminate() {
        assert_eq!(download_progress(0, None), None);
        assert_eq!(download_progress(1024, Some(0)), None);
        assert_eq!(download_progress(0, Some(100)), Some(0));
        assert_eq!(download_progress(42, Some(100)), Some(42));
        assert_eq!(download_progress(100, Some(100)), Some(99));
        assert_eq!(download_progress(u64::MAX, Some(1)), Some(99));
        assert_eq!(download_progress(u64::MAX / 2, Some(u64::MAX)), Some(49));
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
            if let Some(app) = APP_HANDLE.get() {
                // Countdowns are computed at render time: redraw on open.
                render_native_menu(app);
                request_quota_refresh(app.clone());
            }
        }

        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, _menu: &NSMenu) {
            MENU_OPEN.store(false, Ordering::Release);
            // Catch up on a panel resize deferred while an action was highlighted.
            if PANEL_DEFERRED.swap(false, Ordering::AcqRel) {
                if let Some(app) = APP_HANDLE.get() {
                    render_native_menu(app);
                }
            }
        }
    }

    impl QuotaMenuTarget {
        #[unsafe(method(quitApp:))]
        fn quit_app(&self, _sender: Option<&AnyObject>) {
            if let Some(app) = APP_HANDLE.get() { app.exit(0); }
        }

        #[unsafe(method(connectAccount:))]
        fn connect_account(&self, _sender: Option<&AnyObject>) {
            if let Some(app) = APP_HANDLE.get() { let _ = connect_codex_account(app.clone()); }
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

/// Smallest standalone window height (logical px, title bar included).
const MAIN_WINDOW_MIN_HEIGHT: f64 = 160.0;

/// Fits the standalone window height to the panel content (logical px),
/// bounded by the visible screen. Width is untouched; the top edge stays put.
#[tauri::command]
fn fit_main_window(
    window: tauri::WebviewWindow,
    height: f64,
    viewport_height: f64,
) -> Result<(), String> {
    if window.label() != "main"
        || !height.is_finite()
        || !viewport_height.is_finite()
        || viewport_height <= 0.0
    {
        return Ok(());
    }
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let inner = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);
    let monitor = window.current_monitor().map_err(|e| e.to_string())?;
    let max = monitor
        .as_ref()
        .map(|m| m.work_area().size.to_logical::<f64>(scale).height - 40.0)
        .unwrap_or(900.0)
        .max(MAIN_WINDOW_MIN_HEIGHT);
    let wanted = update_window_height(height, viewport_height, inner.height, max)
        .max(MAIN_WINDOW_MIN_HEIGHT);
    if (inner.height - wanted).abs() <= 1.0 {
        return Ok(());
    }
    let before = window.outer_position().map_err(|e| e.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(inner.width, wanted))
        .map_err(|e| e.to_string())?;
    // Keep the top edge where it was (AppKit resizes from the bottom-left),
    // then keep the window on the visible area if it now overflows.
    let mut y = before.y;
    if let Some(monitor) = monitor {
        let area = monitor.work_area();
        let size = window.outer_size().map_err(|e| e.to_string())?;
        let bottom = area.position.y + area.size.height as i32;
        y = y.min(bottom - size.height as i32).max(area.position.y);
    }
    let after = window.outer_position().map_err(|e| e.to_string())?;
    if after.x != before.x || after.y != y {
        window
            .set_position(tauri::PhysicalPosition::new(before.x, y))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Fits intrinsic updater content, bounded by the monitor; overflow scrolls.
#[tauri::command]
fn fit_update_window(
    window: tauri::WebviewWindow,
    height: f64,
    viewport_height: f64,
) -> Result<(), String> {
    if window.label() != "updater"
        || !height.is_finite()
        || !viewport_height.is_finite()
        || viewport_height <= 0.0
    {
        return Ok(());
    }
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let inner = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);
    let monitor = window.current_monitor().map_err(|e| e.to_string())?;
    let max = monitor
        .as_ref()
        .map(|m| m.work_area().size.to_logical::<f64>(scale).height - 40.0)
        .unwrap_or(900.0)
        .max(120.0);
    // On macOS Tauri's reported inner height includes the native titlebar,
    // while the WebView viewport does not. Preserve that measured inset;
    // otherwise even short content is clipped below the window's bottom.
    let wanted = update_window_height(height, viewport_height, inner.height, max);
    if (inner.height - wanted).abs() > 1.0 {
        window
            .set_size(tauri::LogicalSize::new(inner.width, wanted))
            .map_err(|e| e.to_string())?;
    }
    if let Some(monitor) = monitor {
        let area = monitor.work_area();
        let position = window.outer_position().map_err(|e| e.to_string())?;
        let size = window.outer_size().map_err(|e| e.to_string())?;
        let bottom = area.position.y + area.size.height as i32;
        let y = position
            .y
            .min(bottom - size.height as i32)
            .max(area.position.y);
        if y != position.y {
            window
                .set_position(tauri::PhysicalPosition::new(position.x, y))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn update_window_height(content: f64, viewport: f64, inner: f64, max: f64) -> f64 {
    let inset = (inner - viewport).max(0.0);
    (content + inset).clamp(120.0, max)
}

#[cfg(test)]
mod update_window_tests {
    use super::update_window_height;

    #[test]
    fn native_titlebar_inset_is_preserved_without_exceeding_the_screen() {
        assert_eq!(update_window_height(193.0, 161.0, 193.0, 900.0), 225.0);
        assert_eq!(update_window_height(193.0, 193.0, 225.0, 900.0), 225.0);
        assert_eq!(update_window_height(193.0, 193.0, 193.0, 900.0), 193.0);
        assert_eq!(update_window_height(2000.0, 193.0, 225.0, 900.0), 900.0);
        assert_eq!(update_window_height(80.0, 193.0, 225.0, 900.0), 120.0);
    }
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
    pair_slot: Option<usize>,
    /// Model-scoped row (Fable): normal-size model name matching the fill, full opacity.
    model: bool,
    label: String,
    remaining: Option<u32>,
    value: String,
    reset: String,
    /// Time left before the reset ("3h30", "42 min", "3j 8h"), computed from
    /// the structured reset and the local clock; "—" when absent or passed.
    countdown: String,
}

/// Provider, account id and connection state of a card (display only).
#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq)]
struct PanelAccount {
    /// "codex" | "claude"
    kind: &'static str,
    id: String,
    name: String,
    state: String,
    tone: Tone,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq)]
struct PanelProvider {
    /// Which account this card shows (menu/compact routing); `None` for fixtures.
    account: Option<PanelAccount>,
    name: String,
    /// Multi-account card: 5 h and 7 j paired with visible countdowns.
    compact: bool,
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

#[cfg(all(test, target_os = "macos"))]
#[test]
fn codex_compact_countdown_uses_real_reset() {
    let reset = (Local::now() + chrono::Duration::minutes(90)).to_rfc3339();
    let rows = [demo_payload(
        "Limite 5 heures",
        "five-hour",
        63,
        &reset,
        Some(300),
    )];
    let panel = codex_panel(&rows, "ready", "", Some("plus".into()));
    assert_eq!(panel.rows[0].countdown, "1h30");
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn codex_layout_pairs_real_same_bucket_and_preserves_extra_windows() {
    let result = serde_json::json!({"rateLimitsByLimitId":{
        "codex":{"primary":{"usedPercent":100,"windowDurationMins":300},"secondary":{"usedPercent":15,"windowDurationMins":10080}},
        "review":{"secondary":{"usedPercent":73,"windowDurationMins":10080}}
    }});
    let rows = codex::normalize(&result).unwrap();
    let p = codex_presentation(&rows, "ready", "", Some("plus".into()), "compact");
    assert!(p.compact);
    assert_eq!(paired_panel_rows(&p).len(), 2);
    assert_eq!(p.rows.len(), 3);
    assert_eq!(p.rows[0].remaining, Some(0));
    let classic = codex_presentation(&rows, "ready", "", None, "classic");
    assert!(!classic.compact);
    assert_eq!(
        card_height(&classic) - card_height(&p),
        2.0 * PANEL_ROW_HEIGHT - PAIR_BLOCK_HEIGHT
    );
    let weekly = codex_presentation(&rows[1..], "ready", "", None, "compact");
    assert_eq!(paired_panel_rows(&weekly).len(), 1);
    assert_eq!(paired_panel_rows(&weekly)[0].pair_slot, Some(1));
    let model = PanelModel {
        addable: Vec::new(),
        header: panel_header(&[p.clone()], None),
        providers: vec![p],
        connection: None,
    };
    let compact = compact_panel(&model, "fixture", None);
    assert_eq!(
        compact.providers[0]
            .rows
            .iter()
            .filter(|r| r.pair_slot.is_some())
            .count(),
        2
    );
    assert_eq!(compact.providers[0].rows.len(), 3);
    for phase in ["ready", "error", "starting"] {
        let empty = codex_presentation(&[], phase, "", None, "compact");
        assert!(empty.rows.is_empty());
        assert!(paired_panel_rows(&empty).is_empty());
        assert!(empty.note.is_some());
    }
    let crossed = codex_presentation(
        &[rows[0].clone(), rows[2].clone()],
        "error",
        "Lecture impossible",
        None,
        "compact",
    );
    assert_eq!(paired_panel_rows(&crossed).len(), 1);
    assert_eq!(crossed.rows.len(), 2);
    assert_eq!(crossed.note.unwrap().1, Tone::Error);
}

#[cfg(target_os = "macos")]
fn codex_panel(
    rows: &[QuotaPayload],
    phase: &str,
    message: &str,
    plan: Option<String>,
) -> PanelProvider {
    let windows: Vec<_> = rows.iter().filter_map(codex_window).collect();
    let bucket = windows
        .iter()
        .find(|(bucket, _)| bucket.starts_with("codex:codex:"))
        .or_else(|| windows.first())
        .map(|(bucket, _)| bucket);
    let mut slots = [false; 2];
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
                pair_slot: codex_window(p).and_then(|(key, slot)| {
                    if Some(&key) == bucket && !slots[slot] {
                        slots[slot] = true;
                        Some(slot)
                    } else {
                        None
                    }
                }),
                model: false,
                label: match rank(p) {
                    0 => limit_label(Some(&p.reset_at), "Limite 5 heures"),
                    1 => limit_label(Some(&p.reset_at), "Limite hebdomadaire"),
                    2 => "Réserve".into(),
                    _ => quota_label(p),
                },
                remaining: Some(remaining),
                value: value_text(Some(remaining), false, ""),
                countdown: reset_countdown(Some(&p.reset_at), Local::now().timestamp()),
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
        account: None,
        name: "Codex".into(),
        compact: false,
        badge: plan,
        rows,
        note,
    }
}

/// Only windows from one rate-limit bucket/model enter the paired block.
/// Extra buckets and unknown durations stay as complete historical rows.
#[cfg(target_os = "macos")]
fn codex_window(p: &QuotaPayload) -> Option<(String, usize)> {
    if is_reserve(p) {
        return None;
    }
    if p.period == "five-hour" || p.period == "weekly" {
        return Some(("legacy".into(), usize::from(p.period == "weekly")));
    }
    let (bucket, slot) = p.period.rsplit_once(':')?;
    if !bucket.starts_with("codex:") || !matches!(slot, "primary" | "secondary") {
        return None;
    }
    let index = match p.window_minutes {
        Some(300) => 0,
        Some(10080) => 1,
        _ => return None,
    };
    Some((format!("{bucket}:{}", p.model), index))
}

#[cfg(target_os = "macos")]
fn codex_presentation(
    rows: &[QuotaPayload],
    phase: &str,
    message: &str,
    plan: Option<String>,
    layout: &str,
) -> PanelProvider {
    let mut panel = codex_panel(rows, phase, message, plan);
    panel.compact = layout == "compact";
    panel
}

#[cfg(target_os = "macos")]
fn paired_panel_rows(provider: &PanelProvider) -> Vec<&PanelRow> {
    if !provider.compact {
        return vec![];
    }
    if provider.name == "Codex" || provider.name.starts_with("Codex · ") {
        let mut rows: Vec<_> = provider
            .rows
            .iter()
            .filter(|r| r.pair_slot.is_some())
            .collect();
        rows.sort_by_key(|r| r.pair_slot);
        rows
    } else {
        // Claude: the 5 h and 7 j windows (the Fable row never takes a slot).
        let windows: Vec<_> = provider.rows.iter().filter(|r| !r.model).take(2).collect();
        if windows.len() >= 2 {
            windows
        } else {
            vec![]
        }
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
                pair_slot: None,
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
                countdown: reset_countdown(m.reset_at.as_deref(), Local::now().timestamp()),
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
        account: None,
        name: "Claude".into(),
        compact: false,
        badge: snapshot.plan_type.clone(),
        rows,
        note,
    }
}

/// `classic`: Réglages → Affichage « Classique » = the historical separated
/// rows (5 h, hebdo, Fable, resets, percentages); otherwise « Compacte » pairs
/// the 5 h / 7 j bars. The choice applies to one or several visible accounts
/// (Unique, Multi, one visible or more). Never an aggregate. A sole card keeps
/// the plain "Claude" name and no added note.
#[cfg(target_os = "macos")]
fn claude_account_panels(
    accounts: &[claude::AccountSnapshot],
    classic: bool,
) -> Vec<PanelProvider> {
    let visible: Vec<_> = accounts.iter().filter(|a| a.profile.visible).collect();
    let several = visible.len() > 1;
    visible
        .into_iter()
        .map(|a| {
            let mut panel = claude_panel(&a.snapshot);
            panel.compact = !classic;
            let (state, tone) = account_menu::claude_state(a);
            panel.account = Some(PanelAccount {
                kind: "claude",
                id: a.profile.id.clone(),
                name: a.profile.name.clone(),
                state,
                tone,
            });
            if !several {
                return panel;
            }
            panel.name = format!("Claude · {}", a.profile.name);
            if let Some(extra) = &a.snapshot.extra_usage {
                if extra.enabled {
                    let extra_text = if extra.limit_reached {
                        "Usage supplémentaire : plafond atteint"
                    } else {
                        "Usage supplémentaire activé"
                    };
                    panel.note = Some(match panel.note {
                        Some((text, tone)) => (format!("{text} · {extra_text}"), tone),
                        None => (
                            extra_text.into(),
                            if extra.limit_reached {
                                Tone::Warning
                            } else {
                                Tone::Neutral
                            },
                        ),
                    });
                }
            }
            panel
        })
        .collect()
}
#[cfg(target_os = "macos")]
fn compact_tracks() -> ((f64, f64), (f64, f64)) {
    (
        (24.0, PANEL_TRACK_HEIGHT),
        (24.0 + PANEL_TRACK_HEIGHT + 2.0, 3.0),
    )
}

/// Paired 5 h / 7 j block with room for resets; Fable adds only a fine track.
#[cfg(target_os = "macos")]
const PAIR_BLOCK_HEIGHT: f64 = 60.0;

#[cfg(target_os = "macos")]
fn compact_fable(provider: &PanelProvider) -> Option<&PanelRow> {
    if paired_panel_rows(provider).is_empty() {
        return None;
    }
    provider
        .rows
        .iter()
        .find(|row| row.model && row.label.ends_with(" Fable"))
}

#[cfg(target_os = "macos")]
fn pair_block_height(provider: &PanelProvider) -> f64 {
    PAIR_BLOCK_HEIGHT
        + if compact_fable(provider).is_some() {
            5.0
        } else {
            0.0
        }
}

/// Short reset line of a paired window: « Réinit. à 14h30 » / « Réinit. le 03/11 à 19h00 ».
#[cfg(target_os = "macos")]
fn compact_reset(reset: &str) -> String {
    reset.replacen("Réinitialisation", "Réinit.", 1)
}

/// Track/fill thickness of a Unique quota row, measured on v3.1.0 (5 pt,
/// radius 2.5 in make_panel_view).
#[cfg(target_os = "macos")]
const PANEL_TRACK_HEIGHT: f64 = 5.0;

/// Same rounded-up format as `remaining_span`; "—" for absent, invalid or
/// passed resets (never negative, never invented).
#[cfg(target_os = "macos")]
fn reset_countdown(reset_at: Option<&str>, now: i64) -> String {
    reset_at
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.timestamp() - now)
        .filter(|left| *left > 0)
        .map(remaining_span)
        .unwrap_or_else(|| "—".into())
}

/// Visible compact text: live countdown + provider value ("27 %", "Épuisé",
/// missing text). Without a usable reset, only the value is shown.
#[cfg(target_os = "macos")]
fn compact_heading(row: &PanelRow) -> String {
    let text = if row.countdown == "—" {
        row.value.clone()
    } else {
        format!("{} · {}", row.countdown, row.value)
    };
    if row.model {
        format!("Fable · {text}")
    } else {
        text
    }
}

#[cfg(target_os = "macos")]
fn compact_value(row: &PanelRow) -> String {
    match row.remaining {
        Some(0) => "0 % · épuisé".into(),
        Some(n) => format!("{n} % restants"),
        None => row.value.clone(),
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

/// "plus" -> "Plus": plan badge as shown at the right of a provider section.
#[cfg(target_os = "macos")]
fn pretty_badge(badge: &str) -> String {
    let mut chars = badge.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect::<String>())
        .unwrap_or_default()
}

/// Values at or below 20 % remaining are drawn in red.
#[cfg(target_os = "macos")]
fn row_is_low(row: &PanelRow) -> bool {
    row.remaining.is_some_and(|p| p <= 20)
}

/// Gauge fill width on the panel track (logical points).
#[cfg(target_os = "macos")]
fn row_fill_width(row: &PanelRow) -> f64 {
    PANEL_INNER_WIDTH * f64::from(row.remaining.unwrap_or(0).min(100)) / 100.0
}

/// A sub-radius fill renders as a pinched artefact: 0 % shows the track only.
#[cfg(target_os = "macos")]
fn row_fill_visible(row: &PanelRow) -> bool {
    row_fill_width(row) >= 5.0
}

// ---------------------------------------------------------------------------
// Compact standalone window (macOS application mode). It serialises the exact
// panel model of the menu-bar item so both surfaces share labels, order,
// values, notes and gauge colours; the webview only lays it out.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactFill {
    from: &'static str,
    to: &'static str,
    fraction: f64,
    visible: bool,
    /// Width relative to the track, 0..=1.
    ratio: f64,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactRow {
    pair_slot: Option<usize>,
    model: bool,
    label: String,
    remaining: Option<u32>,
    value: String,
    reset: String,
    countdown: String,
    low: bool,
    fill: CompactFill,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactNote {
    text: String,
    tone: &'static str,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactState {
    label: String,
    tone: &'static str,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactProvider {
    /// "codex" | "claude": groups the cards under one provider header.
    kind: &'static str,
    /// Name of that account (the card title under the provider header).
    account_name: Option<String>,
    state: Option<CompactState>,
    name: String,
    section: String,
    /// Same Multi pairing as the native menu card.
    compact: bool,
    badge: Option<String>,
    rows: Vec<CompactRow>,
    note: Option<CompactNote>,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactHeader {
    title: &'static str,
    subtitle: String,
    status: &'static str,
    tone: &'static str,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactConnection {
    label: &'static str,
    logging_in: bool,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactPanel {
    header: CompactHeader,
    providers: Vec<CompactProvider>,
    /// Enabled providers, in order: each gets its header (↗) and its +.
    addable: Vec<&'static str>,
    connection: Option<CompactConnection>,
    version: String,
    /// Last updater state; `None` while no check has reported anything.
    update: Option<UpdateStatus>,
}

#[cfg(target_os = "macos")]
fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Neutral => "neutral",
        Tone::Good => "good",
        Tone::Warning => "warning",
        Tone::Error => "error",
    }
}

#[cfg(target_os = "macos")]
fn connection_label(logging_in: bool) -> &'static str {
    if logging_in {
        "Connexion en cours…"
    } else {
        "Se connecter à Codex…"
    }
}

#[cfg(target_os = "macos")]
fn compact_row(row: &PanelRow) -> CompactRow {
    let (from, to, fraction) = quota_color_transition(row.remaining.unwrap_or(0));
    CompactRow {
        pair_slot: row.pair_slot,
        model: row.model,
        label: row.label.clone(),
        remaining: row.remaining,
        value: row.value.clone(),
        reset: row.reset.clone(),
        countdown: row.countdown.clone(),
        low: row_is_low(row),
        fill: CompactFill {
            from,
            to,
            fraction,
            visible: row_fill_visible(row),
            ratio: row_fill_width(row) / PANEL_INNER_WIDTH,
        },
    }
}

#[cfg(target_os = "macos")]
fn compact_panel(model: &PanelModel, version: &str, update: Option<UpdateStatus>) -> CompactPanel {
    CompactPanel {
        header: CompactHeader {
            title: "Quota Monitor",
            subtitle: model.header.subtitle.clone(),
            status: model.header.status,
            tone: tone_name(model.header.tone),
        },
        providers: model
            .providers
            .iter()
            .map(|p| CompactProvider {
                kind: p.account.as_ref().map_or_else(
                    || {
                        if p.name.starts_with("Claude") {
                            "claude"
                        } else {
                            "codex"
                        }
                    },
                    |a| a.kind,
                ),
                account_name: p.account.as_ref().map(|a| a.name.clone()),
                state: p.account.as_ref().map(|a| CompactState {
                    label: a.state.clone(),
                    tone: tone_name(a.tone),
                }),
                name: p.name.clone(),
                section: p.name.to_uppercase(),
                compact: p.compact,
                badge: p.badge.as_deref().map(pretty_badge),
                rows: p
                    .rows
                    .iter()
                    .map(|r| {
                        let mut row = compact_row(r);
                        row.pair_slot = paired_panel_rows(p)
                            .iter()
                            .position(|paired| std::ptr::eq(*paired, r))
                            .map(|index| r.pair_slot.unwrap_or(index));
                        row
                    })
                    .collect(),
                note: p.note.as_ref().map(|(text, tone)| CompactNote {
                    text: text.clone(),
                    tone: tone_name(*tone),
                }),
            })
            .collect(),
        addable: model.addable.clone(),
        connection: model.connection.map(|logging_in| CompactConnection {
            label: connection_label(logging_in),
            logging_in,
        }),
        version: version.to_string(),
        update,
    }
}

/// Fixtures (smoke, tracking): the providers that have a card offer their +.
#[cfg(target_os = "macos")]
fn addable_from_panels(providers: &[PanelProvider]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for p in providers {
        let kind = p.account.as_ref().map_or_else(
            || {
                if p.name.starts_with("Claude") {
                    "claude"
                } else {
                    "codex"
                }
            },
            |a| a.kind,
        );
        if !out.contains(&kind) {
            out.push(kind);
        }
    }
    out
}

/// Everything the menu-bar panel shows, computed once for both surfaces.
#[cfg(target_os = "macos")]
struct PanelModel {
    /// Providers offering the + of the compact window (the native menu has none).
    addable: Vec<&'static str>,
    header: PanelHeader,
    providers: Vec<PanelProvider>,
    /// `Some(logging_in)` when the Codex connection action is offered.
    connection: Option<bool>,
}

/// Native menu / compact window providers: the ONE saved Compacte/Classique
/// choice (`prefs.layout`) drives every visible Codex and Claude account.
#[cfg(target_os = "macos")]
fn quota_providers(
    prefs: &prefs::Prefs,
    codex: &[codex::CodexAccount],
    accounts: &[claude::AccountSnapshot],
) -> Vec<PanelProvider> {
    let mut providers = Vec::new();
    if prefs.codex_enabled {
        providers.extend(codex_account_panels(codex, &prefs.layout));
    }
    providers.extend(claude_account_panels(
        accounts,
        prefs.classic_claude_layout(),
    ));
    providers
}

/// Same contract as `claude_account_panels`: one card per visible Codex
/// environment, drawn with the saved layout. A sole card keeps the plain
/// historical "Codex" name (and is the exact historical card for the
/// primary); several are named « Codex · {compte} ». Never an aggregate: a
/// home returning the same account as an earlier one is only flagged.
#[cfg(target_os = "macos")]
fn codex_account_panels(accounts: &[codex::CodexAccount], layout: &str) -> Vec<PanelProvider> {
    let visible: Vec<_> = accounts.iter().filter(|a| a.profile.visible).collect();
    let several = visible.len() > 1;
    visible
        .into_iter()
        .map(|a| {
            let mut panel =
                codex_presentation(&a.rows, &a.phase, &a.message, a.plan_type.clone(), layout);
            let (state, tone) = account_menu::codex_state(a);
            panel.account = Some(PanelAccount {
                kind: "codex",
                id: a.profile.id.clone(),
                name: a.profile.name.clone(),
                state,
                tone,
            });
            if several {
                panel.name = format!("Codex · {}", a.profile.name);
            }
            if let Some(other) = &a.duplicate_of {
                let text = format!("Même compte que « {other} » : quotas non additionnés.");
                panel.note = Some(match panel.note {
                    Some((note, tone)) => (format!("{note} · {text}"), tone),
                    None => (text, Tone::Neutral),
                });
            }
            panel
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn panel_model(app: &tauri::AppHandle) -> PanelModel {
    let prefs = prefs::current(app);
    // Every visible Claude profile, drawn with the saved Compacte/Classique choice.
    let claude_all = claude::accounts(app);
    let accounts: Vec<_> = claude_all
        .iter()
        .filter(|a| a.profile.visible && claude_is_visible(&a.snapshot))
        .cloned()
        .collect();
    // Every visible Codex environment (primary = historical collector).
    let codex_accounts = codex::accounts(app);
    let codex_status = app
        .try_state::<codex::Provider>()
        .map(|provider| provider.status());
    let connection_phase = codex_status
        .as_ref()
        .map(|status| status.phase.clone())
        .unwrap_or_else(|| "starting".into());
    let providers = quota_providers(&prefs, &codex_accounts, &accounts);
    let shown_rows: Vec<QuotaPayload> = codex_accounts
        .iter()
        .filter(|a| prefs.codex_enabled && a.profile.visible)
        .flat_map(|a| a.rows.iter().cloned())
        .collect();
    let claude_snaps: Vec<_> = accounts.iter().map(|a| &a.snapshot).collect();
    let header = panel_header(&providers, latest_refresh(&shown_rows, &claude_snaps));
    PanelModel {
        addable: account_menu::addable_providers(&prefs),
        header,
        providers,
        connection: (prefs.codex_enabled && connection_phase != "ready")
            .then_some(connection_phase == "logging_in"),
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn get_compact_panel(app: tauri::AppHandle) -> CompactPanel {
    let update = app
        .state::<UpdateState>()
        .0
        .lock()
        .ok()
        .and_then(|status| status.clone());
    compact_panel(
        &panel_model(&app),
        &app.package_info().version.to_string(),
        update,
    )
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn get_compact_panel() -> Result<(), String> {
    Err("La fenêtre compacte n’existe que sur macOS.".into())
}

/// Native folder chooser for « Suivre un autre compte Claude ». Bounded to the
/// one folder the user picks: returns its path only (no content read, no
/// login, no folder created). The add itself goes through add_claude_profile.
#[cfg(target_os = "macos")]
#[tauri::command]
async fn choose_claude_profile_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let mtm = MainThreadMarker::new().expect("folder picker runs on main thread");
        let _ = tx.send(pick_profile_folder(
            mtm,
            "Choisissez le dossier d’un profil Claude Code déjà connecté (par exemple .claude-travail). Aucun compte n’est créé.",
            "Suivre ce profil",
        ));
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "Sélecteur de dossier interrompu.".to_string())
}

/// Same chooser for « Ajouter un compte » Codex: one existing CODEX_HOME
/// folder, path only (no content read, no login, no folder created).
#[cfg(target_os = "macos")]
#[tauri::command]
async fn choose_codex_profile_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let mtm = MainThreadMarker::new().expect("folder picker runs on main thread");
        let _ = tx.send(pick_profile_folder(
            mtm,
            "Choisissez un dossier CODEX_HOME existant (par exemple .codex-travail). Aucun compte n’est créé ni connecté.",
            "Suivre ce dossier",
        ));
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "Sélecteur de dossier interrompu.".to_string())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn choose_codex_profile_folder() -> Result<Option<String>, String> {
    Err(
        "Sélecteur de dossier indisponible : indiquez le chemin absolu du dossier CODEX_HOME."
            .into(),
    )
}

#[cfg(target_os = "macos")]
fn pick_profile_folder(mtm: MainThreadMarker, message: &str, prompt: &str) -> Option<String> {
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(false);
    // Profiles are usually hidden folders (~/.claude-travail, ~/.codex-travail).
    panel.setShowsHiddenFiles(true);
    panel.setMessage(Some(&NSString::from_str(message)));
    panel.setPrompt(Some(&NSString::from_str(prompt)));
    if let Some(home) = std::env::var_os("HOME") {
        let url =
            objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(&home.to_string_lossy()));
        panel.setDirectoryURL(Some(&url));
    }
    NSApplication::sharedApplication(mtm).activate();
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    panel
        .URL()
        .and_then(|url| url.path())
        .map(|p| p.to_string())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn choose_claude_profile_folder() -> Result<Option<String>, String> {
    Err("Sélecteur de dossier indisponible : indiquez le chemin absolu du profil.".into())
}

/// Same action as the menu's "Se connecter à Codex…" item.
#[tauri::command]
fn connect_codex_account(app: tauri::AppHandle) -> Result<(), String> {
    let provider = app
        .try_state::<codex::Provider>()
        .ok_or_else(|| "Service Codex indisponible.".to_string())?;
    if provider.status().phase == "missing" {
        codex::open_codex_install(app.clone())
    } else {
        codex::start_codex_login(provider, false)
    }
}

#[cfg(target_os = "macos")]
fn note_lines(text: &str) -> f64 {
    // ~46 characters per line at 11 pt across the card's inner width.
    (text.chars().count() as f64 / 46.0).ceil().clamp(1.0, 4.0)
}

#[cfg(target_os = "macos")]
fn card_height(provider: &PanelProvider) -> f64 {
    let paired_count = paired_panel_rows(provider).len();
    let rows = if paired_count > 0 {
        pair_block_height(provider)
            + (provider.rows.len() - paired_count - usize::from(compact_fable(provider).is_some()))
                as f64
                * PANEL_ROW_HEIGHT
    } else {
        provider.rows.len() as f64 * PANEL_ROW_HEIGHT
    };
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
    use objc2_app_kit::NSForegroundColorAttributeName;
    use objc2_foundation::{NSDictionary, NSMutableAttributedString, NSRange};

    let Some(prefix) = row.label.strip_suffix(" Fable") else {
        return;
    };
    // Use the exact fill function, including its continuous warning transitions.
    let color = quota_color(row.remaining.unwrap_or(0), true);
    let keys = unsafe { [NSForegroundColorAttributeName] };
    let values: [&AnyObject; 1] = [&color];
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
            let badge = panel_label(
                mtm,
                &pretty_badge(badge),
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
        // Compacte card: 5 h and 7 j share one block with their countdowns above
        // and their reset line below, independent of any Fable row; the 5 h track/fill keeps the Unique thickness.
        let paired_rows = paired_panel_rows(provider);
        let fable = compact_fable(provider);
        let block_height = pair_block_height(provider);
        if !paired_rows.is_empty() {
            let group = NSView::initWithFrame(
                mtm.alloc(),
                frame(
                    0.0,
                    height - row_top - block_height,
                    PANEL_WIDTH,
                    block_height,
                ),
            );
            root.addSubview(&group);
            let place = |view: &NSView, x: f64, top: f64, w: f64, h: f64| {
                view.setFrame(frame(x, block_height - (top - row_top) - h, w, h));
                group.addSubview(view);
            };
            let tracks = compact_tracks();
            for (position, row) in paired_rows.iter().enumerate() {
                let index = row.pair_slot.unwrap_or(position);
                let window = if index == 0 {
                    "Limite 5 heures"
                } else {
                    "Limite hebdomadaire"
                };
                let tooltip = NSString::from_str(&format!(
                    "{window} · {} · {} · {}",
                    row.label,
                    compact_value(row),
                    row.reset
                ));
                let color = if row_is_low(row) {
                    NSColor::systemRedColor()
                } else {
                    NSColor::labelColor()
                };
                let label = panel_label(
                    mtm,
                    &compact_heading(row),
                    &NSFont::monospacedDigitSystemFontOfSize_weight(11.0, semibold),
                    &color,
                );
                if index == 1 {
                    label.setAlignment(NSTextAlignment::Right);
                }
                label.setToolTip(Some(&tooltip));
                place(
                    &label,
                    20.0 + index as f64 * 138.0,
                    row_top + 2.0,
                    138.0,
                    16.0,
                );
                let (offset, thickness) = if index == 0 { tracks.0 } else { tracks.1 };
                let track = rounded_box(
                    mtm,
                    &NSColor::systemGrayColor().colorWithAlphaComponent(0.28),
                    thickness / 2.0,
                );
                track.setToolTip(Some(&tooltip));
                place(&track, 22.0, row_top + offset, PANEL_INNER_WIDTH, thickness);
                // Same rule as Unique: a fill narrower than its own height is a
                // pinched artefact, so only the track remains.
                let width = row_fill_width(row);
                if width >= thickness {
                    let fill = rounded_box(
                        mtm,
                        &quota_color(row.remaining.unwrap_or(0), row.model),
                        thickness / 2.0,
                    );
                    fill.setToolTip(Some(&tooltip));
                    place(&fill, 22.0, row_top + offset, width, thickness);
                }
                // Reset of this window, under its column (7 j right-aligned like its heading).
                let reset = panel_label(
                    mtm,
                    &compact_reset(&row.reset),
                    &NSFont::systemFontOfSize(10.0),
                    &NSColor::secondaryLabelColor(),
                );
                if index == 1 {
                    reset.setAlignment(NSTextAlignment::Right);
                }
                reset.setToolTip(Some(&tooltip));
                place(
                    &reset,
                    20.0 + index as f64 * 138.0,
                    row_top + if fable.is_some() { 43.0 } else { 38.0 },
                    138.0,
                    13.0,
                );
            }
            if let Some(row) = fable {
                let tooltip =
                    NSString::from_str(&format!("{} · {}", compact_heading(row), row.reset));
                let offset = tracks.1 .0 + tracks.1 .1 + 2.0;
                let thickness = tracks.1 .1;
                let track = rounded_box(
                    mtm,
                    &NSColor::systemGrayColor().colorWithAlphaComponent(0.28),
                    thickness / 2.0,
                );
                track.setToolTip(Some(&tooltip));
                {
                    use objc2_app_kit::NSAccessibility;
                    track.setAccessibilityElement(true);
                    track.setAccessibilityLabel(Some(&tooltip));
                }
                place(&track, 22.0, row_top + offset, PANEL_INNER_WIDTH, thickness);
                let width = row_fill_width(row);
                if width >= thickness {
                    let fill = rounded_box(
                        mtm,
                        &quota_color(row.remaining.unwrap_or(0), true),
                        thickness / 2.0,
                    );
                    fill.setToolTip(Some(&tooltip));
                    place(&fill, 22.0, row_top + offset, width, thickness);
                }
            }
            row_top += block_height;
        }
        for row in provider.rows.iter().filter(|r| {
            !paired_rows.iter().any(|paired| std::ptr::eq(*paired, *r))
                && !fable.is_some_and(|f| std::ptr::eq(f, *r))
        }) {
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
            let low = row_is_low(row);
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
            if row_fill_visible(row) {
                let fill_width = row_fill_width(row);
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
    claude_snaps: &[&claude::ClaudeSnapshot],
) -> Option<DateTime<Local>> {
    let parse = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|d| d.with_timezone(&Local))
    };
    let codex = rows.iter().filter_map(|r| parse(&r.checked_at)).max();
    let claude = claude_snaps
        .iter()
        .filter_map(|s| s.observed_at.as_deref().and_then(parse))
        .max();
    codex.into_iter().chain(claude).max()
}

/// The menu is being tracked: refresh its panel in place instead of swapping menus.
#[cfg(target_os = "macos")]
static MENU_OPEN: AtomicBool = AtomicBool::new(false);
/// Panel item of the menu currently attached to the status item (main thread only).
#[cfg(target_os = "macos")]
static PANEL_ITEM: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// A live panel update was deferred during tracking (see `refresh_open_panel`).
#[cfg(target_os = "macos")]
static PANEL_DEFERRED: AtomicBool = AtomicBool::new(false);

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
        let model = panel_model(&app);
        let snapshots = get_quota_snapshots(app.clone());
        let claude_snap = claude::snapshot(&app);
        let mtm = MainThreadMarker::new().expect("native menu on main thread");
        let indicator = indicator_for(&app, &snapshots, &claude_snap);
        let panel_view = make_panel_view(mtm, &model.header, &model.providers);
        let open_item = PANEL_ITEM.load(Ordering::Acquire);
        if MENU_OPEN.load(Ordering::Acquire) && open_item != 0 {
            // Live update while the user keeps the menu open.
            refresh_open_panel(unsafe { &*(open_item as *const NSMenuItem) }, &panel_view);
            update_status_button(&native, mtm, &indicator);
            return;
        }
        let target = unsafe { &*(native.target as *const QuotaMenuTarget) };
        let (menu, panel) = make_native_quota_menu(
            mtm,
            target,
            &panel_view,
            model.connection,
            &app.package_info().version.to_string(),
        );
        unsafe {
            let item = &*(native.status_item as *const objc2_app_kit::NSStatusItem);
            item.setMenu(Some(&menu));
        }
        // The menu (retained by the status item) keeps the panel item alive.
        PANEL_ITEM.store(Retained::as_ptr(&panel) as usize, Ordering::Release);
        update_status_button(&native, mtm, &indicator);
    });
}

/// Live update of the panel of the menu being tracked (main thread).
/// Returns `false` when the update is deferred.
///
/// Keeps the item's view object and moves the freshly built contents into
/// it. `setView` during tracking makes AppKit rebuild the item's row and
/// re-place the selection background of the highlighted action (« Réglages… »,
/// « Vérifier les mises à jour… ») over the top of the quota panel: the stray
/// blue band. Resizing the panel while an action is highlighted also shifts
/// that background off its row, so a height change waits until nothing is
/// highlighted, the menu closes (`menuDidClose:` re-renders) or reopens.
/// Reproduced by `--menu-tracking-fixture` (menu_fixture.rs).
#[cfg(target_os = "macos")]
fn refresh_open_panel(item: &NSMenuItem, next: &NSView) -> bool {
    let Some(current) = item.view() else {
        item.setView(Some(next));
        return true;
    };
    let size = next.frame().size;
    if current.frame().size != size {
        let highlighted =
            unsafe { item.menu() }.is_some_and(|menu| menu.highlightedItem().is_some());
        if highlighted {
            PANEL_DEFERRED.store(true, Ordering::Release);
            return false;
        }
        current.setFrameSize(size);
    }
    for old in current.subviews().iter() {
        old.removeFromSuperview();
    }
    // Subviews are laid out top-down for `next`'s height (= current's now).
    for sub in next.subviews().iter() {
        current.addSubview(&sub);
    }
    current.setNeedsDisplay(true);
    true
}

#[cfg(target_os = "macos")]
fn make_native_quota_menu(
    mtm: MainThreadMarker,
    target: &QuotaMenuTarget,
    panel_view: &NSView,
    connection: Option<bool>,
    version: &str,
) -> (Retained<NSMenu>, Retained<NSMenuItem>) {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    menu.setDelegate(Some(ProtocolObject::from_ref(target)));
    let panel = NSMenuItem::new(mtm);
    panel.setView(Some(panel_view));
    // A custom view does not make its NSMenuItem informational. With automatic
    // validation off, AppKit otherwise highlights this actionless panel on hover.
    panel.setEnabled(false);
    menu.addItem(&panel);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    // Below the panel: the connection action when needed, then Réglages…, the
    // updater, Quitter. No provider/account entries (accounts live in Réglages).
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
    if let Some(logging_in) = connection {
        add_action(
            connection_label(logging_in),
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
    menu.addItem(&make_version_item(mtm, version));
    (menu, panel)
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

/// Windows/Linux tray lines for the visible Claude accounts. One visible
/// account keeps the exact v3.1.0 wording; several are prefixed by name.
#[cfg(any(not(target_os = "macos"), test))]
fn claude_tray_lines(accounts: &[claude::AccountSnapshot]) -> Vec<String> {
    let visible: Vec<_> = accounts
        .iter()
        .filter(|a| a.profile.visible && claude_is_visible(&a.snapshot))
        .collect();
    let named = visible.len() > 1;
    let mut lines = Vec::new();
    for account in visible {
        let snap = &account.snapshot;
        let name = if named {
            format!("Claude · {}", account.profile.name)
        } else {
            "Claude".to_string()
        };
        if snap.metrics.is_empty() {
            lines.push(format!("{name} · {}", snap.message));
        }
        for m in &snap.metrics {
            let text = match (m.availability, m.used_percent) {
                ("available", Some(_)) if m.exhausted => "épuisé".to_string(),
                ("available", Some(u)) => format!("{} % restants", remaining_percent(u)),
                ("unavailable", _) => "réinitialisée · en attente".to_string(),
                _ => "non communiquée".to_string(),
            };
            lines.push(format!("{name} · {} · {text}", m.label));
        }
        if let Some(line) = claude_status_line(snap.state) {
            lines.push(if named {
                line.replacen("Claude", &name, 1)
            } else {
                line.to_string()
            });
        }
    }
    lines
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
    // Exercise the production projection, including a Fable window equal to the weekly one.
    let claude = claude_panel(&claude::usage_snapshot(
        &serde_json::json!({"subscription_type":"pro", "rate_limits":{
            "five_hour":{"utilization":88,"resets_at":soon},
            "seven_day":{"utilization":37,"resets_at":week},
            "model_scoped":[{"display_name":"Fable","utilization":37,"resets_at":week}]
        }}),
        Local::now().timestamp(),
    ));
    assert_eq!(claude.rows.len(), 3);
    assert!(!claude.rows[1].model);
    assert_eq!(claude.rows[2].label, "Limite Fable");
    let mut scenarios: Vec<(&str, Vec<PanelProvider>)> = vec![
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
                ..claude.clone()
            }],
        ),
        ("none", vec![]),
    ];
    // Multi-account fixtures: real projection (usage_snapshot -> AccountSnapshot
    // -> claude_account_panels), synthetic data only, no Claude process.
    let fixture = |name: &str,
                   plan: &str,
                   used5: Option<f64>,
                   used7: Option<f64>,
                   fable: Option<f64>| {
        let (m5, h7) = if name == "Personnel" {
            (210, 80)
        } else {
            (42, 49)
        };
        let at = |d: chrono::Duration| (Local::now() + d).to_rfc3339();
        let window = |used: Option<f64>, reset: String| {
            used.map(|u| serde_json::json!({"utilization":u,"resets_at":reset}))
        };
        let mut limits = serde_json::json!({
            "five_hour": window(used5, at(chrono::Duration::minutes(m5))),
            "seven_day": window(used7, at(chrono::Duration::hours(h7))),
        });
        if let Some(f) = fable {
            limits["model_scoped"] = serde_json::json!([
                {"display_name":"Fable","utilization":f,"resets_at":at(chrono::Duration::hours(h7 + 20))}
            ]);
        }
        claude::AccountSnapshot {
            profile: claude_accounts::Profile {
                id: name.into(),
                name: name.into(),
                path: format!("/fixture/{name}"),
                visible: true,
            },
            primary: name == "Personnel",
            available: true,
            refreshing: false,
            snapshot: claude::usage_snapshot(
                &serde_json::json!({"subscription_type":plan,"rate_limits":limits}),
                Local::now().timestamp(),
            ),
        }
    };
    let personal = fixture("Personnel", "pro", Some(27.0), Some(14.0), None);
    let work = fixture("Travail", "max", Some(65.0), Some(80.0), None);
    scenarios.push((
        "single-account",
        claude_account_panels(&[personal.clone()], false),
    ));
    // Classique, one account: the exact historical card.
    scenarios.push((
        "single-account-classic",
        claude_account_panels(&[personal.clone()], true),
    ));
    scenarios.push((
        "multi-accounts",
        claude_account_panels(&[personal.clone(), work.clone()], false),
    ));
    scenarios.push((
        "multi-accounts-classic",
        claude_account_panels(&[personal.clone(), work.clone()], true),
    ));
    let fable_pair = [
        fixture("Personnel", "pro", Some(27.0), Some(14.0), Some(40.0)),
        fixture("Travail", "max", Some(65.0), Some(80.0), Some(91.0)),
    ];
    scenarios.push((
        "single-fable",
        claude_account_panels(&fable_pair[..1], false),
    ));
    scenarios.push(("multi-fable", claude_account_panels(&fable_pair, false)));
    scenarios.push((
        "multi-fable-classic",
        claude_account_panels(&fable_pair, true),
    ));
    let mut exhausted = fixture("Épuisé", "max", Some(100.0), None, Some(15.0));
    exhausted.snapshot.extra_usage = Some(claude::ExtraUsage {
        enabled: true,
        limit_reached: true,
    });
    let mut failed = work.clone();
    failed.snapshot =
        claude::result_snapshot(Err("unsupported_or_auth"), None, Local::now().timestamp());
    let mut missing = fixture("Absent", "pro", None, None, None);
    missing.available = false;
    missing.snapshot.state = "error";
    missing.snapshot.message = "Dossier de profil Claude indisponible.".into();
    missing.snapshot.metrics.clear();
    let edge = [exhausted, failed, missing];
    scenarios.push(("multi-edge-cases", claude_account_panels(&edge, false)));
    scenarios.push((
        "multi-edge-cases-classic",
        claude_account_panels(&edge, true),
    ));
    let mut hidden = work.clone();
    hidden.profile.visible = false;
    scenarios.push((
        "multi-one-visible",
        claude_account_panels(&[personal.clone(), hidden.clone()], false),
    ));
    scenarios.push((
        "multi-one-visible-classic",
        claude_account_panels(&[personal, hidden], true),
    ));
    let mut native_codex = codex_rows.clone();
    native_codex[0].period = "codex:codex:primary".into();
    native_codex[1].period = "codex:codex:secondary".into();
    for layout in ["classic", "compact"] {
        let codex = codex_presentation(&native_codex, "ready", "", Some("plus".into()), layout);
        scenarios.push((
            if layout == "compact" {
                "codex-compact"
            } else {
                "codex-classic"
            },
            vec![codex.clone()],
        ));
        scenarios.push((
            if layout == "compact" {
                "codex-both-compact"
            } else {
                "codex-both-classic"
            },
            vec![codex, claude.clone()],
        ));
    }
    scenarios.push((
        "codex-weekly-only",
        vec![codex_presentation(
            &native_codex[1..],
            "ready",
            "",
            None,
            "compact",
        )],
    ));
    scenarios.push((
        "codex-empty-error",
        vec![codex_presentation(
            &[],
            "error",
            "Compte déconnecté",
            None,
            "compact",
        )],
    ));
    // Multi-Codex: production projection (account_view -> mark_duplicates ->
    // codex_account_panels), synthetic rows only, no Codex process.
    let codex_env = |name: &str,
                     primary: bool,
                     phase: &str,
                     message: &str,
                     rows: &[QuotaPayload],
                     identity: Option<u64>| {
        let found = codex_accounts::Discovered {
            profile: claude_accounts::Profile {
                id: format!("profile:{name}"),
                path: format!("/fixture/.codex-{name}"),
                name: name.into(),
                visible: true,
            },
            primary,
            tracked: true,
            launch: (!primary).then(|| format!("/fixture/.codex-{name}").into()),
            available: true,
            visible: true,
        };
        let reading = codex::Reading {
            phase: phase.into(),
            message: message.into(),
            // Signed out: account/read returns no account, hence no plan.
            plan_type: (phase == "ready").then(|| if primary { "plus" } else { "pro" }.into()),
            rows: (!rows.is_empty()).then(|| rows.to_vec()),
            identity,
            last_checked: None,
            available_resets: None,
        };
        let view = codex::PrimaryView {
            status: codex::Status {
                phase: phase.into(),
                message: message.into(),
                plan_type: Some("plus".into()),
                ..codex::Status::default()
            },
            rows: rows.to_vec(),
            identity,
        };
        codex::account_view(
            found,
            true,
            Some(&view),
            (!primary).then_some((reading, false)),
        )
    };
    let mut work_codex = native_codex.clone();
    work_codex[0].remaining = 18;
    work_codex[0].reset_at = (Local::now() + chrono::Duration::minutes(42)).to_rfc3339();
    work_codex[1].remaining = 64;
    let mut codex_envs = vec![
        codex_env("Principal", true, "ready", "", &native_codex, Some(1)),
        codex_env("Travail", false, "ready", "", &work_codex[..2], Some(2)),
        codex_env(
            "Perso",
            false,
            "signed_out",
            "Non connecté. Connectez ce dossier dans un terminal : CODEX_HOME=/fixture/.codex-Perso codex login",
            &[],
            None,
        ),
        codex_env("Alias", false, "ready", "", &native_codex[..2], Some(1)),
    ];
    codex::mark_duplicates(&mut codex_envs);
    scenarios.push((
        "codex-multi-accounts",
        codex_account_panels(&codex_envs, "compact"),
    ));
    scenarios.push((
        "codex-multi-accounts-classic",
        codex_account_panels(&codex_envs, "classic"),
    ));
    scenarios.push((
        "codex-and-claude-multi",
        codex_account_panels(&codex_envs[..2], "compact")
            .into_iter()
            .chain(claude_account_panels(
                &[
                    fixture("Personnel", "pro", Some(27.0), Some(14.0), None),
                    work.clone(),
                ],
                false,
            ))
            .collect(),
    ));
    let target = QuotaMenuTarget::new(mtm);
    // Main-thread AppKit regression checks use the production builder without
    // starting Tauri, reading preferences, or contacting either quota provider.
    for (name, providers) in &scenarios {
        for connection in [None, Some(false), Some(true)] {
            let header = panel_header(providers, (!providers.is_empty()).then(Local::now));
            let view = make_panel_view(mtm, &header, providers);
            let (menu, panel) = make_native_quota_menu(mtm, &target, &view, connection, "fixture");
            assert!(!menu.autoenablesItems());
            assert!(
                !panel.isEnabled(),
                "informational panel must not be selectable"
            );
            assert!(panel.action().is_none());
            assert_eq!(
                menu.numberOfItems(),
                if connection.is_some() { 8 } else { 7 }
            );
            let mut actions = Vec::new();
            let mut separators = 0;
            for item in menu.itemArray() {
                if item.isSeparatorItem() {
                    separators += 1;
                } else if item.action().is_some() {
                    assert!(!item.title().is_empty());
                    actions.push((
                        item.title().to_string(),
                        item.isEnabled(),
                        item.keyEquivalent().to_string(),
                    ));
                } else {
                    assert!(!item.isEnabled(), "non-action item must be disabled");
                }
            }
            assert_eq!(separators, 2);
            assert!(actions.contains(&("Réglages…".into(), true, ",".into())));
            assert!(actions.contains(&("Vérifier les mises à jour…".into(), true, "".into())));
            assert!(actions.contains(&("Quitter Quota Monitor".into(), true, "q".into())));
            if let Some(logging_in) = connection {
                assert_eq!(actions[0].1, !logging_in);
            }
            // Live refresh keeps the item and its view object, never touching
            // the action siblings; exercise height/state changes both ways
            // (nothing highlighted here, so resizes apply at once).
            let host = panel.view().unwrap();
            for (_, next) in &scenarios {
                let next_header = panel_header(next, None);
                let next_view = make_panel_view(mtm, &next_header, next);
                let next_frame = next_view.frame();
                let next_count = next_view.subviews().count();
                assert!(refresh_open_panel(&panel, &next_view));
                assert!(!panel.isEnabled());
                assert_eq!(
                    menu.numberOfItems(),
                    if connection.is_some() { 8 } else { 7 }
                );
                let shown = panel.view().unwrap();
                assert!(
                    std::ptr::eq(Retained::as_ptr(&shown), Retained::as_ptr(&host)),
                    "panel view replaced"
                );
                assert_eq!(shown.frame(), next_frame);
                assert_eq!(shown.subviews().count(), next_count);
            }
            println!("NATIVE_MENU_STRUCTURE_PASS {name} connection={connection:?}");
        }
    }
    // Bottom of the menu, below the panel: exactly the historical actions, with
    // Réglages… first. No provider, account, usage or « + » entry, no submenu.
    for connection in [None, Some(false), Some(true)] {
        let providers = &scenarios[scenarios.len() - 1].1;
        let header = panel_header(providers, Some(Local::now()));
        let view = make_panel_view(mtm, &header, providers);
        let (menu, panel) = make_native_quota_menu(mtm, &target, &view, connection, "fixture");
        let items: Vec<Retained<NSMenuItem>> = menu.itemArray().iter().collect();
        let titles: Vec<String> = items
            .iter()
            .map(|i| {
                if i.isSeparatorItem() {
                    "-".to_string()
                } else {
                    i.title().to_string()
                }
            })
            .collect();
        // The panel and the version line are view items (AppKit's default title).
        let mut expected: Vec<String> = vec!["NSMenuItem".into(), "-".into()];
        if let Some(logging_in) = connection {
            expected.push(connection_label(logging_in).to_string());
        }
        expected.extend(
            [
                "Réglages…",
                "Vérifier les mises à jour…",
                "-",
                "Quitter Quota Monitor",
            ]
            .map(String::from),
        );
        expected.push("NSMenuItem".into());
        assert_eq!(titles, expected, "{titles:?}");
        assert!(
            items.iter().all(|i| i.submenu().is_none()),
            "no detail submenu: {titles:?}"
        );
        assert!(
            !titles.iter().any(|t| t.contains('↗')
                || t.starts_with("Codex")
                || t.starts_with("Claude")
                || t.contains("Ajouter")),
            "{titles:?}"
        );
        // The first action under the panel is Réglages… (the connection action, only
        // when Codex is not connected, keeps its historical place before it).
        let first_action = items
            .iter()
            .skip(2)
            .find(|i| !i.isSeparatorItem())
            .map(|i| i.title().to_string())
            .unwrap();
        assert_eq!(
            first_action,
            connection.map_or("Réglages…", |l| connection_label(l))
        );
        // The live refresh of the open panel is untouched.
        let host = panel.view().unwrap();
        let next = make_panel_view(mtm, &header, &scenarios[0].1);
        assert!(refresh_open_panel(&panel, &next));
        assert!(std::ptr::eq(
            Retained::as_ptr(&panel.view().unwrap()),
            Retained::as_ptr(&host)
        ));
        println!("NATIVE_MENU_BOTTOM_PASS connection={connection:?} items={titles:?}");
    }
    for (name, providers) in scenarios {
        let header = panel_header(&providers, (!providers.is_empty()).then(Local::now));
        let fixture_model = PanelModel {
            addable: addable_from_panels(&providers),
            header: header.clone(),
            providers: providers.clone(),
            connection: None,
        };
        std::fs::write(
            format!("{directory}/{name}.json"),
            serde_json::to_vec_pretty(&compact_panel(&fixture_model, "fixture", None)).unwrap(),
        )
        .unwrap();
        for (mode, appearance) in [
            ("light", unsafe { NSAppearanceNameAqua }),
            ("dark", unsafe { NSAppearanceNameDarkAqua }),
        ] {
            let panel = make_panel_view(mtm, &header, &providers);
            let bounds = panel.bounds();
            if matches!(
                name,
                "single-account"
                    | "single-account-classic"
                    | "multi-accounts"
                    | "single-fable"
                    | "multi-fable"
                    | "multi-fable-classic"
                    | "multi-accounts-classic"
                    | "two-providers"
                    | "codex-compact"
                    | "codex-classic"
            ) {
                // Measure the real AppKit frames (rows are composited in group
                // views since v3.1.0): every track/fill at x = 22, in root space.
                fn bars(root: &NSView, view: &NSView, out: &mut Vec<NSRect>) {
                    for sub in view.subviews().iter() {
                        let r = sub.convertRect_toView(sub.bounds(), Some(root));
                        if (r.origin.x - 22.0).abs() < 0.01 && r.size.height <= 7.0 {
                            out.push(r);
                        }
                        bars(root, &sub, out);
                    }
                }
                let mut found = Vec::new();
                bars(&panel, &panel, &mut found);
                // Top-down order (AppKit y grows upwards).
                found.sort_by(|a, b| {
                    (b.origin.y + b.size.height)
                        .partial_cmp(&(a.origin.y + a.size.height))
                        .unwrap()
                });
                let heights: Vec<f64> = found.iter().map(|r| r.size.height).collect();
                let unique_claude = vec![PANEL_TRACK_HEIGHT; 4];
                match name {
                    "single-fable" | "multi-fable" => {
                        let count = if name == "single-fable" { 1 } else { 2 };
                        assert_eq!(heights, [5.0, 5.0, 3.0, 3.0, 3.0, 3.0].repeat(count));
                        for block in found.chunks(6) {
                            let gap =
                                block[2].origin.y - (block[4].origin.y + block[4].size.height);
                            assert!((gap - 2.0).abs() < 0.01, "Fable gap {gap}");
                        }
                        for provider in &providers {
                            assert_eq!(pair_block_height(provider), 65.0);
                            assert_eq!(card_height(provider), 81.0);
                        }
                        fn labels(view: &NSView, out: &mut Vec<String>) {
                            for sub in view.subviews().iter() {
                                if let Some(label) = sub.downcast_ref::<NSTextField>() {
                                    out.push(label.stringValue().to_string());
                                }
                                labels(&sub, out);
                            }
                        }
                        let mut texts = Vec::new();
                        labels(&panel, &mut texts);
                        assert!(!texts.iter().any(|s| s.contains("Fable")));
                        fn fable_accessibility(view: &NSView, out: &mut Vec<String>) {
                            use objc2_app_kit::NSAccessibility;
                            for sub in view.subviews().iter() {
                                if sub.isAccessibilityElement() {
                                    if let Some(label) = sub.accessibilityLabel() {
                                        let label = label.to_string();
                                        if label.starts_with("Fable ·") {
                                            assert!(
                                                label.contains('%')
                                                    && label.contains("Réinitialisation")
                                            );
                                            assert_eq!(
                                                sub.toolTip().map(|s| s.to_string()),
                                                Some(label.clone())
                                            );
                                            out.push(label);
                                        }
                                    }
                                }
                                fable_accessibility(&sub, out);
                            }
                        }
                        let mut accessible = Vec::new();
                        fable_accessibility(&panel, &mut accessible);
                        assert_eq!(accessible.len(), count);
                        println!("NATIVE_FABLE_THIN_PASS {name} {mode}: block=65 gap=2 thickness=3 no-caption");
                    }
                    "multi-fable-classic" => assert_eq!(heights, vec![5.0; 12]),
                    "codex-compact" => {
                        assert_eq!(heights, vec![5.0, 5.0, 3.0, 3.0, 5.0]);
                        let gap = found[0].origin.y - (found[2].origin.y + found[2].size.height);
                        assert!((gap - 2.0).abs() < 0.01);
                    }
                    "codex-classic" => assert_eq!(heights, vec![5.0; 5]),
                    "single-account-classic" => assert_eq!(heights, unique_claude),
                    // Compacte, one account: one paired 5 h / 7 j block.
                    "single-account" => assert_eq!(
                        heights,
                        vec![PANEL_TRACK_HEIGHT, PANEL_TRACK_HEIGHT, 3.0, 3.0]
                    ),
                    // Classique: two separated Unique rows per account, no 3 pt track.
                    "multi-accounts-classic" => {
                        assert_eq!(heights, vec![PANEL_TRACK_HEIGHT; 8])
                    }
                    "multi-accounts" => {
                        let pair = [PANEL_TRACK_HEIGHT, PANEL_TRACK_HEIGHT, 3.0, 3.0];
                        assert_eq!(heights, [pair, pair].concat());
                        for block in found.chunks(4) {
                            // 2 pt between the bottom of the 5 h track and the 7 j track.
                            let gap =
                                block[0].origin.y - (block[2].origin.y + block[2].size.height);
                            assert!((gap - 2.0).abs() < 0.01, "gap {gap}");
                        }
                    }
                    _ => assert!(heights.iter().all(|h| *h == PANEL_TRACK_HEIGHT)),
                }
                println!("NATIVE_TRACK_PARITY_PASS {name} {mode}: heights={heights:?}");
            }
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
fn install_status_item(app: &tauri::AppHandle) {
    let state = app.state::<NativeMenuState>();
    if state.0.lock().unwrap().is_some() {
        return;
    }
    let mtm = MainThreadMarker::new().expect("menu setup on main thread");
    let target = QuotaMenuTarget::new(mtm);
    let status_item =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    let button = status_item.button(mtm).expect("status item button");
    button.setImage(mascot_image(mtm).as_deref());
    *state.0.lock().unwrap() = Some(NativeQuotaMenu {
        status_item: Retained::as_ptr(&status_item) as usize,
        status_button: Retained::as_ptr(&button) as usize,
        target: Retained::as_ptr(&target) as usize,
    });
    // Owned by the state until `remove_status_item` reclaims them.
    std::mem::forget(status_item);
    std::mem::forget(target);
    render_native_menu(app);
}

/// Removes the menu-bar item (standalone mode) and releases what
/// `install_status_item` leaked. Main thread only.
#[cfg(target_os = "macos")]
fn remove_status_item(app: &tauri::AppHandle) {
    let Some(native) = app.state::<NativeMenuState>().0.lock().unwrap().take() else {
        return;
    };
    MENU_OPEN.store(false, Ordering::Release);
    PANEL_ITEM.store(0, Ordering::Release);
    unsafe {
        let item = Retained::from_raw(native.status_item as *mut objc2_app_kit::NSStatusItem);
        let target = Retained::from_raw(native.target as *mut QuotaMenuTarget);
        if let Some(item) = &item {
            NSStatusBar::systemStatusBar().removeStatusItem(item);
        }
        drop(item);
        drop(target);
    }
}

#[cfg(target_os = "macos")]
fn activate_app(app: &tauri::AppHandle) {
    let _ = app.run_on_main_thread(|| {
        let mtm = MainThreadMarker::new().expect("activation runs on main thread");
        NSApplication::sharedApplication(mtm).activate();
    });
}

/// Marks the standalone window so the webview renders the compact panel view.
#[cfg(target_os = "macos")]
const COMPACT_VIEW_SCRIPT: &str = "window.__QUOTA_MONITOR_VIEW__ = \"compact\";";

/// Dashboard window of the standalone mode. The red button hides it (macOS
/// convention); the Dock icon or Cmd+Tab brings it back, Cmd+Q quits.
#[cfg(target_os = "macos")]
fn show_main_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    let window = if let Some(window) = app.get_webview_window("main") {
        window
    } else {
        let window = tauri::WebviewWindowBuilder::new(
            app,
            "main",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("Quota Monitor")
        // Compact view of the menu-bar panel (320 pt wide) with its actions as
        // icons; the flag is set before any script so the first paint is right.
        .initialization_script(COMPACT_VIEW_SCRIPT)
        .inner_size(360.0, 520.0)
        .min_inner_size(300.0, MAIN_WINDOW_MIN_HEIGHT)
        .center()
        .build()?;
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
        window
    };
    window.show()?;
    window.unminimize()?;
    window.set_focus()?;
    activate_app(app);
    Ok(())
}

/// Applies the saved display mode live (main thread). The way in is always
/// created before the previous one is removed.
#[cfg(target_os = "macos")]
fn apply_display_mode(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let app = handle;
        let layout =
            display_mode::layout(&prefs::current(&app).display_mode, display_mode::SUPPORTED);
        if layout.window {
            if let Err(error) = show_main_window(&app) {
                // Keep the menu-bar item rather than leave no entry point.
                eprintln!("Quota Monitor window unavailable: {error}");
                return;
            }
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
            remove_status_item(&app);
            activate_app(&app);
        } else {
            install_status_item(&app);
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            render_native_menu(&app);
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn apply_display_mode(_app: &tauri::AppHandle) {}

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
    for line in claude_tray_lines(&claude::accounts(app)) {
        menu.append(&MenuItem::new(app, line, false, None::<&str>)?)?;
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
        .manage(settings_focus::SettingsFocusState::default())
        .invoke_handler(tauri::generate_handler![
            ntfy::test_ntfy,
            get_platform,
            get_quota_snapshots,
            get_compact_panel,
            connect_codex_account,
            request_quota_refresh,
            open_settings,
            settings_focus::open_settings_focus,
            settings_focus::take_settings_focus,
            prefs::get_preferences,
            prefs::set_preferences,
            claude::get_claude_snapshot,
            claude::get_claude_accounts,
            claude::add_claude_profile,
            claude::follow_claude_profile,
            claude::untrack_claude_profile,
            choose_claude_profile_folder,
            choose_codex_profile_folder,
            codex::get_codex_accounts,
            codex::add_codex_profile,
            codex::follow_codex_profile,
            codex::untrack_codex_profile,
            claude::request_claude_refresh,
            open_usage_details,
            open_claude_usage,
            fit_settings_window,
            fit_update_window,
            fit_main_window,
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
            claude::migrate_legacy_relay_at_startup(app.handle());
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
            {
                app.manage(NativeMenuState::default());
                let layout = display_mode::layout(
                    &prefs::current(app.handle()).display_mode,
                    display_mode::SUPPORTED,
                );
                let mut standalone = false;
                if layout.window {
                    app.handle()
                        .set_activation_policy(tauri::ActivationPolicy::Regular)?;
                    standalone = show_main_window(app.handle()).is_ok();
                }
                if !standalone {
                    // Menu-bar mode, or fallback so the app is never unreachable.
                    app.handle()
                        .set_activation_policy(tauri::ActivationPolicy::Accessory)?;
                    install_status_item(app.handle());
                }
            }

            #[cfg(not(target_os = "macos"))]
            build_desktop(app)?;

            app.manage(codex::start(app.handle().clone()));
            // Additional followed CODEX_HOME homes (none by default).
            app.state::<codex::Provider>().reconcile(app.handle());
            claude::start_watcher(app.handle().clone());
            Ok(())
        })
        .build(app_context())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Closing the Settings/updater window must not quit the menu-bar app.
            // Standalone mode honours Cmd+Q / Dock "Quit".
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                if code.is_none()
                    && !display_mode::allow_system_exit(
                        &prefs::current(app).display_mode,
                        display_mode::SUPPORTED,
                    )
                {
                    api.prevent_exit();
                }
            }
            // Dock icon click in standalone mode reopens the hidden dashboard.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = &event {
                if display_mode::layout(&prefs::current(app).display_mode, display_mode::SUPPORTED)
                    .window
                {
                    let _ = show_main_window(app);
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

    #[cfg(target_os = "macos")]
    #[test]
    fn compact_window_serialises_the_exact_menu_panel_model() {
        let soon = (Local::now() + chrono::Duration::minutes(95)).to_rfc3339();
        let week = (Local::now() + chrono::Duration::days(4)).to_rfc3339();
        let rows = vec![
            demo_payload("Réserve", "reserve-weekly", 1, &week, None),
            demo_payload("Limite hebdomadaire", "weekly", 85, &week, Some(10080)),
            demo_payload("Limite 5 heures", "five-hour", 20, &soon, Some(300)),
        ];
        let codex = codex_panel(&rows, "ready", "", Some("plus".into()));
        let claude = PanelProvider {
            account: None,
            name: "Claude".into(),
            compact: false,
            badge: None,
            rows: vec![],
            note: Some(("Lecture via Claude Code…".into(), Tone::Warning)),
        };
        let providers = vec![codex.clone(), claude];
        let model = PanelModel {
            addable: Vec::new(),
            header: panel_header(&providers, None),
            providers: providers.clone(),
            connection: Some(false),
        };
        let panel = compact_panel(&model, "9.9.9", None);
        assert_eq!(panel.header.title, "Quota Monitor");
        assert_eq!(panel.header.subtitle, model.header.subtitle);
        assert_eq!(panel.header.status, "Lecture…");
        assert_eq!(panel.header.tone, "warning");
        assert_eq!(panel.providers.len(), 2);
        let compact = &panel.providers[0];
        assert_eq!(
            (compact.name.as_str(), compact.section.as_str()),
            ("Codex", "CODEX")
        );
        assert_eq!(compact.badge.as_deref(), Some("Plus"));
        // Same order, labels, values and reset phrases as the tray rows.
        for (tray, row) in codex.rows.iter().zip(&compact.rows) {
            assert_eq!(row.label, tray.label);
            assert_eq!(row.value, tray.value);
            assert_eq!(row.reset, tray.reset);
            assert_eq!(row.remaining, tray.remaining);
            assert_eq!(row.low, row_is_low(tray));
            assert_eq!(row.fill.visible, row_fill_visible(tray));
            let (from, to, fraction) = quota_color_transition(tray.remaining.unwrap());
            assert_eq!(
                (row.fill.from, row.fill.to, row.fill.fraction),
                (from, to, fraction)
            );
        }
        assert_eq!(compact.rows.len(), 3);
        assert!(compact.rows[0].label.starts_with("Limite sur"));
        assert_eq!(compact.rows[2].label, "Réserve");
        assert!(compact.rows[0].low && compact.rows[0].fill.visible);
        // 1 % is narrower than the gauge radius: track only, like the tray.
        assert!(!compact.rows[2].fill.visible);
        assert_eq!(
            panel.providers[1].note,
            Some(CompactNote {
                text: "Lecture via Claude Code…".into(),
                tone: "warning"
            })
        );
        assert_eq!(
            panel.connection,
            Some(CompactConnection {
                label: "Se connecter à Codex…",
                logging_in: false
            })
        );
        let json = serde_json::to_value(&panel).unwrap();
        assert_eq!(json["version"], "9.9.9");
        assert!(json["update"].is_null());
        assert_eq!(json["providers"][0]["rows"][0]["fill"]["visible"], true);
        assert!(json["connection"]["loggingIn"].is_boolean());
        let logging = compact_panel(
            &PanelModel {
                addable: Vec::new(),
                connection: Some(true),
                ..model
            },
            "9.9.9",
            None,
        );
        assert_eq!(
            logging.connection.map(|c| c.label),
            Some("Connexion en cours…")
        );
    }

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
        // A Fable window equal to the weekly one is NOT folded into it: the 7 j row
        // keeps its own label/countdown and Fable stays a separate, model-scoped row.
        let mut duplicate = snap.clone();
        duplicate.metrics[1].used_percent = duplicate.metrics[2].used_percent;
        let both = claude_panel(&duplicate);
        assert_eq!(both.rows.len(), 3);
        assert!(!both.rows[1].model && !both.rows[1].label.contains("Fable"));
        assert_eq!(both.rows[1].value, "70 %");
        assert!(both.rows[1].reset.contains("Réinitialisation"));
        assert_ne!(both.rows[1].countdown, "—");
        assert_eq!(both.rows[2].label, "Limite Fable");
        assert_eq!(both.rows[2].value, "70 %");
        assert!(both.rows[2].model);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compacte_keeps_the_weekly_reset_and_the_fable_bar_independent() {
        let now = Local::now().timestamp();
        let soon = (Local::now() + chrono::Duration::minutes(210)).to_rfc3339();
        let week = (Local::now() + chrono::Duration::hours(80)).to_rfc3339();
        let limits = |fable: Option<f64>| {
            let mut v = serde_json::json!({"rate_limits":{
                "five_hour":{"utilization":27,"resets_at":soon},
                "seven_day":{"utilization":37,"resets_at":week}
            }});
            if let Some(f) = fable {
                // Same percentage AND reset as the weekly window: still its own row.
                v["rate_limits"]["model_scoped"] =
                    serde_json::json!([{"display_name":"Fable","utilization":f,"resets_at":week}]);
            }
            v
        };
        for fable in [Some(37.0), Some(60.0), None] {
            let snap = claude::usage_snapshot(&limits(fable), now);
            let mut panel = claude_panel(&snap);
            panel.compact = true;
            assert_eq!(
                panel.rows.len(),
                2 + usize::from(fable.is_some()),
                "{fable:?}"
            );
            // The pair is always the 5 h and the 7 j window, never Fable.
            let pair = paired_panel_rows(&panel);
            assert_eq!(pair.len(), 2);
            assert!(pair.iter().all(|r| !r.model));
            // 7 j keeps its countdown and its own reset line, with or without Fable.
            assert_eq!(pair[1].countdown, "3j 8h");
            assert!(compact_reset(&pair[1].reset).starts_with("Réinit. le "));
            assert!(compact_reset(&pair[0].reset).starts_with("Réinit. à "));
            // Fable: a real row only when the provider reported it, never invented.
            let fable_rows: Vec<_> = panel.rows.iter().filter(|r| r.model).collect();
            assert_eq!(fable_rows.len(), usize::from(fable.is_some()));
            if let Some(f) = fable {
                assert_eq!(fable_rows[0].value, format!("{} %", 100.0 - f));
            } else {
                assert!(compact_fable(&panel).is_none());
                assert_eq!(pair_block_height(&panel), 60.0);
            }
            // Fable adds 5 pt for a fine track and gap, never a caption or full row.
            assert_eq!(
                card_height(&panel),
                16.0 + PAIR_BLOCK_HEIGHT + if fable.is_some() { 5.0 } else { 0.0 }
            );
        }
        // Weekly window absent: Fable never takes the weekly slot.
        let only = claude::usage_snapshot(
            &serde_json::json!({"rate_limits":{
                "five_hour":{"utilization":27,"resets_at":soon},
                "model_scoped":[{"display_name":"Fable","utilization":30,"resets_at":week}]
            }}),
            now,
        );
        let mut panel = claude_panel(&only);
        panel.compact = true;
        assert!(paired_panel_rows(&panel).iter().all(|r| !r.model));
        assert!(panel.rows.iter().any(|r| r.model));
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

    #[cfg(target_os = "macos")]
    fn fixture_account(
        id: &str,
        visible: bool,
        rate_limits: serde_json::Value,
    ) -> claude::AccountSnapshot {
        claude::AccountSnapshot {
            profile: claude_accounts::Profile {
                id: id.into(),
                path: format!("/scratch/{id}"),
                name: id.into(),
                visible,
            },
            primary: id == "Personnel",
            available: true,
            refreshing: false,
            snapshot: claude::usage_snapshot(
                &serde_json::json!({"subscription_type":"pro","rate_limits":rate_limits}),
                Local::now().timestamp(),
            ),
        }
    }

    #[cfg(target_os = "macos")]
    fn fixture_limits(used5: f64, used7: f64, minutes5: i64, hours7: i64) -> serde_json::Value {
        let at = |d: chrono::Duration| (Local::now() + d).to_rfc3339();
        serde_json::json!({
            "five_hour":{"utilization":used5,"resets_at":at(chrono::Duration::minutes(minutes5))},
            "seven_day":{"utilization":used7,"resets_at":at(chrono::Duration::hours(hours7))},
            "model_scoped":[{"display_name":"Fable","utilization":10,"resets_at":at(chrono::Duration::hours(hours7 + 1))}],
            "extra_usage":{"is_enabled":true,"spend_limit_reached":true}
        })
    }

    #[cfg(target_os = "macos")]
    fn codex_fixture(
        name: &str,
        primary: bool,
        visible: bool,
        rows: &[QuotaPayload],
    ) -> codex::CodexAccount {
        let found = codex_accounts::Discovered {
            profile: claude_accounts::Profile {
                id: format!("profile:{name}"),
                path: format!("/fixture/{name}"),
                name: name.into(),
                visible,
            },
            primary,
            tracked: true,
            launch: (!primary).then(|| format!("/fixture/{name}").into()),
            available: true,
            visible,
        };
        let status = codex::Status {
            phase: "ready".into(),
            message: "Quotas synchronisés directement avec Codex.".into(),
            ..codex::Status::default()
        };
        let reading = codex::Reading {
            phase: "ready".into(),
            message: status.message.clone(),
            rows: Some(rows.to_vec()),
            ..Default::default()
        };
        let view = codex::PrimaryView {
            status,
            rows: rows.to_vec(),
            identity: None,
        };
        codex::account_view(
            found,
            true,
            Some(&view),
            (!primary).then_some((reading, false)),
        )
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn account_states_are_shown_on_each_card_without_a_detail_route() {
        let rows = [
            payload("five-hour", 63, Some(300)),
            payload("weekly", 85, Some(10080)),
        ];
        let mut signed_out = codex_fixture("Perso", false, true, &[]);
        signed_out.phase = "signed_out".into();
        let mut dup = codex_fixture("Alias", false, true, &rows);
        dup.duplicate_of = Some("Principal".into());
        let codex_accounts = vec![
            codex_fixture("Principal", true, true, &rows),
            codex_fixture("Travail", false, false, &rows),
            signed_out,
            dup,
        ];
        let states: Vec<_> = codex_accounts
            .iter()
            .map(|a| account_menu::codex_state(a).0)
            .collect();
        assert_eq!(
            states,
            ["Connecté", "Connecté", "Non connecté", "Même compte"]
        );
        let limits = fixture_limits(27.0, 14.0, 210, 80);
        let mut signed_out_claude = fixture_account("Travail", false, limits.clone());
        signed_out_claude.snapshot.state = "error";
        signed_out_claude.snapshot.error_kind = Some("not_signed_in");
        assert_eq!(
            account_menu::claude_state(&fixture_account("Personnel", true, limits)).0,
            "Connecté"
        );
        assert_eq!(
            account_menu::claude_state(&signed_out_claude),
            ("Non connecté".into(), Tone::Warning)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compact_window_groups_cards_by_provider_with_their_state_and_no_detail_route() {
        let rows = [
            payload("five-hour", 63, Some(300)),
            payload("weekly", 85, Some(10080)),
        ];
        let mut signed_out = codex_fixture("Perso", false, true, &[]);
        signed_out.phase = "signed_out".into();
        let codex_accounts = vec![
            codex_fixture("Principal", true, true, &rows),
            codex_fixture("Travail", false, true, &rows),
            signed_out,
        ];
        let claude_accounts = vec![fixture_account(
            "Personnel",
            true,
            fixture_limits(27.0, 14.0, 210, 80),
        )];
        let prefs = prefs::Prefs {
            claude_enabled: true,
            ..Default::default()
        };
        let providers = quota_providers(&prefs, &codex_accounts, &claude_accounts);
        let model = PanelModel {
            addable: account_menu::addable_providers(&prefs),
            header: panel_header(&providers, None),
            providers,
            connection: None,
        };
        let panel = compact_panel(&model, "9.9.9", None);
        let cards: Vec<_> = panel
            .providers
            .iter()
            .map(|p| (p.kind, p.state.as_ref().map(|s| s.label.as_str())))
            .collect();
        assert_eq!(
            cards,
            [
                ("codex", Some("Connecté")),
                ("codex", Some("Connecté")),
                ("codex", Some("Non connecté")),
                ("claude", Some("Connecté")),
            ]
        );
        // Each enabled provider gets its header (↗) and its +, once.
        assert_eq!(panel.addable, ["codex", "claude"]);
        let json = serde_json::to_value(&panel).unwrap();
        assert!(
            json["providers"][1].get("accountId").is_none(),
            "no account detail route is serialised"
        );
        assert_eq!(json["providers"][2]["state"]["tone"], "warning");
        assert_eq!(json["addable"], serde_json::json!(["codex", "claude"]));
        // Codex off: its cards and its + disappear together.
        let off = prefs::Prefs {
            codex_enabled: false,
            claude_enabled: true,
            ..Default::default()
        };
        let providers = quota_providers(&off, &codex_accounts, &claude_accounts);
        let model = PanelModel {
            addable: account_menu::addable_providers(&off),
            header: panel_header(&providers, None),
            providers,
            connection: None,
        };
        let panel = compact_panel(&model, "9.9.9", None);
        assert_eq!(panel.addable, ["claude"]);
        assert!(panel.providers.iter().all(|p| p.kind == "claude"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn one_visible_codex_account_is_the_exact_historical_card() {
        let rows = [
            payload("five-hour", 63, Some(300)),
            payload("weekly", 85, Some(10080)),
        ];
        for layout in ["compact", "classic"] {
            let historical = codex_presentation(
                &rows,
                "ready",
                "Quotas synchronisés directement avec Codex.",
                None,
                layout,
            );
            // Unique (primary only) and Multi with a hidden extra.
            for accounts in [
                vec![codex_fixture("Principal", true, true, &rows)],
                vec![
                    codex_fixture("Principal", true, true, &rows),
                    codex_fixture("Travail", false, false, &rows[..1]),
                ],
            ] {
                let mut panels = codex_account_panels(&accounts, layout);
                // The card only gains the account it belongs to (menu/compact routing).
                assert_eq!(panels[0].account.as_ref().map(|a| a.kind), Some("codex"));
                panels[0].account = None;
                assert_eq!(panels, vec![historical.clone()]);
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn several_codex_accounts_get_named_cards_following_the_single_layout() {
        let rows = [
            payload("five-hour", 63, Some(300)),
            payload("weekly", 85, Some(10080)),
        ];
        let work_rows = [
            payload("five-hour", 12, Some(300)),
            payload("weekly", 40, Some(10080)),
        ];
        let mut signed_out = codex_fixture("Perso", false, true, &[]);
        signed_out.phase = "signed_out".into();
        signed_out.message = "Non connecté. Connectez ce dossier dans un terminal : CODEX_HOME=/fixture/Perso codex login".into();
        let mut dup = codex_fixture("Alias", false, true, &rows);
        dup.duplicate_of = Some("Principal".into());
        let accounts = vec![
            codex_fixture("Principal", true, true, &rows),
            codex_fixture("Travail", false, true, &work_rows),
            signed_out,
            dup,
            codex_fixture("Caché", false, false, &work_rows),
        ];
        let compact = codex_account_panels(&accounts, "compact");
        let classic = codex_account_panels(&accounts, "classic");
        let names: Vec<_> = compact.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Codex · Principal",
                "Codex · Travail",
                "Codex · Perso",
                "Codex · Alias"
            ]
        );
        assert!(compact.iter().all(|p| p.compact) && classic.iter().all(|p| !p.compact));
        // Each named card pairs its own real windows (not only the plain "Codex").
        assert_eq!(paired_panel_rows(&compact[0]).len(), 2);
        assert_eq!(paired_panel_rows(&compact[1]).len(), 2);
        // Named card with a weekly-only real window + another bucket: only the
        // real same-bucket window is paired (Codex rule, not "first two rows").
        let weekly = [
            payload("codex:codex:secondary", 40, Some(10080)),
            payload("codex:review:secondary", 70, Some(10080)),
        ];
        let pair = codex_account_panels(
            &[
                codex_fixture("Principal", true, true, &rows),
                codex_fixture("Hebdo", false, true, &weekly),
            ],
            "compact",
        );
        assert_eq!(pair[1].name, "Codex · Hebdo");
        let paired = paired_panel_rows(&pair[1]);
        assert_eq!(paired.len(), 1);
        assert_eq!(paired[0].pair_slot, Some(1));
        assert_eq!(compact[0].rows[0].remaining, Some(63));
        assert_eq!(compact[1].rows[0].remaining, Some(12), "no aggregation");
        // Signed out: a note, no invented bars.
        assert!(compact[2].rows.is_empty());
        let (note, tone) = compact[2].note.clone().unwrap();
        assert!(note.contains("Non connecté") && tone == Tone::Warning);
        // Same identity: flagged, values kept apart.
        assert!(compact[3]
            .note
            .as_ref()
            .unwrap()
            .0
            .contains("Même compte que « Principal »"));
        assert_eq!(compact[3].rows[0].remaining, Some(63));
        // Codex disabled: no card at all; Claude-only providers unaffected.
        let mut prefs = prefs::Prefs::default();
        prefs.codex_enabled = false;
        assert!(quota_providers(&prefs, &accounts, &[]).is_empty());
        prefs.codex_enabled = true;
        prefs.layout = "classic".into();
        assert_eq!(quota_providers(&prefs, &accounts, &[]), classic);
        // Compact model (React compact window) keeps the pair slots per card.
        let model = PanelModel {
            addable: Vec::new(),
            header: panel_header(&compact, None),
            providers: compact.clone(),
            connection: None,
        };
        let serialized = compact_panel(&model, "fixture", None);
        assert_eq!(serialized.providers.len(), 4);
        assert_eq!(serialized.providers[1].section, "CODEX · TRAVAIL");
        assert_eq!(
            serialized.providers[1]
                .rows
                .iter()
                .filter(|r| r.pair_slot.is_some())
                .count(),
            2
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn one_saved_layout_switches_codex_and_claude_in_the_native_menu() {
        let rows = [
            payload("five-hour", 63, Some(300)),
            payload("weekly", 85, Some(10080)),
        ];
        let personal = fixture_account("Personnel", true, fixture_limits(27.0, 14.0, 210, 80));
        let build = |layout: &str| {
            // Same file shape as an older build with only the Codex key.
            let prefs = prefs::parse(
                format!(r#"{{"layout":"{layout}","claudeEnabled":true,"codexLayout":"classic"}}"#)
                    .as_bytes(),
            );
            quota_providers(
                &prefs,
                &[codex_fixture("Principal", true, true, &rows)],
                &[personal.clone()],
            )
        };
        let (compact, classic) = (build("compact"), build("classic"));
        assert_eq!((compact.len(), classic.len()), (2, 2));
        assert!(compact.iter().all(|p| p.compact), "both providers paired");
        assert!(
            classic.iter().all(|p| !p.compact),
            "both providers separated"
        );
        // Presentation only: same rows and values either way.
        for (c, k) in compact.iter().zip(&classic) {
            assert_eq!(c.name, k.name);
            assert_eq!(c.rows, k.rows);
            assert!(card_height(c) < card_height(k));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn layout_choice_applies_to_a_single_visible_account() {
        // Unique (one profile) and Multi with one visible: the saved
        // Compacte/Classique choice always wins over the account count.
        let personal = fixture_account("Personnel", true, fixture_limits(27.0, 14.0, 210, 80));
        let hidden = fixture_account("Travail", false, fixture_limits(65.0, 80.0, 42, 49));
        for accounts in [
            vec![personal.clone()],
            vec![personal.clone(), hidden.clone()],
        ] {
            let compact = claude_account_panels(&accounts, false);
            let classic = claude_account_panels(&accounts, true);
            assert_eq!((compact.len(), classic.len()), (1, 1));
            // Classique: exactly the historical v3.1.0 card.
            assert_eq!(classic[0].account.as_ref().map(|a| a.kind), Some("claude"));
            assert_eq!(
                PanelProvider {
                    account: None,
                    ..classic[0].clone()
                },
                claude_panel(&personal.snapshot)
            );
            assert!(!classic[0].compact);
            // Compacte: paired 5 h / 7 j bars, same rows and values; a sole
            // card gets no account name and no added note.
            assert!(
                compact[0].compact,
                "Compacte must pair a sole account's bars"
            );
            assert_eq!(compact[0].name, "Claude");
            assert_eq!(compact[0].rows, classic[0].rows);
            assert_eq!(compact[0].note, None);
            assert!(card_height(&compact[0]) < card_height(&classic[0]));
        }
        // Nothing visible: nothing rendered and nothing invented.
        for classic in [false, true] {
            assert!(claude_account_panels(&[hidden.clone()], classic).is_empty());
            assert!(claude_account_panels(&[], classic).is_empty());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn classic_layout_keeps_one_separated_section_per_account_without_aggregation() {
        let personal = fixture_account("Personnel", true, fixture_limits(27.0, 14.0, 210, 80));
        let work = fixture_account("Travail", true, fixture_limits(65.0, 80.0, 42, 49));
        let compact = claude_account_panels(&[personal.clone(), work.clone()], false);
        let classic = claude_account_panels(&[personal.clone(), work.clone()], true);
        assert_eq!(classic.len(), 2);
        for (account, (c, k)) in [&personal, &work].iter().zip(compact.iter().zip(&classic)) {
            assert!(!k.compact, "classic never pairs bars");
            assert_eq!(k.name, format!("Claude · {}", account.profile.name));
            assert_eq!(k.badge, account.snapshot.plan_type);
            // Same per-account rows as the historical card: 5 h, hebdo, Fable,
            // each with its own percentage and reset phrase.
            assert_eq!(k.rows, claude_panel(&account.snapshot).rows);
            assert_eq!(k.rows, c.rows);
            assert_eq!(k.note, c.note);
            assert!(k.rows[0].reset.starts_with("Réinitialisation"));
            // Separated rows: taller than the paired block by exactly 2 rows - the paired block.
            assert_eq!(
                card_height(k) - card_height(c),
                3.0 * PANEL_ROW_HEIGHT - (PAIR_BLOCK_HEIGHT + 5.0)
            );
        }
        assert_eq!(classic[0].rows[0].value, "73 %");
        assert_eq!(classic[1].rows[0].value, "35 %");
        // Compact window serialises the same choice as the menu.
        let model = PanelModel {
            addable: Vec::new(),
            header: panel_header(&classic, None),
            providers: classic.clone(),
            connection: None,
        };
        let json = serde_json::to_value(compact_panel(&model, "3.1.0", None)).unwrap();
        assert_eq!(json["providers"][0]["compact"], false);
        assert_eq!(json["providers"][1]["section"], "CLAUDE · TRAVAIL");
        assert_eq!(json["providers"][1]["rows"].as_array().unwrap().len(), 3);
        // Nothing visible: nothing rendered, whatever the layout.
        let mut hidden = personal.clone();
        hidden.profile.visible = false;
        assert!(claude_account_panels(&[hidden], true).is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn multi_account_cards_keep_fable_extra_and_countdowns_per_account() {
        let personal = fixture_account("Personnel", true, fixture_limits(27.0, 14.0, 210, 80));
        let work = fixture_account("Travail", true, fixture_limits(65.0, 80.0, 42, 49));
        let multi = claude_account_panels(&[personal, work], false);
        assert_eq!(multi.len(), 2);
        assert!(multi.iter().all(|p| p.compact));
        assert_eq!(multi[0].name, "Claude · Personnel");
        assert_eq!(multi[1].name, "Claude · Travail");
        for panel in &multi {
            // 5 h, hebdomadaire, then the v3.1.0 Fable row: nothing removed.
            assert_eq!(panel.rows.len(), 3);
            assert_eq!(panel.rows[2].label, "Limite Fable");
            assert!(panel.rows[2].model);
            assert!(panel
                .note
                .as_ref()
                .unwrap()
                .0
                .contains("Usage supplémentaire"));
        }
        assert_eq!(multi[0].rows[0].countdown, "3h30");
        assert_eq!(multi[0].rows[1].countdown, "3j 8h");
        assert_eq!(multi[1].rows[0].countdown, "42 min");
        assert_eq!(multi[1].rows[1].countdown, "2j 1h");
        // Fable becomes a third fine bar without caption in the 65 pt compact block.
        let mut unpaired = multi[0].clone();
        unpaired.compact = false;
        assert_eq!(
            card_height(&unpaired) - card_height(&multi[0]),
            3.0 * PANEL_ROW_HEIGHT - (PAIR_BLOCK_HEIGHT + 5.0)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compact_window_serialises_the_same_multi_cards_as_the_menu() {
        let personal = fixture_account("Personnel", true, fixture_limits(27.0, 14.0, 210, 80));
        let work = fixture_account("Travail", true, fixture_limits(65.0, 80.0, 42, 49));
        let providers = claude_account_panels(&[personal, work], false);
        let model = PanelModel {
            addable: Vec::new(),
            header: panel_header(&providers, None),
            providers: providers.clone(),
            connection: None,
        };
        let panel = compact_panel(&model, "3.1.0", None);
        assert_eq!(panel.providers.len(), 2);
        for (menu, window) in providers.iter().zip(&panel.providers) {
            assert_eq!(window.name, menu.name);
            assert_eq!(window.section, menu.name.to_uppercase());
            assert_eq!(window.compact, menu.compact);
            assert_eq!(window.rows.len(), menu.rows.len());
            for (m, w) in menu.rows.iter().zip(&window.rows) {
                assert_eq!(w.countdown, m.countdown);
                assert_eq!(w.value, m.value);
                assert_eq!(w.label, m.label);
            }
        }
        let json = serde_json::to_value(&panel).unwrap();
        assert_eq!(json["providers"][1]["compact"], true);
        assert_eq!(json["providers"][1]["rows"][0]["countdown"], "42 min");
        assert_eq!(json["providers"][1]["section"], "CLAUDE · TRAVAIL");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn header_freshness_uses_the_newest_visible_claude_account() {
        let mut older = fixture_account("Personnel", true, fixture_limits(1.0, 1.0, 5, 5)).snapshot;
        let mut newer = older.clone();
        older.observed_at = Some("2030-01-01T10:00:00Z".into());
        newer.observed_at = Some("2030-01-01T10:05:00Z".into());
        let at = latest_refresh(&[], &[&older, &newer]).unwrap();
        assert_eq!(
            at.with_timezone(&chrono::Utc).to_rfc3339(),
            "2030-01-01T10:05:00+00:00"
        );
        assert!(latest_refresh(&[], &[]).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn multi_track_thickness_is_the_measured_unique_thickness() {
        // The Unique row draws its track/fill at PANEL_TRACK_HEIGHT (5 pt,
        // radius 2.5 in v3.1.0). Multi 5 h reuses it, weekly is finer, gap 2.
        assert_eq!(PANEL_TRACK_HEIGHT, 5.0);
        let ((top5, h5), (top7, h7)) = compact_tracks();
        assert_eq!(h5, PANEL_TRACK_HEIGHT);
        assert!(h7 < h5);
        assert_eq!(h7, 3.0);
        assert_eq!(top7 - top5 - h5, 2.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compact_heading_shows_countdown_and_honest_fallbacks() {
        let row = PanelRow {
            pair_slot: None,
            model: false,
            label: "Limite 5 heures".into(),
            remaining: Some(0),
            value: "Épuisé".into(),
            reset: "—".into(),
            countdown: "—".into(),
        };
        assert_eq!(compact_value(&row), "0 % · épuisé");
        assert_eq!(compact_heading(&row), "Épuisé");
        let live = PanelRow {
            countdown: "42 min".into(),
            ..row.clone()
        };
        assert_eq!(compact_heading(&live), "42 min · Épuisé");
        let missing = PanelRow {
            remaining: None,
            value: "—".into(),
            ..row
        };
        assert_eq!(compact_value(&missing), "—");
        assert_eq!(compact_heading(&missing), "—");
        // A Fable row keeps its model name in its heading.
        let fable = PanelRow {
            pair_slot: None,
            model: true,
            label: "Limite Fable".into(),
            remaining: Some(86),
            value: "86 %".into(),
            reset: "—".into(),
            countdown: "3j 8h".into(),
        };
        assert_eq!(compact_heading(&fable), "Fable · 3j 8h · 86 %");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compact_countdown_uses_local_time_and_honest_fallbacks() {
        let now = 1_800_000_000;
        let reset = |seconds| {
            chrono::DateTime::from_timestamp(now + seconds, 0)
                .unwrap()
                .to_rfc3339()
        };
        let short = reset(210 * 60);
        let week = reset(80 * 3600);
        assert_eq!(reset_countdown(Some(&short), now), "3h30");
        assert_eq!(reset_countdown(Some(&short), now + 60), "3h29");
        assert_eq!(reset_countdown(Some(&week), now), "3j 8h");
        assert_eq!(reset_countdown(Some(&reset(42 * 60)), now), "42 min");
        for at in [None, Some("invalid"), Some("2000-01-01T00:00:00Z")] {
            assert_eq!(reset_countdown(at, now), "—");
        }
    }
    #[test]
    fn tray_text_keeps_v310_lines_for_one_account_and_names_multi_accounts() {
        let snap = |used: f64| {
            claude::usage_snapshot(
                &serde_json::json!({"rate_limits":{"five_hour":{"utilization":used}}}),
                1_700_000_000,
            )
        };
        let account = |name: &str, used: f64, visible: bool| claude::AccountSnapshot {
            profile: claude_accounts::Profile {
                id: name.into(),
                path: format!("/scratch/{name}"),
                name: name.into(),
                visible,
            },
            primary: name == "Personnel",
            available: true,
            refreshing: false,
            snapshot: snap(used),
        };
        let single = claude_tray_lines(&[
            account("Personnel", 27.0, true),
            account("Travail", 65.0, false),
        ]);
        assert_eq!(single[0], "Claude · Limite 5 heures · 73 % restants");
        assert!(single
            .iter()
            .all(|l| !l.contains("Personnel") && !l.contains("Travail")));
        let multi = claude_tray_lines(&[
            account("Personnel", 27.0, true),
            account("Travail", 65.0, true),
        ]);
        assert!(multi.contains(&"Claude · Personnel · Limite 5 heures · 73 % restants".to_string()));
        assert!(multi.contains(&"Claude · Travail · Limite 5 heures · 35 % restants".to_string()));
        let mut failed = account("Travail", 65.0, true);
        failed.snapshot.state = "error";
        assert!(
            claude_tray_lines(&[account("Personnel", 27.0, true), failed])
                .contains(&"Claude · Travail : erreur de lecture".to_string())
        );
        assert!(claude_tray_lines(&[account("Personnel", 1.0, false)]).is_empty());
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
