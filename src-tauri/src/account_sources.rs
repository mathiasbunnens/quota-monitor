//! Common discovery contract and preference reconciliation for provider
//! accounts (Claude Code, Codex CLI). Names, paths and view preferences only:
//! nothing here opens a credential file, calls a CLI, logs in/out or changes
//! the active account of any manager.
//!
//! Contract for the provider modules (`claude_accounts`, `codex_accounts`):
//! build an ordered list of [`Candidate`]s from the sources (primary first,
//! then `~/.<tool>-*` siblings, `aip` profiles, saved entries), then call
//! [`reconcile`] with the provider's [`AccountPrefs`]. The result keeps the
//! user's Unique/Multi mode, custom names, visibility, exclusions and manual
//! additions; discovery itself never changes the mode. Collection must follow
//! [`Collect`]: `Inherit` (primary, environment untouched), `Isolated` (child
//! only, variable set to the raw path) or `Blocked` (listed, never launched).
use crate::aip;
use crate::claude_accounts::{identity, normalized, AccountPrefs, Profile};
use std::path::{Path, PathBuf};

/// Static facts about one provider.
pub struct Provider {
    /// Historical base folder under `~` (`.claude`, `.codex`); also aip's base.
    pub base: &'static str,
    /// Sibling folder prefix (`.claude-`, `.codex-`).
    pub prefix: &'static str,
    /// Whether `~/<base>` is listed as a sibling when the primary is elsewhere
    /// (historical Codex behaviour; Claude never did).
    pub base_is_sibling: bool,
    /// aip payload file whose presence (never contents) marks a complete profile.
    pub aip_payload: &'static str,
    /// Why an aip profile of this provider cannot be collected by the app.
    pub aip_blocker: Blocker,
    /// aip's `_current` profile is the account the primary already shows
    /// (Claude: the marker names the keychain's account; Codex: it only selects
    /// the next `aip codex` launch).
    pub aip_current_is_primary: bool,
}
pub const CLAUDE: Provider = Provider {
    base: ".claude",
    prefix: ".claude-",
    base_is_sibling: false,
    aip_payload: aip::CLAUDE_PAYLOAD,
    aip_blocker: Blocker::SnapshotOnly,
    aip_current_is_primary: true,
};
pub const CODEX: Provider = Provider {
    base: ".codex",
    prefix: ".codex-",
    base_is_sibling: true,
    aip_payload: aip::CODEX_PAYLOAD,
    aip_blocker: Blocker::GrantUnverified,
    aip_current_is_primary: false,
};

/// Where a candidate was found. On duplicate identities the first source wins
/// (callers list sources in priority order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Historical account: inherited variable, else `~/<base>`.
    Primary,
    /// `~/<prefix>*` folder (or `~/<base>` when the primary is elsewhere).
    Sibling,
    /// Saved in the preferences only (chosen folder or typed path).
    Manual,
    /// `aip` profile folder; `active` = named by `profiles/_current`.
    Aip { active: bool },
}

/// Why a detected account cannot be read. User-facing wording belongs to the
/// provider module; none of these carries a secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    /// aip Claude profile: a keychain snapshot, not a Claude Code home; reading
    /// it would need `aip` to switch the active account (never done).
    SnapshotOnly,
    /// aip Codex profile: an independent OAuth grant cannot be told apart from
    /// an imported copy of the primary's without reading `auth.json` (never
    /// done); launching an imported copy could rotate the primary's token.
    GrantUnverified,
    /// aip profile folder without its documented payload file (empty/corrupted).
    Incomplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collect {
    /// Primary: the child inherits the app environment untouched.
    Inherit,
    /// Runnable home: set the provider variable to the raw path on the child only.
    Isolated,
    /// Listed and explainable, never launched.
    Blocked(Blocker),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// `profile:<hex of the canonical path>` (see `claude_accounts::identity`).
    pub id: String,
    /// Default display name; a saved name replaces it in [`reconcile`].
    pub name: String,
    /// Raw path string as discovered/entered (persisted, displayed, launched);
    /// canonical only for the primary.
    pub raw: String,
    pub origin: Origin,
    pub collect: Collect,
    /// The folder exists right now.
    pub available: bool,
    /// Identity of a listed account this one is a copy of (aip Claude
    /// `_current` = the primary's keychain account): folded by [`reconcile`],
    /// so one account is never collected twice.
    pub same_account_as: Option<String>,
}
impl Candidate {
    fn new(raw: PathBuf, origin: Origin, collect: Collect, name: Option<String>) -> Self {
        let canonical = normalized(&raw);
        let raw = if origin == Origin::Primary {
            &canonical
        } else {
            &raw
        };
        Self {
            id: identity(&canonical),
            name: name.unwrap_or_else(|| default_name(raw, "")),
            raw: raw.to_string_lossy().into_owned(),
            origin,
            collect,
            available: canonical.is_dir(),
            same_account_as: None,
        }
    }
    /// Raw launch value for an isolated child (`CLAUDE_CONFIG_DIR` /
    /// `CODEX_HOME`); `None` for `Inherit` AND `Blocked`: always match
    /// [`Candidate::collect`] before launching anything.
    pub fn isolation(&self) -> Option<&Path> {
        matches!(self.collect, Collect::Isolated).then(|| Path::new(&self.raw))
    }
    /// Can be read by the app right now (exists, not blocked).
    pub fn collectable(&self) -> bool {
        self.available && !matches!(self.collect, Collect::Blocked(_))
    }
}

