//! Codex environments (one `CODEX_HOME` folder each): names, paths and view
//! preferences only. Never reads `auth.json`, tokens or the keychain, never
//! logs in/out and never changes the active Codex session.
//!
//! The primary environment is the historical one (inherited `CODEX_HOME`, else
//! `~/.codex`) and is launched exactly as before (no variable injected).
//! Additional environments are *followed* explicitly (detected `~/.codex-*`
//! folder or a chosen absolute folder); only followed ones get their own
//! app-server child, with `CODEX_HOME` set on that child only.
//!
//! Sources (see `account_sources` and docs/aip-profiles.md §8/§10): the
//! primary, `~/.codex-*` folders (and `~/.codex` when `CODEX_HOME` is
//! elsewhere), `aip` profiles (`~/.codex/profiles/<name>`, dqn/aip v0.11.0)
//! and saved entries. aip homes are listed under their aip name and never
//! launched as an app-server child (a launched copy of the primary's grant
//! could rotate its token, aip's own guard): the collector reads their quotas
//! over HTTPS with the access token saved in the home (`Collect::Credential`,
//! `oauth_usage`), renewed with the CLI's protocol only when the home owns
//! its grant (`oauth_refresh`: aip's imported-grant marker is honoured).
use crate::account_sources::{self, Blocker, Collect, Origin};
use crate::claude_accounts::{identity, normalized, profile_path, AccountPrefs, Profile};
use std::path::{Path, PathBuf};

/// Detected folder prefix (`~/.codex-travail`).
pub const PREFIX: &str = ".codex-";
/// Upper bound of additional app-server children.
pub const MAX_EXTRA: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovered {
    pub profile: Profile,
    pub primary: bool,
    /// Followed (collected). The primary is always followed; a detected
    /// `~/.codex-*` folder is only listed until the user follows it.
    pub tracked: bool,
    /// Exact `CODEX_HOME` value for the child (raw discovered/entered string,
    /// never canonicalised: the CLI canonicalises it itself). `None` for the
    /// primary, which inherits the app environment untouched.
    pub launch: Option<PathBuf>,
    pub available: bool,
    pub visible: bool,
    /// Why this account is listed but never collected (incomplete aip
    /// profile). `Some` implies `tracked == false`, `launch == None` and
    /// `credential == None`.
    pub blocker: Option<Blocker>,
    /// aip home read with its own saved credential (`auth.json` loaded in
    /// memory by `oauth_usage`, raw folder path): `Some` implies `tracked`,
    /// `launch == None` (never an app-server child) and `blocker == None`.
    pub credential: Option<PathBuf>,
}

/// User-facing reason of a blocked account (no path, no secret).
pub fn blocked_message(blocker: Blocker) -> &'static str {
    match blocker {
        Blocker::Incomplete => "Profil aip incomplet (aucun auth.json) : rien à lire.",
    }
}

/// Same rules as the Claude profiles, Codex wording, bounded child count.
pub fn validate(prefs: &AccountPrefs) -> Result<(), String> {
    if !["single", "multi"].contains(&prefs.mode.as_str()) {
        return Err("Mode Codex non pris en charge.".into());
    }
    // Followed homes + aip homes (read by credential, bounded apart) + the
    // optional saved entry of the primary (name/visibility).
    prefs.validate_excluded()?;
    if prefs.profiles.len() > 2 * MAX_EXTRA + 1 {
        return Err(format!(
            "Au plus {MAX_EXTRA} comptes Codex supplémentaires et {MAX_EXTRA} profils aip peuvent être suivis."
        ));
    }
    let mut paths = std::collections::HashSet::new();
    for p in &prefs.profiles {
        if !Path::new(&p.path).is_absolute()
            || p.name.trim().is_empty()
            || p.name.chars().count() > 64
            || p.name.chars().any(char::is_control)
        {
            return Err(
                "Compte Codex invalide : chemin absolu et nom de 1 à 64 caractères requis.".into(),
            );
        }
        let path = normalized(Path::new(&p.path));
        if p.id != identity(&path) || !paths.insert(path) {
            return Err("Compte Codex dupliqué ou identité invalide.".into());
        }
    }
    Ok(())
}

/// Primary home as the CLI resolves it: a non-empty absolute `CODEX_HOME`,
/// else `~/.codex`. Used for identity/deduplication only, never injected.
/// (Production uses `account_sources::primary`, same rule.)
#[cfg(test)]
fn primary_home(home: &Path, env: Option<&Path>) -> PathBuf {
    env.filter(|p| p.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".codex"))
}

fn default_name(path: &Path) -> String {
    let base = path.file_name().unwrap_or_default().to_string_lossy();
    let trimmed = base.trim_start_matches(PREFIX);
    if trimmed.is_empty() { &base } else { trimmed }
        .chars()
        .take(64)
        .collect()
}

/// Order: primary, detected `~/.codex-*` folders (sorted), aip profiles (aip's
/// order), then saved homes outside detection. Only names/paths and file
/// presence are read. Startup and every refresh/`get_codex_accounts` call this,
/// so aip profiles appear without any import or « Suivre » step; the mode is
/// never changed by discovery.
pub fn discover(home: &Path, env: Option<&Path>, prefs: &AccountPrefs) -> Vec<Discovered> {
    let mut candidates = account_sources::all(home, &account_sources::CODEX, env, prefs);
    route_through_aip(&mut candidates);
    let mut runnable = 0usize;
    let mut credentials = 0usize;
    let mut out: Vec<Discovered> = vec![];
    for r in account_sources::reconcile(candidates, prefs) {
        let (blocker, credential) = match r.candidate.collect {
            Collect::Blocked(b) => (Some(b), None),
            // Every spelling of an aip home (detected, symlinked sibling,
            // saved by hand) is read with its own credential, never launched.
            Collect::Credential => (None, Some(PathBuf::from(&r.candidate.raw))),
            _ => (None, None),
        };
        if blocker.is_none() {
            if credential.is_some() {
                // Same bound for credential reads, counted apart: an aip home
                // never takes a slot of the launchable homes (3.2.0 rule).
                if !r.saved && credentials >= MAX_EXTRA {
                    continue;
                }
                credentials += 1;
            } else {
                // Never more unsaved homes than the collector can run (3.2.0
                // rule); blocked aip entries are never run, so they do not count.
                if !r.saved && !r.primary() && runnable > MAX_EXTRA {
                    continue;
                }
                runnable += 1;
            }
        }
        out.push(Discovered {
            profile: r.profile(),
            primary: r.primary(),
            tracked: blocker.is_none(),
            launch: r.candidate.isolation().map(Path::to_path_buf),
            available: r.candidate.available,
            visible: false,
            blocker,
            credential,
        });
    }
    // Same display rules as 3.2.0, applied to the bounded list: only followed
    // (collected) homes are displayed; a blocked home keeps its saved
    // preference (`profile.visible`) but has nothing to draw.
    let selected = prefs.selected_id.as_deref().unwrap_or("");
    let selected_exists = out.iter().any(|a| a.tracked && a.profile.id == selected);
    for a in &mut out {
        a.visible = a.tracked
            && if prefs.mode == "multi" {
                a.profile.visible
            } else if selected_exists {
                a.profile.id == selected
            } else {
                a.primary
            };
    }
    out
}

