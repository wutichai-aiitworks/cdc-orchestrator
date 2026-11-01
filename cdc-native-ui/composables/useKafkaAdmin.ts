import type { TopicActionResult } from "~/composables/useKafkaM1"

export type BrokerItem = {
  id: number
  host: string
  port: number
  controller: boolean
  segment_size?: number | null
  segment_count?: number | null
}

export type BrokersResponse = {
  items: BrokerItem[]
  controller_id: number | null
  cluster_id: string | null
  version: string | null
  broker_count: number
  timestamp: string
}

export type BrokerConfigEntry = {
  name: string
  display_name?: string
  value: string
  display_value?: string
  source?: string | null
}

export type BrokerLogdirPartition = {
  partition: number
  size: number
  offset_lag?: number | null
}

export type BrokerLogdirTopic = {
  name: string
  partitions: BrokerLogdirPartition[]
}

export type BrokerLogdir = {
  name: string
  error?: string | null
  topics_count: number
  partitions_count: number
  size: number
  topics: BrokerLogdirTopic[]
}

export type BrokerDetail = {
  broker: BrokerItem
  segment_size?: number | null
  segment_count?: number | null
  configs: BrokerConfigEntry[]
  logdirs: BrokerLogdir[]
  raw: string
  timestamp: string
}

export type BrokerMetrics = {
  broker_id: number
  segment_size?: number | null
  segment_count?: number | null
  metrics: Array<{ name: string; labels: Record<string, string>; value: number }>
  node?: Record<string, unknown> | null
  health?: Record<string, unknown> | null
  timestamp: string
}

export type AclItem = {
  principal: string
  host: string
  resource_type: string
  resource_name: string
  pattern_type: string
  operation: string
  permission: string
}

export type SchemaSubjectItem = {
  subject: string
  id: number | null
  version: number | null
  schema_type: string | null
  compatibility: string | null
}

export type SchemaDetail = {
  subject: string
  id: number | null
  version: number | null
  schema_type: string | null
  schema: string
  versions: number[]
  compatibility: string | null
}

export type ConsumerGroupDetail = {
  group: string
  state: string | null
  members: number | null
  coordinator: string | null
  balancer: string | null
  total_lag: number
  entries: Array<{
    topic: string
    partition: number
    current_offset: number
    log_end_offset: number
    lag: number
    consumer_id?: string | null
    host?: string | null
  }>
  raw: string
}

export type AclUpsert = {
  principal: string
  host?: string
  resource_type: string
  resource_name: string
  pattern_type?: string
  operation: string
  permission: string
}

