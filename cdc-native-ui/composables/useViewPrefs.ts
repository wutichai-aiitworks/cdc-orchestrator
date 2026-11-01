type PrefMap = Record<string, string | number | boolean>

const STORAGE_KEY = "kafbat-webui.prefs"

function readAll(): PrefMap {
  if (!import.meta.client) return {}
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw)
    return typeof parsed === "object" && parsed ? (parsed as PrefMap) : {}
  } catch {
    return {}
  }
}

function writeAll(value: PrefMap) {
  if (!import.meta.client) return
  localStorage.setItem(STORAGE_KEY, JSON.stringify(value))
}

export const useViewPrefs = () => {
  const getPref = <T extends string | number | boolean>(key: string, fallback: T): T => {
    const all = readAll()
    const value = all[key]
    return (value as T) ?? fallback
  }

  const setPref = (key: string, value: string | number | boolean) => {
    const all = readAll()
    all[key] = value
    writeAll(all)
  }

  return { getPref, setPref }
}
