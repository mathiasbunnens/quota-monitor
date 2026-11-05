//! Read-only HTTPS quota reads for `aip` profiles, authenticated with the
//! credential the profile already holds. The access token is loaded in memory
//! for one request and dropped; nothing here refreshes, rotates, writes back,
//! logs or returns a secret, and no CLI is launched.
//!
//! Sources (primary, checked against the installed CLI versions):
//! - Claude Code 2.1.299 (bundled gateway contract and OAuth constants in the
//!   binary): `GET {BASE_API_URL}/api/oauth/usage` with `Authorization: Bearer
//!   <accessToken>` is "the path claude.ai serves Claude Code's usage data on";
//!   `BASE_API_URL = https://api.anthropic.com`; beta header
//!   `oauth-2025-04-20`; the body carries `five_hour` / `seven_day`
//!   (`utilization`, `resets_at`), `extra_usage` and `limits[]` whose
//!   `kind == "weekly_scoped"` rows name a model (`scope.model.display_name`,
//!   e.g. « Fable ») — the CLI projects those into `get_usage.model_scoped`.
//!   aip's `credentials.json` is the keychain item (`Claude Code-credentials`)
//!   verbatim: `{"claudeAiOauth": {"accessToken", "refreshToken", "expiresAt",
//!   "scopes", "subscriptionType", …}}` (dqn/aip 3961fa0, `src/claude/profile.rs`).
//! - Codex CLI 0.164.0 (`codex-rs/backend-client/src/client/rate_limit_resets.rs`,
//!   `model-provider/src/bearer_auth_provider.rs`, `login/src/auth/storage.rs`):
//!   `GET https://chatgpt.com/backend-api/wham/usage` with `Authorization:
//!   Bearer <tokens.access_token>` and `ChatGPT-Account-ID: <tokens.account_id>`;
//!   the body is `RateLimitStatusPayload` (`plan_type`, `rate_limit.{primary,
//!   secondary}_window.{used_percent, limit_window_seconds, reset_at}`,
//!   `additional_rate_limits[]`, `rate_limit_reset_credits.available_count`).
//!   Token refresh is a separate `POST …/oauth/token` the CLI's AuthManager
//!   performs with the refresh token; a bearer read never triggers it, so the
//!   grant shared with the primary session cannot be rotated from here.
//! - aip itself (3961fa0, `src/claude/usage.rs`, `src/codex/usage.rs`) reads
//!   both endpoints the same way for its dashboard: Codex tokens as they are
//!   (« Codex owns refresh »), the current Claude profile's token as it is,
//!   and it notes that an expired Claude token may answer 429 with
//!   `retry-after: 0`. Unlike aip, this app never refreshes a dormant Claude
//!   snapshot: a refresh spends the single-use refresh token and must be
//!   written back under aip's `credentials.json.lock`; see docs/aip-profiles.md.
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// Anthropic OAuth API base (Claude Code `BASE_API_URL`).
pub const CLAUDE_USAGE_BASE: &str = "https://api.anthropic.com";
/// ChatGPT backend base (Codex `chatgpt_base_url` normalised to `/backend-api`).
pub const CODEX_USAGE_BASE: &str = "https://chatgpt.com/backend-api";
pub const CLAUDE_USAGE_PATH: &str = "/api/oauth/usage";
pub const CODEX_USAGE_PATH: &str = "/wham/usage";
/// Claude Code's OAuth beta header value (`Tp` in the 2.1.299 bundle).
const ANTHROPIC_BETA: &str = "oauth-2025-04-20";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Largest credential file still parsed (a keychain item is a few KiB).
const MAX_CREDENTIAL_BYTES: u64 = 64 * 1024;
/// Largest usage body still parsed.
const MAX_BODY_BYTES: usize = 256 * 1024;
/// Margin before the recorded expiry at which a token is already treated as
/// expired (a doomed request is not sent).
const EXPIRY_SKEW_SECS: i64 = 60;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Why a credential read failed. Never carries a token, a path or a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// No credential file, unreadable, oversized, not JSON or unknown shape.
    CredentialUnreadable,
    /// The credential holds no OAuth access token (API-key home, signed-out
    /// snapshot).
    NoToken,
    /// The credential's own expiry (`expiresAt` / JWT `exp`) has passed; the
    /// provider is not even asked.
    Expired,
    /// HTTP 401: token rejected (expired, revoked, replaced).
    Unauthorized,
    /// HTTP 403: token accepted but not allowed to read usage.
    Forbidden,
    /// HTTP 429 (`retry-after` seconds when the provider gave one). A 429
    /// with `retry-after: 0` is an expired token (aip's observation) and is
    /// reported as `Unauthorized`.
    RateLimited(Option<u64>),
    /// Request or connect timeout.
    Timeout,
    /// Connection/TLS/transport failure.
    Transport,
    /// Any other HTTP status.
    Http(u16),
    /// 2xx body that is not a JSON object.
    Malformed,
    /// Refused base URL (only HTTPS or loopback HTTP may carry a bearer).
    InsecureBase,
}

