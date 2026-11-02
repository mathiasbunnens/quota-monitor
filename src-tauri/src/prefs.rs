//! Persistent, provider-independent application preferences (`prefs.json`).
//! Kept separate from `refresh.json` so existing Codex settings keep working.
use crate::{claude, codex};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Mutex};
use tauri::Manager;

pub const INDICATORS: [&str; 3] = ["codex", "claude", "both"];

/// Presentation of several visible Claude accounts (Réglages → Affichage).
/// Independent of the account mode/visibility; one visible account always
/// keeps the historical card. Absent/unknown = "compact" (paired bars).
pub const CLAUDE_LAYOUT_COMPACT: &str = "compact";
pub const CLAUDE_LAYOUT_CLASSIC: &str = "classic";

pub fn normalize_claude_layout(value: &str) -> &'static str {
    if value == CLAUDE_LAYOUT_CLASSIC {
        CLAUDE_LAYOUT_CLASSIC
    } else {
        CLAUDE_LAYOUT_COMPACT
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub codex_enabled: bool,
    pub claude_enabled: bool,
    pub fable_enabled: bool,
    /// Menu-bar indicator: "codex", "claude" or "both".
    pub indicator: String,
    /// "menubar" (default) or "app" (standalone Dock application, macOS only).
    pub display_mode: String,
    pub ntfy: crate::ntfy::Config,
    pub claude_accounts: crate::claude_accounts::AccountPrefs,
    /// "compact" (default: paired 5 h/7 j bars) or "classic" (one section per
    /// account with the historical separated rows). Display only.
    pub claude_layout: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            codex_enabled: true,
            claude_enabled: false,
            fable_enabled: true,
            indicator: "codex".into(),
            display_mode: crate::display_mode::MENUBAR.into(),
            ntfy: crate::ntfy::Config::default(),
            claude_accounts: crate::claude_accounts::AccountPrefs::default(),
            claude_layout: CLAUDE_LAYOUT_COMPACT.into(),
        }
    }
}

impl Prefs {
    pub fn classic_claude_layout(&self) -> bool {
        self.claude_layout == CLAUDE_LAYOUT_CLASSIC
    }

    pub fn valid(&self) -> bool {
        INDICATORS.contains(&self.indicator.as_str())
    }
}

#[derive(Default)]
pub struct PrefsState(pub Mutex<Prefs>);

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("prefs.json"))
}

pub fn parse(bytes: &[u8]) -> Prefs {
    serde_json::from_slice::<Prefs>(bytes)
        .ok()
        .filter(Prefs::valid)
        .map(|mut prefs| {
            // Unknown/absent mode (older file) keeps the menu-bar behaviour
            // without discarding the rest of the preferences.
            prefs.display_mode = crate::display_mode::normalize(&prefs.display_mode).into();
            prefs.claude_layout = normalize_claude_layout(&prefs.claude_layout).into();
            prefs
        })
        .unwrap_or_default()
}

pub fn load(app: &tauri::AppHandle) -> Prefs {
    path(app)
        .ok()
        .and_then(|p| fs::read(p).ok())
        .map(|bytes| parse(&bytes))
        .unwrap_or_default()
}

