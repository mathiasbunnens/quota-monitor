//! Display mode: menu-bar app (default) or standalone Dock application.
//!
//! Pure decision logic, kept free of any windowing call so it can be unit
//! tested. The macOS wiring in `lib.rs` only executes the `Layout` returned here.

pub const MENUBAR: &str = "menubar";
pub const APP: &str = "app";

/// Whether the standalone mode exists on this platform (macOS only: Windows and
/// Linux always keep their tray icon, so the choice is hidden there).
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// Unknown or missing values fall back to the historical menu-bar behaviour.
pub fn normalize(mode: &str) -> &'static str {
    if mode == APP {
        APP
    } else {
        MENUBAR
    }
}

/// What must be present on screen for a given mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Regular activation policy: Dock icon and Cmd+Tab entry.
    pub dock: bool,
    /// Menu-bar status item.
    pub status_item: bool,
    /// Main dashboard window is the primary entry point.
    pub window: bool,
}

pub fn layout(mode: &str, supported: bool) -> Layout {
    if supported && normalize(mode) == APP {
        // Exclusive choice: the status item is removed; Dock + window remain.
        Layout {
            dock: true,
            status_item: false,
            window: true,
        }
    } else {
        Layout {
            dock: false,
            status_item: true,
            window: false,
        }
    }
}

/// Quitting (Cmd+Q, Dock menu) is honoured only in standalone mode. In
/// menu-bar mode, closing the last window must keep the app alive.
pub fn allow_system_exit(mode: &str, supported: bool) -> bool {
    layout(mode, supported).dock
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_values_default_to_menu_bar() {
        assert_eq!(normalize(""), MENUBAR);
        assert_eq!(normalize("dock"), MENUBAR);
        assert_eq!(normalize(APP), APP);
    }

    #[test]
    fn menu_bar_keeps_tray_without_dock() {
        let l = layout(MENUBAR, true);
        assert_eq!(
            l,
            Layout {
                dock: false,
                status_item: true,
                window: false
            }
        );
        assert!(!allow_system_exit(MENUBAR, true));
    }

    #[test]
    fn standalone_is_exclusive_with_dock_and_window() {
        let l = layout(APP, true);
        assert!(l.dock && l.window && !l.status_item);
        assert!(allow_system_exit(APP, true));
    }

    #[test]
    fn unsupported_platforms_always_keep_the_tray() {
        assert_eq!(layout(APP, false), layout(MENUBAR, false));
        assert!(!allow_system_exit(APP, false));
    }

    #[test]
    fn every_mode_keeps_a_way_to_reach_the_app() {
        for mode in ["", MENUBAR, APP, "garbage"] {
            for supported in [true, false] {
                let l = layout(mode, supported);
                assert!(l.status_item || (l.dock && l.window), "{mode} {supported}");
            }
        }
    }
}
