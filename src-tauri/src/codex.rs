//! Codex account client. Resets require an explicit action; never starts agent turns.
use crate::{set_codex_snapshots, QuotaPayload};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: String,
    pub message: String,
    pub executable: Option<String>,
    pub last_checked: Option<String>,
    pub auth_url: Option<String>,
    pub user_code: Option<String>,
    pub plan_type: Option<String>,
    pub active_instances: Option<usize>,
    pub refresh_seconds: u64,
    pub refresh_settings: RefreshSettings,
    pub available_resets: Option<u64>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            phase: "starting".into(),
            message: "Connexion à Codex…".into(),
            executable: None,
            last_checked: None,
            auth_url: None,
            user_code: None,
            plan_type: None,
            active_instances: None,
            refresh_seconds: 120,
            refresh_settings: RefreshSettings::default(),
            available_resets: None,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshSettings {
    pub custom_seconds: Option<u64>,
}
impl RefreshSettings {
    fn seconds(&self, plan: Option<&str>) -> u64 {
        self.custom_seconds.unwrap_or(match plan {
            Some("plus" | "pro" | "team" | "business" | "enterprise" | "edu") => 60,
            _ => 120,
        })
    }
    fn adaptive_seconds(&self, plan: Option<&str>, count: Option<usize>) -> u64 {
        if let Some(seconds) = self.custom_seconds {
            return seconds;
        }
        let base = self.seconds(plan);
        match count {
            Some(0) => 300,
            Some(1) | None => base,
            Some(2..=3) => (base / 2).max(30),
            Some(_) => (base / 4).max(15),
        }
    }
    fn valid(&self) -> bool {
        self.custom_seconds
            .is_none_or(|n| [30, 60, 120, 300, 600].contains(&n))
    }
}
#[tauri::command]
pub fn set_refresh_settings(
    app: tauri::AppHandle,
    provider: State<'_, Provider>,
    settings: RefreshSettings,
) -> Result<(), String> {
    if !settings.valid() {
        return Err("Intervalle non pris en charge.".into());
    }
    let path = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("refresh.json");
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(
        path,
        serde_json::to_vec(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    {
        let mut status = provider.shared.status.lock().unwrap();
        status.refresh_seconds =
            settings.adaptive_seconds(status.plan_type.as_deref(), status.active_instances);
        status.refresh_settings = settings;
    }
    provider
        .tx
        .send(Action::Reschedule)
        .map_err(|_| "Service Codex indisponible.".into())
}

#[derive(Default)]
struct Shared {
    status: Mutex<Status>,
    stop: AtomicBool,
    /// Provider disabled in Settings: no session, no polling, no publishing.
    paused: AtomicBool,
    child: Mutex<Option<Child>>,
    /// In-memory fingerprint of the account returned by `account/read`
    /// (never a token, never persisted or serialised): duplicate detection.
    identity: Mutex<Option<u64>>,
}
impl Shared {
    fn status(&self, phase: &str, message: &str) {
        if let Ok(mut status) = self.status.lock() {
            status.phase = phase.into();
            status.message = message.into();
            if phase != "ready" {
                status.available_resets = None;
            }
            if phase != "logging_in" {
                status.auth_url = None;
                status.user_code = None;
            }
        }
    }
    fn kill_child(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            // Give stdin EOF time to stop the app-server and its owned helpers.
            let deadline = Instant::now() + Duration::from_millis(750);
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
enum Action {
    Redeem(String, Sender<Result<RedemptionResult, String>>),
    Refresh,
    Reschedule,
    Login(bool),
    CancelLogin,
    Reconnect,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedemptionResult {
    outcome: String,
    quotas_refreshed: bool,
}

fn poll_due(now: Instant, next_poll: Instant, reset_pending: bool) -> bool {
    reset_pending || now >= next_poll
}

fn redemption_result(
    outcome: Result<String, String>,
    quotas_refreshed: bool,
) -> Result<RedemptionResult, String> {
    outcome.map(|outcome| RedemptionResult {
        outcome,
        quotas_refreshed,
    })
}
pub struct Provider {
    shared: Arc<Shared>,
    tx: Sender<Action>,
    /// Additional followed CODEX_HOME environments: one read-only child each.
    extras: Mutex<std::collections::BTreeMap<String, ExtraHandle>>,
    store: Arc<Mutex<AccountStore>>,
}
impl Provider {
    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().clone()
    }
    pub fn refresh(&self) {
        let _ = self.tx.send(Action::Refresh);
        for extra in self.extras.lock().unwrap().values() {
            let _ = extra.tx.send(ExtraAction::Refresh);
        }
    }
    #[cfg(target_os = "macos")]
    pub fn login(&self, device: bool) {
        let _ = self.tx.send(Action::Login(device));
    }
    /// Disabling stops polling and drops the session; late results are discarded.
    pub fn set_enabled(&self, enabled: bool) {
        self.shared.paused.store(!enabled, Ordering::Release);
        if enabled {
            self.shared.status("starting", "Connexion à Codex…");
        } else {
            self.shared
                .status("disabled", "Codex est désactivé dans les Réglages.");
        }
        let _ = self.tx.send(Action::Refresh);
    }
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.kill_child();
        for extra in self.extras.lock().unwrap().values() {
            extra.shared.stop.store(true, Ordering::Release);
            extra.shared.kill_child();
        }
    }

    /// Starts/stops the additional environments' workers so that exactly the
    /// followed, available homes are collected while Codex is enabled
    /// (visibility is display only). A removed or relaunched home gets a new
    /// generation: an answer still in flight from the old child is rejected.
    pub fn reconcile(&self, app: &tauri::AppHandle) {
        let prefs = crate::prefs::current(app);
        let wanted: Vec<_> = if prefs.codex_enabled {
            discovered(app, &prefs)
                .into_iter()
                .filter(|a| !a.primary && a.tracked && a.available)
                .take(crate::codex_accounts::MAX_EXTRA)
                .collect()
        } else {
            vec![]
        };
        let mut changed = false;
        {
            let mut extras = self.extras.lock().unwrap();
            let mut store = self.store.lock().unwrap();
            extras.retain(|id, handle| {
                let keep = wanted.iter().any(|a| {
                    &a.profile.id == id && a.launch.as_deref() == Some(handle.launch.as_path())
                });
                if !keep {
                    // Worker exits within 250 ms; dropping its session stops the child.
                    handle.shared.stop.store(true, Ordering::Release);
                    store.remove(id);
                    changed = true;
                }
                keep
            });
            for (index, a) in wanted.iter().enumerate() {
                if extras.contains_key(&a.profile.id) {
                    continue;
                }
                changed = true;
                let launch = a.launch.clone().expect("followed extra has a launch home");
                let generation = store.register(&a.profile.id);
                let shared = Arc::new(Shared::default());
                let (tx, rx) = mpsc::channel();
                let worker = ExtraWorker {
                    id: a.profile.id.clone(),
                    generation,
                    launch: launch.clone(),
                    shared: shared.clone(),
                    store: self.store.clone(),
                    executable: {
                        let app = app.clone();
                        Box::new(move || executable(&app))
                    },
                    interval: {
                        // Same cadence settings as the primary (activity is not
                        // attributable to one account, so no adaptive speed-up).
                        let primary = self.shared.clone();
                        Box::new(move |plan| {
                            primary
                                .status
                                .lock()
                                .map(|s| s.refresh_settings.seconds(plan))
                                .unwrap_or(120)
                        })
                    },
                    changed: {
                        let app = app.clone();
                        Box::new(move || crate::publish_quotas(&app))
                    },
                    // Staggered starts: never N app-servers at the same instant.
                    first_poll: Instant::now() + Duration::from_secs(2 * index as u64 + 1),
                };
                std::thread::spawn(move || run_extra(worker, rx));
                extras.insert(
                    a.profile.id.clone(),
                    ExtraHandle {
                        shared,
                        tx,
                        launch,
                    },
                );
            }
        }
        if changed {
            crate::publish_quotas(app);
        }
    }
}

fn executable(app: &tauri::AppHandle) -> Option<PathBuf> {
    let custom = saved_path(app)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .or_else(|| std::env::var("QUOTA_CODEX_BINARY").ok());
    discover(custom)
}

fn saved_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|p| p.join("codex-path.txt"))
        .map_err(|e| e.to_string())
}
fn discover(override_path: Option<String>) -> Option<PathBuf> {
    if let Some(path) = override_path.filter(|s| !s.trim().is_empty()) {
        let path = PathBuf::from(path.trim());
        return (path.is_absolute() && path.is_file()).then_some(path);
    }
    let mut paths = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(base).join("Programs/OpenAI/Codex/bin/codex.exe"));
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            paths.push(PathBuf::from(home).join(".local/bin/codex.exe"));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            paths.push(home.join(".local/bin/codex"));
            paths.push(home.join(".volta/bin/codex"));
            paths.push(home.join(".asdf/shims/codex"));
            if let Ok(versions) = fs::read_dir(home.join(".nvm/versions/node")) {
                for version in versions.flatten() {
                    paths.push(version.path().join("bin/codex"));
                }
            }
        }
        paths.extend(
            [
                "/opt/homebrew/bin/codex",
                "/usr/local/bin/codex",
                "/usr/bin/codex",
            ]
            .map(PathBuf::from),
        );
        #[cfg(target_os = "macos")]
        paths.extend(
            [
                "/Applications/Codex.app/Contents/Resources/codex",
                "/Applications/ChatGPT.app/Contents/Resources/codex",
            ]
            .map(PathBuf::from),
        );
    }
    if let Some(search) = std::env::var_os("PATH") {
        let name = if cfg!(target_os = "windows") {
            "codex.exe"
        } else {
            "codex"
        };
        paths.extend(
            std::env::split_paths(&search)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name)),
        );
    }
    paths.into_iter().find(|p| p.is_file())
}

