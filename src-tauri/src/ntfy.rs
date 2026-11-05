//! Opt-in, best-effort ntfy delivery. A single bounded worker owns the persistent
//! attempt ledger; quota collection never waits for disk or network delivery.
use crate::{claude, codex, prefs};
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
    provider: String,
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
        out.extend(codex_account_observations(&codex::accounts(app)));
    }
    if p.claude_enabled {
        out.extend(claude_profile_observations(&claude::raw_accounts(app)));
    }
    out
}

/// Every followed Codex environment (visibility is display only), stable key
/// `Codex:<environment id>:<period>`. The account name appears only when
/// several environments are followed; a single one keeps the v3.1.0 text.
/// A home returning the same identity as an earlier one is skipped, so one
/// account never notifies twice.
fn codex_account_observations(accounts: &[codex::CodexAccount]) -> Vec<Observation> {
    let named = accounts.iter().filter(|a| a.tracked).count() > 1;
    accounts
        .iter()
        .filter(|a| a.tracked && a.duplicate_of.is_none())
        .flat_map(|a| {
            let ready = a.phase == "ready";
            a.raw_rows.iter().map(move |row| Observation {
                key: format!("Codex:{}:{}", a.profile.id, row.period),
                provider: if named {
                    format!("Codex · {}", a.profile.name)
                } else {
                    "Codex".into()
                },
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
            })
        })
        .collect()
}

/// Every profile is observed (visibility is display-only) with a stable key
/// `Claude:<profile id>:<metric>`. The account name appears in the message
/// only when several profiles exist; a single profile keeps the v3.1.0 text.
fn claude_profile_observations(accounts: &[claude::AccountSnapshot]) -> Vec<Observation> {
    let named = accounts.len() > 1;
    accounts
        .iter()
        .flat_map(|a| {
            claude_account_observations(
                &a.snapshot,
                Some(a.profile.id.as_str()),
                if named { a.profile.name.as_str() } else { "" },
            )
        })
        .collect()
}

fn is_fable_key(key: &str) -> bool {
    key.starts_with("Claude:") && key.ends_with(":claude:weekly_fable")
}

#[cfg(test)]
fn claude_observations(snap: &claude::ClaudeSnapshot) -> Vec<Observation> {
    claude_account_observations(snap, None, "")
}
fn claude_account_observations(
    snap: &claude::ClaudeSnapshot,
    id: Option<&str>,
    name: &str,
) -> Vec<Observation> {
    snap.metrics
        .iter()
        .map(|metric| Observation {
            key: id
                .map(|id| format!("Claude:{id}:{}", metric.id))
                .unwrap_or_else(|| format!("Claude:{}", metric.id)),
            provider: if name.is_empty() {
                "Claude".into()
            } else {
                format!("Claude · {name}")
            },
            // v3.1.0 wording: the metric label ("Limite 5 heures", Fable…).
            window: metric.label,
            used: metric.used_percent,
            checked: snap.observed_at.as_deref().and_then(timestamp),
            reset: metric.reset_at.as_deref().and_then(timestamp),
            valid: snap.enabled && snap.state == "ready" && metric.availability == "available",
        })
        .collect()
}

