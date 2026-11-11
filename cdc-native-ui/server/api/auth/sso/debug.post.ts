export default defineEventHandler(async (event) => {
  type Body = { enabled?: boolean | null }
  // Annotated: `.catch(() => ({}))` would otherwise widen the type to `Body | {}`.
  const body: Body = (await readBody<Body>(event).catch(() => ({}))) || {}
  if (body.enabled !== true && body.enabled !== false && body.enabled !== null) {
    throw createError({ statusCode: 400, statusMessage: "enabled must be true, false, or null" })
  }
  setSsoExchangeDebug(body.enabled)
  return getSsoExchangeDebugState()
})
