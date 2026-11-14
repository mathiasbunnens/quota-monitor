//! Renewal of the OAuth credentials saved in `aip` profiles, so their quotas
//! stay readable after the access token expires (authorised by the user:
//! « vas-y, tu peux lancer ça pour que ça reste actif »). Only the renewed
//! tokens are written back, into the profile's own file, with the provider's
//! own protocol; nothing else in the profile (`_current`, `_order`, markers,
//! other fields of the document) is touched, no account is switched, no
//! keychain is read or written, no CLI is launched.
//!
//! ## Claude (`<profile>/credentials.json`) — aip's own protocol
//! dqn/aip 3961fa0 `src/claude/usage.rs::load_fresh_credentials` (checked
//! again on 2026-11-05: still the head of `main`):
//! 1. fast path: read; fresh when `now + 5 min < expiresAt` (a snapshot
//!    without `expiresAt` is treated as expired);
//! 2. in-process gate, then `flock(LOCK_EX | LOCK_NB)` on
//!    `<file name>.lock` (`credentials.json.lock`, created empty, never
//!    truncated), polled every 100 ms for 10 s (aip: `fd_lock::RwLock::try_write`);
//! 3. re-read under the lock: another task/process may have rotated already;
//! 4. `POST https://platform.claude.com/v1/oauth/token`, form-encoded
//!    `grant_type=refresh_token`, `refresh_token`, `client_id`
//!    (`9d1c250a-e61b-44d9-88ed-5944d1962f5e`), `scope` (saved scopes or the
//!    Claude Code defaults);
//! 5. `accessToken`, `refreshToken` (when returned) and `expiresAt = now +
//!    clamp(expires_in, 60 s, 1 day)` (ms) replaced in the document, every
//!    other field kept; pretty JSON written to a temporary file in the same
//!    folder then renamed over `credentials.json`, mode 0600; lock released.
//! The same lock file and the same `flock` make this renewal mutually
//! exclusive with aip's own (tested with another process, see tests).
//!
//! ## Codex (`<home>/auth.json`) — the CLI's protocol, aip's guard
//! codex-rs 0.164.0 (`login/src/auth/manager.rs`, `oauth/client.rs`,
//! `storage.rs`): `POST https://auth.openai.com/oauth/token`, JSON
//! `{client_id: app_EMoamEEZ73f0CkXaXp7hrann, grant_type: refresh_token,
//! refresh_token, refresh_reason}` → `{id_token?, access_token?,
//! refresh_token?}`; the CLI persists them into `auth.json` (`tokens.*`,
//! `last_refresh`) with mode 0600. Refresh tokens rotate and a reuse is
//! detected (`refresh_token_reused` → the CLI logs the account out), so one
//! grant must have one owner. The CLI has NO cross-process lock: its guard is
//! a reload of `auth.json` before every refresh (an externally rotated token
//! is adopted, `ReloadOutcome::ReloadedChanged`) and a same-login check before
//! persisting. This module follows that protocol (re-read before refresh,
//! rotation written atomically) and rotates earlier than the CLI would
//! (margin ≥ 10 min, the CLI's own is 5 min), so a CLI running on the same
//! home finds the fresh tokens instead of competing. A private `auth.json.lock`
//! only serialises two Quota Monitor instances; it does not and cannot
//! protect the CLI (documented limit).
//! aip (`src/codex/profile.rs`) refuses to run a profile whose refresh token
//! is still the one imported from the source session (`aip-imported-refresh.
//! sha256`, SHA-256 lines, « run 'aip codex login' once »): rotating such a
//! grant would log the source session out. The same rule applies here, plus
//! a fingerprint comparison with the sibling profiles and the primary home.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::oauth_usage::{self, ClaudeCredential, CodexCredential, ReadError};

pub const CLAUDE_TOKEN_BASE: &str = "https://platform.claude.com";
pub const CLAUDE_TOKEN_PATH: &str = "/v1/oauth/token";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Claude Code's OAuth scopes (aip `DEFAULT_SCOPES`), used when the snapshot
/// saved none.
const CLAUDE_DEFAULT_SCOPES: &[&str] = &[
    "user:profile",
    "user:inference",
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
];
/// aip: `now + 300 s >= expiresAt` → refresh.
pub const CLAUDE_REFRESH_MARGIN_SECS: i64 = 300;

pub const CODEX_TOKEN_BASE: &str = "https://auth.openai.com";
pub const CODEX_TOKEN_PATH: &str = "/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Floor of the Codex margin (the CLI refreshes at 5 min; rotating earlier
/// keeps a running CLI out of the race).
pub const CODEX_REFRESH_MARGIN_SECS: i64 = 600;
/// aip's imported-grant marker (SHA-256 hex of each imported refresh token).
pub const CODEX_IMPORT_MARKER: &str = "aip-imported-refresh.sha256";

/// aip: 10 s deadline, 100 ms polls.
const LOCK_WAIT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(100);

/// The token endpoint lives next to the usage endpoint only for loopback test
/// servers (one server answers both); production bases are fixed in code.
pub fn claude_token_base(usage_base: &str) -> &str {
    if usage_base == oauth_usage::CLAUDE_USAGE_BASE {
        CLAUDE_TOKEN_BASE
    } else {
        usage_base
    }
}
pub fn codex_token_base(usage_base: &str) -> &str {
    if usage_base == oauth_usage::CODEX_USAGE_BASE {
        CODEX_TOKEN_BASE
    } else {
        usage_base
    }
}

/// In-process gate (aip `REFRESH_GATE`): two collectors never spend the same
/// single-use refresh token at once, whatever the file lock does.
static GATE: Mutex<()> = Mutex::new(());

/// Rotations the provider accepted but the profile file refused: the new
/// document waits in memory and its save is retried on the next cycle; the
/// consumed refresh token is never sent again.
struct Unsaved {
    consumed: String,
    document: Value,
}
/// Behind its own mutex (not `GATE`, which is held across the network call).
static UNSAVED: Mutex<Option<HashMap<PathBuf, Unsaved>>> = Mutex::new(None);

fn with_unsaved<T>(f: impl FnOnce(&mut HashMap<PathBuf, Unsaved>) -> T) -> T {
    let mut guard = UNSAVED.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// SHA-256 hex of a refresh token (aip `refresh_fingerprint`); the token
/// itself is never kept.
pub(crate) fn fingerprint(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ----------------------------------------------------------------- files

/// The credential file and its lock, both required to be regular files (a
/// symlink would be replaced by the rename; aip never writes through one).
fn safe_paths(dir: &Path, payload: &str) -> Result<(PathBuf, PathBuf), ReadError> {
    let file = dir.join(payload);
    let meta = std::fs::symlink_metadata(&file).map_err(|_| ReadError::RefreshUnavailable)?;
    if !meta.is_file() {
        return Err(ReadError::RefreshUnavailable);
    }
    // aip `lock_path`: the full file name + ".lock" (not `with_extension`).
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    let lock = file.with_file_name(name);
    if let Ok(meta) = std::fs::symlink_metadata(&lock) {
        if !meta.is_file() {
            return Err(ReadError::RefreshUnavailable);
        }
    }
    Ok((file, lock))
}

/// Exclusive advisory lock on the lock file, released on drop whatever
/// happens (unlock + close).
pub(crate) struct LockGuard {
    #[cfg_attr(not(unix), allow(dead_code))]
    file: std::fs::File,
}
#[cfg(unix)]
impl Drop for LockGuard {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: plain flock(2) on a file descriptor this guard owns.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(unix)]
fn try_lock(file: &std::fs::File) -> Result<bool, ReadError> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: plain flock(2) on an open descriptor; same call as aip's
    // fd-lock `try_write` (rustix `flock(NonBlockingLockExclusive)`).
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EWOULDBLOCK) | Some(libc::EINTR) => Ok(false),
        _ => Err(ReadError::RefreshUnavailable),
    }
}
#[cfg(not(unix))]
fn try_lock(_file: &std::fs::File) -> Result<bool, ReadError> {
    // aip is unix-only; no compatible lock exists here, so no renewal.
    Err(ReadError::RefreshUnavailable)
}

/// aip: create (never truncate) the lock file, poll `try_write` until the
/// deadline. Public for the cross-process test helper.
pub(crate) fn acquire_lock(lock: &Path, wait: Duration) -> Result<LockGuard, ReadError> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock)
        .map_err(|_| ReadError::RefreshUnavailable)?;
    let deadline = Instant::now() + wait;
    loop {
        if try_lock(&file)? {
            return Ok(LockGuard { file });
        }
        if Instant::now() >= deadline {
            return Err(ReadError::RefreshLocked);
        }
        std::thread::sleep(LOCK_POLL);
    }
}

