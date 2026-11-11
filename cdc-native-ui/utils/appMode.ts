// APP_MODE — the one deployment-mode variable (replaces ORCH_ENV): Production (no
// ribbon), Maintenance (amber ribbon: stable, fixes only) or Enhanced (violet ribbon: new features
// in development). Same rules as AppMode in src/config.rs. Unit-tested in tests/unit/appMode.test.ts.
export type AppMode = "production" | "maintenance" | "enhanced"

/** Common spellings accepted; unknown/empty → null. */
export function parseAppMode(raw: unknown): AppMode | null {
  const v = String(raw ?? "").trim().toLowerCase()
  if (/^prod(uction)?$/.test(v)) return "production"
  if (/^maint(enance|ainance)?$/.test(v)) return "maintenance"
  if (/^enhance(d|ments?)?$/.test(v)) return "enhanced"
  return null
}

/** APP_MODE, else the retired ORCH_ENV (dev → enhanced, prod → production), else production. */
export function resolveAppMode(appMode: unknown, legacyOrchEnv?: unknown): AppMode {
  const m = parseAppMode(appMode)
  if (m) return m
  return /^dev(elopment)?$/i.test(String(legacyOrchEnv ?? "").trim()) ? "enhanced" : "production"
}

// Colours live in AppModeRibbon.vue: Tailwind does not scan utils/. Production has no ribbon.
export const APP_MODE_RIBBON: Partial<Record<AppMode, { label: string; title: string }>> = {
  maintenance: { label: "Maintenance", title: "APP_MODE=Maintenance — stable release, fixes only" },
  enhanced: { label: "Enhanced", title: "APP_MODE=Enhanced — release with new features in development" },
}
