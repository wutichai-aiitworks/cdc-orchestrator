const readableString = (value: unknown) =>
  typeof value === "string" && value.trim() && value !== "true" && value !== "false"
    ? value.trim()
    : ""

/**
 * An API error explained for people: what happened (`title`), what to do (`hint`), and
 * the original technical text (`detail`) for whoever needs to dig in.
 */
export type FriendlyError = {
  kind:
    | "diagnosed"
    | "unreachable"
    | "orchestrator_down"
    | "auth"
    | "not_found"
    | "server"
    | "other"
  title: string
  hint: string
  detail: string
}

const statusOf = (error: any): number | undefined => {
  const s = Number(error?.statusCode || error?.status || error?.response?.status)
  return Number.isFinite(s) && s > 0 ? s : undefined
}

const messageOf = (error: any, fallback: string) =>
  readableString(error?.data?.message) ||
  readableString(error?.data?.statusMessage) ||
  readableString(error?.data?.error) ||
  readableString(error?.statusMessage) ||
  readableString(error?.message) ||
  fallback

/** The request never got an answer: server down/restarting, or the network dropped. */
const NO_RESPONSE = /failed to fetch|<no response>|networkerror|network error|load failed|econnrefused|fetch failed/i

export const describeApiError = (error: any, fallback: string): FriendlyError => {
  const status = statusOf(error)
  const message = messageOf(error, fallback)
  const detail = status ? `[${status}] ${message}` : message

  // Diagnosed by the orchestrator. Nitro nests it under data.data; direct calls don't.
  const diagnosis = error?.data?.data?.diagnosis || error?.data?.diagnosis
  if (diagnosis?.area_label && diagnosis?.title) {
    return {
      kind: "diagnosed",
      title: `${diagnosis.area_label}: ${diagnosis.title}`,
      hint: diagnosis.fix ?? "",
      detail,
    }
  }
  if (!status || NO_RESPONSE.test(message)) {
    return {
      kind: "unreachable",
      title: "Can't reach the CDC service",
      hint: "The orchestrator or this web app may be restarting (for example during a redeploy), or the network dropped. Nothing is lost — it will work again once the service is back.",
      detail,
    }
  }
  if (status === 401 || status === 403) {
    return {
      kind: "auth",
      title: "Your session has expired or you don't have access",
      hint: "Sign in again. If it keeps happening, ask an admin to check your role.",
      detail,
    }
  }
  if ((status === 502 || status === 503 || status === 504) && /unavailable|internal error|no response/i.test(message)) {
    return {
      kind: "orchestrator_down",
      title: "The orchestrator is offline or restarting",
      hint: "Wait a moment and try again. If it persists, check that the cdc-orchestrator container is running.",
      detail,
    }
  }
  if (status === 404) {
    return {
      kind: "not_found",
      title: "Not found",
      hint: "It may have been deleted, or the orchestrator is an older version without this feature.",
      detail,
    }
  }
  if (status >= 500 && /internal error/i.test(message)) {
    return {
      kind: "server",
      title: "The orchestrator hit an internal error",
      hint: "Try again. If it repeats, open the technical detail and check the orchestrator logs.",
      detail,
    }
  }
  // Already a specific message from the orchestrator (validation etc.): show as is.
  return { kind: "other", title: message, hint: "", detail }
}

/** One-line version for toasts: plain words first, technical detail last. */
export const formatApiError = (error: any, fallback: string) => {
  const f = describeApiError(error, fallback)
  if (f.kind === "other") return f.detail
  const detail = f.detail && f.detail !== fallback ? ` — Technical detail: ${f.detail}` : ""
  return `${f.title}. ${f.hint}${detail}`
}
