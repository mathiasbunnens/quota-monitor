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
}
impl Default for AccountPrefs {
    fn default() -> Self {
        Self {
            mode: "single".into(),
            selected_id: None,
            profiles: vec![],
        }
    }
}
impl AccountPrefs {
    pub fn validate(&self) -> Result<(), String> {
        if !["single", "multi"].contains(&self.mode.as_str()) {
            return Err("Mode Claude non pris en charge.".into());
        }
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
fn normalized(path: &Path) -> PathBuf {
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
        .map_err(|_| "Dossier de profil introuvable ou inaccessible.")?;
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
    Ok(Profile {
        id,
        path: raw.to_string(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .take(64)
            .collect(),
        visible: true,
    })
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
    paths
        .into_iter()
        .filter(|raw| seen.insert(normalized(raw)))
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
                visible: saved.map(|s| s.visible).unwrap_or(is_primary),
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
}