/// Folder name without the provider prefix, bounded to 64 chars, never empty.
pub fn default_name(path: &Path, prefix: &str) -> String {
    let base = path.file_name().unwrap_or_default().to_string_lossy();
    let trimmed = if prefix.is_empty() {
        &*base
    } else {
        base.trim_start_matches(prefix)
    };
    let picked = if trimmed.is_empty() { &*base } else { trimmed };
    let name: String = picked.chars().take(64).collect();
    if name.is_empty() {
        "Compte".into()
    } else {
        name
    }
}

/// Historical primary: a non-empty absolute variable, else `~/<base>`.
pub fn primary(home: &Path, provider: &Provider, env: Option<&Path>) -> Candidate {
    let raw = env
        .filter(|p| p.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(provider.base));
    Candidate::new(
        raw,
        Origin::Primary,
        Collect::Inherit,
        Some("Principal".into()),
    )
}

/// `~/<prefix>*` folders (sorted), plus `~/<base>` for providers that list it.
/// Only names are read (`read_dir` + `is_dir`).
pub fn siblings(home: &Path, provider: &Provider) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return vec![];
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            e.file_name().to_str().is_some_and(|n| {
                (n.starts_with(provider.prefix) && n.len() > provider.prefix.len())
                    || (provider.base_is_sibling && n == provider.base)
            }) && e.path().is_dir()
        })
        .map(|e| e.path())
        .collect();
    found.sort();
    found
        .into_iter()
        .map(|raw| {
            let name = default_name(&raw, provider.prefix);
            Candidate::new(raw, Origin::Sibling, Collect::Isolated, Some(name))
        })
        .collect()
}

/// Saved entries (manual additions and renamed/hidden detected accounts) in
/// saved order; duplicates of detected folders are dropped by [`reconcile`].
pub fn saved(prefs: &AccountPrefs) -> Vec<Candidate> {
    prefs
        .profiles
        .iter()
        .map(|p| {
            Candidate::new(
                PathBuf::from(&p.path),
                Origin::Manual,
                Collect::Isolated,
                Some(p.name.clone()),
            )
        })
        .collect()
}

/// aip profiles of this provider (`~/<base>/profiles/<name>`, aip's order).
/// They are listed with their aip name and the provider's blocker: never
/// launched by the app. When the primary is the default `~/<base>` and the
/// provider's `_current` means "the keychain's account", that profile is
/// marked as the same account as `primary` so it is folded, not duplicated.
pub fn aip_profiles(home: &Path, provider: &Provider, primary: &Candidate) -> Vec<Candidate> {
    let base = home.join(provider.base);
    let primary_is_base = normalized(&base) == normalized(Path::new(&primary.raw));
    aip::profiles(&base, provider.aip_payload)
        .into_iter()
        .map(|p| {
            let blocker = if p.complete {
                provider.aip_blocker
            } else {
                Blocker::Incomplete
            };
            let mut c = Candidate::new(
                p.raw,
                Origin::Aip { active: p.active },
                Collect::Blocked(blocker),
                Some(p.name),
            );
            if p.active && p.complete && primary_is_base && provider.aip_current_is_primary {
                c.same_account_as = Some(primary.id.clone());
            }
            c
        })
        .collect()
}