/// One Claude OAuth credential snapshot, as saved by aip (keychain item).
pub struct ClaudeCredential {
    token: String,
    /// Unix seconds.
    expires_at: Option<i64>,
    subscription_type: Option<String>,
}
impl std::fmt::Debug for ClaudeCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCredential")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("subscription_type", &self.subscription_type)
            .finish()
    }
}
impl ClaudeCredential {
    pub fn subscription_type(&self) -> Option<&str> {
        self.subscription_type.as_deref()
    }
    pub fn expired(&self, now: i64) -> bool {
        self.expires_at
            .is_some_and(|e| e.saturating_sub(EXPIRY_SKEW_SECS) <= now)
    }
}

/// One Codex `auth.json` (ChatGPT tokens), as copied/re-logged by aip.
pub struct CodexCredential {
    token: String,
    account_id: Option<String>,
    email: Option<String>,
    /// Unix seconds (JWT `exp` of the access token).
    expires_at: Option<i64>,
    plan_type: Option<String>,
}
impl std::fmt::Debug for CodexCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexCredential")
            .field("token", &"<redacted>")
            .field("account_id", &self.account_id.as_ref().map(|_| "<set>"))
            .field("email", &self.email.as_ref().map(|_| "<set>"))
            .field("expires_at", &self.expires_at)
            .field("plan_type", &self.plan_type)
            .finish()
    }
}
impl CodexCredential {
    pub fn expired(&self, now: i64) -> bool {
        self.expires_at
            .is_some_and(|e| e.saturating_sub(EXPIRY_SKEW_SECS) <= now)
    }
}

// ------------------------------------------------------------ credentials

fn read_bounded(path: &Path) -> Result<Vec<u8>, ReadError> {
    let meta = std::fs::metadata(path).map_err(|_| ReadError::CredentialUnreadable)?;
    if !meta.is_file() || meta.len() > MAX_CREDENTIAL_BYTES {
        return Err(ReadError::CredentialUnreadable);
    }
    std::fs::read(path).map_err(|_| ReadError::CredentialUnreadable)
}

/// aip's `decode_hex_credentials`: `security -w` may return the keychain blob
/// hex-encoded; the JSON is used as is when it already starts with `{`.
fn decode_hex_json(raw: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(raw).ok()?.trim();
    if text.starts_with('{') {
        return Some(text.as_bytes().to_vec());
    }
    if text.len() % 2 != 0 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect();
    bytes.first().is_some_and(|b| *b == b'{').then_some(bytes)
}

fn non_empty(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
        .map(str::to_string)
}

/// Seconds from a JSON timestamp given in seconds or milliseconds.
fn unix_seconds(value: &Value) -> Option<i64> {
    let n = value.as_f64()?;
    if !n.is_finite() || n <= 0.0 {
        return None;
    }
    Some(if n > 1.0e11 {
        (n / 1000.0) as i64
    } else {
        n as i64
    })
}

/// Loads `<profile>/credentials.json` (aip Claude snapshot) in memory.
pub fn load_claude(profile_dir: &Path) -> Result<ClaudeCredential, ReadError> {
    let raw = read_bounded(&profile_dir.join(crate::aip::CLAUDE_PAYLOAD))?;
    let json = decode_hex_json(&raw).ok_or(ReadError::CredentialUnreadable)?;
    let value: Value =
        serde_json::from_slice(&json).map_err(|_| ReadError::CredentialUnreadable)?;
    let oauth = value
        .get("claudeAiOauth")
        .filter(|v| v.is_object())
        .ok_or(ReadError::CredentialUnreadable)?;
    let token = non_empty(&oauth["accessToken"]).ok_or(ReadError::NoToken)?;
    Ok(ClaudeCredential {
        token,
        expires_at: unix_seconds(&oauth["expiresAt"]),
        subscription_type: crate::claude::plan_token(&oauth["subscriptionType"])
            .or_else(|| crate::claude::plan_token(&oauth["planType"])),
    })
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Unverified JWT payload (claims are informational: expiry, identity hints).
fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&b64url_decode(payload)?).ok()
}

