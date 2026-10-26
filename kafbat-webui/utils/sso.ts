type RuntimeCfg = ReturnType<typeof useRuntimeConfig>

function envOr(name: string, fallback: unknown): string {
  const fromEnv = import.meta.server ? process.env[name] : ""
  return String(fromEnv || fallback || "").trim()
}

export function getSsoProvider(cfg: RuntimeCfg): "supertokens_gateway" | "authelia" {
  const value = envOr("SSO_PROVIDER", cfg.public.ssoProvider || "supertokens_gateway").toLowerCase()
  return value === "authelia" ? "authelia" : "supertokens_gateway"
}

export function sanitizeNextPath(raw: unknown, fallback = "/"): string {
  if (typeof raw !== "string") return fallback
  const value = raw.trim()
  if (!value.startsWith("/") || value.startsWith("//") || value.includes("\\")) return fallback
  if (value.startsWith("/api/")) return fallback
  if (value.startsWith("/auth/callback") || value.startsWith("/api/auth/")) return fallback
  if (value === "/auth/login" || value.startsWith("/auth/login?")) return fallback
  if (value === "/login" || value.startsWith("/login?")) return fallback
  if (value.length > 512) return fallback
  return value
}

export function buildSupertokensLoginUrl(cfg: RuntimeCfg): string {
  const appId = envOr("SSO_APP_ID", cfg.public.ssoAppId)
  const loginUrl = envOr("SSO_LOGIN_URL", cfg.public.ssoLoginUrl)
  const callbackUrl = envOr("SSO_CALLBACK_URL", cfg.public.ssoCallbackUrl)
  if (!appId || !loginUrl || !callbackUrl) return ""
  const shared = new URL(loginUrl)
  shared.searchParams.set("app_id", appId)
  shared.searchParams.set("return_url", callbackUrl)
  return shared.toString()
}

export function buildSsoStartPath(next = "/", extras: Record<string, string> = {}): string {
  const params = new URLSearchParams({ next: sanitizeNextPath(next) })
  for (const [key, value] of Object.entries(extras)) {
    if (value) params.set(key, value)
  }
  return `/api/auth/sso/start?${params.toString()}`
}
