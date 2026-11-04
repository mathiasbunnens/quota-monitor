//! Account entries of the native menu bar menu, below the quota panel:
//! per provider « Codex ↗ » (detailed usage), one entry per account with its
//! connection state and a native submenu chevron (detail), and
//! « + Ajouter un compte ». Pure model here, AppKit items in `append_items`.
//!
//! These are ordinary NSMenuItems (never `setView` on an open menu, see
//! `refresh_open_panel`): AppKit draws the chevron of an item with a submenu.
use crate::settings_focus::SettingsFocus;
use crate::{claude, codex, prefs, Tone};

#[derive(Clone, Debug, PartialEq)]
pub struct MenuAccount {
    pub id: String,
    pub name: String,
    pub state: String,
    pub tone: Tone,
    pub path: String,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuProvider {
    /// "codex" | "claude"
    pub kind: &'static str,
    pub title: &'static str,
    pub accounts: Vec<MenuAccount>,
}

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

/// Enabled providers with every listed account (hidden ones included: this is
/// where an account is reached, not a quota view). Codex first, like the panel.
pub fn menu_providers(
    prefs: &prefs::Prefs,
    codex_accounts: &[codex::CodexAccount],
    claude_accounts: &[claude::AccountSnapshot],
) -> Vec<MenuProvider> {
    let mut providers = Vec::new();
    if prefs.codex_enabled {
        providers.push(MenuProvider {
            kind: "codex",
            title: "Codex",
            accounts: codex_accounts
                .iter()
                .filter(|a| a.tracked)
                .map(|a| {
                    let (state, tone) = codex_state(a);
                    MenuAccount {
                        id: a.profile.id.clone(),
                        name: a.profile.name.clone(),
                        state,
                        tone,
                        path: a.profile.path.clone(),
                        primary: a.primary,
                    }
                })
                .collect(),
        });
    }
    if prefs.claude_enabled {
        providers.push(MenuProvider {
            kind: "claude",
            title: "Claude",
            accounts: claude_accounts
                .iter()
                .map(|a| {
                    let (state, tone) = claude_state(a);
                    MenuAccount {
                        id: a.profile.id.clone(),
                        name: a.profile.name.clone(),
                        state,
                        tone,
                        path: a.profile.path.clone(),
                        primary: a.primary,
                    }
                })
                .collect(),
        });
    }
    providers
}

/// Menu title of one account: name and connection state on the row, the
/// native chevron on its right opens the detail.
pub fn account_title(account: &MenuAccount) -> String {
    format!("{} · {}", account.name, account.state)
}

pub const USAGE_TITLE_ARROW: &str = "↗";
pub const ADD_TITLE: &str = "+ Ajouter un compte";

const SEPARATOR: char = '\u{1f}';

/// What an action item carries (`representedObject`): provider, optional account, add flag.
pub fn encode_route(kind: &str, account: Option<&str>, add: bool) -> String {
    format!(
        "{kind}{SEPARATOR}{}{SEPARATOR}{}",
        account.unwrap_or(""),
        if add { "add" } else { "" }
    )
}

pub fn decode_route(token: &str) -> Option<SettingsFocus> {
    let mut parts = token.split(SEPARATOR);
    let (kind, account, add) = (parts.next()?, parts.next()?, parts.next()?);
    SettingsFocus::new(kind, Some(account), add == "add")
}

/// Appends, for each provider, its header (↗ usage), its accounts (submenu =
/// detail) and « + Ajouter un compte », each block closed by a separator.
/// `target` implements `openProviderUsage:` and `openAccountSettings:`.
#[cfg(target_os = "macos")]
pub fn append_items(
    mtm: objc2_foundation::MainThreadMarker,
    menu: &objc2_app_kit::NSMenu,
    target: &objc2::runtime::AnyObject,
    providers: &[MenuProvider],
) {
    use objc2_app_kit::{NSMenu, NSMenuItem};
    use objc2_foundation::NSString;
    let action = |title: &str, sel: objc2::runtime::Sel, token: &str| {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&NSString::from_str(title));
        item.setEnabled(true);
        unsafe {
            item.setTarget(Some(target));
            item.setAction(Some(sel));
            item.setRepresentedObject(Some(&NSString::from_str(token)));
        }
        item
    };
    for provider in providers {
        menu.addItem(&action(
            &format!("{} {USAGE_TITLE_ARROW}", provider.title),
            objc2::sel!(openProviderUsage:),
            provider.kind,
        ));
        for account in &provider.accounts {
            // An item with a submenu gets AppKit's own chevron on its right.
            let row = NSMenuItem::new(mtm);
            row.setTitle(&NSString::from_str(&account_title(account)));
            row.setEnabled(true);
            let detail = NSMenu::new(mtm);
            detail.setAutoenablesItems(false);
            let path = NSMenuItem::new(mtm);
            path.setTitle(&NSString::from_str(&account.path));
            path.setEnabled(false);
            detail.addItem(&path);
            detail.addItem(&NSMenuItem::separatorItem(mtm));
            detail.addItem(&action(
                "Réglages…",
                objc2::sel!(openAccountSettings:),
                &encode_route(provider.kind, Some(&account.id), false),
            ));
            row.setSubmenu(Some(&detail));
            menu.addItem(&row);
        }
        menu.addItem(&action(
            ADD_TITLE,
            objc2::sel!(openAccountSettings:),
            &encode_route(provider.kind, None, true),
        ));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
    }
}

/// Reads the route carried by the clicked item.
#[cfg(target_os = "macos")]
pub fn represented_text(sender: Option<&objc2::runtime::AnyObject>) -> Option<String> {
    use objc2_app_kit::NSMenuItem;
    use objc2_foundation::NSString;
    let item: &NSMenuItem =
        unsafe { &*(sender? as *const objc2::runtime::AnyObject as *const NSMenuItem) };
    let object = item.representedObject()?;
    let text = object.downcast_ref::<NSString>()?;
    Some(text.to_string())
}

/// Detailed usage page of a provider (same pages as the « ↗ » of the app).
pub fn usage_command(kind: &str) -> Option<&'static str> {
    match kind {
        "codex" => Some("open_usage_details"),
        "claude" => Some("open_claude_usage"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(
        kind: &'static str,
        id: &str,
        name: &str,
        state: &str,
        tone: Tone,
        primary: bool,
    ) -> MenuAccount {
        MenuAccount {
            id: id.into(),
            name: name.into(),
            state: state.into(),
            tone,
            path: format!("/fixture/{kind}/{name}"),
            primary,
        }
    }

    #[test]
    fn routes_round_trip_and_reject_unknown_providers() {
        let token = encode_route("codex", Some("profile:travail"), false);
        assert_eq!(
            decode_route(&token),
            SettingsFocus::new("codex", Some("profile:travail"), false)
        );
        let add = decode_route(&encode_route("claude", None, true)).unwrap();
        assert_eq!(
            (add.provider.as_str(), add.account_id, add.add),
            ("claude", None, true)
        );
        assert_eq!(decode_route("gemini\u{1f}\u{1f}add"), None);
        assert_eq!(decode_route("garbage"), None);
        assert_eq!(usage_command("codex"), Some("open_usage_details"));
        assert_eq!(usage_command("claude"), Some("open_claude_usage"));
        assert_eq!(usage_command("other"), None);
    }

    #[test]
    fn model_has_the_named_account_shape() {
        let a = named(
            "codex",
            "primary",
            "Principal",
            "Connecté",
            Tone::Good,
            true,
        );
        assert_eq!(
            (a.name.as_str(), a.state.as_str(), a.primary),
            ("Principal", "Connecté", true)
        );
    }
}
