//! HTTP client for the cdc-rsync sidecar (host-to-host file sync over SSH, ADR 0007).
//!
//! The orchestrator stores profiles and proxies; the sidecar decides direct vs relay per run.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{RsyncEndpoint, RsyncOptions, RsyncProfile};
use crate::saved_secret::REDACTED;
use crate::tool_sidecar::SidecarAccess;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use urlencoding::encode;

pub const TRANSPORTS: [&str; 3] = ["auto", "direct", "relay"];

pub fn effective_auth(e: &RsyncEndpoint) -> &str {
    match e.auth.trim() {
        "" if !e.private_key.trim().is_empty() => "key",
        "" => "password",
        other => other,
    }
}

fn endpoint_bad(_role: &str, msg: impl Into<String>) -> Result<(), AppError> {
    Err(AppError::BadRequest(msg.into()))
}

fn check_endpoint_identity(role: &str, e: &RsyncEndpoint) -> Result<(), AppError> {
    if e.host.trim().is_empty() || e.username.trim().is_empty() {
        return endpoint_bad(
            role,
            format!("{role}.host and {role}.username are required"),
        );
    }
    if e.host.trim().starts_with('-') || e.username.trim().starts_with('-') {
        return endpoint_bad(
            role,
            format!("{role}.host and {role}.username must not start with '-'"),
        );
    }
    let path = e.path.trim();
    if path.is_empty() || path == "~" {
        return endpoint_bad(
            role,
            format!("{role}.path is required (absolute, or relative to the login's home)"),
        );
    }
    if path.starts_with('-') {
        return endpoint_bad(role, format!("{role}.path must not start with '-'"));
    }
    if !e.port.trim().is_empty() && !matches!(e.port.trim().parse::<u32>(), Ok(1..=65535)) {
        return endpoint_bad(role, format!("{role}.port must be 1-65535"));
    }
    Ok(())
}

fn check_endpoint_auth(role: &str, e: &RsyncEndpoint) -> Result<(), AppError> {
    match effective_auth(e) {
        "key" if e.private_key.trim().is_empty() => {
            endpoint_bad(role, format!("{role}.private_key is required for auth=key"))
        }
        "password" if e.password.is_empty() => endpoint_bad(
            role,
            format!("{role}.password is required for auth=password"),
        ),
        "key" | "password" => {
            let fp = e.host_key_fingerprint.trim();
            if !fp.is_empty() && !fp.starts_with("SHA256:") {
                return endpoint_bad(
                    role,
                    format!("{role}.host_key_fingerprint must look like SHA256:… (ssh-keygen -lf)"),
                );
            }
            let bin = e.rsync_path.trim();
            if !bin.is_empty() && !bin.starts_with('/') {
                return endpoint_bad(role, format!("{role}.rsync_path must be an absolute path"));
            }
            Ok(())
        }
        other => endpoint_bad(
            role,
            format!("{role}.auth must be 'key' or 'password', got '{other}'"),
        ),
    }
}

/// Save-time checks so mistakes show in the editor; the sidecar re-validates on every run.
pub fn check_endpoint(role: &str, e: &RsyncEndpoint) -> Result<(), AppError> {
    check_endpoint_identity(role, e)?;
    check_endpoint_auth(role, e)
}

pub fn validate_profile(
    source: &RsyncEndpoint,
    target: &RsyncEndpoint,
    options: &RsyncOptions,
) -> Result<(), AppError> {
    check_endpoint("source", source)?;
    check_endpoint("target", target)?;
    let transport = options.transport.as_deref().unwrap_or("auto");
    if !TRANSPORTS.contains(&transport) {
        return Err(AppError::BadRequest(format!(
            "options.transport must be one of {}",
            TRANSPORTS.join(", ")
        )));
    }
    if transport == "direct" && effective_auth(target) != "key" {
        return Err(AppError::BadRequest(
            "transport=direct needs key auth on the target (it is forwarded through the source); \
             use auto or relay for a password target"
                .into(),
        ));
    }
    if options.max_delete.is_some() && !options.delete {
        return Err(AppError::BadRequest(
            "options.max_delete requires options.delete".into(),
        ));
    }
    Ok(())
}

