//! Claude provider. Source: the official Claude Code `statusLine` JSON, relayed
//! locally and opt-in. No credential is read, stored or sent; the relay keeps
//! only `rate_limits` percentages/reset times and a local receipt time.
//! Independent from Codex: own state, own file, own watcher, own generation.
use crate::prefs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
    *LIVE.lock().unwrap() = None;
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

pub fn relay_installed(settings_path: &Path) -> bool {
    matches!(read_settings(settings_path), Ok(Some(v)) if is_ours(&v))
}

/// Installs the relay only when Claude Code has no statusLine of its own.
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

static LIVE: std::sync::Mutex<Option<ClaudeSnapshot>> = std::sync::Mutex::new(None);
static REFRESH_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn snapshot(app: &tauri::AppHandle) -> ClaudeSnapshot {
    filter_fable(raw_snapshot(app), prefs::current(app).fable_enabled)
}

fn filter_fable(mut snap: ClaudeSnapshot, enabled: bool) -> ClaudeSnapshot {
    if !enabled {
        snap.metrics.retain(|m| m.id != "claude:weekly_fable");
    }
    snap
}

pub(crate) fn raw_snapshot(app: &tauri::AppHandle) -> ClaudeSnapshot {
    let enabled = prefs::current(app).claude_enabled;
    let mut snap = LIVE.lock().unwrap().clone().unwrap_or_else(|| {
        compute(
            enabled,
            crate::claude_cli::discover().is_some(),
            FileState::Missing,
            unix_now(),
        )
    });
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
    let Some(executable) = crate::claude_cli::discover() else {
        // Do not silently return: surface the missing executable even if a
        // previous successful refresh left cached data in LIVE.
        if generation == GENERATION.load(Ordering::Acquire) && prefs::current(app).claude_enabled {
            *LIVE.lock().unwrap() = Some(compute(true, false, FileState::Missing, unix_now()));
            crate::publish_quotas(app);
        }
        return;
    };
    let Ok(dir) = data_dir(app) else {
        return;
    };
    let result = crate::claude_cli::read_usage(&executable, &dir.join("claude-cli-workspace"));
    if generation != GENERATION.load(Ordering::Acquire) || !prefs::current(app).claude_enabled {
        return;
    }
    let now = unix_now();
    let mut snap = match result {
        Ok(value) if value["rate_limits_available"] == true => usage_snapshot(&value, now),
        Ok(_) => {
            let mut s = compute(true, true, FileState::Missing, now);
            s.state = "error";
            s.error_kind = Some("auth");
            s.message = "Limites d’abonnement indisponibles. Vérifiez claude auth login (compte claude.ai).".into();
            s
        }
        Err(kind) => {
            let mut s = LIVE
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| compute(true, true, FileState::Missing, now));
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
    *LIVE.lock().unwrap() = Some(snap);
    crate::publish_quotas(app);
}

// ------------------------------------------------------------------ commands

#[tauri::command]
pub fn get_claude_snapshot(app: tauri::AppHandle) -> ClaudeSnapshot {
    snapshot(&app)
}

/// Claude exposes quotas only while Claude Code runs: this re-reads the local
/// relay file; it never contacts a server and never touches Codex.
#[tauri::command]
pub fn request_claude_refresh(app: tauri::AppHandle) -> ClaudeSnapshot {
    REFRESH_REQUESTED.store(true, Ordering::Release);
    snapshot(&app)
}

#[tauri::command]
pub fn install_claude_relay(app: tauri::AppHandle) -> Result<ClaudeSnapshot, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    install_relay(&claude_settings_path(&app)?, &exe, &data_dir(&app)?)?;
    crate::publish_quotas(&app);
    Ok(snapshot(&app))
}

#[tauri::command]
pub fn uninstall_claude_relay(app: tauri::AppHandle) -> Result<ClaudeSnapshot, String> {
    uninstall_relay(&claude_settings_path(&app)?)?;
    crate::publish_quotas(&app);
    Ok(snapshot(&app))
}

/// Watches for new events only while Claude is enabled; idle otherwise.
pub fn start_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<ClaudeSnapshot> = None;
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
            if last_read.elapsed() >= Duration::from_secs(interval)
                || REFRESH_REQUESTED.swap(false, Ordering::AcqRel)
            {
                refresh_live(&app);
                last_read = std::time::Instant::now();
            }
            let generation = GENERATION.load(Ordering::Acquire);
            let next = snapshot(&app);
            if generation != GENERATION.load(Ordering::Acquire) {
                continue; // disabled/re-enabled meanwhile: drop the stale result
            }
            if last.as_ref() != Some(&next) {
                last = Some(next);
                crate::publish_quotas(&app);
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
