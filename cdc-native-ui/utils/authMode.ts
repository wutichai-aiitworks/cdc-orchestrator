type RuntimeCfg = ReturnType<typeof useRuntimeConfig>

export type AuthMode = "none" | "local" | "sso"

/**
 * AUTH_MODE = none | local | sso
 * - none:  open (dev only)
 * - local: standalone username/password (ADMIN_USERNAME / ADMIN_PASSWORD, default admin/admin)
 * - sso:   SuperTokens gateway or Authelia
 */
export function getAuthMode(cfg: RuntimeCfg): AuthMode {
  const fromEnv = import.meta.server ? process.env.AUTH_MODE : ""
  const mode = String(fromEnv || cfg.public.authMode || "none").trim().toLowerCase()
  if (mode === "sso") return "sso"
  if (mode === "local") return "local"
  return "none"
}

export function isAuthDisabled(cfg: RuntimeCfg): boolean {
  return getAuthMode(cfg) === "none"
}

export function isSsoAuth(cfg: RuntimeCfg): boolean {
  return getAuthMode(cfg) === "sso"
}

export function isLocalAuth(cfg: RuntimeCfg): boolean {
  return getAuthMode(cfg) === "local"
}
