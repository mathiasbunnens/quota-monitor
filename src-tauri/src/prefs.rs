//! Persistent, provider-independent application preferences (`prefs.json`).
//! Kept separate from `refresh.json` so existing Codex settings keep working.
use crate::{claude, codex};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Mutex};
use tauri::Manager;

pub const INDICATORS: [&str; 3] = ["codex", "claude", "both"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub codex_enabled: bool,
    pub claude_enabled: bool,
    pub fable_enabled: bool,
    /// Menu-bar indicator: "codex", "claude" or "both".
    pub indicator: String,
    pub ntfy: crate::ntfy::Config,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            codex_enabled: true,
            claude_enabled: false,
            fable_enabled: true,
            indicator: "codex".into(),
            ntfy: crate::ntfy::Config::default(),
        }
    }
}

impl Prefs {
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
pub fn set_preferences(app: tauri::AppHandle, prefs: Prefs) -> Result<Prefs, String> {
    if !prefs.valid() {
        return Err("Indicateur de barre des menus non pris en charge.".into());
    }
    let file = path(&app)?;
    prefs.ntfy.validate()?;
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec(&prefs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, &file).map_err(|e| e.to_string())?;

    let previous = {
        let state = app.state::<PrefsState>();
        let mut guard = state.0.lock().map_err(|_| "Préférences indisponibles.")?;
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
    if previous.claude_enabled != prefs.claude_enabled {
        claude::on_enabled_changed();
    }
    crate::publish_quotas(&app);
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_codex_only_behaviour() {
        let p = Prefs::default();
        assert!(p.codex_enabled && !p.claude_enabled);
        assert!(p.fable_enabled);
        assert_eq!(p.indicator, "codex");
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
            ntfy: crate::ntfy::Config::default(),
        };
        let bytes = serde_json::to_vec(&saved).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["fableEnabled"],
            false
        );
        assert_eq!(parse(&bytes), saved);
    }
}
