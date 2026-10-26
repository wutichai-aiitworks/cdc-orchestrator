//! Classify Kafka Connect connectors (source vs sink) and extract database connection metadata.

use std::collections::HashMap;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorKind {
    Source,
    Sink,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConnectorConnectionView {
    pub host: String,
    pub port: String,
    pub database: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jdbc_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConnectorInsight {
    pub kind: ConnectorKind,
    pub database_type: String,
    pub database_label: String,
    pub class_name: String,
    pub connection: ConnectorConnectionView,
    pub configured_schemas: Vec<String>,
    pub configured_tables: Vec<String>,
}

pub fn insight_from_config(config: &HashMap<String, String>) -> ConnectorInsight {
    let class_name = cfg(config, "connector.class");
    let kind = infer_kind(&class_name);
    let jdbc_url = nonempty(cfg(config, "connection.url"));
    let mut database_type = infer_database_type(&class_name, jdbc_url.as_deref());
    let parsed_jdbc = jdbc_url.as_deref().and_then(parse_jdbc_url);

    let mut host = first_nonempty(&[
        cfg(config, "database.hostname"),
        cfg(config, "database.host"),
        parsed_jdbc
            .as_ref()
            .map(|j| j.host.clone())
            .unwrap_or_default(),
    ]);
    let mut port = first_nonempty(&[
        cfg(config, "database.port"),
        parsed_jdbc
            .as_ref()
            .map(|j| j.port.clone())
            .unwrap_or_default(),
    ]);
    let mut database = first_nonempty(&[
        cfg(config, "database.dbname"),
        cfg(config, "database.names"),
        cfg(config, "database.include.list")
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string(),
        parsed_jdbc
            .as_ref()
            .map(|j| j.database.clone())
            .unwrap_or_default(),
    ]);
    let username = first_nonempty(&[
        cfg(config, "database.user"),
        cfg(config, "connection.username"),
        cfg(config, "database.username"),
    ]);
    let password = first_nonempty(&[
        cfg(config, "database.password"),
        cfg(config, "connection.password"),
    ]);

    if database_type == "unknown" {
        if let Some(from_url) = parsed_jdbc.as_ref().map(|j| j.database_type.clone()) {
            if !from_url.is_empty() {
                database_type = from_url;
            }
        }
    }
    if port.is_empty() {
        port = default_port(&database_type).to_string();
    }
    if host.is_empty() {
        if let Some(j) = &parsed_jdbc {
            host = j.host.clone();
        }
    }
    if database.is_empty() {
        if let Some(j) = &parsed_jdbc {
            database = j.database.clone();
        }
    }

    let mut configured_schemas = split_csv(&[
        cfg(config, "schema.include.list"),
        parsed_jdbc
            .as_ref()
            .and_then(|j| j.schema.clone())
            .unwrap_or_default(),
    ]);
    if configured_schemas.is_empty() && matches!(database_type.as_str(), "mysql" | "mariadb") {
        configured_schemas = split_csv(&[cfg(config, "database.include.list")]);
    }
    let configured_tables = split_csv(&[cfg(config, "table.include.list")]);

    ConnectorInsight {
        kind,
        database_label: database_label(&database_type),
        database_type,
        class_name,
        connection: ConnectorConnectionView {
            host,
            port,
            database,
            username,
            password,
            jdbc_url,
        },
        configured_schemas,
        configured_tables,
    }
}

pub fn connection_fields(insight: &ConnectorInsight) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("host".into(), insight.connection.host.clone());
    map.insert("port".into(), insight.connection.port.clone());
    map.insert("database".into(), insight.connection.database.clone());
    map.insert("username".into(), insight.connection.username.clone());
    map.insert("password".into(), insight.connection.password.clone());
    map
}

pub fn config_from_json(value: &serde_json::Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(obj) = value.as_object() else {
        return map;
    };
    for (k, v) in obj {
        let s = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => continue,
            other => other.to_string().trim_matches('"').to_string(),
        };
        map.insert(k.clone(), s);
    }
    map
}

fn infer_kind(class_name: &str) -> ConnectorKind {
    let c = class_name.to_ascii_lowercase();
    if c.contains("sink") || c.contains(".jdbc.jdbcsinkconnector") {
        ConnectorKind::Sink
    } else {
        ConnectorKind::Source
    }
}

fn infer_database_type(class_name: &str, jdbc_url: Option<&str>) -> String {
    let c = class_name.to_ascii_lowercase();
    if c.contains("postgresql") || c.contains("postgres") {
        return "postgres".into();
    }
    if c.contains("mariadb") {
        return "mariadb".into();
    }
    if c.contains("mysql") {
        return "mysql".into();
    }
    if c.contains("sqlserver") {
        return "sqlserver".into();
    }
    if c.contains("oracle") {
        return "oracle".into();
    }
    if let Some(url) = jdbc_url {
        if let Some(parsed) = parse_jdbc_url(url) {
            return parsed.database_type;
        }
    }
    "unknown".into()
}

pub fn database_label(database_type: &str) -> String {
    match database_type {
        "postgres" | "postgresql" => "PostgreSQL".into(),
        "mysql" => "MySQL".into(),
        "mariadb" => "MariaDB".into(),
        "sqlserver" => "SQL Server".into(),
        "oracle" => "Oracle".into(),
        _ => "Database".into(),
    }
}