/// Same-folder temporary file (created exclusively, 0600), written, synced
/// and renamed over `file`. The old file stays intact on any failure.
fn write_atomic(file: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = file.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{name}.qm-{}-{nonce}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut out = options.open(&tmp)?;
        out.write_all(content.as_bytes())?;
        out.sync_all()?;
        drop(out);
        std::fs::rename(&tmp, file)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(unix)]
fn restrict(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn restrict(_file: &Path) {}

/// Refresh-token fingerprints of the other credential files that could share
/// the grant: sibling profiles (same `profiles` folder) and `extra` (the
/// primary home for Codex). Unreadable files are ignored.
fn peer_fingerprints(
    dir: &Path,
    payload: &str,
    extra: Option<&Path>,
    field: &[&str],
) -> Vec<String> {
    let me = crate::claude_accounts::normalized(dir);
    let mut peers: Vec<PathBuf> = vec![];
    if let Some(parent) = me.parent() {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                if crate::claude_accounts::normalized(&path) != me {
                    peers.push(path);
                }
            }
        }
    }
    if let Some(extra) = extra {
        if crate::claude_accounts::normalized(extra) != me {
            peers.push(extra.to_path_buf());
        }
    }
    peers
        .iter()
        .filter_map(|p| oauth_usage::read_bounded(&p.join(payload)).ok())
        .filter_map(|raw| {
            let value: Value = serde_json::from_slice(&raw).ok()?;
            let mut cursor = &value;
            for key in field {
                cursor = cursor.get(key)?;
            }
            oauth_usage::non_empty(cursor).map(|t| fingerprint(&t))
        })
        .collect()
}

// ------------------------------------------------------------------ HTTP

enum Body<'a> {
    Form(Vec<(&'a str, String)>),
    Json(Value),
}

/// `application/x-www-form-urlencoded` (RFC 1866: space → `+`, everything
/// outside the unreserved set percent-encoded), as reqwest's `.form()` does.
fn form_encode(fields: &[(&str, String)]) -> String {
    fn encode(s: &str, out: &mut String) {
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                    out.push(b as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
    }
    let mut out = String::new();
    for (i, (k, v)) in fields.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        encode(k, &mut out);
        out.push('=');
        encode(v, &mut out);
    }
    out
}

/// `POST {base}{path}` to the provider's token endpoint. Status mapping:
/// 2xx object → `Ok`; 401/403, or 400 whose body says `invalid_grant` /
/// `refresh_token_*` → `RefreshRejected`; 429 → `RateLimited`; other
/// statuses → `Http`; transport → `Timeout`/`Transport`. The response body
/// never leaves this function except as the parsed token object.
fn post_token(base: &str, path: &str, timeout: Duration, body: Body) -> Result<Value, ReadError> {
    use std::io::Read;
    let base = oauth_usage::checked_base(base)?;
    let request = oauth_usage::client(timeout)?
        .post(format!("{base}{path}"))
        .header("Accept", "application/json");
    let request = match body {
        Body::Form(fields) => request
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form_encode(&fields)),
        Body::Json(value) => request
            .header("Content-Type", "application/json")
            .body(value.to_string()),
    };
    let response = request.send().map_err(|e| {
        if e.is_timeout() {
            ReadError::Timeout
        } else {
            ReadError::Transport
        }
    })?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let mut bytes = Vec::new();
    let _ = response.take(64 * 1024).read_to_end(&mut bytes);
    let parsed: Option<Value> = serde_json::from_slice(&bytes).ok();
    match status {
        200..=299 => parsed.filter(Value::is_object).ok_or(ReadError::Malformed),
        401 | 403 => Err(ReadError::RefreshRejected),
        400 => {
            let code = parsed.as_ref().and_then(|v| {
                oauth_usage::non_empty(&v["error"])
                    .or_else(|| oauth_usage::non_empty(&v["error"]["code"]))
                    .or_else(|| oauth_usage::non_empty(&v["error"]["type"]))
                    .or_else(|| oauth_usage::non_empty(&v["code"]))
            });
            let rejected = code.as_deref().is_some_and(|c| {
                let c = c.to_ascii_lowercase();
                c == "invalid_grant" || c.starts_with("refresh_token_")
            });
            Err(if rejected {
                ReadError::RefreshRejected
            } else {
                ReadError::Http(400)
            })
        }
        429 => Err(ReadError::RateLimited(retry_after)),
        other => Err(ReadError::Http(other)),
    }
}

// ---------------------------------------------------------------- Claude

fn claude_refresh_token(doc: &Value) -> Option<String> {
    oauth_usage::non_empty(&doc["claudeAiOauth"]["refreshToken"])
}

/// aip `is_token_expired`: no `expiresAt` → refresh; else `now + 300 s >= exp`.
pub(crate) fn claude_needs_refresh(doc: &Value, now: i64) -> bool {
    match oauth_usage::unix_seconds(&doc["claudeAiOauth"]["expiresAt"]) {
        None => true,
        Some(exp) => now.saturating_add(CLAUDE_REFRESH_MARGIN_SECS) >= exp,
    }
}

/// aip `apply_token_response` on the document (other fields untouched).
fn apply_claude(doc: &mut Value, response: &Value, now: i64) -> Result<(), ReadError> {
    let access = oauth_usage::non_empty(&response["access_token"]).ok_or(ReadError::Malformed)?;
    let oauth = doc
        .get_mut("claudeAiOauth")
        .filter(|v| v.is_object())
        .ok_or(ReadError::Malformed)?;
    oauth["accessToken"] = Value::String(access);
    if let Some(refresh) = oauth_usage::non_empty(&response["refresh_token"]) {
        oauth["refreshToken"] = Value::String(refresh);
    }
    let expires_in = response["expires_in"]
        .as_i64()
        .unwrap_or(3600)
        .clamp(60, 86_400);
    oauth["expiresAt"] = Value::from(now.saturating_add(expires_in).saturating_mul(1000));
    Ok(())
}

fn claude_document(file: &Path) -> Result<Value, ReadError> {
    oauth_usage::parse_claude_json(&oauth_usage::read_bounded(file)?)
}

/// Whether this document's grant may be rotated from here.
fn claude_eligibility(dir: &Path, doc: &Value) -> Result<(), ReadError> {
    let token = claude_refresh_token(doc).ok_or(ReadError::NoRefreshToken)?;
    let mine = fingerprint(&token);
    if peer_fingerprints(
        dir,
        crate::aip::CLAUDE_PAYLOAD,
        None,
        &["claudeAiOauth", "refreshToken"],
    )
    .contains(&mine)
    {
        return Err(ReadError::SharedGrant);
    }
    Ok(())
}

/// The profile's credential, renewed first when it is (about to be) expired
/// or when `rejected` is the access token the provider just refused.
/// Renewal is skipped (the current credential is returned while it lasts)
/// when the grant cannot be rotated safely: no refresh token, grant shared
/// with another profile file, unsafe paths; once the token is really dead the
/// matching error is returned instead.
pub fn fresh_claude(
    dir: &Path,
    token_base: &str,
    timeout: Duration,
    now: i64,
    rejected: Option<&str>,
) -> Result<ClaudeCredential, ReadError> {
    fresh_claude_with(dir, token_base, timeout, now, rejected).map(|(c, _)| c)
}

