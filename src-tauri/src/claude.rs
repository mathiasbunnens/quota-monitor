//! Claude quota provider: isolated official CLI get_usage requests per profile.
//! No direct credential reads or session switching. Each profile owns its cache
//! and refresh ticket. Legacy statusLine relay helpers remain for compatibility.
use crate::prefs;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tauri::Manager;

pub const RELAY_FLAG: &str = "--claude-statusline-relay";
const DATA_FILE: &str = "claude-statusline.json";
const MAX_INPUT: u64 = 256 * 1024;
const STALE_AFTER_SECS: i64 = 30 * 60;
const DOC_URL: &str = "https://code.claude.com/docs/en/statusline";
const FIVE_HOUR_SECS: u64 = 5 * 3600;
const SEVEN_DAY_SECS: u64 = 7 * 24 * 3600;

/// Bumped on every enable/disable; late work from an older generation is dropped.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn on_enabled_changed() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
    REFRESH_REQUESTED.store(true, Ordering::Release);
    LIVE.lock().unwrap().invalidate();
}

// ---------------------------------------------------------------- relay file

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub used_percent: f64,
    pub resets_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RelayFile {
    pub schema_version: u32,
    /// Local receipt time (unix seconds): not the provider's collection time.
    pub received_at: i64,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

fn window(value: &Value) -> Option<Window> {
    let used = value.get("used_percentage")?.as_f64()?;
    if !used.is_finite() || !(0.0..=100.0).contains(&used) {
        return None;
    }
    Some(Window {
        used_percent: used,
        resets_at: value.get("resets_at").and_then(Value::as_i64),
    })
}

/// Extracts only the two documented windows. Anything else is ignored.
pub fn parse_event(input: &[u8]) -> Option<(Option<Window>, Option<Window>)> {
    let value: Value = serde_json::from_slice(input).ok()?;
    let limits = value.get("rate_limits");
    let get = |key: &str| limits.and_then(|l| l.get(key)).and_then(window);
    Some((get("five_hour"), get("seven_day")))
}

/// A window absent from an event keeps its previous value only while that
/// value's reset time is still in the future (Claude drops windows after reset).
pub fn merge(
    previous: Option<RelayFile>,
    five: Option<Window>,
    seven: Option<Window>,
    now: i64,
) -> RelayFile {
    let keep = |old: Option<Window>| old.filter(|w| w.resets_at.is_some_and(|r| r > now));
    let (old5, old7) = previous
        .map(|p| (p.five_hour, p.seven_day))
        .unwrap_or_default();
    RelayFile {
        schema_version: 1,
        received_at: now,
        five_hour: five.or_else(|| keep(old5)),
        seven_day: seven.or_else(|| keep(old7)),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        #[cfg(not(unix))]
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)
}

fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Entry point of `--claude-statusline-relay --data-dir <dir>`. Never fails
/// loudly and prints nothing: it must not disturb Claude Code.
pub fn relay_main(args: &[String]) -> i32 {
    let Some(dir) = args
        .iter()
        .position(|a| a == "--data-dir")
        .and_then(|i| args.get(i + 1))
    else {
        return 0;
    };
    let mut input = Vec::new();
    if std::io::stdin()
        .take(MAX_INPUT)
        .read_to_end(&mut input)
        .is_err()
    {
        return 0;
    }
    let Some((five, seven)) = parse_event(&input) else {
        return 0;
    };
    let path = Path::new(dir).join(DATA_FILE);
    let previous = fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<RelayFile>(&b).ok());
    let merged = merge(previous, five, seven, unix_now());
    if let Ok(bytes) = serde_json::to_vec(&merged) {
        let _ = write_private(&path, &bytes);
    }
    0
}

// ------------------------------------------------------------------ snapshot

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    kind: &'static str,
    documentation_url: &'static str,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Capability {
    refresh: &'static str,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Metric {
    pub id: &'static str,
    pub label: &'static str,
    pub used_percent: Option<f64>,
    pub reset_at: Option<String>,
    /// available | not_reported | unavailable
    pub availability: &'static str,
    pub period_duration_seconds: u64,
    pub exhausted: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSnapshot {
    pub provider: &'static str,
    pub schema_version: u32,
    pub enabled: bool,
    pub configured: bool,
    /// disabled | not_configured | waiting_for_data | ready | stale | error
    pub state: &'static str,
    pub source: Option<Source>,
    pub observed_at: Option<String>,
    pub provider_fetched_at: Option<String>,
    pub message: String,
    pub error_kind: Option<&'static str>,
    pub metrics: Vec<Metric>,
    pub capability: Capability,
    /// Subscription reported by get_usage (e.g. "pro", "max"); never account identity.
    pub plan_type: Option<String>,
    /// Paid extra usage on top of the subscription: enabled / limit reached.
    pub extra_usage: Option<ExtraUsage>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtraUsage {
    pub enabled: bool,
    pub limit_reached: bool,
}

/// Keeps only a short plan token such as "pro" or "max_20x".
pub fn plan_token(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 24)
        .filter(|s| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
        .map(str::to_ascii_lowercase)
}

pub fn extra_usage(value: &serde_json::Value) -> Option<ExtraUsage> {
    let enabled = value["is_enabled"].as_bool()?;
    Some(ExtraUsage {
        enabled,
        limit_reached: value["spend_limit_reached"].as_bool().unwrap_or(false),
    })
}

pub enum FileState {
    Missing,
    Malformed,
    Data(RelayFile),
}

fn iso(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn metric(
    id: &'static str,
    label: &'static str,
    period: u64,
    value: Option<&Window>,
    now: i64,
) -> Metric {
    let base = Metric {
        id,
        label,
        used_percent: None,
        reset_at: None,
        availability: "not_reported",
        period_duration_seconds: period,
        exhausted: false,
    };
    let Some(w) = value else { return base };
    if w.resets_at.is_some_and(|r| r <= now) {
        // The window rolled over after the last event: old numbers are not valid.
        return Metric {
            availability: "unavailable",
            reset_at: w.resets_at.and_then(iso),
            ..base
        };
    }
    Metric {
        used_percent: Some(w.used_percent),
        reset_at: w.resets_at.and_then(iso),
        availability: "available",
        exhausted: w.used_percent >= 100.0,
        ..base
    }
}

pub fn compute(enabled: bool, configured: bool, file: FileState, now: i64) -> ClaudeSnapshot {
    let mut snap = ClaudeSnapshot {
        provider: "claude",
        schema_version: 1,
        enabled,
        configured,
        state: "disabled",
        source: Some(Source {
            kind: "official_cli_statusline",
            documentation_url: DOC_URL,
        }),
        observed_at: None,
        provider_fetched_at: None,
        message: String::new(),
        error_kind: None,
        metrics: vec![],
        capability: Capability {
            refresh: "event_only",
        },
        plan_type: None,
        extra_usage: None,
    };
    if !enabled {
        snap.message = "Claude est désactivé dans les Réglages.".into();
        return snap;
    }
    if !configured {
        snap.state = "not_configured";
        snap.message =
            "Claude Code introuvable. Vérifiez son installation et redémarrez Quota Monitor."
                .into();
        return snap;
    }
    let data = match file {
        FileState::Missing => {
            snap.state = "waiting_for_data";
            snap.message = "En attente d’une première activité de Claude Code.".into();
            return snap;
        }
        FileState::Malformed => {
            snap.state = "error";
            snap.error_kind = Some("malformed");
            snap.message = "Les données reçues de Claude Code sont illisibles.".into();
            return snap;
        }
        FileState::Data(data) => data,
    };
    snap.observed_at = iso(data.received_at);
    snap.metrics = vec![
        metric(
            "claude:five_hour",
            "Limite 5 heures",
            FIVE_HOUR_SECS,
            data.five_hour.as_ref(),
            now,
        ),
        metric(
            "claude:seven_day",
            "Limite hebdomadaire",
            SEVEN_DAY_SECS,
            data.seven_day.as_ref(),
            now,
        ),
    ];
    if !snap.metrics.iter().any(|m| m.availability == "available") {
        snap.state = "waiting_for_data";
        snap.message =
            "Claude Code n’a communiqué aucune limite valide pour le moment (elles apparaissent après la première réponse d’une session).".into();
    } else if now - data.received_at > STALE_AFTER_SECS {
        snap.state = "stale";
        snap.message =
            "Dernière observation ancienne : Claude Code n’a rien envoyé récemment.".into();
    } else {
        snap.state = "ready";
    }
    snap
}

// ------------------------------------------------------------ paths & config

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().app_local_data_dir().map_err(|e| e.to_string())
}

fn claude_settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| app.path().home_dir().ok().map(|h| h.join(".claude")))
        .ok_or("Dossier utilisateur introuvable.")?;
    Ok(dir.join("settings.json"))
}

