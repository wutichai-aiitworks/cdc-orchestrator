export type MigrationToolCatalogEntry = {
  id: string
  title: string
  enabled: boolean
  discover_path: string
  contract_path: string
  ui_path: string
  orchestrator_env: string[]
}

export type MigrationToolsDiscover = {
  contract_version: string
  timestamp: string
  tools: MigrationToolCatalogEntry[]
}

export const useMigrationTools = () => {
  const fetchCatalog = () =>
    $fetch<MigrationToolsDiscover>("/api/cdc/migration-tools/discover")

  const fetchToolDiscover = (toolId: string) =>
    $fetch<Record<string, unknown>>(`/api/cdc/migration-tools/${encodeURIComponent(toolId)}/discover`)

  const fetchToolContract = (toolId: string) =>
    $fetch<Record<string, unknown>>(`/api/cdc/migration-tools/${encodeURIComponent(toolId)}/contract`)

  return { fetchCatalog, fetchToolDiscover, fetchToolContract }
}

/** Merge contract.options.defaults into a local options object. */
export function applyContractOptionDefaults<T extends Record<string, unknown>>(
  target: T,
  contract: Record<string, unknown> | null | undefined,
): T {
  const options = contract?.options as Record<string, unknown> | undefined
  const defaults = options?.defaults as Record<string, unknown> | undefined
  if (!defaults) return target
  return { ...defaults, ...target } as T
}