/// A `~/.codex-*` folder that resolves into an aip profile (symlink) is that
/// aip home: it inherits its name and collection mode (credential read, or
/// blocked when incomplete), so no spelling of an aip home is ever launched.
/// The first spelling keeps its raw path and position.
fn route_through_aip(candidates: &mut [account_sources::Candidate]) {
    let aip: Vec<_> = candidates
        .iter()
        .filter(|c| matches!(c.origin, Origin::Aip { .. }))
        .map(|c| (c.id.clone(), c.name.clone(), c.origin, c.collect))
        .collect();
    for c in candidates.iter_mut() {
        if matches!(c.origin, Origin::Primary | Origin::Aip { .. }) {
            continue;
        }
        if let Some((_, name, origin, collect)) = aip.iter().find(|(id, ..)| *id == c.id) {
            c.name = name.clone(); // a saved name still wins in `reconcile`
            c.origin = *origin;
            c.collect = *collect;
        }
    }
}

/// Pre-aip discovery (3.2.0), kept verbatim as the non-regression reference.
#[cfg(test)]
pub(crate) fn legacy_discover(
    home: &Path,
    env: Option<&Path>,
    prefs: &AccountPrefs,
) -> Vec<Discovered> {
    let primary_raw = primary_home(home, env);
    let primary = normalized(&primary_raw);
    let mut seen = std::collections::HashSet::new();
    seen.insert(primary.clone());
    let saved_primary = prefs
        .profiles
        .iter()
        .find(|s| normalized(Path::new(&s.path)) == primary);
    let primary_profile = Profile {
        id: identity(&primary),
        path: primary.to_string_lossy().into_owned(),
        name: saved_primary
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Principal".into()),
        visible: saved_primary.map(|s| s.visible).unwrap_or(true),
    };
    let mut out = vec![Discovered {
        available: primary.is_dir(),
        launch: None,
        primary: true,
        tracked: true,
        visible: false,
        profile: primary_profile,
        blocker: None,
        credential: None,
    }];
    // Detected homes first (sorted, so the order is stable whether or not the
    // user renamed one), then saved homes outside `~/` detection.
    let saved_of = |p: &Path| {
        prefs
            .profiles
            .iter()
            .find(|s| normalized(Path::new(&s.path)) == p)
    };
    let push = |out: &mut Vec<Discovered>, raw: PathBuf, p: PathBuf, detected: bool| {
        let saved = saved_of(&p);
        // Never more unsaved homes than the collector can run.
        if saved.is_none() && out.len() > MAX_EXTRA {
            return;
        }
        let raw = saved.map(|s| PathBuf::from(&s.path)).unwrap_or(raw);
        out.push(Discovered {
            available: p.is_dir(),
            profile: Profile {
                id: identity(&p),
                path: raw.to_string_lossy().into_owned(),
                name: saved
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| default_name(&raw)),
                // Automatically added homes follow the current mode: shown in
                // Multi, kept aside in Unique (which shows the selected one).
                visible: saved
                    .map(|s| s.visible)
                    .unwrap_or(detected && prefs.mode == "multi"),
            },
            launch: Some(raw),
            primary: false,
            tracked: true,
            visible: false,
            blocker: None,
            credential: None,
        });
    };
    if let Ok(entries) = std::fs::read_dir(home) {
        let mut detected: Vec<_> = entries
            .flatten()
            .filter(|e| {
                e.file_name().to_str().is_some_and(|n| {
                    (n.starts_with(PREFIX) && n.len() > PREFIX.len()) || n == ".codex"
                }) && e.path().is_dir()
            })
            .map(|e| e.path())
            .collect();
        detected.sort();
        for raw in detected {
            let p = normalized(&raw);
            if prefs.excluded.contains(&identity(&p)) || !seen.insert(p.clone()) {
                continue;
            }
            push(&mut out, raw, p, true);
        }
    }
    for saved in &prefs.profiles {
        let p = normalized(Path::new(&saved.path));
        if prefs.excluded.contains(&identity(&p)) || !seen.insert(p.clone()) {
            continue;
        }
        push(&mut out, PathBuf::from(&saved.path), p, false);
    }
    let selected = prefs.selected_id.as_deref().unwrap_or("");
    let selected_exists = out.iter().any(|a| a.tracked && a.profile.id == selected);
    for a in &mut out {
        a.visible = a.tracked
            && if prefs.mode == "multi" {
                a.profile.visible
            } else if selected_exists {
                a.profile.id == selected
            } else {
                a.primary
            };
    }
    out
}

pub use crate::claude_accounts::FollowTarget;

