//! Opt-in, best-effort ntfy delivery. A single bounded worker owns the persistent
//! attempt ledger; quota collection never waits for disk or network delivery.
use crate::{claude, codex, prefs, QuotaState};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, sync::mpsc, time::Duration};
use tauri::Manager;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub enabled: bool,
    pub server_url: String,
    pub topic: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            server_url: "https://ntfy.sh".into(),
            topic: String::new(),
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled && self.topic.is_empty() {
            return Ok(());
        }
        self.endpoint().map(|_| ())
    }
    fn endpoint(&self) -> Result<reqwest::Url, String> {
        let mut url =
            reqwest::Url::parse(&self.server_url).map_err(|_| "URL ntfy invalide.".to_string())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "Utilisez une URL HTTP(S) sans identifiants, paramètres ni fragment.".into(),
            );
        }
        if self.topic.is_empty()
            || self.topic.len() > 128
            || !self
                .topic
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(
                "Topic ntfy requis : 1 à 128 lettres, chiffres, tirets ou underscores.".into(),
            );
        }
        url.path_segments_mut()
            .map_err(|_| "URL ntfy invalide.")?
            .pop_if_empty()
            .push(&self.topic);
        Ok(url)
    }
}

fn send(config: &Config, message: &str) -> Result<(), String> {
    let endpoint = config.endpoint()?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Client ntfy indisponible.".to_string())?;
    // Never include server responses/URLs in errors: the topic is a bearer secret.
    let response = client
        .post(endpoint)
        .header("Title", "Quota Monitor")
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(message.to_owned())
        .send()
        .map_err(|_| "Envoi ntfy impossible (réseau ou délai dépassé).".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Serveur ntfy : HTTP {}.",
            response.status().as_u16()
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn test_ntfy(config: Config) -> Result<(), String> {
    // Explicit test is allowed with automatic notifications disabled.
    tauri::async_runtime::spawn_blocking(move || {
        send(
            &config,
            "Test Quota Monitor : les notifications de quota sont configurées.",
        )
    })
    .await
    .map_err(|_| "Test ntfy interrompu.".to_string())?
}

struct Signal(mpsc::SyncSender<()>);
pub fn wake(app: &tauri::AppHandle) {
    if let Some(signal) = app.try_state::<Signal>() {
        let _ = signal.0.try_send(());
    }
}

#[derive(Clone, Debug)]
struct Observation {
    key: String,
    provider: &'static str,
    window: &'static str,
    used: Option<f64>,
    checked: Option<i64>,
    reset: Option<i64>,
    valid: bool,
}
fn timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|d| d.timestamp())
}
fn observations(app: &tauri::AppHandle, p: &prefs::Prefs) -> Vec<Observation> {
    let mut out = Vec::new();
    if p.codex_enabled {
        let ready = app
            .try_state::<codex::Provider>()
            .is_some_and(|s| s.status().phase == "ready");
        if let Ok(rows) = app.state::<QuotaState>().0.lock() {
            for row in rows.iter() {
                out.push(Observation {
                    key: format!("Codex:{}", row.period),
                    provider: "Codex",
                    window: if crate::is_reserve(row) {
                        "réserve"
                    } else {
                        match row.window_minutes {
                            Some(300) => "5 heures",
                            Some(10080) => "7 jours",
                            _ => "autre limite",
                        }
                    },
                    used: row.used_percent,
                    checked: timestamp(&row.checked_at),
                    reset: timestamp(&row.reset_at),
                    valid: ready && row.confidence == "high" && row.source == "codex",
                });
            }
        }
    }
    if p.claude_enabled {
        out.extend(claude_observations(&claude::raw_snapshot(app)));
    }
    out
}

fn claude_observations(snap: &claude::ClaudeSnapshot) -> Vec<Observation> {
    snap.metrics
        .iter()
        .map(|metric| Observation {
            key: format!("Claude:{}", metric.id),
            provider: "Claude",
            window: metric.label,
            used: metric.used_percent,
            checked: snap.observed_at.as_deref().and_then(timestamp),
            reset: metric.reset_at.as_deref().and_then(timestamp),
            valid: snap.enabled && snap.state == "ready" && metric.availability == "available",
        })
        .collect()
}

