import type { H3Event } from "h3"

export const orchestratorFetch = async <T>(
  event: H3Event,
  path: string,
  options: Record<string, any> = {}
): Promise<T> => {
  const config = useRuntimeConfig(event)
  const base = String(config.orchestratorUrl || "http://127.0.0.1:9788").replace(/\/$/, "")
  const url = `${base}${path}`
  const headers: Record<string, string> = {}
  if (config.orchestratorToken) {
    headers.Authorization = `Bearer ${config.orchestratorToken}`
  }
  try {
    return (await $fetch<T>(url, {
      ...options,
      headers: { ...headers, ...(options.headers || {}) },
    })) as T
  } catch (error: any) {
    const status = Number(error?.response?.status || error?.statusCode || error?.status || 502)
    const statusMessage =
      typeof error?.statusMessage === "string" &&
      !["Not Found", "Method Not Allowed", "Bad Request", "Server Error"].includes(error.statusMessage)
        ? error.statusMessage
        : ""
    const detail =
      (typeof error?.data?.message === "string" && error.data.message) ||
      (typeof error?.data?.error === "string" && error.data.error) ||
      statusMessage ||
      ""
    const message =
      detail ||
      (status === 404
        ? "This feature is not available on the connected orchestrator. Upgrade or restart the orchestrator service."
        : status === 405
          ? "This operation is not supported by the connected orchestrator version."
          : status === 401 || status === 403
            ? "The orchestrator rejected this request. Check the configured access token."
            : status >= 500
              ? "The orchestrator is unavailable or returned an internal error."
              : "The orchestrator rejected the request.")

    // The orchestrator answers 502/504 when *its* dependency (Kafka Connect, a database)
    // failed. Forwarded as-is, front-door gateways (Cloudflare, HAProxy error pages)
    // replace the body of a 502/504 and the browser sees only "502". When the
    // orchestrator did answer with a reason, surface it as 424 Failed Dependency so the
    // message survives; an unreachable orchestrator (no response) stays a 502.
    const answered = Boolean(error?.response) && Boolean(detail)
    const clientStatus = answered && (status === 502 || status === 504) ? 424 : status

    throw createError({
      statusCode: clientStatus,
      statusMessage: message,
      message,
      data: {
        upstream_status: status,
        upstream_path: path,
        audit_id: error?.data?.audit_id,
        // Where the failure is + how to fix it (orchestrator ≥ diagnose module).
        diagnosis: error?.data?.diagnosis,
      },
    })
  }
}
