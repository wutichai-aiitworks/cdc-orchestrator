//! Saved passwords in tool profiles (pgdump, pgcopy, pgloader, mongocopy, redisshake).
//!
//! The API returns `********` instead of a stored password, and editors send it back to mean
//! "unchanged". That marker may only stand for the stored password of the *same login*: if the
//! host, port, username or engine changed, using the stored password would send it to a server it
//! was never meant for (an edited host on the next run, or a connection test).

use crate::error::AppError;
use crate::models::SchemaCompareConn;

pub const REDACTED: &str = "********";

/// Replace a redacted password with the stored one when the login is unchanged.
/// A changed login with a stored password is a 400 ("enter the password again"); with no stored
/// password the marker becomes an empty password (never sent as a literal).
pub fn keep_password(
    role: &str,
    incoming: &mut SchemaCompareConn,
    existing: Option<&SchemaCompareConn>,
) -> Result<(), AppError> {
    if incoming.password != REDACTED {
        return Ok(());
    }
    match existing {
        Some(c) if same_login(incoming, c) => {
            incoming.password = c.password.clone();
            Ok(())
        }
        Some(c) if !c.password.is_empty() => Err(AppError::BadRequest(format!(
            "{role} host, port or username changed; enter the {role} password again"
        ))),
        _ => {
            incoming.password.clear();
            Ok(())
        }
    }
}

/// A `********` password sent by the Schema Compare page (its saved profiles are returned
/// redacted) is replaced by the stored password of a saved compare profile **with the same login**
/// (engine, host, port, username). No match → 400, so the marker is never sent to a database.
pub fn restore_from_saved(
    role: &str,
    incoming: &mut SchemaCompareConn,
    saved: &[crate::models::SchemaCompareProfile],
) -> Result<(), AppError> {
    if incoming.password != REDACTED {
        return Ok(());
    }
    let found = saved
        .iter()
        .flat_map(|p| [&p.source, &p.target])
        .find(|c| !c.password.is_empty() && c.password != REDACTED && same_login(incoming, c));
    match found {
        Some(c) => {
            incoming.password = c.password.clone();
            Ok(())
        }
        None => Err(AppError::BadRequest(format!(
            "no saved {role} password for this host, port and username; enter the {role} password again"
        ))),
    }
}

/// [`restore_from_saved`] for a connection map (`host`, `port`, `username`, `password`).
pub fn restore_map_from_saved(
    role: &str,
    db_type: &str,
    conn: &mut std::collections::HashMap<String, String>,
    saved: &[crate::models::SchemaCompareProfile],
) -> Result<(), AppError> {
    if conn.get("password").map(String::as_str) != Some(REDACTED) {
        return Ok(());
    }
    let get = |k: &str| conn.get(k).cloned().unwrap_or_default();
    let mut c = SchemaCompareConn {
        db_type: db_type.to_string(),
        host: get("host"),
        port: get("port"),
        database: get("database"),
        username: {
            let u = get("username");
            if u.is_empty() {
                get("user")
            } else {
                u
            }
        },
        password: REDACTED.to_string(),
        schema: String::new(),
        tunnel_id: None,
        ssh_tunnel_id: None,
    };
    restore_from_saved(role, &mut c, saved)?;
    conn.insert("password".into(), c.password);
    Ok(())
}

pub fn same_login(a: &SchemaCompareConn, b: &SchemaCompareConn) -> bool {
    engine(&a.db_type) == engine(&b.db_type)
        && a.host.trim().eq_ignore_ascii_case(b.host.trim())
        && port_or_default(&a.port, &a.db_type) == port_or_default(&b.port, &b.db_type)
        && a.username.trim() == b.username.trim()
}

fn engine(db_type: &str) -> String {
    match db_type.trim().to_ascii_lowercase().as_str() {
        "" | "postgres" | "postgresql" | "pg" => "postgres".into(),
        "mariadb" => "mysql".into(),
        "sqlserver" => "mssql".into(),
        "mongo" => "mongodb".into(),
        other => other.into(),
    }
}

