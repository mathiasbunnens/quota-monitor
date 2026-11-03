//! Codex environments (one `CODEX_HOME` folder each): names, paths and view
//! preferences only. Never reads `auth.json`, tokens or the keychain, never
//! logs in/out and never changes the active Codex session.
//!
//! The primary environment is the historical one (inherited `CODEX_HOME`, else
//! `~/.codex`) and is launched exactly as before (no variable injected).
//! Additional environments are *followed* explicitly (detected `~/.codex-*`
//! folder or a chosen absolute folder); only followed ones get their own
//! app-server child, with `CODEX_HOME` set on that child only.
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
}

/// Same rules as the Claude profiles, Codex wording, bounded child count.
pub fn validate(prefs: &AccountPrefs) -> Result<(), String> {
    if !["single", "multi"].contains(&prefs.mode.as_str()) {
        return Err("Mode Codex non pris en charge.".into());
    }
    // Followed homes + the optional saved entry of the primary (name/visibility).
    if prefs.profiles.len() > MAX_EXTRA + 1 {
        return Err(format!(
            "Au plus {MAX_EXTRA} comptes Codex supplémentaires peuvent être suivis."
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

/// Order: primary, followed homes (saved order), then detected `~/.codex-*`
/// folders not followed yet. Only names/paths are read (`read_dir` + `is_dir`).
pub fn discover(home: &Path, env: Option<&Path>, prefs: &AccountPrefs) -> Vec<Discovered> {
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
    }];
    for saved in &prefs.profiles {
        let p = normalized(Path::new(&saved.path));
        if !seen.insert(p.clone()) {
            continue;
        }
        out.push(Discovered {
            available: p.is_dir(),
            launch: Some(PathBuf::from(&saved.path)),
            primary: false,
            tracked: true,
            visible: false,
            profile: Profile {
                id: identity(&p),
                ..saved.clone()
            },
        });
    }
    if let Ok(entries) = std::fs::read_dir(home) {
        let mut detected: Vec<_> = entries
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| (n.starts_with(PREFIX) && n.len() > PREFIX.len()) || n == ".codex")
                    && e.path().is_dir()
            })
            .map(|e| e.path())
            .collect();
        detected.sort();
        for raw in detected {
            let p = normalized(&raw);
            if !seen.insert(p.clone()) {
                continue;
            }
            out.push(Discovered {
                available: true,
                profile: Profile {
                    id: identity(&p),
                    path: raw.to_string_lossy().into_owned(),
                    name: default_name(&raw),
                    visible: false,
                },
                launch: Some(raw),
                primary: false,
                tracked: false,
                visible: false,
            });
        }
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
        if !a.available {
            return Err(format!(
                "Le dossier de « {} » est indisponible : vérifiez qu’il existe encore.",
                a.profile.name
            ));
        }
        if a.tracked && a.visible {
            return Err(format!(
                "« {} » est déjà affiché. Choisissez un autre dossier CODEX_HOME.",
                a.profile.name
            ));
        }
    }
    let followed_extras = found.iter().filter(|a| a.tracked && !a.primary).count();
    if !existing.is_some_and(|a| a.tracked) && followed_extras >= MAX_EXTRA {
        return Err(format!(
            "Au plus {MAX_EXTRA} comptes Codex supplémentaires peuvent être suivis."
        ));
    }
    let profile = existing.map(|a| a.profile.clone()).unwrap_or_else(|| Profile {
        id: id.clone(),
        path: raw.clone(),
        name: default_name(Path::new(&raw)),
        visible: true,
    });
    let mut next = prefs.clone();
    let upsert = |next: &mut AccountPrefs, profile: Profile| {
        match next.profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(saved) => saved.visible = profile.visible,
            None => next.profiles.push(profile),
        }
    };
    if !found
        .iter()
        .any(|a| a.profile.id != id && a.tracked && a.available)
    {
        // No other usable account (e.g. no ~/.codex): this one is displayed alone.
        upsert(&mut next, Profile { visible: true, ..profile });
        next.mode = "single".into();
        next.selected_id = Some(id);
        validate(&next)?;
        return Ok(next);
    }
    let was_single = next.mode != "multi";
    next.mode = "multi".into();
    for a in found.iter().filter(|a| a.tracked && a.profile.id != id) {
        if a.visible {
            upsert(&mut next, Profile { visible: true, ..a.profile.clone() });
        } else if was_single {
            upsert(&mut next, Profile { visible: false, ..a.profile.clone() });
        }
    }
    upsert(&mut next, Profile { visible: true, ..profile });
    validate(&next)?;
    Ok(next)
}