#[cfg(test)]
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn is_ours(settings: &Value) -> bool {
    settings["statusLine"]["command"]
        .as_str()
        .is_some_and(|c| c.contains(RELAY_FLAG))
}

fn read_settings(path: &Path) -> Result<Option<Value>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .ok()
            .filter(Value::is_object)
            .map(Some)
            .ok_or_else(|| {
                "Le fichier de réglages de Claude Code est illisible ; il n’a pas été modifié."
                    .into()
            }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Impossible de lire les réglages de Claude Code.".into()),
    }
}

#[cfg(test)]
pub fn relay_installed(settings_path: &Path) -> bool {
    matches!(read_settings(settings_path), Ok(Some(v)) if is_ours(&v))
}

/// Installs the relay only when Claude Code has no statusLine of its own (kept for the migration tests).
#[cfg(test)]
pub fn install_relay(settings_path: &Path, exe: &Path, data_dir: &Path) -> Result<(), String> {
    let mut settings = read_settings(settings_path)?.unwrap_or_else(|| json!({}));
    if settings.get("statusLine").is_some() && !is_ours(&settings) {
        return Err("Claude Code utilise déjà une statusLine personnalisée. Elle n’est pas remplacée : retire-la toi-même ou chaîne-la manuellement avant d’activer le relais.".into());
    }
    let backup = settings_path.with_extension("json.quota-monitor.bak");
    if settings_path.exists() && !backup.exists() {
        fs::copy(settings_path, &backup).map_err(|e| e.to_string())?;
    }
    settings["statusLine"] = json!({
        "type": "command",
        "command": format!("{} {} --data-dir {}", quote(exe), RELAY_FLAG, quote(data_dir)),
    });
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    write_private(settings_path, &bytes).map_err(|e| e.to_string())
}

/// The statusLine relay was the data source of Quota Monitor 3.0.x. Collection
/// now goes through the official CLI (`get_usage`), so a relay entry left in
/// the Claude Code settings only launches this executable for nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum RelayMigration {
    /// No entry of ours (absent file, another statusLine): nothing was touched.
    NotNeeded,
    /// Our entry was removed; every other setting is kept as it was.
    Removed,
    /// Settings unreadable or not writable: left as they were.
    Skipped,
}

/// Silent, idempotent migration: removes ONLY a statusLine that launches this
/// app's relay. The user's own statusLine and unreadable files are never
/// modified; the backup written at install time stays in place.
pub fn migrate_legacy_relay(settings_path: &Path) -> RelayMigration {
    match read_settings(settings_path) {
        Err(_) => RelayMigration::Skipped,
        Ok(Some(settings)) if is_ours(&settings) => match uninstall_relay(settings_path) {
            Ok(()) => RelayMigration::Removed,
            Err(_) => RelayMigration::Skipped,
        },
        Ok(_) => RelayMigration::NotNeeded,
    }
}

/// Startup hook: migrates the primary profile's settings (where the relay was installed).
pub fn migrate_legacy_relay_at_startup(app: &tauri::AppHandle) {
    if let Ok(path) = claude_settings_path(app) {
        let _ = migrate_legacy_relay(&path);
    }
}

pub fn uninstall_relay(settings_path: &Path) -> Result<(), String> {
    let Some(mut settings) = read_settings(settings_path)? else {
        return Ok(());
    };
    if settings.get("statusLine").is_none() {
        return Ok(());
    }
    if !is_ours(&settings) {
        return Err(
            "La statusLine actuelle n’appartient pas à Quota Monitor ; elle n’est pas modifiée."
                .into(),
        );
    }
    settings.as_object_mut().map(|o| o.remove("statusLine"));
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    write_private(settings_path, &bytes).map_err(|e| e.to_string())
}

fn read_file_state(dir: &Path) -> FileState {
    match fs::read(dir.join(DATA_FILE)) {
        Ok(bytes) if bytes.len() as u64 <= MAX_INPUT => serde_json::from_slice::<RelayFile>(&bytes)
            .ok()
            .filter(|f| f.schema_version == 1)
            .map(FileState::Data)
            .unwrap_or(FileState::Malformed),
        Ok(_) => FileState::Malformed,
        Err(_) => FileState::Missing,
    }
}

static LIVE: std::sync::Mutex<crate::claude_accounts::RefreshStore> =
    std::sync::Mutex::new(crate::claude_accounts::RefreshStore::new());