/// Explicit « Ajouter un compte » (detected home or chosen existing folder):
/// follows a CODEX_HOME folder without reading it, logging in or creating
/// anything. Mirrors the Claude flow: switches to Multi keeping what was
/// displayed plus the new account; alone, it becomes the Unique account.
pub fn follow(
    home: &Path,
    env: Option<&Path>,
    prefs: &AccountPrefs,
    target: FollowTarget,
) -> Result<AccountPrefs, String> {
    // An explicit add brings back a home the user removed earlier.
    let mut cleaned = prefs.clone();
    let reinstated = match &target {
        FollowTarget::Detected(id) => cleaned.unexclude(id),
        FollowTarget::Path(raw) => cleaned.unexclude(&identity(&profile_path(raw)?)),
    };
    let prefs = &cleaned;
    let found = discover(home, env, prefs);
    let (id, raw) = match target {
        FollowTarget::Detected(id) => {
            let Some(a) = found.iter().find(|a| a.profile.id == id) else {
                return Err(
                    "Ce dossier Codex n’est plus détecté : actualisez la liste ou choisissez son dossier."
                        .into(),
                );
            };
            (id.to_string(), a.profile.path.clone())
        }
        FollowTarget::Path(raw) => (identity(&profile_path(raw)?), raw.to_string()),
    };
    let existing = found.iter().find(|a| a.profile.id == id);
    if let Some(a) = existing {
        if let Some(blocker) = a.blocker {
            // Nothing to read: no switch, no copy of credentials.
            return Err(blocked_message(blocker).into());
        }
        if !a.available {
            return Err(format!(
                "Le dossier de « {} » est indisponible : vérifiez qu’il existe encore.",
                a.profile.name
            ));
        }
        if a.tracked && a.visible && !reinstated {
            return Err(format!(
                "« {} » est déjà affiché. Choisissez un autre dossier CODEX_HOME.",
                a.profile.name
            ));
        }
    }
    // The bound of app-server children; aip homes are read apart.
    let followed_extras = found
        .iter()
        .filter(|a| a.tracked && !a.primary && a.credential.is_none())
        .count();
    if !existing.is_some_and(|a| a.tracked) && followed_extras >= MAX_EXTRA {
        return Err(format!(
            "Au plus {MAX_EXTRA} comptes Codex supplémentaires peuvent être suivis."
        ));
    }
    let profile = existing
        .map(|a| a.profile.clone())
        .unwrap_or_else(|| Profile {
            id: id.clone(),
            path: raw.clone(),
            name: default_name(Path::new(&raw)),
            visible: true,
        });
    let mut next = prefs.clone();
    let upsert = |next: &mut AccountPrefs, profile: Profile| match next
        .profiles
        .iter_mut()
        .find(|p| p.id == profile.id)
    {
        Some(saved) => saved.visible = profile.visible,
        None => next.profiles.push(profile),
    };
    if !found
        .iter()
        .any(|a| a.profile.id != id && a.tracked && a.available)
    {
        // No other usable account (e.g. no ~/.codex): this one is displayed alone.
        upsert(
            &mut next,
            Profile {
                visible: true,
                ..profile
            },
        );
        next.mode = "single".into();
        next.selected_id = Some(id);
        validate(&next)?;
        return Ok(next);
    }
    let was_single = next.mode != "multi";
    next.mode = "multi".into();
    for a in found.iter().filter(|a| a.tracked && a.profile.id != id) {
        if a.visible {
            upsert(
                &mut next,
                Profile {
                    visible: true,
                    ..a.profile.clone()
                },
            );
        } else if was_single {
            upsert(
                &mut next,
                Profile {
                    visible: false,
                    ..a.profile.clone()
                },
            );
        }
    }
    upsert(
        &mut next,
        Profile {
            visible: true,
            ..profile
        },
    );
    validate(&next)?;
    Ok(next)
}

