// Deployment mode (docs/patterns/deployment-mode). Keep APP_MODE; rename APP_REQUIRE_* and the retired names to this repo's.
// Reference: hyper cdc-orchestrator src/config.rs (AppMode).
use std::env;

/// APP_MODE — the one deployment-mode variable: Production (default) | Maintenance | Enhanced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMode {
    /// Live system in normal service. No UI ribbon; hardened by default.
    Production,
    /// Stable release, fixes only. Amber ribbon.
    Maintenance,
    /// Release with new features in development. Violet ribbon.
    Enhanced,
}

impl AppMode {
    /// Case-insensitive, common spellings. Unknown or empty → None.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "production" | "prod" => Some(Self::Production),
            "maintenance" | "maintainance" | "maint" => Some(Self::Maintenance),
            "enhanced" | "enhance" | "enhancement" | "enhancements" => Some(Self::Enhanced),
            _ => None,
        }
    }

    /// APP_MODE, else the retired APP_ENV (dev → Enhanced, prod → Production), else Production.
    pub fn resolve(mode: Option<&str>, legacy_env: Option<&str>) -> Self {
        if let Some(m) = mode.and_then(Self::parse) {
            return m;
        }
        match legacy_env.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("dev" | "development") => Self::Enhanced,
            _ => Self::Production,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Maintenance => "maintenance",
            Self::Enhanced => "enhanced",
        }
    }

    /// Default for APP_REQUIRE_AUTH / APP_REQUIRE_PERSISTENCE / APP_STRICT_PERSISTENCE when unset.
    pub fn hardened_by_default(&self) -> bool {
        *self == Self::Production
    }
}

/// Unset or empty → None. Compose's `${VAR:-}` yields "", which must mean "default", not false.
pub fn env_truthy(key: &str) -> Option<bool> {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

pub struct ModeConfig {
    pub mode: AppMode,
    pub require_auth: bool,
    pub require_persistence: bool,
    pub strict_persistence: bool,
    /// The retired APP_ENV, if still set; warn once at startup.
    pub legacy_env: Option<String>,
}

impl ModeConfig {
    pub fn from_env() -> Self {
        let non_empty = |k: &str| env::var(k).ok().filter(|v| !v.trim().is_empty());
        // The retired environment variable in this repo (ORCH_ENV in the reference) — rename it.
        let legacy_env = non_empty("APP_ENV");
        let mode = AppMode::resolve(non_empty("APP_MODE").as_deref(), legacy_env.as_deref());
        let hardened = mode.hardened_by_default();
        Self {
            mode,
            require_auth: env_truthy("APP_REQUIRE_AUTH").unwrap_or(hardened),
            require_persistence: env_truthy("APP_REQUIRE_PERSISTENCE").unwrap_or(hardened),
            strict_persistence: env_truthy("APP_STRICT_PERSISTENCE").unwrap_or(hardened),
            legacy_env,
        }
    }

    /// Call once at startup, after tracing is initialised.
    pub fn log_startup(&self) {
        tracing::info!(
            mode = self.mode.as_str(),
            require_auth = self.require_auth,
            require_persistence = self.require_persistence,
            "starting"
        );
        if let Some(v) = &self.legacy_env {
            tracing::warn!(
                app_env = %v,
                mode = self.mode.as_str(),
                "APP_ENV is retired: use APP_MODE=Production|Maintenance|Enhanced only"
            );
        }
    }
}