/// « Ne plus suivre » : forgets the saved entry only. The folder and its
/// contents are never touched; a `~/.codex-*` folder goes back to detected.
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
        Some(a) if a.tracked => {}
        _ => return Err("Ce compte Codex n’est pas suivi.".into()),
    }
    let mut next = prefs.clone();
    next.profiles.retain(|p| p.id != id);
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
        let p = std::env::temp_dir().join(format!("qm-codex-accounts-{name}-{}", std::process::id()));
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
    fn discovery_lists_detected_homes_by_name_only_without_following_them() {
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
        assert!(!found[1].primary && !found[1].tracked && !found[1].visible);
        assert_eq!(found[1].profile.name, "work");
        assert_eq!(found[1].launch.as_deref(), Some(home.join(".codex-work").as_path()));
        assert_eq!(
            std::fs::read_to_string(home.join(".codex-work/auth.json")).unwrap(),
            "sentinel"
        );
        // Background discovery never changes the historical mode.
        assert_eq!(prefs.mode, "single");
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
        // ~/.codex is then an ordinary detected (unfollowed) home.
        assert!(found.iter().any(|a| !a.primary && !a.tracked));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&custom, home.join(".codex-alias")).unwrap();
            let again = discover(&home, Some(&custom), &AccountPrefs::default());
            assert_eq!(again.len(), found.len(), "alias of the primary is not a second account");
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
        let next = follow(&home, None, &prefs, FollowTarget::Detected(&work.profile.id)).unwrap();
        assert_eq!(next.mode, "multi");
        assert_eq!(visible(&home, &next), vec!["Principal", "work"]);
        let after = discover(&home, None, &next);
        assert!(after.iter().find(|a| a.profile.name == "work").unwrap().tracked);
        // The other detected home stays merely detected.
        assert!(!after.iter().find(|a| a.profile.name == "perso").unwrap().tracked);
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
        let raw = format!("{}/", elsewhere.to_str().unwrap());
        let next = follow(&home, None, &AccountPrefs::default(), FollowTarget::Path(&raw)).unwrap();
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
            let err =
                follow(&home, None, &next, FollowTarget::Path(alias.to_str().unwrap())).unwrap_err();
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
        assert!(follow(&home, None, &p, FollowTarget::Path(home.join("file").to_str().unwrap())).is_err());
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
        assert!(untrack(&home, None, &followed, &primary).is_err(), "primary is never removed");
        let back = untrack(&home, None, &followed, &id).unwrap();
        assert_eq!(back.mode, "single");
        assert!(back.selected_id.is_none());
        let found = discover(&home, None, &back);
        assert!(!found[1].tracked && !found[1].visible, "back to merely detected");
        assert_eq!(visible(&home, &back), vec!["Principal"]);
        assert_eq!(
            std::fs::read_to_string(home.join(".codex-work/sentinel")).unwrap(),
            "kept"
        );
        assert!(untrack(&home, None, &back, &id).is_err());
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
        for i in 0..=MAX_EXTRA {
            let d = home.join(format!("h{i}"));
            std::fs::create_dir_all(&d).unwrap();
            prefs.profiles.push(Profile {
                id: identity(&normalized(&d)),
                path: d.to_str().unwrap().into(),
                name: format!("H{i}"),
                visible: true,
            });
        }
        assert!(validate(&prefs).is_err(), "bounded number of children");
        std::fs::remove_dir_all(home).unwrap();
    }
}