fn same_login(a: &RsyncEndpoint, b: &RsyncEndpoint) -> bool {
    let port = |e: &RsyncEndpoint| match e.port.trim() {
        "" => "22".to_string(),
        p => p.to_string(),
    };
    a.host.trim().eq_ignore_ascii_case(b.host.trim())
        && port(a) == port(b)
        && a.username.trim() == b.username.trim()
}

/// Editors send `********` back for unchanged secrets. A stored secret is reused only for the same
/// login (host, port, username) — otherwise an edited host would receive another server's key or
/// password (ADR 0005 §11, applied to password, private key and passphrase alike).
pub fn keep_endpoint_secrets(
    role: &str,
    incoming: &mut RsyncEndpoint,
    existing: Option<&RsyncEndpoint>,
) -> Result<(), AppError> {
    let same = existing.is_some_and(|e| same_login(incoming, e));
    let stored = existing.cloned().unwrap_or_default();
    for (what, slot, old) in [
        ("password", &mut incoming.password, stored.password),
        ("private key", &mut incoming.private_key, stored.private_key),
        (
            "key passphrase",
            &mut incoming.passphrase,
            stored.passphrase,
        ),
    ] {
        if slot != REDACTED {
            continue;
        }
        if same || old.is_empty() {
            *slot = old;
        } else {
            return Err(AppError::BadRequest(format!(
                "{role} host, port or username changed; enter the {role} {what} again"
            )));
        }
    }
    Ok(())
}

/// Each run is a new sidecar job; `profile_id` lets the sidecar refuse a second concurrent run
/// and keys the relay staging copy (so relay runs stay incremental).
pub fn profile_run_payload(profile: &RsyncProfile, dry_run: Option<bool>) -> Value {
    let mut body = json!({
        "profile_id": profile.id.to_string(),
        "name": profile.name,
        "source": profile.source,
        "target": profile.target,
        "options": profile.options,
    });
    if let Some(d) = dry_run {
        body["dry_run"] = json!(d);
    }
    body
}

fn base_url(cfg: &AppConfig) -> Result<&str, AppError> {
    cfg.rsync_url
        .as_deref()
        .map(|u| u.trim_end_matches('/'))
        .ok_or_else(|| {
            AppError::BadRequest("rsync sidecar is not configured (ORCH_RSYNC_URL)".into())
        })
}

fn access<'a>(cfg: &'a AppConfig, client: &'a Client) -> Result<SidecarAccess<'a>, AppError> {
    Ok(SidecarAccess {
        client,
        base_url: base_url(cfg)?,
        bearer_token: cfg.rsync_token.as_deref(),
        tool: "rsync",
        token_env: "ORCH_RSYNC_TOKEN",
        extra_bad_request: &[],
    })
}

