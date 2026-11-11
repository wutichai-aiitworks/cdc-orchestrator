// APP_MODE (docs/patterns/deployment-mode): Production (no ribbon) | Maintenance (amber) | Enhanced (violet).
// Same rules as the backend parser — keep both test files in step.
// No Tailwind classes here: utils/ is usually not in tailwind.config `content`, so they would be purged.
export type AppMode = "production" | "maintenance" | "enhanced"

export function parseAppMode(raw: unknown): AppMode | null {
  const v = String(raw ?? "").trim().toLowerCase()
  if (/^prod(uction)?$/.test(v)) return "production"
  if (/^maint(enance|ainance)?$/.test(v)) return "maintenance"
  if (/^enhance(d|ments?)?$/.test(v)) return "enhanced"
  return null
}

/** APP_MODE, else the retired APP_ENV (dev → enhanced, prod → production), else production. */
export function resolveAppMode(mode: unknown, legacyEnv?: unknown): AppMode {
  const m = parseAppMode(mode)
  if (m) return m
  return /^dev(elopment)?$/i.test(String(legacyEnv ?? "").trim()) ? "enhanced" : "production"
}

export const APP_MODE_RIBBON: Partial<Record<AppMode, { label: string; title: string }>> = {
  maintenance: { label: "Maintenance", title: "APP_MODE=Maintenance — stable release, fixes only" },
  enhanced: { label: "Enhanced", title: "APP_MODE=Enhanced — release with new features in development" },
}