/// Loads `<home>/auth.json` (Codex home) in memory. An API-key-only home has
/// no ChatGPT quotas (`NoToken`).
pub fn load_codex(home: &Path) -> Result<CodexCredential, ReadError> {
    let raw = read_bounded(&home.join(crate::aip::CODEX_PAYLOAD))?;
    let value: Value = serde_json::from_slice(&raw).map_err(|_| ReadError::CredentialUnreadable)?;
    if !value.is_object() {
        return Err(ReadError::CredentialUnreadable);
    }
    let tokens = &value["tokens"];
    let token = non_empty(&tokens["access_token"]).ok_or(ReadError::NoToken)?;
    let access = jwt_claims(&token).unwrap_or(Value::Null);
    let id = non_empty(&tokens["id_token"])
        .and_then(|t| jwt_claims(&t))
        .unwrap_or(Value::Null);
    let auth_claims = |claims: &Value| claims["https://api.openai.com/auth"].clone();
    let account_id = non_empty(&tokens["account_id"])
        .or_else(|| non_empty(&auth_claims(&id)["chatgpt_account_id"]))
        .or_else(|| non_empty(&auth_claims(&access)["chatgpt_account_id"]));
    Ok(CodexCredential {
        token,
        account_id,
        email: non_empty(&id["email"]).map(|e| e.to_lowercase()),
        expires_at: unix_seconds(&access["exp"]),
        plan_type: crate::claude::plan_token(&auth_claims(&id)["chatgpt_plan_type"])
            .or_else(|| crate::claude::plan_token(&auth_claims(&access)["chatgpt_plan_type"])),
    })
}

// ------------------------------------------------------------------- HTTP

/// A bearer may only travel over HTTPS, or over plain HTTP to the loopback
/// interface (local test servers). Profiles never choose the base.
fn checked_base(base: &str) -> Result<&str, ReadError> {
    let base = base.trim_end_matches('/');
    if base.starts_with("https://")
        || base.starts_with("http://127.0.0.1:")
        || base.starts_with("http://[::1]:")
    {
        Ok(base)
    } else {
        Err(ReadError::InsecureBase)
    }
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client, ReadError> {
    reqwest::blocking::Client::builder()
        .connect_timeout(timeout.min(Duration::from_secs(5)))
        .timeout(timeout)
        // A redirect would carry the Authorization header to another host.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("quota-monitor/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| ReadError::Transport)
}

fn send_json(request: reqwest::blocking::RequestBuilder) -> Result<Value, ReadError> {
    use std::io::Read;
    let response = request.send().map_err(|e| {
        if e.is_timeout() {
            ReadError::Timeout
        } else {
            ReadError::Transport
        }
    })?;
    let status = response.status().as_u16();
    match status {
        200..=299 => {}
        401 => return Err(ReadError::Unauthorized),
        403 => return Err(ReadError::Forbidden),
        429 => {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            return Err(match retry_after {
                Some(0) => ReadError::Unauthorized,
                other => ReadError::RateLimited(other),
            });
        }
        other => return Err(ReadError::Http(other)),
    }
    let mut body = Vec::new();
    response
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                ReadError::Timeout
            } else {
                ReadError::Transport
            }
        })?;
    if body.len() > MAX_BODY_BYTES {
        return Err(ReadError::Malformed);
    }
    serde_json::from_slice::<Value>(&body)
        .ok()
        .filter(Value::is_object)
        .ok_or(ReadError::Malformed)
}

/// `GET {base}/api/oauth/usage` for one Claude credential. The raw body is
/// returned (an object); see [`claude_usage_value`] for the `get_usage` shape.
pub fn fetch_claude(
    base: &str,
    credential: &ClaudeCredential,
    timeout: Duration,
    now: i64,
) -> Result<Value, ReadError> {
    let base = checked_base(base)?;
    if credential.expired(now) {
        return Err(ReadError::Expired);
    }
    let request = client(timeout)?
        .get(format!("{base}{CLAUDE_USAGE_PATH}"))
        .header("Authorization", format!("Bearer {}", credential.token))
        .header("anthropic-beta", ANTHROPIC_BETA)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("Accept", "application/json");
    send_json(request)
}