static REFRESH_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    #[serde(flatten)]
    pub profile: crate::claude_accounts::Profile,
    pub primary: bool,
    pub available: bool,
    pub refreshing: bool,
    pub snapshot: ClaudeSnapshot,
}
fn profiles(app: &tauri::AppHandle, p: &prefs::Prefs) -> Vec<crate::claude_accounts::Discovered> {
    let Ok(home) = app.path().home_dir() else {
        return vec![];
    };
    let config = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    crate::claude_accounts::discover(&home, config.as_deref(), &p.claude_accounts)
}
/// Display view of every discovered profile (Fable preference applied per
/// account). Visibility is presentation only: hidden profiles are included.
pub fn accounts(app: &tauri::AppHandle) -> Vec<AccountSnapshot> {
    let fable = prefs::current(app).fable_enabled;
    collect_accounts(app, Some(fable))
}
/// Raw view (no Fable filtering/coalescing) for notifications.
pub fn raw_accounts(app: &tauri::AppHandle) -> Vec<AccountSnapshot> {
    collect_accounts(app, None)
}
fn collect_accounts(app: &tauri::AppHandle, fable: Option<bool>) -> Vec<AccountSnapshot> {
    let p = prefs::current(app);
    let executable = crate::claude_cli::discover().is_some();
    profiles(app, &p)
        .into_iter()
        .map(|a| {
            let store = LIVE.lock().unwrap();
            let snap = store.snapshot(&a.profile.id).unwrap_or_else(|| {
                compute(p.claude_enabled, executable, FileState::Missing, unix_now())
            });
            let refreshing = store.refreshing(&a.profile.id);
            drop(store);
            let mut profile = a.profile;
            profile.visible = a.visible && p.claude_enabled;
            AccountSnapshot {
                profile,
                primary: a.primary,
                available: a.available,
                refreshing,
                snapshot: account_view(snap, p.claude_enabled, a.available, fable),
            }
        })
        .collect()
}
/// Per-account projection. `fable: Some(enabled)` is the display view (v3.1.0
/// Fable visibility + "Limite Fable" coalescing); `None` keeps raw metrics.
pub(crate) fn account_view(
    snap: ClaudeSnapshot,
    enabled: bool,
    available: bool,
    fable: Option<bool>,
) -> ClaudeSnapshot {
    let snap = prepare_snapshot(snap, enabled, available);
    match fable {
        Some(fable_enabled) => filter_fable(snap, fable_enabled),
        None => snap,
    }
}
#[tauri::command]
pub fn get_claude_accounts(app: tauri::AppHandle) -> Vec<AccountSnapshot> {
    accounts(&app)
}
#[tauri::command]
pub fn add_claude_profile(app: tauri::AppHandle, path: String) -> Result<prefs::Prefs, String> {
    follow_profile(app, crate::claude_accounts::FollowTarget::Path(&path))
}
/// « Suivre un autre compte Claude » with a profile already detected.
#[tauri::command]
pub fn follow_claude_profile(app: tauri::AppHandle, id: String) -> Result<prefs::Prefs, String> {
    follow_profile(app, crate::claude_accounts::FollowTarget::Detected(&id))
}
/// « Supprimer <compte> » : stops tracking (preference only, no file touched).
#[tauri::command]
pub fn untrack_claude_profile(app: tauri::AppHandle, id: String) -> Result<prefs::Prefs, String> {
    let mut p = prefs::current(&app);
    let home = app
        .path()
        .home_dir()
        .map_err(|_| "Dossier utilisateur introuvable.")?;
    let config = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    p.claude_accounts =
        crate::claude_accounts::remove(&home, config.as_deref(), &p.claude_accounts, &id)?;
    prefs::set_preferences(app, p)
}
fn follow_profile(
    app: tauri::AppHandle,
    target: crate::claude_accounts::FollowTarget,
) -> Result<prefs::Prefs, String> {
    let mut p = prefs::current(&app);
    let home = app
        .path()
        .home_dir()
        .map_err(|_| "Dossier utilisateur introuvable.")?;
    let config = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    p.claude_accounts =
        crate::claude_accounts::follow(&home, config.as_deref(), &p.claude_accounts, target)?;
    prefs::set_preferences(app, p)
}
/// Legacy consumers and the tray indicator show one real account, never an
/// average. Multi mode uses the first visible account in deterministic order.
pub fn snapshot(app: &tauri::AppHandle) -> ClaudeSnapshot {
    accounts(app)
        .into_iter()
        .find(|a| a.profile.visible)
        .map(|a| a.snapshot)
        .unwrap_or_else(|| compute(false, false, FileState::Missing, unix_now()))
}

fn filter_fable(mut snap: ClaudeSnapshot, enabled: bool) -> ClaudeSnapshot {
    if !enabled {
        snap.metrics.retain(|m| m.id != "claude:weekly_fable");
    }
    snap.metrics = display_metrics(&snap.metrics);
    snap
}

/// Presentation only: never change raw observations or notification identities.
/// Equal rounded percentages alone are not evidence of a duplicate window.
pub(crate) fn display_metrics(metrics: &[Metric]) -> Vec<Metric> {
    let duplicate = metrics
        .iter()
        .find(|m| m.id == "claude:weekly_fable")
        .filter(|f| {
            f.availability == "available" && f.used_percent.is_some() && f.reset_at.is_some()
        })
        .is_some_and(|f| {
            metrics.iter().any(|w| {
                w.id == "claude:seven_day"
                    && w.availability == "available"
                    && w.used_percent == f.used_percent
                    && w.reset_at == f.reset_at
                    && w.period_duration_seconds == f.period_duration_seconds
            })
        });
    let mut rows = Vec::new();
    for m in metrics {
        if duplicate && m.id == "claude:seven_day" {
            rows.push(
                metrics
                    .iter()
                    .find(|m| m.id == "claude:weekly_fable")
                    .unwrap()
                    .clone(),
            );
        } else if !(duplicate && m.id == "claude:weekly_fable") {
            rows.push(m.clone());
        }
    }
    for m in &mut rows {
        if m.id == "claude:weekly_fable" {
            m.label = "Limite Fable";
        }
    }
    rows
}

fn prepare_snapshot(mut snap: ClaudeSnapshot, enabled: bool, available: bool) -> ClaudeSnapshot {
    snap.enabled = enabled;
    snap.source = Some(Source {
        kind: "official_cli_control",
        documentation_url: "https://code.claude.com/docs/en/cli-reference",
    });
    snap.capability = Capability {
        refresh: "on_demand",
    };
    for metric in &mut snap.metrics {
        if metric
            .reset_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|d| d.timestamp() <= unix_now())
        {
            metric.used_percent = None;
            metric.availability = "unavailable";
            metric.exhausted = false;
        }
    }
    if !enabled {
        snap.state = "disabled";
        snap.metrics.clear();
    } else if !available {
        snap.state = "error";
        snap.error_kind = Some("profile_unavailable");
        snap.message = "Dossier de profil Claude indisponible.".into();
        snap.metrics.clear();
    } else if !snap.configured {
        snap.message =
            "Claude Code introuvable. Vérifiez son installation et redémarrez Quota Monitor."
                .into();
    } else if snap.observed_at.is_none() && snap.error_kind.is_none() {
        snap.message = "Lecture des quotas via Claude Code…".into();
    }
    snap
}

pub(crate) fn usage_snapshot(value: &Value, now: i64) -> ClaudeSnapshot {
    let limits = &value["rate_limits"];
    let data = RelayFile {
        schema_version: 1,
        received_at: now,
        five_hour: crate::claude_cli::window(&limits["five_hour"]),
        seven_day: crate::claude_cli::window(&limits["seven_day"]),
    };
    let mut snap = compute(true, true, FileState::Data(data), now);
    if let Some(fable) = crate::claude_cli::fable_window(limits) {
        let fable = metric(
            "claude:weekly_fable",
            "Limite hebdomadaire Fable",
            SEVEN_DAY_SECS,
            Some(&fable),
            now,
        );
        if fable.availability == "available" {
            snap.state = "ready";
        }
        snap.metrics.push(fable);
    }
    snap.message = "Quotas lus à la demande via Claude Code.".into();
    snap.plan_type = plan_token(&value["subscription_type"]);
    snap.extra_usage = extra_usage(&limits["extra_usage"]);
    snap
}

