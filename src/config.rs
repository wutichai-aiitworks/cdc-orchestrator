use std::collections::HashMap;
use std::env;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvMode {
    Dev,
    Prod,
}

impl EnvMode {
    fn from_env(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "prod" | "production" => Self::Prod,
            _ => Self::Dev,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Prod => "prod",
        }
    }
}

#[derive(Clone, Debug)]
pub enum EngineProvider {
    DockerApi,
    Kubernetes,
}

impl EngineProvider {
    fn from_env(value: String) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "kubernetes" => Self::Kubernetes,
            _ => Self::DockerApi,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DockerApi => "docker_api",
            Self::Kubernetes => "kubernetes",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub enabled: bool,
    pub env_mode: EnvMode,
    pub engine_provider: EngineProvider,
    pub container_project_name: String,
    pub managed_containers: Vec<String>,
    pub connect_base_url: String,
    pub redpanda_container: String,
    pub schema_registry_url: Option<String>,
    pub ksql_url: Option<String>,
    pub command_timeout_secs: u64,
    pub api_token: Option<String>,
    pub jdbc_admin_url: Option<String>,
    pub jdbc_admin_token: Option<String>,
    pub pgcopy_url: Option<String>,
    pub pgcopy_token: Option<String>,
    pub redis_shake_url: Option<String>,
    pub redis_shake_token: Option<String>,
    pub pgloader_url: Option<String>,
    pub pgloader_token: Option<String>,
    pub mongocopy_url: Option<String>,
    pub mongocopy_token: Option<String>,
    pub rqlite_url: Option<String>,
    pub rqlite_user: Option<String>,
    pub rqlite_pass: Option<String>,
    pub docker_socket_path: String,
    pub log_containers: HashMap<String, String>,
    pub poll_interval: Duration,
    /// How often the changelog pruner runs (seconds). Per-profile retention window
    /// is set on the source config; this only controls the sweep cadence.
    pub changelog_prune_interval_secs: u64,
    pub history_capacity: usize,
    pub stop_deletes_connector: bool,
    /// When true, refuse to start without a strong API token.
    pub require_auth: bool,
    /// When true, refuse to start / operate without rqlite.
    pub require_persistence: bool,
    /// When true, rqlite read failures do not silently fall back to memory.
    pub strict_persistence: bool,
    pub cors_origins: Vec<String>,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let env_mode = EnvMode::from_env(&env::var("ORCH_ENV").unwrap_or_else(|_| "dev".into()));
        let host = env::var("ORCH_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("ORCH_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(9788);
        let enabled = env::var("ORCH_ENABLED")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(true);
        let engine_provider = EngineProvider::from_env(
            env::var("ORCH_ENGINE_PROVIDER").unwrap_or_else(|_| "docker_api".to_string()),
        );
        let container_project_name =
            env::var("ORCH_CONTAINER_PROJECT_NAME").unwrap_or_else(|_| "cdc-stack".to_string());
        let managed_containers = env::var("ORCH_MANAGED_CONTAINERS")
            .unwrap_or_else(|_| {
                "cdc-message,cdc-admin,cdc-connect,cdc-ksqldb,cdc-redis,cdc-vector".to_string()
            })
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let connect_base_url = env::var("ORCH_CONNECT_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:28083".to_string());
        let redpanda_container =
            env::var("ORCH_REDPANDA_CONTAINER").unwrap_or_else(|_| "cdc-message".to_string());
        let schema_registry_url = env::var("ORCH_SCHEMA_REGISTRY_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| Some("http://cdc-message:8081".to_string()));
        let ksql_url = env::var("ORCH_KSQL_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());
        let command_timeout_secs = env::var("ORCH_COMMAND_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);
        let api_token = env::var("ORCH_API_TOKEN")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());

        let jdbc_admin_url = env::var("ORCH_JDBC_ADMIN_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());
        let jdbc_admin_token = env::var("ORCH_JDBC_ADMIN_TOKEN")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());

        let pgcopy_url = env::var("ORCH_PGCOPY_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());
        let pgcopy_token = env::var("ORCH_PGCOPY_TOKEN")
            .ok()
            .or_else(|| env::var("PGCOPY_API_TOKEN").ok())
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());

        let redis_shake_url = env::var("ORCH_REDIS_SHAKE_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());
        let redis_shake_token = env::var("ORCH_REDIS_SHAKE_TOKEN")
            .ok()
            .or_else(|| env::var("REDIS_SHAKE_API_TOKEN").ok())
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());

        let pgloader_url = env::var("ORCH_PGLOADER_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());
        let pgloader_token = env::var("ORCH_PGLOADER_TOKEN")
            .ok()
            .or_else(|| env::var("PGLOADER_API_TOKEN").ok())
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());

        let mongocopy_url = env::var("ORCH_MONGOCOPY_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());
        let mongocopy_token = env::var("ORCH_MONGOCOPY_TOKEN")
            .ok()
            .or_else(|| env::var("MONGOCOPY_API_TOKEN").ok())
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());

        let rqlite_url = env::var("ORCH_RQLITE_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());
        let rqlite_user = env::var("ORCH_RQLITE_USER")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());
        let rqlite_pass = env::var("ORCH_RQLITE_PASS")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());

        let docker_socket_path = env::var("ORCH_DOCKER_SOCKET_PATH")
            .or_else(|_| env::var("DOCKER_SOCKET_PATH"))
            .unwrap_or_else(|_| "/var/run/docker.sock".to_string());

        let mut log_containers = HashMap::new();
        log_containers.insert(
            "connect".to_string(),
            env::var("ORCH_CONTAINER_CONNECT").unwrap_or_else(|_| "cdc-connect".to_string()),
        );
        log_containers.insert(
            "kafka".to_string(),
            env::var("ORCH_CONTAINER_KAFKA").unwrap_or_else(|_| redpanda_container.clone()),
        );
        log_containers.insert(
            "vector".to_string(),
            env::var("ORCH_CONTAINER_VECTOR").unwrap_or_else(|_| "cdc-vector".to_string()),
        );
        log_containers.insert(
            "redis".to_string(),
            env::var("ORCH_CONTAINER_REDIS").unwrap_or_else(|_| "cdc-redis".to_string()),
        );

        let poll_interval_ms = env::var("ORCH_POLL_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30_000)
            .max(1);
        let changelog_prune_interval_secs = env::var("ORCH_CHANGELOG_PRUNE_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3_600)
            .max(60);
        let history_capacity = env::var("ORCH_HISTORY_CAPACITY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(500)
            .max(1);
        let stop_deletes_connector = env::var("ORCH_STOP_DELETES_CONNECTOR")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);

        // Prod defaults: require auth + persistence. Dev defaults: optional.
        let require_auth = env_truthy("ORCH_REQUIRE_AUTH").unwrap_or(env_mode == EnvMode::Prod);
        let require_persistence =
            env_truthy("ORCH_REQUIRE_PERSISTENCE").unwrap_or(env_mode == EnvMode::Prod);
        let strict_persistence =
            env_truthy("ORCH_STRICT_PERSISTENCE").unwrap_or(env_mode == EnvMode::Prod);

        let cors_origins = env::var("ORCH_CORS_ORIGINS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();

        Self {
            host,
            port,
            enabled,
            env_mode,
            engine_provider,
            container_project_name,
            managed_containers,
            connect_base_url,
            redpanda_container,
            schema_registry_url,
            ksql_url,
            command_timeout_secs,
            api_token,
            jdbc_admin_url,
            jdbc_admin_token,
            pgcopy_url,
            pgcopy_token,
            redis_shake_url,
            redis_shake_token,
            pgloader_url,
            pgloader_token,
            mongocopy_url,
            mongocopy_token,
            rqlite_url,
            rqlite_user,
            rqlite_pass,
            docker_socket_path,
            log_containers,
            poll_interval: Duration::from_millis(poll_interval_ms),
            changelog_prune_interval_secs,
            history_capacity,
            stop_deletes_connector,
            require_auth,
            require_persistence,
            strict_persistence,
            cors_origins,
        }
    }

    pub fn persistence_enabled(&self) -> bool {
        self.rqlite_url.is_some()
    }

    /// Fail fast for production misconfiguration.
    pub fn validate_or_die(&self) {
        if self.require_auth {
            match &self.api_token {
                None => {
                    eprintln!(
                        "FATAL: ORCH_REQUIRE_AUTH/ORCH_ENV=prod requires ORCH_API_TOKEN (min 24 chars)"
                    );
                    std::process::exit(1);
                }
                Some(tok) if is_weak_token(tok) => {
                    eprintln!(
                        "FATAL: ORCH_API_TOKEN is weak/placeholder; use a strong secret (>=24 chars)"
                    );
                    std::process::exit(1);
                }
                Some(_) => {}
            }
        } else if self.api_token.is_none() {
            tracing::warn!("ORCH_API_TOKEN unset — all non-health routes are open (dev only)");
        } else if self
            .api_token
            .as_ref()
            .map(|t| is_weak_token(t))
            .unwrap_or(false)
        {
            tracing::warn!("ORCH_API_TOKEN looks weak — do not use in production");
        }

        if self.require_persistence {
            if self.rqlite_url.is_none() {
                eprintln!("FATAL: ORCH_REQUIRE_PERSISTENCE/ORCH_ENV=prod requires ORCH_RQLITE_URL");
                std::process::exit(1);
            }
        } else if !self.persistence_enabled() {
            tracing::warn!(
                "ORCH_RQLITE_URL unset — profiles/rules are memory-only and lost on restart"
            );
        }

        if self.host == "0.0.0.0" && self.env_mode == EnvMode::Prod {
            tracing::warn!(
                "ORCH_HOST=0.0.0.0 in prod — prefer binding to a private interface or publish compose ports as 127.0.0.1 only"
            );
        }
    }
}

fn env_truthy(key: &str) -> Option<bool> {
    env::var(key).ok().map(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub fn is_weak_token(v: &str) -> bool {
    let t = v.trim();
    t.len() < 24
        || t.eq_ignore_ascii_case("replace-with-secure-token")
        || t.eq_ignore_ascii_case("changeme")
        || t.eq_ignore_ascii_case("secret")
        || t.eq_ignore_ascii_case("password")
}