/// Every source in priority order: primary, siblings, aip, saved.
pub fn all(
    home: &Path,
    provider: &Provider,
    env: Option<&Path>,
    prefs: &AccountPrefs,
) -> Vec<Candidate> {
    let first = primary(home, provider, env);
    let mut out = vec![first.clone()];
    out.extend(siblings(home, provider));
    out.extend(aip_profiles(home, provider, &first));
    out.extend(saved(prefs));
    out
}

/// One account after the preferences were applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// Name and raw path already replaced by the saved entry when there is one.
    pub candidate: Candidate,
    /// A saved entry exists (renamed, hidden/shown or added by the user).
    pub saved: bool,
    /// Visibility preference (saved, else the default for this mode/account).
    pub preferred_visible: bool,
    /// Effective visibility in the current mode (display only).
    pub visible: bool,
    /// aip names folded into this account (`same_account_as`), e.g. the
    /// primary carrying « perso » because aip's `_current` says so.
    pub aliases: Vec<String>,
}
impl Resolved {
    pub fn primary(&self) -> bool {
        self.candidate.origin == Origin::Primary
    }
    /// Preference-shaped view (what the settings UI edits and writes back).
    pub fn profile(&self) -> Profile {
        Profile {
            id: self.candidate.id.clone(),
            path: self.candidate.raw.clone(),
            name: self.candidate.name.clone(),
            visible: self.preferred_visible,
        }
    }
}

