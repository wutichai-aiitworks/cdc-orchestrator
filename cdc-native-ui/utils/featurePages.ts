// Pages that exist only while an orchestrator feature switch is on (off = 404). Used by the server
// (server/middleware/feature-pages.ts, direct requests) and the page meta (middleware/feature.ts).
export const FEATURE_PAGES: Record<string, string> = {
  "/ksqldb": "ksql_enabled", // ORCH_KSQL_ENABLED (default false)
}
