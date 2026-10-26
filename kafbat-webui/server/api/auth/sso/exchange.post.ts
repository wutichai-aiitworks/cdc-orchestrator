import { deleteCookie, getCookie } from "h3"
import { sanitizeNextPath } from "~/utils/sso"
import { getSessionConfig } from "~/server/utils/sessionConfig"
import {
  getSsoMachineToken,
  invalidateSsoMachineToken,
  isSsoMachineAuthConfigured,
  isSsoMachineTokenError,
} from "~/server/utils/ssoMachineToken"

const AUTHELIA_PKCE_COOKIE = "cdc_atla_pkce"

function ssoExchangeDebug(): boolean {
  return isSsoExchangeDebug()
}

function decodeJwtPayload(token: string): Record<string, unknown> {
  const parts = token.split(".")
  if (parts.length < 2) return {}
  try {
    const parsed = JSON.parse(Buffer.from(parts[1], "base64url").toString("utf8"))
    return parsed && typeof parsed === "object" ? (parsed as Record<string, unknown>) : {}
  } catch {
    return {}
  }
}

function firstNonEmptyString(...values: unknown[]): string {
  for (const value of values) {
    if (typeof value === "string" && value.trim()) return value.trim()
  }
  return ""
}

function looksLikeEmail(value: string): boolean {
  return /^[^\s@]+@[^\s@]+$/.test(value.trim())
}

function toStringArray(value: unknown): string[] {
  if (Array.isArray(value)) return value.filter((x): x is string => typeof x === "string" && x.trim().length > 0)
  if (typeof value === "string" && value.trim()) {
    return value.includes(",") ? value.split(",").map((v) => v.trim()).filter(Boolean) : [value.trim()]
  }
  return []
}

function isMachineTokenRejection(err: any): boolean {
  const status = err?.statusCode ?? err?.status ?? err?.response?.status
  if (status !== 401) return false
  const detail = String(err?.data?.error ?? err?.data?.message ?? "")
  return /machine token|client_credentials|client_id/i.test(detail)
}

