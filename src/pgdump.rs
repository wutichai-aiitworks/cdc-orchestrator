//! HTTP client for the cdc-pgdump sidecar (PostgreSQL pg_dump / pg_dump → pg_restore).

use std::time::Duration;

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{PgDumpDestination, PgDumpProfile, SchemaCompareConn};
use crate::pgcopy::is_postgres;
use crate::tool_sidecar::{map_error_status, SidecarAccess};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use urlencoding::encode;

pub const MODE_DUMP: &str = "dump";
pub const MODE_DUMP_RESTORE: &str = "dump_restore";

/// Profile-level checks done before saving, so mistakes surface in the editor rather than at run time.
/// The sidecar re-validates everything (options, patterns) when a job starts.
pub fn validate_profile(
    mode: &str,
    source: &SchemaCompareConn,
    target: Option<&SchemaCompareConn>,
    format: Option<&str>,
) -> Result<(), AppError> {
    if mode != MODE_DUMP && mode != MODE_DUMP_RESTORE {
        return Err(AppError::BadRequest(format!(
            "mode must be '{MODE_DUMP}' or '{MODE_DUMP_RESTORE}'"
        )));
    }
    check_conn("source", source)?;
    if mode == MODE_DUMP_RESTORE {
        let target = target.ok_or_else(|| {
            AppError::BadRequest("target is required for mode dump_restore".into())
        })?;
        check_conn("target", target)?;
        if let Some(f) = format {
            if !f.is_empty() && f != "custom" {
                return Err(AppError::BadRequest(
                    "dump_restore requires options.format = custom".into(),
                ));
            }
        }
    }
    Ok(())
}

pub const MAX_DESTINATIONS: usize = 5;
const REDACTED: &str = "********";

/// Save-time checks for SFTP destinations (the sidecar re-validates on run).
pub fn validate_destinations(dests: &[PgDumpDestination]) -> Result<(), AppError> {
    if dests.len() > MAX_DESTINATIONS {
        return Err(AppError::BadRequest(format!(
            "at most {MAX_DESTINATIONS} SFTP destinations"
        )));
    }
    for (i, d) in dests.iter().enumerate() {
        let at = format!("destinations[{i}]");
        if d.host.trim().is_empty()
            || d.username.trim().is_empty()
            || d.remote_dir.trim().is_empty()
        {
            return Err(AppError::BadRequest(format!(
                "{at}: host, username and remote_dir are required"
            )));
        }
        match effective_auth(d) {
            "key" if d.private_key.trim().is_empty() => {
                return Err(AppError::BadRequest(format!(
                    "{at}: private_key is required for auth=key"
                )))
            }
            "password" if d.password.is_empty() => {
                return Err(AppError::BadRequest(format!(
                    "{at}: password is required for auth=password"
                )))
            }
            "key" | "password" => {}
            other => {
                return Err(AppError::BadRequest(format!(
                    "{at}: auth must be 'key' or 'password', got '{other}'"
                )))
            }
        }
        let fp = d.host_key_fingerprint.trim();
        if !fp.is_empty() && !fp.starts_with("SHA256:") {
            return Err(AppError::BadRequest(format!(
                "{at}: host_key_fingerprint must look like SHA256:… (ssh-keygen -lf)"
            )));
        }
    }
    Ok(())
}

fn effective_auth(d: &PgDumpDestination) -> &str {
    match d.auth.trim() {
        "" if !d.private_key.trim().is_empty() => "key",
        "" => "password",
        other => other,
    }
}

