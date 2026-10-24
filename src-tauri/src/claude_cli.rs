//! Structured Claude Code control protocol. No user message/model request is sent.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

/// Find Claude Code even when a GUI-launched process lacks the shell PATH.
fn candidates(
    home: Option<&Path>,
    search_path: Option<&std::ffi::OsStr>,
    override_path: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    if let Some(path) = override_path {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            paths.push(path);
        }
    }
    if let Some(home) = home {
        paths.push(home.join(".local/bin").join(name));
        #[cfg(not(windows))]
        {
            for directory in [
                ".volta/bin",
                ".asdf/shims",
                ".local/share/mise/shims",
                ".bun/bin",
                ".npm-global/bin",
            ] {
                paths.push(home.join(directory).join(name));
            }
            // nvm/fnm directories are usually set only by interactive shells.
            for (root, bin) in [
                (".nvm/versions/node", "bin"),
                (".fnm/node-versions", "installation/bin"),
            ] {
                if let Ok(versions) = std::fs::read_dir(home.join(root)) {
                    let mut versions: Vec<_> = versions.flatten().map(|v| v.path()).collect();
                    versions.sort();
                    for version in versions.into_iter().rev() {
                        paths.push(version.join(bin).join(name));
                    }
                }
            }
        }
    }
    if let Some(search_path) = search_path {
        paths.extend(
            std::env::split_paths(search_path)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name)),
        );
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join("Programs/Claude/claude.exe"));
    }
    #[cfg(not(windows))]
    paths.extend(
        [
            "/opt/homebrew/bin/claude",
            "/usr/local/bin/claude",
            "/usr/bin/claude",
        ]
        .map(PathBuf::from),
    );
    paths
}

pub fn discover() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    }
    .map(PathBuf::from)
    .filter(|p| p.is_absolute());
    let path = std::env::var_os("PATH");
    let override_path = std::env::var_os("QUOTA_CLAUDE_BINARY");
    candidates(home.as_deref(), path.as_deref(), override_path.as_deref())
        .into_iter()
        .find(|p| p.is_file())
}

/// Preserve the Node runtime co-located with an npm/nvm launcher.
fn claude_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    if let Some(bin) = executable.parent() {
        let mut paths = vec![bin.to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
    }
    command
}
pub fn read_usage(executable: &Path, cwd: &Path) -> Result<Value, &'static str> {
    std::fs::create_dir_all(cwd).map_err(|_| "workspace")?;
    let mut child = claude_command(executable)
        .args([
            "--safe-mode",
            "--strict-mcp-config",
            "--tools",
            "",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "-p",
        ])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "launch")?;
    let stdout = child.stdout.take().ok_or("launch")?;
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        // Bound each line; never save/log the initialize response (account identity).
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            use std::io::Read;
            let count = reader.by_ref().take(1_048_577).read_until(b'\n', &mut line);
            match count {
                Ok(0) | Err(_) => break,
                _ => {}
            }
            if line.len() > 1_048_576 {
                let _ = tx.send(Err("malformed"));
                break;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                if value["type"] == "control_response" && tx.send(Ok(value)).is_err() {
                    break;
                }
            }
        }
    });
    let result = (|| {
        let input = child.stdin.as_mut().ok_or("launch")?;
        for (id, subtype) in [("init", "initialize"), ("usage", "get_usage")] {
            let request =
                json!({"type":"control_request", "request_id":id, "request":{"subtype":subtype}});
            writeln!(input, "{request}").map_err(|_| "transport")?;
            input.flush().map_err(|_| "transport")?;
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                let value = rx
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .map_err(|_| "timeout")??;
                let response = &value["response"];
                if response["request_id"] != id {
                    continue;
                }
                if response["subtype"] != "success" {
                    return Err("unsupported_or_auth");
                }
                if id == "usage" {
                    return Ok(response["response"].clone());
                }
                break;
            }
        }
        Err("malformed")
    })();
    drop(child.stdin.take());
    for _ in 0..20 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
    drop(rx);
    let _ = reader.join();
    result
}