export default defineEventHandler(async (event) => {
  const cfg = useRuntimeConfig(event)
  const body = (await readBody<{
    provider?: string
    ticket?: string
    next?: string
    code?: string
    state?: string
    code_verifier?: string
  }>(event).catch(() => ({}))) || {}
  const provider = String(body.provider || cfg.public.ssoProvider || "supertokens_gateway").trim().toLowerCase()
  let next = sanitizeNextPath(body.next)

  if (provider === "authelia") {
    const code = String(body.code || "").trim()
    const state = String(body.state || "").trim()
    if (!code || !state) {
      throw createError({ statusCode: 400, statusMessage: "code and state are required" })
    }

    const rawPkce = getCookie(event, AUTHELIA_PKCE_COOKIE) || ""
    deleteCookie(event, AUTHELIA_PKCE_COOKIE, { path: "/" })
    let codeVerifier = ""
    let cookieState = ""
    if (rawPkce) {
      try {
        const parsed = JSON.parse(Buffer.from(rawPkce, "base64url").toString("utf8")) as {
          state?: string
          codeVerifier?: string
          next?: string
        }
        cookieState = String(parsed.state || "")
        codeVerifier = String(parsed.codeVerifier || "")
        next = sanitizeNextPath(parsed.next || next)
      } catch {
        // Invalid cookie; treated as unauthorized below.
      }
    }
    const bodyCodeVerifier = String(body.code_verifier || "").trim()
    if ((!codeVerifier || !cookieState || cookieState !== state) && bodyCodeVerifier) {
      codeVerifier = bodyCodeVerifier
      cookieState = state
    }
    if (!codeVerifier || !cookieState || cookieState !== state) {
      throw createError({ statusCode: 403, statusMessage: "Unauthorized", message: "Invalid or expired OIDC state" })
    }

    const tokenUrl = String(cfg.autheliaTokenUrl || "").trim()
    const redirectUri = String(cfg.public.ssoCallbackUrl || "").trim()
    const clientId = String(cfg.public.autheliaClientId || "").trim()
    const clientSecret = String(cfg.autheliaClientSecret || "").trim()
    if (!tokenUrl || !redirectUri || !clientId) {
      throw createError({ statusCode: 500, statusMessage: "authelia is not configured correctly" })
    }

    const form = new URLSearchParams({
      grant_type: "authorization_code",
      client_id: clientId,
      code,
      redirect_uri: redirectUri,
      code_verifier: codeVerifier,
    })
    const headers: Record<string, string> = { "content-type": "application/x-www-form-urlencoded" }
    if (clientSecret) {
      headers.authorization = `Basic ${Buffer.from(`${clientId}:${clientSecret}`, "utf8").toString("base64")}`
    }

    const tokenRes = await $fetch<any>(tokenUrl, {
      method: "POST",
      body: form.toString(),
      headers,
      timeout: 8000,
    }).catch((err: any) => {
      throw createError({
        statusCode: err?.statusCode || 502,
        statusMessage: "Authelia token exchange failed",
        message: err?.data?.error_description || err?.message || "token_exchange_failed",
      })
    })

    const accessToken = String(tokenRes?.access_token || "").trim()
    const idToken = String(tokenRes?.id_token || "").trim()
    if (!accessToken) {
      throw createError({ statusCode: 502, statusMessage: "Authelia returned no access token" })
    }

    const claims = { ...decodeJwtPayload(accessToken), ...decodeJwtPayload(idToken) }
    const email = firstNonEmptyString(claims.email, claims.preferred_username, claims.upn)
    const userId = firstNonEmptyString(claims.sub, email, "authelia")
    const groups = toStringArray(claims.groups)

    const session = await useSession(event, getSessionConfig())
    await session.update({
      userId,
      email: looksLikeEmail(email) ? email : "",
      groups,
      createdAt: Date.now(),
      authProvider: "authelia",
    })
    return { ok: true, next, sub: userId, username: email || userId }
  }

  const ticket = String(body.ticket || "").trim()
  if (!ticket) {
    throw createError({ statusCode: 400, statusMessage: "ticket is required" })
  }

  const exchangeUrl = String(cfg.ssoExchangeUrl || "").trim()
  const appId = String(cfg.public.ssoAppId || "").trim()
  if (!exchangeUrl || !appId) {
    throw createError({ statusCode: 500, statusMessage: "sso is not configured correctly" })
  }

  const timeoutMs = Number(cfg.ssoExchangeTimeoutMs || 12000)
  const safeTimeoutMs = Number.isFinite(timeoutMs) && timeoutMs > 0 ? timeoutMs : 12000
  const machineAuth = isSsoMachineAuthConfigured(cfg)

  const callExchange = async () => {
    const headers: Record<string, string> = { "content-type": "application/json" }
    if (machineAuth) {
      headers.authorization = `Bearer ${await getSsoMachineToken(cfg, Math.min(safeTimeoutMs, 8000))}`
    }
    const requestBody = { ticket, app_id: appId }
    if (ssoExchangeDebug()) {
      console.info("[auth:sso:exchange] request", {
        url: exchangeUrl,
        method: "POST",
        authorization: headers.authorization ? "Bearer ***" : "(none)",
        body: { app_id: appId, ticket: `${ticket.slice(0, 8)}… (${ticket.length})` },
      })
    }
    try {
      const upstream = await $fetch<any>(exchangeUrl, {
        method: "POST",
        headers,
        body: requestBody,
        timeout: safeTimeoutMs,
      })
      if (ssoExchangeDebug()) console.info("[auth:sso:exchange] response", upstream)
      return upstream
    } catch (err: any) {
      if (ssoExchangeDebug()) {
        console.error("[auth:sso:exchange] response error", {
          status: err?.statusCode ?? err?.status ?? err?.response?.status,
          data: err?.data,
          message: err?.data?.error ?? err?.data?.message ?? err?.message,
        })
      }
      throw err
    }
  }

  let res: any
  try {
    try {
      res = await callExchange()
    } catch (err: any) {
      if (!machineAuth || !isMachineTokenRejection(err)) throw err
      invalidateSsoMachineToken()
      res = await callExchange()
    }
  } catch (err: any) {
    if (isSsoMachineTokenError(err)) throw err
    if (isMachineTokenRejection(err)) {
      invalidateSsoMachineToken()
      throw createError({
        statusCode: 500,
        statusMessage: "SSO app credentials rejected",
        message: "SSO is misconfigured on this app. Please contact the administrator.",
      })
    }
    const status = err?.statusCode ?? err?.status ?? err?.response?.status
    if (status === 401) {
      throw createError({
        statusCode: 401,
        statusMessage: "Unauthorized",
        message: "SSO ticket is invalid or already used. Please sign in again.",
      })
    }
    throw createError({
      statusCode: status || 502,
      statusMessage: "SSO ticket exchange failed",
      message: err?.data?.message || err?.message || "exchange_failed",
    })
  }

  const email = firstNonEmptyString(res?.email, res?.user?.email, res?.preferred_username, res?.username)
  const userId = firstNonEmptyString(res?.user_id, res?.sub, res?.user?.id, email)
  const groups = toStringArray(res?.groups ?? res?.roles)
  const session = await useSession(event, getSessionConfig())
  await session.update({
    userId,
    email: looksLikeEmail(email) ? email : "",
    groups,
    createdAt: Date.now(),
    authProvider: "supertokens_gateway",
  })
  return { ok: true, next, sub: userId, username: email || userId }
})