// Raw used == 0 means genuinely full, not a rounded remaining percentage.
// Old exhaustion-policy entries establish a silent baseline on first fresh data.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Counter {
    Refill {
        reset: Option<i64>,
        full: bool,
        attempted: bool,
        checked: i64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        muted: bool,
    },
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
    /// Bind v3.1.0 unqualified Claude entries once to the startup primary
    /// profile. Later primaries use their own identity and cannot inherit them.
    fn migrate_primary(&mut self, id: &str) -> bool {
        let mut changed = false;
        for metric in [
            "claude:five_hour",
            "claude:seven_day",
            "claude:weekly_fable",
        ] {
            if let Some(value) = self.0.remove(&format!("Claude:{metric}")) {
                self.0
                    .entry(format!("Claude:{id}:{metric}"))
                    .or_insert(value);
                changed = true;
            }
        }
        changed
    }
    /// Bind historical unqualified Codex entries (`Codex:<period>`) once to
    /// the startup primary environment. Qualified keys are left untouched.
    fn migrate_codex_primary(&mut self, id: &str) -> bool {
        let legacy: Vec<String> = self
            .0
            .keys()
            .filter(|k| k.starts_with("Codex:") && !k.starts_with("Codex:profile:"))
            .cloned()
            .collect();
        for key in &legacy {
            if let Some(value) = self.0.remove(key) {
                self.0
                    .entry(format!("Codex:{id}:{}", &key["Codex:".len()..]))
                    .or_insert(value);
            }
        }
        !legacy.is_empty()
    }
    fn observe(&mut self, enabled: bool, o: &Observation, now: i64) -> Option<String> {
        if !enabled
            || !o.valid
            || !o.checked.is_some_and(|at| (0..=120).contains(&(now - at)))
            || o.reset.is_some_and(|at| at <= now)
        {
            return None;
        }
        let used = o
            .used
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))?;
        let checked = o.checked?;
        let previous = match self.0.get(&o.key) {
            Some(Counter::Refill {
                reset,
                full,
                attempted,
                checked: at,
                muted,
            }) => {
                if checked < *at {
                    return None;
                }
                Some((*reset, *full, *attempted, *muted))
            }
            // Do not interpret an old exhaustion flag as a refill observation.
            _ => None,
        };
        let full = used == 0.0;
        let (prior_reset, was_full, attempted, was_muted) =
            previous.unwrap_or((None, full, false, false));
        // A changed future date alone is not proof of a reset. With a known
        // window, latch the attempt until that window actually elapses. Without
        // reset metadata the only evidence available is a new nonfull -> full edge.
        let rearmed = prior_reset.is_some_and(|at| checked >= at)
            || (prior_reset.is_none() && !full)
            || was_muted;
        let attempted = attempted && !rearmed;
        let notify = previous.is_some() && !was_muted && !was_full && full && !attempted;
        let reset = o.reset.or_else(|| prior_reset.filter(|at| *at > now));
        self.0.insert(
            o.key.clone(),
            Counter::Refill {
                reset,
                full,
                attempted: attempted || notify,
                checked,
                muted: false,
            },
        );
        if !notify {
            return None;
        }
        Some(format!(
            "{} · {} : quota reconstitué (100 % restant).",
            o.provider, o.window
        ))
    }
    #[cfg(test)]
    fn observe_all(&mut self, observations: &[Observation], now: i64) -> Option<String> {
        self.observe_all_with_fable(observations, now, true)
    }
    fn mute_fable(&mut self) {
        // Every profile's Fable counter (and a not-yet-migrated legacy key).
        for (_, counter) in self.0.iter_mut().filter(|(key, _)| is_fable_key(key)) {
            let reset = match counter {
                Counter::Refill { reset, .. }
                | Counter::Observed { reset, .. }
                | Counter::Legacy(reset) => *reset,
            };
            let checked = match counter {
                Counter::Refill { checked, .. } => *checked,
                _ => 0,
            };
            *counter = Counter::Refill {
                reset,
                full: true,
                attempted: false,
                checked,
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
                if !fable_enabled && is_fable_key(&o.key) {
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
        if let Some(primary) = claude::accounts(&app).into_iter().find(|a| a.primary) {
            if ledger.migrate_primary(&primary.profile.id) && ledger.save(&path).is_err() {
                eprintln!("Notifications ntfy suspendues : migration non enregistrable.");
                return;
            }
        }
        let codex_primary = codex::discovered(&app, &prefs::current(&app))
            .into_iter()
            .find(|a| a.primary);
        if let Some(primary) = codex_primary {
            if ledger.migrate_codex_primary(&primary.profile.id) && ledger.save(&path).is_err() {
                eprintln!("Notifications ntfy suspendues : migration non enregistrable.");
                return;
            }
        }
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
                // per refresh includes all newly replenished windows/models.
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
            provider: "Codex".into(),
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
    fn initial_full_is_silent_including_polls_restart_and_missing_values() {
        let mut ledger = Ledger::default();
        assert!(ledger.observe(true, &sample(0.0), 1000).is_none());
        let bytes = serde_json::to_vec(&ledger).unwrap();
        let mut ledger: Ledger = serde_json::from_slice(&bytes).unwrap();
        for _ in 0..20 {
            assert!(ledger.observe(true, &sample(0.0), 1000).is_none());
        }
        let mut missing = sample(0.0);
        missing.used = None;
        assert!(ledger.observe(true, &missing, 1000).is_none());
        missing.used = Some(0.0);
        missing.valid = false;
        assert!(ledger.observe(true, &missing, 1000).is_none());
        assert!(ledger.observe(true, &sample(0.0), 1000).is_none());
        assert!(ledger.observe(true, &sample(99.0), 1000).is_none());
        assert!(ledger.observe(true, &sample(0.0), 1000).is_some());
    }
    fn account(id: &str, name: &str, primary: bool, used: f64) -> claude::AccountSnapshot {
        claude::AccountSnapshot {
            profile: crate::claude_accounts::Profile {
                id: id.into(),
                path: format!("/fixture/{id}"),
                name: name.into(),
                visible: true,
            },
            primary,
            available: true,
            refreshing: false,
            snapshot: claude::usage_snapshot(
                &serde_json::json!({"rate_limits":{
                    "five_hour":{"utilization":used},
                    "seven_day":{"utilization":used},
                    "model_scoped":[{"display_name":"Fable","utilization":used}]
                }}),
                1000,
            ),
        }
    }
    #[test]
    fn refill_policy_applies_per_profile_with_stable_keys_and_account_labels() {
        let rows = |a: f64, b: f64, name_a: &str| {
            claude_profile_observations(&[
                account("profile-a", name_a, true, a),
                account("profile-b", "Travail", false, b),
            ])
        };
        let first = rows(50.0, 50.0, "Personnel");
        assert_eq!(first.len(), 6);
        assert_eq!(first[0].key, "Claude:profile-a:claude:five_hour");
        assert_eq!(first[5].key, "Claude:profile-b:claude:weekly_fable");
        let mut ledger = Ledger::default();
        assert!(ledger.observe_all(&first, 1000).is_none());
        // Exhaustion (raw used 100 %) is never an alert under the v3.1.0 policy.
        assert!(ledger
            .observe_all(&rows(50.0, 100.0, "Personnel"), 1000)
            .is_none());
        // Only profile B returns to 100 % remaining: one line per B metric.
        let message = ledger
            .observe_all(&rows(50.0, 0.0, "Personnel"), 1000)
            .unwrap();
        assert_eq!(message.lines().count(), 3);
        assert!(message
            .contains("Claude · Travail · Limite 5 heures : quota reconstitué (100 % restant)."));
        assert!(
            message.contains("Claude · Travail · Limite Fable")
                || message.contains("Claude · Travail · Limite hebdomadaire Fable")
        );
        assert!(!message.contains("Personnel"));
        // Once per cycle, across restart, and a rename never replays.
        let mut ledger: Ledger =
            serde_json::from_slice(&serde_json::to_vec(&ledger).unwrap()).unwrap();
        assert!(ledger
            .observe_all(&rows(50.0, 0.0, "Renommé"), 1000)
            .is_none());
        let message = ledger
            .observe_all(&rows(0.0, 0.0, "Renommé"), 1000)
            .unwrap();
        assert_eq!(message.lines().count(), 3);
        assert!(message.contains("Claude · Renommé · Limite 5 heures"));
    }
    #[test]
    fn single_profile_keeps_the_exact_v310_message() {
        let rows =
            |used| claude_profile_observations(&[account("profile-a", "Principal", true, used)]);
        let mut ledger = Ledger::default();
        assert!(ledger.observe_all(&rows(40.0), 1000).is_none());
        let message = ledger.observe_all(&rows(0.0), 1000).unwrap();
        assert!(message
            .lines()
            .any(|l| l == "Claude · Limite 5 heures : quota reconstitué (100 % restant)."));
        assert!(!message.contains("Principal"));
        assert!(ledger.0.contains_key("Claude:profile-a:claude:five_hour"));
    }
    #[test]
    fn v310_legacy_keys_migrate_once_to_the_primary_without_replay() {
        let mut ledger: Ledger = serde_json::from_value(serde_json::json!({
            "Codex:primary": {"reset":null,"full":true,"attempted":true,"checked":900},
            "Claude:claude:five_hour": {"reset":null,"full":true,"attempted":true,"checked":900},
            "Claude:claude:seven_day": {"reset":null,"full":false,"attempted":false,"checked":900},
            "Claude:claude:weekly_fable": {"reset":null,"full":true,"attempted":false,"checked":900,"muted":true}
        }))
        .unwrap();
        assert!(ledger.migrate_primary("profile-a"));
        assert!(!ledger.migrate_primary("profile-b"));
        assert!(ledger.0.contains_key("Codex:primary"));
        for metric in ["five_hour", "seven_day", "weekly_fable"] {
            assert!(!ledger.0.contains_key(&format!("Claude:claude:{metric}")));
            assert!(ledger
                .0
                .contains_key(&format!("Claude:profile-a:claude:{metric}")));
            assert!(!ledger
                .0
                .contains_key(&format!("Claude:profile-b:claude:{metric}")));
        }
        // Already full and attempted before migration: no duplicate on restart.
        let primary = claude_profile_observations(&[account("profile-a", "Principal", true, 0.0)]);
        let message = ledger.observe_all(&primary, 1000).unwrap_or_default();
        assert!(!message.contains("Limite 5 heures"), "{message}");
        // The non-full weekly window still alerts on its own refill edge.
        assert!(message.contains("Limite hebdomadaire"), "{message}");
        assert!(
            !message.contains("Fable"),
            "muted Fable baseline never replays"
        );
    }
    #[test]
    fn fable_opt_out_mutes_every_profile_but_not_other_quotas() {
        let rows = |used| {
            claude_profile_observations(&[
                account("profile-a", "Personnel", true, used),
                account("profile-b", "Travail", false, used),
            ])
        };
        let mut ledger = Ledger::default();
        assert!(ledger
            .observe_all_with_fable(&rows(50.0), 1000, true)
            .is_none());
        let message = ledger
            .observe_all_with_fable(&rows(0.0), 1000, false)
            .unwrap();
        assert_eq!(message.lines().count(), 4);
        assert!(!message.contains("Fable"));
        for id in ["profile-a", "profile-b"] {
            assert!(matches!(
                ledger.0.get(&format!("Claude:{id}:claude:weekly_fable")),
                Some(Counter::Refill { muted: true, .. })
            ));
        }
        // Re-enabling does not replay the hidden crossing for either profile.
        assert!(ledger
            .observe_all_with_fable(&rows(0.0), 1000, true)
            .is_none());
    }
    #[test]
    fn exact_threshold_direction_duplicates_restart_and_rearm() {
        let mut l = Ledger::default();
        for used in [50.0, 80.0, 90.0, 99.6, 99.99, 100.0, 0.4, 0.00001] {
            assert!(l.observe(true, &sample(used), 1000).is_none());
        }
        assert!(l.observe(true, &sample(0.0), 1000).is_some());
        let bytes = serde_json::to_vec(&l).unwrap();
        let mut l: Ledger = serde_json::from_slice(&bytes).unwrap();
        assert!(l.observe(true, &sample(0.0), 1000).is_none());
        assert!(l.observe(true, &sample(0.1), 1000).is_none());
        assert!(l.observe(true, &sample(0.0), 1000).is_none());
        let mut next = sample(0.0);
        next.reset = Some(3000);
        assert!(l.observe(true, &next, 1000).is_none());
        next.key = "Claude:five_hour".into();
        next.provider = "Claude".into();
        assert!(l.observe(true, &next, 1000).is_none());
        next.used = Some(99.0);
        assert!(l.observe(true, &next, 1000).is_none());
        next.used = Some(0.0);
        assert!(l.observe(true, &next, 1000).unwrap().contains("Claude"));
        next.reset = None;
        assert!(l.observe(true, &next, 1000).is_none());
        next.key = "Claude:seven_day".into();
        assert!(l.observe(true, &next, 1000).is_none());
        next.used = Some(99.0);
        assert!(l.observe(true, &next, 1000).is_none());
        next.used = Some(0.0);
        assert!(l.observe(true, &next, 1000).is_some());
        next.reset = Some(3000);
        assert!(l.observe(true, &next, 1000).is_none());
    }
    #[test]
    fn invalid_disabled_stale_and_errors_never_send_or_rearm() {
        let mut l = Ledger::default();
        assert!(l.observe(false, &sample(100.0), 1000).is_none());
        for used in [f64::NAN, f64::INFINITY, -1.0, 100.1] {
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
        assert!(l.observe(true, &sample(0.0), 1000).is_some());
        invalid.used = Some(0.0);
        assert!(l.observe(true, &invalid, 1000).is_none());
        assert!(l.observe(true, &sample(100.0), 1000).is_none());
    }
    #[test]
    fn codex_raw_percentage_not_rounded_display_drives_threshold() {
        for (used, expected) in [
            (0.0, true),
            (0.4, false),
            (0.00001, false),
            (99.6, false),
            (100.0, false),
        ] {
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
        assert!(ledger.observe(true, &sample(0.0), 1000).is_some());
        ledger.save(&path).unwrap();
        let mut restored: Ledger = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(restored.observe(true, &sample(100.0), 1000).is_none());
        assert!(restored.observe(true, &sample(0.0), 1000).is_none());
        restored.save(&path).unwrap();
        let mut restored: Ledger = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(restored.observe(true, &sample(0.0), 1000).is_none());
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
        let message = armed().observe(true, &sample(0.0), 1000).unwrap();
        let result = send(&config, &message);
        (result, server.join().unwrap())
    }
    #[test]
    fn real_local_http_post_endpoint_minimal_payload_and_status_errors() {
        let (result, request) = local_post(200);
        result.unwrap();
        assert!(request.starts_with("POST /base/local-test-only HTTP/1.1"));
        assert!(request.contains("Quota Monitor"));
        assert!(request.contains("Codex · 5 heures : quota reconstitué (100 % restant)."));
        assert!(!request.contains("consommé"));
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
        let message = l.observe(true, &sample(0.0), 1000).unwrap();
        assert!(send(&c, &message).is_err());
        assert!(l.observe(true, &sample(100.0), 1000).is_none());
    }

    #[test]
    fn claude_snapshot_conversion_filters_errors_stale_and_unknown() {
        let data = claude::RelayFile {
            schema_version: 1,
            received_at: 1000,
            five_hour: Some(claude::Window {
                used_percent: 0.0,
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
    fn elapsed_reset_alone_never_notifies_and_next_full_transition_rearms() {
        let mut l = armed();
        assert!(l.observe(true, &sample(0.0), 1000).is_some());
        let mut next = sample(100.0);
        next.checked = Some(2100);
        next.reset = None;
        assert!(l.observe(true, &next, 2100).is_none());
        next.used = Some(0.0);
        assert!(l.observe(true, &next, 2100).is_some());
        assert!(l.observe(true, &next, 2100).is_none());
    }

    #[test]
    fn legacy_policy_migrates_silently_and_nonfull_state_survives_restart() {
        for reset in [
            serde_json::json!(2000),
            serde_json::Value::Null,
            serde_json::json!({"reset":2000,"exhausted":true}),
            serde_json::json!({"reset":2000,"exhausted":false,"muted":true}),
        ] {
            let mut ledger: Ledger =
                serde_json::from_value(serde_json::json!({"Codex:primary":reset})).unwrap();
            assert!(ledger.observe(true, &sample(0.0), 1000).is_none());
            assert!(ledger.observe(true, &sample(99.0), 1000).is_none());
            let mut ledger: Ledger =
                serde_json::from_slice(&serde_json::to_vec(&ledger).unwrap()).unwrap();
            assert!(ledger.observe(true, &sample(0.0), 1000).is_some());
            assert!(ledger.observe(true, &sample(0.0), 1000).is_none());
        }
    }

    #[test]
    fn fable_disable_restart_missing_and_reenable_never_replay_hidden_crossings() {
        let mut fable = sample(90.0);
        fable.key = "Claude:claude:weekly_fable".into();
        fable.provider = "Claude".into();
        fable.window = "Limite hebdomadaire Fable";
        let mut ledger = Ledger::default();
        assert!(ledger
            .observe_all_with_fable(&[fable.clone()], 1000, true)
            .is_none());
        assert!(ledger.observe_all_with_fable(&[], 1000, false).is_none());
        fable.used = Some(0.0);
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
        fable.used = Some(0.0);
        assert!(ledger
            .observe_all_with_fable(&[fable.clone()], 1000, true)
            .unwrap()
            .contains("Fable"));
        // Other quotas still cross normally while Fable is muted.
        ledger.observe_all_with_fable(&[sample(90.0)], 1000, false);
        let message = ledger
            .observe_all_with_fable(&[sample(0.0), fable.clone()], 1000, false)
            .unwrap();
        assert!(message.contains("Codex"));
        assert!(!message.contains("Fable"));
        // Hidden drops/reset changes cannot rearm an alert on enable either.
        fable.used = Some(99.0);
        ledger.observe_all_with_fable(&[fable.clone()], 1000, false);
        fable.reset = Some(4000);
        fable.used = Some(0.0);
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
        let mut rows = claude(0.0);
        rows.push(sample(0.0));
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
        let mut unknown = sample(0.0);
        unknown.reset = None;
        assert!(ledger.observe(true, &unknown, 1000).is_none());
        assert!(ledger.observe(true, &sample(0.0), 1000).is_none());
        assert!(ledger.observe(true, &unknown, 1000).is_none());
        unknown.checked = Some(2100);
        assert!(ledger.observe(true, &unknown, 2100).is_none());
        assert!(ledger.observe(true, &unknown, 2100).is_none());
    }

    #[test]
    fn multiple_known_cycles_require_nonfull_to_full_not_just_a_reset_date() {
        let mut ledger = armed();
        assert!(ledger.observe(true, &sample(0.0), 1000).is_some());
        // Three subsequent cycles, with persistence between consumption and refill.
        for now in [2100, 3100, 4100] {
            let mut row = sample(0.0);
            row.checked = Some(now);
            row.reset = Some(now + 900);
            assert!(ledger.observe(true, &row, now).is_none());
            row.used = Some(80.0);
            assert!(ledger.observe(true, &row, now).is_none());
            ledger = serde_json::from_slice(&serde_json::to_vec(&ledger).unwrap()).unwrap();
            row.used = Some(0.4); // 99.6% remaining may display 100, but is NOT full.
            assert!(ledger.observe(true, &row, now).is_none());
            row.used = Some(0.0);
            assert!(ledger.observe(true, &row, now).is_some());
            assert!(ledger.observe(true, &row, now).is_none());
        }
    }

    #[test]
    fn unknown_resets_use_exact_edges_and_old_samples_cannot_rearm() {
        let mut ledger = Ledger::default();
        let mut row = sample(50.0);
        row.reset = None;
        assert!(ledger.observe(true, &row, 1000).is_none());
        for now in [1010, 1020, 1030] {
            row.checked = Some(now);
            row.used = Some(0.0);
            assert!(ledger.observe(true, &row, now).is_some());
            assert!(ledger.observe(true, &row, now).is_none());
            let mut old = row.clone();
            old.checked = Some(now - 1);
            old.used = Some(50.0);
            assert!(ledger.observe(true, &old, now).is_none());
            assert!(ledger.observe(true, &row, now).is_none());
            row.used = Some(50.0);
            assert!(ledger.observe(true, &row, now).is_none());
        }
    }

    #[test]
    fn legacy_nonfull_or_full_first_sample_never_replays_the_old_policy() {
        for full in [true, false] {
            for exhausted in [true, false] {
                let mut ledger: Ledger = serde_json::from_value(serde_json::json!({
                    "Codex:primary":{"reset":2000,"exhausted":exhausted}
                }))
                .unwrap();
                let used = if full { 0.0 } else { 50.0 };
                assert!(ledger.observe(true, &sample(used), 1000).is_none());
                if full {
                    assert!(ledger.observe(true, &sample(50.0), 1000).is_none());
                }
                assert!(ledger.observe(true, &sample(0.0), 1000).is_some());
            }
        }
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

    fn codex_account(
        name: &str,
        primary: bool,
        used5: f64,
        identity: Option<u64>,
    ) -> codex::CodexAccount {
        let found = crate::codex_accounts::Discovered {
            profile: crate::claude_accounts::Profile {
                id: format!("profile:{name}"),
                path: format!("/fixture/{name}"),
                name: name.into(),
                visible: true,
            },
            primary,
            tracked: true,
            launch: (!primary).then(|| format!("/fixture/{name}").into()),
            available: true,
            visible: true,
            blocker: None,
            credential: None,
        };
        let checked = chrono::DateTime::from_timestamp(1000, 0)
            .unwrap()
            .to_rfc3339();
        let mut rows = codex::normalize(&serde_json::json!({"rateLimits":{
            "primary":{"usedPercent":used5,"windowDurationMins":300,"resetsAt":2000}
        }}))
        .unwrap();
        for row in &mut rows {
            row.checked_at = checked.clone();
        }
        let reading = codex::Reading {
            phase: "ready".into(),
            rows: Some(rows.clone()),
            identity,
            ..Default::default()
        };
        let primary_view = codex::PrimaryView {
            status: codex::Status {
                phase: "ready".into(),
                ..codex::Status::default()
            },
            rows,
            identity,
        };
        codex::account_view(
            found,
            true,
            Some(&primary_view),
            (!primary).then_some((reading, false)),
        )
    }

    #[test]
    fn codex_accounts_notify_separately_without_duplicates_and_legacy_keys_migrate_once() {
        // One environment: v3.1.0 wording, key qualified by the environment id.
        let single = codex_account_observations(&[codex_account("Principal", true, 50.0, None)]);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].provider, "Codex");
        assert_eq!(single[0].key, "Codex:profile:Principal:codex:codex:primary");
        // Several: per-account keys and names; same identity notifies once.
        let mut accounts = vec![
            codex_account("Principal", true, 50.0, Some(7)),
            codex_account("Travail", false, 50.0, Some(8)),
            codex_account("Alias", false, 50.0, Some(7)),
        ];
        codex::mark_duplicates(&mut accounts);
        let rows = codex_account_observations(&accounts);
        let keys: Vec<_> = rows.iter().map(|o| o.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "Codex:profile:Principal:codex:codex:primary",
                "Codex:profile:Travail:codex:codex:primary"
            ]
        );
        assert_eq!(rows[1].provider, "Codex · Travail");
        let mut ledger = Ledger::default();
        assert!(ledger.observe_all(&rows, 1000).is_none()); // silent baseline
        let refilled: Vec<_> = vec![
            codex_account("Principal", true, 0.0, Some(7)),
            codex_account("Travail", false, 50.0, Some(8)),
            codex_account("Alias", false, 0.0, Some(7)),
        ];
        let mut refilled = refilled;
        codex::mark_duplicates(&mut refilled);
        let message = ledger
            .observe_all(&codex_account_observations(&refilled), 1000)
            .unwrap();
        assert_eq!(
            message,
            "Codex · Principal · 5 heures : quota reconstitué (100 % restant)."
        );
        // Legacy unqualified Codex entries move once to the primary id.
        let mut legacy: Ledger = serde_json::from_value(serde_json::json!({
            "Codex:codex:codex:primary": {"reset":null,"full":true,"attempted":true,"checked":900},
            "Claude:claude:five_hour": {"reset":null,"full":true,"attempted":false,"checked":900}
        }))
        .unwrap();
        assert!(legacy.migrate_codex_primary("profile:Principal"));
        assert!(legacy
            .0
            .contains_key("Codex:profile:Principal:codex:codex:primary"));
        assert!(!legacy.0.contains_key("Codex:codex:codex:primary"));
        assert!(
            legacy.0.contains_key("Claude:claude:five_hour"),
            "Claude keys untouched"
        );
        assert!(!legacy.migrate_codex_primary("profile:Other"), "only once");
    }
}