// Remember BOTH sides of the threshold: first discovery at 100 is not a crossing.
// Legacy values represent already reserved attempts and remain exhausted.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Counter {
    Observed {
        reset: Option<i64>,
        exhausted: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        muted: bool,
    },
    Legacy(Option<i64>),
}
// Persist before sending: an interrupted process must not repeatedly notify.
#[derive(Default, Serialize, Deserialize)]
struct Ledger(BTreeMap<String, Counter>);
impl Ledger {
    fn observe(&mut self, enabled: bool, o: &Observation, now: i64) -> Option<String> {
        if !enabled
            || !o.valid
            || !o.checked.is_some_and(|at| (0..=120).contains(&(now - at)))
            || o.reset.is_some_and(|at| at <= now)
        {
            return None;
        }
        let used = o.used.filter(|v| v.is_finite() && *v >= 0.0)?;
        let previous = self.0.get(&o.key).map(|counter| match counter {
            Counter::Observed {
                reset, exhausted, ..
            } => (*reset, *exhausted),
            Counter::Legacy(reset) => (*reset, true),
        });
        let exhausted = used >= 100.0;
        let new_cycle = previous.is_some_and(|(reset, _)| {
            reset.is_some_and(|at| o.checked.is_some_and(|checked| checked >= at))
                || matches!((reset, o.reset), (Some(a), Some(b)) if a != b)
        });
        let was_muted = matches!(
            self.0.get(&o.key),
            Some(Counter::Observed { muted: true, .. })
        );
        let notify = !was_muted && exhausted && previous.is_some_and(|(_, was)| !was || new_cycle);
        // Missing reset metadata never rearms. Keep a known future reset so it
        // can still establish a new cycle after a temporary metadata omission.
        let reset = o
            .reset
            .or_else(|| previous.and_then(|(reset, _)| reset.filter(|at| *at > now)));
        self.0.insert(
            o.key.clone(),
            Counter::Observed {
                reset,
                exhausted,
                muted: false,
            },
        );
        if !notify {
            return None;
        }
        let reset = o
            .reset
            .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
            .map(|at| format!(" Réinitialisation : {} UTC.", at.format("%d/%m/%Y %H:%M")))
            .unwrap_or_default();
        Some(format!(
            "{} · {} : 100 % consommé (0 % restant).{}",
            o.provider, o.window, reset
        ))
    }
    #[cfg(test)]
    fn observe_all(&mut self, observations: &[Observation], now: i64) -> Option<String> {
        self.observe_all_with_fable(observations, now, true)
    }
    fn mute_fable(&mut self) {
        if let Some(counter) = self.0.get_mut("Claude:claude:weekly_fable") {
            let reset = match counter {
                Counter::Observed { reset, .. } | Counter::Legacy(reset) => *reset,
            };
            *counter = Counter::Observed {
                reset,
                exhausted: true,
                muted: true,
            };
        }
    }
    fn observe_all_with_fable(
        &mut self,
        observations: &[Observation],
        now: i64,
        fable_enabled: bool,
    ) -> Option<String> {
        // Preserve a silent baseline even when Fable is absent/invalid. The first
        // valid sample after re-enabling cannot turn a hidden crossing into an alert.
        if !fable_enabled {
            self.mute_fable();
        }
        let messages: Vec<_> = observations
            .iter()
            .filter_map(|o| {
                let message = self.observe(true, o, now);
                if !fable_enabled && o.key == "Claude:claude:weekly_fable" {
                    None
                } else {
                    message
                }
            })
            .collect();
        if !fable_enabled {
            self.mute_fable();
        }
        (!messages.is_empty()).then(|| messages.join("\n"))
    }
    fn save(&self, path: &Path) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec(self).map_err(|_| "État ntfy invalide.")?,
        )
        .and_then(|_| fs::rename(tmp, path))
        .map_err(|_| "État ntfy non enregistrable.".into())
    }
}