pub fn current(app: &tauri::AppHandle) -> Prefs {
    app.try_state::<PrefsState>()
        .and_then(|state| state.0.lock().ok().map(|p| p.clone()))
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_preferences(app: tauri::AppHandle) -> Prefs {
    current(&app)
}

#[tauri::command]
pub fn set_preferences(app: tauri::AppHandle, mut prefs: Prefs) -> Result<Prefs, String> {
    prefs.display_mode = if crate::display_mode::SUPPORTED {
        crate::display_mode::normalize(&prefs.display_mode).into()
    } else {
        crate::display_mode::MENUBAR.into()
    };
    // Presentation only: never a collection, session or cache change.
    prefs.claude_layout = normalize_claude_layout(&prefs.claude_layout).into();
    // Validation (indicator, ntfy, Claude profiles) happens in save_to before
    // the file or the in-memory state is replaced.
    let file = path(&app)?;
    let previous = {
        let state = app.state::<PrefsState>();
        let mut guard = state.0.lock().map_err(|_| "Préférences indisponibles.")?;
        save_to(&file, &prefs)?;
        std::mem::replace(&mut *guard, prefs.clone())
    };
    if previous.codex_enabled != prefs.codex_enabled {
        if !prefs.codex_enabled {
            crate::set_codex_snapshots(&app, None);
        }
        if let Some(provider) = app.try_state::<codex::Provider>() {
            provider.set_enabled(prefs.codex_enabled);
        }
    }
    if collection_changed(&previous, &prefs) {
        claude::on_enabled_changed();
    }
    if previous.display_mode != prefs.display_mode {
        crate::apply_display_mode(&app);
    }
    crate::publish_quotas(&app);
    Ok(prefs)
}

fn collection_changed(previous: &Prefs, next: &Prefs) -> bool {
    let sources = |p: &Prefs| {
        p.claude_accounts
            .profiles
            .iter()
            .map(|a| (a.id.clone(), a.path.clone()))
            .collect::<std::collections::BTreeSet<_>>()
    };
    previous.claude_enabled != next.claude_enabled || sources(previous) != sources(next)
}

fn save_to(file: &std::path::Path, prefs: &Prefs) -> Result<(), String> {
    if !prefs.valid() {
        return Err("Indicateur de barre des menus non pris en charge.".into());
    }
    prefs.ntfy.validate()?;
    prefs.claude_accounts.validate()?;
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec(&prefs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, &file).map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v310_preferences_file_loads_without_loss_and_defaults_to_single_primary() {
        // Shape written by v3.1.0 (display mode, Fable opt-out, ntfy), no
        // claudeAccounts key yet. Refresh settings live in the Codex file.
        let bytes = br#"{"codexEnabled":true,"claudeEnabled":true,"fableEnabled":false,"indicator":"both","displayMode":"app","ntfy":{"enabled":true,"serverUrl":"https://ntfy.example.invalid","topic":"fixture-topic_1"}}"#;
        let p = parse(bytes);
        assert!(p.codex_enabled && p.claude_enabled && !p.fable_enabled);
        assert_eq!(p.indicator, "both");
        assert_eq!(p.display_mode, "app");
        assert!(p.ntfy.enabled);
        assert_eq!(p.ntfy.server_url, "https://ntfy.example.invalid");
        assert_eq!(p.ntfy.topic, "fixture-topic_1");
        assert_eq!(p.claude_accounts.mode, "single");
        assert_eq!(p.claude_accounts.selected_id, None);
        assert!(p.claude_accounts.profiles.is_empty());
        // Round trip keeps every v3.1.0 key.
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        for key in [
            "codexEnabled",
            "claudeEnabled",
            "fableEnabled",
            "indicator",
            "displayMode",
            "ntfy",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert_eq!(parse(&serde_json::to_vec(&p).unwrap()), p);
    }
    #[test]
    fn visibility_selection_mode_and_names_do_not_restart_collection() {
        let before = Prefs::default();
        let mut after = before.clone();
        after.claude_accounts.mode = "multi".into();
        after.claude_accounts.selected_id = Some("display-only".into());
        assert!(!collection_changed(&before, &after));
        after.claude_enabled = true;
        assert!(collection_changed(&before, &after));
        let before = after.clone();
        after
            .claude_accounts
            .profiles
            .push(crate::claude_accounts::Profile {
                id: "profile:a".into(),
                path: "/fixture/a".into(),
                name: "A".into(),
                visible: true,
            });
        assert!(collection_changed(&before, &after));
        let before = after.clone();
        after.claude_accounts.profiles[0].name = "Personnel".into();
        after.claude_accounts.profiles[0].visible = false;
        assert!(!collection_changed(&before, &after));
    }
    #[test]
    fn account_preferences_persist_on_disk_and_invalid_edits_do_not_replace_them() {
        let root = std::env::temp_dir().join(format!("qm-prefs-profile-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let profile_dir = root.join("profile");
        fs::create_dir_all(&profile_dir).unwrap();
        let profile_dir = profile_dir.canonicalize().unwrap();
        let file = root.join("prefs.json");
        let mut p = parse(br#"{"claudeEnabled":true,"indicator":"both"}"#);
        p.claude_accounts.mode = "multi".into();
        p.claude_accounts
            .profiles
            .push(crate::claude_accounts::Profile {
                id: crate::claude_accounts::identity(&profile_dir),
                path: profile_dir.to_str().unwrap().into(),
                name: "Personnel".into(),
                visible: false,
            });
        p.claude_accounts.selected_id = Some(p.claude_accounts.profiles[0].id.clone());
        save_to(&file, &p).unwrap();
        assert_eq!(parse(&fs::read(&file).unwrap()), p);
        let bytes = fs::read(&file).unwrap();
        p.claude_accounts.profiles[0].name = String::new();
        assert!(save_to(&file, &p).is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn claude_layout_migrates_normalises_and_persists_independently() {
        // Absent (v3.1.0 / previous multi build): compact, everything else kept.
        let old = parse(br#"{"claudeEnabled":true,"indicator":"both","displayMode":"app","claudeAccounts":{"mode":"multi","selectedId":null,"profiles":[]}}"#);
        assert_eq!(old.claude_layout, "compact");
        assert!(!old.classic_claude_layout());
        assert!(old.claude_enabled && old.indicator == "both" && old.display_mode == "app");
        assert_eq!(old.claude_accounts.mode, "multi");
        // Unknown value is normalised without resetting other preferences.
        let odd = parse(br#"{"claudeLayout":"multi","claudeEnabled":true,"fableEnabled":false,"claudeAccounts":{"mode":"multi","selectedId":null,"profiles":[]}}"#);
        assert_eq!(odd.claude_layout, "compact");
        assert!(odd.claude_enabled && !odd.fable_enabled);
        assert_eq!(odd.claude_accounts.mode, "multi");
        // Classic survives a round trip and is written as its own key.
        let classic = parse(br#"{"claudeLayout":"classic","claudeAccounts":{"mode":"single","selectedId":null,"profiles":[]}}"#);
        assert!(classic.classic_claude_layout());
        assert_eq!(classic.claude_accounts.mode, "single");
        let v = serde_json::to_value(&classic).unwrap();
        assert_eq!(v["claudeLayout"], "classic");
        assert_eq!(parse(&serde_json::to_vec(&classic).unwrap()), classic);
    }
    #[test]
    fn claude_layout_change_does_not_restart_collection_or_touch_accounts() {
        let mut before = Prefs::default();
        before.claude_enabled = true;
        before.claude_accounts.mode = "multi".into();
        before
            .claude_accounts
            .profiles
            .push(crate::claude_accounts::Profile {
                id: "profile:a".into(),
                path: "/fixture/a".into(),
                name: "A".into(),
                visible: false,
            });
        let mut after = before.clone();
        after.claude_layout = "classic".into();
        assert!(!collection_changed(&before, &after));
        assert_eq!(before.claude_accounts, after.claude_accounts);
    }
    #[test]
    fn layout_preference_is_saved_to_disk_with_other_preferences() {
        let root = std::env::temp_dir().join(format!("qm-prefs-layout-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("prefs.json");
        let mut p = parse(br#"{"claudeEnabled":true,"indicator":"both"}"#);
        p.claude_layout = "classic".into();
        save_to(&file, &p).unwrap();
        let back = parse(&fs::read(&file).unwrap());
        assert_eq!(back, p);
        assert!(back.classic_claude_layout() && back.indicator == "both");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn defaults_keep_codex_only_behaviour() {
        let p = Prefs::default();
        assert!(p.codex_enabled && !p.claude_enabled);
        assert!(p.fable_enabled);
        assert_eq!(p.indicator, "codex");
        assert_eq!(p.display_mode, "menubar");
    }

    #[test]
    fn display_mode_migrates_and_persists() {
        // Older file without the key: menu bar, other prefs kept.
        let old = parse(br#"{"claudeEnabled":true,"indicator":"both"}"#);
        assert_eq!(old.display_mode, "menubar");
        assert!(old.claude_enabled && old.indicator == "both");
        // Unknown value is normalised, not a reset of everything else.
        let odd = parse(br#"{"displayMode":"dock","claudeEnabled":true}"#);
        assert_eq!(odd.display_mode, "menubar");
        assert!(odd.claude_enabled);
        assert_eq!(parse(br#"{"displayMode":"app"}"#).display_mode, "app");
    }

    #[test]
    fn missing_or_invalid_files_fall_back_to_defaults() {
        assert_eq!(parse(b"not json"), Prefs::default());
        assert_eq!(parse(br#"{"indicator":"mistral"}"#), Prefs::default());
        // Partial file from an older version keeps defaults for absent keys.
        let p = parse(br#"{"claudeEnabled":true}"#);
        assert!(p.claude_enabled && p.codex_enabled && p.indicator == "codex");
        assert!(p.fable_enabled);
    }

    #[test]
    fn preferences_round_trip_through_json() {
        let saved = Prefs {
            codex_enabled: false,
            claude_enabled: true,
            fable_enabled: false,
            indicator: "both".into(),
            display_mode: "app".into(),
            ntfy: crate::ntfy::Config::default(),
            claude_accounts: crate::claude_accounts::AccountPrefs::default(),
            claude_layout: "classic".into(),
        };
        let bytes = serde_json::to_vec(&saved).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["fableEnabled"],
            false
        );
        assert_eq!(parse(&bytes), saved);
    }
}