pub async fn sidecar_request(
    cfg: &AppConfig,
    client: &Client,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, AppError> {
    access(cfg, client)?.request(method, path, body).await
}

crate::sidecar_v1_handlers!(access);

pub async fn test_route(cfg: &AppConfig, client: &Client, body: Value) -> Result<Value, AppError> {
    sidecar_request(cfg, client, Method::POST, "/v1/test-route", Some(body)).await
}

pub async fn delete_job(cfg: &AppConfig, client: &Client, job_id: &str) -> Result<Value, AppError> {
    access(cfg, client)?.delete_job(job_id).await
}

pub async fn list_staging(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    sidecar_request(cfg, client, Method::GET, "/v1/staging", None).await
}

pub async fn delete_staging(cfg: &AppConfig, client: &Client, id: &str) -> Result<Value, AppError> {
    sidecar_request(
        cfg,
        client,
        Method::DELETE,
        &format!("/v1/staging/{}", encode(id)),
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(host: &str) -> RsyncEndpoint {
        RsyncEndpoint {
            name: host.into(),
            host: host.into(),
            port: "22".into(),
            username: "deploy".into(),
            auth: "key".into(),
            private_key: "-----BEGIN OPENSSH PRIVATE KEY-----".into(),
            path: "/srv/www/".into(),
            ..Default::default()
        }
    }

    #[test]
    fn endpoints_are_validated() {
        assert!(check_endpoint("source", &ep("a")).is_ok());
        for mutate in [
            (|e: &mut RsyncEndpoint| e.path = " ".into()) as fn(&mut RsyncEndpoint),
            |e| e.path = "-rf".into(),
            |e| e.host = "-oProxyCommand=x".into(),
            |e| e.port = "70000".into(),
            |e| e.private_key.clear(),
            |e| e.host_key_fingerprint = "MD5:aa".into(),
            |e| e.rsync_path = "rsync".into(),
            |e| e.auth = "token".into(),
        ] {
            let mut e = ep("a");
            mutate(&mut e);
            assert!(check_endpoint("source", &e).is_err(), "{e:?}");
        }
        let mut pw = ep("a");
        pw.auth = "password".into();
        assert!(check_endpoint("source", &pw).is_err());
        pw.password = "x".into();
        assert!(check_endpoint("source", &pw).is_ok());
    }

    #[test]
    fn direct_transport_needs_a_key_target() {
        let mut pw = ep("t");
        pw.auth = "password".into();
        pw.password = "x".into();
        let direct = RsyncOptions {
            transport: Some("direct".into()),
            ..Default::default()
        };
        assert!(validate_profile(&ep("s"), &pw, &direct).is_err());
        assert!(validate_profile(&ep("s"), &pw, &RsyncOptions::default()).is_ok());
        assert!(validate_profile(&ep("s"), &ep("t"), &direct).is_ok());
        let bogus = RsyncOptions {
            transport: Some("ftp".into()),
            ..Default::default()
        };
        assert!(validate_profile(&ep("s"), &ep("t"), &bogus).is_err());
        let limit = RsyncOptions {
            max_delete: Some(5),
            ..Default::default()
        };
        assert!(validate_profile(&ep("s"), &ep("t"), &limit).is_err());
    }

    #[test]
    fn redacted_secrets_kept_only_for_the_same_login() {
        let mut stored = ep("Host-A");
        stored.passphrase = "pp".into();
        let mut same = ep("host-a");
        same.private_key = REDACTED.into();
        same.passphrase = REDACTED.into();
        keep_endpoint_secrets("source", &mut same, Some(&stored)).unwrap();
        assert_eq!(same.private_key, stored.private_key);
        assert_eq!(same.passphrase, "pp");

        let mut moved = ep("other-host");
        moved.private_key = REDACTED.into();
        assert!(keep_endpoint_secrets("source", &mut moved, Some(&stored)).is_err());

        // Nothing stored: the marker becomes empty, never a literal "********".
        let mut fresh = ep("other-host");
        fresh.password = REDACTED.into();
        keep_endpoint_secrets("source", &mut fresh, Some(&stored)).unwrap();
        assert_eq!(fresh.password, "");
        let mut new_profile = ep("x");
        new_profile.private_key = REDACTED.into();
        keep_endpoint_secrets("target", &mut new_profile, None).unwrap();
        assert_eq!(new_profile.private_key, "");
    }

    #[test]
    fn run_payload_carries_profile_and_dry_run_override() {
        let now = chrono::Utc::now();
        let p = RsyncProfile {
            id: uuid::Uuid::nil(),
            name: "site".into(),
            source: ep("s"),
            target: ep("t"),
            options: RsyncOptions {
                delete: true,
                ..Default::default()
            },
            last_job_id: None,
            created_at: now,
            updated_at: now,
        };
        let body = profile_run_payload(&p, Some(true));
        assert_eq!(body["profile_id"], uuid::Uuid::nil().to_string());
        assert_eq!(body["dry_run"], true);
        assert_eq!(body["options"]["delete"], true);
        assert!(
            body["options"].get("checksum").is_none(),
            "unset options are not sent"
        );
        assert!(profile_run_payload(&p, None).get("dry_run").is_none());
    }
}
