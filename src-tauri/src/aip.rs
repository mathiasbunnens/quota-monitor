//! Read-only metadata of `aip` profiles (crate `aip-cli`, github.com/dqn/aip,
//! v0.11.0 = revision 3961fa05f7e040edb2ab5cf5866fd57b0b6bab78).
//!
//! Format confirmed in `src/tool.rs` of that revision: the manager always uses
//! `~/.claude/profiles/<name>` and `~/.codex/profiles/<name>` (it ignores
//! `CLAUDE_CONFIG_DIR` / `CODEX_HOME`); `<name>` is ASCII alphanumeric, `-` or
//! `_`; `profiles/_current` names the selected profile and `profiles/_order`
//! the dashboard order. Each profile holds one payload file: Claude
//! `credentials.json` (a snapshot of the keychain, not a runnable home), Codex
//! `auth.json` (a `CODEX_HOME`-shaped home; the launcher refuses an imported
//! OAuth grant until `aip codex login` replaced it).
//!
//! This module only lists directory names and the presence of files. It never
//! opens `credentials.json`, `auth.json` or `aip-imported-refresh.sha256`,
//! never writes anything and never calls the `aip` executable. Loading a
//! profile's credential in memory for one usage read is the collector's job
//! (`oauth_usage`), which never writes it back either.
use std::path::{Path, PathBuf};

pub const PROFILES_DIR: &str = "profiles";
pub const CURRENT_MARKER: &str = "_current";
pub const ORDER_FILE: &str = "_order";
/// Claude payload (keychain snapshot). Presence only, never read.
pub const CLAUDE_PAYLOAD: &str = "credentials.json";
/// Codex payload (home credentials). Presence only, never read.
pub const CODEX_PAYLOAD: &str = "auth.json";
/// Largest `_current` / `_order` file still treated as metadata.
const MAX_MARKER_BYTES: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Directory name as listed by aip (its profile name).
    pub name: String,
    /// `<base>/profiles/<name>` exactly as joined, never canonicalised.
    pub raw: PathBuf,
    /// Named by `_current` (Claude: the keychain holds this account; Codex:
    /// selected for the next `aip codex` launch only).
    pub active: bool,
    /// The documented payload file exists (name checked, never opened).
    pub complete: bool,
}

/// aip's own rule (`Tool::profile_dir`): a name it can switch to or launch.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && name != CURRENT_MARKER
        && name != ORDER_FILE
}

/// Small text marker; `None` when absent, oversized, binary or unreadable.
fn marker(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_MARKER_BYTES {
        return None;
    }
    String::from_utf8(std::fs::read(path).ok()?).ok()
}

