use super::*;
use crate::oauth_usage::test_support::{Reply, Server};
use serde_json::json;
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

#[test]
fn codex_connection_identity_includes_account_even_when_bearer_is_shared() {
    let credential = |account: &str| {
        crate::oauth_usage::codex_credential(
            &json!({"tokens":{"access_token":"SYNTHETIC-SHARED","account_id":account}}),
        )
        .unwrap()
    };
    assert!(credential("account-A").same_connection(&credential("account-A")));
    assert!(!credential("account-A").same_connection(&credential("account-B")));
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qm-ai-profiles-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn cli(&self, id: &str) -> PathBuf {
        let path = root(&self.0).join("profiles").join(id).join("cli-config");
        fs::create_dir_all(&path).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn oauth(token: &str, exp: i64) -> Vec<u8> {
    json!({"claudeAiOauth":{"accessToken":token,"refreshToken":"SYNTHETIC-SHARED-NO-ROTATE","expiresAt":exp*1000,"subscriptionType":"max"}}).to_string().into_bytes()
}
fn identity(home: &Path) {
    fs::write(
        home.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"account-A"}}"#,
    )
    .unwrap();
}
fn identity_reply() -> Reply {
    Reply::json(200, r#"{"account":{"uuid":"account-A"}}"#)
}
fn usage_reply() -> Reply {
    Reply::json(
        200,
        r#"{"five_hour":{"utilization":17,"resets_at":"2099-01-01T00:00:00Z"},"seven_day":{"utilization":33,"resets_at":"2099-01-02T00:00:00Z"}}"#,
    )
}
fn no_keychain(_: &str) -> Result<Vec<u8>, ReadError> {
    panic!("file credential must not fall back to Keychain")
}

#[test]
fn exact_raw_keychain_slot_and_identity_guard_no_primary_fallback() {
    let f = Fixture::new();
    let home = f.cli("a");
    identity(&home);
    assert_eq!(
        keychain_service(Path::new("test")),
        "Claude Code-credentials-9f86d081"
    );
    assert_ne!(
        keychain_service(&home),
        keychain_service(Path::new(&format!("{}/", home.display())))
    );
    let service = keychain_service(&home);
    let server = Server::start(vec![identity_reply(), usage_reply()]);
    let calls = AtomicUsize::new(0);
    let result = read_claude_with(
        &home,
        &server.base,
        Duration::from_secs(2),
        1_800_000_000,
        |s| {
            assert_eq!(s, service);
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(oauth("SYNTHETIC-KEYCHAIN", 2_000_000_000))
        },
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(result["subscription_type"], "max");
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /api/oauth/profile "));
    assert!(requests[1].starts_with("GET /api/oauth/usage "));
    for request in requests {
        assert_eq!(
            Server::header(&request, "authorization").as_deref(),
            Some("Bearer SYNTHETIC-KEYCHAIN")
        );
    }
    assert!(!home.join(".credentials.json").exists());
}

#[test]
fn collision_missing_identity_and_changed_connection_withhold_usage() {
    let f = Fixture::new();
    let home = f.cli("a");
    identity(&home);
    let server = Server::start(vec![Reply::json(
        200,
        r#"{"account":{"uuid":"other-account"}}"#,
    )]);
    assert_eq!(
        read_claude_with(
            &home,
            &server.base,
            Duration::from_secs(2),
            1_800_000_000,
            |_| Ok(oauth("SYNTHETIC", 2_000_000_000))
        ),
        Err(ClaudeError::AccountMismatch)
    );
    assert_eq!(
        server.requests().len(),
        1,
        "no quota read after identity mismatch"
    );
    fs::remove_file(home.join(".claude.json")).unwrap();
    assert_eq!(
        read_claude_with(
            &home,
            &server.base,
            Duration::from_secs(2),
            1_800_000_000,
            |_| Ok(oauth("SYNTHETIC", 2_000_000_000))
        ),
        Err(ClaudeError::IdentityUnavailable)
    );
    identity(&home);
    let server = Server::start(vec![identity_reply(), usage_reply()]);
    let calls = AtomicUsize::new(0);
    assert_eq!(
        read_claude_with(
            &home,
            &server.base,
            Duration::from_secs(2),
            1_800_000_000,
            |_| {
                let n = calls.fetch_add(1, Ordering::Relaxed);
                Ok(oauth(
                    if n == 0 {
                        "SYNTHETIC-OLD"
                    } else {
                        "SYNTHETIC-NEW"
                    },
                    2_000_000_000,
                ))
            }
        ),
        Err(ClaudeError::Changed)
    );
}

#[test]
fn claude_continuity_expiry_owner_renewal_and_revocation_are_read_only() {
    let f = Fixture::new();
    let home = f.cli("a");
    identity(&home);
    let path = home.join(".credentials.json");
    let now = 1_800_000_000;
    fs::write(&path, oauth("SYNTHETIC-OLD", now + 600)).unwrap();
    let old = fs::read(&path).unwrap();
    let server = Server::start(vec![
        identity_reply(),
        usage_reply(),
        identity_reply(),
        usage_reply(),
        Reply::json(401, "{}"),
    ]);
    assert!(read_claude_with(
        &home,
        &server.base,
        Duration::from_secs(2),
        now,
        no_keychain
    )
    .is_ok());
    assert_eq!(fs::read(&path).unwrap(), old);
    assert_eq!(
        read_claude_with(
            &home,
            &server.base,
            Duration::from_secs(2),
            now + 601,
            no_keychain
        ),
        Err(ClaudeError::Read(ReadError::Expired))
    );
    assert_eq!(
        server.requests().len(),
        2,
        "expired bearer never sent, no refresh request"
    );
    // Simulate atomic publication by the owning CLI/manager, not Quota Monitor.
    let fresh = oauth("SYNTHETIC-NEW", now + 3600);
    fs::write(home.join("owner.tmp"), &fresh).unwrap();
    fs::rename(home.join("owner.tmp"), &path).unwrap();
    assert!(read_claude_with(
        &home,
        &server.base,
        Duration::from_secs(2),
        now + 602,
        no_keychain
    )
    .is_ok());
    assert_eq!(
        read_claude_with(
            &home,
            &server.base,
            Duration::from_secs(2),
            now + 603,
            no_keychain
        ),
        Err(ClaudeError::Read(ReadError::Unauthorized))
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 5);
    assert!(requests.iter().all(|r| r.starts_with("GET ")));
    assert_eq!(
        Server::header(&requests[3], "authorization").as_deref(),
        Some("Bearer SYNTHETIC-NEW")
    );
    assert_eq!(fs::read(&path).unwrap(), fresh);
    assert!(!home.join(".credentials.json.lock").exists());
}

#[test]
fn claude_errors_and_corrupt_file_never_read_other_credential_slots() {
    let f = Fixture::new();
    let home = f.cli("a");
    identity(&home);
    let path = home.join(".credentials.json");
    fs::write(&path, b"corrupt").unwrap();
    assert_eq!(
        read_claude_with(
            &home,
            "http://127.0.0.1:9",
            Duration::from_millis(30),
            1_800_000_000,
            no_keychain
        ),
        Err(ClaudeError::Read(ReadError::CredentialUnreadable))
    );
    fs::write(&path, oauth("SYNTHETIC", 2_000_000_000)).unwrap();
    for (reply, expected) in [
        (
            Reply::json(429, "{}").with_header("retry-after", "60"),
            ReadError::RateLimited(Some(60)),
        ),
        (Reply::json(403, "{}"), ReadError::Forbidden),
        (Reply::json(500, "{}"), ReadError::Http(500)),
        (Reply::json(200, "not json"), ReadError::Malformed),
    ] {
        let server = Server::start(vec![identity_reply(), reply]);
        assert_eq!(
            read_claude_with(
                &home,
                &server.base,
                Duration::from_secs(2),
                1_800_000_000,
                no_keychain
            ),
            Err(ClaudeError::Read(expected))
        );
    }
    let server = Server::start(vec![Reply::hang()]);
    assert_eq!(
        read_claude_with(
            &home,
            &server.base,
            Duration::from_millis(30),
            1_800_000_000,
            no_keychain
        ),
        Err(ClaudeError::Read(ReadError::Timeout))
    );
}

#[test]
fn metadata_is_bounded_rejects_traversal_unknown_apps_and_surfaces() {
    let f = Fixture::new();
    f.cli("valid");
    let metadata = root(&f.0).join("profiles.json");
    fs::write(&metadata,r#"{"profiles":[{"id":"valid","name":"Work","surfaces":{"cli":true}}, {"id":"../escape","name":"Bad","surfaces":{"cli":true}}, {"id":"unknown","app":"other","name":"Unknown","surfaces":{"cli":true}}, {"id":"desktop","app":"codex","name":"Desk","surfaces":{"gui":true,"cli":false}}]}"#).unwrap();
    assert_eq!(profiles(&f.0, "claude").len(), 1);
    let desktop = profiles(&f.0, "codex");
    assert_eq!(desktop.len(), 1);
    assert!(!desktop[0].complete);
    assert!(!desktop[0].raw.ends_with("cli-config"));
    fs::write(&metadata, "bad json").unwrap();
    assert!(profiles(&f.0, "claude").is_empty());
    fs::write(&metadata, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(profiles(&f.0, "claude").is_empty());
}

#[cfg(unix)]
#[test]
fn aliases_preferences_primary_and_removed_metadata_do_not_enable_cli_or_change_mode() {
    use crate::{
        account_sources::{self, Collect},
        claude_accounts::{identity, AccountPrefs, Profile},
    };
    let f = Fixture::new();
    let home = f.cli("a");
    fs::write(
        root(&f.0).join("profiles.json"),
        r#"{"profiles":[{"id":"a","name":"Work","surfaces":{"cli":true}}]}"#,
    )
    .unwrap();
    let alias = f.0.join(".claude-alias");
    std::os::unix::fs::symlink(&home, &alias).unwrap();
    let prefs = AccountPrefs {
        mode: "single".into(),
        profiles: vec![Profile {
            id: identity(&home),
            name: "My name".into(),
            path: alias.to_string_lossy().into(),
            visible: false,
        }],
        ..Default::default()
    };
    let resolved = account_sources::reconcile(
        account_sources::all(&f.0, &account_sources::CLAUDE, None, &prefs),
        &prefs,
    );
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[1].candidate.name, "My name");
    assert_eq!(resolved[1].candidate.raw, home.to_string_lossy());
    assert_eq!(resolved[1].candidate.collect, Collect::ManagedCredential);
    assert!(!resolved[1].visible);
    assert_eq!(prefs.mode, "single");
    let primary = account_sources::reconcile(
        account_sources::all(&f.0, &account_sources::CLAUDE, Some(&home), &prefs),
        &prefs,
    );
    assert_eq!(primary.len(), 1);
    assert!(primary[0].primary());
    assert_eq!(primary[0].candidate.collect, Collect::Inherit);
    let mut excluded = prefs.clone();
    excluded.excluded.push(identity(&home));
    assert_eq!(
        account_sources::reconcile(
            account_sources::all(&f.0, &account_sources::CLAUDE, None, &excluded),
            &excluded
        )
        .len(),
        1
    );
    fs::remove_file(root(&f.0).join("profiles.json")).unwrap();
    let found = account_sources::reconcile(
        account_sources::all(&f.0, &account_sources::CLAUDE, None, &prefs),
        &prefs,
    );
    assert_eq!(
        found[1].candidate.collect,
        Collect::ManagedCredential,
        "saved leftover cannot bypass refresh guard"
    );
}

#[test]
fn distinct_managed_claude_accounts_use_their_own_tokens_and_results() {
    let f = Fixture::new();
    let server = Server::start(vec![
        Reply::json(200, r#"{"account":{"uuid":"account-A"}}"#),
        Reply::json(200, r#"{"five_hour":{"utilization":12}}"#),
        Reply::json(200, r#"{"account":{"uuid":"account-B"}}"#),
        Reply::json(200, r#"{"five_hour":{"utilization":78}}"#),
    ]);
    for (id, uuid, token, expected) in [
        ("alpha", "account-A", "SYNTHETIC-A", 12),
        ("beta", "account-B", "SYNTHETIC-B", 78),
    ] {
        let home = f.cli(id);
        let credential = oauth(token, 2_000_000_000);
        fs::write(home.join(".credentials.json"), &credential).unwrap();
        fs::write(
            home.join(".claude.json"),
            json!({"oauthAccount":{"accountUuid":uuid}}).to_string(),
        )
        .unwrap();
        let value = read_claude_with(
            &home,
            &server.base,
            Duration::from_secs(2),
            1_800_000_000,
            no_keychain,
        )
        .unwrap();
        assert_eq!(value["rate_limits"]["five_hour"]["utilization"], expected);
        assert_eq!(
            fs::read(home.join(".credentials.json")).unwrap(),
            credential
        );
    }
    let requests = server.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        Server::header(&requests[1], "authorization").as_deref(),
        Some("Bearer SYNTHETIC-A")
    );
    assert_eq!(
        Server::header(&requests[3], "authorization").as_deref(),
        Some("Bearer SYNTHETIC-B")
    );
}

#[test]
fn codex_owner_replacing_auth_during_http_hides_stale_result_without_writing() {
    use crate::oauth_usage::test_support::jwt;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let f = Fixture::new();
    let home = f.cli("code");
    let auth = home.join("auth.json");
    let doc = |label| {
        json!({"tokens":{"access_token":jwt(&json!({"exp":2_000_000_000,"label":label})),"refresh_token":"SYNTHETIC-SHARED","account_id":"acct"}}).to_string()
    };
    fs::write(&auth, doc("old")).unwrap();
    let fresh = doc("new");
    let expected = fresh.clone();
    let path = auth.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let owner = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut b = [0];
            stream.read_exact(&mut b).unwrap();
            request.push(b[0]);
        }
        // Publish the owner's new grant before completing the old read.
        let tmp = path.with_extension("owner.tmp");
        fs::write(&tmp, fresh).unwrap();
        fs::rename(tmp, path).unwrap();
        let body = r#"{"plan_type":"plus","rate_limit":{}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    assert_eq!(
        read_codex(&home, &base, Duration::from_secs(3), 1_800_000_000).err(),
        Some(ReadError::CredentialUnreadable)
    );
    owner.join().unwrap();
    assert_eq!(fs::read_to_string(auth).unwrap(), expected);
}
