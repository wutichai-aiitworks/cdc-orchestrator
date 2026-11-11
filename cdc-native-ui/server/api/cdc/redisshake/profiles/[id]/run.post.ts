import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

// ?mode=once|cdc[&snapshot=true|false][&empty_target=true|false] starts this run in another mode
// than the profile default (empty_target: delete the target's keys before the full copy).
export default defineEventHandler((event) => {
  const id = getRouterParam(event, "id")
  const q = getQuery(event)
  const params = new URLSearchParams()
  if (q.mode === "once" || q.mode === "cdc") params.set("mode", String(q.mode))
  if (q.mode === "cdc" && (q.snapshot === "true" || q.snapshot === "false")) params.set("snapshot", String(q.snapshot))
  if (q.mode && (q.empty_target === "true" || q.empty_target === "false")) params.set("empty_target", String(q.empty_target))
  const qs = params.toString() ? `?${params}` : ""
  return orchestratorFetch(event, `/cdc/redisshake/profiles/${encodeURIComponent(String(id))}/run${qs}`, {
    method: "POST",
  })
})