/// Profiles under `<base>/profiles` in aip's dashboard order (`_order` lines
/// first, the rest alphabetically). `base` is the manager's fixed base
/// (`~/.claude` or `~/.codex`). No manager, no `profiles` folder, a file in
/// its place or unreadable entries all yield an empty list. Entries that are
/// not directories or carry a name aip itself rejects are skipped.
pub fn profiles(base: &Path, payload: &str) -> Vec<Profile> {
    let dir = base.join(PROFILES_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            (valid_name(&name) && e.path().is_dir()).then(|| (name, e.path()))
        })
        .collect();
    found.sort();
    let order = marker(&dir.join(ORDER_FILE)).unwrap_or_default();
    let mut ordered: Vec<(String, PathBuf)> = vec![];
    for line in order.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(i) = found.iter().position(|(n, _)| n == line) {
            ordered.push(found.remove(i));
        }
    }
    ordered.extend(found);
    let current = marker(&dir.join(CURRENT_MARKER))
        .map(|s| s.trim().to_string())
        .filter(|s| valid_name(s));
    ordered
        .into_iter()
        .map(|(name, raw)| Profile {
            active: current.as_deref() == Some(name.as_str()),
            complete: raw.join(payload).is_file(),
            name,
            raw,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Synthetic fixture only: fake names, sentinel payloads, no real account.
    fn base(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("qm-aip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn profile(base: &Path, name: &str, payload: Option<&str>) {
        let dir = base.join(PROFILES_DIR).join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(file) = payload {
            std::fs::write(dir.join(file), "SYNTHETIC-SENTINEL-NOT-A-CREDENTIAL").unwrap();
        }
    }

    #[test]
    fn absent_manager_empty_folder_or_file_in_its_place_yield_nothing() {
        let b = base("absent");
        assert!(profiles(&b, CLAUDE_PAYLOAD).is_empty(), "no profiles dir");
        std::fs::create_dir_all(b.join(PROFILES_DIR)).unwrap();
        assert!(profiles(&b, CLAUDE_PAYLOAD).is_empty(), "empty dir");
        std::fs::write(b.join(PROFILES_DIR).join(CURRENT_MARKER), "ghost\n").unwrap();
        assert!(profiles(&b, CLAUDE_PAYLOAD).is_empty(), "marker alone");
        let f = base("file");
        std::fs::write(f.join(PROFILES_DIR), "not a directory").unwrap();
        assert!(profiles(&f, CODEX_PAYLOAD).is_empty());
        std::fs::remove_dir_all(b).unwrap();
        std::fs::remove_dir_all(f).unwrap();
    }

    #[test]
    fn lists_directories_in_aip_order_with_current_and_payload_presence() {
        let b = base("order");
        profile(&b, "perso", Some(CODEX_PAYLOAD));
        profile(&b, "travail", Some(CODEX_PAYLOAD));
        profile(&b, "vide", None);
        std::fs::write(b.join(PROFILES_DIR).join("stray-file"), "x").unwrap();
        std::fs::write(
            b.join(PROFILES_DIR).join(ORDER_FILE),
            "\n travail \nsupprime\n\n",
        )
        .unwrap();
        std::fs::write(b.join(PROFILES_DIR).join(CURRENT_MARKER), "perso\n").unwrap();
        let found = profiles(&b, CODEX_PAYLOAD);
        let names: Vec<_> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["travail", "perso", "vide"],
            "_order then alphabetical"
        );
        assert!(found[1].active && !found[0].active && !found[2].active);
        assert!(found[0].complete && found[1].complete && !found[2].complete);
        assert_eq!(found[1].raw, b.join(PROFILES_DIR).join("perso"));
        // The payload itself is never opened: the sentinel is untouched.
        assert_eq!(
            std::fs::read_to_string(found[1].raw.join(CODEX_PAYLOAD)).unwrap(),
            "SYNTHETIC-SENTINEL-NOT-A-CREDENTIAL"
        );
        std::fs::remove_dir_all(b).unwrap();
    }

    #[test]
    fn corrupted_markers_and_invalid_names_are_ignored_without_failing() {
        let b = base("corrupt");
        profile(&b, "ok_1", Some(CLAUDE_PAYLOAD));
        profile(&b, "bad name", Some(CLAUDE_PAYLOAD));
        profile(&b, "évil", Some(CLAUDE_PAYLOAD));
        let dir = b.join(PROFILES_DIR);
        // Reserved names are not profiles even when they are directories.
        std::fs::create_dir_all(dir.join(ORDER_FILE)).unwrap();
        std::fs::write(dir.join(CURRENT_MARKER), [0xff, 0xfe, 0x00]).unwrap();
        let found = profiles(&b, CLAUDE_PAYLOAD);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "ok_1");
        assert!(!found[0].active, "binary marker selects nothing");
        std::fs::write(dir.join(CURRENT_MARKER), "missing-profile").unwrap();
        assert!(!profiles(&b, CLAUDE_PAYLOAD)[0].active);
        std::fs::write(dir.join(CURRENT_MARKER), "../ok_1").unwrap();
        assert!(
            !profiles(&b, CLAUDE_PAYLOAD)[0].active,
            "traversal rejected"
        );
        std::fs::write(dir.join(CURRENT_MARKER), vec![b'a'; 5000]).unwrap();
        assert!(!profiles(&b, CLAUDE_PAYLOAD)[0].active, "oversized marker");
        std::fs::write(dir.join(CURRENT_MARKER), "ok_1").unwrap();
        assert!(profiles(&b, CLAUDE_PAYLOAD)[0].active);
        std::fs::remove_dir_all(b).unwrap();
    }

    #[test]
    fn valid_name_matches_the_manager_rule() {
        for ok in ["personal", "work-account", "test_123", "A"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "_current", "_order", "../evil", "foo/bar", "a b", "é"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }
}
