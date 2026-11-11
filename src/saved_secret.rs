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
            db_type: db_type.into(),
            host: host.into(),
            port: port.into(),
            database: "shop".into(),
            username: user.into(),
            password: password.into(),
            schema: String::new(),
        }
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
