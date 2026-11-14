//! Option A (docs/ai-profiles.md §Renouvellement) — ai-profiles sessions are
//! renewed ONLY by the provider's official CLI, run on that profile's own
//! config dir (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`). Quota Monitor never calls an
//! OAuth token endpoint, never holds a refresh token and never writes a
//! credential: it reads the current access token, and when that token is about
//! to expire it asks the official tool to do what it does on any start-up.
//!
//! Verified on the installed CLIs with synthetic homes (no model call):
//! - Claude Code 2.1.299: `initialize` + `get_usage` control requests
//!   (`claude_cli::read_usage_for`) refresh when ≤ 5 min remain.
//! - Codex 0.164.0: app-server `getAuthStatus {includeToken:true}` performs the
//!   CLI's proactive refresh (5 min window, `manager.rs`
//!   `CHATGPT_ACCESS_TOKEN_REFRESH_WINDOW_MINUTES`).
//!
//! This module is the scheduler only: margin, one run at a time per profile,
//! backoff, and a « À reconnecter » latch that is never retried until the
//! profile's connection changes (the user reconnected it via ai-profiles).

use std::{collections::HashMap, sync::Mutex};

/// Inside both CLIs' 5-minute proactive window, so the tool really refreshes.
pub const MARGIN_SECS: i64 = 240;
const BACKOFF_FIRST_SECS: i64 = 60;
const BACKOFF_MAX_SECS: i64 = 30 * 60;

/// What the official tool reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It ran and reported a usable session (verified afterwards by re-reading).
    Renewed,
    /// It ran and reported the session as gone (refresh refused / revoked).
    Revoked,
    /// Launch failure, timeout, network: transient, retried with backoff.
    Failed,
}

/// Per-profile state published with the profile (never a token).
#[derive(serde::Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Renewal {
    /// active | renewed | reconnect | retrying
    pub state: &'static str,
    /// Official tool used: `claude` or `codex`.
    pub tool: &'static str,
    /// Last successful renewal by the official tool (unix seconds).
    pub renewed_at: Option<i64>,
}

#[derive(Default)]
struct Entry {
    busy: bool,
    /// Fingerprint of the connection the official tool declared revoked.
    reconnect: Option<u64>,
    failures: u32,
    retry_at: i64,
    renewed_at: Option<i64>,
}

static ENTRIES: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);

fn with<R>(key: &str, f: impl FnOnce(&mut Entry) -> R) -> R {
    let mut guard = ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
    f(guard
        .get_or_insert_with(HashMap::new)
        .entry(key.to_string())
        .or_default())
}

fn view(e: &Entry, tool: &'static str, now: i64) -> Renewal {
    let state = if e.reconnect.is_some() {
        "reconnect"
    } else if e.retry_at > now {
        "retrying"
    } else if e.renewed_at.is_some() {
        "renewed"
    } else {
        "active"
    };
    Renewal {
        state,
        tool,
        renewed_at: e.renewed_at,
    }
}

/// Current state without running anything (credential unreadable, etc.).
pub fn status(key: &str, tool: &'static str, now: i64) -> Renewal {
    with(key, |e| view(e, tool, now))
}

/// Connection as read now: `None` when no token can be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    pub expires_at: Option<i64>,
    pub fingerprint: u64,
}

pub fn fingerprint(token: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut h);
    h.finish()
}

fn due(seen: &Seen, now: i64) -> bool {
    // No expiry recorded: nothing to anticipate (a 401 is reported by the read).
    seen.expires_at.is_some_and(|at| at - now <= MARGIN_SECS)
}

/// Renew `key` through `run` (the official tool) when its token is within the
/// margin, then verify with `reload`. Never two runs at once for one key,
/// never again after a revocation until the connection changes, backoff after
/// a failure. Holds no lock while `run` executes.
pub fn ensure(
    key: &str,
    tool: &'static str,
    seen: Option<Seen>,
    now: i64,
    run: impl FnOnce() -> Outcome,
    reload: impl FnOnce() -> Option<Seen>,
) -> Renewal {
    let Some(seen) = seen else {
        return status(key, tool, now);
    };
    let go = with(key, |e| {
        if e.reconnect.is_some_and(|fp| fp != seen.fingerprint) {
            // Reconnected through ai-profiles since: start afresh.
            e.reconnect = None;
            e.failures = 0;
            e.retry_at = 0;
            // This token comes from the user's reconnection, not the tool.
            e.renewed_at = None;
        }
        let go = due(&seen, now) && !e.busy && e.reconnect.is_none() && e.retry_at <= now;
        if go {
            e.busy = true;
        }
        go
    });
    if !go {
        return status(key, tool, now);
    }
    struct Release<'a>(&'a str);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            with(self.0, |e| e.busy = false);
        }
    }
    let _release = Release(key);
    let outcome = match run() {
        // Trust the file/Keychain, not the tool's answer: a tool that could not
        // reach the provider still reports the old token. Renewed = a NEW
        // connection that is no longer within the margin.
        Outcome::Renewed => match reload() {
            Some(after) if after.fingerprint != seen.fingerprint && !due(&after, now) => {
                Outcome::Renewed
            }
            None => Outcome::Revoked,
            Some(_) => Outcome::Failed,
        },
        other => other,
    };
    with(key, |e| {
        match outcome {
            Outcome::Renewed => {
                e.failures = 0;
                e.retry_at = 0;
                e.renewed_at = Some(now);
            }
            Outcome::Revoked => e.reconnect = Some(seen.fingerprint),
            Outcome::Failed => {
                e.failures = (e.failures + 1).min(10);
                let wait = BACKOFF_FIRST_SECS << (e.failures - 1).min(5);
                e.retry_at = now + wait.min(BACKOFF_MAX_SECS);
            }
        }
        view(e, tool, now)
    })
}

