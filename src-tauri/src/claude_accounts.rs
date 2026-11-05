//! Profile paths and view preferences only. Never reads authentication material.
//!
//! Sources (see `account_sources` and docs/aip-profiles.md): the primary
//! (inherited `CLAUDE_CONFIG_DIR`, else `~/.claude`), `~/.claude-*` folders,
//! `aip` profiles (`~/.claude/profiles/<name>`, dqn/aip v0.11.0) and saved
//! entries. aip Claude profiles are keychain snapshots, not Claude Code homes:
//! they are listed with their aip name and never launched; the collector
//! (`claude.rs`) reads them over HTTPS with the token saved in the snapshot
//! (`Collect::Credential`, see `oauth_usage`), and aip's `_current` profile is
//! folded into the primary.
use crate::account_sources::{self, Blocker, Collect, Origin};
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
    /// for the primary profile, which must inherit the app environment untouched,
    /// AND for blocked accounts, which are never launched: always match
    /// [`Discovered::collect`] first.
    pub launch: Option<PathBuf>,
    pub available: bool,
    pub visible: bool,
    /// How the app may read this account: `Inherit` (primary), `Isolated`
    /// (child with `CLAUDE_CONFIG_DIR` = `launch`), `Credential` (aip
    /// snapshot read over HTTPS with its own saved token, `profile.path` =
    /// the profile folder) or `Blocked` (listed and explained, never read:
    /// incomplete aip profile).
    pub collect: Collect,
    pub origin: Origin,
    /// aip profile names folded into this account (aip `_current` = the
    /// keychain's account = the primary). Never collected separately.
    pub aliases: Vec<String>,
}
impl Discovered {
    /// Why this account cannot be read (`None`: collectable as before).
    pub fn blocker(&self) -> Option<Blocker> {
        match self.collect {
            Collect::Blocked(b) => Some(b),
            _ => None,
        }
    }
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

/// User-facing reason of a blocked account (no path, no secret).
pub fn blocked_message(blocker: Blocker) -> &'static str {
    match blocker {
        Blocker::Incomplete => {
            "Profil aip incomplet (aucune sauvegarde credentials.json) : rien à lire."
        }
    }
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
        if let Some(blocker) = existing.blocker() {
            // Never turned into a followed home (no switch, no copy of secrets).
            return Err(blocked_message(blocker).into());
        }
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
    if !found
        .iter()
        .any(|a| a.profile.id != id && a.available && a.blocker().is_none())
    {
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
        } else if was_single && a.blocker().is_none() {
            // Leaving Unique: other profiles stay hidden instead of defaulting
            // (blocked aip snapshots are hidden by default: nothing to save).
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
    // Back to Unique when no other readable or displayed account remains.
    if !found
        .iter()
        .any(|a| !a.primary && a.profile.id != id && (a.blocker().is_none() || a.visible))
    {
        next.mode = "single".into();
        next.selected_id = None;
    }
    next.validate()?;
    Ok(next)
}

pub fn discover(home: &Path, config: Option<&Path>, prefs: &AccountPrefs) -> Vec<Discovered> {
    let mut candidates = account_sources::all(home, &account_sources::CLAUDE, config, prefs);
    route_through_aip(&mut candidates);
    account_sources::reconcile(candidates, prefs)
        .into_iter()
        .map(|r| Discovered {
            profile: r.profile(),
            primary: r.primary(),
            launch: r.candidate.isolation().map(Path::to_path_buf),
            available: r.candidate.available,
            visible: r.visible,
            collect: r.candidate.collect,
            origin: r.candidate.origin,
            aliases: r.aliases,
        })
        .collect()
}

/// A `~/.claude-*` folder or a saved path that resolves into an aip profile
/// (symlink, alias spelling) is that aip snapshot: it inherits its collection
/// mode (credential read, or blocked when incomplete) and fold, so no spelling
/// of a snapshot is ever launched as a Claude Code home.
/// The first spelling keeps its raw path and position (first source wins).
fn route_through_aip(candidates: &mut [account_sources::Candidate]) {
    let aip: Vec<_> = candidates
        .iter()
        .filter(|c| matches!(c.origin, Origin::Aip { .. }))
        .map(|c| {
            (
                c.id.clone(),
                c.name.clone(),
                c.origin,
                c.collect,
                c.same_account_as.clone(),
            )
        })
        .collect();
    for c in candidates.iter_mut() {
        if matches!(c.origin, Origin::Primary | Origin::Aip { .. }) {
            continue;
        }
        if let Some((_, name, origin, collect, same)) = aip.iter().find(|(id, ..)| *id == c.id) {
            // Default name = the aip profile name (a saved name still wins).
            c.name = name.clone();
            c.origin = *origin;
            c.collect = *collect;
            c.same_account_as = same.clone();
        }
    }
}

/// Pre-aip discovery (3.2.0), kept verbatim as the non-regression reference.
#[cfg(test)]
pub(crate) fn legacy_discover(
    home: &Path,
    config: Option<&Path>,
    prefs: &AccountPrefs,
) -> Vec<Discovered> {
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
                collect: if is_primary {
                    Collect::Inherit
                } else {
                    Collect::Isolated
                },
                origin: if is_primary {
                    Origin::Primary
                } else {
                    Origin::Manual
                },
                aliases: vec![],
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

    // ------------------------------------------------------------ aip (dqn/aip v0.11.0)
    // Synthetic fixtures only: fake homes under the temp dir, fake profile names,
    // sentinel payloads that are never credentials and must stay untouched.
    const SENTINEL: &str = "SYNTHETIC-SENTINEL-NOT-A-CREDENTIAL";
    fn fresh(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("qm-claude-aip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p.canonicalize().unwrap()
    }
    fn aip_dir(home: &Path) -> PathBuf {
        home.join(".claude").join(crate::aip::PROFILES_DIR)
    }
    fn aip_profile(home: &Path, name: &str, complete: bool) -> PathBuf {
        let dir = aip_dir(home).join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if complete {
            std::fs::write(dir.join(crate::aip::CLAUDE_PAYLOAD), SENTINEL).unwrap();
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
    fn shown(found: &[Discovered]) -> Vec<String> {
        found
            .iter()
            .filter(|a| a.visible)
            .map(|a| a.profile.name.clone())
            .collect()
    }
    fn multi() -> AccountPrefs {
        AccountPrefs {
            mode: "multi".into(),
            ..AccountPrefs::default()
        }
    }
    /// Payload bytes and the aip markers, to prove nothing was rewritten.
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
    fn aip_profiles_appear_with_their_name_and_stable_identity_and_the_current_one_is_the_primary()
    {
        let home = fresh("discover");
        std::fs::create_dir_all(home.join(".claude-work")).unwrap();
        aip_profile(&home, "perso", true);
        let boulot = aip_profile(&home, "boulot", true);
        aip_profile(&home, "vide", false);
        set_current(&home, "perso");
        let before = aip_state(&home);
        let prefs = AccountPrefs::default();
        let found = discover(&home, None, &prefs);
        // Primary, ~/.claude-* siblings, then aip in aip order; `perso` is the
        // keychain's account = the primary: folded, never a second entry.
        assert_eq!(names(&found), ["Principal", "work", "boulot", "vide"]);
        assert_eq!(found.iter().filter(|a| a.primary).count(), 1);
        let b = &found[2];
        assert_eq!(
            b.profile.id,
            identity(&boulot),
            "path identity, like any profile"
        );
        assert_eq!(b.profile.path, boulot.to_str().unwrap(), "raw path kept");
        assert!(b.available);
        // Background discovery never changes the mode: Unique shows the primary.
        assert_eq!(shown(&found), ["Principal"]);
        assert_eq!(prefs, AccountPrefs::default());
        // Multi: readable aip snapshots are displayed by default like any
        // collectable account; the incomplete one is listed only.
        assert_eq!(
            shown(&discover(&home, None, &multi())),
            ["Principal", "work", "boulot"]
        );
        assert_eq!(b.collect, Collect::Credential);
        assert!(b.launch.is_none(), "never a CLAUDE_CONFIG_DIR child");
        // Stable across scans (startup and every refresh rescan the same way).
        let again = discover(&home, None, &prefs);
        assert_eq!(
            again.iter().map(|a| &a.profile).collect::<Vec<_>>(),
            found.iter().map(|a| &a.profile).collect::<Vec<_>>()
        );
        assert_eq!(
            aip_state(&home),
            before,
            "nothing read into or written to aip files"
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn discovery_without_aip_is_identical_to_the_3_2_0_discovery() {
        let home = fresh("legacy");
        for d in [".claude", ".claude-a", ".claude-b", "Elsewhere"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join(".claude-file"), "not a dir").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(home.join(".claude-a"), home.join(".claude-alias")).unwrap();
        let custom = home.join("custom");
        std::fs::create_dir_all(&custom).unwrap();
        let entry = |raw: String, name: &str, visible: bool| Profile {
            id: identity(&normalized(Path::new(&raw))),
            path: raw,
            name: name.into(),
            visible,
        };
        let sep = std::path::MAIN_SEPARATOR;
        let elsewhere = format!("{}{sep}", home.join("Elsewhere").display());
        let gone = home.join("gone").to_string_lossy().into_owned();
        let b = home.join(".claude-b");
        for mode in ["single", "multi"] {
            for selected in [None, Some(identity(&b)), Some("profile:00".to_string())] {
                for config in [None, Some(custom.as_path())] {
                    let prefs = AccountPrefs {
                        mode: mode.into(),
                        selected_id: selected.clone(),
                        profiles: vec![
                            entry(elsewhere.clone(), "Ailleurs", true),
                            entry(b.to_string_lossy().into_owned(), "B", false),
                            entry(gone.clone(), "Parti", true),
                        ],
                        excluded: vec![identity(&home.join(".claude-a"))],
                    };
                    let old = legacy_discover(&home, config, &prefs);
                    let new = discover(&home, config, &prefs);
                    let key = |a: &Discovered| {
                        (
                            a.profile.clone(),
                            a.primary,
                            a.launch.clone(),
                            a.available,
                            a.visible,
                        )
                    };
                    assert_eq!(
                        old.iter().map(key).collect::<Vec<_>>(),
                        new.iter().map(key).collect::<Vec<_>>(),
                        "{mode} {selected:?} {config:?}"
                    );
                    assert!(new
                        .iter()
                        .all(|a| a.blocker().is_none() && a.aliases.is_empty()));
                    assert_eq!(
                        new.iter().map(|a| a.collect).collect::<Vec<_>>(),
                        new.iter()
                            .map(|a| if a.primary {
                                Collect::Inherit
                            } else {
                                Collect::Isolated
                            })
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn absent_or_corrupted_aip_data_never_fails_discovery() {
        let home = fresh("corrupt");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let base = discover(&home, None, &multi());
        assert_eq!(names(&base), ["Principal"]);
        // `profiles` is a file, then an empty folder, then garbage markers.
        std::fs::write(aip_dir(&home), "not a folder").unwrap();
        assert_eq!(names(&discover(&home, None, &multi())), ["Principal"]);
        std::fs::remove_file(aip_dir(&home)).unwrap();
        std::fs::create_dir_all(aip_dir(&home)).unwrap();
        assert_eq!(names(&discover(&home, None, &multi())), ["Principal"]);
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
        // A credentials.json that is a folder is not a payload: incomplete.
        std::fs::create_dir_all(aip_dir(&home).join("odd").join(crate::aip::CLAUDE_PAYLOAD))
            .unwrap();
        let found = discover(&home, None, &multi());
        assert_eq!(names(&found), ["Principal", "odd", "ok"]);
        assert!(
            found[0].aliases.is_empty(),
            "unreadable _current folds nothing"
        );
        assert_eq!(found[1].blocker(), Some(Blocker::Incomplete));
        assert!(found[1].launch.is_none() && !found[1].visible);
        assert_eq!(found[2].collect, Collect::Credential);
        assert!(found[2].launch.is_none() && found[2].visible);
        // `_current` naming a missing profile folds nothing either.
        set_current(&home, "ghost");
        assert!(discover(&home, None, &multi())[0].aliases.is_empty());
        // No ~/.claude at all: the primary stays explainable, no aip scan error.
        let empty = fresh("corrupt-none");
        let found = discover(&empty, None, &multi());
        assert_eq!(names(&found), ["Principal"]);
        assert!(!found[0].available);
        std::fs::remove_dir_all(home).unwrap();
        std::fs::remove_dir_all(empty).unwrap();
    }

    #[test]
    fn renamed_or_deleted_aip_profiles_follow_the_folder_rules_and_keep_user_choices() {
        let home = fresh("rename");
        std::fs::create_dir_all(home.join(".claude-work")).unwrap();
        let boulot = aip_profile(&home, "boulot", true);
        aip_profile(&home, "temp", true);
        let id = identity(&boulot);
        // The user renamed it and chose to display it (explicit choice kept).
        let mut prefs = multi();
        prefs.profiles.push(Profile {
            id: id.clone(),
            path: boulot.to_string_lossy().into_owned(),
            name: "Boulot".into(),
            visible: true,
        });
        let found = discover(&home, None, &prefs);
        assert_eq!(names(&found), ["Principal", "work", "Boulot", "temp"]);
        assert!(found[2].visible && found[2].collect == Collect::Credential);
        // aip rename = new folder = new identity; the saved entry lingers as
        // unavailable (never read: the credential file is gone with the folder).
        std::fs::rename(&boulot, aip_dir(&home).join("pro")).unwrap();
        // Unsaved profile deleted by `aip delete`: simply gone.
        std::fs::remove_dir_all(aip_dir(&home).join("temp")).unwrap();
        let after = discover(&home, None, &prefs);
        assert_eq!(names(&after), ["Principal", "work", "pro", "Boulot"]);
        let old = after.iter().find(|a| a.profile.id == id).unwrap();
        assert!(!old.available && old.profile.name == "Boulot" && old.visible);
        let pro = after.iter().find(|a| a.profile.name == "pro").unwrap();
        assert_ne!(pro.profile.id, id);
        assert_eq!(pro.collect, Collect::Credential);
        // Removing the stale entry is remembered; the folder stays untouched.
        let next = remove(&home, None, &prefs, &id).unwrap();
        assert_eq!(
            names(&discover(&home, None, &next)),
            ["Principal", "work", "pro"]
        );
        assert_eq!(next.mode, "multi", "work is still a readable extra");
        // A readable aip snapshot counts as an extra: removing the sibling keeps Multi.
        let work = identity(&home.join(".claude-work"));
        let with_pro = remove(&home, None, &next, &work).unwrap();
        assert_eq!(with_pro.mode, "multi", "pro is still a readable extra");
        // Removing the last readable extra returns to Unique (preference only,
        // the folder stays untouched).
        let pro_id = pro.profile.id.clone();
        let hidden = remove(&home, None, &with_pro, &pro_id).unwrap();
        assert_eq!(hidden.mode, "single");
        assert_eq!(names(&discover(&home, None, &hidden)), ["Principal"]);
        assert!(aip_dir(&home)
            .join("pro")
            .join(crate::aip::CLAUDE_PAYLOAD)
            .is_file());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn duplicates_of_the_active_account_and_of_snapshots_are_never_collected_twice() {
        let home = fresh("dupes");
        let perso = aip_profile(&home, "perso", true);
        let boulot = aip_profile(&home, "boulot", true);
        set_current(&home, "perso");
        let sep = std::path::MAIN_SEPARATOR;
        let mut prefs = multi();
        // Saved spellings of the same folders (trailing separator), as the
        // settings UI would write them back.
        for (dir, name) in [(&perso, "Perso"), (&boulot, "Boulot")] {
            prefs.profiles.push(Profile {
                id: identity(dir),
                path: format!("{}{sep}", dir.display()),
                name: name.into(),
                visible: true,
            });
        }
        #[cfg(unix)]
        {
            // Sibling symlinks into the aip folders.
            std::os::unix::fs::symlink(&perso, home.join(".claude-perso")).unwrap();
            std::os::unix::fs::symlink(&boulot, home.join(".claude-boulot")).unwrap();
        }
        let found = discover(&home, None, &prefs);
        // One primary, carrying aip's active profile; one Boulot entry, read
        // with its own credential (never launched, whatever its spelling).
        assert_eq!(names(&found), ["Principal", "Boulot"]);
        assert_eq!(found[0].aliases, ["Perso"]);
        assert!(found[0].launch.is_none() && found[0].collect == Collect::Inherit);
        assert_eq!(found[1].collect, Collect::Credential);
        assert!(found[1].launch.is_none());
        // Exactly one launchable account (the primary); one credential read.
        assert_eq!(
            found
                .iter()
                .filter(|a| a.collect != Collect::Credential)
                .count(),
            1
        );
        // Explicit add of an already displayed snapshot (folder picker or
        // detected id): same answer as any displayed account, preferences unchanged.
        for target in [
            FollowTarget::Path(boulot.to_str().unwrap()),
            FollowTarget::Detected(&found[1].profile.id),
        ] {
            let err = follow(&home, None, &prefs, target).unwrap_err();
            assert!(err.contains("déjà affiché"), "{err}");
        }
        // A custom CLAUDE_CONFIG_DIR: the keychain account is not the primary,
        // so aip's `_current` is no longer folded; it is listed and read alone.
        let custom = home.join("custom");
        std::fs::create_dir_all(&custom).unwrap();
        let found = discover(&home, Some(&custom), &multi());
        assert!(found[0].aliases.is_empty());
        let current = found
            .iter()
            .find(|a| a.origin == Origin::Aip { active: true })
            .unwrap();
        assert_eq!(current.collect, Collect::Credential);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn following_a_real_profile_with_aip_snapshots_around_treats_them_like_siblings() {
        let home = fresh("follow");
        std::fs::create_dir_all(home.join(".claude-work")).unwrap();
        aip_profile(&home, "boulot", true);
        let prefs = AccountPrefs::default();
        let work = identity(&home.join(".claude-work"));
        let next = follow(&home, None, &prefs, FollowTarget::Detected(&work)).unwrap();
        assert_eq!(next.mode, "multi");
        assert_eq!(shown(&discover(&home, None, &next)), ["Principal", "work"]);
        // Leaving Unique keeps the other readable accounts hidden instead of
        // defaulting them visible (same rule as a `~/.claude-*` sibling).
        let saved_aip = next
            .profiles
            .iter()
            .find(|p| p.path.contains("profiles"))
            .expect("the readable aip snapshot is saved hidden like a sibling");
        assert!(!saved_aip.visible);
        // A readable snapshot is a usable second account: a chosen folder
        // joins it in Multi.
        let solo = fresh("follow-solo");
        aip_profile(&solo, "boulot", true);
        let extra = solo.join("extra");
        std::fs::create_dir_all(&extra).unwrap();
        let next = follow(
            &solo,
            Some(&solo.join("missing-primary")),
            &AccountPrefs::default(),
            FollowTarget::Path(extra.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(
            next.mode, "multi",
            "a readable snapshot is a usable second account"
        );
        std::fs::remove_dir_all(home).unwrap();
        std::fs::remove_dir_all(solo).unwrap();
    }
}