/// Editors send secrets back redacted; restore them from the stored destination that matches best.
/// Two destinations may share host and user (different directories or auth), so every identifying
/// field counts; position breaks ties and is the fallback when nothing matches.
pub fn keep_destination_secrets(
    incoming: &mut [PgDumpDestination],
    existing: &[PgDumpDestination],
) {
    fn score(a: &PgDumpDestination, b: &PgDumpDestination) -> u32 {
        if a.host != b.host || a.username != b.username {
            return 0;
        }
        1 + [
            a.port == b.port,
            a.remote_dir == b.remote_dir,
            a.name == b.name,
            effective_auth(a) == effective_auth(b),
        ]
        .iter()
        .filter(|m| **m)
        .count() as u32
    }
    for (i, d) in incoming.iter_mut().enumerate() {
        let prior = existing
            .iter()
            .enumerate()
            .map(|(j, e)| (score(d, e), std::cmp::Reverse(j.abs_diff(i)), e))
            .filter(|(s, _, _)| *s > 0)
            .max_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)))
            .map(|(_, _, e)| e)
            .or_else(|| existing.get(i));
        for (secret, old) in [
            (&mut d.password, prior.map(|p| p.password.clone())),
            (&mut d.private_key, prior.map(|p| p.private_key.clone())),
            (&mut d.passphrase, prior.map(|p| p.passphrase.clone())),
        ] {
            if secret == REDACTED {
                *secret = old.unwrap_or_default();
            }
        }
    }
}

/// For an ad-hoc send: a redacted secret is filled from a stored destination only when it is the
/// same SFTP login (host case-insensitive, port with 22 default, username, auth). Unlike
/// `keep_destination_secrets` there is no positional fallback: a changed login must re-enter it.
pub fn fill_send_secrets(
    d: &mut PgDumpDestination,
    existing: &[PgDumpDestination],
) -> Result<(), AppError> {
    let redacted = [&d.password, &d.private_key, &d.passphrase]
        .iter()
        .any(|s| s.as_str() == REDACTED);
    if !redacted {
        return Ok(());
    }
    let port = |p: &str| {
        if p.trim().is_empty() {
            "22".to_string()
        } else {
            p.trim().to_string()
        }
    };
    let prior = existing.iter().find(|e| {
        e.host.trim().eq_ignore_ascii_case(d.host.trim())
            && port(&e.port) == port(&d.port)
            && e.username.trim() == d.username.trim()
            && effective_auth(e) == effective_auth(d)
    });
    let Some(prior) = prior else {
        return Err(AppError::BadRequest(
            "SFTP host, port, username or sign-in method differs from the saved server; enter the password or key again".into(),
        ));
    };
    for (secret, old) in [
        (&mut d.password, &prior.password),
        (&mut d.private_key, &prior.private_key),
        (&mut d.passphrase, &prior.passphrase),
    ] {
        if secret == REDACTED {
            *secret = old.clone();
        }
    }
    Ok(())
}

fn check_conn(role: &str, conn: &SchemaCompareConn) -> Result<(), AppError> {
    if !is_postgres(&conn.db_type) {
        return Err(AppError::BadRequest(format!(
            "pgdump {role} must be PostgreSQL"
        )));
    }
    if conn.host.trim().is_empty() || conn.database.trim().is_empty() {
        return Err(AppError::BadRequest(format!(
            "{role}.host and {role}.database are required"
        )));
    }
    if conn.username.trim().is_empty() {
        return Err(AppError::BadRequest(format!("{role}.username is required")));
    }
    Ok(())
}

/// Each run is a new sidecar job; `profile_id` lets the sidecar refuse a second concurrent run.
pub fn profile_run_payload(profile: &PgDumpProfile) -> Value {
    let mut body = json!({
        "profile_id": profile.id.to_string(),
        "name": profile.name,
        "mode": profile.mode,
        "source": profile.source,
        "include_tables": profile.include_tables,
        "options": profile.options,
        "destinations": profile.destinations,
    });
    if profile.mode == MODE_DUMP_RESTORE {
        if let Some(target) = &profile.target {
            body["target"] = json!(target);
        }
    }
    body
}

fn base_url(cfg: &AppConfig) -> Result<&str, AppError> {
    cfg.pgdump_url
        .as_deref()
        .map(|u| u.trim_end_matches('/'))
        .ok_or_else(|| {
            AppError::BadRequest("pgdump sidecar is not configured (ORCH_PGDUMP_URL)".into())
        })
}

const PGDUMP_EXTRA_BAD: &[u16] = &[507];

