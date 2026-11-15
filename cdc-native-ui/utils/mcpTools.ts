// Groups for the MCP Tools page (same grouping as docs/mcp-tools.md). Unknown tools land in "Other",
// so a new tool still shows up before anyone updates this list.
export const MCP_TOOL_GROUPS: { title: string; tools: string[] }[] = [
  { title: "Production monitoring", tools: ["cdc_source_health", "cdc_source_readiness", "cdc_ssh_tunnels", "cdc_ssh_tunnel_probe", "cdc_ssh_tunnel_reconnect", "cdc_redis_sync_status"] },
  { title: "Overview", tools: ["cdc_help", "cdc_overview", "cdc_health", "cdc_capability", "cdc_engine_status", "cdc_cluster", "cdc_metrics"] },
  { title: "Latency, logs, audit", tools: ["cdc_latency", "cdc_pipeline_latency", "cdc_latency_probe", "cdc_pipeline_logs", "cdc_audit_events", "cdc_logs_tail", "cdc_alert_history"] },
  { title: "Pipelines", tools: ["cdc_profile_list", "cdc_profile_get", "cdc_profile_dlq", "cdc_tables", "cdc_source_validate", "cdc_target_validate", "cdc_pipeline_create", "cdc_profile_upsert", "cdc_profile_enable", "cdc_profile_action", "cdc_pipeline_position", "cdc_pipeline_reposition"] },
  { title: "Copy & Sync", tools: ["cdc_copy_profile_list", "cdc_copy_profile_get", "cdc_copy_test_connection", "cdc_copy_jobs", "cdc_copy_job_logs", "cdc_pg_copy_create", "cdc_copy_run", "cdc_copy_job_stop", "cdc_copy_profile_delete", "cdc_migration_tools_discover", "cdc_migration_tool_contract"] },
  { title: "Kafka Connect", tools: ["cdc_connector_list", "cdc_connector_status", "cdc_connector_timeline", "cdc_connector_templates", "cdc_connector_validate", "cdc_connector_create", "cdc_connector_update", "cdc_connector_action", "cdc_connector_delete"] },
  { title: "Topics and consumers", tools: ["cdc_topics_list", "cdc_kafka_topics", "cdc_topic_messages", "cdc_topic_metadata", "cdc_consumer_groups", "cdc_kafka_lag", "cdc_topic_create", "cdc_topic_delete", "cdc_topic_recreate", "cdc_topic_clear_messages"] },
  { title: "Alerts", tools: ["cdc_notification_channels_list", "cdc_notification_channel_upsert", "cdc_notification_channel_test", "cdc_notification_channel_delete", "cdc_notification_deliveries", "cdc_notification_rules_list", "cdc_notification_rule_upsert", "cdc_notification_rule_delete"] },
  { title: "Event processing", tools: ["cdc_processing_pipelines_list", "cdc_processing_pipeline_get", "cdc_processing_pipeline_upsert", "cdc_processing_pipeline_delete"] },
  { title: "Engine", tools: ["cdc_engine_up", "cdc_engine_down", "cdc_engine_restart"] },
  { title: "Full API", tools: ["cdc_api_catalog", "cdc_api_get", "cdc_api_call"] },
]

export const groupOfTool = (name: string) => MCP_TOOL_GROUPS.find((g) => g.tools.includes(name))?.title || "Other"

export const MCP_EXAMPLE_PROMPTS: { ask: string; tools: string }[] = [
  { ask: "Is the source of Orders Sync healthy? Is the slot holding WAL?", tools: "cdc_source_health" },
  { ask: "What must the DBA fix before we start CDC on this source?", tools: "cdc_source_readiness" },
  { ask: "Are the SSH tunnels up? Which pipelines use them?", tools: "cdc_ssh_tunnels" },
  { ask: "Is the Redis real-time sync still running?", tools: "cdc_redis_sync_status" },
  { ask: "Which pipeline is slowest right now?", tools: "cdc_latency" },
  { ask: "What failed in the last hour?", tools: "cdc_audit_events, cdc_pipeline_logs" },
  { ask: "Alert me on LINE when a connector fails", tools: "cdc_notification_channel_upsert (write)" },
]

export const MCP_DAILY_CHECK =
  "Check production CDC: run cdc_source_health for every running pipeline, cdc_ssh_tunnels and cdc_redis_sync_status. List only warnings, with what to do. Do not change anything."