/// [`fresh_claude`] plus whether this call rotated the grant.
fn fresh_claude_with(
    dir: &Path,
    token_base: &str,
    timeout: Duration,
    now: i64,
    rejected: Option<&str>,
) -> Result<(ClaudeCredential, bool), ReadError> {
    let file = dir.join(crate::aip::CLAUDE_PAYLOAD);
    let doc = claude_document(&file)?;
    let credential = oauth_usage::claude_credential(&doc)?;
    let pending = with_unsaved(|m| {
        m.get(&file).and_then(|u| {
            (claude_refresh_token(&doc)
                .map(|t| fingerprint(&t))
                .as_deref()
                == Some(u.consumed.as_str()))
            .then(|| u.document.clone())
        })
    });
    let confirmed_dead = rejected.is_some_and(|t| credential.carries(t));
    if pending.is_none() && !confirmed_dead && !claude_needs_refresh(&doc, now) {
        return Ok((credential, false));
    }
    if let Err(e) = claude_eligibility(dir, &doc) {
        if pending.is_some() {
            // Already rotated in memory: only the save matters now.
        } else if confirmed_dead || credential.expired(now) {
            return Err(e);
        } else {
            return Ok((credential, false));
        }
    }
    let _gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
    let (file, lock) = safe_paths(dir, crate::aip::CLAUDE_PAYLOAD)?;
    let _lock = acquire_lock(&lock, LOCK_WAIT)?;
    // Re-read under the lock: aip or another process may have rotated.
    let mut doc = claude_document(&file)?;
    let current_refresh = claude_refresh_token(&doc);
    if let Some(document) = pending {
        let still_stale = current_refresh
            .as_deref()
            .map(fingerprint)
            .is_some_and(|f| with_unsaved(|m| m.get(&file).is_some_and(|u| u.consumed == f)));
        if still_stale {
            return match write_atomic(&file, &pretty(&document)) {
                Ok(()) => {
                    restrict(&file);
                    with_unsaved(|m| m.remove(&file));
                    oauth_usage::claude_credential(&document).map(|c| (c, true))
                }
                Err(_) => Err(ReadError::RefreshUnsaved),
            };
        }
        with_unsaved(|m| m.remove(&file)); // someone re-saved the profile
    }
    let credential = oauth_usage::claude_credential(&doc)?;
    let confirmed_dead = rejected.is_some_and(|t| credential.carries(t));
    if !confirmed_dead && !claude_needs_refresh(&doc, now) {
        return Ok((credential, false));
    }
    if let Err(e) = claude_eligibility(dir, &doc) {
        return if confirmed_dead || credential.expired(now) {
            Err(e)
        } else {
            Ok((credential, false))
        };
    }
    let refresh = current_refresh.ok_or(ReadError::NoRefreshToken)?;
    let scopes: Vec<String> = doc["claudeAiOauth"]["scopes"]
        .as_array()
        .map(|s| s.iter().filter_map(oauth_usage::non_empty).collect())
        .unwrap_or_default();
    let scope = if scopes.is_empty() {
        CLAUDE_DEFAULT_SCOPES.join(" ")
    } else {
        scopes.join(" ")
    };
    let consumed = fingerprint(&refresh);
    let response = post_token(
        token_base,
        CLAUDE_TOKEN_PATH,
        timeout,
        Body::Form(vec![
            ("grant_type", "refresh_token".into()),
            ("refresh_token", refresh),
            ("client_id", CLAUDE_CLIENT_ID.into()),
            ("scope", scope),
        ]),
    )?;
    apply_claude(&mut doc, &response, now)?;
    match write_atomic(&file, &pretty(&doc)) {
        Ok(()) => {
            restrict(&file);
            oauth_usage::claude_credential(&doc).map(|c| (c, true))
        }
        Err(_) => {
            with_unsaved(|m| {
                m.insert(
                    file.clone(),
                    Unsaved {
                        consumed,
                        document: doc,
                    },
                )
            });
            Err(ReadError::RefreshUnsaved)
        }
    }
}

fn pretty(doc: &Value) -> String {
    serde_json::to_string_pretty(doc).unwrap_or_default()
}

/// Reads the profile's quotas, renewing the credential first when needed and
/// once more (bounded) when the provider refuses the token it still holds.
pub fn read_claude_profile(
    dir: &Path,
    usage_base: &str,
    timeout: Duration,
    now: i64,
) -> Result<Value, ReadError> {
    let token_base = claude_token_base(usage_base);
    let (credential, rotated) = fresh_claude_with(dir, token_base, timeout, now, None)?;
    match oauth_usage::fetch_claude(usage_base, &credential, timeout, now) {
        Ok(raw) => Ok(oauth_usage::claude_usage_value(
            &raw,
            credential.subscription_type(),
        )),
        Err(ReadError::Unauthorized) if !rotated => {
            // One bounded retry: the provider refused a token the file said
            // was valid. Under the lock, a token rotated meanwhile by another
            // process is used as is; otherwise the grant is refreshed once.
            // A token just rotated here is never refreshed twice in a cycle.
            let renewed = fresh_claude(dir, token_base, timeout, now, Some(credential.token()))?;
            let raw = oauth_usage::fetch_claude(usage_base, &renewed, timeout, now)?;
            Ok(oauth_usage::claude_usage_value(
                &raw,
                renewed.subscription_type(),
            ))
        }
        Err(e) => Err(e),
    }
}

// ----------------------------------------------------------------- Codex

fn codex_refresh_token(doc: &Value) -> Option<String> {
    oauth_usage::non_empty(&doc["tokens"]["refresh_token"])
}

/// `exp - margin <= now`; an opaque token (no `exp`) is only refreshed when
/// the provider refused it.
pub(crate) fn codex_needs_refresh(credential: &CodexCredential, now: i64, margin: i64) -> bool {
    credential
        .expires_at()
        .is_some_and(|exp| exp.saturating_sub(margin) <= now)
}

/// aip `check_refresh_ownership`: no marker (legacy snapshot) or a listed
/// fingerprint means the grant is the source session's.
fn codex_imported(home: &Path, mine: &str) -> bool {
    match std::fs::read_to_string(home.join(CODEX_IMPORT_MARKER)) {
        Ok(marker) => marker.lines().any(|line| line.trim() == mine),
        Err(_) => true,
    }
}

