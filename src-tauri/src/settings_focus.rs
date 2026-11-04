//! Where the Settings window should land when it is opened from another
//! surface (native menu, compact window): the Fournisseurs tab on the right
//! provider, optionally on one account's detail or on its add flow.
//! Display only: nothing here collects, logs in or changes preferences.
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsFocus {
    /// "codex" or "claude".
    pub provider: String,
    /// Account whose detail is unfolded (its stable id).
    pub account_id: Option<String>,
    /// Open the add-account flow of the provider.
    pub add: bool,
}

pub fn valid_provider(provider: &str) -> bool {
    matches!(provider, "codex" | "claude")
}

impl SettingsFocus {
    pub fn new(provider: &str, account_id: Option<&str>, add: bool) -> Option<Self> {
        valid_provider(provider).then(|| Self {
            provider: provider.into(),
            account_id: account_id.filter(|id| !id.is_empty()).map(str::to_string),
            add,
        })
    }
}

/// Pending focus, taken once by the Settings window (a window that is still
/// loading has not subscribed to the event yet).
#[derive(Default)]
pub struct SettingsFocusState(Mutex<Option<SettingsFocus>>);

impl SettingsFocusState {
    pub fn set(&self, focus: SettingsFocus) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(focus);
        }
    }
    pub fn take(&self) -> Option<SettingsFocus> {
        self.0.lock().ok().and_then(|mut slot| slot.take())
    }
}

/// Stores the focus, shows the Settings window and tells an already open one.
pub fn open(app: &tauri::AppHandle, focus: SettingsFocus) -> Result<(), String> {
    app.state::<SettingsFocusState>().set(focus);
    crate::show_settings_window(app).map_err(|e| e.to_string())?;
    let _ = app.emit_to("settings", "settings-focus", ());
    Ok(())
}

#[tauri::command]
pub fn take_settings_focus(state: tauri::State<SettingsFocusState>) -> Option<SettingsFocus> {
    state.take()
}

#[tauri::command]
pub async fn open_settings_focus(
    app: tauri::AppHandle,
    provider: String,
    account_id: Option<String>,
    add: Option<bool>,
) -> Result<(), String> {
    let focus = SettingsFocus::new(&provider, account_id.as_deref(), add.unwrap_or(false))
        .ok_or("Fournisseur inconnu.")?;
    open(&app, focus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_names_a_known_provider_and_keeps_the_account_it_was_given() {
        let focus = SettingsFocus::new("claude", Some("claude:travail"), false).unwrap();
        assert_eq!(
            (
                focus.provider.as_str(),
                focus.account_id.as_deref(),
                focus.add
            ),
            ("claude", Some("claude:travail"), false)
        );
        assert_eq!(
            SettingsFocus::new("codex", Some(""), true)
                .unwrap()
                .account_id,
            None
        );
        assert!(SettingsFocus::new("gemini", None, false).is_none());
        let json = serde_json::to_value(&focus).unwrap();
        assert_eq!(json["accountId"], "claude:travail");
        assert_eq!(json["add"], false);
    }

    #[test]
    fn a_pending_focus_is_delivered_once_and_the_last_request_wins() {
        let state = SettingsFocusState::default();
        assert_eq!(state.take(), None);
        state.set(SettingsFocus::new("codex", None, true).unwrap());
        state.set(SettingsFocus::new("claude", Some("primary"), false).unwrap());
        assert_eq!(state.take().map(|f| f.provider), Some("claude".into()));
        assert_eq!(state.take(), None);
    }
}