/// Projects the raw usage body into the exact shape Claude Code's `get_usage`
/// control response has (`rate_limits.{five_hour, seven_day, model_scoped,
/// extra_usage}`, `subscription_type`), so the existing parser is reused.
pub fn claude_usage_value(raw: &Value, subscription_type: Option<&str>) -> Value {
    let model_scoped: Vec<Value> = raw["limits"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|row| row["kind"].as_str() == Some("weekly_scoped"))
                .filter_map(|row| {
                    let name = row["scope"]["model"]["display_name"].as_str()?;
                    let percent = row["percent"]
                        .as_f64()
                        .or_else(|| row["utilization"].as_f64())?;
                    Some(json!({
                        "display_name": name,
                        "utilization": percent,
                        "resets_at": row["resets_at"].clone(),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "rate_limits_available": true,
        "subscription_type": subscription_type,
        "rate_limits": {
            "five_hour": raw["five_hour"].clone(),
            "seven_day": raw["seven_day"].clone(),
            "model_scoped": model_scoped,
            "extra_usage": raw["extra_usage"].clone(),
        }
    })
}

/// Loads the profile's credential and reads its quotas (one request).
pub fn read_claude_profile(
    profile_dir: &Path,
    base: &str,
    timeout: Duration,
    now: i64,
) -> Result<Value, ReadError> {
    let credential = load_claude(profile_dir)?;
    let raw = fetch_claude(base, &credential, timeout, now)?;
    Ok(claude_usage_value(&raw, credential.subscription_type()))
}

/// What one Codex credential read yields (no secret).
#[derive(Clone, Debug, PartialEq)]
pub struct CodexUsage {
    /// Shaped like app-server's `account/rateLimits/read` result
    /// (`rateLimitsByLimitId`, `rateLimitResetCredits`): `codex::normalize` input.
    pub rate_limits: Value,
    pub plan_type: Option<String>,
    /// Identity hints for deduplication (same scheme as `account/read`).
    pub email: Option<String>,
    pub account_id: Option<String>,
}

/// `GET {base}/wham/usage` for one Codex credential.
pub fn fetch_codex(
    base: &str,
    credential: &CodexCredential,
    timeout: Duration,
    now: i64,
) -> Result<Value, ReadError> {
    let base = checked_base(base)?;
    if credential.expired(now) {
        return Err(ReadError::Expired);
    }
    let mut request = client(timeout)?
        .get(format!("{base}{CODEX_USAGE_PATH}"))
        .header("Authorization", format!("Bearer {}", credential.token))
        .header("Accept", "application/json");
    if let Some(account) = &credential.account_id {
        request = request.header("ChatGPT-Account-Id", account);
    }
    send_json(request)
}

/// Projects `RateLimitStatusPayload` into app-server's camelCase shape.
pub fn codex_rate_limits_value(raw: &Value) -> Value {
    let window = |w: &Value| -> Value {
        if !w.is_object() {
            return Value::Null;
        }
        json!({
            "usedPercent": w["used_percent"].clone(),
            "windowDurationMins": w["limit_window_seconds"].as_i64().map(|s| s / 60),
            "resetsAt": w["reset_at"].clone(),
        })
    };
    let bucket = |id: &str, name: Option<&str>, limit: &Value| -> Value {
        json!({
            "limitId": id,
            "limitName": name,
            "primary": window(&limit["primary_window"]),
            "secondary": window(&limit["secondary_window"]),
        })
    };
    let mut by_id = serde_json::Map::new();
    by_id.insert("codex".into(), bucket("codex", None, &raw["rate_limit"]));
    if let Some(rows) = raw["additional_rate_limits"].as_array() {
        for row in rows {
            let Some(id) = row["metered_feature"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            by_id.insert(
                id.into(),
                bucket(id, row["limit_name"].as_str(), &row["rate_limit"]),
            );
        }
    }
    let mut out = json!({ "rateLimitsByLimitId": Value::Object(by_id) });
    if let Some(count) = raw["rate_limit_reset_credits"]["available_count"].as_u64() {
        out["rateLimitResetCredits"] = json!({ "availableCount": count });
    }
    out
}

/// Loads the home's credential and reads its quotas (one request).
pub fn read_codex_home(
    home: &Path,
    base: &str,
    timeout: Duration,
    now: i64,
) -> Result<CodexUsage, ReadError> {
    let credential = load_codex(home)?;
    let raw = fetch_codex(base, &credential, timeout, now)?;
    Ok(CodexUsage {
        rate_limits: codex_rate_limits_value(&raw),
        plan_type: crate::claude::plan_token(&raw["plan_type"]).or(credential.plan_type),
        email: credential.email,
        account_id: non_empty(&raw["account_id"]).or(credential.account_id),
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Minimal loopback HTTP server for synthetic fixtures (no dependency).
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    pub struct Reply {
        pub status: u16,
        pub body: String,
        pub headers: Vec<(String, String)>,
        /// Accept the connection, read the request, never answer.
        pub hang: bool,
    }
    impl Reply {
        pub fn json(status: u16, body: &str) -> Self {
            Self {
                status,
                body: body.into(),
                headers: vec![],
                hang: false,
            }
        }
        pub fn with_header(mut self, name: &str, value: &str) -> Self {
            self.headers.push((name.into(), value.into()));
            self
        }
        pub fn hang() -> Self {
            Self {
                status: 0,
                body: String::new(),
                headers: vec![],
                hang: true,
            }
        }
    }

    pub struct Server {
        pub base: String,
        /// Request heads (request line + headers) in arrival order.
        pub requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        /// Serves the replies in order, one connection each, then exits.
        pub fn start(replies: Vec<Reply>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(vec![]));
            let log = requests.clone();
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stopping = stop.clone();
            let handle = std::thread::spawn(move || {
                for reply in replies {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    if stopping.load(std::sync::atomic::Ordering::Acquire) {
                        return; // the drop's wake-up connection, not a request
                    }
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while stream.read(&mut byte).is_ok_and(|n| n == 1) {
                        head.push(byte[0]);
                        if head.ends_with(b"\r\n\r\n") {
                            break;
                        }
                    }
                    log.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&head).into_owned());
                    if reply.hang {
                        std::thread::sleep(Duration::from_secs(3));
                        continue;
                    }
                    let extra: String = reply
                        .headers
                        .iter()
                        .map(|(k, v)| format!("{k}: {v}\r\n"))
                        .collect();
                    let response = format!(
                        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                        reply.status,
                        reply.body.len(),
                        extra,
                        reply.body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
            });
            Self {
                base,
                requests,
                stop,
                handle: Some(handle),
            }
        }
        pub fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
        pub fn header(request: &str, name: &str) -> Option<String> {
            request.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim()
                    .eq_ignore_ascii_case(name)
                    .then(|| v.trim().to_string())
            })
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            // Unblock a pending accept so the thread can finish (unused
            // replies are normal: an expired token never asks the server).
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            let Some(h) = self.handle.take() else { return };
            let addr = self
                .base
                .trim_start_matches("http://")
                .parse::<std::net::SocketAddr>()
                .ok();
            for _ in 0..50 {
                if h.is_finished() {
                    break;
                }
                if let Some(addr) = addr {
                    let _ = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = h.join();
        }
    }

    /// Unsigned JWT with the given payload (header `{"alg":"none"}`).
    pub fn jwt(payload: &serde_json::Value) -> String {
        fn b64(bytes: &[u8]) -> String {
            const T: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | *b as u32)
                    << (8 * (3 - chunk.len()));
                for i in 0..=chunk.len() {
                    out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
                }
            }
            out
        }
        format!(
            "{}.{}.{}",
            b64(br#"{"alg":"none","typ":"JWT"}"#),
            b64(payload.to_string().as_bytes()),
            b64(b"synthetic-signature")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{jwt, Reply, Server};
    use super::*;
    use std::path::PathBuf;

    const NOW: i64 = 1_780_000_000;
    const CLAUDE_TOKEN: &str = "sk-ant-oat01-SYNTHETIC-CLAUDE-TOKEN-NOT-REAL";
    const T: Duration = Duration::from_secs(2);

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("qm-oauth-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn claude_dir(root: &Path, name: &str, body: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(crate::aip::CLAUDE_PAYLOAD), body).unwrap();
        dir
    }
    fn claude_json(token: &str, expires_ms: i64, plan: &str) -> String {
        json!({"claudeAiOauth": {"accessToken": token, "refreshToken": "sk-ant-ort01-SYNTHETIC-REFRESH",
            "expiresAt": expires_ms, "scopes": ["user:inference", "user:profile"], "subscriptionType": plan}})
        .to_string()
    }
    fn codex_dir(root: &Path, name: &str, body: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(crate::aip::CODEX_PAYLOAD), body).unwrap();
        dir
    }
    fn codex_json(exp: i64, email: &str, account: &str, plan: &str) -> String {
        let access = jwt(
            &json!({"exp": exp, "https://api.openai.com/auth": {"chatgpt_account_id": account, "chatgpt_plan_type": plan}}),
        );
        let id = jwt(
            &json!({"email": email, "https://api.openai.com/auth": {"chatgpt_account_id": account, "chatgpt_plan_type": plan}}),
        );
        json!({"OPENAI_API_KEY": null, "tokens": {"id_token": id, "access_token": access,
            "refresh_token": "SYNTHETIC-REFRESH-NOT-REAL", "account_id": account}, "last_refresh": "2026-01-01T00:00:00Z"})
        .to_string()
    }
    const CLAUDE_BODY: &str = r#"{"five_hour":{"utilization":12.5,"resets_at":"2026-07-14T17:30:00+00:00"},
        "seven_day":{"utilization":61,"resets_at":"2026-07-19T12:00:00+00:00"},
        "seven_day_sonnet":null,
        "limits":[{"kind":"session","percent":12.5},{"kind":"weekly_all","percent":61},
          {"kind":"weekly_scoped","percent":33,"resets_at":"2026-07-19T12:00:00+00:00","scope":{"model":{"display_name":"Fable"}}}],
        "extra_usage":{"is_enabled":true,"spend_limit_reached":false,"monthly_limit":5000,"used_credits":120}}"#;
    const CODEX_BODY: &str = r#"{"plan_type":"plus",
        "rate_limit":{"allowed":true,"limit_reached":false,
          "primary_window":{"used_percent":40,"limit_window_seconds":18000,"reset_after_seconds":3600,"reset_at":1800003600},
          "secondary_window":{"used_percent":7,"limit_window_seconds":604800,"reset_after_seconds":86400,"reset_at":1800086400}},
        "credits":{"has_credits":false,"unlimited":false,"balance":null},
        "additional_rate_limits":[{"limit_name":"Reserve","metered_feature":"luna_reserve",
          "rate_limit":{"allowed":true,"limit_reached":false,"primary_window":{"used_percent":3,"limit_window_seconds":18000,"reset_after_seconds":10,"reset_at":1800000010},"secondary_window":null}}],
        "rate_limit_reset_credits":{"available_count":2,"credits":[]},
        "account_id":"acct-synthetic-1","user_id":"user-synthetic-1"}"#;

    #[test]
    fn claude_credential_loads_plain_or_hex_json_and_detects_expiry() {
        let root = scratch("claude-load");
        let plain = claude_dir(
            &root,
            "plain",
            &claude_json(CLAUDE_TOKEN, (NOW + 3600) * 1000, "max"),
        );
        let c = load_claude(&plain).unwrap();
        assert_eq!(c.subscription_type(), Some("max"));
        assert!(!c.expired(NOW));
        assert!(c.expired(NOW + 3600));
        assert_eq!(
            format!("{c:?}").contains("SYNTHETIC"),
            false,
            "debug never prints the token"
        );
        let hex: String = claude_json(CLAUDE_TOKEN, NOW - 10, "pro")
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        let hexed = claude_dir(&root, "hex", &hex);
        let c = load_claude(&hexed).unwrap();
        assert!(
            c.expired(NOW),
            "expiresAt in the past (seconds accepted too)"
        );
        for (name, body) in [
            ("sentinel", "SYNTHETIC-SENTINEL-NOT-A-CREDENTIAL"),
            ("array", "[]"),
            ("other-shape", r#"{"foo":1}"#),
        ] {
            let d = claude_dir(&root, name, body);
            assert_eq!(
                load_claude(&d).err(),
                Some(ReadError::CredentialUnreadable),
                "{name}"
            );
        }
        let empty = claude_dir(&root, "no-token", r#"{"claudeAiOauth":{"accessToken":""}}"#);
        assert_eq!(load_claude(&empty).err(), Some(ReadError::NoToken));
        assert_eq!(
            load_claude(&root.join("absent")).err(),
            Some(ReadError::CredentialUnreadable)
        );
        let big = claude_dir(&root, "big", &"x".repeat(70_000));
        assert_eq!(
            load_claude(&big).err(),
            Some(ReadError::CredentialUnreadable)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn codex_credential_reads_tokens_claims_and_rejects_api_key_homes() {
        let root = scratch("codex-load");
        let home = codex_dir(
            &root,
            "chatgpt",
            &codex_json(NOW + 600, "Someone@Example.invalid", "acct-1", "pro"),
        );
        let c = load_codex(&home).unwrap();
        assert_eq!(c.account_id.as_deref(), Some("acct-1"));
        assert_eq!(c.email.as_deref(), Some("someone@example.invalid"));
        assert_eq!(c.plan_type.as_deref(), Some("pro"));
        assert!(!c.expired(NOW) && c.expired(NOW + 600));
        assert!(
            !format!("{c:?}").contains("eyJ"),
            "debug never prints a token"
        );
        let api = codex_dir(
            &root,
            "apikey",
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-SYNTHETIC"}"#,
        );
        assert_eq!(load_codex(&api).err(), Some(ReadError::NoToken));
        let junk = codex_dir(&root, "junk", "SYNTHETIC-SENTINEL-NOT-A-CREDENTIAL");
        assert_eq!(
            load_codex(&junk).err(),
            Some(ReadError::CredentialUnreadable)
        );
        // Opaque (non-JWT) tokens are still usable: no expiry known, no claims.
        let opaque = codex_dir(
            &root,
            "opaque",
            r#"{"OPENAI_API_KEY":null,"tokens":{"access_token":"opaque-synthetic","refresh_token":"r","account_id":"acct-2"}}"#,
        );
        let c = load_codex(&opaque).unwrap();
        assert_eq!(c.expires_at, None);
        assert_eq!(c.account_id.as_deref(), Some("acct-2"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_read_sends_the_profile_token_to_the_usage_path_and_projects_get_usage() {
        let root = scratch("claude-http");
        let dir = claude_dir(
            &root,
            "work",
            &claude_json(CLAUDE_TOKEN, (NOW + 3600) * 1000, "max_20x"),
        );
        let server = Server::start(vec![Reply::json(200, CLAUDE_BODY)]);
        let value = read_claude_profile(&dir, &server.base, T, NOW).unwrap();
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].starts_with("GET /api/oauth/usage HTTP/1.1"),
            "{}",
            requests[0]
        );
        assert_eq!(
            Server::header(&requests[0], "authorization").as_deref(),
            Some(&*format!("Bearer {CLAUDE_TOKEN}"))
        );
        assert_eq!(
            Server::header(&requests[0], "anthropic-beta").as_deref(),
            Some("oauth-2025-04-20")
        );
        assert_eq!(
            Server::header(&requests[0], "anthropic-version").as_deref(),
            Some("2023-06-01")
        );
        assert_eq!(value["rate_limits_available"], true);
        assert_eq!(value["subscription_type"], "max_20x");
        assert_eq!(value["rate_limits"]["five_hour"]["utilization"], 12.5);
        assert_eq!(
            value["rate_limits"]["seven_day"]["resets_at"],
            "2026-07-19T12:00:00+00:00"
        );
        assert_eq!(
            value["rate_limits"]["model_scoped"][0]["display_name"],
            "Fable"
        );
        assert_eq!(value["rate_limits"]["model_scoped"][0]["utilization"], 33.0);
        assert_eq!(value["rate_limits"]["extra_usage"]["is_enabled"], true);
        // The existing CLI parser accepts the projection unchanged.
        let snap = crate::claude::usage_snapshot(&value, NOW);
        let ids: Vec<_> = snap
            .metrics
            .iter()
            .map(|m| (m.id, m.used_percent))
            .collect();
        assert_eq!(
            ids,
            [
                ("claude:five_hour", Some(12.5)),
                ("claude:seven_day", Some(61.0)),
                ("claude:weekly_fable", Some(33.0))
            ]
        );
        assert_eq!(snap.plan_type.as_deref(), Some("max_20x"));
        assert_eq!(snap.state, "ready");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_errors_are_classified_and_never_echo_the_token() {
        let root = scratch("claude-errors");
        let dir = claude_dir(
            &root,
            "p",
            &claude_json(CLAUDE_TOKEN, (NOW + 3600) * 1000, "pro"),
        );
        let cases = [
            (
                Reply::json(401, r#"{"error":{"type":"authentication_error"}}"#),
                ReadError::Unauthorized,
            ),
            (
                Reply::json(403, r#"{"error":{"type":"permission_error"}}"#),
                ReadError::Forbidden,
            ),
            (
                Reply::json(429, r#"{"error":{"type":"rate_limit_error"}}"#),
                ReadError::RateLimited(None),
            ),
            (
                Reply::json(429, "{}").with_header("retry-after", "37"),
                ReadError::RateLimited(Some(37)),
            ),
            // aip's observation: an expired token answers 429 with retry-after 0.
            (
                Reply::json(429, "{}").with_header("retry-after", "0"),
                ReadError::Unauthorized,
            ),
            (Reply::json(503, "down"), ReadError::Http(503)),
            (Reply::json(200, "not json at all"), ReadError::Malformed),
            (Reply::json(200, "[1,2]"), ReadError::Malformed),
            (Reply::hang(), ReadError::Timeout),
        ];
        for (reply, expected) in cases {
            let server = Server::start(vec![reply]);
            let err = read_claude_profile(&dir, &server.base, Duration::from_millis(800), NOW)
                .unwrap_err();
            assert_eq!(err, expected);
            assert!(!format!("{err:?}").contains("SYNTHETIC"));
        }
        // Expired locally: the provider is not contacted at all.
        let stale = claude_dir(
            &root,
            "stale",
            &claude_json(CLAUDE_TOKEN, (NOW - 5) * 1000, "pro"),
        );
        let server = Server::start(vec![Reply::json(200, CLAUDE_BODY)]);
        assert_eq!(
            read_claude_profile(&stale, &server.base, T, NOW).err(),
            Some(ReadError::Expired)
        );
        assert!(server.requests().is_empty());
        // Connection refused (nothing listening) is a transport error.
        let dead = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        assert_eq!(
            read_claude_profile(&dir, &dead, T, NOW).err(),
            Some(ReadError::Transport)
        );
        // A bearer never goes to a non-loopback plain-HTTP base.
        assert_eq!(
            read_claude_profile(&dir, "http://example.invalid", T, NOW).err(),
            Some(ReadError::InsecureBase)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn codex_read_sends_bearer_and_account_header_and_maps_the_payload_for_normalize() {
        let root = scratch("codex-http");
        let home = codex_dir(
            &root,
            "perso",
            &codex_json(
                NOW + 600,
                "perso@example.invalid",
                "acct-synthetic-1",
                "plus",
            ),
        );
        let server = Server::start(vec![Reply::json(200, CODEX_BODY)]);
        let usage = read_codex_home(&home, &server.base, T, NOW).unwrap();
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].starts_with("GET /wham/usage HTTP/1.1"),
            "{}",
            requests[0]
        );
        let auth = Server::header(&requests[0], "authorization").unwrap();
        assert!(auth.starts_with("Bearer eyJ"), "{auth}");
        assert_eq!(
            Server::header(&requests[0], "chatgpt-account-id").as_deref(),
            Some("acct-synthetic-1")
        );
        assert_eq!(usage.plan_type.as_deref(), Some("plus"));
        assert_eq!(usage.email.as_deref(), Some("perso@example.invalid"));
        assert_eq!(usage.account_id.as_deref(), Some("acct-synthetic-1"));
        let rows = crate::codex::normalize(&usage.rate_limits).unwrap();
        let summary: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.period.as_str(),
                    r.window_minutes,
                    r.used_percent,
                    r.reset_at.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "codex:codex:primary",
                    Some(300),
                    Some(40.0),
                    "2027-01-15T09:00:00+00:00"
                ),
                (
                    "codex:codex:secondary",
                    Some(10080),
                    Some(7.0),
                    "2027-01-16T08:00:00+00:00"
                ),
                (
                    "codex:luna_reserve:primary",
                    Some(300),
                    Some(3.0),
                    "2027-01-15T08:00:10+00:00"
                ),
            ]
        );
        assert_eq!(
            usage.rate_limits["rateLimitResetCredits"]["availableCount"],
            2
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn codex_errors_are_classified_and_expired_tokens_stay_local() {
        let root = scratch("codex-errors");
        let home = codex_dir(
            &root,
            "h",
            &codex_json(NOW + 600, "x@example.invalid", "acct", "plus"),
        );
        for (reply, expected) in [
            (
                Reply::json(401, r#"{"detail":"Unauthorized"}"#),
                ReadError::Unauthorized,
            ),
            (Reply::json(403, "{}"), ReadError::Forbidden),
            (Reply::json(429, "{}"), ReadError::RateLimited(None)),
            (Reply::json(500, "{}"), ReadError::Http(500)),
            (Reply::json(200, "<html>"), ReadError::Malformed),
            (Reply::hang(), ReadError::Timeout),
        ] {
            let server = Server::start(vec![reply]);
            assert_eq!(
                read_codex_home(&home, &server.base, Duration::from_millis(800), NOW).err(),
                Some(expected)
            );
        }
        let stale = codex_dir(
            &root,
            "stale",
            &codex_json(NOW - 1, "x@example.invalid", "acct", "plus"),
        );
        let server = Server::start(vec![Reply::json(200, CODEX_BODY)]);
        assert_eq!(
            read_codex_home(&stale, &server.base, T, NOW).err(),
            Some(ReadError::Expired)
        );
        assert!(server.requests().is_empty());
        // A null rate_limit (no window) is a valid empty read, not an error.
        let server = Server::start(vec![Reply::json(
            200,
            r#"{"plan_type":"free","rate_limit":null}"#,
        )]);
        let usage = read_codex_home(&home, &server.base, T, NOW).unwrap();
        assert!(crate::codex::normalize(&usage.rate_limits)
            .unwrap()
            .is_empty());
        assert_eq!(usage.plan_type.as_deref(), Some("free"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn two_profiles_are_read_with_their_own_tokens_and_the_files_are_untouched() {
        let root = scratch("routing");
        let a = claude_dir(
            &root,
            "a",
            &claude_json("sk-ant-oat01-SYNTHETIC-A", (NOW + 99) * 1000, "pro"),
        );
        let b = claude_dir(
            &root,
            "b",
            &claude_json("sk-ant-oat01-SYNTHETIC-B", (NOW + 99) * 1000, "max"),
        );
        let before = |d: &Path| std::fs::read(d.join(crate::aip::CLAUDE_PAYLOAD)).unwrap();
        let (fa, fb) = (before(&a), before(&b));
        let server = Server::start(vec![
            Reply::json(200, CLAUDE_BODY),
            Reply::json(200, CLAUDE_BODY),
        ]);
        let va = read_claude_profile(&a, &server.base, T, NOW).unwrap();
        let vb = read_claude_profile(&b, &server.base, T, NOW).unwrap();
        let tokens: Vec<_> = server
            .requests()
            .iter()
            .map(|r| Server::header(r, "authorization").unwrap())
            .collect();
        assert_eq!(
            tokens,
            [
                "Bearer sk-ant-oat01-SYNTHETIC-A",
                "Bearer sk-ant-oat01-SYNTHETIC-B"
            ]
        );
        assert_eq!(
            (
                va["subscription_type"].as_str(),
                vb["subscription_type"].as_str()
            ),
            (Some("pro"), Some("max"))
        );
        assert_eq!(
            (before(&a), before(&b)),
            (fa, fb),
            "credentials never rewritten"
        );
        assert!(
            !std::fs::read_dir(&a)
                .unwrap()
                .flatten()
                .any(|e| e.file_name() != crate::aip::CLAUDE_PAYLOAD),
            "nothing created in the profile"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