fn read_protocol_line(reader: &mut impl BufRead) -> std::io::Result<Vec<u8>> {
    let mut line = Vec::new();
    std::io::Read::take(reader, 1_048_577).read_until(b'\n', &mut line)?;
    if line.len() > 1_048_576 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Codex message too large",
        ));
    }
    Ok(line)
}

struct Session {
    shared: Arc<Shared>,
    stdin: Option<ChildStdin>,
    messages: Receiver<Value>,
    next_id: u64,
    notices: Vec<Value>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stdin.take();
        self.shared.kill_child();
    }
}
impl Session {
    fn start(path: &PathBuf, shared: Arc<Shared>) -> Result<Self, String> {
        // Primary: the app environment is inherited untouched (no CODEX_HOME).
        Self::start_in(path, shared, None)
    }
    /// `codex_home`: set on this child only, raw followed value (the CLI
    /// validates and canonicalises it, without fallback to the primary).
    fn start_in(
        path: &PathBuf,
        shared: Arc<Shared>,
        codex_home: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let mut command = Command::new(path);
        if let Some(home) = codex_home {
            command.env("CODEX_HOME", home);
        }
        if let Some(bin) = path.parent().filter(|dir| dir.join("node").is_file()) {
            let mut search_path = bin.to_string_lossy().into_owned();
            if let Some(existing) = std::env::var_os("PATH") {
                search_path.push(':');
                search_path.push_str(&existing.to_string_lossy());
            }
            command.env("PATH", search_path);
        }
        command
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // A quota client needs neither a project directory nor project-local config.
        command.current_dir(std::env::temp_dir());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|_| {
            "Impossible de lancer Codex. Vérifie le chemin et les permissions.".to_string()
        })?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        *shared.child.lock().unwrap() = Some(child);
        let (tx, messages) = mpsc::sync_channel(128);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let Ok(line) = read_protocol_line(&mut reader) else {
                    break;
                };
                if line.is_empty() {
                    break;
                }
                if let Ok(value) = serde_json::from_slice(&line) {
                    if tx.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        let mut session = Self {
            shared,
            stdin: Some(stdin),
            messages,
            next_id: 0,
            notices: Vec::new(),
        };
        session.call("initialize", json!({"clientInfo":{"name":"quota_codex_monitor","version":env!("CARGO_PKG_VERSION")}}))?;
        session.send(json!({"method":"initialized","params":{}}))?;
        Ok(session)
    }
    fn send(&mut self, message: Value) -> Result<(), String> {
        writeln!(
            self.stdin.as_mut().ok_or("Connexion Codex fermée.")?,
            "{message}"
        )
        .map_err(|_| "La connexion à Codex a été fermée.".into())
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.shared.stop.load(Ordering::Acquire) {
                return Err("Arrêt de Codex.".into());
            }
            if Instant::now() >= deadline {
                return Err("Codex ne répond pas. Vérifie ta connexion puis réessaie.".into());
            }
            match self.messages.recv_timeout(Duration::from_millis(100)) {
                Ok(message) if message.get("id").and_then(Value::as_u64) == Some(id) => {
                    if message.get("error").is_some() {
                        return Err("Codex n’a pas pu traiter la demande. Vérifie ta connexion, ton compte ChatGPT et la version de Codex.".into());
                    }
                    return message
                        .get("result")
                        .cloned()
                        .ok_or("Réponse Codex invalide.".into());
                }
                Ok(message) => {
                    if self.notices.len() >= 128 {
                        return Err("Trop de notifications Codex.".into());
                    }
                    self.notices.push(message);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("Codex s’est arrêté. Une reconnexion sera tentée.".into()),
            }
        }
    }
    fn drain(&mut self) -> Vec<Value> {
        self.notices.extend(self.messages.try_iter().take(128));
        std::mem::take(&mut self.notices)
    }
}