#[cfg(test)]
pub(crate) fn forget(key: &str) {
    if let Some(map) = ENTRIES.lock().unwrap().as_mut() {
        map.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn seen(exp: i64, fp: u64) -> Option<Seen> {
        Some(Seen {
            expires_at: Some(exp),
            fingerprint: fp,
        })
    }

    #[test]
    fn far_from_expiry_runs_nothing() {
        forget("t-far");
        let runs = Cell::new(0);
        let r = ensure(
            "t-far",
            "claude",
            seen(10_000, 1),
            0,
            || {
                runs.set(1);
                Outcome::Renewed
            },
            || None,
        );
        assert_eq!(runs.get(), 0);
        assert_eq!(r.state, "active");
    }

    #[test]
    fn dormant_profile_is_renewed_on_every_expiry_over_several_cycles() {
        forget("t-dormant");
        let runs = Cell::new(0);
        let mut exp = 1_000;
        for cycle in 0..5 {
            let now = exp - 100; // each cycle lands inside the margin
            let next = now + 8 * 3600;
            let r = ensure(
                "t-dormant",
                "codex",
                seen(exp, cycle),
                now,
                || {
                    runs.set(runs.get() + 1);
                    Outcome::Renewed
                },
                || seen(next, cycle + 1),
            );
            assert_eq!(r.state, "renewed");
            assert_eq!(r.renewed_at, Some(now));
            exp = next;
        }
        assert_eq!(runs.get(), 5);
    }

    #[test]
    fn revoked_latches_reconnect_without_loop_until_connection_changes() {
        forget("t-rev");
        let runs = Cell::new(0);
        let run = || {
            runs.set(runs.get() + 1);
            Outcome::Revoked
        };
        assert_eq!(
            ensure("t-rev", "claude", seen(0, 7), 100, run, || None).state,
            "reconnect"
        );
        for now in [200, 5_000, 90_000] {
            let r = ensure(
                "t-rev",
                "claude",
                seen(0, 7),
                now,
                || {
                    runs.set(runs.get() + 1);
                    Outcome::Revoked
                },
                || None,
            );
            assert_eq!(r.state, "reconnect");
        }
        assert_eq!(runs.get(), 1, "never re-run against a revoked connection");
        // Credential cleared by the CLI: still « À reconnecter ».
        assert_eq!(
            ensure(
                "t-rev",
                "claude",
                None,
                100_000,
                || Outcome::Renewed,
                || None
            )
            .state,
            "reconnect"
        );
        // User reconnected via ai-profiles: new fingerprint, fresh token.
        assert_eq!(
            ensure(
                "t-rev",
                "claude",
                seen(200_000, 8),
                100_001,
                || Outcome::Renewed,
                || None
            )
            .state,
            "active"
        );
    }

    #[test]
    fn tool_success_without_fresh_token_is_a_failure_with_backoff() {
        forget("t-down");
        let runs = Cell::new(0);
        let r = ensure(
            "t-down",
            "codex",
            seen(50, 3),
            0,
            || {
                runs.set(1);
                Outcome::Renewed
            },
            || seen(50, 3),
        );
        assert_eq!(r.state, "retrying");
        // Within backoff: no run.
        let r = ensure(
            "t-down",
            "codex",
            seen(50, 3),
            30,
            || {
                runs.set(2);
                Outcome::Renewed
            },
            || seen(9_999, 4),
        );
        assert_eq!((r.state, runs.get()), ("retrying", 1));
        // After the 60 s backoff: retried, then a doubled backoff on failure.
        ensure(
            "t-down",
            "codex",
            seen(50, 3),
            61,
            || Outcome::Failed,
            || None,
        );
        assert_eq!(with("t-down", |e| e.retry_at), 61 + 120);
    }

    #[test]
    fn never_two_runs_at_once_for_one_profile() {
        forget("t-busy");
        let inner = Cell::new(None);
        ensure(
            "t-busy",
            "claude",
            seen(0, 1),
            10,
            || {
                // A second poll while the first renewal is still running.
                let ran = Cell::new(false);
                let r = ensure(
                    "t-busy",
                    "claude",
                    seen(0, 1),
                    10,
                    || {
                        ran.set(true);
                        Outcome::Renewed
                    },
                    || None,
                );
                inner.set(Some((ran.get(), r.state)));
                Outcome::Failed
            },
            || None,
        );
        assert_eq!(inner.get(), Some((false, "active")));
        assert!(!with("t-busy", |e| e.busy), "released after the run");
    }

    #[test]
    fn no_expiry_recorded_never_launches_the_tool() {
        forget("t-none");
        let r = ensure(
            "t-none",
            "claude",
            Some(Seen {
                expires_at: None,
                fingerprint: 1,
            }),
            0,
            || panic!("must not run"),
            || None,
        );
        assert_eq!(r.state, "active");
    }
}