/// Map a sidecar error status to the closest orchestrator error, keeping the sidecar's code
/// (409 already running, 429 at capacity, 507 low disk) in the message.
pub fn map_sidecar_error(status: u16, msg: &str) -> AppError {
    map_error_status("pgdump", "ORCH_PGDUMP_TOKEN", status, msg, PGDUMP_EXTRA_BAD)
}

fn access<'a>(cfg: &'a AppConfig, client: &'a Client) -> Result<SidecarAccess<'a>, AppError> {
    Ok(SidecarAccess {
        client,
        base_url: base_url(cfg)?,
        bearer_token: cfg.pgdump_token.as_deref(),
        tool: "pgdump",
        token_env: "ORCH_PGDUMP_TOKEN",
        extra_bad_request: PGDUMP_EXTRA_BAD,
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

pub async fn delete_job(cfg: &AppConfig, client: &Client, job_id: &str) -> Result<Value, AppError> {
    access(cfg, client)?.delete_job(job_id).await
}

/// Upload a finished job's file to one ad-hoc SFTP server (runs in the background on the sidecar).
pub async fn send_file(
    cfg: &AppConfig,
    client: &Client,
    job_id: &str,
    body: Value,
) -> Result<Value, AppError> {
    let path = format!("/v1/jobs/{}/send", encode(job_id));
    sidecar_request(cfg, client, Method::POST, &path, Some(body)).await
}

/// Open the dump file stream. Uses a dedicated client with only a connect timeout: the shared
/// client's total timeout (ORCH_COMMAND_TIMEOUT_SECS) would cut multi-GB downloads mid-stream.
pub async fn open_file(cfg: &AppConfig, job_id: &str) -> Result<reqwest::Response, AppError> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(format!("pgdump download client: {e}")))?;
    let url = format!("{}/v1/jobs/{}/file", base_url(cfg)?, encode(job_id));
    let mut req = client.get(&url);
    if let Some(token) = &cfg.pgdump_token {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| AppError::Internal(format!("pgdump auth header: {e}")))?;
        req = req.header(AUTHORIZATION, value);
    }
    let res = req
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("pgdump sidecar request failed: {e}")))?;
    let status = res.status();
    if status.is_success() {
        return Ok(res);
    }
    let text = res.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or(text);
    Err(map_sidecar_error(status.as_u16(), &msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(host: &str) -> SchemaCompareConn {
        SchemaCompareConn {
            db_type: "postgres".into(),
            host: host.into(),
            port: "5432".into(),
            database: "app".into(),
            username: "u".into(),
            password: "p".into(),
            schema: String::new(),
        }
    }

    #[test]
    fn dump_mode_needs_only_a_postgres_source() {
        assert!(validate_profile(MODE_DUMP, &pg("h"), None, None).is_ok());
        let mut mysql = pg("h");
        mysql.db_type = "mysql".into();
        assert!(validate_profile(MODE_DUMP, &mysql, None, None).is_err());
        assert!(validate_profile("restore", &pg("h"), None, None).is_err());
        let mut no_user = pg("h");
        no_user.username = " ".into();
        assert!(validate_profile(MODE_DUMP, &no_user, None, None).is_err());
    }

    #[test]
    fn dump_restore_needs_target_and_custom_format() {
        assert!(validate_profile(MODE_DUMP_RESTORE, &pg("a"), None, None).is_err());
        assert!(
            validate_profile(MODE_DUMP_RESTORE, &pg("a"), Some(&pg("b")), Some("plain")).is_err()
        );
        assert!(
            validate_profile(MODE_DUMP_RESTORE, &pg("a"), Some(&pg("b")), Some("custom")).is_ok()
        );
        assert!(validate_profile(MODE_DUMP_RESTORE, &pg("a"), Some(&pg("b")), None).is_ok());
    }

    fn dest(host: &str) -> PgDumpDestination {
        PgDumpDestination {
            name: host.into(),
            host: host.into(),
            port: "22".into(),
            username: "u".into(),
            auth: "password".into(),
            password: "pw".into(),
            remote_dir: "/backups".into(),
            ..Default::default()
        }
    }

    #[test]
    fn send_secrets_only_for_the_same_login() {
        let saved = vec![dest("backup.example.com")];
        let mut d = dest("BACKUP.example.com");
        d.password = REDACTED.into();
        d.remote_dir = "/elsewhere".into();
        fill_send_secrets(&mut d, &saved).unwrap();
        assert_eq!(d.password, "pw", "directory may differ; login is the same");

        let changes: [fn(&mut PgDumpDestination); 4] = [
            |d: &mut PgDumpDestination| d.host = "evil.example.com".into(),
            |d: &mut PgDumpDestination| d.port = "2222".into(),
            |d: &mut PgDumpDestination| d.username = "root".into(),
            |d: &mut PgDumpDestination| d.auth = "key".into(),
        ];
        for change in changes {
            let mut d = dest("backup.example.com");
            d.password = REDACTED.into();
            change(&mut d);
            assert!(fill_send_secrets(&mut d, &saved).is_err());
        }

        let mut typed = dest("new-host");
        typed.password = "typed".into();
        fill_send_secrets(&mut typed, &saved).unwrap();
        assert_eq!(
            typed.password, "typed",
            "typed secrets pass through untouched"
        );
    }

    #[test]
    fn destinations_are_validated() {
        assert!(validate_destinations(&[dest("a")]).is_ok());
        let mut no_pw = dest("a");
        no_pw.password.clear();
        assert!(validate_destinations(&[no_pw]).is_err());
        let mut key = dest("a");
        key.auth = "key".into();
        assert!(validate_destinations(&[key.clone()]).is_err());
        key.private_key = "-----BEGIN OPENSSH PRIVATE KEY-----".into();
        assert!(validate_destinations(&[key]).is_ok());
        let mut fp = dest("a");
        fp.host_key_fingerprint = "MD5:aa".into();
        assert!(validate_destinations(&[fp]).is_err());
        assert!(validate_destinations(&vec![dest("a"); 6]).is_err());
        let mut no_dir = dest("a");
        no_dir.remote_dir = " ".into();
        assert!(validate_destinations(&[no_dir]).is_err());
    }

    #[test]
    fn redacted_destination_secrets_are_kept_on_update() {
        let mut stored = dest("b");
        stored.private_key = "KEY-B".into();
        let existing = vec![dest("a"), stored];
        // Editor reordered the list and sent redacted values back.
        let mut incoming = vec![dest("b"), dest("a"), dest("new")];
        incoming[0].password = REDACTED.into();
        incoming[0].private_key = REDACTED.into();
        incoming[1].password = REDACTED.into();
        incoming[2].password = "fresh".into();
        keep_destination_secrets(&mut incoming, &existing);
        assert_eq!(incoming[0].password, "pw");
        assert_eq!(incoming[0].private_key, "KEY-B");
        assert_eq!(incoming[1].password, "pw");
        assert_eq!(incoming[2].password, "fresh");
    }

    #[test]
    fn same_host_and_user_destinations_keep_their_own_secrets() {
        let mut by_key = dest("h");
        by_key.name = "key".into();
        by_key.auth = "key".into();
        by_key.password.clear();
        by_key.private_key = "KEY".into();
        let mut by_pw = dest("h");
        by_pw.name = "pw".into();
        by_pw.password = "PW".into();
        by_pw.remote_dir = "/other".into();
        let existing = vec![by_key.clone(), by_pw.clone()];
        let mut incoming = existing.clone();
        incoming[0].private_key = REDACTED.into();
        incoming[1].password = REDACTED.into();
        incoming.swap(0, 1); // order changed in the editor
        keep_destination_secrets(&mut incoming, &existing);
        assert_eq!(incoming[0].password, "PW");
        assert_eq!(incoming[1].private_key, "KEY");
    }

    #[test]
    fn sidecar_errors_keep_their_code() {
        assert!(matches!(map_sidecar_error(404, "x"), AppError::NotFound(_)));
        assert!(
            matches!(map_sidecar_error(409, "running"), AppError::BadRequest(m) if m.contains("409"))
        );
        assert!(matches!(
            map_sidecar_error(507, "disk"),
            AppError::BadRequest(_)
        ));
        assert!(matches!(
            map_sidecar_error(500, "boom"),
            AppError::Upstream(_)
        ));
    }
}