/// codex-rs `persist_tokens` on the document (other fields untouched).
fn apply_codex(doc: &mut Value, response: &Value, now: i64) -> Result<(), ReadError> {
    let access = oauth_usage::non_empty(&response["access_token"]).ok_or(ReadError::Malformed)?;
    let tokens = doc
        .get_mut("tokens")
        .filter(|v| v.is_object())
        .ok_or(ReadError::Malformed)?;
    tokens["access_token"] = Value::String(access);
    if let Some(id) = oauth_usage::non_empty(&response["id_token"]) {
        tokens["id_token"] = Value::String(id);
    }
    if let Some(refresh) = oauth_usage::non_empty(&response["refresh_token"]) {
        tokens["refresh_token"] = Value::String(refresh);
    }
    let stamp = chrono::DateTime::from_timestamp(now, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
        .unwrap_or_default();
    doc["last_refresh"] = Value::String(stamp);
    Ok(())
}

fn codex_document(file: &Path) -> Result<Value, ReadError> {
    oauth_usage::parse_codex_json(&oauth_usage::read_bounded(file)?)
}

/// Whether this home's grant may be rotated from here (aip's imported-grant
/// marker, then sibling profiles and the primary home).
fn codex_eligibility(home: &Path, doc: &Value, primary: Option<&Path>) -> Result<(), ReadError> {
    let token = codex_refresh_token(doc).ok_or(ReadError::NoRefreshToken)?;
    let mine = fingerprint(&token);
    if codex_imported(home, &mine)
        || peer_fingerprints(
            home,
            crate::aip::CODEX_PAYLOAD,
            primary,
            &["tokens", "refresh_token"],
        )
        .contains(&mine)
    {
        return Err(ReadError::SharedGrant);
    }
    Ok(())
}

/// Codex counterpart of [`fresh_claude`]. `margin` is the renewal lead (at
/// least [`CODEX_REFRESH_MARGIN_SECS`]); `primary` is the primary home whose
/// grant must never be rotated from here.
pub fn fresh_codex(
    home: &Path,
    token_base: &str,
    timeout: Duration,
    now: i64,
    rejected: Option<&str>,
    margin: i64,
    primary: Option<&Path>,
) -> Result<CodexCredential, ReadError> {
    fresh_codex_with(home, token_base, timeout, now, rejected, margin, primary).map(|(c, _)| c)
}

fn fresh_codex_with(
    home: &Path,
    token_base: &str,
    timeout: Duration,
    now: i64,
    rejected: Option<&str>,
    margin: i64,
    primary: Option<&Path>,
) -> Result<(CodexCredential, bool), ReadError> {
    let margin = margin.max(CODEX_REFRESH_MARGIN_SECS);
    let file = home.join(crate::aip::CODEX_PAYLOAD);
    let doc = codex_document(&file)?;
    let credential = oauth_usage::codex_credential(&doc)?;
    let pending = with_unsaved(|m| {
        m.get(&file).and_then(|u| {
            (codex_refresh_token(&doc)
                .map(|t| fingerprint(&t))
                .as_deref()
                == Some(u.consumed.as_str()))
            .then(|| u.document.clone())
        })
    });
    let confirmed_dead = rejected.is_some_and(|t| credential.carries(t));
    if pending.is_none() && !confirmed_dead && !codex_needs_refresh(&credential, now, margin) {
        return Ok((credential, false));
    }
    if let Err(e) = codex_eligibility(home, &doc, primary) {
        if pending.is_some() {
            // Already rotated in memory: only the save matters now.
        } else if confirmed_dead || credential.expired(now) {
            return Err(e);
        } else {
            return Ok((credential, false));
        }
    }
    let _gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
    let (file, lock) = safe_paths(home, crate::aip::CODEX_PAYLOAD)?;
    let _lock = acquire_lock(&lock, LOCK_WAIT)?;
    // The CLI's guarded reload, in reverse: re-read before refreshing so a
    // rotation done by the CLI (or another Quota Monitor) is adopted.
    let mut doc = codex_document(&file)?;
    let current_refresh = codex_refresh_token(&doc);
    if let Some(document) = pending {
        let still_stale = current_refresh
            .as_deref()
            .map(fingerprint)
            .is_some_and(|f| with_unsaved(|m| m.get(&file).is_some_and(|u| u.consumed == f)));
        if still_stale {
            return match write_atomic(&file, &pretty(&document)) {
                Ok(()) => {
                    restrict(&file);
                    with_unsaved(|m| m.remove(&file));
                    oauth_usage::codex_credential(&document).map(|c| (c, true))
                }
                Err(_) => Err(ReadError::RefreshUnsaved),
            };
        }
        with_unsaved(|m| m.remove(&file));
    }
    let credential = oauth_usage::codex_credential(&doc)?;
    let confirmed_dead = rejected.is_some_and(|t| credential.carries(t));
    if !confirmed_dead && !codex_needs_refresh(&credential, now, margin) {
        return Ok((credential, false));
    }
    if let Err(e) = codex_eligibility(home, &doc, primary) {
        return if confirmed_dead || credential.expired(now) {
            Err(e)
        } else {
            Ok((credential, false))
        };
    }
    let refresh = current_refresh.ok_or(ReadError::NoRefreshToken)?;
    let consumed = fingerprint(&refresh);
    let response = post_token(
        token_base,
        CODEX_TOKEN_PATH,
        timeout,
        Body::Json(json!({
            "client_id": CODEX_CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh,
            "refresh_reason": if confirmed_dead { "unauthorized_recovery" } else { "proactive_expiry" },
        })),
    )?;
    apply_codex(&mut doc, &response, now)?;
    match write_atomic(&file, &pretty(&doc)) {
        Ok(()) => {
            restrict(&file);
            oauth_usage::codex_credential(&doc).map(|c| (c, true))
        }
        Err(_) => {
            with_unsaved(|m| {
                m.insert(
                    file.clone(),
                    Unsaved {
                        consumed,
                        document: doc,
                    },
                )
            });
            Err(ReadError::RefreshUnsaved)
        }
    }
}

/// Codex counterpart of [`read_claude_profile`].
pub fn read_codex_home(
    home: &Path,
    usage_base: &str,
    timeout: Duration,
    now: i64,
    margin: i64,
    primary: Option<&Path>,
) -> Result<oauth_usage::CodexUsage, ReadError> {
    let token_base = codex_token_base(usage_base);
    let (credential, rotated) =
        fresh_codex_with(home, token_base, timeout, now, None, margin, primary)?;
    let (credential, raw) = match oauth_usage::fetch_codex(usage_base, &credential, timeout, now) {
        Ok(raw) => (credential, raw),
        Err(ReadError::Unauthorized) if !rotated => {
            let renewed = fresh_codex(
                home,
                token_base,
                timeout,
                now,
                Some(credential.token()),
                margin,
                primary,
            )?;
            let raw = oauth_usage::fetch_codex(usage_base, &renewed, timeout, now)?;
            (renewed, raw)
        }
        Err(e) => return Err(e),
    };
    Ok(oauth_usage::codex_usage(credential, &raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth_usage::test_support::{jwt, Reply, Server};
    use std::fs;

    const NOW: i64 = 1_780_000_000;
    const T: Duration = Duration::from_secs(2);
    const OLD_ACCESS: &str = "sk-ant-oat01-SYNTHETIC-old";
    const OLD_REFRESH: &str = "sk-ant-ort01-SYNTHETIC-old";
    const NEW_ACCESS: &str = "sk-ant-oat01-SYNTHETIC-new";
    const NEW_REFRESH: &str = "sk-ant-ort01-SYNTHETIC-new";

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("qm-refresh-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }
    fn claude_doc(access: &str, refresh: Option<&str>, expires: i64) -> Value {
        let mut oauth = json!({"accessToken": access, "expiresAt": expires * 1000,
            "scopes": ["user:inference", "user:profile"], "subscriptionType": "max",
            "rateLimitTier": "default_claude_max_5x", "unknownFutureField": {"keep": true}});
        if let Some(r) = refresh {
            oauth["refreshToken"] = Value::String(r.into());
        }
        json!({"claudeAiOauth": oauth, "otherTopLevel": "kept"})
    }
    /// `<root>/profiles/<name>/credentials.json` (aip layout).
    fn claude_profile(root: &Path, name: &str, doc: &Value) -> PathBuf {
        let dir = root.join("profiles").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(crate::aip::CLAUDE_PAYLOAD), doc.to_string()).unwrap();
        dir
    }
    fn claude_file(dir: &Path) -> Value {
        serde_json::from_slice(&fs::read(dir.join(crate::aip::CLAUDE_PAYLOAD)).unwrap()).unwrap()
    }
    fn token_reply(access: &str, refresh: Option<&str>, expires_in: i64) -> Reply {
        let mut body =
            json!({"access_token": access, "token_type": "Bearer", "expires_in": expires_in});
        if let Some(r) = refresh {
            body["refresh_token"] = Value::String(r.into());
        }
        Reply::json(200, &body.to_string())
    }
    const USAGE_BODY: &str = r#"{"five_hour":{"utilization":12.5,"resets_at":"2026-07-14T17:30:00+00:00"},
        "seven_day":{"utilization":61,"resets_at":"2026-07-19T12:00:00+00:00"},"limits":[]}"#;
    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// Full cycle on aip's protocol: an expiring snapshot → form POST to the
    /// token path with the refresh grant → new tokens written (other fields
    /// kept, 0600) → usage read with the new bearer → a later cycle reads
    /// again without any refresh.
    #[test]
    fn claude_expiring_snapshot_is_renewed_written_and_read_then_left_alone() {
        let root = scratch("claude-cycle");
        let dir = claude_profile(
            &root,
            "work",
            &claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW + 120),
        );
        let server = Server::start(vec![
            token_reply(NEW_ACCESS, Some(NEW_REFRESH), 3600),
            Reply::json(200, USAGE_BODY),
            Reply::json(200, USAGE_BODY),
        ]);
        let value = read_claude_profile(&dir, &server.base, T, NOW).unwrap();
        assert_eq!(value["rate_limits"]["five_hour"]["utilization"], 12.5);
        assert_eq!(value["subscription_type"], "max");
        let requests = server.requests();
        assert_eq!(requests.len(), 2, "{requests:?}");
        assert!(
            requests[0].starts_with("POST /v1/oauth/token HTTP/1.1"),
            "{}",
            requests[0]
        );
        assert_eq!(
            Server::header(&requests[0], "content-type").as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        let form = requests[0].rsplit("\r\n\r\n").next().unwrap();
        assert_eq!(
            form,
            format!(
                "grant_type=refresh_token&refresh_token={OLD_REFRESH}&client_id={CLAUDE_CLIENT_ID}&scope=user%3Ainference+user%3Aprofile"
            )
        );
        assert!(
            Server::header(&requests[0], "authorization").is_none(),
            "no bearer on the token request"
        );
        assert!(requests[1].starts_with("GET /api/oauth/usage HTTP/1.1"));
        assert_eq!(
            Server::header(&requests[1], "authorization").as_deref(),
            Some(&*format!("Bearer {NEW_ACCESS}"))
        );
        let saved = claude_file(&dir);
        assert_eq!(saved["claudeAiOauth"]["accessToken"], NEW_ACCESS);
        assert_eq!(saved["claudeAiOauth"]["refreshToken"], NEW_REFRESH);
        assert_eq!(saved["claudeAiOauth"]["expiresAt"], (NOW + 3600) * 1000);
        assert_eq!(
            saved["claudeAiOauth"]["scopes"],
            json!(["user:inference", "user:profile"])
        );
        assert_eq!(
            saved["claudeAiOauth"]["rateLimitTier"],
            "default_claude_max_5x"
        );
        assert_eq!(saved["claudeAiOauth"]["unknownFutureField"]["keep"], true);
        assert_eq!(saved["otherTopLevel"], "kept");
        #[cfg(unix)]
        assert_eq!(mode(&dir.join(crate::aip::CLAUDE_PAYLOAD)), 0o600);
        assert!(
            dir.join("credentials.json.lock").is_file(),
            "aip's lock file"
        );
        assert!(!fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        // Later cycle: fresh (now + 3600 > margin) → usage only.
        let value = read_claude_profile(&dir, &server.base, T, NOW + 60).unwrap();
        assert_eq!(value["rate_limits"]["seven_day"]["utilization"], 61.0);
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].starts_with("GET /api/oauth/usage "));
        fs::remove_dir_all(root).unwrap();
    }

    /// aip's rules: no `expiresAt` → refresh; margin exactly 5 min; the
    /// response without a new refresh token keeps the old one; `expires_in`
    /// absent → 1 h, clamped to [60 s, 1 day]; hex snapshot written as JSON.
    #[test]
    fn claude_refresh_rules_match_aip() {
        assert!(claude_needs_refresh(&json!({"claudeAiOauth": {}}), NOW));
        assert!(claude_needs_refresh(&claude_doc("a", None, NOW + 300), NOW));
        assert!(!claude_needs_refresh(
            &claude_doc("a", None, NOW + 301),
            NOW
        ));
        let mut doc = claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW);
        apply_claude(&mut doc, &json!({"access_token": NEW_ACCESS}), NOW).unwrap();
        assert_eq!(doc["claudeAiOauth"]["refreshToken"], OLD_REFRESH);
        assert_eq!(doc["claudeAiOauth"]["expiresAt"], (NOW + 3600) * 1000);
        apply_claude(
            &mut doc,
            &json!({"access_token": NEW_ACCESS, "expires_in": 5}),
            NOW,
        )
        .unwrap();
        assert_eq!(doc["claudeAiOauth"]["expiresAt"], (NOW + 60) * 1000);
        apply_claude(
            &mut doc,
            &json!({"access_token": NEW_ACCESS, "expires_in": 999_999}),
            NOW,
        )
        .unwrap();
        assert_eq!(doc["claudeAiOauth"]["expiresAt"], (NOW + 86_400) * 1000);
        assert_eq!(
            apply_claude(&mut doc, &json!({"token_type": "Bearer"}), NOW).err(),
            Some(ReadError::Malformed)
        );
        let root = scratch("claude-hex");
        let hex: String = claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW - 10)
            .to_string()
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        let dir = root.join("profiles").join("hex");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(crate::aip::CLAUDE_PAYLOAD), hex).unwrap();
        let server = Server::start(vec![token_reply(NEW_ACCESS, None, 600)]);
        let c = fresh_claude(&dir, &server.base, T, NOW, None).unwrap();
        assert!(c.carries(NEW_ACCESS));
        let saved = claude_file(&dir);
        assert_eq!(saved["claudeAiOauth"]["accessToken"], NEW_ACCESS);
        assert_eq!(saved["claudeAiOauth"]["refreshToken"], OLD_REFRESH, "kept");
        assert_eq!(saved["otherTopLevel"], "kept");
        fs::remove_dir_all(root).unwrap();
    }

    /// Provider answers on the token endpoint: `invalid_grant`, 401, 429
    /// (+retry-after), 5xx, non-JSON, timeout, connection refused. No answer
    /// writes the file, no error carries a token, and an expired token is
    /// never sent to the usage API.
    #[test]
    fn claude_refresh_failures_are_classified_and_leak_nothing() {
        let root = scratch("claude-errors");
        let dir = claude_profile(
            &root,
            "work",
            &claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW - 10),
        );
        let before = fs::read(dir.join(crate::aip::CLAUDE_PAYLOAD)).unwrap();
        let cases: Vec<(Reply, ReadError)> = vec![
            (
                Reply::json(400, r#"{"error":"invalid_grant","error_description":"x"}"#),
                ReadError::RefreshRejected,
            ),
            (
                Reply::json(400, r#"{"error":{"code":"refresh_token_reused"}}"#),
                ReadError::RefreshRejected,
            ),
            (
                Reply::json(400, r#"{"error":"invalid_request"}"#),
                ReadError::Http(400),
            ),
            (Reply::json(401, "{}"), ReadError::RefreshRejected),
            (Reply::json(403, "{}"), ReadError::RefreshRejected),
            (
                Reply::json(429, "{}").with_header("retry-after", "7"),
                ReadError::RateLimited(Some(7)),
            ),
            (Reply::json(503, "down"), ReadError::Http(503)),
            (Reply::json(200, "not json"), ReadError::Malformed),
            (Reply::hang(), ReadError::Timeout),
        ];
        for (reply, expected) in cases {
            let server = Server::start(vec![reply]);
            let err = read_claude_profile(&dir, &server.base, T, NOW).unwrap_err();
            assert_eq!(err, expected);
            assert!(!format!("{err:?}").contains("SYNTHETIC"));
            let requests = server.requests();
            assert_eq!(requests.len(), 1, "token endpoint only: {requests:?}");
            assert!(requests[0].starts_with("POST /v1/oauth/token "));
            assert_eq!(
                fs::read(dir.join(crate::aip::CLAUDE_PAYLOAD)).unwrap(),
                before,
                "{expected:?}"
            );
        }
        let server = Server::start(vec![]);
        let base = server.base.clone();
        drop(server);
        assert_eq!(
            read_claude_profile(&dir, &base, T, NOW).unwrap_err(),
            ReadError::Transport
        );
        assert_eq!(
            fresh_claude(&dir, "http://example.invalid:1", T, NOW, None).unwrap_err(),
            ReadError::InsecureBase
        );
        // No refresh token: the dead snapshot is reported, nothing sent.
        let none = claude_profile(&root, "none", &claude_doc(OLD_ACCESS, None, NOW - 10));
        let server = Server::start(vec![Reply::json(200, USAGE_BODY)]);
        assert_eq!(
            read_claude_profile(&none, &server.base, T, NOW).unwrap_err(),
            ReadError::NoRefreshToken
        );
        assert!(server.requests().is_empty());
        // …but a still-valid token without refresh token is read as before.
        let valid = claude_profile(&root, "valid", &claude_doc(OLD_ACCESS, None, NOW + 200));
        read_claude_profile(&valid, &server.base, T, NOW).unwrap();
        assert_eq!(server.requests().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    /// 401 on a token the file says is valid: one bounded renewal, then the
    /// usage read again; a second 401 is final. When another process
    /// already replaced the token, that token is used without any POST.
    #[test]
    fn claude_rejected_token_is_renewed_once_or_replaced_by_the_file() {
        let root = scratch("claude-401");
        let dir = claude_profile(
            &root,
            "work",
            &claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW + 3600),
        );
        let server = Server::start(vec![
            Reply::json(401, "{}"),
            token_reply(NEW_ACCESS, Some(NEW_REFRESH), 3600),
            Reply::json(200, USAGE_BODY),
        ]);
        read_claude_profile(&dir, &server.base, T, NOW).unwrap();
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].starts_with("POST /v1/oauth/token "));
        assert!(requests[1].contains("refresh_reason") == false);
        assert_eq!(
            Server::header(&requests[2], "authorization").as_deref(),
            Some(&*format!("Bearer {NEW_ACCESS}"))
        );
        assert_eq!(
            claude_file(&dir)["claudeAiOauth"]["accessToken"],
            NEW_ACCESS
        );
        // Two 401 in a row: exactly one POST, then final.
        let server = Server::start(vec![
            Reply::json(401, "{}"),
            token_reply(
                "sk-ant-oat01-SYNTHETIC-3",
                Some("sk-ant-ort01-SYNTHETIC-3"),
                3600,
            ),
            Reply::json(429, "{}").with_header("retry-after", "0"),
        ]);
        assert_eq!(
            read_claude_profile(&dir, &server.base, T, NOW).unwrap_err(),
            ReadError::Unauthorized
        );
        assert_eq!(server.requests().len(), 3);
        // The file was rotated by someone else between the read and the
        // 401 (simulated: rewrite before the retry): no POST, new token used.
        let server = Server::start(vec![Reply::json(200, USAGE_BODY)]);
        fs::write(
            dir.join(crate::aip::CLAUDE_PAYLOAD),
            claude_doc(
                "sk-ant-oat01-SYNTHETIC-other",
                Some("sk-ant-ort01-SYNTHETIC-other"),
                NOW + 3600,
            )
            .to_string(),
        )
        .unwrap();
        let c = fresh_claude(&dir, &server.base, T, NOW, Some("sk-ant-oat01-SYNTHETIC-3")).unwrap();
        assert!(c.carries("sk-ant-oat01-SYNTHETIC-other"));
        assert!(server.requests().is_empty());
        // A token rotated in this very cycle and still refused: final, no
        // second rotation (exactly one POST per cycle).
        fs::write(
            dir.join(crate::aip::CLAUDE_PAYLOAD),
            claude_doc(OLD_ACCESS, Some("sk-ant-ort01-SYNTHETIC-cycle"), NOW - 10).to_string(),
        )
        .unwrap();
        let server = Server::start(vec![
            token_reply(
                "sk-ant-oat01-SYNTHETIC-c2",
                Some("sk-ant-ort01-SYNTHETIC-c2"),
                3600,
            ),
            Reply::json(401, "{}"),
        ]);
        assert_eq!(
            read_claude_profile(&dir, &server.base, T, NOW).unwrap_err(),
            ReadError::Unauthorized
        );
        assert_eq!(server.requests().len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    /// A grant shared with another snapshot (same refresh token in a sibling
    /// profile, e.g. saved twice or the `_current` copy of the keychain) is
    /// never rotated: read while valid, reported once dead.
    #[test]
    fn claude_shared_grant_is_never_rotated() {
        let root = scratch("claude-shared");
        let dormant = claude_profile(
            &root,
            "dormant",
            &claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW + 100),
        );
        claude_profile(
            &root,
            "current",
            &claude_doc("sk-ant-oat01-SYNTHETIC-cur", Some(OLD_REFRESH), NOW + 7200),
        );
        let server = Server::start(vec![Reply::json(200, USAGE_BODY)]);
        read_claude_profile(&dormant, &server.base, T, NOW).unwrap();
        assert_eq!(server.requests().len(), 1, "read with the token it has");
        assert!(server.requests()[0].starts_with("GET "));
        assert_eq!(
            read_claude_profile(&dormant, &server.base, T, NOW + 200).unwrap_err(),
            ReadError::SharedGrant
        );
        assert_eq!(server.requests().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    /// Two collectors in this process on the same expired snapshot: one
    /// rotation, both end with the new token.
    #[test]
    fn claude_concurrent_renewals_rotate_once() {
        let root = scratch("claude-concurrent");
        let dir = claude_profile(
            &root,
            "work",
            &claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW - 10),
        );
        let server = Server::start(vec![token_reply(NEW_ACCESS, Some(NEW_REFRESH), 3600)]);
        let results: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let (dir, base) = (dir.clone(), server.base.clone());
                    s.spawn(move || fresh_claude(&dir, &base, T, NOW, None))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for r in &results {
            assert!(r.as_ref().unwrap().carries(NEW_ACCESS));
        }
        assert_eq!(server.requests().len(), 1, "one refresh for two collectors");
        fs::remove_dir_all(root).unwrap();
    }

    /// Save failures after a successful rotation: the old file is intact
    /// (temporary removed), the error is explicit, the consumed refresh token
    /// is never sent again, and the save is retried on the next cycle.
    #[cfg(unix)]
    #[test]
    fn claude_unsaved_rotation_is_kept_in_memory_and_saved_later() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("claude-unsaved");
        let dir = claude_profile(
            &root,
            "work",
            &claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW - 10),
        );
        let _lock_file = fs::File::create(dir.join("credentials.json.lock")).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        let server = Server::start(vec![token_reply(NEW_ACCESS, Some(NEW_REFRESH), 3600)]);
        let err = read_claude_profile(&dir, &server.base, T, NOW).unwrap_err();
        assert_eq!(err, ReadError::RefreshUnsaved);
        assert!(err.transient());
        assert_eq!(server.requests().len(), 1);
        assert_eq!(
            claude_file(&dir)["claudeAiOauth"]["accessToken"],
            OLD_ACCESS,
            "old file intact"
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2, "no temporary left");
        // Next cycle while still read-only: no POST (the consumed grant is
        // never replayed), same honest state.
        assert_eq!(
            read_claude_profile(&dir, &server.base, T, NOW + 120).unwrap_err(),
            ReadError::RefreshUnsaved
        );
        assert_eq!(server.requests().len(), 1);
        // Folder writable again: the pending document is saved, no POST.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let server = Server::start(vec![Reply::json(200, USAGE_BODY)]);
        read_claude_profile(&dir, &server.base, T, NOW + 240).unwrap();
        assert_eq!(server.requests().len(), 1);
        assert!(server.requests()[0].starts_with("GET "));
        let saved = claude_file(&dir);
        assert_eq!(saved["claudeAiOauth"]["accessToken"], NEW_ACCESS);
        assert_eq!(saved["claudeAiOauth"]["refreshToken"], NEW_REFRESH);
        assert_eq!(mode(&dir.join(crate::aip::CLAUDE_PAYLOAD)), 0o600);
        fs::remove_dir_all(root).unwrap();
    }

    /// Unsafe layouts are never written through: a symlinked
    /// `credentials.json` or a non-regular lock path refuse the renewal (the
    /// valid token is still read).
    #[cfg(unix)]
    #[test]
    fn claude_symlinked_credentials_are_not_rewritten() {
        let root = scratch("claude-symlink");
        // The link target lives outside the profiles folder (no sibling
        // shares the grant): only the layout refuses the rewrite.
        let real = root.join("elsewhere");
        fs::create_dir_all(&real).unwrap();
        fs::write(
            real.join(crate::aip::CLAUDE_PAYLOAD),
            claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW - 10).to_string(),
        )
        .unwrap();
        let linked = root.join("profiles").join("linked");
        fs::create_dir_all(&linked).unwrap();
        std::os::unix::fs::symlink(
            real.join(crate::aip::CLAUDE_PAYLOAD),
            linked.join(crate::aip::CLAUDE_PAYLOAD),
        )
        .unwrap();
        let server = Server::start(vec![Reply::json(200, USAGE_BODY)]);
        assert_eq!(
            fresh_claude(&linked, &server.base, T, NOW, None).unwrap_err(),
            ReadError::RefreshUnavailable
        );
        assert_eq!(
            fresh_claude(&linked, &server.base, T, NOW - 7200, None)
                .unwrap()
                .token(),
            OLD_ACCESS
        );
        let bad_lock = claude_profile(
            &root,
            "badlock",
            &claude_doc(OLD_ACCESS, Some("sk-ant-ort01-SYNTHETIC-badlock"), NOW - 10),
        );
        fs::create_dir_all(bad_lock.join("credentials.json.lock")).unwrap();
        assert_eq!(
            fresh_claude(&bad_lock, &server.base, T, NOW, None).unwrap_err(),
            ReadError::RefreshUnavailable
        );
        assert!(server.requests().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    // ------------------------------------------------ cross-process lock

    /// Helper run in ANOTHER PROCESS (this test binary re-executed with
    /// `--exact`): takes the same `flock` aip takes on the given lock path,
    /// signals it with a `.held` file, optionally rewrites the credential
    /// file, holds for the given time, exits (a killed holder releases the
    /// lock through the kernel). Does nothing when the variables are absent.
    #[test]
    fn lock_holder_helper() {
        let Some(lock) = std::env::var_os("QM_LOCK_HOLDER_LOCK") else {
            return;
        };
        let lock = PathBuf::from(lock);
        let hold = std::env::var("QM_LOCK_HOLDER_HOLD_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000u64);
        let guard = acquire_lock(&lock, Duration::from_secs(5)).expect("helper takes the lock");
        fs::write(lock.with_extension("held"), b"1").unwrap();
        // Rotate half-way through the hold: the waiter's fast path saw the
        // stale file, its re-read under the lock must see this one.
        std::thread::sleep(Duration::from_millis(hold / 2));
        if let (Some(file), Some(content)) = (
            std::env::var_os("QM_LOCK_HOLDER_WRITE_FILE"),
            std::env::var("QM_LOCK_HOLDER_WRITE_CONTENT").ok(),
        ) {
            fs::write(file, content).unwrap();
        }
        std::thread::sleep(Duration::from_millis(hold / 2));
        drop(guard);
    }
    fn spawn_holder(
        lock: &Path,
        hold_ms: u64,
        write: Option<(&Path, String)>,
    ) -> std::process::Child {
        let _ = fs::remove_file(lock.with_extension("held"));
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "oauth_refresh::tests::lock_holder_helper",
            "--test-threads",
            "1",
        ])
        .env("QM_LOCK_HOLDER_LOCK", lock)
        .env("QM_LOCK_HOLDER_HOLD_MS", hold_ms.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
        if let Some((file, content)) = write {
            cmd.env("QM_LOCK_HOLDER_WRITE_FILE", file)
                .env("QM_LOCK_HOLDER_WRITE_CONTENT", content);
        }
        let child = cmd.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !lock.with_extension("held").exists() {
            assert!(Instant::now() < deadline, "helper never took the lock");
            std::thread::sleep(Duration::from_millis(20));
        }
        child
    }

    /// Simulation of aip's `load_fresh_credentials` in another process (same
    /// lock file, same `flock(LOCK_EX|LOCK_NB)` as fd-lock's `try_write`):
    /// contention waits then times out (`RefreshLocked`, nothing sent, file
    /// intact); a holder that rotates the file before releasing makes the
    /// re-read under the lock adopt its token without any POST; a holder that
    /// crashes releases the lock.
    #[cfg(unix)]
    #[test]
    fn cross_process_lock_is_aip_compatible_simulation() {
        let root = scratch("xproc");
        let dir = claude_profile(
            &root,
            "work",
            &claude_doc(OLD_ACCESS, Some(OLD_REFRESH), NOW - 10),
        );
        let lock = dir.join("credentials.json.lock");
        // 1. contention: bounded wait, then RefreshLocked.
        let mut holder = spawn_holder(&lock, 1500, None);
        let t0 = Instant::now();
        assert_eq!(
            acquire_lock(&lock, Duration::from_millis(300)).err(),
            Some(ReadError::RefreshLocked)
        );
        assert!(
            t0.elapsed() >= Duration::from_millis(300)
                && t0.elapsed() < Duration::from_millis(1400)
        );
        holder.wait().unwrap();
        acquire_lock(&lock, Duration::from_millis(300)).expect("released by the holder's exit");
        // 2. the other process rotates under the lock: adopted, no POST.
        let rotated = claude_doc(
            "sk-ant-oat01-SYNTHETIC-aip",
            Some("sk-ant-ort01-SYNTHETIC-aip"),
            NOW + 3600,
        );
        let mut holder = spawn_holder(
            &lock,
            700,
            Some((&dir.join(crate::aip::CLAUDE_PAYLOAD), rotated.to_string())),
        );
        let server = Server::start(vec![Reply::json(200, USAGE_BODY)]);
        let t0 = Instant::now();
        let c = fresh_claude(&dir, &server.base, T, NOW, None).unwrap();
        assert!(
            t0.elapsed() >= Duration::from_millis(300),
            "waited for the holder"
        );
        assert!(c.carries("sk-ant-oat01-SYNTHETIC-aip"));
        assert!(
            server.requests().is_empty(),
            "no refresh: the other process did it"
        );
        holder.wait().unwrap();
        // 3. crash while holding: the kernel releases the lock.
        let mut holder = spawn_holder(&lock, 60_000, None);
        holder.kill().unwrap();
        holder.wait().unwrap();
        let t0 = Instant::now();
        acquire_lock(&lock, Duration::from_secs(5)).expect("lock released by the kernel");
        assert!(t0.elapsed() < Duration::from_secs(2));
        fs::remove_dir_all(root).unwrap();
    }

    // ------------------------------------------------------------ Codex

    const CODEX_REFRESH: &str = "SYNTHETIC-CODEX-REFRESH-old";
    fn codex_doc(exp: i64, refresh: Option<&str>) -> Value {
        let access = jwt(
            &json!({"exp": exp, "https://api.openai.com/auth": {"chatgpt_account_id": "acct-1", "chatgpt_plan_type": "plus"}}),
        );
        let id = jwt(
            &json!({"email": "someone@example.invalid", "https://api.openai.com/auth": {"chatgpt_account_id": "acct-1"}}),
        );
        let mut tokens = json!({"id_token": id, "access_token": access, "account_id": "acct-1", "login_owner_id": "owner-1"});
        if let Some(r) = refresh {
            tokens["refresh_token"] = Value::String(r.into());
        }
        json!({"OPENAI_API_KEY": null, "auth_mode": "chatgpt", "tokens": tokens, "last_refresh": "2026-01-01T00:00:00Z", "future": [1]})
    }
    fn codex_home(root: &Path, name: &str, doc: &Value, marker: Option<&str>) -> PathBuf {
        let dir = root.join("profiles").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(crate::aip::CODEX_PAYLOAD), doc.to_string()).unwrap();
        if let Some(m) = marker {
            fs::write(dir.join(CODEX_IMPORT_MARKER), m).unwrap();
        }
        dir
    }
    fn codex_file(dir: &Path) -> Value {
        serde_json::from_slice(&fs::read(dir.join(crate::aip::CODEX_PAYLOAD)).unwrap()).unwrap()
    }
    const CODEX_USAGE_BODY: &str = r#"{"plan_type":"plus","rate_limit":{"primary_window":{"used_percent":40,"limit_window_seconds":18000,"reset_at":1800003600},"secondary_window":null}}"#;

    /// Independent grant (marker lists an older import only): JSON POST with
    /// the CLI's client id, tokens + `last_refresh` persisted (other fields
    /// kept, 0600), usage read with the new bearer, later cycle left alone.
    #[test]
    fn codex_independent_grant_is_renewed_with_the_cli_protocol() {
        let root = scratch("codex-cycle");
        let marker = format!("{}\n", fingerprint("SYNTHETIC-CODEX-REFRESH-imported"));
        let home = codex_home(
            &root,
            "work",
            &codex_doc(NOW + 500, Some(CODEX_REFRESH)),
            Some(&marker),
        );
        let new_access = jwt(
            &json!({"exp": NOW + 3600, "https://api.openai.com/auth": {"chatgpt_account_id": "acct-1"}}),
        );
        let new_id = jwt(&json!({"email": "someone@example.invalid"}));
        let server = Server::start(vec![
            Reply::json(200, &json!({"id_token": new_id, "access_token": new_access, "refresh_token": "SYNTHETIC-CODEX-REFRESH-new"}).to_string()),
            Reply::json(200, CODEX_USAGE_BODY),
            Reply::json(200, CODEX_USAGE_BODY),
        ]);
        let usage = read_codex_home(&home, &server.base, T, NOW, 600, None).unwrap();
        assert_eq!(usage.plan_type.as_deref(), Some("plus"));
        let requests = server.requests();
        assert_eq!(requests.len(), 2, "{requests:?}");
        assert!(requests[0].starts_with("POST /oauth/token HTTP/1.1"));
        assert_eq!(
            Server::header(&requests[0], "content-type").as_deref(),
            Some("application/json")
        );
        let body: Value =
            serde_json::from_str(requests[0].rsplit("\r\n\r\n").next().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"client_id": CODEX_CLIENT_ID, "grant_type": "refresh_token", "refresh_token": CODEX_REFRESH, "refresh_reason": "proactive_expiry"})
        );
        assert_eq!(
            Server::header(&requests[1], "authorization").as_deref(),
            Some(&*format!("Bearer {new_access}"))
        );
        assert_eq!(
            Server::header(&requests[1], "chatgpt-account-id").as_deref(),
            Some("acct-1")
        );
        let saved = codex_file(&home);
        assert_eq!(saved["tokens"]["access_token"], new_access);
        assert_eq!(saved["tokens"]["id_token"], new_id);
        assert_eq!(
            saved["tokens"]["refresh_token"],
            "SYNTHETIC-CODEX-REFRESH-new"
        );
        assert_eq!(saved["tokens"]["account_id"], "acct-1");
        assert_eq!(saved["tokens"]["login_owner_id"], "owner-1");
        assert_eq!(saved["auth_mode"], "chatgpt");
        assert_eq!(saved["future"], json!([1]));
        assert_eq!(saved["last_refresh"], "2026-05-28T20:26:40.000000Z");
        #[cfg(unix)]
        assert_eq!(mode(&home.join(crate::aip::CODEX_PAYLOAD)), 0o600);
        assert_eq!(
            fs::read_to_string(home.join(CODEX_IMPORT_MARKER)).unwrap(),
            marker,
            "marker untouched"
        );
        read_codex_home(&home, &server.base, T, NOW + 60, 600, None).unwrap();
        assert_eq!(server.requests().len(), 3);
        assert!(server.requests()[2].starts_with("GET /wham/usage "));
        fs::remove_dir_all(root).unwrap();
    }

    /// aip's protections: no marker (legacy snapshot) or a listed fingerprint
    /// = grant still shared with the source session → never rotated; the
    /// same for a grant equal to a sibling's or the primary home's. Valid
    /// tokens are still read; dead ones are reported as shared.
    #[test]
    fn codex_imported_or_shared_grants_are_never_rotated() {
        let root = scratch("codex-shared");
        let legacy = codex_home(
            &root,
            "legacy",
            &codex_doc(NOW + 100, Some(CODEX_REFRESH)),
            None,
        );
        let imported = codex_home(
            &root,
            "imported",
            &codex_doc(NOW + 100, Some("SYNTHETIC-CODEX-REFRESH-imp")),
            Some(&format!(
                "old\n{}\n",
                fingerprint("SYNTHETIC-CODEX-REFRESH-imp")
            )),
        );
        let server = Server::start(vec![
            Reply::json(200, CODEX_USAGE_BODY),
            Reply::json(200, CODEX_USAGE_BODY),
        ]);
        read_codex_home(&legacy, &server.base, T, NOW, 600, None).unwrap();
        read_codex_home(&imported, &server.base, T, NOW, 600, None).unwrap();
        assert!(server.requests().iter().all(|r| r.starts_with("GET ")));
        assert_eq!(
            read_codex_home(&legacy, &server.base, T, NOW + 200, 600, None).unwrap_err(),
            ReadError::SharedGrant
        );
        assert_eq!(
            read_codex_home(&imported, &server.base, T, NOW + 200, 600, None).unwrap_err(),
            ReadError::SharedGrant
        );
        assert_eq!(server.requests().len(), 2);
        // Independent per its marker, but the primary home holds the same grant.
        let primary = root.join("primary-home");
        fs::create_dir_all(&primary).unwrap();
        fs::write(
            primary.join(crate::aip::CODEX_PAYLOAD),
            codex_doc(NOW + 9000, Some(CODEX_REFRESH)).to_string(),
        )
        .unwrap();
        let twin = codex_home(
            &root,
            "twin",
            &codex_doc(NOW - 5, Some(CODEX_REFRESH)),
            Some("unrelated\n"),
        );
        assert_eq!(
            read_codex_home(&twin, &server.base, T, NOW, 600, Some(&primary)).unwrap_err(),
            ReadError::SharedGrant
        );
        // Same grant in a sibling profile (legacy above holds CODEX_REFRESH too).
        assert_eq!(
            read_codex_home(&twin, &server.base, T, NOW, 600, None).unwrap_err(),
            ReadError::SharedGrant
        );
        // No refresh token at all.
        let bare = codex_home(&root, "bare", &codex_doc(NOW - 5, None), Some("x\n"));
        assert_eq!(
            read_codex_home(&bare, &server.base, T, NOW, 600, None).unwrap_err(),
            ReadError::NoRefreshToken
        );
        assert_eq!(server.requests().len(), 2, "nothing else sent");
        for home in [&legacy, &imported, &twin, &bare] {
            assert_eq!(
                codex_file(home)["tokens"]["access_token"]
                    .as_str()
                    .map(str::len)
                    .unwrap_or(0)
                    > 0,
                true
            );
            assert!(
                !home.join("auth.json.lock").exists(),
                "no lock file created without a rotation"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// Codex failure paths on the token endpoint (the CLI's codes), the
    /// bounded 401 retry, the margin rule and an opaque token.
    #[test]
    fn codex_refresh_failures_margin_and_rejected_token() {
        let root = scratch("codex-errors");
        let home = codex_home(
            &root,
            "work",
            &codex_doc(NOW - 5, Some(CODEX_REFRESH)),
            Some("other\n"),
        );
        let before = fs::read(home.join(crate::aip::CODEX_PAYLOAD)).unwrap();
        for (reply, expected) in [
            (
                Reply::json(400, r#"{"error":{"code":"refresh_token_expired"}}"#),
                ReadError::RefreshRejected,
            ),
            (
                Reply::json(400, r#"{"error":{"code":"refresh_token_invalidated"}}"#),
                ReadError::RefreshRejected,
            ),
            (
                Reply::json(401, r#"{"error":{"code":"refresh_token_reused"}}"#),
                ReadError::RefreshRejected,
            ),
            (Reply::json(429, "{}"), ReadError::RateLimited(None)),
            (Reply::json(500, "{}"), ReadError::Http(500)),
            (
                Reply::json(200, r#"{"id_token":"x"}"#),
                ReadError::Malformed,
            ),
        ] {
            let server = Server::start(vec![reply]);
            assert_eq!(
                read_codex_home(&home, &server.base, T, NOW, 600, None).unwrap_err(),
                expected
            );
            assert_eq!(server.requests().len(), 1);
            assert_eq!(
                fs::read(home.join(crate::aip::CODEX_PAYLOAD)).unwrap(),
                before
            );
        }
        // Margin: exp - margin <= now; the floor is 600 s whatever the caller asks.
        let c = oauth_usage::load_codex(&codex_home(
            &root,
            "m",
            &codex_doc(NOW + 700, Some("r")),
            Some("x\n"),
        ))
        .unwrap();
        assert!(!codex_needs_refresh(&c, NOW, 600));
        assert!(codex_needs_refresh(&c, NOW, 700));
        let server = Server::start(vec![Reply::json(200, CODEX_USAGE_BODY)]);
        let fresh = codex_home(
            &root,
            "fresh",
            &codex_doc(NOW + 700, Some("r")),
            Some("x\n"),
        );
        read_codex_home(&fresh, &server.base, T, NOW, 10, None).unwrap();
        assert!(
            server.requests()[0].starts_with("GET "),
            "margin floor applied: no refresh at 700 s"
        );
        // 401 on a valid-looking token: one renewal (`unauthorized_recovery`), then read.
        let new_access = jwt(&json!({"exp": NOW + 3600}));
        let server = Server::start(vec![
            Reply::json(401, "{}"),
            Reply::json(200, &json!({"access_token": new_access}).to_string()),
            Reply::json(200, CODEX_USAGE_BODY),
        ]);
        let valid = codex_home(
            &root,
            "valid",
            &codex_doc(NOW + 9000, Some("SYNTHETIC-CODEX-REFRESH-v")),
            Some("x\n"),
        );
        read_codex_home(&valid, &server.base, T, NOW, 600, None).unwrap();
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].contains("unauthorized_recovery"));
        let saved = codex_file(&valid);
        assert_eq!(saved["tokens"]["access_token"], new_access);
        assert_eq!(
            saved["tokens"]["refresh_token"], "SYNTHETIC-CODEX-REFRESH-v",
            "kept when not rotated"
        );
        // Opaque token (no exp): read as is, renewed only on 401.
        let opaque = root.join("profiles").join("opaque");
        fs::create_dir_all(&opaque).unwrap();
        fs::write(opaque.join(crate::aip::CODEX_PAYLOAD), r#"{"tokens":{"access_token":"opaque-synthetic","refresh_token":"r2","account_id":"a"}}"#).unwrap();
        fs::write(opaque.join(CODEX_IMPORT_MARKER), "x\n").unwrap();
        let server = Server::start(vec![Reply::json(200, CODEX_USAGE_BODY)]);
        read_codex_home(&opaque, &server.base, T, NOW, 600, None).unwrap();
        assert!(server.requests()[0].starts_with("GET "));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn form_encoding_and_fingerprint_match_the_references() {
        assert_eq!(
            form_encode(&[("a b", "c&d=e".into()), ("scope", "user:x y".into())]),
            "a+b=c%26d%3De&scope=user%3Ax+y"
        );
        // echo -n abc | shasum -a 256
        assert_eq!(
            fingerprint("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            claude_token_base(oauth_usage::CLAUDE_USAGE_BASE),
            CLAUDE_TOKEN_BASE
        );
        assert_eq!(
            codex_token_base(oauth_usage::CODEX_USAGE_BASE),
            CODEX_TOKEN_BASE
        );
        assert_eq!(
            claude_token_base("http://127.0.0.1:1"),
            "http://127.0.0.1:1"
        );
    }
}