pub fn start(app: tauri::AppHandle) {
    let (tx, rx) = mpsc::sync_channel(1);
    app.manage(Signal(tx));
    std::thread::spawn(move || {
        let Ok(dir) = app.path().app_local_data_dir() else {
            return;
        };
        if fs::create_dir_all(&dir).is_err() {
            return;
        }
        let path = dir.join("ntfy-attempts.json");
        let mut ledger = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Ledger>(&bytes) {
                Ok(l) => l,
                Err(_) => {
                    eprintln!("Notifications ntfy suspendues : état local invalide.");
                    return;
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ledger::default(),
            Err(_) => {
                eprintln!("Notifications ntfy suspendues : état local illisible.");
                return;
            }
        };
        while rx.recv().is_ok() {
            let p = prefs::current(&app);
            let before = ledger.0.clone();
            if !p.fable_enabled {
                ledger.mute_fable();
            }
            if !p.ntfy.enabled || p.ntfy.endpoint().is_err() {
                // Fable's opt-out survives even when global delivery is off.
                if before != ledger.0 && ledger.save(&path).is_err() {
                    eprintln!("Notifications ntfy suspendues : enregistrement impossible.");
                    return;
                }
                continue;
            }
            let rows = observations(&app, &p);
            if prefs::current(&app) != p {
                continue;
            }
            let message = ledger.observe_all_with_fable(
                &rows,
                chrono::Utc::now().timestamp(),
                p.fable_enabled,
            );
            if before != ledger.0 && ledger.save(&path).is_err() {
                eprintln!("Notifications ntfy suspendues : enregistrement impossible.");
                return;
            }
            if let Some(message) = message {
                // A queued observation may outlive a settings edit. One delivery
                // per refresh includes all newly exhausted windows/models.
                if prefs::current(&app) == p {
                    if let Err(error) = send(&p.ntfy, &message) {
                        eprintln!("{error}");
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn sample(used: f64) -> Observation {
        Observation {
            key: "Codex:primary".into(),
            provider: "Codex",
            window: "5 heures",
            used: Some(used),
            checked: Some(1000),
            reset: Some(2000),
            valid: true,
        }
    }
    fn armed() -> Ledger {
        let mut ledger = Ledger::default();
        assert!(ledger.observe(true, &sample(99.0), 1000).is_none());
        ledger
    }
    #[test]
    fn initial_exhaustion_is_silent_including_polls_restart_and_missing_values() {
        let mut ledger = Ledger::default();
        assert!(ledger.observe(true, &sample(100.0), 1000).is_none());
        let bytes = serde_json::to_vec(&ledger).unwrap();
        let mut ledger: Ledger = serde_json::from_slice(&bytes).unwrap();
        for _ in 0..20 {
            assert!(ledger.observe(true, &sample(100.0), 1000).is_none());
        }
        let mut missing = sample(0.0);
        missing.used = None;
        assert!(ledger.observe(true, &missing, 1000).is_none());
        missing.used = Some(0.0);
        missing.valid = false;
        assert!(ledger.observe(true, &missing, 1000).is_none());
        assert!(ledger.observe(true, &sample(100.0), 1000).is_none());
        assert!(ledger.observe(true, &sample(99.0), 1000).is_none());
        assert!(ledger.observe(true, &sample(100.0), 1000).is_some());
    }
    #[test]
    fn exact_threshold_direction_duplicates_restart_and_rearm() {
        let mut l = Ledger::default();
        for used in [0.0, 50.0, 99.6, 99.99] {
            assert!(l.observe(true, &sample(used), 1000).is_none());
        }
        assert!(l.observe(true, &sample(100.0), 1000).is_some());
        let bytes = serde_json::to_vec(&l).unwrap();
        let mut l: Ledger = serde_json::from_slice(&bytes).unwrap();
        assert!(l.observe(true, &sample(100.0), 1000).is_none());
        assert!(l.observe(true, &sample(99.9), 1000).is_none());
        assert!(l.observe(true, &sample(100.0), 1000).is_some());
        let mut next = sample(100.0);
        next.reset = Some(3000);
        assert!(l.observe(true, &next, 1000).is_some());
        next.key = "Claude:five_hour".into();
        next.provider = "Claude";
        assert!(l.observe(true, &next, 1000).is_none());
        next.used = Some(99.0);
        assert!(l.observe(true, &next, 1000).is_none());
        next.used = Some(100.0);
        assert!(l.observe(true, &next, 1000).unwrap().contains("Claude"));
        next.reset = None;
        assert!(l.observe(true, &next, 1000).is_none());
        next.key = "Claude:seven_day".into();
        assert!(l.observe(true, &next, 1000).is_none());
        next.used = Some(99.0);
        assert!(l.observe(true, &next, 1000).is_none());
        next.used = Some(100.0);
        assert!(l.observe(true, &next, 1000).is_some());
        next.reset = Some(3000);
        assert!(l.observe(true, &next, 1000).is_none());
    }
    #[test]
    fn invalid_disabled_stale_and_errors_never_send_or_rearm() {
        let mut l = Ledger::default();
        assert!(l.observe(false, &sample(100.0), 1000).is_none());
        for used in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(l.observe(true, &sample(used), 1000).is_none());
        }
        let mut invalid = sample(100.0);
        invalid.valid = false;
        assert!(l.observe(true, &invalid, 1000).is_none());
        let mut o = sample(100.0);
        for checked in [None, Some(0), Some(1001)] {
            o.checked = checked;
            assert!(l.observe(true, &o, 1000).is_none());
        }
        o = sample(100.0);
        o.reset = Some(999);
        assert!(l.observe(true, &o, 1000).is_none());
        o = sample(100.0);
        o.used = None;
        assert!(l.observe(true, &o, 1000).is_none());
        assert!(l.0.is_empty());
        assert!(l.observe(true, &sample(99.0), 1000).is_none());
        assert!(l.observe(true, &sample(100.0), 1000).is_some());
        invalid.used = Some(0.0);
        assert!(l.observe(true, &invalid, 1000).is_none());
        assert!(l.observe(true, &sample(100.0), 1000).is_none());
    }
    #[test]
    fn codex_raw_percentage_not_rounded_display_drives_threshold() {
        for (used, expected) in [(0.0, false), (99.6, false), (100.0, true)] {
            let rows = codex::normalize(&serde_json::json!({"rateLimits":{
                "primary":{"usedPercent":used,"windowDurationMins":300}
            }}))
            .unwrap();
            assert_eq!(rows[0].used_percent, Some(used));
            let mut o = sample(used);
            o.used = rows[0].used_percent;
            assert_eq!(armed().observe(true, &o, 1000).is_some(), expected);
        }
    }
    #[test]
    fn ledger_is_persisted_and_reloaded_from_disk() {
        let dir = std::env::temp_dir().join(format!("ntfy-ledger-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("attempts.json");
        let mut ledger = armed();
        assert!(ledger.observe(true, &sample(100.0), 1000).is_some());
        ledger.save(&path).unwrap();
        let mut restored: Ledger = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(restored.observe(true, &sample(100.0), 1000).is_none());
        assert!(restored.observe(true, &sample(0.0), 1000).is_none());
        restored.save(&path).unwrap();
        let mut restored: Ledger = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(restored.observe(true, &sample(100.0), 1000).is_some());
        assert!(restored.save(&dir.join("missing/attempts.json")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn config_defaults_validation_and_roundtrip() {
        let default = Config::default();
        assert!(!default.enabled);
        assert_eq!(default.server_url, "https://ntfy.sh");
        for url in [
            "ftp://localhost",
            "https://user:pass@localhost",
            "https://localhost?q=1",
            "https://localhost/#x",
        ] {
            assert!(Config {
                server_url: url.into(),
                topic: "safe".into(),
                enabled: true
            }
            .endpoint()
            .is_err());
        }
        for topic in ["", "../x", "a/b", "x?y", "a b"] {
            assert!(Config {
                topic: topic.into(),
                ..default.clone()
            }
            .endpoint()
            .is_err());
        }
        let c = Config {
            enabled: true,
            server_url: "http://localhost/base/".into(),
            topic: "random-123".into(),
        };
        assert_eq!(
            c.endpoint().unwrap().as_str(),
            "http://localhost/base/random-123"
        );
        assert_eq!(
            serde_json::from_slice::<Config>(&serde_json::to_vec(&c).unwrap()).unwrap(),
            c
        );
    }
    fn local_post(status: u16) -> (Result<(), String>, String) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let config = Config {
            server_url: format!("http://{}/base", listener.local_addr().unwrap()),
            topic: "local-test-only".into(),
            enabled: true,
        };
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let len: usize = headers
                        .lines()
                        .find_map(|s| s.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            write!(
                socket,
                "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            String::from_utf8(request).unwrap()
        });
        let message = armed().observe(true, &sample(100.0), 1000).unwrap();
        let result = send(&config, &message);
        (result, server.join().unwrap())
    }
    #[test]
    fn real_local_http_post_endpoint_minimal_payload_and_status_errors() {
        let (result, request) = local_post(200);
        result.unwrap();
        assert!(request.starts_with("POST /base/local-test-only HTTP/1.1"));
        assert!(request.contains("Quota Monitor"));
        assert!(request.contains("Codex · 5 heures : 100 % consommé (0 % restant)."));
        assert!(request.contains("UTC."));
        assert!(!request.contains("usedPercent"));
        assert!(local_post(403).0.unwrap_err().contains("403"));
        assert!(local_post(302).0.is_err());
    }
    #[test]
    fn network_failure_is_bounded_and_attempt_is_not_repeated() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let c = Config {
            server_url: url,
            topic: "local-test-only".into(),
            enabled: true,
        };
        let mut l = armed();
        let message = l.observe(true, &sample(100.0), 1000).unwrap();
        assert!(send(&c, &message).is_err());
        assert!(l.observe(true, &sample(100.0), 1000).is_none());
    }

    #[test]
    fn claude_snapshot_conversion_filters_errors_stale_and_unknown() {
        let data = claude::RelayFile {
            schema_version: 1,
            received_at: 1000,
            five_hour: Some(claude::Window {
                used_percent: 100.0,
                resets_at: Some(2000),
            }),
            seven_day: None,
        };
        let mut snap = claude::compute(true, true, claude::FileState::Data(data), 1000);
        let observations = claude_observations(&snap);
        let mut ledger = Ledger::default();
        let mut below = observations[0].clone();
        below.used = Some(99.0);
        assert!(ledger.observe(true, &below, 1000).is_none());
        assert!(ledger
            .observe(true, &observations[0], 1000)
            .unwrap()
            .contains("Claude · Limite 5 heures"));
        assert!(Ledger::default()
            .observe(true, &observations[1], 1000)
            .is_none());
        for state in ["error", "stale", "waiting_for_data", "disabled"] {
            snap.state = state;
            assert!(Ledger::default()
                .observe(true, &claude_observations(&snap)[0], 1000)
                .is_none());
        }
    }

    #[test]
    fn elapsed_reset_rearms_even_when_next_reset_is_unknown() {
        let mut l = armed();
        assert!(l.observe(true, &sample(100.0), 1000).is_some());
        let mut next = sample(100.0);
        next.checked = Some(2100);
        next.reset = None;
        assert!(l.observe(true, &next, 2100).is_some());
        assert!(l.observe(true, &next, 2100).is_none());
    }

    #[test]
    fn legacy_attempts_remain_silent_and_below_threshold_state_survives_restart() {
        for reset in [serde_json::json!(2000), serde_json::Value::Null] {
            let mut ledger: Ledger =
                serde_json::from_value(serde_json::json!({"Codex:primary":reset})).unwrap();
            assert!(ledger.observe(true, &sample(100.0), 1000).is_none());
            assert!(ledger.observe(true, &sample(99.0), 1000).is_none());
            let mut ledger: Ledger =
                serde_json::from_slice(&serde_json::to_vec(&ledger).unwrap()).unwrap();
            assert!(ledger.observe(true, &sample(100.0), 1000).is_some());
            assert!(ledger.observe(true, &sample(100.0), 1000).is_none());
        }
    }

    #[test]
    fn fable_disable_restart_missing_and_reenable_never_replay_hidden_crossings() {
        let mut fable = sample(90.0);
        fable.key = "Claude:claude:weekly_fable".into();
        fable.provider = "Claude";
        fable.window = "Limite hebdomadaire Fable";
        let mut ledger = Ledger::default();
        assert!(ledger
            .observe_all_with_fable(&[fable.clone()], 1000, true)
            .is_none());
        assert!(ledger.observe_all_with_fable(&[], 1000, false).is_none());
        fable.used = Some(100.0);
        assert!(ledger
            .observe_all_with_fable(&[fable.clone()], 1000, false)
            .is_none());
        let mut ledger: Ledger =
            serde_json::from_slice(&serde_json::to_vec(&ledger).unwrap()).unwrap();
        // An invalid first sample must not consume the muted baseline.
        fable.valid = false;
        assert!(ledger
            .observe_all_with_fable(&[fable.clone()], 1000, true)
            .is_none());
        fable.valid = true;
        fable.reset = Some(3000);
        assert!(ledger
            .observe_all_with_fable(&[fable.clone()], 1000, true)
            .is_none());
        assert!(ledger
            .observe_all_with_fable(&[fable.clone()], 1000, true)
            .is_none());
        fable.used = Some(99.0);
        assert!(ledger
            .observe_all_with_fable(&[fable.clone()], 1000, true)
            .is_none());
        fable.used = Some(100.0);
        assert!(ledger
            .observe_all_with_fable(&[fable.clone()], 1000, true)
            .unwrap()
            .contains("Fable"));
        // Other quotas still cross normally while Fable is muted.
        ledger.observe_all_with_fable(&[sample(90.0)], 1000, false);
        let message = ledger
            .observe_all_with_fable(&[sample(100.0), fable.clone()], 1000, false)
            .unwrap();
        assert!(message.contains("Codex"));
        assert!(!message.contains("Fable"));
        // Hidden drops/reset changes cannot rearm an alert on enable either.
        fable.used = Some(0.0);
        ledger.observe_all_with_fable(&[fable.clone()], 1000, false);
        fable.reset = Some(4000);
        fable.used = Some(100.0);
        assert!(ledger
            .observe_all_with_fable(&[fable], 1000, true)
            .is_none());
    }

    #[test]
    fn simultaneous_windows_and_fable_are_grouped_and_independently_deduplicated() {
        let claude = |used: f64| {
            claude_observations(&claude::usage_snapshot(
                &serde_json::json!({
                    "rate_limits":{
                        "five_hour":{"utilization":used},
                        "seven_day":{"utilization":used},
                        "model_scoped":[{"display_name":"Fable","utilization":used}]
                    }
                }),
                1000,
            ))
        };
        let mut ledger = armed();
        assert!(ledger.observe_all(&claude(99.6), 1000).is_none());
        let mut rows = claude(100.0);
        rows.push(sample(100.0));
        // A repeated row in the same batch must not duplicate a line either.
        rows.push(rows[2].clone());
        let message = ledger.observe_all(&rows, 1000).unwrap();
        assert_eq!(message.lines().count(), 4);
        assert_eq!(message.matches("Fable").count(), 1);
        assert!(ledger.observe_all(&rows, 1000).is_none());
        let mut ledger: Ledger =
            serde_json::from_slice(&serde_json::to_vec(&ledger).unwrap()).unwrap();
        assert!(ledger.observe_all(&rows, 1000).is_none());
        let mut fable = rows[2].clone();
        fable.used = None;
        assert!(ledger.observe_all(&[fable.clone()], 1000).is_none());
        fable.used = Some(99.0);
        fable.valid = false;
        assert!(ledger.observe_all(&[fable.clone()], 1000).is_none());
        assert!(ledger.observe_all(&rows, 1000).is_none());
        fable.valid = true;
        assert!(ledger.observe_all(&[fable], 1000).is_none());
        let message = ledger.observe_all(&rows, 1000).unwrap();
        assert_eq!(message.lines().count(), 1);
        assert!(message.contains("Fable"));
    }

    #[test]
    fn missing_reset_does_not_forget_a_known_cycle_or_rearm_on_metadata_enrichment() {
        let mut ledger = Ledger::default();
        let mut unknown = sample(100.0);
        unknown.reset = None;
        assert!(ledger.observe(true, &unknown, 1000).is_none());
        assert!(ledger.observe(true, &sample(100.0), 1000).is_none());
        assert!(ledger.observe(true, &unknown, 1000).is_none());
        unknown.checked = Some(2100);
        assert!(ledger.observe(true, &unknown, 2100).is_some());
        assert!(ledger.observe(true, &unknown, 2100).is_none());
    }

    #[test]
    fn unresponsive_local_server_times_out() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let c = Config {
            server_url: format!("http://{}", listener.local_addr().unwrap()),
            topic: "local-timeout-only".into(),
            enabled: true,
        };
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(12)))
                .unwrap();
            let mut bytes = [0; 4096];
            while matches!(socket.read(&mut bytes), Ok(n) if n > 0) {}
        });
        let started = std::time::Instant::now();
        assert!(send(&c, "test local timeout").is_err());
        assert!(started.elapsed() < Duration::from_secs(11));
        server.join().unwrap();
    }
}