pub fn normalize(result: &Value) -> Result<Vec<QuotaPayload>, String> {
    let buckets: Vec<(String, &Value)> =
        if let Some(map) = result.get("rateLimitsByLimitId").and_then(Value::as_object) {
            map.iter().map(|(id, v)| (id.clone(), v)).collect()
        } else if let Some(bucket) = result.get("rateLimits").filter(|v| v.is_object()) {
            vec![(
                bucket
                    .get("limitId")
                    .and_then(Value::as_str)
                    .unwrap_or("codex")
                    .into(),
                bucket,
            )]
        } else {
            return Err("Codex n’a renvoyé aucune information de quota exploitable.".into());
        };
    let now = chrono::Utc::now().to_rfc3339();
    let mut snapshots = Vec::new();
    for (id, bucket) in buckets {
        let name = bucket
            .get("limitName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&id);
        for slot in ["primary", "secondary"] {
            let Some(window) = bucket.get(slot).filter(|v| !v.is_null()) else {
                continue;
            };
            let used = window
                .get("usedPercent")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .ok_or("Pourcentage Codex invalide.")?;
            let mins = window
                .get("windowDurationMins")
                .and_then(Value::as_i64)
                .filter(|v| *v > 0);
            let duration = match mins {
                Some(300) => "5 heures".into(),
                Some(10080) => "7 jours".into(),
                Some(value) if value % 1440 == 0 => format!("{} jours", value / 1440),
                Some(value) if value % 60 == 0 => format!("{} heures", value / 60),
                Some(value) => format!("{value} min"),
                None => slot.into(),
            };
            let is_reserve = id.to_ascii_lowercase().contains("reserve")
                || id.to_ascii_lowercase().contains("luna")
                || id.to_ascii_lowercase().contains("base_model_inference");
            let display_label = if is_reserve {
                "Réserve".into()
            } else if mins == Some(300) {
                "Codex · 5 heures".into()
            } else if mins == Some(10080) {
                "Codex · 7 jours".into()
            } else {
                format!("{name} · {duration}")
            };
            let reset = window
                .get("resetsAt")
                .and_then(Value::as_i64)
                .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();
            snapshots.push(QuotaPayload {
                used_percent: Some(used),
                model: name.into(),
                remaining: (100.0 - used.clamp(0.0, 100.0)).round() as u32,
                limit: 100,
                reset_at: reset,
                checked_at: now.clone(),
                confidence: "high".into(),
                period: format!("codex:{id}:{slot}"),
                reset_label: None,
                source: "codex".into(),
                label: Some(display_label),
                window_minutes: mins,
            });
        }
    }
    Ok(snapshots)
}

pub fn start(app: tauri::AppHandle) -> Provider {
    let shared = Arc::new(Shared::default());
    if let Ok(dir) = app.path().app_local_data_dir() {
        if let Some(settings) = fs::read(dir.join("refresh.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RefreshSettings>(&bytes).ok())
            .filter(RefreshSettings::valid)
        {
            let mut status = shared.status.lock().unwrap();
            status.refresh_seconds = settings.seconds(None);
            status.refresh_settings = settings;
        }
    }
    if !crate::prefs::current(&app).codex_enabled {
        shared.paused.store(true, Ordering::Release);
        shared.status("disabled", "Codex est désactivé dans les Réglages.");
    }
    let (tx, rx) = mpsc::channel();
    let worker_shared = shared.clone();
    std::thread::spawn(move || worker(app, worker_shared, rx));
    Provider {
        shared,
        tx,
        extras: Mutex::new(Default::default()),
        store: Arc::new(Mutex::new(AccountStore::default())),
    }
}
fn worker(app: tauri::AppHandle, shared: Arc<Shared>, rx: Receiver<Action>) {
    let mut detector = crate::activity::Detector::new();
    let mut next_activity = Instant::now();
    let mut session: Option<Session> = None;
    let mut next_poll = Instant::now();
    let mut failures = 0_u32;
    let mut login: Option<(String, Instant)> = None;
    while !shared.stop.load(Ordering::Acquire) {
        let mut action = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(action) => Some(action),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(_) => break,
        };
        let mut pending_reset = None;
        if matches!(action, Some(Action::Redeem(..))) {
            let Some(Action::Redeem(key, reply)) = action.take() else {
                unreachable!()
            };
            let ready = shared.status.lock().unwrap().phase == "ready";
            if !ready || login.is_some() || session.is_none() {
                let _ = reply.send(Err("Reconnecte Codex avant de réessayer.".into()));
                continue;
            }
            let result = session
                .as_mut()
                .unwrap()
                .call(
                    "account/rateLimitResetCredit/consume",
                    json!({"idempotencyKey":key}),
                )
                .and_then(|value| reset_outcome(&value));
            // Read and publish in this iteration, before acknowledging the reset.
            // Activity changes must not defer this read to the adaptive interval.
            pending_reset = Some((reply, result));
        }
        if shared.paused.load(Ordering::Acquire) {
            if session.is_some() || login.is_some() {
                session = None; // Drop stops the app-server child.
                login = None;
                set_codex_snapshots(&app, None);
            }
            next_poll = Instant::now();
            continue;
        }
        if Instant::now() >= next_activity {
            let count = detector.count();
            let mut status = shared.status.lock().unwrap();
            let old = status.refresh_seconds;
            status.active_instances = count;
            status.refresh_seconds = status
                .refresh_settings
                .adaptive_seconds(status.plan_type.as_deref(), count);
            if status.refresh_seconds < old {
                next_poll =
                    next_poll.min(Instant::now() + Duration::from_secs(status.refresh_seconds));
            } else if status.refresh_seconds > old && status.phase == "ready" {
                next_poll = Instant::now() + Duration::from_secs(status.refresh_seconds);
            }
            next_activity = Instant::now() + Duration::from_secs(10);
        }
        if matches!(action, Some(Action::Reconnect)) {
            shared.status("starting", "Reconnexion à Codex…");
            set_codex_snapshots(&app, None);
            session = None;
            login = None;
            next_poll = Instant::now();
        }
        if matches!(action, Some(Action::Reschedule)) {
            next_poll =
                Instant::now() + Duration::from_secs(shared.status.lock().unwrap().refresh_seconds);
        }
        if matches!(action, Some(Action::Refresh)) {
            next_poll = Instant::now();
        }
        if session.is_none()
            && (Instant::now() >= next_poll || matches!(action, Some(Action::Login(_))))
        {
            let custom = saved_path(&app)
                .ok()
                .and_then(|p| fs::read_to_string(p).ok())
                .or_else(|| std::env::var("QUOTA_CODEX_BINARY").ok());
            if let Some(path) = discover(custom) {
                shared.status.lock().unwrap().executable =
                    Some(path.to_string_lossy().into_owned());
                match Session::start(&path, shared.clone()) {
                    Ok(value) => {
                        session = Some(value);
                    }
                    Err(error) => {
                        shared.status("error", &error);
                        set_codex_snapshots(&app, None);
                        next_poll = Instant::now() + Duration::from_secs(30);
                        continue;
                    }
                }
            } else {
                shared.status.lock().unwrap().executable = None;
                shared.status("missing", "Installe Codex CLI ou indique son exécutable pour activer la synchronisation automatique.");
                set_codex_snapshots(&app, None);
                next_poll = Instant::now() + Duration::from_secs(15);
                continue;
            }
        }
        let Some(client) = session.as_mut() else {
            continue;
        };
        if let Some(Action::Login(device)) = action {
            if login.is_none() {
                shared.status("logging_in", "Préparation de la connexion ChatGPT…");
                match client.call(
                    "account/login/start",
                    json!({"type":if device {"chatgptDeviceCode"} else {"chatgpt"}}),
                ) {
                    Ok(result) => {
                        if let (Some(id), Some(url)) = (
                            result["loginId"].as_str(),
                            result[if device { "verificationUrl" } else { "authUrl" }].as_str(),
                        ) {
                            if !trusted_login_url(url.as_ref()) {
                                shared.status("error", "Adresse de connexion Codex inattendue.");
                                session = None;
                                continue;
                            }
                            login = Some((id.into(), Instant::now()));
                            {
                                let mut status = shared.status.lock().unwrap();
                                status.auth_url = Some(url.into());
                                status.user_code = result["userCode"].as_str().map(str::to_owned);
                                status.message = "Termine la connexion dans le navigateur. Il pourra ensuite être fermé.".into();
                            }
                            // Only an explicit sign-in action opens a browser.
                            let _ = app.opener().open_url(url, None::<&str>);
                        } else {
                            shared.status(
                                "error",
                                "Réponse de connexion invalide. Mets Codex à jour.",
                            );
                            session = None;
                        }
                    }
                    Err(error) => {
                        shared.status("error", &error);
                        session = None;
                    }
                }
            }
            continue;
        }
        if matches!(action, Some(Action::CancelLogin))
            || login
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(600))
        {
            if let Some((id, _)) = login.take() {
                let _ = client.call("account/login/cancel", json!({"loginId":id}));
            }
            shared.status(
                "signed_out",
                "Connexion annulée. Connecte ton compte ChatGPT pour lire les quotas.",
            );
            next_poll = Instant::now();
        }
        for notice in client.drain() {
            match notice["method"].as_str() {
                Some("account/login/completed") => {
                    login = None;
                    if notice["params"]["success"].as_bool() == Some(true) {
                        next_poll = Instant::now();
                    } else {
                        shared.status("signed_out", "La connexion n’a pas abouti. Réessaie.");
                        next_poll = Instant::now() + Duration::from_secs(60);
                    }
                }
                Some("account/updated" | "account/rateLimits/updated") if login.is_none() => {
                    next_poll = Instant::now();
                }
                _ => {}
            }
        }
        if login.is_some() || !poll_due(Instant::now(), next_poll, pending_reset.is_some()) {
            continue;
        }
        let result = client
            .call("account/read", json!({"refreshToken":false}))
            .and_then(|account| {
                *shared.identity.lock().unwrap() = account_fingerprint(&account);
                let mut status = shared.status.lock().unwrap();
                status.plan_type = account["account"]["planType"]
                    .as_str()
                    .map(|s| s.to_ascii_lowercase());
                status.refresh_seconds = status
                    .refresh_settings
                    .adaptive_seconds(status.plan_type.as_deref(), status.active_instances);
                drop(status);
                match account["account"]["type"].as_str() {
                    Some("chatgpt" | "chatgptAuthTokens") => client
                        .call("account/rateLimits/read", Value::Null)
                        .and_then(|v| {
                            let rows = normalize(&v)?;
                            shared.status.lock().unwrap().available_resets = available_resets(&v);
                            Ok(Some(rows))
                        }),
                    _ => Ok(None),
                }
            });
        if shared.paused.load(Ordering::Acquire) {
            // Disabled while the read was in flight: never publish or re-enable.
            session = None;
            continue;
        }
        let quotas_refreshed = matches!(&result, Ok(Some(_)));
        match result {
            Ok(Some(snapshots)) => {
                let empty = snapshots.is_empty();
                shared.status(
                    "ready",
                    if empty {
                        "Connecté à Codex. Aucune fenêtre de quota n’est disponible pour ce compte."
                    } else {
                        "Quotas synchronisés directement avec Codex."
                    },
                );
                shared.status.lock().unwrap().last_checked = Some(chrono::Utc::now().to_rfc3339());
                set_codex_snapshots(&app, Some(snapshots));
                failures = 0;
                next_poll = Instant::now()
                    + Duration::from_secs(shared.status.lock().unwrap().refresh_seconds);
            }
            Ok(None) => {
                shared.status("signed_out", "Connecte un compte ChatGPT à Codex. Une clé API ne fournit pas les quotas de ton abonnement.");
                set_codex_snapshots(&app, None);
                next_poll = Instant::now() + Duration::from_secs(60);
                // A new session picks up an external CLI login on the next attempt.
                session = None;
            }
            Err(error) => {
                shared.status("error", &error);
                set_codex_snapshots(&app, None);
                failures = (failures + 1).min(4);
                session = None;
                next_poll = Instant::now()
                    + Duration::from_secs(
                        (15 * (1 << failures)).max(shared.status.lock().unwrap().refresh_seconds),
                    );
            }
        }
        if let Some((reply, outcome)) = pending_reset {
            // A confirmed redemption remains confirmed even if the following read
            // fails. The UI must not invite spending a second reset in that case.
            let _ = reply.send(redemption_result(outcome, quotas_refreshed));
        }
    }
}

#[tauri::command]
pub fn get_codex_status(provider: State<'_, Provider>) -> Status {
    provider.status()
}
fn available_resets(value: &Value) -> Option<u64> {
    value["rateLimitResetCredits"]["availableCount"].as_u64()
}
fn reset_outcome(value: &Value) -> Result<String, String> {
    match value["outcome"].as_str() {
        Some(outcome @ ("reset" | "alreadyRedeemed" | "nothingToReset" | "noCredit")) => {
            Ok(outcome.into())
        }
        _ => Err("Réponse inconnue. Vérifie les quotas avant de réessayer.".into()),
    }
}
#[derive(Debug, PartialEq)]
pub(crate) enum RedeemRoute {
    Primary,
    Extra(String),
}

/// Which collector may spend a reset for `id`. Only a followed, available,
/// connected, non-duplicate account qualifies; an unknown id is an error and
/// never falls back to the primary session.
pub(crate) fn redeem_route(accounts: &[CodexAccount], id: &str) -> Result<RedeemRoute, String> {
    let account = accounts
        .iter()
        .find(|a| a.profile.id == id)
        .ok_or("Compte Codex introuvable.")?;
    if !account.tracked || !account.available || account.phase != "ready" {
        return Err("Ce compte Codex n’est pas connecté.".into());
    }
    if account.duplicate_of.is_some() {
        return Err(
            "Ce compte est un doublon : utilise la réinitialisation du compte d’origine.".into(),
        );
    }
    Ok(if account.primary {
        RedeemRoute::Primary
    } else {
        RedeemRoute::Extra(id.to_string())
    })
}

#[tauri::command]
pub async fn redeem_codex_reset(
    app: tauri::AppHandle,
    provider: State<'_, Provider>,
    idempotency_key: String,
    account_id: String,
) -> Result<RedemptionResult, String> {
    // Only UUID-shaped keys are accepted. Retries must retain the original key.
    if idempotency_key.len() != 36
        || !idempotency_key.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err("Identifiant de réinitialisation invalide.".into());
    }
    let (tx, rx) = mpsc::channel();
    match redeem_route(&accounts(&app), &account_id)? {
        RedeemRoute::Primary => provider
            .tx
            .send(Action::Redeem(idempotency_key, tx))
            .map_err(|_| "Service Codex indisponible.")?,
        RedeemRoute::Extra(id) => provider
            .extras
            .lock()
            .unwrap()
            .get(&id)
            .ok_or("Compte Codex introuvable.")?
            .tx
            .send(ExtraAction::Redeem(idempotency_key, tx))
            .map_err(|_| "Service Codex indisponible.")?,
    }
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv()
            .map_err(|_| "Service Codex interrompu.".to_owned())?
    })
    .await
    .map_err(|_| "Réinitialisation interrompue.".to_owned())?
}
#[tauri::command]
pub fn start_codex_login(provider: State<'_, Provider>, device: bool) -> Result<(), String> {
    provider
        .tx
        .send(Action::Login(device))
        .map_err(|_| "Service Codex indisponible.".into())
}
#[tauri::command]
pub fn cancel_codex_login(provider: State<'_, Provider>) {
    let _ = provider.tx.send(Action::CancelLogin);
}
#[tauri::command]
pub fn set_codex_path(
    app: tauri::AppHandle,
    provider: State<'_, Provider>,
    path: String,
) -> Result<(), String> {
    let file = saved_path(&app)?;
    fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    if path.trim().is_empty() {
        if file.exists() {
            fs::remove_file(&file).map_err(|e| e.to_string())?;
        }
    } else {
        let p = PathBuf::from(path.trim());
        if !p.is_absolute() || !p.is_file() {
            return Err("Choisis le chemin absolu d’un exécutable Codex existant.".into());
        }
        #[cfg(target_os = "windows")]
        if p.extension().and_then(|v| v.to_str()) != Some("exe") {
            return Err("Sélectionne codex.exe, pas un script .cmd.".into());
        }
        fs::write(file, path.trim()).map_err(|e| e.to_string())?;
    }
    provider
        .tx
        .send(Action::Reconnect)
        .map_err(|_| "Service Codex indisponible.".into())
}
#[tauri::command]
pub fn open_codex_install(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url("https://developers.openai.com/codex/cli", None::<&str>)
        .map_err(|e| e.to_string())
}
fn trusted_login_url(value: &str) -> bool {
    tauri::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("auth.openai.com")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
    })
}

