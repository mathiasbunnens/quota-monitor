//! bartekczyz/ai-profiles at 26bf0e8b25a1908474ccf822200bb76698ae06cb.
//! Discovery and read-only usage transport. No wrapper/CLI execution or mutation.
//! Credentials are reloaded each cycle; only the owning CLI/manager renews them.
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::oauth_usage::{self, ReadError};
use std::time::Duration;

pub fn root(home: &Path) -> PathBuf {
    home.join("Library/Application Support/ai-profiles")
}

pub fn is_managed_home(home: &Path, path: &Path) -> bool {
    let base = crate::claude_accounts::normalized(&root(home).join("profiles"));
    let path = crate::claude_accounts::normalized(path);
    let Ok(relative) = path.strip_prefix(base) else {
        return false;
    };
    let parts: Vec<_> = relative.components().collect();
    parts.len() == 2 && parts[1].as_os_str() == "cli-config"
}

#[cfg(test)]
#[path = "ai_profiles_tests.rs"]
mod tests;

pub struct Profile {
    pub name: String,
    pub raw: PathBuf,
    pub complete: bool,
}

pub fn profiles(home: &Path, app: &str) -> Vec<Profile> {
    let root = root(home);
    let path = root.join("profiles.json");
    let Ok(meta) = std::fs::metadata(&path) else {
        return vec![];
    };
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return vec![];
    }
    let Some(value) = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    else {
        return vec![];
    };
    value["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let id = p["id"].as_str()?;
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            {
                return None;
            }
            if p.get("app").and_then(Value::as_str).unwrap_or("claude") != app {
                return None;
            }
            let name = p["name"].as_str()?.trim();
            if name.is_empty() || name.chars().any(char::is_control) {
                return None;
            }
            let dir = root.join("profiles").join(id);
            // Desktop-only entries use their REAL profile directory, not a fabricated home.
            let cli = p["surfaces"]["cli"] == true;
            let raw = if cli { dir.join("cli-config") } else { dir };
            Some(Profile {
                name: format!(
                    "{} · ai-profiles",
                    name.chars().take(48).collect::<String>()
                ),
                complete: cli && raw.is_dir(),
                raw,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeError {
    Read(ReadError),
    IdentityUnavailable,
    AccountMismatch,
    Changed,
}
impl From<ReadError> for ClaudeError {
    fn from(e: ReadError) -> Self {
        Self::Read(e)
    }
}

/// Hash the exact wrapper value, not a canonical path or the bare stock slot.
/// The identity endpoint below guards the 32-bit Keychain namespace collision.
pub fn keychain_service(raw: &Path) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(raw.to_string_lossy().as_bytes());
    let suffix: String = hash[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("Claude Code-credentials-{suffix}")
}

fn document(path: &Path) -> Option<Value> {
    serde_json::from_slice(&oauth_usage::read_bounded(path).ok()?).ok()
}
fn expected_account(home: &Path) -> Option<String> {
    document(&home.join(".claude.json"))?
        .pointer("/oauthAccount/accountUuid")?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn load_claude_with(
    home: &Path,
    keychain: &impl Fn(&str) -> Result<Vec<u8>, ReadError>,
) -> Result<oauth_usage::ClaudeCredential, ReadError> {
    let path = home.join(".credentials.json");
    let raw = match std::fs::symlink_metadata(&path) {
        Ok(_) => oauth_usage::read_bounded(&path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => keychain(&keychain_service(home))?,
        Err(_) => return Err(ReadError::CredentialUnreadable),
    };
    let value = oauth_usage::parse_claude_json(&raw)?;
    oauth_usage::claude_credential(&value)
}

/// Fail closed: unavailable identity cannot display a different account's usage.
fn read_claude_with(
    home: &Path,
    base: &str,
    timeout: Duration,
    now: i64,
    keychain: impl Fn(&str) -> Result<Vec<u8>, ReadError>,
) -> Result<Value, ClaudeError> {
    let credential = load_claude_with(home, &keychain)?;
    if credential.expired(now) {
        return Err(ReadError::Expired.into());
    }
    let expected = expected_account(home).ok_or(ClaudeError::IdentityUnavailable)?;
    let base = oauth_usage::checked_base(base)?;
    let response = oauth_usage::client(timeout)?
        .get(format!("{base}/api/oauth/profile"))
        .header("Authorization", format!("Bearer {}", credential.token()))
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .map_err(|e| {
            ClaudeError::Read(if e.is_timeout() {
                ReadError::Timeout
            } else {
                ReadError::Transport
            })
        })?;
    let status = response.status().as_u16();
    if status == 401 {
        return Err(ReadError::Unauthorized.into());
    }
    if status == 403 {
        return Err(ReadError::Forbidden.into());
    }
    if status == 429 {
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok());
        return Err(ReadError::RateLimited(retry).into());
    }
    if status >= 500 {
        return Err(ReadError::Http(status).into());
    }
    if status != 200 {
        return Err(ClaudeError::IdentityUnavailable);
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ClaudeError::IdentityUnavailable)?;
    if bytes.len() > 64 * 1024 {
        return Err(ClaudeError::IdentityUnavailable);
    }
    let identity: Value =
        serde_json::from_slice(&bytes).map_err(|_| ClaudeError::IdentityUnavailable)?;
    let uuid = identity
        .pointer("/account/uuid")
        .and_then(Value::as_str)
        .ok_or(ClaudeError::IdentityUnavailable)?;
    if !uuid.trim().eq_ignore_ascii_case(&expected) {
        return Err(ClaudeError::AccountMismatch);
    }
    let usage = oauth_usage::fetch_claude(base, &credential, timeout, now)?;
    if expected_account(home).as_deref() != Some(&expected)
        || !load_claude_with(home, &keychain)?.carries(credential.token())
    {
        return Err(ClaudeError::Changed);
    }
    Ok(oauth_usage::claude_usage_value(
        &usage,
        credential.subscription_type(),
    ))
}

pub fn read_claude(
    home: &Path,
    base: &str,
    timeout: Duration,
    now: i64,
) -> Result<Value, ClaudeError> {
    read_claude_with(home, base, timeout, now, read_keychain)
}

pub fn read_codex(
    home: &Path,
    base: &str,
    timeout: Duration,
    now: i64,
) -> Result<oauth_usage::CodexUsage, ReadError> {
    let credential = oauth_usage::load_codex(home)?;
    let raw = oauth_usage::fetch_codex(base, &credential, timeout, now)?;
    if !oauth_usage::load_codex(home)?.same_connection(&credential) {
        // Owner rotated or replaced the connection while the request was in flight.
        // Do not publish a reading attached to a different current connection.
        return Err(ReadError::CredentialUnreadable);
    }
    Ok(oauth_usage::codex_usage(credential, &raw))
}

// Unit tests MUST NOT touch the real Keychain. Tests inject a synthetic reader.
#[cfg(any(test, not(target_os = "macos")))]
fn read_keychain(_: &str) -> Result<Vec<u8>, ReadError> {
    Err(ReadError::NoToken)
}

#[cfg(all(not(test), target_os = "macos"))]
fn read_keychain(service: &str) -> Result<Vec<u8>, ReadError> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::Instant,
    };
    let user = std::env::var("USER")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or(ReadError::CredentialUnreadable)?;
    let mut child = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", service, "-a", &user, "-w"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ReadError::CredentialUnreadable)?;
    let stdout = child.stdout.take().ok_or(ReadError::CredentialUnreadable)?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| ReadError::CredentialUnreadable)?
        .map_err(|_| ReadError::CredentialUnreadable)?;
    match status {
        None => Err(ReadError::Timeout),
        Some(s) if s.code() == Some(44) => Err(ReadError::NoToken),
        Some(s) if s.success() && bytes.len() <= 64 * 1024 => Ok(bytes),
        _ => Err(ReadError::CredentialUnreadable),
    }
}

pub fn codex_error(e: ReadError) -> String {
    let detail = match e {
        ReadError::Expired | ReadError::Unauthorized => "Session expirée ou révoquée. Renouvelez-la dans le CLI/manager de ce profil ; Quota Monitor reprendra au prochain cycle, sans faire tourner un grant partagé.",
        ReadError::NoToken => "Non connecté : aucun jeton OAuth de compte dans ce profil.",
        ReadError::CredentialUnreadable => "Connexion absente ou illisible dans ce profil.",
        ReadError::Forbidden => "Lecture des quotas refusée (HTTP 403).",
        ReadError::RateLimited(_) => "Lecture limitée par le fournisseur ; nouvelle tentative après la pause.",
        ReadError::Timeout | ReadError::Transport => "Lecture impossible (réseau ou délai dépassé).",
        ReadError::Malformed => "Réponse du fournisseur inexploitable, aucun quota inventé.",
        ReadError::Http(_) => "Erreur HTTP du fournisseur ; nouvelle tentative plus tard.",
        _ => "Lecture de la session indisponible ; aucune modification de la connexion.",
    };
    format!("ai-profiles : {detail}")
}