pub fn default_port(database_type: &str) -> &'static str {
    match database_type {
        "mysql" | "mariadb" => "3306",
        "sqlserver" => "1433",
        "oracle" => "1521",
        _ => "5432",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedJdbc {
    database_type: String,
    host: String,
    port: String,
    database: String,
    schema: Option<String>,
}

fn parse_jdbc_url(raw: &str) -> Option<ParsedJdbc> {
    let url = raw.trim();
    if url.is_empty() {
        return None;
    }
    if let Some(rest) = url.strip_prefix("jdbc:postgresql://") {
        return parse_host_path_jdbc(rest, "postgres", "5432");
    }
    if let Some(rest) = url.strip_prefix("jdbc:mysql://") {
        return parse_host_path_jdbc(rest, "mysql", "3306");
    }
    if let Some(rest) = url.strip_prefix("jdbc:mariadb://") {
        return parse_host_path_jdbc(rest, "mariadb", "3306");
    }
    if let Some(rest) = url.strip_prefix("jdbc:sqlserver://") {
        let (hostport, params) = rest.split_once(';').unwrap_or((rest, ""));
        let (host, port) = split_host_port(hostport, "1433");
        let mut database = String::new();
        for part in params.split(';') {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            if k.eq_ignore_ascii_case("databaseName") || k.eq_ignore_ascii_case("database") {
                database = v.to_string();
            }
        }
        return Some(ParsedJdbc {
            database_type: "sqlserver".into(),
            host,
            port,
            database,
            schema: None,
        });
    }
    if let Some(rest) = url.strip_prefix("jdbc:oracle:thin:@//") {
        let (hostport, db) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port) = split_host_port(hostport, "1521");
        return Some(ParsedJdbc {
            database_type: "oracle".into(),
            host,
            port,
            database: db.to_string(),
            schema: None,
        });
    }
    if let Some(rest) = url.strip_prefix("jdbc:oracle:thin:@") {
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() >= 3 {
            return Some(ParsedJdbc {
                database_type: "oracle".into(),
                host: parts[0].to_string(),
                port: parts[1].to_string(),
                database: parts[2].to_string(),
                schema: None,
            });
        }
    }
    None
}

fn parse_host_path_jdbc(rest: &str, database_type: &str, default_port: &str) -> Option<ParsedJdbc> {
    let (authority_and_path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (hostport, path) = authority_and_path
        .split_once('/')
        .unwrap_or((authority_and_path, ""));
    let (host, port) = split_host_port(hostport, default_port);
    let database = path.split('/').next().unwrap_or("").to_string();
    let mut schema = None;
    for part in query.split('&') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        if k.eq_ignore_ascii_case("currentSchema") || k.eq_ignore_ascii_case("current_schema") {
            schema = Some(v.to_string());
        }
    }
    Some(ParsedJdbc {
        database_type: database_type.into(),
        host,
        port,
        database,
        schema,
    })
}

fn split_host_port(hostport: &str, default_port: &str) -> (String, String) {
    if let Some((h, p)) = hostport.rsplit_once(':') {
        if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
            return (h.trim_matches(['[', ']']).to_string(), p.to_string());
        }
    }
    (
        hostport.trim_matches(['[', ']']).to_string(),
        default_port.to_string(),
    )
}

fn cfg(config: &HashMap<String, String>, key: &str) -> String {
    config
        .get(key)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn nonempty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn first_nonempty(values: &[String]) -> String {
    values
        .iter()
        .find(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string())
        .unwrap_or_default()
}

fn split_csv(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        for part in value.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() && !out.iter().any(|existing: &String| existing == trimmed) {
                out.push(trimmed.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn classifies_postgres_source() {
        let insight = insight_from_config(&map(&[
            (
                "connector.class",
                "io.debezium.connector.postgresql.PostgresConnector",
            ),
            ("database.hostname", "cdc-postgres-source"),
            ("database.port", "5432"),
            ("database.user", "demo"),
            ("database.password", "demo"),
            ("database.dbname", "demo"),
            ("schema.include.list", "bookings"),
        ]));
        assert_eq!(insight.kind, ConnectorKind::Source);
        assert_eq!(insight.database_type, "postgres");
        assert_eq!(insight.database_label, "PostgreSQL");
        assert_eq!(insight.connection.host, "cdc-postgres-source");
        assert_eq!(insight.configured_schemas, vec!["bookings".to_string()]);
    }

    #[test]
    fn classifies_jdbc_sink_from_url() {
        let insight = insight_from_config(&map(&[
            (
                "connector.class",
                "io.debezium.connector.jdbc.JdbcSinkConnector",
            ),
            (
                "connection.url",
                "jdbc:postgresql://cdc-postgres-target:5432/demo?currentSchema=bookings",
            ),
            ("connection.username", "demo"),
            ("connection.password", "secret"),
        ]));
        assert_eq!(insight.kind, ConnectorKind::Sink);
        assert_eq!(insight.database_type, "postgres");
        assert_eq!(insight.connection.host, "cdc-postgres-target");
        assert_eq!(insight.connection.port, "5432");
        assert_eq!(insight.connection.database, "demo");
        assert_eq!(insight.configured_schemas, vec!["bookings".to_string()]);
        assert_eq!(insight.connection.password, "secret");
    }

    #[test]
    fn parses_mysql_and_sqlserver_jdbc() {
        let mysql = parse_jdbc_url("jdbc:mysql://db:3306/orders").unwrap();
        assert_eq!(mysql.database_type, "mysql");
        assert_eq!(mysql.host, "db");
        assert_eq!(mysql.database, "orders");
        let mssql =
            parse_jdbc_url("jdbc:sqlserver://sql:1433;databaseName=erp;encrypt=false").unwrap();
        assert_eq!(mssql.database_type, "sqlserver");
        assert_eq!(mssql.host, "sql");
        assert_eq!(mssql.database, "erp");
    }
}