#[tauri::command]
pub fn open_codex_login(
    app: tauri::AppHandle,
    provider: State<'_, Provider>,
) -> Result<(), String> {
    let url = provider
        .status()
        .auth_url
        .ok_or("Aucune connexion en cours.")?;
    if !trusted_login_url(url.as_ref()) {
        return Err("Adresse invalide.".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

// ------------------------------------------------- additional CODEX_HOME homes

/// In-memory fingerprint of the ChatGPT account returned by `account/read`
/// (email, case-insensitive; else the routed account id). Only used to tell
/// that two homes are the same account. Never a token, never persisted,
/// serialised, logged or displayed.
fn account_fingerprint(account: &Value) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let key = account["account"]["email"]
        .as_str()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .map(|e| format!("email:{e}"))
        .or_else(|| {
            account["workspaceRouting"]["chatgptAccountId"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(|id| format!("account:{id}"))
        })?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    Some(hasher.finish())
}

/// Last published reading of one additional environment.
#[derive(Clone, Default)]
pub(crate) struct Reading {
    pub phase: String,
    pub message: String,
    pub plan_type: Option<String>,
    /// `None`: no quota known (signed out, error, starting). Never zero-filled.
    pub rows: Option<Vec<QuotaPayload>>,
    pub identity: Option<u64>,
    pub last_checked: Option<String>,
    /// Earned reset credits reported by this account's own rate-limit read.
    pub available_resets: Option<u64>,
}
impl Reading {
    fn new(phase: &str, message: &str) -> Self {
        Self {
            phase: phase.into(),
            message: message.into(),
            ..Self::default()
        }
    }
}
struct Slot {
    generation: u64,
    reading: Reading,
    refreshing: bool,
}
/// Per-account cache with a generation per registration: a worker only
/// publishes into the slot it was started for (never a stale/removed one).
#[derive(Default)]
pub(crate) struct AccountStore {
    next: u64,
    slots: std::collections::BTreeMap<String, Slot>,
}
impl AccountStore {
    pub fn register(&mut self, id: &str) -> u64 {
        self.next = self.next.wrapping_add(1);
        self.slots.insert(
            id.to_string(),
            Slot {
                generation: self.next,
                reading: Reading::new("starting", "Connexion à Codex…"),
                refreshing: false,
            },
        );
        self.next
    }
    pub fn remove(&mut self, id: &str) {
        self.slots.remove(id);
    }
    pub fn begin(&mut self, id: &str, generation: u64) -> bool {
        match self.slots.get_mut(id) {
            Some(slot) if slot.generation == generation => {
                slot.refreshing = true;
                true
            }
            _ => false,
        }
    }
    pub fn commit(&mut self, id: &str, generation: u64, reading: Reading) -> bool {
        match self.slots.get_mut(id) {
            Some(slot) if slot.generation == generation => {
                slot.refreshing = false;
                slot.reading = reading;
                true
            }
            _ => false,
        }
    }
    pub fn get(&self, id: &str) -> Option<(Reading, bool)> {
        self.slots
            .get(id)
            .map(|s| (s.reading.clone(), s.refreshing))
    }
}

struct ExtraHandle {
    shared: Arc<Shared>,
    tx: Sender<ExtraAction>,
    launch: PathBuf,
}
enum ExtraAction {
    Refresh,
    /// Explicit, user-confirmed reset for THIS home only (idempotency key).
    Redeem(String, Sender<Result<RedemptionResult, String>>),
}
pub(crate) struct ExtraWorker {
    id: String,
    generation: u64,
    /// Raw followed value, set as CODEX_HOME on this child only.
    launch: PathBuf,
    shared: Arc<Shared>,
    store: Arc<Mutex<AccountStore>>,
    executable: Box<dyn Fn() -> Option<PathBuf> + Send>,
    interval: Box<dyn Fn(Option<&str>) -> u64 + Send>,
    changed: Box<dyn Fn() + Send>,
    first_poll: Instant,
}

fn signed_out_message(launch: &std::path::Path) -> String {
    format!(
        "Non connecté. Connectez ce dossier dans un terminal : CODEX_HOME={} codex login",
        crate::claude::shell_word(&launch.to_string_lossy())
    )
}

/// Read-only polling of one additional environment: `account/read`
/// (`refreshToken:false`), then `account/rateLimits/read` for a ChatGPT
/// account only. Never logs in/out, never redeems a reset, never starts a
/// turn. A signed-out home (`account: null`) is reported as such.
fn poll_extra(
    w: &ExtraWorker,
    session: &mut Option<Session>,
    failures: &mut u32,
) -> (Reading, Duration) {
    let now = || Some(chrono::Utc::now().to_rfc3339());
    let backoff = |failures: u32, base: u64| Duration::from_secs((15 * (1 << failures)).max(base));
    if session.is_none() {
        let Some(exe) = (w.executable)() else {
            return (
                Reading::new(
                    "missing",
                    "Installe Codex CLI ou indique son exécutable pour activer la synchronisation automatique.",
                ),
                Duration::from_secs(60),
            );
        };
        match Session::start_in(&exe, w.shared.clone(), Some(&w.launch)) {
            Ok(value) => *session = Some(value),
            Err(error) => {
                *failures = (*failures + 1).min(4);
                let mut reading = Reading::new("error", &error);
                reading.last_checked = now();
                return (reading, backoff(*failures, (w.interval)(None)));
            }
        }
    }
    let client = session.as_mut().unwrap();
    let result = client
        .call("account/read", json!({"refreshToken":false}))
        .and_then(|account| {
            let plan = account["account"]["planType"]
                .as_str()
                .map(|s| s.to_ascii_lowercase());
            match account["account"]["type"].as_str() {
                Some("chatgpt" | "chatgptAuthTokens") => client
                    .call("account/rateLimits/read", Value::Null)
                    .and_then(|v| {
                        Ok(Some((
                            plan,
                            account_fingerprint(&account),
                            normalize(&v)?,
                            available_resets(&v),
                        )))
                    }),
                _ => Ok(None),
            }
        });
    match result {
        Ok(Some((plan, identity, rows, resets))) => {
            *failures = 0;
            let delay = Duration::from_secs((w.interval)(plan.as_deref()));
            let mut reading = Reading::new(
                "ready",
                if rows.is_empty() {
                    "Connecté à Codex. Aucune fenêtre de quota n’est disponible pour ce compte."
                } else {
                    "Quotas synchronisés directement avec Codex."
                },
            );
            reading.plan_type = plan;
            reading.identity = identity;
            reading.available_resets = resets;
            reading.rows = Some(rows);
            reading.last_checked = now();
            (reading, delay)
        }
        Ok(None) => {
            // A new session later picks up a login made outside the app.
            *session = None;
            let mut reading = Reading::new("signed_out", &signed_out_message(&w.launch));
            reading.last_checked = now();
            (
                reading,
                Duration::from_secs((w.interval)(None).max(120)),
            )
        }
        Err(error) => {
            *session = None;
            *failures = (*failures + 1).min(4);
            let mut reading = Reading::new("error", &error);
            reading.last_checked = now();
            (reading, backoff(*failures, (w.interval)(None)))
        }
    }
}

fn run_extra(w: ExtraWorker, rx: Receiver<ExtraAction>) {
    let mut session: Option<Session> = None;
    let mut next_poll = w.first_poll;
    let mut failures = 0_u32;
    while !w.shared.stop.load(Ordering::Acquire) {
        let mut pending_reset = None;
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(ExtraAction::Refresh) => next_poll = Instant::now(),
            Ok(ExtraAction::Redeem(key, reply)) => {
                let ready = w
                    .store
                    .lock()
                    .unwrap()
                    .get(&w.id)
                    .is_some_and(|(reading, _)| reading.phase == "ready");
                match session.as_mut().filter(|_| ready) {
                    Some(client) => {
                        // This child only: its own CODEX_HOME, never the primary.
                        let outcome = client
                            .call(
                                "account/rateLimitResetCredit/consume",
                                json!({"idempotencyKey":key}),
                            )
                            .and_then(|value| reset_outcome(&value));
                        // Re-read this account before acknowledging.
                        next_poll = Instant::now();
                        pending_reset = Some((reply, outcome));
                    }
                    None => {
                        let _ = reply
                            .send(Err("Reconnecte ce compte Codex avant de réessayer.".into()));
                        continue;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        if let Some(client) = session.as_mut() {
            for notice in client.drain() {
                if matches!(
                    notice["method"].as_str(),
                    Some("account/updated" | "account/rateLimits/updated")
                ) {
                    next_poll = Instant::now();
                }
            }
        }
        if Instant::now() < next_poll || w.shared.stop.load(Ordering::Acquire) {
            continue;
        }
        let current = w.store.lock().unwrap().begin(&w.id, w.generation);
        if !current {
            break; // removed or relaunched: this worker is obsolete
        }
        (w.changed)();
        let (reading, delay) = poll_extra(&w, &mut session, &mut failures);
        next_poll = Instant::now() + delay;
        if w.shared.stop.load(Ordering::Acquire) {
            break;
        }
        let refreshed = reading.phase == "ready" && reading.rows.is_some();
        let committed = w.store.lock().unwrap().commit(&w.id, w.generation, reading);
        if committed {
            (w.changed)();
        }
        if let Some((reply, outcome)) = pending_reset {
            // A confirmed redemption stays confirmed even if the re-read fails.
            let _ = reply.send(redemption_result(outcome, refreshed && committed));
        }
    }
    drop(session); // closes stdin and stops this child only
    w.shared.kill_child();
}

/// Discovered environments for this app (names/paths only).
pub(crate) fn discovered(
    app: &tauri::AppHandle,
    prefs: &crate::prefs::Prefs,
) -> Vec<crate::codex_accounts::Discovered> {
    let Ok(home) = app.path().home_dir() else {
        return vec![];
    };
    let env = std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    crate::codex_accounts::discover(&home, env.as_deref(), &prefs.codex_accounts)
}

/// One Codex environment as displayed and notified. `rows` follows the
/// historical plan filter; `raw_rows` keeps every window for notifications.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccount {
    #[serde(flatten)]
    pub profile: crate::claude_accounts::Profile,
    pub primary: bool,
    pub tracked: bool,
    pub available: bool,
    pub refreshing: bool,
    pub phase: String,
    pub message: String,
    pub plan_type: Option<String>,
    pub last_checked: Option<String>,
    /// Earned resets of THIS account (`None`: unknown or not connected).
    pub available_resets: Option<u64>,
    pub rows: Vec<QuotaPayload>,
    /// Name of an earlier account returning the same identity (not added up).
    pub duplicate_of: Option<String>,
    #[serde(skip)]
    pub raw_rows: Vec<QuotaPayload>,
    #[serde(skip)]
    identity: Option<u64>,
}

/// Primary data: the historical collector, untouched.
pub(crate) struct PrimaryView {
    pub status: Status,
    pub rows: Vec<QuotaPayload>,
    pub identity: Option<u64>,
}

pub(crate) fn account_view(
    d: crate::codex_accounts::Discovered,
    enabled: bool,
    primary: Option<&PrimaryView>,
    slot: Option<(Reading, bool)>,
) -> CodexAccount {
    let reading = if d.primary {
        primary
            .map(|p| Reading {
                phase: p.status.phase.clone(),
                message: p.status.message.clone(),
                plan_type: p.status.plan_type.clone(),
                rows: Some(p.rows.clone()),
                identity: p.identity,
                last_checked: p.status.last_checked.clone(),
                available_resets: p.status.available_resets,
            })
            .unwrap_or_else(|| Reading::new("starting", "Connexion à Codex…"))
    } else if !d.tracked {
        Reading::new("detected", "Dossier détecté, non suivi.")
    } else if !enabled {
        Reading::new("disabled", "Codex est désactivé dans les Réglages.")
    } else if !d.available {
        Reading::new(
            "unavailable",
            "Dossier CODEX_HOME indisponible : vérifiez qu’il existe encore.",
        )
    } else {
        slot.as_ref()
            .map(|(r, _)| r.clone())
            .unwrap_or_else(|| Reading::new("starting", "Connexion à Codex…"))
    };
    let refreshing = !d.primary && slot.is_some_and(|(_, busy)| busy);
    let raw = if enabled {
        reading.rows.clone().unwrap_or_default()
    } else {
        vec![]
    };
    let mut profile = d.profile;
    profile.visible = d.visible && enabled;
    let available_resets = (enabled && reading.phase == "ready")
        .then_some(reading.available_resets)
        .flatten();
    CodexAccount {
        rows: crate::visible_quotas(raw.clone(), reading.plan_type.as_deref()),
        raw_rows: raw,
        profile,
        primary: d.primary,
        tracked: d.tracked,
        available: d.available,
        refreshing,
        phase: reading.phase,
        message: reading.message,
        plan_type: reading.plan_type,
        last_checked: reading.last_checked,
        available_resets,
        duplicate_of: None,
        identity: reading.identity,
    }
}

/// Two followed homes returning the same identity are flagged on the later
/// one (never merged, never added up).
pub(crate) fn mark_duplicates(accounts: &mut [CodexAccount]) {
    for i in 0..accounts.len() {
        let Some(identity) = accounts[i].identity.filter(|_| accounts[i].tracked) else {
            continue;
        };
        accounts[i].duplicate_of = accounts[..i]
            .iter()
            .find(|a| a.tracked && a.identity == Some(identity))
            .map(|a| a.profile.name.clone());
    }
}

/// Every discovered environment (primary first), display view.
pub fn accounts(app: &tauri::AppHandle) -> Vec<CodexAccount> {
    let prefs = crate::prefs::current(app);
    let provider = app.try_state::<Provider>();
    let primary = provider.as_ref().map(|p| PrimaryView {
        status: p.status(),
        rows: app
            .state::<crate::QuotaState>()
            .0
            .lock()
            .map(|rows| rows.clone())
            .unwrap_or_default(),
        identity: *p.shared.identity.lock().unwrap(),
    });
    let mut list: Vec<_> = discovered(app, &prefs)
        .into_iter()
        .map(|d| {
            let slot = provider
                .as_ref()
                .and_then(|p| p.store.lock().unwrap().get(&d.profile.id));
            account_view(d, prefs.codex_enabled, primary.as_ref(), slot)
        })
        .collect();
    mark_duplicates(&mut list);
    list
}

#[tauri::command]
pub fn get_codex_accounts(app: tauri::AppHandle) -> Vec<CodexAccount> {
    accounts(&app)
}

fn edit_accounts(
    app: tauri::AppHandle,
    edit: impl FnOnce(
        &std::path::Path,
        Option<&std::path::Path>,
        &crate::claude_accounts::AccountPrefs,
    ) -> Result<crate::claude_accounts::AccountPrefs, String>,
) -> Result<crate::prefs::Prefs, String> {
    let mut p = crate::prefs::current(&app);
    let home = app
        .path()
        .home_dir()
        .map_err(|_| "Dossier utilisateur introuvable.")?;
    let env = std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    p.codex_accounts = edit(&home, env.as_deref(), &p.codex_accounts)?;
    crate::prefs::set_preferences(app, p)
}

/// « Ajouter un compte » with a chosen/typed existing CODEX_HOME folder.
#[tauri::command]
pub fn add_codex_profile(app: tauri::AppHandle, path: String) -> Result<crate::prefs::Prefs, String> {
    edit_accounts(app, |home, env, prefs| {
        crate::codex_accounts::follow(home, env, prefs, crate::codex_accounts::FollowTarget::Path(&path))
    })
}

/// Follow a detected `~/.codex-*` folder.
#[tauri::command]
pub fn follow_codex_profile(app: tauri::AppHandle, id: String) -> Result<crate::prefs::Prefs, String> {
    edit_accounts(app, |home, env, prefs| {
        crate::codex_accounts::follow(home, env, prefs, crate::codex_accounts::FollowTarget::Detected(&id))
    })
}

/// Stop following (never deletes or changes the folder).
#[tauri::command]
pub fn untrack_codex_profile(app: tauri::AppHandle, id: String) -> Result<crate::prefs::Prefs, String> {
    edit_accounts(app, |home, env, prefs| {
        crate::codex_accounts::untrack(home, env, prefs, &id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_refresh_bypasses_an_adaptive_poll_delay() {
        let now = Instant::now();
        // Reproduce the activity detector extending the interval to five minutes.
        let next_poll = now + Duration::from_secs(300);
        assert!(!poll_due(now, next_poll, false));
        assert!(poll_due(now, next_poll, true));
        assert!(poll_due(next_poll, next_poll, false));
    }
    #[test]
    fn a_failed_refresh_does_not_undo_a_confirmed_reset() {
        for outcome in ["reset", "alreadyRedeemed", "nothingToReset", "noCredit"] {
            let result = redemption_result(Ok(outcome.into()), false).unwrap();
            assert_eq!(result.outcome, outcome);
            assert!(!result.quotas_refreshed);
        }
        assert!(
            redemption_result(Ok("reset".into()), true)
                .unwrap()
                .quotas_refreshed
        );
        assert!(redemption_result(Err("unknown outcome".into()), true).is_err());
    }
    #[test]
    fn reset_count_preserves_unknown_and_uses_authoritative_count() {
        assert_eq!(available_resets(&json!({})), None);
        assert_eq!(
            available_resets(&json!({"rateLimitResetCredits":null})),
            None
        );
        assert_eq!(
            available_resets(&json!({"rateLimitResetCredits":{"availableCount":0}})),
            Some(0)
        );
        assert_eq!(
            available_resets(&json!({"rateLimitResetCredits":{"availableCount":3,"credits":[]}})),
            Some(3)
        );
        assert_eq!(
            available_resets(&json!({"rateLimitResetCredits":{"availableCount":-1}})),
            None
        );
    }
    #[test]
    fn reset_outcomes_do_not_infer_quota() {
        for outcome in ["reset", "alreadyRedeemed", "nothingToReset", "noCredit"] {
            assert_eq!(reset_outcome(&json!({"outcome":outcome})).unwrap(), outcome);
        }
        assert!(reset_outcome(&json!({})).is_err());
    }
    #[test]
    fn oversized_protocol_messages_are_rejected() {
        assert!(read_protocol_line(&mut std::io::Cursor::new(vec![b'x'; 1_048_577])).is_err());
        assert_eq!(
            read_protocol_line(&mut std::io::Cursor::new(b"{}\n")).unwrap(),
            b"{}\n"
        );
        assert!(read_protocol_line(&mut std::io::Cursor::new(b""))
            .unwrap()
            .is_empty());
    }
    #[test]
    fn login_urls_reject_untrusted_destinations() {
        assert!(trusted_login_url(
            "https://auth.openai.com/authorize?state=test"
        ));
        for url in [
            "http://auth.openai.com/",
            "https://auth.openai.com.evil.test/",
            "https://auth.openai.com@evil.test/",
            "https://auth.openai.com:8443/",
            "file:///tmp/login",
        ] {
            assert!(!trusted_login_url(url));
        }
    }
    #[test]
    fn refresh_presets_and_override() {
        let auto = RefreshSettings::default();
        assert_eq!(auto.adaptive_seconds(Some("plus"), Some(0)), 300);
        assert_eq!(auto.adaptive_seconds(Some("plus"), Some(1)), 60);
        assert_eq!(auto.adaptive_seconds(Some("plus"), Some(2)), 30);
        assert_eq!(auto.adaptive_seconds(Some("plus"), Some(4)), 15);
        assert_eq!(
            RefreshSettings {
                custom_seconds: Some(120)
            }
            .adaptive_seconds(Some("plus"), Some(4)),
            120
        );
        assert_eq!(auto.seconds(Some("plus")), 60);
        assert_eq!(auto.seconds(Some("free")), 120);
        assert_eq!(auto.seconds(None), 120);
        assert_eq!(
            RefreshSettings {
                custom_seconds: Some(300)
            }
            .seconds(Some("plus")),
            300
        );
        assert!(!RefreshSettings {
            custom_seconds: Some(0)
        }
        .valid());
    }
    #[test]
    fn weekly_only_and_multiple_buckets_are_preserved() {
        let data = json!({"rateLimitsByLimitId":{
            "codex":{"primary":null,"secondary":{"usedPercent":59,"windowDurationMins":10080,"resetsAt":1790341067}},
            "other":{"limitName":"Other","primary":{"usedPercent":12,"windowDurationMins":300,"resetsAt":null}}
        }});
        let rows = normalize(&data).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].remaining, 41);
        assert_eq!(rows[0].period, "codex:codex:secondary");
        assert_eq!(rows[0].label.as_deref(), Some("Codex · 7 jours"));
        assert!(rows[1].reset_at.is_empty());
    }
    #[test]
    fn empty_success_is_distinct_from_malformed_data() {
        assert!(
            normalize(&json!({"rateLimits":{"primary":null,"secondary":null}}))
                .unwrap()
                .is_empty()
        );
        assert!(normalize(&json!({"rateLimitsByLimitId":{}}))
            .unwrap()
            .is_empty());
        assert!(normalize(&json!({})).is_err());
        assert!(normalize(&json!({"rateLimits":{"primary":{"usedPercent":"invalid"}}})).is_err());
    }
    #[test]
    fn remaining_percentage_is_clamped() {
        let rows = normalize(
            &json!({"rateLimits":{"primary":{"usedPercent":125},"secondary":{"usedPercent":-1}}}),
        )
        .unwrap();
        assert_eq!(rows[0].remaining, 0);
        assert_eq!(rows[1].remaining, 100);
    }
    #[cfg(unix)]
    const FAKE_CODEX: &str = r#"#!/bin/sh
log="__LOG__"
[ -f "$CODEX_HOME/fixture-broken" ] && exit 3
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{"id":\([0-9]*\),.*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  printf '%s|%s|%s\n' "$method" "${CODEX_HOME-<unset>}" "$line" >> "$log"
  [ -z "$id" ] && continue
  case "$method" in
    initialize) printf '{"id":%s,"result":{}}\n' "$id" ;;
    account/read)
      if [ -f "$CODEX_HOME/fixture-identity" ]; then
        printf '{"id":%s,"result":{"account":{"type":"chatgpt","email":"%s","planType":"plus"},"requiresOpenaiAuth":true}}\n' "$id" "$(cat "$CODEX_HOME/fixture-identity")"
      else
        printf '{"id":%s,"result":{"account":null,"requiresOpenaiAuth":true}}\n' "$id"
      fi ;;
    account/rateLimits/read)
      used=$(cat "$CODEX_HOME/fixture-used" 2>/dev/null || echo 20)
      credits=""
      [ -f "$CODEX_HOME/fixture-resets" ] && credits=",\"rateLimitResetCredits\":{\"availableCount\":$(cat "$CODEX_HOME/fixture-resets")}"
      printf '{"id":%s,"result":{"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":%s,"windowDurationMins":300,"resetsAt":1900000000},"secondary":{"usedPercent":10,"windowDurationMins":10080,"resetsAt":1900500000}}}%s}}\n' "$id" "$used" "$credits" ;;
    account/rateLimitResetCredit/consume)
      left=$(cat "$CODEX_HOME/fixture-resets" 2>/dev/null || echo 0)
      if [ "$left" -gt 0 ]; then
        echo $((left - 1)) > "$CODEX_HOME/fixture-resets"
        printf '{"id":%s,"result":{"outcome":"reset"}}\n' "$id"
      else
        printf '{"id":%s,"result":{"outcome":"noCredit"}}\n' "$id"
      fi ;;
    *) printf '{"id":%s,"error":{"code":-1,"message":"unexpected"}}\n' "$id" ;;
  esac
done
"#;
    #[cfg(unix)]
    fn fake_codex(root: &std::path::Path) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let log = root.join("calls.log");
        let exe = root.join("fake-codex");
        fs::write(&exe, FAKE_CODEX.replace("__LOG__", log.to_str().unwrap())).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
        (exe, log)
    }
    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("qm-codex-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }
    #[cfg(unix)]
    fn spawn_extra(
        store: &Arc<Mutex<AccountStore>>,
        id: &str,
        launch: &std::path::Path,
        exe: &std::path::Path,
    ) -> (Arc<Shared>, Sender<ExtraAction>, std::thread::JoinHandle<()>) {
        let generation = store.lock().unwrap().register(id);
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::channel();
        let exe = exe.to_path_buf();
        let worker = ExtraWorker {
            id: id.into(),
            generation,
            launch: launch.to_path_buf(),
            shared: shared.clone(),
            store: store.clone(),
            executable: Box::new(move || Some(exe.clone())),
            interval: Box::new(|_| 3600),
            changed: Box::new(|| {}),
            first_poll: Instant::now(),
        };
        (shared, tx, std::thread::spawn(move || run_extra(worker, rx)))
    }
    #[cfg(unix)]
    fn wait_for(store: &Arc<Mutex<AccountStore>>, ids: &[&str]) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            let done = ids.iter().all(|id| {
                store
                    .lock()
                    .unwrap()
                    .get(id)
                    .is_some_and(|(r, busy)| !busy && r.phase != "starting")
            });
            if done {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("fake Codex children did not answer in time");
    }

    #[cfg(unix)]
    #[test]
    fn each_followed_home_gets_its_own_child_reads_without_login_and_fails_alone() {
        let root = scratch("extras");
        let (exe, log) = fake_codex(&root);
        let home = |name: &str, identity: Option<&str>, used: Option<&str>| {
            let dir = root.join(name);
            fs::create_dir_all(&dir).unwrap();
            if let Some(identity) = identity {
                fs::write(dir.join("fixture-identity"), identity).unwrap();
            }
            if let Some(used) = used {
                fs::write(dir.join("fixture-used"), used).unwrap();
            }
            dir
        };
        let alpha = home("alpha", Some("Same@Example.invalid"), Some("30"));
        let beta = home("beta", None, None); // signed out: account null
        let gamma = home("gamma", Some("same@example.invalid"), Some("55"));
        let dead = home("dead", None, None);
        fs::write(dead.join("fixture-broken"), "").unwrap();
        let before = std::env::var_os("CODEX_HOME");
        let store = Arc::new(Mutex::new(AccountStore::default()));
        let mut workers = vec![];
        let homes = [("alpha", &alpha), ("beta", &beta), ("gamma", &gamma), ("dead", &dead)];
        for (id, dir) in homes {
            workers.push(spawn_extra(&store, id, dir, &exe));
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            wait_for(&store, &["alpha", "beta", "gamma", "dead"]);
            let get = |id: &str| store.lock().unwrap().get(id).unwrap().0;
            let (a, b, g, d) = (get("alpha"), get("beta"), get("gamma"), get("dead"));
            assert_eq!(a.phase, "ready");
            assert_eq!(a.plan_type.as_deref(), Some("plus"));
            let rows = a.rows.as_ref().unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].remaining, 70);
            assert_eq!(g.rows.as_ref().unwrap()[0].remaining, 45, "no aggregation");
            assert!(a.identity.is_some() && a.identity == g.identity, "same account detected");
            assert_eq!(b.phase, "signed_out");
            assert!(b.rows.is_none() && b.identity.is_none(), "no invented quota");
            assert!(b.message.contains("CODEX_HOME=") && b.message.contains("codex login"));
            assert!(b.message.contains(beta.to_str().unwrap()));
            assert_eq!(d.phase, "error");
            assert!(d.rows.is_none());
            // Process environment unchanged; each child saw only its home.
            assert_eq!(std::env::var_os("CODEX_HOME"), before);
            let calls = fs::read_to_string(&log).unwrap();
            for line in calls.lines() {
                let mut parts = line.splitn(3, '|');
                let (method, seen, raw) = (
                    parts.next().unwrap(),
                    parts.next().unwrap(),
                    parts.next().unwrap(),
                );
                assert!(
                    ["initialize", "initialized", "account/read", "account/rateLimits/read"]
                        .contains(&method),
                    "unexpected call {method}"
                );
                assert!(homes.iter().any(|(_, dir)| dir.to_str() == Some(seen)), "{seen}");
                if method == "account/read" {
                    assert!(raw.contains(r#""refreshToken":false"#), "{raw}");
                }
                if method == "account/rateLimits/read" {
                    assert_ne!(seen, beta.to_str().unwrap(), "no quota read when signed out");
                }
            }
            assert!(!calls.contains("login") && !calls.contains("logout"));
            assert!(!calls.contains("rateLimitResetCredit"));
        }));
        // finally: stop and reap every child, even after a failed assertion.
        for (shared, _, _) in &workers {
            shared.stop.store(true, Ordering::Release);
        }
        for (shared, tx, handle) in workers {
            drop(tx);
            handle.join().unwrap();
            assert!(shared.child.lock().unwrap().is_none(), "child reaped");
        }
        fs::remove_dir_all(root).unwrap();
        result.unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn primary_session_inherits_the_app_environment_without_codex_home() {
        let root = scratch("primary-env");
        let (exe, log) = fake_codex(&root);
        let shared = Arc::new(Shared::default());
        let session = Session::start(&exe, shared.clone());
        let seen = fs::read_to_string(&log).unwrap_or_default();
        drop(session);
        shared.kill_child();
        fs::remove_dir_all(root).unwrap();
        let expected = std::env::var("CODEX_HOME").unwrap_or_else(|_| "<unset>".into());
        let first = seen.lines().next().expect("initialize logged");
        assert_eq!(first.split('|').nth(1), Some(expected.as_str()));
    }

    #[test]
    fn account_store_rejects_stale_and_removed_generations() {
        let mut store = AccountStore::default();
        let g1 = store.register("a");
        assert!(store.begin("a", g1));
        assert!(store.get("a").unwrap().1, "refreshing state is per account");
        assert!(store.get("b").is_none());
        let g2 = store.register("a"); // home relaunched while g1 was in flight
        assert_ne!(g1, g2);
        assert!(!store.commit("a", g1, Reading::new("ready", "late")));
        assert_eq!(store.get("a").unwrap().0.phase, "starting");
        assert!(store.begin("a", g2));
        assert!(store.commit("a", g2, Reading::new("ready", "fresh")));
        let gb = store.register("b");
        assert!(store.commit("b", gb, Reading::new("error", "b failed")));
        assert_eq!(store.get("a").unwrap().0.message, "fresh", "errors isolated");
        store.remove("a");
        assert!(!store.begin("a", g2));
        assert!(!store.commit("a", g2, Reading::new("ready", "removed")));
        assert!(store.get("a").is_none());
    }

    #[test]
    fn fingerprint_uses_only_the_returned_account_id_never_tokens() {
        let fp = |v: Value| account_fingerprint(&v);
        let a = fp(json!({"account":{"type":"chatgpt","email":" A@Example.invalid ","planType":"plus"}}));
        assert!(a.is_some());
        assert_eq!(a, fp(json!({"account":{"type":"chatgpt","email":"a@example.invalid"}})));
        assert_ne!(a, fp(json!({"account":{"type":"chatgpt","email":"b@example.invalid"}})));
        assert_eq!(fp(json!({"account":null,"requiresOpenaiAuth":true})), None);
        assert_eq!(fp(json!({"account":{"type":"chatgpt","email":null}})), None);
        assert!(fp(json!({"account":{"type":"chatgpt","email":null},"workspaceRouting":{"chatgptAccountId":"acc-1"}})).is_some());
        // Tokens/keys are never looked at, even if a payload carried them.
        assert_eq!(fp(json!({"account":{"type":"apiKey"},"accessToken":"x","apiKey":"y"})), None);
    }

    fn found(name: &str, primary: bool, tracked: bool, available: bool, visible: bool) -> crate::codex_accounts::Discovered {
        crate::codex_accounts::Discovered {
            profile: crate::claude_accounts::Profile {
                id: format!("profile:{name}"),
                path: format!("/fixture/{name}"),
                name: name.into(),
                visible,
            },
            primary,
            tracked,
            launch: (!primary).then(|| PathBuf::from(format!("/fixture/{name}"))),
            available,
            visible,
        }
    }
    fn ready(identity: u64, used: f64) -> Reading {
        let mut r = Reading::new("ready", "ok");
        r.identity = Some(identity);
        r.rows = Some(
            normalize(&json!({"rateLimits":{"primary":{"usedPercent":used,"windowDurationMins":300}}}))
                .unwrap(),
        );
        r
    }

    #[test]
    fn account_views_cover_none_signed_out_unavailable_and_duplicates_without_sums() {
        let primary = PrimaryView {
            status: Status {
                phase: "ready".into(),
                plan_type: Some("plus".into()),
                ..Status::default()
            },
            rows: ready(1, 40.0).rows.unwrap(),
            identity: Some(1),
        };
        let mut list = vec![
            account_view(found("Principal", true, true, true, true), true, Some(&primary), None),
            account_view(found("work", false, true, true, true), true, None, Some((ready(2, 10.0), false))),
            account_view(found("alias", false, true, true, true), true, None, Some((ready(1, 40.0), true))),
            account_view(
                found("out", false, true, true, true),
                true,
                None,
                Some((Reading::new("signed_out", "Non connecté."), false)),
            ),
            account_view(found("gone", false, true, false, true), true, None, None),
            account_view(found("seen", false, false, true, false), true, None, None),
        ];
        mark_duplicates(&mut list);
        assert_eq!(list[0].rows[0].remaining, 60);
        assert_eq!(list[1].duplicate_of, None);
        assert_eq!(list[2].duplicate_of.as_deref(), Some("Principal"));
        assert!(list[2].refreshing && !list[1].refreshing);
        // Each card keeps its own value: nothing is added up.
        assert_eq!(list[2].rows[0].remaining, 60);
        assert_eq!(list[1].rows[0].remaining, 90);
        assert_eq!(list[3].phase, "signed_out");
        assert!(list[3].rows.is_empty());
        assert_eq!(list[4].phase, "unavailable");
        assert_eq!(list[5].phase, "detected");
        assert!(!list[5].profile.visible && list[5].duplicate_of.is_none());
        // Disabled provider: nothing visible, nothing reported.
        let off = account_view(found("work", false, true, true, true), false, None, Some((ready(2, 10.0), false)));
        assert!(!off.profile.visible && off.rows.is_empty() && off.phase == "disabled");
        // Serialised view never carries the fingerprint.
        let json = serde_json::to_value(&list[2]).unwrap();
        assert!(json.get("identity").is_none() && json.get("rawRows").is_none());
        assert_eq!(json["duplicateOf"], "Principal");
    }

    #[test]
    fn reset_credits_are_reported_per_account_and_only_when_ready() {
        let primary = PrimaryView {
            status: Status {
                phase: "ready".into(),
                available_resets: Some(2),
                ..Status::default()
            },
            rows: ready(1, 40.0).rows.unwrap(),
            identity: Some(1),
        };
        let mut with_credit = ready(2, 10.0);
        with_credit.available_resets = Some(1);
        let mut signed_out = Reading::new("signed_out", "Non connecté.");
        signed_out.available_resets = Some(9); // never shown without a connection
        let list = [
            account_view(
                found("Principal", true, true, true, true),
                true,
                Some(&primary),
                None,
            ),
            account_view(
                found("work", false, true, true, true),
                true,
                None,
                Some((with_credit, false)),
            ),
            account_view(
                found("out", false, true, true, true),
                true,
                None,
                Some((signed_out, false)),
            ),
            account_view(found("seen", false, false, true, false), true, None, None),
        ];
        assert_eq!(
            list[0].available_resets,
            Some(2),
            "primary keeps its own count"
        );
        assert_eq!(
            list[1].available_resets,
            Some(1),
            "each home has its own count"
        );
        assert_eq!(list[2].available_resets, None);
        assert_eq!(list[3].available_resets, None);
        assert_eq!(
            serde_json::to_value(&list[1]).unwrap()["availableResets"],
            1
        );
    }

    #[test]
    fn a_reset_is_routed_to_the_asked_account_and_never_defaults_to_the_primary() {
        let primary = PrimaryView {
            status: Status {
                phase: "ready".into(),
                ..Status::default()
            },
            rows: ready(1, 40.0).rows.unwrap(),
            identity: Some(1),
        };
        let mut list = vec![
            account_view(
                found("Principal", true, true, true, true),
                true,
                Some(&primary),
                None,
            ),
            account_view(
                found("work", false, true, true, true),
                true,
                None,
                Some((ready(2, 10.0), false)),
            ),
            account_view(
                found("alias", false, true, true, true),
                true,
                None,
                Some((ready(1, 40.0), false)),
            ),
            account_view(
                found("out", false, true, true, true),
                true,
                None,
                Some((Reading::new("signed_out", "x"), false)),
            ),
            account_view(found("seen", false, false, true, false), true, None, None),
        ];
        mark_duplicates(&mut list);
        assert_eq!(
            redeem_route(&list, "profile:Principal"),
            Ok(RedeemRoute::Primary)
        );
        assert_eq!(
            redeem_route(&list, "profile:work"),
            Ok(RedeemRoute::Extra("profile:work".into()))
        );
        for refused in [
            "profile:alias",
            "profile:out",
            "profile:seen",
            "profile:unknown",
            "",
        ] {
            assert!(
                redeem_route(&list, refused).is_err(),
                "{refused} must not reset anything"
            );
        }
        // A primary that is not connected is refused as well.
        let mut off = list.clone();
        off[0].phase = "signed_out".into();
        assert!(redeem_route(&off, "profile:Principal").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn an_extra_home_redeems_on_its_own_child_only_and_refreshes_only_itself() {
        let root = scratch("extra-redeem");
        let (exe, log) = fake_codex(&root);
        let home = |name: &str, resets: &str| {
            let dir = root.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("fixture-identity"),
                format!("{name}@example.invalid"),
            )
            .unwrap();
            fs::write(dir.join("fixture-resets"), resets).unwrap();
            dir
        };
        let alpha = home("alpha", "2");
        let beta = home("beta", "3");
        let store = Arc::new(Mutex::new(AccountStore::default()));
        let workers = vec![
            spawn_extra(&store, "alpha", &alpha, &exe),
            spawn_extra(&store, "beta", &beta, &exe),
        ];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            wait_for(&store, &["alpha", "beta"]);
            let count = |id: &str| store.lock().unwrap().get(id).unwrap().0.available_resets;
            assert_eq!((count("alpha"), count("beta")), (Some(2), Some(3)));
            let key = "0b8f3f0e-8a5e-4c55-9d52-5d2f3c1e9a10";
            let (tx, rx) = mpsc::channel();
            workers[0]
                .1
                .send(ExtraAction::Redeem(key.into(), tx))
                .unwrap();
            let done = rx.recv_timeout(Duration::from_secs(15)).unwrap().unwrap();
            assert_eq!(done.outcome, "reset");
            assert!(
                done.quotas_refreshed,
                "the account is re-read before the answer"
            );
            assert_eq!(
                (count("alpha"), count("beta")),
                (Some(1), Some(3)),
                "only alpha changed"
            );
            let calls = fs::read_to_string(&log).unwrap();
            let consumed: Vec<_> = calls
                .lines()
                .filter(|l| l.starts_with("account/rateLimitResetCredit/consume|"))
                .collect();
            assert_eq!(consumed.len(), 1);
            assert!(
                consumed[0].contains(&format!("|{}|", alpha.display())),
                "{}",
                consumed[0]
            );
            assert!(consumed[0].contains(key));
            assert!(!consumed[0].contains(beta.to_str().unwrap()));
            // A home that is not ready refuses instead of falling back to anything else.
            let (tx, rx) = mpsc::channel();
            store
                .lock()
                .unwrap()
                .commit("beta", 2, Reading::new("signed_out", "x"));
            workers[1]
                .1
                .send(ExtraAction::Redeem(key.into(), tx))
                .unwrap();
            assert!(rx.recv_timeout(Duration::from_secs(15)).unwrap().is_err());
        }));
        for (shared, _, _) in &workers {
            shared.stop.store(true, Ordering::Release);
        }
        for (shared, tx, handle) in workers {
            drop(tx);
            handle.join().unwrap();
            assert!(shared.child.lock().unwrap().is_none());
        }
        fs::remove_dir_all(root).unwrap();
        result.unwrap();
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "opt-in: launches the installed Codex CLI on two EMPTY scratch CODEX_HOME folders"]
    fn live_empty_scratch_homes_report_signed_out_without_leaking_the_primary() {
        // Scratch homes under evidence/<task>/ only; `ephemeral` credential
        // storage so the CLI never consults the keychain. Nothing is read
        // from ~/.codex; only phase/identity presence/file names are printed.
        let exe = discover(std::env::var("QUOTA_CODEX_BINARY").ok()).expect("Codex executable");
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../evidence/t_007e7d14/runtime")
            .join(format!("run-{}", std::process::id()));
        let homes: Vec<PathBuf> = ["empty-a", "empty-b"]
            .iter()
            .map(|name| {
                let dir = root.join(name);
                fs::create_dir_all(&dir).unwrap();
                fs::write(dir.join("config.toml"), "cli_auth_credentials_store = \"ephemeral\"\n").unwrap();
                dir.canonicalize().unwrap()
            })
            .collect();
        let before = std::env::var_os("CODEX_HOME");
        let store = Arc::new(Mutex::new(AccountStore::default()));
        let workers: Vec<_> = homes
            .iter()
            .enumerate()
            .map(|(i, dir)| spawn_extra(&store, &format!("empty-{i}"), dir, &exe))
            .collect();
        let started = Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            wait_for(&store, &["empty-0", "empty-1"]); // hard 15 s deadline
            for i in 0..homes.len() {
                let reading = store.lock().unwrap().get(&format!("empty-{i}")).unwrap().0;
                println!(
                    "home {i}: phase={} identity_present={} rows_present={}",
                    reading.phase,
                    reading.identity.is_some(),
                    reading.rows.is_some()
                );
                assert_eq!(reading.phase, "signed_out", "{}", reading.message);
                assert!(reading.identity.is_none() && reading.rows.is_none(), "no primary leak");
            }
            assert_eq!(std::env::var_os("CODEX_HOME"), before, "process environment untouched");
        }));
        // finally: stop and reap both children, even after a failure or timeout.
        for (shared, _, _) in &workers {
            shared.stop.store(true, Ordering::Release);
            shared.kill_child();
        }
        for (shared, tx, handle) in workers {
            drop(tx);
            handle.join().unwrap();
            assert!(shared.child.lock().unwrap().is_none(), "child reaped");
        }
        println!("elapsed={} ms", started.elapsed().as_millis());
        for (i, dir) in homes.iter().enumerate() {
            let mut names: Vec<_> = fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            println!("home {i} entries (names only): {names:?}");
            assert!(!names.iter().any(|n| n == "auth.json"), "no credentials written");
        }
        result.unwrap();
    }

    #[test]
    #[ignore = "requires installed Codex and an existing ChatGPT login"]
    fn live_codex_account_read() {
        let mut detector = crate::activity::Detector::new();
        let instances = detector.count().expect("local process metadata available");
        println!("Detected {instances} top-level Codex instances using local metadata.");
        let path = discover(std::env::var("QUOTA_CODEX_BINARY").ok()).expect("Codex executable");
        let mut session = Session::start(&path, Arc::new(Shared::default())).unwrap();
        let account = session
            .call("account/read", json!({"refreshToken":false}))
            .unwrap();
        assert_eq!(account["account"]["type"].as_str(), Some("chatgpt"));
        let result = session
            .call("account/rateLimits/read", Value::Null)
            .unwrap();
        let rows = normalize(&result).unwrap();
        println!(
            "Available earned resets reported by Codex: {:?}",
            available_resets(&result)
        );
        println!(
            "Validated {} quota windows; no account or credential fields printed.",
            rows.len()
        );
    }
}