fn refresh_live(app: &tauri::AppHandle) {
    let generation = GENERATION.load(Ordering::Acquire);
    let p = prefs::current(app);
    if !p.claude_enabled {
        return;
    }
    let executable = crate::claude_cli::discover();
    let Ok(dir) = data_dir(app) else {
        return;
    };
    // Refresh is serial and each call uses a distinct canonical profile path.
    for a in profiles(app, &p) {
        if GENERATION.load(Ordering::Acquire) != generation || !prefs::current(app).claude_enabled {
            break;
        }
        poll_profile(&LIVE, &a, executable.as_deref(), &dir, || {
            crate::publish_quotas(app)
        });
    }
}
fn poll_profile(
    store: &std::sync::Mutex<crate::claude_accounts::RefreshStore>,
    account: &crate::claude_accounts::Discovered,
    executable: Option<&Path>,
    dir: &Path,
    mut changed: impl FnMut(),
) -> bool {
    let profile = &account.profile;
    let Some(ticket) = store.lock().unwrap().begin(&profile.id) else {
        return false;
    };
    changed();
    let previous = store.lock().unwrap().snapshot(&profile.id);
    let snap = if let Some(exe) = executable {
        let cwd = profile_workspace(dir, &profile.id);
        let result = collect(account, exe, &cwd);
        result_snapshot_for(result, previous, unix_now(), account.launch.as_deref())
    } else {
        compute(true, false, FileState::Missing, unix_now())
    };
    let committed = store.lock().unwrap().finish(ticket, snap);
    if committed {
        changed();
    }
    committed
}
/// Primary: exactly as 3.1.0 — inherit the app environment untouched (Claude
/// Code derives its credential slot from CLAUDE_CONFIG_DIR's presence/value, so
/// forcing it, even to ~/.claude, signs the child out). Extra profiles: the
/// path string as discovered/entered, never canonicalized.
fn collect(
    account: &crate::claude_accounts::Discovered,
    exe: &Path,
    cwd: &Path,
) -> Result<Value, &'static str> {
    match account.launch.as_deref() {
        None => crate::claude_cli::read_usage(exe, cwd),
        Some(config) => crate::claude_cli::read_usage_for(exe, cwd, config),
    }
}
fn profile_workspace(dir: &Path, id: &str) -> PathBuf {
    // Bound each path component on filesystems with NAME_MAX=255.
    id.as_bytes()
        .chunks(64)
        .fold(dir.join("claude-cli-workspaces"), |p, part| {
            p.join(std::str::from_utf8(part).unwrap())
        })
}
/// `launch` is the account's CLAUDE_CONFIG_DIR (None = primary, inherited env).
pub(crate) fn result_snapshot_for(
    result: Result<Value, &'static str>,
    previous: Option<ClaudeSnapshot>,
    now: i64,
    launch: Option<&Path>,
) -> ClaudeSnapshot {
    let signed_out = matches!(&result, Ok(v) if v["rate_limits_available"] == false
        && v["subscription_type"].is_null());
    let mut snap = result_snapshot(result, previous, now);
    if signed_out {
        snap.error_kind = Some("not_signed_in");
        snap.message = match launch {
            None => "Claude non connecté. Connectez-vous avec claude auth login.".into(),
            Some(path) => format!(
                "Profil Claude non connecté. Connectez-vous avec CLAUDE_CONFIG_DIR={} claude auth login.",
                shell_word(&path.to_string_lossy())
            ),
        };
    }
    snap
}
/// Copyable POSIX shell word for the actionable login hint.
pub(crate) fn shell_word(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+@%,".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}
pub(crate) fn result_snapshot(
    result: Result<Value, &'static str>,
    previous: Option<ClaudeSnapshot>,
    now: i64,
) -> ClaudeSnapshot {
    let mut snap = match result {
        // v3.1.0 parser: five_hour/seven_day + model_scoped Fable. The legacy
        // seven_day_{fable,sonnet,opus} keys of 2d298ad are not reintroduced
        // (they would duplicate Fable and change the Unique presentation).
        Ok(value) if value["rate_limits_available"] == true => usage_snapshot(&value, now),
        Ok(_) => {
            let mut s = compute(true, true, FileState::Missing, now);
            s.state = "error";
            s.error_kind = Some("auth");
            s.message = "Limites d’abonnement indisponibles. Vérifiez claude auth login (compte claude.ai).".into();
            s
        }
        Err(kind) => {
            let mut s = previous.unwrap_or_else(|| compute(true, true, FileState::Missing, now));
            s.state = "error";
            s.error_kind = Some(kind);
            s.message = "Lecture Claude impossible : vérifiez la connexion et une version de Claude Code compatible avec get_usage. Dernières valeurs éventuellement conservées, non actualisées.".into();
            s
        }
    };
    snap.source = Some(Source {
        kind: "official_cli_control",
        documentation_url: "https://code.claude.com/docs/en/cli-reference",
    });
    snap.capability = Capability {
        refresh: "on_demand",
    };
    snap
}

// ------------------------------------------------------------------ commands

#[tauri::command]
pub fn get_claude_snapshot(app: tauri::AppHandle) -> ClaudeSnapshot {
    snapshot(&app)
}

/// Coalesces a quota-only CLI refresh for all profiles; never touches Codex.
#[tauri::command]
pub fn request_claude_refresh(app: tauri::AppHandle) -> ClaudeSnapshot {
    REFRESH_REQUESTED.store(true, Ordering::Release);
    snapshot(&app)
}

fn refresh_due(
    elapsed: Duration,
    interval: Duration,
    requested: &std::sync::atomic::AtomicBool,
) -> bool {
    let manual = requested.swap(false, Ordering::AcqRel);
    elapsed >= interval || manual
}

/// One serial collector, idle while disabled. View-only choices do not filter
/// collection, and reset-time changes from every account update the native menu.
pub fn start_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<Vec<AccountSnapshot>> = None;
        #[cfg(target_os = "macos")]
        let mut rendered_minute = 0;
        let mut last_read = std::time::Instant::now() - Duration::from_secs(3600);
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if !prefs::current(&app).claude_enabled {
                last = None;
                continue;
            }
            let interval = app
                .try_state::<crate::codex::Provider>()
                .map(|p| p.status().refresh_seconds)
                .unwrap_or(60)
                .max(15);
            if refresh_due(
                last_read.elapsed(),
                Duration::from_secs(interval),
                &REFRESH_REQUESTED,
            ) {
                refresh_live(&app);
                last_read = std::time::Instant::now();
            }
            let generation = GENERATION.load(Ordering::Acquire);
            let next = accounts(&app);
            if generation != GENERATION.load(Ordering::Acquire) {
                continue; // disabled/re-enabled meanwhile: drop the stale result
            }
            if last.as_ref() != Some(&next) {
                last = Some(next);
                crate::publish_quotas(&app);
            }
            #[cfg(target_os = "macos")]
            {
                // Reuse this watcher's tick; presentation must not trigger
                // collection, notifications or frontend events to move a clock.
                let minute = chrono::Local::now().timestamp() / 60;
                if minute != rendered_minute {
                    rendered_minute = minute;
                    crate::render_native_menu(&app);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;
    fn file(five: Option<(f64, i64)>, seven: Option<(f64, i64)>, at: i64) -> FileState {
        let w = |v: Option<(f64, i64)>| {
            v.map(|(u, r)| Window {
                used_percent: u,
                resets_at: Some(r),
            })
        };
        FileState::Data(RelayFile {
            schema_version: 1,
            received_at: at,
            five_hour: w(five),
            seven_day: w(seven),
        })
    }

    #[test]
    fn fable_display_merges_only_matching_known_weekly_windows() {
        let reset = iso(NOW + 3600).unwrap();
        let raw = usage_snapshot(
            &json!({"rate_limits": {
                "five_hour":{"utilization":10},
                "seven_day":{"utilization":37.5,"resets_at":reset},
                "model_scoped":[{"display_name":"Fable","utilization":37.5,"resets_at":reset}]
            }}),
            NOW,
        );
        let shown = filter_fable(raw.clone(), true);
        assert_eq!(shown.metrics.len(), 2);
        assert_eq!(shown.metrics[1].id, "claude:weekly_fable");
        assert_eq!(shown.metrics[1].label, "Limite Fable");
        assert_eq!(shown.metrics[1].used_percent, Some(37.5));
        assert_eq!(raw.metrics.len(), 3);
        assert_eq!(raw.metrics[1].id, "claude:seven_day");
        let hidden = filter_fable(raw.clone(), false);
        assert_eq!(hidden.metrics.len(), 2);
        assert_eq!(hidden.metrics[1].id, "claude:seven_day");
        for kind in 0..6 {
            let mut changed = raw.clone();
            let f = &mut changed.metrics[2];
            match kind {
                0 => f.used_percent = Some(37.6),
                1 => f.reset_at = iso(NOW + 7200),
                2 => f.reset_at = None,
                3 => f.used_percent = None,
                4 => f.availability = "unavailable",
                _ => f.period_duration_seconds = FIVE_HOUR_SECS,
            }
            assert_eq!(filter_fable(changed, true).metrics.len(), 3);
        }
        let mut unknown = raw;
        unknown.metrics[1].reset_at = None;
        unknown.metrics[2].reset_at = None;
        assert_eq!(filter_fable(unknown, true).metrics.len(), 3);
    }

    #[test]
    fn fable_preference_filters_display_without_mutating_raw_data() {
        let raw = usage_snapshot(
            &json!({"rate_limits":{
                "five_hour":{"utilization":20}, "seven_day":{"utilization":30},
                "model_scoped":[{"display_name":"Fable","utilization":40}]
            }}),
            NOW,
        );
        let hidden = filter_fable(raw.clone(), false);
        assert_eq!(hidden.metrics.len(), 2);
        assert_eq!(hidden.metrics[0].used_percent, raw.metrics[0].used_percent);
        assert_eq!(hidden.metrics[1].used_percent, raw.metrics[1].used_percent);
        assert_eq!(filter_fable(raw, true).metrics.len(), 3);
        for value in [json!({}), json!({"rate_limits":{"model_scoped":null}})] {
            let snap = usage_snapshot(&value, NOW);
            for enabled in [true, false] {
                assert!(!filter_fable(snap.clone(), enabled)
                    .metrics
                    .iter()
                    .any(|m| m.id == "claude:weekly_fable"));
            }
        }
    }

    #[test]
    fn fable_model_scoped_window_preserves_percentage_reset_and_old_quotas() {
        let reset = iso(NOW + 3600).unwrap();
        let payload = json!({"subscription_type":"max", "rate_limits": {
            "five_hour":{"utilization":23.5,"resets_at":reset},
            "seven_day":{"utilization":42,"resets_at":reset},
            "model_scoped":[{"display_name":"Fable","utilization":99.6,"resets_at":reset}]
        }});
        let snap = usage_snapshot(&payload, NOW);
        assert_eq!(snap.state, "ready");
        assert_eq!(snap.metrics.len(), 3);
        assert_eq!(snap.metrics[0].used_percent, Some(23.5));
        assert_eq!(snap.metrics[1].used_percent, Some(42.0));
        let fable = &snap.metrics[2];
        assert_eq!(fable.id, "claude:weekly_fable");
        assert_eq!(fable.label, "Limite hebdomadaire Fable");
        assert_eq!(fable.used_percent, Some(99.6));
        assert_eq!(fable.reset_at.as_deref(), Some(reset.as_str()));
        assert_eq!(fable.period_duration_seconds, SEVEN_DAY_SECS);
        assert!(!fable.exhausted);
        let expired = usage_snapshot(&payload, NOW + 3600);
        assert_eq!(expired.metrics[2].availability, "unavailable");
        assert_eq!(expired.metrics[2].used_percent, None);
    }

    #[test]
    fn fable_absent_null_invalid_or_other_model_never_fabricates_a_counter() {
        for scoped in [
            Value::Null,
            json!([]),
            json!({}),
            json!([null]),
            json!([{"display_name":"Fable","utilization":null}]),
            json!([{"display_name":"Fable","utilization":-1}]),
            json!([{"display_name":"Fable","utilization":101}]),
            json!([{"display_name":"Opus","utilization":10}]),
        ] {
            let snap = usage_snapshot(&json!({"rate_limits":{"model_scoped":scoped}}), NOW);
            assert_eq!(snap.metrics.len(), 2);
        }
        let old = usage_snapshot(
            &json!({"rate_limits":{"five_hour":{"utilization":50}}}),
            NOW,
        );
        assert_eq!(old.metrics.len(), 2);
        assert_eq!(old.metrics[0].used_percent, Some(50.0));
        assert_eq!(old.metrics[1].used_percent, None);
        for (used, exhausted) in [(0, false), (100, true)] {
            let snap = usage_snapshot(
                &json!({"rate_limits":{"model_scoped":[
                    {"display_name":"Fable","utilization":used,"resets_at":null}
                ]}}),
                NOW,
            );
            assert_eq!(snap.state, "ready");
            assert_eq!(snap.metrics.len(), 3);
            assert_eq!(snap.metrics[2].used_percent, Some(used as f64));
            assert_eq!(snap.metrics[2].reset_at, None);
            assert_eq!(snap.metrics[2].exhausted, exhausted);
        }
    }

    #[test]
    fn due_poll_consumes_manual_request_without_scheduling_a_duplicate() {
        let requested = std::sync::atomic::AtomicBool::new(true);
        assert!(refresh_due(
            Duration::from_secs(60),
            Duration::from_secs(60),
            &requested
        ));
        assert!(!refresh_due(
            Duration::from_secs(1),
            Duration::from_secs(60),
            &requested
        ));
        requested.store(true, Ordering::Release);
        assert!(refresh_due(
            Duration::ZERO,
            Duration::from_secs(60),
            &requested
        ));
    }
    #[cfg(unix)]
    #[test]
    fn fake_cli_end_to_end_isolates_failures_caches_session_and_late_results() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("qm-poll-integration-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let exe = root.join("fake-claude");
        fs::write(&exe, r#"#!/bin/sh
printf '%s\n' "$CLAUDE_CONFIG_DIR" > "$PWD/profile-seen"
read init
printf '%s\n' '{"type":"control_response","response":{"request_id":"init","subtype":"success","response":{}}}'
read usage
case "$CLAUDE_CONFIG_DIR" in
  *broken) printf '%s\n' '{"type":"control_response","response":{"request_id":"usage","subtype":"error"}}' ;;
  *work) used=80 ;;
  *) used=25 ;;
esac
if [ -n "$used" ]; then
printf '{"type":"control_response","response":{"request_id":"usage","subtype":"success","response":{"rate_limits_available":true,"subscription_type":"pro","rate_limits":{"five_hour":{"utilization":%s,"resets_at":"2030-01-01T00:00:00Z"}}}}}\n' "$used"
fi
cat >/dev/null
"#).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
        let store = std::sync::Mutex::new(crate::claude_accounts::RefreshStore::new());
        let before = std::env::var_os("CLAUDE_CONFIG_DIR");
        let mut profiles = vec![];
        let mut accounts = vec![];
        for name in ["personal", "broken", "work"] {
            let path = root.join(name);
            fs::create_dir_all(&path).unwrap();
            // Non-sensitive session sentinel: the production collector must not touch it.
            fs::write(path.join("session-sentinel"), "unchanged").unwrap();
            let account = crate::claude_accounts::Discovered {
                profile: crate::claude_accounts::Profile {
                    id: crate::claude_accounts::identity(&path),
                    path: path.to_str().unwrap().into(),
                    name: name.into(),
                    visible: true,
                },
                primary: false,
                launch: Some(path.clone()),
                available: true,
                visible: true,
            };
            assert!(poll_profile(&store, &account, Some(&exe), &root, || {}));
            let profile = account.profile.clone();
            accounts.push(account);
            assert_eq!(
                fs::read_to_string(profile_workspace(&root, &profile.id).join("profile-seen"))
                    .unwrap()
                    .trim(),
                profile.path
            );
            assert_eq!(
                fs::read_to_string(path.join("session-sentinel")).unwrap(),
                "unchanged"
            );
            profiles.push(profile);
        }
        assert_eq!(std::env::var_os("CLAUDE_CONFIG_DIR"), before);
        assert_eq!(
            store
                .lock()
                .unwrap()
                .snapshot(&profiles[0].id)
                .unwrap()
                .metrics[0]
                .used_percent,
            Some(25.0)
        );
        assert_eq!(
            store
                .lock()
                .unwrap()
                .snapshot(&profiles[1].id)
                .unwrap()
                .state,
            "error"
        );
        assert!(store
            .lock()
            .unwrap()
            .snapshot(&profiles[1].id)
            .unwrap()
            .metrics
            .is_empty());
        assert_eq!(
            store
                .lock()
                .unwrap()
                .snapshot(&profiles[2].id)
                .unwrap()
                .metrics[0]
                .used_percent,
            Some(80.0)
        );
        let held = store.lock().unwrap().begin(&profiles[0].id).unwrap();
        assert!(!poll_profile(
            &store,
            &accounts[0],
            Some(&exe),
            &root,
            || panic!("overlap")
        ));
        store.lock().unwrap().invalidate();
        assert!(!store
            .lock()
            .unwrap()
            .finish(held, compute(true, true, FileState::Missing, NOW)));
        let cached = store.lock().unwrap().snapshot(&profiles[0].id);
        assert!(!poll_profile(
            &store,
            &accounts[0],
            Some(&exe),
            &root,
            || store.lock().unwrap().invalidate()
        ));
        assert_eq!(store.lock().unwrap().snapshot(&profiles[0].id), cached);
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    const ENV_PROBE_CLI: &str = r#"#!/bin/sh
printf '%s\n' "${CLAUDE_CONFIG_DIR-<unset>}" > "$PWD/profile-seen"
read init
printf '%s\n' '{"type":"control_response","response":{"request_id":"init","subtype":"success","response":{}}}'
read usage
case "$CLAUDE_CONFIG_DIR" in
  *loggedout*) printf '%s\n' '{"type":"control_response","response":{"request_id":"usage","subtype":"success","response":{"rate_limits_available":false,"subscription_type":null}}}' ;;
  *) printf '%s\n' '{"type":"control_response","response":{"request_id":"usage","subtype":"success","response":{"rate_limits_available":true,"subscription_type":"max","rate_limits":{"five_hour":{"utilization":10,"resets_at":"2030-01-01T00:00:00Z"}}}}}' ;;