export const useKafkaAdmin = () => {
  const fetchBrokers = () => $fetch<BrokersResponse>("/api/cdc/brokers")
  const fetchBroker = (id: number | string) => $fetch<BrokerDetail>(`/api/cdc/brokers/${id}`)
  const fetchBrokerLogdirs = (id: number | string) =>
    $fetch<{ broker_id: number; items: BrokerLogdir[] }>(`/api/cdc/brokers/${id}/logdirs`)
  const fetchBrokerConfigs = (id: number | string) =>
    $fetch<{ broker_id: number; items: BrokerConfigEntry[] }>(`/api/cdc/brokers/${id}/configs`)
  const updateBrokerConfig = (id: number | string, body: { name: string; value: string }) =>
    $fetch<TopicActionResult>(`/api/cdc/brokers/${id}/configs`, { method: "PUT", body })
  const fetchBrokerMetrics = (id: number | string) => $fetch<BrokerMetrics>(`/api/cdc/brokers/${id}/metrics`)

  const fetchAcls = (params?: {
    search?: string
    host?: string
    resource?: string
    resource_type?: string
    operation?: string
    permission?: string
    pattern_type?: string
  }) =>
    $fetch<{ items: AclItem[]; total: number; configured: boolean }>("/api/cdc/acls", { query: params })
  const createAcl = (body: AclUpsert) => $fetch<TopicActionResult>("/api/cdc/acls", { method: "POST", body })
  const deleteAcl = (body: AclUpsert) => $fetch<TopicActionResult>("/api/cdc/acls", { method: "DELETE", body })

  const fetchSchemas = () =>
    $fetch<{ items: SchemaSubjectItem[]; total: number; configured: boolean }>("/api/cdc/schemas")
  const fetchSchema = (subject: string, version?: string) =>
    $fetch<SchemaDetail>(`/api/cdc/schemas/${encodeURIComponent(subject)}`, {
      query: version ? { version } : undefined,
    })
  const createSchema = (body: {
    subject: string
    schema: string
    schema_type?: string
    compatibility?: string
  }) => $fetch<TopicActionResult>("/api/cdc/schemas", { method: "POST", body })
  const updateSchema = (subject: string, body: { schema: string; schema_type?: string }) =>
    $fetch<TopicActionResult>(`/api/cdc/schemas/${encodeURIComponent(subject)}`, { method: "PUT", body })
  const deleteSchema = (subject: string) =>
    $fetch<TopicActionResult>(`/api/cdc/schemas/${encodeURIComponent(subject)}`, { method: "DELETE" })
  const checkSchemaCompatibility = (subject: string, body: { schema: string; schema_type?: string }) =>
    $fetch<{ compatible: boolean; message: string }>(
      `/api/cdc/schemas/${encodeURIComponent(subject)}/compatibility`,
      { method: "POST", body }
    )
  const setSchemaCompatibility = (subject: string, compatibility: string) =>
    $fetch<TopicActionResult>(
      `/api/cdc/schemas/${encodeURIComponent(subject)}/compatibility`,
      { method: "PUT", body: { compatibility } }
    )
  const deleteSchemaVersion = (subject: string, version: string | number) =>
    $fetch<TopicActionResult>(
      `/api/cdc/schemas/${encodeURIComponent(subject)}/versions/${version}`,
      { method: "DELETE" }
    )

  const fetchKsqlInfo = () =>
    $fetch<{ configured: boolean; reachable?: boolean; info?: Record<string, unknown>; error?: string }>(
      "/api/cdc/ksqldb/info"
    )
  const fetchKsqlTables = () =>
    $fetch<{ configured: boolean; items: Array<Record<string, unknown>> }>("/api/cdc/ksqldb/tables")
  const fetchKsqlStreams = () =>
    $fetch<{ configured: boolean; items: Array<Record<string, unknown>> }>("/api/cdc/ksqldb/streams")
  const fetchKsqlQueries = () =>
    $fetch<{ configured: boolean; items: Array<Record<string, unknown>> }>("/api/cdc/ksqldb/queries")
  const runKsql = (ksql: string) => $fetch("/api/cdc/ksqldb/query", { method: "POST", body: { ksql } })
  const terminateKsqlQuery = (query_id: string) =>
    $fetch("/api/cdc/ksqldb/terminate", { method: "POST", body: { query_id } })
  const dropKsqlEntity = (entity_type: string, name: string, delete_topic?: boolean) =>
    $fetch("/api/cdc/ksqldb/drop", { method: "POST", body: { entity_type, name, delete_topic } })
  const explainKsqlQuery = (query_id: string) =>
    $fetch(`/api/cdc/ksqldb/explain/${encodeURIComponent(query_id)}`)

  const produceMessage = (
    topic: string,
    body: { value: string; key?: string; partition?: number; headers?: Array<{ name: string; value: string }> }
  ) =>
    $fetch<TopicActionResult>(`/api/cdc/topics/${encodeURIComponent(topic)}/messages`, {
      method: "POST",
      body,
    })
  const copyTopic = (topic: string, name: string) =>
    $fetch<TopicActionResult>(`/api/cdc/topics/${encodeURIComponent(topic)}/copy`, {
      method: "POST",
      body: { name },
    })
  const updateTopicConfigs = (topic: string, configs: Record<string, string>) =>
    $fetch<TopicActionResult>(`/api/cdc/topics/${encodeURIComponent(topic)}/configs`, {
      method: "PUT",
      body: { configs },
    })

  const fetchGroup = (group: string) =>
    $fetch<ConsumerGroupDetail>(`/api/cdc/consumer-groups/${encodeURIComponent(group)}`)
  const deleteGroup = (group: string) =>
    $fetch<TopicActionResult>(`/api/cdc/consumer-groups/${encodeURIComponent(group)}`, { method: "DELETE" })
  const resetGroup = (
    group: string,
    body: { reset_type: string; topic?: string; partitions?: number[]; timestamp_ms?: number }
  ) =>
    $fetch<TopicActionResult>(`/api/cdc/consumer-groups/${encodeURIComponent(group)}/reset`, {
      method: "POST",
      body,
    })

  const deleteTopicOffsets = (group: string, topic: string) =>
    $fetch<TopicActionResult>(
      `/api/cdc/consumer-groups/${encodeURIComponent(group)}/topics/${encodeURIComponent(topic)}/offsets`,
      { method: "DELETE" }
    )

  return {
    fetchBrokers,
    fetchBroker,
    fetchBrokerLogdirs,
    fetchBrokerConfigs,
    updateBrokerConfig,
    fetchBrokerMetrics,
    fetchAcls,
    createAcl,
    deleteAcl,
    fetchSchemas,
    fetchSchema,
    createSchema,
    updateSchema,
    deleteSchema,
    checkSchemaCompatibility,
    setSchemaCompatibility,
    deleteSchemaVersion,
    fetchKsqlInfo,
    fetchKsqlTables,
    fetchKsqlStreams,
    fetchKsqlQueries,
    runKsql,
    terminateKsqlQuery,
    dropKsqlEntity,
    explainKsqlQuery,
    produceMessage,
    copyTopic,
    updateTopicConfigs,
    fetchGroup,
    deleteGroup,
    resetGroup,
    deleteTopicOffsets,
  }
}
