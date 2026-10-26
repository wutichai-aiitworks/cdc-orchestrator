type RuntimeCfg = ReturnType<typeof useRuntimeConfig>

interface CachedMachineToken {
  token: string
  expiresAtMs: number
}

export const SSO_MACHINE_TOKEN_ERROR_SOURCE = "sso_machine_token"
const EXPIRY_SKEW_MS = 30_000

let cached: CachedMachineToken | null = null
let inflight: Promise<string> | null = null

export function resolveSsoTokenUrl(cfg: RuntimeCfg): string {
  const exchangeUrl = String(cfg.ssoExchangeUrl || "").trim()
  if (!exchangeUrl) return ""
  try {
    return new URL("/auth/token", exchangeUrl).toString()
  } catch {
    return ""
  }
}

export function isSsoMachineAuthConfigured(cfg: RuntimeCfg): boolean {
  return Boolean(String(cfg.public.ssoAppId || "").trim() && String(cfg.ssoAppSecret || "").trim())
}

export function invalidateSsoMachineToken(): void {
  cached = null
}

export function isSsoMachineTokenError(err: any): boolean {
  return err?.data?.source === SSO_MACHINE_TOKEN_ERROR_SOURCE
}

function machineTokenError(statusCode: number, statusMessage: string, message: string) {
  return createError({ statusCode, statusMessage, message, data: { source: SSO_MACHINE_TOKEN_ERROR_SOURCE } })
}

async function requestMachineToken(cfg: RuntimeCfg, timeoutMs: number): Promise<string> {
  const appId = String(cfg.public.ssoAppId || "").trim()
  const appSecret = String(cfg.ssoAppSecret || "").trim()
  const tokenUrl = resolveSsoTokenUrl(cfg)
  if (!appId || !appSecret || !tokenUrl) {
    throw machineTokenError(500, "SSO machine token is not configured", "SSO is misconfigured on this app.")
  }

  const basic = Buffer.from(`${appId}:${appSecret}`, "utf8").toString("base64")
  let res: any
  try {
    res = await $fetch<any>(tokenUrl, {
      method: "POST",
      headers: { authorization: `Basic ${basic}`, "content-type": "application/json" },
      body: { grant_type: "client_credentials" },
      timeout: timeoutMs,
    })
  } catch (err: any) {
    const status = err?.statusCode ?? err?.status ?? err?.response?.status
    if (status === 400 || status === 401) {
      if (isSsoExchangeDebug()) {
        console.error(
          `[auth:sso:machine-token] gateway rejected app credentials status=${status} (check SSO_APP_ID / SSO_APP_SECRET)`,
        )
      }
      throw machineTokenError(
        500,
        "SSO app credentials rejected",
        "SSO is misconfigured on this app. Please contact the administrator.",
      )
    }
    if (isSsoExchangeDebug()) {
      console.error(`[auth:sso:machine-token] token request failed status=${status ?? "network"}`)
    }
    throw machineTokenError(502, "SSO gateway unreachable", "Could not reach the SSO gateway. Please try again.")
  }

  const token = String(res?.access_token || "").trim()
  const expiresIn = Number(res?.expires_in)
  if (!token) {
    throw machineTokenError(502, "SSO machine token missing", "SSO gateway returned no access token.")
  }
  const lifetimeMs = Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn * 1000 : 60_000
  cached = { token, expiresAtMs: Date.now() + lifetimeMs }
  return token
}

export async function getSsoMachineToken(cfg: RuntimeCfg, timeoutMs = 8000): Promise<string> {
  if (cached && Date.now() < cached.expiresAtMs - EXPIRY_SKEW_MS) return cached.token
  inflight ??= requestMachineToken(cfg, timeoutMs).finally(() => {
    inflight = null
  })
  return inflight
}