esac
cat >/dev/null
"#;
    #[cfg(unix)]
    fn env_probe_cli(root: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let exe = root.join("env-probe-claude");
        fs::write(&exe, ENV_PROBE_CLI).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
        exe
    }
    /// Child half of `primary_inherits_app_environment_exactly` (re-executed
    /// test binary with a controlled environment). No-op when run directly.
    #[cfg(unix)]
    #[test]
    #[ignore = "helper re-executed by primary_inherits_app_environment_exactly"]
    fn primary_env_probe_child() {
        let (Some(home), Some(expected)) = (
            std::env::var_os("QM_PROBE_HOME"),
            std::env::var("QM_PROBE_EXPECT").ok(),
        ) else {
            return;
        };
        let home = PathBuf::from(home);
        let before = std::env::var_os("CLAUDE_CONFIG_DIR");
        // Same inputs as profiles(): the app's own CLAUDE_CONFIG_DIR, if any.
        let config = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
        let found = crate::claude_accounts::discover(&home, config.as_deref(), &Default::default());
        let primary = found.iter().find(|a| a.primary).unwrap();
        let exe = env_probe_cli(&home);
        let store = std::sync::Mutex::new(crate::claude_accounts::RefreshStore::new());
        assert!(poll_profile(&store, primary, Some(&exe), &home, || {}));
        let seen =
            fs::read_to_string(profile_workspace(&home, &primary.profile.id).join("profile-seen"))
                .unwrap();
        assert_eq!(seen.trim_end_matches('\n'), expected);
        assert_eq!(std::env::var_os("CLAUDE_CONFIG_DIR"), before);
        let snap = store.lock().unwrap().snapshot(&primary.profile.id).unwrap();
        assert_eq!(snap.plan_type.as_deref(), Some("max"));
        println!("QM_PRIMARY_ENV_PROBE_OK");
    }
    #[cfg(unix)]
    #[test]
    fn primary_inherits_app_environment_exactly() {
        let root = std::env::temp_dir().join(format!("qm-primary-env-{}", std::process::id()));
        fs::create_dir_all(root.join(".claude")).unwrap();
        let home = root.canonicalize().unwrap();
        // Inherited value is passed through verbatim (double slash, trailing slash).
        let raw = format!("{}//.claude/", home.display());
        for (set, expected) in [(None, "<unset>".to_string()), (Some(raw.clone()), raw)] {
            let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
            cmd.args([
                "--exact",
                "claude::tests::primary_env_probe_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("QM_PROBE_HOME", &home)
            .env("QM_PROBE_EXPECT", &expected);
            match &set {
                Some(v) => cmd.env("CLAUDE_CONFIG_DIR", v),
                None => cmd.env_remove("CLAUDE_CONFIG_DIR"),
            };
            let out = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("QM_PRIMARY_ENV_PROBE_OK"),
                "primary probe with CLAUDE_CONFIG_DIR={set:?} failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn extra_profiles_get_exact_raw_path_and_primary_alias_is_deduplicated() {
        let root = std::env::temp_dir().join(format!("qm-extra-env-{}", std::process::id()));
        fs::create_dir_all(root.join(".claude")).unwrap();
        fs::create_dir_all(root.join(".claude-work")).unwrap();
        fs::create_dir_all(root.join(".claude-loggedout")).unwrap();
        fs::create_dir_all(root.join("manual")).unwrap();
        let home = root.canonicalize().unwrap();
        let exe = env_probe_cli(&home);
        let mut prefs = crate::claude_accounts::AccountPrefs::default();
        let manual_raw = format!("{}/manual/", home.display());
        let manual =
            crate::claude_accounts::manual_profile(&home, None, &prefs, &manual_raw).unwrap();
        assert_eq!(manual.path, manual_raw);
        prefs.profiles.push(manual);
        // A saved profile that designates ~/.claude is the primary (one entry, no env).
        let alias = home.join(".claude/");
        prefs.profiles.push(crate::claude_accounts::Profile {
            id: crate::claude_accounts::identity(&home.join(".claude")),
            path: alias.to_str().unwrap().into(),
            name: "Alias".into(),
            visible: true,
        });
        assert!(prefs.validate().is_ok());
        let found = crate::claude_accounts::discover(&home, None, &prefs);
        assert_eq!(found.len(), 4);
        // The settings UI writes profile.path back: it must stay the raw string.
        assert_eq!(found[3].profile.path, manual_raw);
        assert!(found[0].primary && found[0].launch.is_none());
        assert_eq!(found.iter().filter(|a| a.primary).count(), 1);
        assert_eq!(
            found[0].profile.id,
            crate::claude_accounts::identity(&home.join(".claude"))
        );
        let before = std::env::var_os("CLAUDE_CONFIG_DIR");
        let store = std::sync::Mutex::new(crate::claude_accounts::RefreshStore::new());
        let expected = [
            format!("{}/.claude-loggedout", home.display()),
            format!("{}/.claude-work", home.display()),
            manual_raw.clone(),
        ];
        for (account, expected) in found[1..].iter().zip(expected) {
            assert_eq!(account.launch.as_deref(), Some(Path::new(&expected)));
            assert!(poll_profile(&store, account, Some(&exe), &home, || {}));
            let seen = fs::read_to_string(
                profile_workspace(&home, &account.profile.id).join("profile-seen"),
            )
            .unwrap();
            assert_eq!(seen.trim_end_matches('\n'), expected);
        }
        assert_eq!(std::env::var_os("CLAUDE_CONFIG_DIR"), before);
        let out = store
            .lock()
            .unwrap()
            .snapshot(&found[1].profile.id)
            .unwrap();
        assert_eq!(
            (out.state, out.error_kind),
            ("error", Some("not_signed_in"))
        );
        assert!(out.metrics.is_empty());
        assert!(out.message.contains(&format!(
            "CLAUDE_CONFIG_DIR={}/.claude-loggedout claude",
            home.display()
        )));
        // The signed-out account does not break the others.
        let work = store
            .lock()
            .unwrap()
            .snapshot(&found[2].profile.id)
            .unwrap();
        assert_eq!(work.state, "ready");
        assert_eq!(work.metrics[0].used_percent, Some(10.0));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn signed_out_reply_is_not_connected_not_an_empty_quota() {
        let signed_out = json!({"rate_limits_available":false,"subscription_type":null});
        let previous = result_snapshot(
            Ok(
                json!({"rate_limits_available":true,"subscription_type":"max","rate_limits":{
                "five_hour":{"utilization":0,"resets_at":"2030-01-01T00:00:00Z"}}}),
            ),
            None,
            NOW,
        );
        let primary = result_snapshot_for(Ok(signed_out.clone()), Some(previous), NOW, None);
        assert_eq!(
            (primary.state, primary.error_kind),
            ("error", Some("not_signed_in"))
        );
        assert!(primary.metrics.is_empty() && primary.plan_type.is_none());
        assert!(primary.message.contains("claude auth login"));
        assert!(!primary.message.contains("CLAUDE_CONFIG_DIR"));
        let extra = result_snapshot_for(
            Ok(signed_out),
            None,
            NOW,
            Some(Path::new("/Users/me/.claude-work")),
        );
        assert_eq!(extra.error_kind, Some("not_signed_in"));
        assert!(extra
            .message
            .contains("CLAUDE_CONFIG_DIR=/Users/me/.claude-work claude"));
        let spaced = result_snapshot_for(
            Ok(json!({"rate_limits_available":false})),
            None,
            NOW,
            Some(Path::new("/Users/me/Claude Work")),
        );
        assert!(spaced
            .message
            .contains("CLAUDE_CONFIG_DIR='/Users/me/Claude Work' claude"));
        // A connected account without subscription limits keeps the old auth error.
        let api = result_snapshot_for(
            Ok(json!({"rate_limits_available":false,"subscription_type":"pro"})),
            None,
            NOW,
            None,
        );
        assert_eq!(api.error_kind, Some("auth"));
    }
    /// Read-only live check of the primary collector on this machine. Prints
    /// only subtype / subscription_type / rate_limits_available.
    #[test]
    #[ignore = "Explicit opt-in: real Claude Code get_usage for the primary profile"]
    fn live_primary_collector_is_signed_in() {
        let exe = crate::claude_cli::discover().expect("Claude Code installed");
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let config = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
        let found = crate::claude_accounts::discover(&home, config.as_deref(), &Default::default());
        let primary = found.iter().find(|a| a.primary).unwrap();
        assert!(primary.launch.is_none());
        let cwd = std::env::temp_dir().join("qm-live-primary-probe");
        let value = collect(primary, &exe, &cwd);
        let (subtype, value) = match value {
            Ok(v) => ("success", v),
            Err(kind) => (kind, Value::Null),
        };
        println!(
            "QM_LIVE subtype={subtype} subscription_type={} rate_limits_available={}",
            value["subscription_type"], value["rate_limits_available"]
        );
        assert_eq!(value["rate_limits_available"], true);
    }
    #[test]
    fn usage_keeps_fable_extra_and_errors_in_their_own_cache() {
        // v3.1.0 schema: Fable arrives in model_scoped, not seven_day_fable.
        let value = json!({"rate_limits_available":true,"subscription_type":"max_20x","rate_limits": {
            "five_hour":{"utilization":25,"resets_at":"2030-01-01T00:00:00Z"},
            "seven_day":{"utilization":100,"resets_at":"2030-01-02T00:00:00Z"},
            "model_scoped":[{"display_name":"Fable","utilization":12,"resets_at":"2030-01-02T00:00:00Z"}],
            "extra_usage":{"is_enabled":true,"spend_limit_reached":true}
        }});
        let a = result_snapshot(Ok(value), None, NOW);
        assert_eq!(a.metrics.len(), 3);
        assert_eq!(a.metrics[2].id, "claude:weekly_fable");
        assert_eq!(a.metrics[0].used_percent, Some(25.0));
        assert_eq!(a.plan_type.as_deref(), Some("max_20x"));
        assert!(a.extra_usage.as_ref().unwrap().limit_reached);
        let b = result_snapshot(Err("unsupported_or_auth"), None, NOW);
        assert_eq!(b.state, "error");
        assert!(b.metrics.is_empty());
        let cached_a = result_snapshot(Err("timeout"), Some(a.clone()), NOW);
        assert_eq!(cached_a.metrics, a.metrics);
        assert_eq!(cached_a.observed_at, a.observed_at);
        assert_eq!(cached_a.state, "error");
        assert_ne!(
            profile_workspace(Path::new("/scratch"), "a"),
            profile_workspace(Path::new("/scratch"), "b")
        );
    }
    #[test]
    fn fable_visibility_and_coalescing_apply_to_each_account_view_but_not_raw() {
        let reset = iso(NOW + 3600).unwrap();
        let raw = usage_snapshot(
            &json!({"rate_limits": {
                "five_hour":{"utilization":10,"resets_at":reset},
                "seven_day":{"utilization":37.5,"resets_at":reset},
                "model_scoped":[{"display_name":"Fable","utilization":37.5,"resets_at":reset}]
            }}),
            NOW,
        );
        // Display view of one account: v3.1.0 coalescing into "Limite Fable".
        let shown = account_view(raw.clone(), true, true, Some(true));
        assert_eq!(shown.metrics.len(), 2);
        assert_eq!(shown.metrics[1].label, "Limite Fable");
        // Fable hidden by the persisted preference: hidden for this account too.
        let hidden = account_view(raw.clone(), true, true, Some(false));
        assert!(hidden.metrics.iter().all(|m| m.id != "claude:weekly_fable"));
        assert_eq!(hidden.metrics.len(), 2);
        // Raw view (notifications): every metric, original labels.
        let untouched = account_view(raw.clone(), true, true, None);
        assert_eq!(untouched.metrics.len(), 3);
        assert_eq!(untouched.metrics[2].label, "Limite hebdomadaire Fable");
        // Unavailable profile directory: explicit error, no metrics.
        let missing = account_view(raw, true, false, Some(true));
        assert_eq!(missing.state, "error");
        assert!(missing.metrics.is_empty());
    }

    #[test]
    fn disabled_and_unconfigured_have_no_metrics() {
        let s = compute(false, true, file(Some((10.0, NOW + 60)), None, NOW), NOW);
        assert_eq!((s.state, s.metrics.len()), ("disabled", 0));
        let s = compute(true, false, FileState::Missing, NOW);
        assert_eq!((s.state, s.metrics.len()), ("not_configured", 0));
        assert_eq!(
            compute(true, true, FileState::Missing, NOW).state,
            "waiting_for_data"
        );
        let s = compute(true, true, FileState::Malformed, NOW);
        assert_eq!((s.state, s.error_kind), ("error", Some("malformed")));
    }

    #[test]
    fn both_windows_ready() {
        let s = compute(
            true,
            true,
            file(Some((37.0, NOW + 900)), Some((5.0, NOW + 9000)), NOW - 10),
            NOW,
        );
        assert_eq!(s.state, "ready");
        assert_eq!(s.metrics[0].used_percent, Some(37.0));
        assert_eq!(s.metrics[1].used_percent, Some(5.0));
        assert!(s
            .metrics
            .iter()
            .all(|m| !m.exhausted && m.availability == "available"));
        assert_eq!(s.provider_fetched_at, None);
    }

    #[test]
    fn partial_data_shows_the_absent_window_explicitly() {
        let s = compute(true, true, file(Some((0.0, NOW + 900)), None, NOW), NOW);
        assert_eq!(s.state, "ready");
        assert_eq!(s.metrics[0].used_percent, Some(0.0)); // zero is a valid value
        assert_eq!(s.metrics[1].availability, "not_reported");
        assert_eq!(s.metrics[1].used_percent, None);
    }

    #[test]
    fn exhausted_is_a_metric_attribute_not_an_error() {
        let s = compute(
            true,
            true,
            file(Some((100.0, NOW + 900)), Some((20.0, NOW + 9000)), NOW),
            NOW,
        );
        assert_eq!(s.state, "ready");
        assert!(s.metrics[0].exhausted && !s.metrics[1].exhausted);
        assert_eq!(s.error_kind, None);
    }

    #[test]
    fn windows_past_their_reset_are_not_shown_as_valid() {
        let s = compute(
            true,
            true,
            file(Some((90.0, NOW - 1)), Some((5.0, NOW + 9000)), NOW - 100),
            NOW,
        );
        assert_eq!(s.metrics[0].availability, "unavailable");
        assert_eq!(s.metrics[0].used_percent, None);
        let s = compute(
            true,
            true,
            file(Some((90.0, NOW - 1)), None, NOW - 100),
            NOW,
        );
        assert_eq!(s.state, "waiting_for_data");
    }

    #[test]
    fn old_observations_are_stale() {
        let s = compute(
            true,
            true,
            file(Some((10.0, NOW + 9000)), None, NOW - STALE_AFTER_SECS - 1),
            NOW,
        );
        assert_eq!(s.state, "stale");
        assert_eq!(s.metrics[0].used_percent, Some(10.0));
    }

    #[test]
    fn parse_keeps_only_documented_percentages() {
        let event = br#"{"session_id":"x","cwd":"/secret","rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1800003600},"seven_day":{"used_percentage":140,"resets_at":1}}}"#;
        let (five, seven) = parse_event(event).unwrap();
        assert_eq!(
            five,
            Some(Window {
                used_percent: 23.5,
                resets_at: Some(1_800_003_600)
            })
        );
        assert_eq!(seven, None); // out-of-range percentage rejected
        assert_eq!(parse_event(br#"{"model":"x"}"#), Some((None, None)));
        assert_eq!(parse_event(b"nope"), None);
        let serialised = serde_json::to_string(&merge(None, five, None, NOW)).unwrap();
        assert!(!serialised.contains("secret") && !serialised.contains("session"));
    }

    #[test]
    fn merge_keeps_unreset_windows_only() {
        let prev = RelayFile {
            schema_version: 1,
            received_at: NOW - 5,
            five_hour: Some(Window {
                used_percent: 10.0,
                resets_at: Some(NOW - 1),
            }),
            seven_day: Some(Window {
                used_percent: 4.0,
                resets_at: Some(NOW + 100),
            }),
        };
        let m = merge(Some(prev), None, None, NOW);
        assert_eq!(m.five_hour, None);
        assert_eq!(m.seven_day.unwrap().used_percent, 4.0);
    }

    #[test]
    fn legacy_relay_is_removed_silently_and_only_when_it_is_ours() {
        let dir = std::env::temp_dir().join(format!("qm-claude-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.json");
        let exe = Path::new("/Apps/Quota Monitor.app/Contents/MacOS/qm");
        // Absent file: nothing to do, nothing created.
        assert_eq!(migrate_legacy_relay(&settings), RelayMigration::NotNeeded);
        assert!(!settings.exists());
        // Our relay: removed, every other key kept, backup of the original kept, idempotent.
        fs::write(&settings, br#"{"theme":"dark","model":"opus"}"#).unwrap();
        install_relay(&settings, exe, &dir).unwrap();
        assert_eq!(migrate_legacy_relay(&settings), RelayMigration::Removed);
        let v: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert_eq!(v, json!({"theme":"dark","model":"opus"}));
        assert!(settings.with_extension("json.quota-monitor.bak").exists());
        assert_eq!(migrate_legacy_relay(&settings), RelayMigration::NotNeeded);
        // A statusLine of the user's own is never touched (byte-identical).
        let own = br#"{"statusLine":{"type":"command","command":"mine.sh"}}"#;
        fs::write(&settings, own).unwrap();
        assert_eq!(migrate_legacy_relay(&settings), RelayMigration::NotNeeded);
        assert_eq!(fs::read(&settings).unwrap(), own);
        // Unreadable settings are left alone.
        fs::write(&settings, b"{broken").unwrap();
        assert_eq!(migrate_legacy_relay(&settings), RelayMigration::Skipped);
        assert_eq!(fs::read(&settings).unwrap(), b"{broken");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_never_replaces_a_foreign_status_line_and_uninstalls_cleanly() {
        let dir = std::env::temp_dir().join(format!("qm-claude-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.json");
        let exe = Path::new("/Apps/Quota Monitor's/qm");
        // Foreign statusLine is refused and left byte-identical.
        let original = br#"{"theme":"dark","statusLine":{"type":"command","command":"mine.sh"}}"#;
        fs::write(&settings, original).unwrap();
        assert!(install_relay(&settings, exe, &dir).is_err());
        assert_eq!(fs::read(&settings).unwrap(), original);
        assert!(uninstall_relay(&settings).is_err());
        // Without statusLine: installed, other keys kept, backup made.
        fs::write(&settings, br#"{"theme":"dark"}"#).unwrap();
        install_relay(&settings, exe, &dir).unwrap();
        assert!(relay_installed(&settings));
        assert!(settings.with_extension("json.quota-monitor.bak").exists());
        let v: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert!(v["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains("Monitor'\\''s"));
        uninstall_relay(&settings).unwrap();
        assert!(!relay_installed(&settings));
        let v: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert_eq!(v, json!({"theme":"dark"}));
        // Unreadable settings are never overwritten.
        fs::write(&settings, b"{broken").unwrap();
        assert!(install_relay(&settings, exe, &dir).is_err());
        assert_eq!(fs::read(&settings).unwrap(), b"{broken");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn relay_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("qm-claude-perm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join(DATA_FILE);
        write_private(&path, b"{}").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_and_extra_usage_keep_only_safe_scalars() {
        use serde_json::json;
        assert_eq!(plan_token(&json!("Pro")), Some("pro".into()));
        assert_eq!(plan_token(&json!("max_20x")), Some("max_20x".into()));
        assert_eq!(plan_token(&json!("me@example.com")), None);
        assert_eq!(plan_token(&json!(null)), None);
        assert_eq!(
            extra_usage(&json!({"is_enabled": true, "spend_limit_reached": true})),
            Some(ExtraUsage {
                enabled: true,
                limit_reached: true
            })
        );
        assert_eq!(extra_usage(&json!(null)), None);
    }
}
