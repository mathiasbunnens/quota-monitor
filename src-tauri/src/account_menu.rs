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
}