fn port_or_default(port: &str, db_type: &str) -> String {
    let p = port.trim();
    if !p.is_empty() {
        return p.to_string();
    }
    match engine(db_type).as_str() {
        "postgres" => "5432",
        "mysql" => "3306",
        "mssql" => "1433",
        "mongodb" => "27017",
        "redis" => "6379",
        _ => "",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(
        db_type: &str,
        host: &str,
        port: &str,
        user: &str,
        password: &str,
    ) -> SchemaCompareConn {
        SchemaCompareConn {
            tunnel_id: None,
            ssh_tunnel_id: None,
            db_type: db_type.into(),
            host: host.into(),
            port: port.into(),
            database: "shop".into(),
            username: user.into(),
            password: password.into(),
            schema: String::new(),
        }
    }

    fn saved(
        source: SchemaCompareConn,
        target: SchemaCompareConn,
    ) -> crate::models::SchemaCompareProfile {
        crate::models::SchemaCompareProfile {
            id: uuid::Uuid::new_v4(),
            name: "p".into(),
            source,
            target,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn compare_page_marker_restored_from_a_saved_profile_with_the_same_login() {
        let list = vec![saved(
            conn("postgres", "db1", "5432", "app", "s3cret"),
            conn("postgres", "db2", "", "app", "t4rget"),
        )];
        // Source side matches profile.source; target side (default port) matches profile.target.
        let mut a = conn("postgres", "DB1", "5432", "app", REDACTED);
        restore_from_saved("source", &mut a, &list).unwrap();
        assert_eq!(a.password, "s3cret");
        let mut b = conn("postgres", "db2", "5432", "app", REDACTED);
        restore_from_saved("target", &mut b, &list).unwrap();
        assert_eq!(b.password, "t4rget");
        // Another host, user or engine never gets a stored password.
        for other in [
            conn("postgres", "evil", "5432", "app", REDACTED),
            conn("postgres", "db1", "5432", "root", REDACTED),
            conn("mysql", "db1", "5432", "app", REDACTED),
        ] {
            let mut c = other;
            assert!(restore_from_saved("source", &mut c, &list).is_err());
            assert_eq!(c.password, REDACTED);
        }
        // A real password is left alone.
        let mut d = conn("postgres", "evil", "5432", "app", "typed");
        restore_from_saved("source", &mut d, &list).unwrap();
        assert_eq!(d.password, "typed");
    }

    #[test]
    fn compare_marker_restored_in_a_connection_map() {
        let list = vec![saved(
            conn("postgres", "db1", "5432", "app", "s3cret"),
            conn("postgres", "x", "5432", "u", ""),
        )];
        let mut m: std::collections::HashMap<String, String> = [
            ("host", "db1"),
            ("port", "5432"),
            ("username", "app"),
            ("password", REDACTED),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        restore_map_from_saved("source", "postgres", &mut m, &list).unwrap();
        assert_eq!(m["password"], "s3cret");
        m.insert("host".into(), "other".into());
        m.insert("password".into(), REDACTED.into());
        assert!(restore_map_from_saved("source", "postgres", &mut m, &list).is_err());
    }

    #[test]
    fn redacted_password_restored_only_for_same_login() {
        let saved = conn("postgres", "db-1", "5432", "reader", "secret");

        let mut same = conn("postgresql", "DB-1 ", "", "reader", REDACTED);
        keep_password("source", &mut same, Some(&saved)).unwrap();
        assert_eq!(
            same.password, "secret",
            "case/space/default port/alias are the same login"
        );

        for moved in [
            conn("postgres", "evil.example", "5432", "reader", REDACTED),
            conn("postgres", "db-1", "6543", "reader", REDACTED),
            conn("postgres", "db-1", "5432", "admin", REDACTED),
            conn("mysql", "db-1", "5432", "reader", REDACTED),
        ] {
            let mut moved = moved;
            let err = keep_password("source", &mut moved, Some(&saved)).unwrap_err();
            assert!(
                err.to_string().contains("enter the source password again"),
                "{err}"
            );
        }
    }

    #[test]
    fn default_ports_follow_the_engine() {
        let saved = conn("redis", "cache", "6379", "", "pw");
        let mut same = conn("redis", "cache", "", "", REDACTED);
        keep_password("target", &mut same, Some(&saved)).unwrap();
        assert_eq!(same.password, "pw");

        let saved = conn("mongodb", "m", "27017", "app", "pw");
        let mut other_port = conn("mongodb", "m", "27018", "app", REDACTED);
        assert!(keep_password("source", &mut other_port, Some(&saved)).is_err());
    }

    #[test]
    fn new_password_and_missing_saved_secret() {
        let saved = conn("postgres", "db-1", "5432", "reader", "secret");
        let mut typed = conn("postgres", "elsewhere", "5432", "reader", "typed");
        keep_password("source", &mut typed, Some(&saved)).unwrap();
        assert_eq!(typed.password, "typed");

        let mut none_saved = conn("postgres", "db-1", "5432", "reader", REDACTED);
        keep_password("target", &mut none_saved, None).unwrap();
        assert_eq!(
            none_saved.password, "",
            "a redaction marker is never sent as a password"
        );

        // A login that never had a password may change freely.
        let open = conn("redis", "cache", "6379", "", "");
        let mut moved = conn("redis", "cache-2", "6379", "", REDACTED);
        keep_password("source", &mut moved, Some(&open)).unwrap();
        assert_eq!(moved.password, "");
    }
}
