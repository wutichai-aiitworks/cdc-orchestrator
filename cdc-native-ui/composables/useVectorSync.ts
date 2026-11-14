// AI vector sync (ADR 0018) — /api/cdc/vectorsync/** → orchestrator → cdc-vectorsync sidecar.

export type VsStatus = {
  state: string
  message?: string
  counters?: Record<string, number>
  lag?: { total: number | null; topics: Record<string, number> }
  documents?: number | null
  embedding?: { provider?: string; model?: string; avg_ms?: number | null }
  retention_trimmed?: string[]
  missing_topics?: string[]
  last_error?: string | null
  last_error_at?: string | null
  last_event_at?: string | null
  recent_errors?: { at: string; error: string }[]
  started_at?: string
  group_id?: string
  generation?: number
}

export type VsProfile = {
  id: string
  name: string
  description: string
  enabled: boolean
  source_profile_id: string | null
  topics: string[]
  start_from: string
  embedding: Record<string, any>
  target: Record<string, any>
  projection: any
  id_strategy: any
  delete_mode: string
  generation: number
  created_at: string
  updated_at: string
  status: VsStatus
}

export type VsTopic = { name: string; table?: string; partitions: number; messages: number; trimmed: boolean }
export type VsSources = { pipelines: { profile_id: string; name: string; status: string; source_type: string; topic_prefix: string; topics: VsTopic[] }[]; other: VsTopic[] }
export type VsCheck = { name: string; ok: boolean; detail: string; hint?: string; dimensions?: number; latency_ms?: number; documents?: number | null }
export type VsCapability = {
  enabled: boolean
  sidecar_ready?: boolean
  hint?: string
  error?: string
  secrets_encrypted?: boolean
  versions?: Record<string, string | null>
  providers?: string[]
  default_models?: Record<string, string>
  docling?: { available: boolean; version?: string | null; max_mb?: number; max_pages?: number; formats?: string[] }
}
export type VsPreviewRecord = { partition: number; offset: number; op?: string; doc_id?: string; action?: string; content?: string; chars?: number; metadata?: Record<string, any>; error?: string; skipped?: string; chunks?: number; chunk_samples?: string[] }
export type VsPreviewTopic = { topic: string; table: string; columns: string[]; records: VsPreviewRecord[]; messages: number; error?: string }
export type VsHit = { id: string; score: number; content: string; metadata: Record<string, any> }

const base = "/api/cdc/vectorsync"
const enc = encodeURIComponent

export const useVectorSync = () => ({
  capability: () => $fetch<VsCapability>(`${base}/capability`),
  sources: () => $fetch<VsSources>(`${base}/sources`),
  test: (body: { embedding?: any; target?: any; profile_id?: string | null }) => $fetch<{ ok: boolean; checks: VsCheck[] }>(`${base}/test`, { method: "POST", body }),
  models: (embedding: any, profile_id?: string | null) => $fetch<{ models: string[]; error?: string }>(`${base}/embedding-models`, { method: "POST", body: { embedding, profile_id } }),
  preview: (body: any) => $fetch<{ topics: VsPreviewTopic[] }>(`${base}/preview`, { method: "POST", body }),
  list: () => $fetch<{ profiles: VsProfile[]; sidecar_ready: boolean }>(`${base}/profiles`),
  get: (id: string) => $fetch<VsProfile>(`${base}/profiles/${enc(id)}`),
  create: (body: any) => $fetch<VsProfile>(`${base}/profiles`, { method: "POST", body }),
  update: (id: string, body: any) => $fetch<VsProfile>(`${base}/profiles/${enc(id)}`, { method: "PUT", body }),
  remove: (id: string, dropCollection: boolean) => $fetch(`${base}/profiles/${enc(id)}`, { method: "DELETE", query: dropCollection ? { drop_collection: "true" } : {} }),
  pause: (id: string) => $fetch<VsProfile>(`${base}/profiles/${enc(id)}/pause`, { method: "POST" }),
  resume: (id: string) => $fetch<VsProfile>(`${base}/profiles/${enc(id)}/resume`, { method: "POST" }),
  rebuild: (id: string) => $fetch<VsProfile>(`${base}/profiles/${enc(id)}/rebuild`, { method: "POST" }),
  search: (id: string, body: { query: string; k?: number; filter?: Record<string, any> }) => $fetch<{ hits: VsHit[]; took_ms: number }>(`${base}/profiles/${enc(id)}/search`, { method: "POST", body }),
  deadLetters: (id: string, limit = 200) => $fetch<{ items: any[]; total: number }>(`${base}/profiles/${enc(id)}/dead-letters`, { query: { limit } }),
  clearDeadLetters: (id: string) => $fetch<{ cleared: number }>(`${base}/profiles/${enc(id)}/dead-letters`, { method: "DELETE" }),
})
