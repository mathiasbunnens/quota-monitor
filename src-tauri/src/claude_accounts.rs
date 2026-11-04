//! Profile paths and view preferences only. Never reads authentication material.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub path: String,
    pub name: String,
    pub visible: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountPrefs {
    pub mode: String,
    pub selected_id: Option<String>,
    pub profiles: Vec<Profile>,
    /// Identities of detected folders the user removed: automatic detection
    /// never brings them back (an explicit add does). Absent in old prefs.
    pub excluded: Vec<String>,
}
impl Default for AccountPrefs {
    fn default() -> Self {
        Self {
            mode: "single".into(),
            selected_id: None,
            profiles: vec![],
            excluded: vec![],
        }
    }
}
/// Upper bound of remembered exclusions (identities are short hex strings).
pub const MAX_EXCLUDED: usize = 64;
impl AccountPrefs {
    pub(crate) fn validate_excluded(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        if self.excluded.len() > MAX_EXCLUDED
            || !self
                .excluded
                .iter()
                .all(|id| id.starts_with("profile:") && id.len() <= 4096 && seen.insert(id))
        {
            return Err("Liste des comptes retirés invalide.".into());
        }
        Ok(())
    }
    /// Forget a removed folder (explicit add) / remember it (removal).
    pub(crate) fn unexclude(&mut self, id: &str) -> bool {
        let before = self.excluded.len();
        self.excluded.retain(|e| e != id);
        self.excluded.len() != before
    }
    pub(crate) fn exclude(&mut self, id: &str) {
        if !self.excluded.iter().any(|e| e == id) {
            self.excluded.push(id.to_string());
            // Bounded: the oldest exclusions give way first.
            while self.excluded.len() > MAX_EXCLUDED {
                self.excluded.remove(0);
            }
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !["single", "multi"].contains(&self.mode.as_str()) {
            return Err("Mode Claude non pris en charge.".into());
        }
        self.validate_excluded()?;
        let mut paths = std::collections::HashSet::new();
        for p in &self.profiles {
            if !Path::new(&p.path).is_absolute()
                || p.name.trim().is_empty()
                || p.name.chars().count() > 64
                || p.name.chars().any(char::is_control)
            {
                return Err(
                    "Profil Claude invalide : chemin absolu et nom de 1 à 64 caractères requis."
                        .into(),
                );
            }
            let path = normalized(Path::new(&p.path));
            if p.id != identity(&path) || !paths.insert(path) {
                return Err("Profil Claude dupliqué ou identité invalide.".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Discovered {
    pub profile: Profile,
    pub primary: bool,
    /// Exact path string for CLAUDE_CONFIG_DIR of the child (never canonicalized:
    /// Claude Code derives its credential location from the raw value). `None`
    /// for the primary profile, which must inherit the app environment untouched.
    pub launch: Option<PathBuf>,
    pub available: bool,
    pub visible: bool,
}
pub(crate) fn normalized(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}
// Reversible path encoding is collision-free and deterministic across restarts.
// This is an identity for a *local profile*, not an OAuth/account identifier.
pub fn identity(path: &Path) -> String {
    let mut id = String::from("profile:");
    use std::fmt::Write;
    for b in path.as_os_str().as_encoded_bytes() {
        let _ = write!(id, "{b:02x}");
    }
    id
}
pub fn profile_path(raw: &str) -> Result<PathBuf, String> {
    let p = Path::new(raw);
    if !p.is_absolute() {
        return Err("Le chemin du profil doit être absolu.".into());
    }
    let p = p
        .canonicalize()
        .map_err(|_| "Dossier de profil introuvable ou inaccessible : vérifiez le chemin ou utilisez « Choisir un dossier… ».")?;
    if !p.is_dir() {
        return Err("Le profil doit être un dossier.".into());
    }
    if p.to_str().is_none() {
        return Err("Le chemin du profil doit être UTF-8.".into());
    }
    Ok(p)
}
pub fn manual_profile(
    home: &Path,
    config: Option<&Path>,
    prefs: &AccountPrefs,
    raw: &str,
) -> Result<Profile, String> {
    let path = profile_path(raw)?;
    let id = identity(&path);
    if discover(home, config, prefs)
        .iter()
        .any(|a| a.profile.id == id)
    {
        return Err("Ce profil Claude est déjà détecté ou configuré.".into());
    }
    let base = path.file_name().unwrap_or_default().to_string_lossy();
    let trimmed = base.trim_start_matches(".claude-");
    Ok(Profile {
        id,
        path: raw.to_string(),
        name: if trimmed.is_empty() { &base } else { trimmed }
            .chars()
            .take(64)
            .collect(),
        visible: true,
    })
}

/// What the user explicitly chose in « Suivre un autre compte Claude ».
pub enum FollowTarget<'a> {
    /// A profile already discovered (`~/.claude-*` or saved).
    Detected(&'a str),
    /// An existing folder chosen with the folder picker or typed (absolute).
    Path(&'a str),
}

/// Explicit add flow: link an *existing* Claude Code profile folder (no login,
/// no session or account created, no content read). Detected and chosen
/// folders are deduplicated by canonical identity. The result switches to
/// Multi with what was displayed before plus the followed profile visible.
/// Background discovery never goes through here, so it never changes mode.
pub fn follow(
    home: &Path,
    config: Option<&Path>,
    prefs: &AccountPrefs,
    target: FollowTarget,
) -> Result<AccountPrefs, String> {
    // An explicit add brings back a folder the user removed earlier.
    let mut cleaned = prefs.clone();
    let reinstated = match &target {
        FollowTarget::Detected(id) => cleaned.unexclude(id),
        FollowTarget::Path(raw) => cleaned.unexclude(&identity(&profile_path(raw)?)),
    };
    let prefs = &cleaned;
    let found = discover(home, config, prefs);
    let (id, added) = match target {
        FollowTarget::Detected(id) => {
            if !found.iter().any(|a| a.profile.id == id) {
                return Err(
                    "Ce profil Claude n’est plus détecté : actualisez la liste ou choisissez son dossier."
                        .into(),
                );
            }
            (id.to_string(), None)
        }
        FollowTarget::Path(raw) => {
            let id = identity(&profile_path(raw)?);
            if found.iter().any(|a| a.profile.id == id) {
                (id, None)
            } else {
                let p = manual_profile(home, config, prefs, raw)?;
                (p.id.clone(), Some(p))
            }
        }
    };
    if let Some(existing) = found.iter().find(|a| a.profile.id == id) {
        if !existing.available {
            return Err(format!(
                "Le dossier de « {} » est indisponible : vérifiez qu’il existe encore.",
                existing.profile.name
            ));
        }
        if existing.visible && !reinstated {
            return Err(format!(
                "« {} » est déjà affiché. Choisissez un autre profil Claude Code.",
                existing.profile.name
            ));
        }
    }
    let mut next = prefs.clone();
    if !found.iter().any(|a| a.profile.id != id && a.available) {
        // No other usable profile (e.g. no ~/.claude yet): this becomes the
        // single displayed account instead of a Multi view with a ghost.
        if let Some(profile) = added {
            next.profiles.push(profile);
        }
        next.mode = "single".into();
        next.selected_id = Some(id);
        next.validate()?;
        return Ok(next);
    }
    let was_single = next.mode != "multi";
    next.mode = "multi".into();
    let mut upsert = |profile: Profile| match next.profiles.iter_mut().find(|p| p.id == profile.id)
    {
        Some(saved) => saved.visible = profile.visible,
        None => next.profiles.push(profile),
    };
    for a in &found {
        if a.profile.id == id {
            continue;
        }
        if a.visible {
            // What was displayed stays displayed (the principal when going 1 → 2).
            upsert(Profile {
                visible: true,
                ..a.profile.clone()
            });
        } else if was_single {
            // Leaving Unique: other profiles stay hidden instead of defaulting.
            upsert(Profile {
                visible: false,
                ..a.profile.clone()
            });
        }
    }
    match added {
        Some(profile) => upsert(profile),
        None => {
            let a = found.iter().find(|a| a.profile.id == id).unwrap();
            upsert(Profile {
                visible: true,
                ..a.profile.clone()
            });
        }
    }
    next.validate()?;
    Ok(next)
}

/// « Supprimer <compte> » : stops tracking only. The saved entry is forgotten
/// and the folder identity is remembered so detection does not add it back;
/// the folder and its contents are never touched. The primary stays.
pub fn remove(
    home: &Path,
    config: Option<&Path>,
    prefs: &AccountPrefs,
    id: &str,
) -> Result<AccountPrefs, String> {
    let found = discover(home, config, prefs);
    match found.iter().find(|a| a.profile.id == id) {
        Some(a) if a.primary => {
            return Err("Le compte Claude principal ne peut pas être supprimé.".into())
        }
        Some(_) => {}
        None => return Err("Ce compte Claude n’est plus suivi.".into()),
    }
    let mut next = prefs.clone();
    next.profiles.retain(|p| p.id != id);
    next.exclude(id);
    if next.selected_id.as_deref() == Some(id) {
        next.selected_id = None;
    }
    if !found.iter().any(|a| !a.primary && a.profile.id != id) {
        next.mode = "single".into();
        next.selected_id = None;
    }
    next.validate()?;
    Ok(next)
}

pub fn discover(home: &Path, config: Option<&Path>, prefs: &AccountPrefs) -> Vec<Discovered> {
    let primary = normalized(
        &config
            .filter(|p| p.is_absolute())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".claude")),
    );
    let mut paths = vec![primary.clone()];
    if let Ok(entries) = std::fs::read_dir(home) {
        let mut extras: Vec<_> = entries
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with(".claude-"))
                    && e.path().is_dir()
            })
            .map(|e| e.path())
            .collect();
        extras.sort();
        paths.extend(extras);
    }
    paths.extend(prefs.profiles.iter().map(|p| PathBuf::from(&p.path)));
    let mut seen = std::collections::HashSet::new();
    let selected = prefs.selected_id.as_deref().unwrap_or("");
    let selected_exists = paths.iter().any(|p| identity(&normalized(p)) == selected);
    let multi = prefs.mode == "multi";
    paths
        .into_iter()
        .filter(|raw| seen.insert(normalized(raw)))
        // A removed folder is not detected again (the primary cannot be removed).
        .filter(|raw| {
            let p = normalized(raw);
            p == primary || !prefs.excluded.contains(&identity(&p))
        })
        .map(|raw| {
            let p = normalized(&raw);
            let id = identity(&p);
            let is_primary = p == primary;
            let saved = prefs
                .profiles
                .iter()
                .find(|s| normalized(Path::new(&s.path)) == p);
            let profile = Profile {
                id: id.clone(),
                // Extras keep the discovered/entered string (also what the
                // settings UI writes back), so edits never canonicalize it.
                path: if is_primary { &p } else { &raw }
                    .to_string_lossy()
                    .into_owned(),
                name: saved.map(|s| s.name.clone()).unwrap_or_else(|| {
                    if is_primary {
                        "Principal".into()
                    } else {
                        p.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .trim_start_matches(".claude-")
                            .to_string()
                    }
                }),
                // Automatically added folders follow the current mode: shown in
                // Multi, kept aside in Unique (which shows the selected one).
                visible: saved.map(|s| s.visible).unwrap_or(is_primary || multi),
            };
            let visible = if prefs.mode == "multi" {
                profile.visible
            } else if selected_exists {
                selected == id
            } else {
                is_primary
            };
            Discovered {
                available: p.is_dir(),
                launch: if is_primary { None } else { Some(raw) },
                profile,
                primary: is_primary,
                visible,
            }
        })
        .collect()
}
// Refresh ownership is independent of view visibility. Invalidating never frees
// a running child: its eventual result releases the slot but cannot publish.
#[derive(Default)]
struct Entry {
    snapshot: Option<crate::claude::ClaudeSnapshot>,
    active: Option<u64>,
}
pub struct Ticket {
    id: String,
    generation: u64,
    sequence: u64,
}
pub struct RefreshStore {
    generation: u64,
    sequence: u64,
    entries: std::collections::BTreeMap<String, Entry>,
}
impl RefreshStore {
    pub const fn new() -> Self {
        Self {
            generation: 0,
            sequence: 0,
            entries: std::collections::BTreeMap::new(),
        }
    }
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        for entry in self.entries.values_mut() {
            if let Some(snap) = &mut entry.snapshot {
                if snap.state == "ready" {
                    snap.state = "stale";
                    snap.message =
                        "Dernières valeurs conservées ; nouvelle lecture requise.".into();
                }
            }
        }
    }
    pub fn begin(&mut self, id: &str) -> Option<Ticket> {
        let e = self.entries.entry(id.to_string()).or_default();
        if e.active.is_some() {
            return None;
        }
        self.sequence = self.sequence.wrapping_add(1);
        e.active = Some(self.sequence);
        Some(Ticket {
            id: id.to_string(),
            generation: self.generation,
            sequence: self.sequence,
        })
    }
    pub fn finish(&mut self, ticket: Ticket, snapshot: crate::claude::ClaudeSnapshot) -> bool {
        let Some(e) = self.entries.get_mut(&ticket.id) else {
            return false;
        };
        if e.active != Some(ticket.sequence) {
            return false;
        }
        e.active = None;
        if ticket.generation != self.generation {
            return false;
        }
        e.snapshot = Some(snapshot);
        true
    }
    pub fn snapshot(&self, id: &str) -> Option<crate::claude::ClaudeSnapshot> {
        self.entries.get(id).and_then(|e| e.snapshot.clone())
    }
    pub fn refreshing(&self, id: &str) -> bool {
        self.entries.get(id).is_some_and(|e| e.active.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("qm-accounts-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn refresh_store_isolates_accounts_rejects_overlap_and_stale_completion() {
        let mut store = RefreshStore::new();
        let a = store.begin("a").unwrap();
        assert!(store.begin("a").is_none());
        let b = store.begin("b").unwrap();
        let good = crate::claude::compute(
            true,
            true,
            crate::claude::FileState::Data(crate::claude::RelayFile {
                schema_version: 1,
                received_at: 0,
                five_hour: Some(crate::claude::Window {
                    used_percent: 25.0,
                    resets_at: None,
                }),
                seven_day: None,
            }),
            0,
        );
        assert!(store.finish(b, good.clone()));
        assert!(store.snapshot("a").is_none());
        assert_eq!(store.snapshot("b"), Some(good.clone()));
        store.invalidate();
        assert!(store.begin("a").is_none()); // old process still owns slot
        assert!(!store.finish(a, good.clone()));
        assert!(store.snapshot("a").is_none());
        let a2 = store.begin("a").unwrap();
        let failed = crate::claude::compute(true, true, crate::claude::FileState::Malformed, 0);
        assert!(store.finish(a2, failed.clone()));
        assert_eq!(store.snapshot("a"), Some(failed));
        assert_eq!(store.snapshot("b").unwrap().state, "stale");
        assert_eq!(store.snapshot("b").unwrap().metrics, good.metrics);
    }
    #[test]
    fn legacy_preferences_keep_primary_single_and_other_values() {
        let p = crate::prefs::parse(br#"{"codexEnabled":false,"claudeEnabled":true,"indicator":"both","ntfy":{"enabled":false,"serverUrl":"https://ntfy.sh","topic":"old"}}"#);
        assert!(!p.codex_enabled && p.claude_enabled);
        assert_eq!(p.indicator, "both");
        assert_eq!(p.ntfy.topic, "old");
        assert_eq!(p.claude_accounts, AccountPrefs::default());
    }
    #[test]
    fn manual_add_honors_config_primary_and_rejects_alias_duplicates() {
        let home = root("manual");
        let primary = home.join("custom-primary");
        let extra = home.join("custom-extra");
        std::fs::create_dir_all(&primary).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        let mut p = AccountPrefs::default();
        let profiles = discover(&home, Some(&primary), &p);
        assert!(profiles[0].primary && profiles[0].profile.path.ends_with("custom-primary"));
        assert!(manual_profile(&home, Some(&primary), &p, primary.to_str().unwrap()).is_err());
        let a = manual_profile(&home, Some(&primary), &p, extra.to_str().unwrap()).unwrap();
        p.selected_id = Some(a.id.clone());
        p.profiles.push(a);
        let found = discover(&home, Some(&primary), &p);
        assert_eq!(found.len(), 2);
        assert!(!found[0].visible && found[1].visible);
        assert!(manual_profile(&home, Some(&primary), &p, extra.to_str().unwrap()).is_err());
        #[cfg(unix)]
        {
            let alias = home.join("alias");
            std::os::unix::fs::symlink(&extra, &alias).unwrap();
            assert!(manual_profile(&home, Some(&primary), &p, alias.to_str().unwrap()).is_err());
        }
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn discovery_handles_zero_one_many_without_reading_contents() {
        let home = root("discovery");
        let prefs = AccountPrefs::default();
        let zero = discover(&home, None, &prefs);
        assert_eq!(zero.len(), 1); // unavailable primary remains explainable
        assert!(!zero[0].available);
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        assert_eq!(discover(&home, None, &prefs).len(), 1);
        std::fs::create_dir_all(home.join(".claude-work")).unwrap();
        std::fs::write(home.join(".claude-not-a-directory"), "ignored").unwrap();
        let many = discover(&home, None, &prefs);
        assert_eq!(many.len(), 2);
        assert!(many[0].primary && many[0].visible && !many[1].visible);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn paths_are_absolute_canonical_deduplicated_and_preferences_roundtrip() {
        let home = root("paths");
        let dir = home.join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(profile_path("relative").is_err());
        assert!(profile_path(home.join("missing").to_str().unwrap()).is_err());
        #[cfg(unix)]
        std::os::unix::fs::symlink(&dir, home.join(".claude-alias")).unwrap();
        assert_eq!(discover(&home, None, &AccountPrefs::default()).len(), 1);
        let mut p = crate::prefs::Prefs::default();
        let profiles = discover(&home, None, &p.claude_accounts);
        let mut a = profiles[0].profile.clone();
        a.name = "Personnel".into();
        a.visible = false;
        p.claude_accounts.profiles.push(a.clone());
        p.claude_accounts.mode = "multi".into();
        let round = crate::prefs::parse(&serde_json::to_vec(&p).unwrap());
        assert_eq!(p, round);
        let hidden = discover(&home, None, &round.claude_accounts);
        assert_eq!(hidden[0].profile.name, "Personnel");
        assert!(!hidden[0].visible);
        p.claude_accounts.mode = "single".into();
        assert!(discover(&home, None, &p.claude_accounts)[0].visible);
        std::fs::remove_dir_all(home).unwrap();
    }
    fn visible_ids(home: &Path, prefs: &AccountPrefs) -> Vec<String> {
        discover(home, None, prefs)
            .into_iter()
            .filter(|a| a.visible)
            .map(|a| a.profile.id)
            .collect()
    }
    #[test]
    fn startup_discovery_never_changes_the_historical_mode() {
        let home = root("startup");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let prefs = AccountPrefs::default();
        std::fs::create_dir_all(home.join(".claude-new")).unwrap();
        let found = discover(&home, None, &prefs);
        assert_eq!(found.len(), 2);
        assert_eq!(prefs.mode, "single");
        assert_eq!(
            visible_ids(&home, &prefs),
            vec![found[0].profile.id.clone()]
        );
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn following_a_detected_second_profile_switches_to_multi_with_principal_and_new_visible() {
        let home = root("follow-detected");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(home.join(".claude-work")).unwrap();
        let prefs = AccountPrefs::default();
        let found = discover(&home, None, &prefs);
        let (primary, work) = (found[0].profile.id.clone(), found[1].profile.id.clone());
        let next = follow(&home, None, &prefs, FollowTarget::Detected(&work)).unwrap();
        assert_eq!(next.mode, "multi");
        assert_eq!(
            visible_ids(&home, &next),
            vec![primary.clone(), work.clone()]
        );
        // No artificial duplicate: one saved entry per identity.
        assert_eq!(next.profiles.len(), 2);
        assert_eq!(discover(&home, None, &next).len(), 2);
        // Following it again is an actionable error, not a duplicate.
        let again = follow(&home, None, &next, FollowTarget::Detected(&work)).unwrap_err();
        assert!(again.contains("déjà affiché"), "{again}");
        // Choosing the detected folder via the picker resolves to the same profile.
        let via_path = follow(
            &home,
            None,
            &prefs,
            FollowTarget::Path(home.join(".claude-work").to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(via_path, next);
        assert!(follow(&home, None, &prefs, FollowTarget::Detected("profile:gone")).is_err());
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn following_a_chosen_folder_keeps_its_raw_launch_path_and_rejects_aliases() {
        let home = root("follow-path");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let elsewhere = home.join("Elsewhere").join("claude-pro");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let raw = format!("{}/", elsewhere.to_str().unwrap());
        let prefs = AccountPrefs::default();
        let next = follow(&home, None, &prefs, FollowTarget::Path(&raw)).unwrap();
        assert_eq!(next.mode, "multi");
        let added = next.profiles.iter().find(|p| p.path == raw).unwrap();
        assert_eq!(added.name, "claude-pro");
        let found = discover(&home, None, &next);
        assert!(found.iter().all(|a| a.visible));
        let extra = found.iter().find(|a| !a.primary).unwrap();
        assert_eq!(extra.launch.as_deref(), Some(Path::new(&raw)));
        assert!(found.iter().find(|a| a.primary).unwrap().launch.is_none());
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
        let missing = follow(
            &home,
            None,
            &prefs,
            FollowTarget::Path(home.join("missing").to_str().unwrap()),
        )
        .unwrap_err();
        assert!(missing.contains("introuvable"), "{missing}");
        assert!(follow(&home, None, &prefs, FollowTarget::Path("relative/dir")).is_err());
        let primary = home.join(".claude");
        let dup = follow(
            &home,
            None,
            &prefs,
            FollowTarget::Path(primary.to_str().unwrap()),
        )
        .unwrap_err();
        assert!(dup.contains("déjà affiché"), "{dup}");
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn following_without_any_usable_profile_selects_it_in_unique() {
        let home = root("follow-zero");
        let extra = home.join("work");
        std::fs::create_dir_all(&extra).unwrap();
        let prefs = AccountPrefs::default();
        let next = follow(
            &home,
            None,
            &prefs,
            FollowTarget::Path(extra.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(next.mode, "single");
        let found = discover(&home, None, &next);
        assert!(!found[0].available && !found[0].visible);
        assert_eq!(
            visible_ids(&home, &next),
            vec![next.selected_id.clone().unwrap()]
        );
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn following_from_unique_keeps_the_displayed_account_and_hides_the_others() {
        let home = root("follow-unique");
        for d in [".claude", ".claude-a", ".claude-b"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let found = discover(&home, None, &AccountPrefs::default());
        let ids: Vec<_> = found.iter().map(|a| a.profile.id.clone()).collect();
        let prefs = AccountPrefs {
            selected_id: Some(ids[1].clone()),
            ..AccountPrefs::default()
        };
        let next = follow(&home, None, &prefs, FollowTarget::Detected(&ids[2])).unwrap();
        assert_eq!(
            visible_ids(&home, &next),
            vec![ids[1].clone(), ids[2].clone()]
        );
        assert_eq!(next.selected_id, prefs.selected_id);
        // Already Multi: other choices are left untouched.
        let mut multi = next.clone();
        multi
            .profiles
            .iter_mut()
            .for_each(|p| p.visible = p.id == ids[1]);
        let after = follow(&home, None, &multi, FollowTarget::Detected(&ids[0])).unwrap();
        assert_eq!(
            visible_ids(&home, &after),
            vec![ids[0].clone(), ids[1].clone()]
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn removing_a_detected_profile_excludes_it_without_touching_files_and_plus_restores_it() {
        let home = root("remove");
        for d in [".claude", ".claude-a", ".claude-b"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join(".claude-a/sentinel"), "kept").unwrap();
        let found = discover(&home, None, &AccountPrefs::default());
        let ids: Vec<_> = found.iter().map(|a| a.profile.id.clone()).collect();
        // Auto-added: Unique keeps only the principal displayed, Multi shows all.
        assert_eq!(
            visible_ids(&home, &AccountPrefs::default()),
            vec![ids[0].clone()]
        );
        let multi = AccountPrefs {
            mode: "multi".into(),
            ..AccountPrefs::default()
        };
        assert_eq!(visible_ids(&home, &multi).len(), 3);
        assert!(
            remove(&home, None, &multi, &ids[0]).is_err(),
            "principal stays"
        );
        assert!(remove(&home, None, &multi, "profile:00").is_err());
        let next = remove(&home, None, &multi, &ids[1]).unwrap();
        assert_eq!(next.excluded, vec![ids[1].clone()]);
        for _ in 0..2 {
            let again = discover(&home, None, &next);
            assert_eq!(again.len(), 2, "the next scan does not add it back");
        }
        assert_eq!(
            std::fs::read_to_string(home.join(".claude-a/sentinel")).unwrap(),
            "kept"
        );
        // Removing the last extra returns to the historical single account.
        let alone = remove(&home, None, &next, &ids[2]).unwrap();
        assert_eq!(
            (alone.mode.as_str(), alone.selected_id.as_ref()),
            ("single", None)
        );
        // The + restores a removed folder (and forgets the exclusion).
        let back = follow(
            &home,
            None,
            &next,
            FollowTarget::Path(home.join(".claude-a").to_str().unwrap()),
        )
        .unwrap();
        assert!(back.excluded.is_empty());
        assert!(visible_ids(&home, &back).contains(&ids[1]));
        // Old prefs without `excluded` still load; a bad list is refused.
        let old: AccountPrefs =
            serde_json::from_str(r#"{"mode":"multi","selectedId":null,"profiles":[]}"#).unwrap();
        assert!(old.excluded.is_empty() && old.validate().is_ok());
        let mut bad = next.clone();
        bad.excluded.push("x".into());
        assert!(bad.validate().is_err());
        std::fs::remove_dir_all(home).unwrap();
    }
}
