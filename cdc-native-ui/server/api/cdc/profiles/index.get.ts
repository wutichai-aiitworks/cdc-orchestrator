import { orchestratorFetch } from "../../../utils/orchestratorFetch"

// Pipelines plus each one's source/target database version (ADR 0019 §8). Versions come from a
// separate store; if that call fails the list still loads (db_versions is then empty).
export default defineEventHandler(async (event) => {
  const [list, versions] = await Promise.all([
    orchestratorFetch<Record<string, unknown>>(event, "/cdc/profiles"),
    orchestratorFetch<{ versions?: Record<string, unknown> }>(event, "/cdc/profiles/db-versions").catch(() => null),
  ])
  return { ...list, db_versions: versions?.versions ?? {} }
})
