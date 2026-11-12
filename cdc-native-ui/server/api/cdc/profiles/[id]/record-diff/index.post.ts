import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

const strings = (value: unknown) =>
  Array.isArray(value) ? value.map((v) => String(v).trim()).filter(Boolean).slice(0, 64) : []

// Queue a record diff of one table (source vs target by key). Body is whitelisted.
export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Profile ID is required" })
  const raw = (await readBody(event).catch(() => ({}))) || {}
  const options = raw.options && typeof raw.options === "object" ? raw.options : {}
  const limit = Number(options.recheck_limit)
  const body = {
    table: String(raw.table || "").trim(),
    key_columns: strings(raw.key_columns),
    compare_columns: strings(raw.compare_columns),
    compare_all_columns: raw.compare_all_columns === true,
    options: {
      trim_trailing_spaces: options.trim_trailing_spaces !== false,
      case_insensitive: options.case_insensitive === true,
      recheck: options.recheck !== false,
      recheck_limit: Number.isFinite(limit) && limit >= 0 ? Math.min(Math.floor(limit), 1_000_000) : 10_000,
    },
  }
  if (!body.table) throw createError({ statusCode: 400, statusMessage: "Choose a table to compare" })
  return orchestratorFetch(event, `/cdc/profiles/${id}/record-diff`, { method: "POST", body })
})