/// Applies the preferences to candidates without changing them:
/// 1. one entry per identity (first source wins, order kept);
/// 2. removed accounts (`excluded`) stay removed, the primary never is;
/// 3. a saved entry supplies name, visibility and raw path;
/// 4. `same_account_as` copies fold into their account as aliases;
/// 5. default visibility of an unsaved account: primary, or Multi and collectable;
/// 6. Unique shows the selected account when it is listed, else the primary.
pub fn reconcile(candidates: Vec<Candidate>, prefs: &AccountPrefs) -> Vec<Resolved> {
    let multi = prefs.mode == "multi";
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Resolved> = vec![];
    for mut c in candidates {
        let primary = c.origin == Origin::Primary;
        if !seen.insert(c.id.clone()) || (!primary && prefs.excluded.contains(&c.id)) {
            continue;
        }
        // Same rule as the provider modules: match on the canonical path, not
        // on the stored id (robust to an id that drifted in an old file).
        let saved = prefs
            .profiles
            .iter()
            .find(|s| identity(&normalized(Path::new(&s.path))) == c.id);
        if let Some(s) = saved {
            c.name = s.name.clone();
            if !primary {
                c.raw = s.path.clone();
            }
        }
        let preferred_visible = saved
            .map(|s| s.visible)
            .unwrap_or(primary || (multi && c.collectable()));
        out.push(Resolved {
            candidate: c,
            saved: saved.is_some(),
            preferred_visible,
            visible: false,
            aliases: vec![],
        });
    }
    // Fold copies of a listed account (never two collections of one account).
    let mut i = 0;
    while i < out.len() {
        let target = out[i].candidate.same_account_as.as_ref().and_then(|t| {
            out.iter()
                .position(|r| &r.candidate.id == t && r.candidate.same_account_as.is_none())
        });
        match target {
            Some(t) if t != i => {
                let copy = out.remove(i);
                let t = if t > i { t - 1 } else { t };
                out[t].aliases.push(copy.candidate.name);
            }
            _ => i += 1,
        }
    }
    let selected = prefs.selected_id.as_deref().unwrap_or("");
    let selected_exists = out.iter().any(|r| r.candidate.id == selected);
    for r in &mut out {
        r.visible = if multi {
            r.preferred_visible
        } else if selected_exists {
            r.candidate.id == selected
        } else {
            r.primary()
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Synthetic fixtures only: fake homes under the temp dir, sentinel files,
    /// no real account, no real manager.
    fn home(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("qm-sources-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p.canonicalize().unwrap()
    }
    fn aip_profile(home: &Path, provider: &Provider, name: &str, complete: bool) -> PathBuf {
        let dir = home.join(provider.base).join(aip::PROFILES_DIR).join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if complete {
            std::fs::write(dir.join(provider.aip_payload), "SYNTHETIC-SENTINEL").unwrap();
        }
        dir
    }
    fn set_current(home: &Path, provider: &Provider, name: &str) {
        std::fs::write(
            home.join(provider.base)
                .join(aip::PROFILES_DIR)
                .join(aip::CURRENT_MARKER),
            format!("{name}\n"),
        )
        .unwrap();
    }
    fn names(r: &[Resolved]) -> Vec<String> {
        r.iter().map(|r| r.candidate.name.clone()).collect()
    }
    fn visible(r: &[Resolved]) -> Vec<String> {
        r.iter()
            .filter(|r| r.visible)
            .map(|r| r.candidate.name.clone())
            .collect()
    }
    fn multi() -> AccountPrefs {
        AccountPrefs {
            mode: "multi".into(),
            ..AccountPrefs::default()
        }
    }

    #[test]
    fn parity_with_the_existing_claude_and_codex_discovery() {
        let h = home("parity");
        for d in [
            ".claude",
            ".claude-work",
            ".codex",
            ".codex-work",
            ".codex-perso",
        ] {
            std::fs::create_dir_all(h.join(d)).unwrap();
        }
        let elsewhere = h.join("Elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let saved_entry = |raw: &Path, name: &str, visible: bool| Profile {
            id: identity(&normalized(raw)),
            path: raw.to_string_lossy().into_owned(),
            name: name.into(),
            visible,
        };
        for mode in ["single", "multi"] {
            let prefs = AccountPrefs {
                mode: mode.into(),
                profiles: vec![
                    saved_entry(&h.join(".claude-work"), "Travail", false),
                    saved_entry(&elsewhere, "Ailleurs", true),
                ],
                ..AccountPrefs::default()
            };
            let legacy = crate::claude_accounts::discover(&h, None, &prefs);
            let common = reconcile(all(&h, &CLAUDE, None, &prefs), &prefs);
            assert_eq!(legacy.len(), common.len(), "{mode}");
            for (l, c) in legacy.iter().zip(&common) {
                assert_eq!(l.profile, c.profile(), "{mode}");
                assert_eq!(
                    (l.primary, l.available, l.visible),
                    (c.primary(), c.candidate.available, c.visible)
                );
                assert_eq!(l.launch.as_deref(), c.candidate.isolation(), "{mode}");
            }
            let prefs = AccountPrefs {
                profiles: vec![saved_entry(&h.join(".codex-perso"), "Perso", false)],
                ..prefs
            };
            let legacy = crate::codex_accounts::discover(&h, None, &prefs);
            let common = reconcile(all(&h, &CODEX, None, &prefs), &prefs);
            assert_eq!(legacy.len(), common.len(), "{mode}");
            for (l, c) in legacy.iter().zip(&common) {
                assert_eq!(l.profile, c.profile(), "{mode}");
                assert_eq!(
                    (l.primary, l.available, l.visible),
                    (c.primary(), c.candidate.available, c.visible)
                );
                assert_eq!(l.launch.as_deref(), c.candidate.isolation(), "{mode}");
            }
            // Custom CODEX_HOME: ~/.codex becomes an ordinary sibling (Codex only).
            let custom = h.join("custom-home");
            std::fs::create_dir_all(&custom).unwrap();
            let legacy = crate::codex_accounts::discover(&h, Some(&custom), &prefs);
            let common = reconcile(all(&h, &CODEX, Some(&custom), &prefs), &prefs);
            assert_eq!(
                legacy
                    .iter()
                    .map(|l| l.profile.path.clone())
                    .collect::<Vec<_>>(),
                common
                    .iter()
                    .map(|c| c.candidate.raw.clone())
                    .collect::<Vec<_>>()
            );
            assert!(common[0].primary() && common[0].candidate.raw == custom.to_str().unwrap());
        }
        std::fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn aip_absent_or_empty_adds_nothing_and_keeps_the_mode() {
        let h = home("aip-absent");
        std::fs::create_dir_all(h.join(".claude")).unwrap();
        let prefs = AccountPrefs::default();
        let found = reconcile(all(&h, &CLAUDE, None, &prefs), &prefs);
        assert_eq!(names(&found), vec!["Principal"]);
        std::fs::create_dir_all(h.join(".claude").join(aip::PROFILES_DIR)).unwrap();
        std::fs::write(h.join(".codex"), "a file, not a home").unwrap();
        assert_eq!(
            names(&reconcile(all(&h, &CLAUDE, None, &prefs), &prefs)),
            vec!["Principal"]
        );
        assert_eq!(
            names(&reconcile(all(&h, &CODEX, None, &prefs), &prefs)),
            vec!["Principal"]
        );
        assert_eq!(prefs.mode, "single");
        std::fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn aip_claude_profiles_are_listed_blocked_and_the_current_one_folds_into_the_primary() {
        let h = home("aip-claude");
        std::fs::create_dir_all(h.join(".claude")).unwrap();
        aip_profile(&h, &CLAUDE, "perso", true);
        let work = aip_profile(&h, &CLAUDE, "work", true);
        aip_profile(&h, &CLAUDE, "vide", false);
        set_current(&h, &CLAUDE, "perso");
        let prefs = AccountPrefs::default();
        let found = reconcile(all(&h, &CLAUDE, None, &prefs), &prefs);
        // Background discovery: Unique untouched, the primary alone displayed.
        // aip order (no `_order` file): alphabetical.
        assert_eq!(names(&found), vec!["Principal", "vide", "work"]);
        assert_eq!(
            found[0].aliases,
            vec!["perso"],
            "_current = the keychain = the primary"
        );
        assert_eq!(visible(&found), vec!["Principal"]);
        assert_eq!(prefs.mode, "single");
        let w = &found[2].candidate;
        assert_eq!(w.origin, Origin::Aip { active: false });
        assert_eq!(w.collect, Collect::Blocked(Blocker::SnapshotOnly));
        assert!(w.available && !w.collectable() && w.isolation().is_none());
        assert_eq!(w.raw, work.to_str().unwrap());
        assert_eq!(w.id, identity(&work));
        assert_eq!(
            found[1].candidate.collect,
            Collect::Blocked(Blocker::Incomplete)
        );
        // Multi: blocked accounts are listed but not displayed until the user asks.
        let found = reconcile(all(&h, &CLAUDE, None, &multi()), &multi());
        assert_eq!(visible(&found), vec!["Principal"]);
        // The sentinel payloads were never opened or changed.
        assert_eq!(
            std::fs::read_to_string(work.join(aip::CLAUDE_PAYLOAD)).unwrap(),
            "SYNTHETIC-SENTINEL"
        );
        // A custom CLAUDE_CONFIG_DIR means the keychain's account is not the
        // primary: nothing is folded, the current profile stays a blocked entry.
        let custom = h.join("custom");
        std::fs::create_dir_all(&custom).unwrap();
        let found = reconcile(all(&h, &CLAUDE, Some(&custom), &prefs), &prefs);
        assert_eq!(names(&found), vec!["Principal", "perso", "vide", "work"]);
        assert!(found[0].aliases.is_empty());
        assert_eq!(found[1].candidate.origin, Origin::Aip { active: true });
        std::fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn aip_codex_homes_expose_their_raw_path_but_stay_blocked_until_a_grant_is_proven() {
        let h = home("aip-codex");
        std::fs::create_dir_all(h.join(".codex")).unwrap();
        std::fs::create_dir_all(h.join(".codex-travail")).unwrap();
        let perso = aip_profile(&h, &CODEX, "perso", true);
        set_current(&h, &CODEX, "perso");
        let prefs = multi();
        let found = reconcile(all(&h, &CODEX, None, &prefs), &prefs);
        assert_eq!(names(&found), vec!["Principal", "travail", "perso"]);
        let p = &found[2].candidate;
        // `_current` only selects the next `aip codex` launch: no fold.
        assert_eq!(p.origin, Origin::Aip { active: true });
        assert!(found[0].aliases.is_empty());
        assert_eq!(p.collect, Collect::Blocked(Blocker::GrantUnverified));
        assert_eq!(
            p.raw,
            perso.to_str().unwrap(),
            "raw CODEX_HOME value kept for a future policy"
        );
        assert!(
            p.isolation().is_none(),
            "never launched by the common layer"
        );
        assert_eq!(visible(&found), vec!["Principal", "travail"]);
        // The sibling home is collected as before.
        assert_eq!(found[1].candidate.collect, Collect::Isolated);
        assert_eq!(
            found[1].candidate.isolation(),
            Some(h.join(".codex-travail").as_path())
        );
        std::fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn saved_names_visibility_exclusions_and_manual_duplicates_reconcile_across_sources() {
        let h = home("prefs");
        std::fs::create_dir_all(h.join(".codex")).unwrap();
        let work = aip_profile(&h, &CODEX, "boulot", true);
        aip_profile(&h, &CODEX, "other", true);
        let id = identity(&work);
        // The user had added the aip folder by hand (trailing separator kept)
        // and renamed it: one entry, aip origin, saved name and raw path.
        let raw = format!("{}{}", work.to_str().unwrap(), std::path::MAIN_SEPARATOR);
        let mut prefs = multi();
        prefs.profiles.push(Profile {
            id: id.clone(),
            path: raw.clone(),
            name: "Boulot".into(),
            visible: true,
        });
        let found = reconcile(all(&h, &CODEX, None, &prefs), &prefs);
        assert_eq!(names(&found), vec!["Principal", "Boulot", "other"]);
        assert!(found[1].saved && found[1].candidate.origin == Origin::Aip { active: false });
        assert_eq!(found[1].candidate.raw, raw);
        assert!(
            found[1].visible,
            "explicit visibility wins over the blocked default"
        );
        assert!(!found[2].visible);
        // Removal is remembered across scans; the primary cannot be removed.
        prefs.exclude(&id);
        prefs.exclude(&found[0].candidate.id);
        for _ in 0..2 {
            let again = reconcile(all(&h, &CODEX, None, &prefs), &prefs);
            assert_eq!(names(&again), vec!["Principal", "other"]);
        }
        // Unique: the selected account is shown when listed, else the primary.
        let mut single = AccountPrefs {
            mode: "single".into(),
            selected_id: Some(found[2].candidate.id.clone()),
            ..AccountPrefs::default()
        };
        assert_eq!(
            visible(&reconcile(all(&h, &CODEX, None, &single), &single)),
            vec!["other"]
        );
        single.selected_id = Some("profile:gone".into());
        assert_eq!(
            visible(&reconcile(all(&h, &CODEX, None, &single), &single)),
            vec!["Principal"]
        );
        // Deleted aip profile: a saved entry lingers as unavailable (like any
        // saved folder), an unsaved one simply disappears.
        std::fs::remove_dir_all(h.join(".codex").join(aip::PROFILES_DIR).join("other")).unwrap();
        prefs.excluded.clear();
        let after = reconcile(all(&h, &CODEX, None, &prefs), &prefs);
        assert_eq!(names(&after), vec!["Principal", "Boulot"]);
        std::fs::remove_dir_all(&work).unwrap();
        let after = reconcile(all(&h, &CODEX, None, &prefs), &prefs);
        assert_eq!(names(&after), vec!["Principal", "Boulot"]);
        assert!(!after[1].candidate.available && after[1].candidate.origin == Origin::Manual);
        std::fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn folding_never_loses_an_account_whose_target_is_missing_and_identity_is_path_based() {
        let h = home("fold");
        let a = Candidate {
            id: "profile:aa".into(),
            name: "A".into(),
            raw: "/synthetic/a".into(),
            origin: Origin::Aip { active: true },
            collect: Collect::Blocked(Blocker::SnapshotOnly),
            available: true,
            same_account_as: Some("profile:missing".into()),
        };
        let found = reconcile(vec![a.clone()], &multi());
        assert_eq!(
            found.len(),
            1,
            "no target listed: kept as its own blocked entry"
        );
        assert!(found[0].aliases.is_empty());
        // Renaming an aip profile is delete + save: a new folder is a new identity,
        // the old saved name does not migrate (same rule as moved folders).
        let x = aip_profile(&h, &CLAUDE, "x", true);
        let y = h.join(".claude").join(aip::PROFILES_DIR).join("y");
        assert_ne!(identity(&x), identity(&y));
        std::fs::remove_dir_all(h).unwrap();
    }

    #[test]
    fn default_names_are_bounded_and_never_empty() {
        assert_eq!(
            default_name(Path::new("/h/.claude-work"), ".claude-"),
            "work"
        );
        assert_eq!(default_name(Path::new("/h/.codex-"), ".codex-"), ".codex-");
        assert_eq!(default_name(Path::new("/h/profiles/perso"), ""), "perso");
        assert_eq!(default_name(Path::new("/"), ""), "Compte");
        assert_eq!(
            default_name(Path::new(&format!("/h/{}", "n".repeat(100))), "").len(),
            64
        );
    }
}
