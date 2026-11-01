export const buildQuery = (query: Record<string, any>) => {
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null || value === "") continue
    params.set(key, String(value))
  }
  const s = params.toString()
  return s ? `?${s}` : ""
}
