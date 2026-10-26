export type ProcessingStage = {
  id: string
  name: string
  kind: "transform" | "filter" | "enrich"
  execution: "grpc" | "data_service"
  endpoint?: string | null
  dataservice?: {
    base_url: string
    datasource: string
    mapper: string
    api_key_env: string
    lookup_field: string
    lookup_parameter: string
    result_key_field: string
    output_field?: string | null
    static_parameters: Record<string, unknown>
  } | null
  timeout_ms: number
  retry: {
    max_attempts: number
    initial_backoff_ms: number
    max_backoff_ms: number
  }
  failure_policy: "fail_pipeline" | "emit_unchanged" | "drop" | "dead_letter"
  enabled: boolean
  attributes?: Record<string, string>
}

export type ProcessingPipeline = {
  id: string
  name: string
  input_topic: string
  output_topic: string
  dead_letter_topic: string
  consumer_group: string
  batch: {
    max_records: number
    max_bytes: number
    max_wait_ms: number
  }
  stages: ProcessingStage[]
  preserve_partition_order: boolean
  enabled: boolean
}

export const useProcessingPipelines = () => {
  const fetchPipelines = () =>
    $fetch<ProcessingPipeline[]>("/api/cdc/processing-pipelines")

  const validatePipeline = (body: ProcessingPipeline) =>
    $fetch<{ valid: boolean; errors: string[] }>("/api/cdc/processing-pipelines/validate", {
      method: "POST",
      body,
    })

  const createPipeline = (body: ProcessingPipeline) =>
    $fetch<ProcessingPipeline>("/api/cdc/processing-pipelines", {
      method: "POST",
      body,
    })

  const updatePipeline = (body: ProcessingPipeline) =>
    $fetch<ProcessingPipeline>(
      `/api/cdc/processing-pipelines/${encodeURIComponent(body.id)}`,
      { method: "PUT", body }
    )

  const deletePipeline = (id: string) =>
    $fetch<void>(`/api/cdc/processing-pipelines/${encodeURIComponent(id)}`, {
      method: "DELETE",
    })

  return {
    fetchPipelines,
    validatePipeline,
    createPipeline,
    updatePipeline,
    deletePipeline,
  }
}
