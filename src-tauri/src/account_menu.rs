//! Connection state of an account (Connecté / Non connecté / …) shown on its
//! card, and the providers offering a « + ». The native menu has no account
//! entries below its panel: accounts are managed in Réglages only.
use crate::{claude, codex, prefs, Tone};

/// Connection state of a Claude profile (same wording as Settings and the Dashboard).
pub fn claude_state(account: &claude::AccountSnapshot) -> (String, Tone) {
    let s = &account.snapshot;
    let (label, tone) = if !account.available {
        ("Indisponible", Tone::Neutral)
    } else if account.refreshing {
        ("Actualisation…", Tone::Warning)
    } else if s.state == "disabled" {
        ("Désactivé", Tone::Neutral)
    } else if s.error_kind == Some("not_signed_in") {
        ("Non connecté", Tone::Warning)
    } else if s.error_kind == Some("session_expired") {
        // aip snapshot whose saved token no longer works: actionable, no quota.
        ("Session expirée", Tone::Warning)
    } else if s.error_kind == Some("profile_blocked") {
        // Detected (incomplete aip profile) but never read by design: not an error.
        ("Non collectable", Tone::Neutral)
    } else {
        match s.state {
            "ready" => ("Connecté", Tone::Good),
            "stale" => ("Connecté", Tone::Warning),
            "waiting_for_data" => ("Lecture…", Tone::Warning),
            "not_configured" => ("Non installé", Tone::Neutral),
            _ => ("Erreur", Tone::Error),
        }
    };
    (label.into(), tone)
}

/// Connection state of a Codex home.
pub fn codex_state(account: &codex::CodexAccount) -> (String, Tone) {
    let (label, tone) = if !account.available {
        ("Indisponible", Tone::Neutral)
    } else if account.refreshing {
        ("Actualisation…", Tone::Warning)
    } else if account.duplicate_of.is_some() && account.phase == "ready" {
        ("Même compte", Tone::Warning)
    } else {
        match account.phase.as_str() {
            "ready" => ("Connecté", Tone::Good),
            "signed_out" => ("Non connecté", Tone::Warning),
            "disabled" => ("Désactivé", Tone::Neutral),
            "missing" => ("Non installé", Tone::Neutral),
            "error" => ("Erreur", Tone::Error),
            // aip home whose saved token no longer works: actionable, no quota.
            "expired" => ("Session expirée", Tone::Warning),
            // Incomplete aip home, never read by design: not an error.
            "blocked" => ("Non collectable", Tone::Neutral),
            _ => ("Connexion…", Tone::Warning),
        }
    };
    (label.into(), tone)
}

/// Providers offering the « + » of the compact window: the enabled ones,
/// Codex first.
pub fn addable_providers(prefs: &prefs::Prefs) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    if prefs.codex_enabled {
        kinds.push("codex");
    }
    if prefs.claude_enabled {
        kinds.push("claude");
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_enabled_providers_offer_the_plus_codex_first() {
        let mut prefs = prefs::Prefs {
            claude_enabled: true,
            ..Default::default()
        };
        assert_eq!(addable_providers(&prefs), ["codex", "claude"]);
        prefs.claude_enabled = false;
        assert_eq!(addable_providers(&prefs), ["codex"]);
        prefs.codex_enabled = false;
        assert!(addable_providers(&prefs).is_empty());
    }

    #[test]
    fn a_blocked_aip_snapshot_reads_as_non_collectable_not_as_an_error() {
        let mut snapshot = claude::compute(true, true, claude::FileState::Missing, 0);
        snapshot.state = "error";
        snapshot.error_kind = Some("profile_blocked");
        let account = claude::AccountSnapshot {
            profile: crate::claude_accounts::Profile {
                id: "profile:00".into(),
                path: "/synthetic/.claude/profiles/boulot".into(),
                name: "boulot".into(),
                visible: true,
            },
            primary: false,
            available: true,
            refreshing: false,
            snapshot,
        };
        assert_eq!(
            claude_state(&account),
            ("Non collectable".to_string(), Tone::Neutral)
        );
    }

    #[test]
    fn a_blocked_aip_codex_home_reads_as_non_collectable_not_as_an_error() {
        use crate::account_sources::Blocker;
        let found = |blocker, available| crate::codex_accounts::Discovered {
            profile: crate::claude_accounts::Profile {
                id: "profile:01".into(),
                path: "/synthetic/.codex/profiles/boulot".into(),
                name: "boulot".into(),
                visible: true,
            },
            primary: false,
            tracked: false,
            launch: None,
            available,
            visible: true,
            blocker: Some(blocker),
            credential: None,
        };
        for blocker in [Blocker::Incomplete] {
            let view = codex::account_view(found(blocker, true), true, None, None);
            assert_eq!(view.phase, "blocked");
            assert!(view.rows.is_empty() && view.plan_type.is_none());
            assert_eq!(
                codex_state(&view),
                ("Non collectable".to_string(), Tone::Neutral)
            );
        }
        // A deleted folder still reads « Indisponible »; Codex disabled wins.
        let gone = codex::account_view(found(Blocker::Incomplete, false), true, None, None);
        assert_eq!(codex_state(&gone).0, "Indisponible");
        let off = codex::account_view(found(Blocker::Incomplete, true), false, None, None);
        assert_eq!(codex_state(&off).0, "Désactivé");
    }
}
