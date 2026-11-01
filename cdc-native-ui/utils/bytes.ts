export const formatBytes = (bytes?: number | null) => {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return "—"
  if (bytes < 1024) return `${bytes} B`
  const units = ["KB", "MB", "GB", "TB"]
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  const rounded = value >= 10 || unit === 0 ? Math.round(value) : Math.round(value * 10) / 10
  return `${rounded} ${units[unit]}`
}

export const formatDurationMs = (ms: number) => {
  if (!Number.isFinite(ms) || ms < 1000) return ""
  const second = 1000
  const minute = 60 * second
  const hour = 60 * minute
  const day = 24 * hour
  const week = 7 * day
  if (ms % week === 0) return `${ms / week}w`
  if (ms % day === 0) return `${ms / day}d`
  if (ms % hour === 0) return `${ms / hour}h`
  if (ms % minute === 0) return `${ms / minute}m`
  return `${Math.round(ms / second)}s`
}