/// Claude Code 2.1.299's get_usage schema projects weekly server limits into
/// model_scoped ({display_name, utilization, resets_at}); not seven_day_fable.
/// An absent/hidden/null bucket must not become a zero-percent counter.
pub fn fable_window(limits: &Value) -> Option<crate::claude::Window> {
    limits["model_scoped"].as_array()?.iter().find_map(|row| {
        row["display_name"]
            .as_str()
            .filter(|name| name.eq_ignore_ascii_case("Fable"))
            .and_then(|_| window(row))
    })
}

pub fn window(value: &Value) -> Option<crate::claude::Window> {
    let used = value["utilization"].as_f64()?;
    if !used.is_finite() || !(0.0..=100.0).contains(&used) {
        return None;
    }
    let reset = value["resets_at"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp());
    Some(crate::claude::Window {
        used_percent: used,
        resets_at: reset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn gui_without_shell_path_still_discovers_nvm_and_fnm() {
        let dir = std::env::temp_dir().join(format!("qm-claude-discover-{}", std::process::id()));
        let nvm = dir.join(".nvm/versions/node/v22.5.0/bin/claude");
        let fnm = dir.join(".fnm/node-versions/v23.1.0/installation/bin/claude");
        std::fs::create_dir_all(nvm.parent().unwrap()).unwrap();
        std::fs::create_dir_all(fnm.parent().unwrap()).unwrap();
        std::fs::write(&nvm, b"").unwrap();
        std::fs::write(&fnm, b"").unwrap();
        let found = candidates(Some(&dir), None, None);
        assert!(found.contains(&nvm));
        assert!(found.contains(&fnm));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_install_uses_exe_suffix() {
        let home = Path::new(r"C:\Users\test");
        let found = candidates(Some(home), None, None);
        assert!(found.contains(&home.join(".local/bin/claude.exe")));
        assert!(!found.contains(&home.join(".local/bin/claude")));
    }

    #[test]
    fn explicit_binary_has_priority() {
        let path = std::env::current_dir().unwrap().join("custom-claude");
        let found = candidates(None, None, Some(path.as_os_str()));
        assert_eq!(found.first(), Some(&path));
    }

    #[cfg(unix)]
    #[test]
    fn gui_launch_finds_sibling_interpreter() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("qm-claude-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let interpreter = dir.join("qm-fake-node");
        let claude = dir.join("claude");
        std::fs::write(&interpreter, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
        std::fs::write(&claude, "#!/usr/bin/env qm-fake-node\nprintf 'running'\n").unwrap();
        for executable in [&interpreter, &claude] {
            std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let result = claude_command(&claude).output().unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout.as_slice(), b"running");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn structured_windows_preserve_zero_absence_and_reset() {
        assert_eq!(
            window(&json!({"utilization":0,"resets_at":"2026-10-21T14:00:00Z"}))
                .unwrap()
                .used_percent,
            0.0
        );
        assert!(window(&Value::Null).is_none());
        assert!(window(&json!({"utilization":101})).is_none());
        assert!(
            window(&json!({"utilization":14,"resets_at":"2026-10-21T14:00:00Z"}))
                .unwrap()
                .resets_at
                .is_some()
        );
    }
    #[cfg(unix)]
    #[test]
    fn transport_accepts_structured_reply_and_reaps_child() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("quota-cli-fixture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("fake-cli");
        std::fs::write(&exe, "#!/bin/sh\nread first\nprintf '%s\\n' '{\"type\":\"control_response\",\"response\":{\"request_id\":\"init\",\"subtype\":\"success\",\"response\":{}}}'\nread second\nprintf '%s\\n' '{\"type\":\"control_response\",\"response\":{\"request_id\":\"usage\",\"subtype\":\"success\",\"response\":{\"rate_limits_available\":true,\"rate_limits\":{\"five_hour\":{\"utilization\":42}}}}}'\ncat >/dev/null\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        let value = read_usage(&exe, &dir).unwrap();
        assert_eq!(value["rate_limits"]["five_hour"]["utilization"], 42);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "Explicit opt-in real official CLI, no inference"]
    fn live_official_cli() {
        let exe = discover().expect("Claude Code installed");
        let root = std::env::temp_dir().join("quota-monitor-cli-live-test");
        let value = read_usage(&exe, &root).expect("get_usage");
        assert!(value["rate_limits_available"].as_bool().is_some());
        assert!(window(&value["rate_limits"]["five_hour"]).is_some());
    }
}
