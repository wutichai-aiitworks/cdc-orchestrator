type RuntimeCfg = ReturnType<typeof useRuntimeConfig>

export type AuthMode = "none" | "sso"

/**
 * Same switch as hyper-dataservice-webui: AUTH_MODE = none | sso
 * CDC has no local gateway login.
 */
export function getAuthMode(cfg: RuntimeCfg): AuthMode {
  const fromEnv = import.meta.server ? process.env.AUTH_MODE : ""
  const mode = String(fromEnv || cfg.public.authMode || "none").trim().toLowerCase()
  if (mode === "sso") return "sso"
  return "none"
}

export function isAuthDisabled(cfg: RuntimeCfg): boolean {
  return getAuthMode(cfg) === "none"
}

export function isSsoAuth(cfg: RuntimeCfg): boolean {
  return getAuthMode(cfg) === "sso"
}
