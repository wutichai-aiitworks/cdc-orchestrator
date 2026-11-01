export type TopicItem = {
  name: string
  internal: boolean
  partition_count: number
  replication_factor: number
  out_of_sync_replicas: number
  messages_count: number | null
  segment_size: number | null
  cleanup_policy?: string | null
}

export type PagedTopics = {
  items: TopicItem[]
  total: number
  page: number
  page_size: number
  timestamp: string
}

export type TopicActionResult = {
  action: string
  status: string
  message: string
  audit_id: string
  timestamp: string
}

export type CreateTopicPayload = {
  name: string
  partitions?: number
  replication_factor?: number
  cleanup_policy?: string
  min_insync_replicas?: number
  retention_ms?: number
  retention_bytes?: number
  max_message_bytes?: number
  custom_params?: Array<{ name: string; value: string }>
}

export type TopicMessages = {
  topic: string
  items: Array<{
    partition: number | null
    offset: number | null
    timestamp: string | null
    payload: string
    key?: string | null
  }>
  total: number
  limit: number
  offset: string
  mode: string
  timestamp: string
}

export type PagedGroups = {
  items: Array<{
    group: string
    state?: string | null
    members?: number | null
    topic_count?: number | null
    coordinator?: string | null
    total_lag?: number | null
  }>
  total: number
  page: number
  page_size: number
  timestamp: string
}

export type GroupLag = {
  group: string
  total_lag: number
  entries: Array<{
    topic: string
    partition: number
    current_offset: number
    log_end_offset: number
    lag: number
  }>
  timestamp: string
}

export type TopicMetadata = {
  topic: string
  partition_count: number
  urp: number
  partitions: Array<{
    partition: number
    leader: number | null
    replicas: number[]
    in_sync_replicas: number[]
    log_start_offset?: number | null
    high_watermark: number | null
    message_count?: number | null
  }>
  configs: Record<string, string>
  raw: string[]
  timestamp: string
}

export type TopicSortBy =
  | "name"
  | "partitions"
  | "out_of_sync_replicas"
  | "replication_factor"
  | "messages_count"
  | "size"

export const useKafkaM1 = () => {
  const fetchTopics = (params: {
    page?: number
    page_size?: number
    search?: string
    hide_internal?: boolean
    sort_by?: TopicSortBy
    sort_order?: "asc" | "desc"
  }) => $fetch<PagedTopics>("/api/cdc/topics", { query: params })

  const createTopic = (body: CreateTopicPayload) =>
    $fetch<TopicActionResult>("/api/cdc/topics", { method: "POST", body })

  const deleteTopic = (topic: string) =>
    $fetch<TopicActionResult>(`/api/cdc/topics/${encodeURIComponent(topic)}`, { method: "DELETE" })

  const recreateTopic = (topic: string) =>
    $fetch<TopicActionResult>(`/api/cdc/topics/${encodeURIComponent(topic)}/recreate`, {
      method: "POST",
    })

  const clearTopicMessages = (topic: string) =>
    $fetch<TopicActionResult>(`/api/cdc/topics/${encodeURIComponent(topic)}/clear-messages`, {
      method: "POST",
    })

  const copyTopic = (topic: string, newName: string) =>
    $fetch<TopicActionResult>(`/api/cdc/topics/${encodeURIComponent(topic)}/copy`, {
      method: "POST",
      body: { new_name: newName },
    })

  const fetchTopicMessages = (
    topic: string,
    params: {
      limit?: number
      offset?: string
      partition?: number
      include_meta?: boolean
      mode?: "latest" | "earliest"
      q?: string
    }
  ) =>
    $fetch<TopicMessages>(`/api/cdc/topics/${encodeURIComponent(topic)}/messages`, {
      query: params,
    })

  const fetchTopicMetadata = (topic: string) =>
    $fetch<TopicMetadata>(`/api/cdc/topics/${encodeURIComponent(topic)}/metadata`)

  const fetchGroups = (params: { page?: number; page_size?: number; search?: string; state?: string }) =>
    $fetch<PagedGroups>("/api/cdc/consumer-groups", { query: params })

  const fetchGroupLag = (group: string) =>
    $fetch<GroupLag>(`/api/cdc/consumer-groups/${encodeURIComponent(group)}/lag`)

  return {
    fetchTopics,
    createTopic,
    deleteTopic,
    recreateTopic,
    clearTopicMessages,
    copyTopic,
    fetchTopicMessages,
    fetchTopicMetadata,
    fetchGroups,
    fetchGroupLag,
  }
}
