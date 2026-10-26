export default defineEventHandler(async (event) => {
  const body = (await readBody<{ enabled?: boolean | null }>(event).catch(() => ({}))) || {}
  if (body.enabled !== true && body.enabled !== false && body.enabled !== null) {
    throw createError({ statusCode: 400, statusMessage: "enabled must be true, false, or null" })
  }
  setSsoExchangeDebug(body.enabled)
  return getSsoExchangeDebugState()
})