/// « Supprimer <compte> » : forgets the saved entry and remembers the home so
/// detection does not add it back. The folder and its contents are never touched.
pub fn untrack(
    home: &Path,
    env: Option<&Path>,
    prefs: &AccountPrefs,
    id: &str,
) -> Result<AccountPrefs, String> {
    let found = discover(home, env, prefs);
    match found.iter().find(|a| a.profile.id == id) {
        Some(a) if a.primary => {
            return Err("Le compte Codex principal ne peut pas être retiré.".into())
        }
        // A blocked aip profile can be removed from the list (preference only).
        Some(a) if a.tracked || a.blocker.is_some() => {}
        _ => return Err("Ce compte Codex n’est pas suivi.".into()),
    }
    let mut next = prefs.clone();
    next.profiles.retain(|p| p.id != id);
    next.exclude(id);
    if next.selected_id.as_deref() == Some(id) {
        next.selected_id = None;
    }
    let remaining = found
        .iter()
        .filter(|a| a.tracked && !a.primary && a.profile.id != id)
        .count();
    if remaining == 0 {
        next.mode = "single".into();
        next.selected_id = None;
    }
    validate(&next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("qm-codex-accounts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p.canonicalize().unwrap()
    }
    fn visible(home: &Path, prefs: &AccountPrefs) -> Vec<String> {
        discover(home, None, prefs)
            .into_iter()
            .filter(|a| a.visible)
            .map(|a| a.profile.name)
            .collect()
    }

    #[test]
    fn discovery_follows_detected_homes_automatically_by_name_only() {
        let home = root("discovery");
        let prefs = AccountPrefs::default();
        // No ~/.codex yet: the primary stays listed (unavailable), never invented.
        let zero = discover(&home, None, &prefs);
        assert_eq!(zero.len(), 1);
        assert!(zero[0].primary && !zero[0].available && zero[0].launch.is_none());
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".codex-work")).unwrap();
        std::fs::write(home.join(".codex-not-a-dir"), "ignored").unwrap();
        // Sentinel in a home: discovery must never open or change it.
        std::fs::write(home.join(".codex-work/auth.json"), "sentinel").unwrap();
        let found = discover(&home, None, &prefs);
        assert_eq!(found.len(), 2);
        assert!(found[0].primary && found[0].tracked && found[0].visible);
        assert_eq!(found[0].profile.name, "Principal");
        // Auto-added, but kept aside in Unique: the principal stays the one displayed.
        assert!(!found[1].primary && found[1].tracked && !found[1].visible);
        assert_eq!(found[1].profile.name, "work");
        assert_eq!(
            found[1].launch.as_deref(),
            Some(home.join(".codex-work").as_path())
        );
        assert_eq!(
            std::fs::read_to_string(home.join(".codex-work/auth.json")).unwrap(),
            "sentinel"
        );
        // Background discovery never changes the historical mode.
        assert_eq!(prefs.mode, "single");
        // In Multi the same home is shown by default.
        let multi = AccountPrefs {
            mode: "multi".into(),
            ..AccountPrefs::default()
        };
        assert_eq!(visible(&home, &multi), vec!["Principal", "work"]);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn primary_inherits_codex_home_untouched_and_aliases_are_deduplicated() {
        let home = root("primary");
        let custom = home.join("custom-home");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        let found = discover(&home, Some(&custom), &AccountPrefs::default());
        assert!(found[0].primary && found[0].launch.is_none());
        assert_eq!(found[0].profile.path, custom.to_str().unwrap());
        // ~/.codex is then an ordinary auto-followed home.
        assert!(found.iter().any(|a| !a.primary && a.tracked));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&custom, home.join(".codex-alias")).unwrap();
            let again = discover(&home, Some(&custom), &AccountPrefs::default());
            assert_eq!(
                again.len(),
                found.len(),
                "alias of the primary is not a second account"
            );
        }
        // Relative/empty CODEX_HOME is ignored like the CLI ignores an empty value.
        let rel = discover(&home, Some(Path::new("relative")), &AccountPrefs::default());
        assert!(rel[0].profile.path.ends_with(".codex"));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn following_a_detected_home_switches_to_multi_with_primary_and_it_visible() {
        let home = root("follow");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".codex-work")).unwrap();
        std::fs::create_dir_all(home.join(".codex-perso")).unwrap();
        let prefs = AccountPrefs::default();
        let found = discover(&home, None, &prefs);
        let work = found.iter().find(|a| a.profile.name == "work").unwrap();
        let next = follow(
            &home,
            None,
            &prefs,
            FollowTarget::Detected(&work.profile.id),
        )
        .unwrap();
        assert_eq!(next.mode, "multi");
        assert_eq!(visible(&home, &next), vec!["Principal", "work"]);
        let after = discover(&home, None, &next);
        assert!(
            after
                .iter()
                .find(|a| a.profile.name == "work")
                .unwrap()
                .tracked
        );
        // The other detected home is followed too, but not displayed.
        assert!(
            after
                .iter()
                .find(|a| a.profile.name == "perso")
                .unwrap()
                .tracked
        );
        assert!(
            !after
                .iter()
                .find(|a| a.profile.name == "perso")
                .unwrap()
                .visible
        );
        let again =
            follow(&home, None, &next, FollowTarget::Detected(&work.profile.id)).unwrap_err();
        assert!(again.contains("déjà affiché"), "{again}");
        let via_path = follow(
            &home,
            None,
            &prefs,
            FollowTarget::Path(home.join(".codex-work").to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(via_path, next);
        assert!(follow(&home, None, &prefs, FollowTarget::Detected("profile:gone")).is_err());
        let primary = follow(
            &home,
            None,
            &prefs,
            FollowTarget::Path(home.join(".codex").to_str().unwrap()),
        )
        .unwrap_err();
        assert!(primary.contains("déjà affiché"), "{primary}");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_chosen_folder_keeps_its_raw_launch_value_and_bad_paths_are_refused() {
        let home = root("manual");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        let elsewhere = home.join("Elsewhere").join("codex-pro");
        std::fs::create_dir_all(&elsewhere).unwrap();
        // Native separator: on Windows the canonical temp root is a verbatim `\\?\` path,
        // where a trailing `/` is not a separator and the folder would not resolve.
        let raw = format!(
            "{}{}",
            elsewhere.to_str().unwrap(),
            std::path::MAIN_SEPARATOR
        );
        let next = follow(
            &home,
            None,
            &AccountPrefs::default(),
            FollowTarget::Path(&raw),
        )
        .unwrap();
        let added = next.profiles.iter().find(|p| p.path == raw).unwrap();
        assert_eq!(added.name, "codex-pro");
        let extra = discover(&home, None, &next)
            .into_iter()
            .find(|a| !a.primary)
            .unwrap();
        assert!(extra.tracked && extra.visible);
        assert_eq!(extra.launch.as_deref(), Some(Path::new(&raw)));
        #[cfg(unix)]
        {
            let alias = home.join("alias");
            std::os::unix::fs::symlink(&elsewhere, &alias).unwrap();
            let err = follow(
                &home,
                None,
                &next,
                FollowTarget::Path(alias.to_str().unwrap()),
            )
            .unwrap_err();
            assert!(err.contains("déjà affiché"), "{err}");
        }
        let p = AccountPrefs::default();
        let missing = follow(
            &home,
            None,
            &p,
            FollowTarget::Path(home.join("missing").to_str().unwrap()),
        )
        .unwrap_err();
        assert!(missing.contains("introuvable"), "{missing}");
        assert!(follow(&home, None, &p, FollowTarget::Path("relative/dir")).is_err());
        std::fs::write(home.join("file"), "x").unwrap();
        assert!(follow(
            &home,
            None,
            &p,
            FollowTarget::Path(home.join("file").to_str().unwrap())
        )
        .is_err());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn untracking_stops_following_without_touching_files() {
        let home = root("untrack");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".codex-work")).unwrap();
        std::fs::write(home.join(".codex-work/sentinel"), "kept").unwrap();
        let prefs = AccountPrefs::default();
        let id = discover(&home, None, &prefs)[1].profile.id.clone();
        let followed = follow(&home, None, &prefs, FollowTarget::Detected(&id)).unwrap();
        let primary = discover(&home, None, &followed)[0].profile.id.clone();
        assert!(
            untrack(&home, None, &followed, &primary).is_err(),
            "primary is never removed"
        );
        let back = untrack(&home, None, &followed, &id).unwrap();
        assert_eq!(back.mode, "single");
        assert!(back.selected_id.is_none());
        // Removed: not listed again by the next scan, remembered in the prefs.
        let found = discover(&home, None, &back);
        assert_eq!(found.len(), 1, "removed home is not detected back");
        assert_eq!(back.excluded, vec![id.clone()]);
        assert_eq!(visible(&home, &back), vec!["Principal"]);
        assert_eq!(
            std::fs::read_to_string(home.join(".codex-work/sentinel")).unwrap(),
            "kept"
        );
        assert!(home.join(".codex-work").is_dir(), "the folder itself stays");
        assert!(untrack(&home, None, &back, &id).is_err());
        // The + brings it back explicitly (exclusion forgotten).
        let again = follow(
            &home,
            None,
            &back,
            FollowTarget::Path(home.join(".codex-work").to_str().unwrap()),
        )
        .unwrap();
        assert!(again.excluded.is_empty());
        assert_eq!(visible(&home, &again), vec!["Principal", "work"]);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn removal_survives_the_next_scan_and_old_prefs_still_load() {
        let home = root("excluded");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".codex-a")).unwrap();
        std::fs::create_dir_all(home.join(".codex-b")).unwrap();
        // Old prefs.json (no `excluded` key) load unchanged and follow detection.
        let old: AccountPrefs = serde_json::from_str(
            r#"{"mode":"multi","selectedId":null,"profiles":[{"id":"x","path":"/p","name":"Perso","visible":false}]}"#,
        )
        .unwrap();
        assert!(old.excluded.is_empty());
        assert_eq!(old.profiles[0].name, "Perso");
        let prefs = AccountPrefs {
            mode: "multi".into(),
            ..AccountPrefs::default()
        };
        let a = discover(&home, None, &prefs)
            .into_iter()
            .find(|x| x.profile.name == "a")
            .unwrap();
        let next = untrack(&home, None, &prefs, &a.profile.id).unwrap();
        for _ in 0..2 {
            assert_eq!(visible(&home, &next), vec!["Principal", "b"]);
        }
        validate(&next).unwrap();
        // Round-trips through prefs.json, and a corrupted list is refused.
        let json = serde_json::to_string(&next).unwrap();
        assert!(json.contains("\"excluded\""));
        let mut bad = next.clone();
        bad.excluded.push("not-an-identity".into());
        assert!(validate(&bad).is_err());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_saved_name_and_visibility_are_kept_and_the_order_stays_stable() {
        let home = root("saved");
        for d in [".codex", ".codex-a", ".codex-b"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let b = discover(&home, None, &AccountPrefs::default())
            .into_iter()
            .find(|x| x.profile.name == "b")
            .unwrap();
        let prefs = AccountPrefs {
            mode: "multi".into(),
            profiles: vec![Profile {
                name: "Travail".into(),
                visible: false,
                ..b.profile
            }],
            ..AccountPrefs::default()
        };
        validate(&prefs).unwrap();
        let found = discover(&home, None, &prefs);
        let names: Vec<_> = found.iter().map(|x| x.profile.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Principal", "a", "Travail"],
            "renaming does not reorder"
        );
        assert!(
            !found[2].visible,
            "saved visibility wins over the Multi default"
        );
        assert!(
            found[1].visible,
            "unsaved auto-added home is shown in Multi"
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn unique_selection_and_multi_visibility_are_display_only() {
        let home = root("visibility");
        for d in [".codex", ".codex-a", ".codex-b"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let mut prefs = AccountPrefs::default();
        for name in ["a", "b"] {
            let id = discover(&home, None, &prefs)
                .into_iter()
                .find(|x| x.profile.name == name)
                .unwrap()
                .profile
                .id;
            prefs = follow(&home, None, &prefs, FollowTarget::Detected(&id)).unwrap();
        }
        assert_eq!(visible(&home, &prefs), vec!["Principal", "a", "b"]);
        // Hide "a": still followed (collected), only not displayed.
        for p in &mut prefs.profiles {
            if p.name == "a" {
                p.visible = false;
            }
        }
        validate(&prefs).unwrap();
        let found = discover(&home, None, &prefs);
        let a = found.iter().find(|x| x.profile.name == "a").unwrap();
        assert!(a.tracked && !a.visible);
        assert_eq!(visible(&home, &prefs), vec!["Principal", "b"]);
        // Unique: only the selected followed account.
        prefs.mode = "single".into();
        prefs.selected_id = Some(a.profile.id.clone());
        assert_eq!(visible(&home, &prefs), vec!["a"]);
        // A selection that is not followed falls back to the primary.
        prefs.selected_id = Some("profile:gone".into());
        assert_eq!(visible(&home, &prefs), vec!["Principal"]);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn following_without_a_usable_primary_selects_it_in_unique() {
        let home = root("zero");
        let extra = home.join("work");
        std::fs::create_dir_all(&extra).unwrap();
        let next = follow(
            &home,
            None,
            &AccountPrefs::default(),
            FollowTarget::Path(extra.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(next.mode, "single");
        assert_eq!(visible(&home, &next), vec!["work"]);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn validation_rejects_relative_duplicate_and_too_many_homes() {
        let home = root("validate");
        let mut prefs = AccountPrefs::default();
        assert!(validate(&prefs).is_ok());
        prefs.mode = "odd".into();
        assert!(validate(&prefs).unwrap_err().contains("Codex"));
        prefs.mode = "multi".into();
        let dir = home.join("a");
        std::fs::create_dir_all(&dir).unwrap();
        let ok = Profile {
            id: identity(&normalized(&dir)),
            path: dir.to_str().unwrap().into(),
            name: "A".into(),
            visible: true,
        };
        prefs.profiles.push(ok.clone());
        assert!(validate(&prefs).is_ok());
        prefs.profiles.push(ok.clone());
        assert!(validate(&prefs).is_err(), "duplicate");
        prefs.profiles.pop();
        prefs.profiles.push(Profile {
            path: "relative".into(),
            ..ok.clone()
        });
        assert!(validate(&prefs).is_err(), "relative");
        prefs.profiles.pop();
        for i in 0..=2 * MAX_EXTRA {
            let d = home.join(format!("h{i}"));
            std::fs::create_dir_all(&d).unwrap();
            prefs.profiles.push(Profile {
                id: identity(&normalized(&d)),
                path: d.to_str().unwrap().into(),
                name: format!("H{i}"),
                visible: true,
            });
        }
        assert!(validate(&prefs).is_err(), "bounded number of saved homes");
        std::fs::remove_dir_all(home).unwrap();
    }

    // ------------------------------------------------------------ aip (dqn/aip v0.11.0)
    // Synthetic fixtures only: fake homes under the temp dir, fake profile
    // names, sentinel payloads that are never credentials and must stay intact.
    const SENTINEL: &str = "SYNTHETIC-SENTINEL-NOT-A-CREDENTIAL";
    fn aip_dir(home: &Path) -> PathBuf {
        home.join(".codex").join(crate::aip::PROFILES_DIR)
    }
    fn aip_profile(home: &Path, name: &str, complete: bool) -> PathBuf {
        let dir = aip_dir(home).join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if complete {
            std::fs::write(dir.join(crate::aip::CODEX_PAYLOAD), SENTINEL).unwrap();
        }
        dir
    }
    fn set_current(home: &Path, name: &str) {
        std::fs::write(
            aip_dir(home).join(crate::aip::CURRENT_MARKER),
            format!("{name}\n"),
        )
        .unwrap();
    }
    fn names(found: &[Discovered]) -> Vec<String> {
        found.iter().map(|a| a.profile.name.clone()).collect()
    }
    fn multi() -> AccountPrefs {
        AccountPrefs {
            mode: "multi".into(),
            ..AccountPrefs::default()
        }
    }
    /// Launch values the real collector would use (`codex::collection_plan`).
    fn launched(found: &[Discovered]) -> Vec<PathBuf> {
        crate::codex::collection_plan(found.to_vec())
            .into_iter()
            .map(|a| a.launch.unwrap())
            .collect()
    }
    /// aip homes read with their credential (never launched), raw folders.
    fn read_by_credential(found: &[Discovered]) -> Vec<PathBuf> {
        crate::codex::credential_plan(found.to_vec())
            .into_iter()
            .map(|a| a.credential.unwrap())
            .collect()
    }
    /// Every file under the aip folder with its bytes (proves nothing changed).
    fn aip_state(home: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = vec![];
        let mut stack = vec![aip_dir(home)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push((p.clone(), std::fs::read(&p).unwrap_or_default()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn aip_homes_are_listed_by_name_read_by_credential_and_never_change_the_mode() {
        let home = root("aip-discover");
        std::fs::create_dir_all(home.join(".codex-work")).unwrap();
        aip_profile(&home, "boulot", true);
        let perso = aip_profile(&home, "perso", true);
        aip_profile(&home, "vide", false);
        set_current(&home, "perso");
        let before = aip_state(&home);
        let prefs = AccountPrefs::default();
        let found = discover(&home, None, &prefs);
        // Primary, ~/.codex-* siblings, then aip in aip order (alphabetical
        // without `_order`). Codex's `_current` only selects the next
        // `aip codex` launch: not folded into the primary.
        assert_eq!(
            names(&found),
            ["Principal", "work", "boulot", "perso", "vide"]
        );
        assert_eq!(found.iter().filter(|a| a.primary).count(), 1);
        assert!(found[0].tracked && found[0].launch.is_none() && found[0].blocker.is_none());
        assert!(found[1].tracked && found[1].blocker.is_none());
        let p = &found[3];
        assert_eq!(p.blocker, None);
        assert_eq!(
            p.credential.as_deref(),
            Some(perso.as_path()),
            "read from its own auth.json"
        );
        assert_eq!(
            p.profile.id,
            identity(&perso),
            "path identity, like any home"
        );
        assert_eq!(p.profile.path, perso.to_str().unwrap(), "raw path kept");
        assert!(
            p.available && p.tracked && p.launch.is_none(),
            "never an app-server child"
        );
        assert_eq!(found[4].blocker, Some(Blocker::Incomplete));
        assert!(!found[4].tracked && found[4].credential.is_none());
        assert!(found[2..].iter().all(|a| a.launch.is_none()));
        // Unique untouched: the primary alone is displayed, prefs unchanged.
        assert_eq!(visible(&home, &prefs), ["Principal"]);
        assert_eq!(prefs, AccountPrefs::default());
        // Multi: readable aip homes displayed by default, the incomplete one not.
        assert_eq!(
            visible(&home, &multi()),
            ["Principal", "work", "boulot", "perso"]
        );
        // Only the ordinary sibling gets a child; the aip homes are read with
        // their credential; stable across rescans (startup and every refresh).
        assert_eq!(launched(&found), [home.join(".codex-work")]);
        assert_eq!(
            read_by_credential(&found),
            [aip_dir(&home).join("boulot"), perso.clone()]
        );
        assert_eq!(discover(&home, None, &prefs), found);
        // aip ignores CODEX_HOME: with a custom primary, the aip homes are
        // still those of ~/.codex, still read by credential; ~/.codex is a sibling.
        let custom = home.join("custom");
        std::fs::create_dir_all(&custom).unwrap();
        let other = discover(&home, Some(&custom), &multi());
        assert_eq!(
            names(&other),
            ["Principal", ".codex", "work", "boulot", "perso", "vide"]
        );
        assert_eq!(
            launched(&other),
            [home.join(".codex"), home.join(".codex-work")]
        );
        assert_eq!(read_by_credential(&other).len(), 2);
        assert_eq!(aip_state(&home), before, "nothing read into or written");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn discovery_without_aip_is_identical_to_the_3_2_0_discovery() {
        let home = root("aip-legacy");
        for d in [".codex", ".codex-a", ".codex-b", "Elsewhere", "custom"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        // More siblings than the collector can run (MAX_EXTRA bound).
        for i in 0..MAX_EXTRA + 2 {
            std::fs::create_dir_all(home.join(format!(".codex-x{i:02}"))).unwrap();
        }
        std::fs::write(home.join(".codex-file"), "not a dir").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(home.join(".codex-a"), home.join(".codex-alias")).unwrap();
        let entry = |raw: String, name: &str, visible: bool| Profile {
            id: identity(&normalized(Path::new(&raw))),
            path: raw,
            name: name.into(),
            visible,
        };
        let sep = std::path::MAIN_SEPARATOR;
        let elsewhere = format!("{}{sep}", home.join("Elsewhere").display());
        let gone = home.join("gone").to_string_lossy().into_owned();
        let b = home.join(".codex-b");
        let custom = home.join("custom");
        for mode in ["single", "multi"] {
            for selected in [None, Some(identity(&b)), Some("profile:00".to_string())] {
                for env in [None, Some(custom.as_path())] {
                    let prefs = AccountPrefs {
                        mode: mode.into(),
                        selected_id: selected.clone(),
                        profiles: vec![
                            entry(elsewhere.clone(), "Ailleurs", true),
                            entry(b.to_string_lossy().into_owned(), "B", false),
                            entry(gone.clone(), "Parti", true),
                        ],
                        excluded: vec![identity(&home.join(".codex-a"))],
                    };
                    let old = legacy_discover(&home, env, &prefs);
                    let new = discover(&home, env, &prefs);
                    assert_eq!(old, new, "{mode} {selected:?} {env:?}");
                    assert!(new
                        .iter()
                        .all(|a| a.blocker.is_none() && a.credential.is_none()));
                }
            }
        }
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn absent_or_corrupted_aip_data_never_fails_discovery() {
        let home = root("aip-corrupt");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        assert_eq!(names(&discover(&home, None, &multi())), ["Principal"]);
        // `profiles` is a file, then an empty folder.
        std::fs::write(aip_dir(&home), "not a folder").unwrap();
        assert_eq!(names(&discover(&home, None, &multi())), ["Principal"]);
        std::fs::remove_file(aip_dir(&home)).unwrap();
        std::fs::create_dir_all(aip_dir(&home)).unwrap();
        assert_eq!(names(&discover(&home, None, &multi())), ["Principal"]);
        // Garbage around one valid profile.
        aip_profile(&home, "ok", true);
        aip_profile(&home, "bad name", true);
        std::fs::write(aip_dir(&home).join("stray"), SENTINEL).unwrap();
        std::fs::write(
            aip_dir(&home).join(crate::aip::CURRENT_MARKER),
            [0xff, 0x00],
        )
        .unwrap();
        std::fs::write(
            aip_dir(&home).join(crate::aip::ORDER_FILE),
            vec![b'x'; 9000],
        )
        .unwrap();
        // An auth.json that is a folder is not a payload: incomplete.
        std::fs::create_dir_all(aip_dir(&home).join("odd").join(crate::aip::CODEX_PAYLOAD))
            .unwrap();
        let found = discover(&home, None, &multi());
        assert_eq!(names(&found), ["Principal", "odd", "ok"]);
        assert_eq!(found[1].blocker, Some(Blocker::Incomplete));
        assert!(found[1].launch.is_none() && !found[1].visible && !found[1].tracked);
        assert_eq!(found[2].blocker, None);
        assert!(found[2].launch.is_none() && found[2].visible && found[2].credential.is_some());
        assert!(launched(&found).is_empty());
        assert_eq!(read_by_credential(&found), [aip_dir(&home).join("ok")]);
        set_current(&home, "ghost");
        assert_eq!(discover(&home, None, &multi()).len(), 3);
        // No ~/.codex at all: the primary stays explainable, no aip error.
        let empty = root("aip-corrupt-none");
        let found = discover(&empty, None, &multi());
        assert_eq!(names(&found), ["Principal"]);
        assert!(!found[0].available);
        std::fs::remove_dir_all(home).unwrap();
        std::fs::remove_dir_all(empty).unwrap();
    }

    #[test]
    fn renamed_or_deleted_aip_homes_keep_user_choices_and_can_be_removed() {
        let home = root("aip-rename");
        std::fs::create_dir_all(home.join(".codex-work")).unwrap();
        let boulot = aip_profile(&home, "boulot", true);
        aip_profile(&home, "temp", true);
        let id = identity(&boulot);
        // Detected only: readable, displayed in Multi like a sibling.
        let prefs = multi();
        let found = discover(&home, None, &prefs);
        assert_eq!(names(&found), ["Principal", "work", "boulot", "temp"]);
        assert!(found[2].visible && found[2].tracked, "its own card");
        assert_eq!(found[2].blocker, None);
        // Unique with an aip home selected: it is the displayed account.
        let mut single = prefs.clone();
        single.mode = "single".into();
        single.selected_id = Some(id.clone());
        assert_eq!(visible(&home, &single), ["boulot"]);
        assert_eq!(launched(&found), [home.join(".codex-work")]);
        assert_eq!(read_by_credential(&found).len(), 2);
        // aip rename = new folder = new identity; aip delete of an unsaved
        // profile: simply gone. Nothing is launched either way.
        std::fs::rename(&boulot, aip_dir(&home).join("pro")).unwrap();
        std::fs::remove_dir_all(aip_dir(&home).join("temp")).unwrap();
        let after = discover(&home, None, &prefs);
        assert_eq!(names(&after), ["Principal", "work", "pro"]);
        assert!(after.iter().all(|a| a.profile.id != id));
        let pro = after[2].clone();
        assert_eq!(pro.blocker, None);
        assert_eq!(launched(&after), [home.join(".codex-work")]);
        assert_eq!(read_by_credential(&after), [aip_dir(&home).join("pro")]);
        // Removing the aip home is remembered (a preference only: the
        // folder stays untouched) and never adds a saved entry.
        let next = untrack(&home, None, &prefs, &pro.profile.id).unwrap();
        assert!(next.profiles.is_empty() && next.excluded == [pro.profile.id.clone()]);
        assert_eq!(names(&discover(&home, None, &next)), ["Principal", "work"]);
        assert_eq!(next.mode, "multi", "work is still a followed extra");
        assert!(aip_dir(&home)
            .join("pro")
            .join(crate::aip::CODEX_PAYLOAD)
            .is_file());
        // The primary still cannot be removed.
        let primary = after[0].profile.id.clone();
        assert!(untrack(&home, None, &next, &primary).is_err());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn duplicates_of_aip_homes_are_one_credential_entry_and_explicit_adds_behave_like_siblings() {
        let home = root("aip-dupes");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        let perso = aip_profile(&home, "perso", true);
        let boulot = aip_profile(&home, "boulot", true);
        let vide = aip_profile(&home, "vide", false);
        set_current(&home, "perso");
        let sep = std::path::MAIN_SEPARATOR;
        let saved_perso = format!("{}{sep}", perso.display());
        let prefs = multi();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&boulot, home.join(".codex-boulot")).unwrap();
        let found = discover(&home, None, &prefs);
        #[cfg(unix)]
        {
            // One entry per home: the sibling link is aip's « boulot »
            // (first spelling keeps its place/raw path).
            assert_eq!(names(&found), ["Principal", "boulot", "perso", "vide"]);
            assert_eq!(
                found[1].profile.path,
                home.join(".codex-boulot").to_str().unwrap()
            );
            assert_eq!(found[1].blocker, None);
            assert_eq!(
                found[1].credential.as_deref(),
                Some(home.join(".codex-boulot").as_path()),
                "the link's own spelling is the credential folder"
            );
        }
        let p = found.iter().find(|a| a.profile.name == "perso").unwrap();
        assert_eq!(p.blocker, None);
        // Never a second child: the primary is the only launched account; the
        // two complete aip homes are read with their credential once each.
        assert!(launched(&found).is_empty());
        assert_eq!(read_by_credential(&found).len(), 2);
        // Explicit add of an already displayed aip home (folder picker, typed
        // path with another spelling, detected id): same answer as any
        // displayed account, nothing saved.
        let id_boulot = identity(&boulot);
        for target in [
            FollowTarget::Path(boulot.to_str().unwrap()),
            FollowTarget::Path(&saved_perso),
            FollowTarget::Detected(&id_boulot),
        ] {
            let err = follow(&home, None, &prefs, target).unwrap_err();
            assert!(err.contains("déjà affiché"), "{err}");
        }
        let err = follow(
            &home,
            None,
            &prefs,
            FollowTarget::Path(vide.to_str().unwrap()),
        )
        .unwrap_err();
        assert!(err.contains("incomplet"), "{err}");
        // aip homes never consume the MAX_EXTRA bound of real children.
        for i in 0..MAX_EXTRA {
            std::fs::create_dir_all(home.join(format!(".codex-h{i}"))).unwrap();
        }
        let many = discover(&home, None, &multi());
        assert_eq!(launched(&many).len(), MAX_EXTRA);
        assert_eq!(read_by_credential(&many).len(), 2);
        assert_eq!(many.iter().filter(|a| a.blocker.is_some()).count(), 1);
        std::fs::remove_dir_all(home).unwrap();
    }

    /// A home under `~/.codex/profiles` that the user had added by hand in
    /// 3.2.0 (saved entry) keeps its name, visibility and raw saved path, but
    /// is now read like every aip home: with its own credential, never as an
    /// app-server child (a launched copy of the primary's grant could rotate
    /// the primary's token). An incomplete one stays blocked even when saved.
    #[test]
    fn a_home_inside_aip_added_by_hand_before_this_version_is_read_by_credential() {
        let home = root("aip-legacy-manual");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        let perso = aip_profile(&home, "perso", true);
        let vide = aip_profile(&home, "vide", false);
        aip_profile(&home, "boulot", true);
        set_current(&home, "perso");
        let before = aip_state(&home);
        let sep = std::path::MAIN_SEPARATOR;
        let raw = format!("{}{sep}", perso.display());
        for mode in ["multi", "single"] {
            let mut prefs = AccountPrefs {
                mode: mode.into(),
                selected_id: Some(identity(&perso)),
                ..AccountPrefs::default()
            };
            for (path, name) in [
                (raw.clone(), "Perso"),
                (vide.to_string_lossy().into(), "Vide"),
            ] {
                prefs.profiles.push(Profile {
                    id: identity(&normalized(Path::new(&path))),
                    path,
                    name: name.into(),
                    visible: true,
                });
            }
            validate(&prefs).unwrap();
            let old = legacy_discover(&home, None, &prefs);
            let new = discover(&home, None, &prefs);
            let o = old.iter().find(|a| a.profile.name == "Perso").unwrap();
            let n = new.iter().find(|a| a.profile.name == "Perso").unwrap();
            assert_eq!(
                (&n.profile, n.tracked, n.available, n.visible),
                (&o.profile, o.tracked, o.available, o.visible),
                "{mode}: saved name, raw path and visibility kept"
            );
            assert_eq!(n.launch, None, "{mode}: never an app-server child");
            assert_eq!(n.credential.as_deref(), Some(Path::new(&raw)), "{mode}");
            assert!(
                launched(&new).is_empty(),
                "{mode}: no aip spelling is launched"
            );
            let n_vide = new.iter().find(|a| a.profile.name == "Vide").unwrap();
            assert_eq!(n_vide.blocker, Some(Blocker::Incomplete));
            assert!(!n_vide.tracked && n_vide.credential.is_none());
            let boulot = new.iter().find(|a| a.profile.name == "boulot").unwrap();
            assert_eq!(boulot.blocker, None);
            assert!(boulot.launch.is_none() && boulot.tracked && boulot.credential.is_some());
            // Still manageable like any followed home: an explicit add says it
            // is already displayed (no aip refusal), removal works.
            if mode == "multi" {
                let err = follow(&home, None, &prefs, FollowTarget::Path(&raw)).unwrap_err();
                assert!(err.contains("déjà affiché"), "{err}");
            }
            let next = untrack(&home, None, &prefs, &identity(&perso)).unwrap();
            assert!(next.profiles.iter().all(|p| p.id != identity(&perso)));
            // Once removed, the home is excluded; it is never re-added silently.
            assert!(discover(&home, None, &next)
                .iter()
                .all(|a| a.profile.id != identity(&perso)));
            // Adding it back reinstates it as a credential-read aip home.
            let back = follow(&home, None, &next, FollowTarget::Path(&raw)).unwrap();
            let again = discover(&home, None, &back);
            let p = again
                .iter()
                .find(|a| a.profile.id == identity(&perso))
                .unwrap();
            assert!(p.tracked && p.visible && p.launch.is_none() && p.credential.is_some());
        }
        assert_eq!(aip_state(&home), before, "discovery reads nothing");
        std::fs::remove_dir_all(home).unwrap();
    }

    /// The `MAX_EXTRA + 1` bound of `validate` counts saved entries only.
    /// Detected aip homes are not written to the preferences by discovery or
    /// removal (untrack only excludes them), so they cannot crowd out
    /// followable homes nor make a full but valid list invalid; the credential
    /// plan is bounded apart from the app-server plan.
    #[test]
    fn aip_homes_never_take_a_slot_of_the_followed_homes_bound() {
        let home = root("aip-cap");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        for name in ["a", "b", "c", "d"] {
            aip_profile(&home, name, name != "d");
        }
        // MAX_EXTRA - 1 followed (saved) homes, plus the primary's entry.
        let mut prefs = multi();
        let entry = |p: &Path| Profile {
            id: identity(&normalized(p)),
            path: p.to_string_lossy().into_owned(),
            name: p.file_name().unwrap().to_string_lossy().into_owned(),
            visible: true,
        };
        prefs.profiles.push(entry(&home.join(".codex")));
        for i in 0..MAX_EXTRA - 1 {
            let p = home.join(format!("Elsewhere{i}"));
            std::fs::create_dir_all(&p).unwrap();
            prefs.profiles.push(entry(&p));
        }
        validate(&prefs).unwrap();
        // Removing every aip home: preferences grow by exclusions only.
        let mut removed = prefs.clone();
        for a in discover(&home, None, &prefs)
            .into_iter()
            .filter(|a| a.blocker.is_some() || a.credential.is_some())
        {
            removed = untrack(&home, None, &removed, &a.profile.id).unwrap();
        }
        assert_eq!(removed.profiles, prefs.profiles);
        assert_eq!(removed.excluded.len(), 4);
        // The last slot still goes to a real home, with 4 aip homes listed.
        let last = home.join("Last");
        std::fs::create_dir_all(&last).unwrap();
        let full = follow(
            &home,
            None,
            &prefs,
            FollowTarget::Path(last.to_str().unwrap()),
        )
        .unwrap();
        validate(&full).unwrap();
        // The 3 displayed aip homes are saved like siblings would be; the
        // saved-list bound counts both kinds.
        assert_eq!(full.profiles.len(), MAX_EXTRA + 1 + 3);
        let found = discover(&home, None, &full);
        assert_eq!(found.iter().filter(|a| a.blocker.is_some()).count(), 1);
        assert_eq!(read_by_credential(&found).len(), 3);
        assert_eq!(launched(&found).len(), MAX_EXTRA);
        assert!(launched(&found).contains(&last));
        // An unsaved ~/.codex-* home beyond the bound is still cut, not an aip
        // one counted in its place (aip homes are never in the launch plan).
        std::fs::create_dir_all(home.join(".codex-extra")).unwrap();
        let found = discover(&home, None, &full);
        assert_eq!(launched(&found).len(), MAX_EXTRA);
        assert_eq!(read_by_credential(&found).len(), 3);
        assert!(found
            .iter()
            .all(|a| (a.blocker.is_none() && a.credential.is_none()) || a.launch.is_none()));
        // More complete aip homes than the credential bound: cut there too.
        for i in 0..MAX_EXTRA {
            aip_profile(&home, &format!("many{i}"), true);
        }
        let found = discover(&home, None, &full);
        assert_eq!(read_by_credential(&found).len(), MAX_EXTRA);
        assert_eq!(launched(&found).len(), MAX_EXTRA);
        std::fs::remove_dir_all(home).unwrap();
    }
}
