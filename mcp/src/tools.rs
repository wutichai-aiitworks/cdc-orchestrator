use axum::http::Method;
use serde_json::{json, Value};

use crate::{
    arg_str, guard_write, orch, td, urlencoding_simple, ApiError, AppState, ToolCall, ToolDef,
};

pub fn definitions() -> Vec<ToolDef> {
    vec![
        td(
            "cdc_help",
            "Catalog of CDC MCP tools and example prompts. Start here when the user wants to operate CDC without the web UI.",
            json!({}),
            true,
        ),
        td(
            "cdc_overview",
            "One-shot status: health, capability, engine, connectors, and topic summary.",
            json!({}),
            true,
        ),
        td("cdc_health", "Orchestrator health check (GET /health).", json!({}), true),
        td("cdc_capability", "Feature flags and stack capability.", json!({}), true),
        td(
            "cdc_migration_tools_discover",
            "Catalog copy/sync tools: pgcopy, RedisShake, pgloader, mongocopy.",
            json!({}),
            true,
        ),
        td(
            "cdc_migration_tool_contract",
            "Profile schema, option defaults, and API map for pgcopy or redisshake.",
            json!({
                "type": "object",
                "required": ["tool_id"],
                "properties": {
                    "tool_id": {"type": "string", "enum": ["pgcopy", "redisshake", "pgloader", "mongocopy"]},
                },
            }),
            true,
        ),
        td("cdc_engine_status", "Redpanda/Connect/engine container status.", json!({}), true),
        td("cdc_cluster", "Kafka/Redpanda cluster info.", json!({}), true),
        td("cdc_metrics", "Orchestrator metrics snapshot.", json!({}), true),
        td("cdc_profile_list", "List CDC pipelines/profiles.", json!({}), true),
        td(
            "cdc_profile_get",
            "Get one CDC profile by id, including captured schemas/tables.",
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
            true,
        ),
        td(
            "cdc_profile_dlq",
            "Get a profile sink DLQ topic, policy, and current rejected-message count.",
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
            true,
        ),
        td(
            "cdc_tables",
            "List captured schemas/tables from profiles plus Kafka topics that represent those tables.",
            json!({}),
            true,
        ),
        td("cdc_connector_list", "List Kafka Connect connectors.", json!({}), true),
        td(
            "cdc_connector_status",
            "Connector + task state.",
            json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}),
            true,
        ),
        td(
            "cdc_connector_timeline",
            "Recent connector state transitions and task errors.",
            json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}),
            true,
        ),
        td("cdc_connector_templates", "Starter connector config templates.", json!({}), true),
        td(
            "cdc_topics_list",
            "Paged Kafka topics with partitions, replica health, message counts, and size.",
            json!({
                "type":"object",
                "properties":{
                    "search":{"type":"string"},
                    "page":{"type":"integer","default":1},
                    "page_size":{"type":"integer","default":50},
                    "hide_internal":{"type":"boolean","default":false},
                    "sort_by":{"type":"string","enum":["name","partitions","out_of_sync_replicas","replication_factor","messages_count","size"]},
                    "sort_order":{"type":"string","enum":["asc","desc"]}
                }
            }),
            true,
        ),
        td("cdc_kafka_topics", "Simple topic name list (legacy).", json!({}), true),
        td(
            "cdc_topic_messages",
            "Sample CDC/Kafka records from a topic (table stream).",
            json!({
                "type":"object","required":["topic"],
                "properties":{
                    "topic":{"type":"string","description":"Topic name, e.g. demo.bookings.airports_data"},
                    "limit":{"type":"integer","default":20},
                    "mode":{"type":"string","enum":["latest","earliest"],"default":"latest"},
                    "partition":{"type":"integer"},
                    "offset":{"type":"string","description":"start, end, or numeric offset"}
                }
            }),
            true,
        ),
        td(
            "cdc_topic_metadata",
            "Topic partitions, high watermarks, and configs.",
            json!({"type":"object","required":["topic"],"properties":{"topic":{"type":"string"}}}),
            true,
        ),
        td(
            "cdc_consumer_groups",
            "Paged consumer groups.",
            json!({
                "type":"object",
                "properties":{
                    "search":{"type":"string"},
                    "page":{"type":"integer","default":1},
                    "page_size":{"type":"integer","default":50}
                }
            }),
            true,
        ),
        td(
            "cdc_kafka_lag",
            "Consumer group lag by topic/partition.",
            json!({"type":"object","required":["group"],"properties":{"group":{"type":"string"}}}),
            true,
        ),
        td(
            "cdc_source_validate",
            "Probe a source database connection before creating a pipeline. Demo defaults: host cdc-postgres-source, db/user/pass demo.",
            json!({
                "type":"object",
                "properties":{
                    "source_type":{"type":"string","enum":["postgres","mysql","mariadb","sqlserver","oracle"],"default":"postgres"},
                    "host":{"type":"string","default":"cdc-postgres-source"},
                    "port":{"type":["string","integer"],"default":"5432"},
                    "database":{"type":"string","default":"demo"},
                    "username":{"type":"string","default":"demo"},
                    "password":{"type":"string","default":"demo"}
                }
            }),
            true,
        ),
        td(
            "cdc_target_validate",
            "Probe a target database connection. Demo defaults: host cdc-postgres-target, db/user/pass demo.",
            json!({
                "type":"object",
                "properties":{
                    "target_type":{"type":"string","enum":["postgres","mysql","mariadb","sqlserver","oracle"],"default":"postgres"},
                    "host":{"type":"string","default":"cdc-postgres-target"},
                    "port":{"type":["string","integer"],"default":"5432"},
                    "database":{"type":"string","default":"demo"},
                    "username":{"type":"string","default":"demo"},
                    "password":{"type":"string","default":"demo"}
                }
            }),
            true,
        ),
        td("cdc_alert_history", "Recent CDC alerts.", json!({"type":"object","properties":{"limit":{"type":"integer","default":100}}}), true),
        td("cdc_notification_rules_list", "Notification rules.", json!({}), true),
        td(
            "cdc_logs_tail",
            "Tail logs for a stack service (cdc-message, cdc-connect, cdc-orchestrator, ...).",
            json!({"type":"object","required":["service"],"properties":{"service":{"type":"string"},"tail":{"type":"integer","default":200}}}),
            true,
        ),
        td("cdc_processing_pipelines_list", "List event-processing pipelines.", json!({}), true),
        td(
            "cdc_processing_pipeline_get",
            "Get one processing pipeline.",
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
            true,
        ),
        td("cdc_engine_up", "Start the CDC engine stack (write).", json!({}), false),
        td("cdc_engine_down", "Stop the CDC engine stack (write).", json!({}), false),
        td("cdc_engine_restart", "Restart the CDC engine stack (write).", json!({}), false),
        td(
            "cdc_pipeline_create",
            "Create a CDC pipeline from prompt-friendly fields (no web UI). Optionally validate and start it.",
            json!({
                "type":"object","required":["name"],
                "properties":{
                    "name":{"type":"string"},
                    "description":{"type":"string","description":"Optional pipeline description"},
                    "pattern":{"type":"string","enum":["source_to_stream","source_to_target","source_to_lake","source_to_olake"],"default":"source_to_target","description":"source_to_target=Replica (default, can start). source_to_stream=Event-first. source_to_lake=Event-first + Iceberg from Kafka. source_to_olake=Analytics-first OLake (draft only; do not start)."},
                    "source_type":{"type":"string","default":"postgres"},
                    "source_host":{"type":"string","default":"cdc-postgres-source"},
                    "source_port":{"type":"string","default":"5432"},
                    "source_database":{"type":"string","default":"demo"},
                    "source_username":{"type":"string","default":"demo"},
                    "source_password":{"type":"string"},
                    "schemas":{"description":"Comma-separated or JSON array","type":["string","array"]},
                    "tables":{"description":"Optional table list, e.g. bookings.airports_data","type":["string","array"]},
                    "target_name":{"type":"string","description":"User-defined target name"},
                    "target_type":{"type":"string","default":"postgres"},
                    "target_host":{"type":"string","default":"cdc-postgres-target"},
                    "target_port":{"type":"string","default":"5432"},
                    "target_database":{"type":"string","default":"demo"},
                    "target_username":{"type":"string","default":"demo"},
                    "target_password":{"type":"string"},
                    "lake_name":{"type":"string","description":"Lake destination name"},
                    "lake_format":{"type":"string","enum":["iceberg","parquet"],"default":"iceberg"},
                    "lake_zone":{"type":"string","enum":["bronze","silver"],"default":"bronze"},
                    "lake_change_mode":{"type":"string","enum":["changelog","upsert","scd2"],"default":"changelog"},
                    "lake_catalog":{"type":"string","enum":["jdbc","rest","hadoop","glue","nessie"],"default":"jdbc"},
                    "lake_endpoint":{"type":"string","default":"http://cdc-lake:9000"},
                    "lake_bucket":{"type":"string","default":"lake"},
                    "lake_region":{"type":"string","default":"us-east-1"},
                    "lake_access_key":{"type":"string","default":"cdcadmin"},
                    "lake_secret_key":{"type":"string"},
                    "lake_warehouse":{"type":"string","default":"s3://lake/warehouse"},
                    "lake_catalog_uri":{"type":"string","default":"jdbc:postgresql://cdc-postgres-target:5432/demo"},
                    "domain":{"type":"string","description":"Industry-agnostic bounded context, e.g. commerce"},
                    "dataset":{"type":"string","description":"Logical dataset name, e.g. orders"},
                    "sensitivity":{"type":"string","enum":["public","internal","confidential","restricted"],"default":"internal"},
                    "industry_pack":{"type":"string","description":"Optional overlay: retail, banking, healthcare, ..."},
                    "delivery_mode":{"type":"string","enum":["realtime","micro_batch"],"default":"realtime"},
                    "interval_seconds":{"type":"integer"},
                    "capture_mode":{"type":"string","enum":["logical","trigger","sync_once","incremental"],"default":"logical","description":"logical=WAL CDC; trigger=changelog triggers; sync_once=one-time copy; incremental=JDBC watermark poll (no WAL/triggers; hard deletes not captured)"},
                    "sink_error_policy":{"type":"string","enum":["fail_fast","dlq"],"default":"fail_fast","description":"Source-to-target sink rejected-record behavior. dlq continues processing and requires correction/replay."},
                    "timestamp_column":{"type":"string","description":"Incremental: watermark column such as updated_at"},
                    "incrementing_column":{"type":"string","description":"Incremental: strictly incrementing PK column such as id. Required for replica upsert."},
                    "stream_platform":{"type":"string","enum":["redpanda","kafka"],"default":"redpanda"},
                    "validate":{"type":"boolean","default":true},
                    "start":{"type":"boolean","default":false,"description":"Enable and start after create"}
                }
            }),
            false,
        ),
        td(
            "cdc_profile_upsert",
            "Create a profile with a raw orchestrator body (write).",
            json!({"type":"object","required":["body"],"properties":{"body":{"type":"object"}}}),
            false,
        ),
        td(
            "cdc_profile_enable",
            "Enable or disable a profile (write).",
            json!({"type":"object","required":["id","enabled"],"properties":{"id":{"type":"string"},"enabled":{"type":"boolean"}}}),
            false,
        ),
        td(
            "cdc_profile_action",
            "start/pause/resume/restart/stop a profile (write).",
            json!({"type":"object","required":["id","action"],"properties":{"id":{"type":"string"},"action":{"type":"string","enum":["start","pause","resume","restart","stop"]}}}),
            false,
        ),
        td(
            "cdc_connector_validate",
            "Validate a connector config before create/update.",
            json!({"type":"object","required":["name","config"],"properties":{"name":{"type":"string"},"config":{"type":"object"}}}),
            false,
        ),
        td(
            "cdc_connector_create",
            "Create a Kafka Connect connector (write).",
            json!({"type":"object","required":["name","config"],"properties":{"name":{"type":"string"},"config":{"type":"object"}}}),
            false,
        ),
        td(
            "cdc_connector_update",
            "Update connector config (write).",
            json!({"type":"object","required":["name","config"],"properties":{"name":{"type":"string"},"config":{"type":"object"}}}),
            false,
        ),
        td(
            "cdc_connector_action",
            "pause/resume/restart a connector (write).",
            json!({"type":"object","required":["name","action"],"properties":{"name":{"type":"string"},"action":{"type":"string","enum":["pause","resume","restart"]}}}),
            false,
        ),
        td(
            "cdc_connector_delete",
            "Delete a connector (write).",
            json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}),
            false,
        ),
        td(
            "cdc_topic_create",
            "Create a Kafka topic (write).",
            json!({
                "type":"object","required":["name"],
                "properties":{
                    "name":{"type":"string"},
                    "partitions":{"type":"integer","default":1},
                    "replication_factor":{"type":"integer","default":1},
                    "cleanup_policy":{"type":"string","enum":["delete","compact","compact,delete"]},
                    "retention_ms":{"type":"integer"}
                }
            }),
            false,
        ),
        td(
            "cdc_topic_delete",
            "Delete a Kafka topic (write).",
            json!({"type":"object","required":["topic"],"properties":{"topic":{"type":"string"}}}),
            false,
        ),
        td(
            "cdc_topic_recreate",
            "Recreate a Kafka topic (write).",
            json!({"type":"object","required":["topic"],"properties":{"topic":{"type":"string"}}}),
            false,
        ),
        td(
            "cdc_topic_clear_messages",
            "Clear messages on a DELETE-policy topic (write).",
            json!({"type":"object","required":["topic"],"properties":{"topic":{"type":"string"}}}),
            false,
        ),
        td(
            "cdc_notification_rule_upsert",
            "Create or update a notification rule (write).",
            json!({"type":"object","required":["body"],"properties":{"id":{"type":"string"},"body":{"type":"object"}}}),
            false,
        ),
        td(
            "cdc_notification_rule_delete",
            "Delete a notification rule (write).",
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
            false,
        ),
        td(
            "cdc_processing_pipeline_upsert",
            "Create or replace an event-processing pipeline (write).",
            json!({"type":"object","required":["body"],"properties":{"id":{"type":"string"},"body":{"type":"object"}}}),
            false,
        ),
        td(
            "cdc_processing_pipeline_delete",
            "Delete an event-processing pipeline (write).",
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
            false,
        ),
    ]
}

pub fn prompts() -> Value {
    json!([
        {
            "name": "cdc_status",
            "description": "Summarize engine, connectors, topics, and alerts without using the web UI.",
            "arguments": []
        },
        {
            "name": "cdc_tables",
            "description": "List captured schemas/tables and matching Kafka topics.",
            "arguments": []
        },
        {
            "name": "cdc_inspect_records",
            "description": "Sample recent CDC records for a table or topic.",
            "arguments": [
                {"name": "table_or_topic", "description": "Table name or Kafka topic", "required": true},
                {"name": "limit", "description": "How many records", "required": false}
            ]
        },
        {
            "name": "cdc_configure_pipeline",
            "description": "Create (and optionally start) a CDC pipeline from source/target details.",
            "arguments": [
                {"name": "name", "description": "Pipeline name", "required": true},
                {"name": "schemas", "description": "Source schemas, comma-separated", "required": false},
                {"name": "tables", "description": "Optional table list", "required": false},
                {"name": "start", "description": "true to start after create", "required": false}
            ]
        }
    ])
}

pub fn prompt_description(name: &str) -> &'static str {
    match name {
        "cdc_status" => "Summarize CDC stack status.",
        "cdc_tables" => "List captured tables and topics.",
        "cdc_inspect_records" => "Sample CDC records.",
        "cdc_configure_pipeline" => "Configure a CDC pipeline via tools.",
        _ => "CDC prompt",
    }
}

pub fn prompt_messages(name: &str, arguments: &Value) -> Result<Value, ApiError> {
    let text = match name {
        "cdc_status" => "Use MCP tools (no web UI). Call cdc_overview, then cdc_alert_history with limit 20. Summarize engine health, connector states, topic counts, and any alerts. Suggest next actions.".to_string(),
        "cdc_tables" => "Use cdc_tables and cdc_profile_list. Present schemas, tables, and matching Kafka topics. If nothing is captured yet, explain how to create a pipeline with cdc_pipeline_create.".to_string(),
        "cdc_inspect_records" => {
            let topic = arguments
                .get("table_or_topic")
                .and_then(Value::as_str)
                .unwrap_or("<table_or_topic>");
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(20);
            format!("Find the Kafka topic for `{topic}` via cdc_topics_list (search) or cdc_tables. Then call cdc_topic_messages with topic, limit={limit}, mode=latest. Summarize record count and a few payloads. Do not dump secrets.")
        }
        "cdc_configure_pipeline" => {
            let name = arguments.get("name").and_then(Value::as_str).unwrap_or("demo-pipeline");
            let schemas = arguments.get("schemas").and_then(Value::as_str).unwrap_or("bookings");
            let tables = arguments.get("tables").and_then(Value::as_str).unwrap_or("");
            let start = arguments.get("start").and_then(Value::as_str).unwrap_or("false");
            format!(
                "Create CDC pipeline `{name}` without the web UI. 1) cdc_source_validate (demo defaults: host cdc-postgres-source, db/user/pass demo). 2) If source_to_target, cdc_target_validate (cdc-postgres-target). 3) cdc_pipeline_create with schemas=`{schemas}`, tables=`{tables}`, start={start}. 4) cdc_overview to confirm connectors. Ask the user for missing passwords."
            )
        }
        _ => return Err(ApiError::Message(format!("unknown prompt: {name}"))),
    };
    Ok(json!([{"role":"user","content":{"type":"text","text": text}}]))
}

pub async fn call(state: &AppState, call: ToolCall) -> Result<Value, ApiError> {
    match call.name.as_str() {
        "cdc_help" => Ok(help_catalog(state)),
        "cdc_overview" => overview(state).await,
        "cdc_health" => orch(state, Method::GET, "/health", None).await,
        "cdc_capability" => orch(state, Method::GET, "/cdc/capability", None).await,
        "cdc_migration_tools_discover" => {
            orch(state, Method::GET, "/cdc/migration-tools/discover", None).await
        }
        "cdc_migration_tool_contract" => {
            let tool_id = arg_str(&call.arguments, "tool_id")?;
            orch(
                state,
                Method::GET,
                &format!("/cdc/migration-tools/{tool_id}/contract"),
                None,
            )
            .await
        }
        "cdc_engine_status" => orch(state, Method::GET, "/cdc/engine/status", None).await,
        "cdc_cluster" => orch(state, Method::GET, "/cdc/cluster", None).await,
        "cdc_metrics" => orch(state, Method::GET, "/cdc/metrics", None).await,
        "cdc_profile_list" => orch(state, Method::GET, "/cdc/profiles", None).await,
        "cdc_profile_get" => {
            let id = arg_str(&call.arguments, "id")?;
            orch(state, Method::GET, &format!("/cdc/profiles/{id}"), None).await
        }
        "cdc_profile_dlq" => {
            let id = arg_str(&call.arguments, "id")?;
            orch(state, Method::GET, &format!("/cdc/profiles/{id}/dlq"), None).await
        }
        "cdc_tables" => tables(state).await,
        "cdc_connector_list" => orch(state, Method::GET, "/cdc/connectors", None).await,
        "cdc_connector_status" => {
            let name = arg_str(&call.arguments, "name")?;
            orch(
                state,
                Method::GET,
                &format!("/cdc/connectors/{}/status", urlencoding_simple(name)),
                None,
            )
            .await
        }
        "cdc_connector_timeline" => {
            let name = arg_str(&call.arguments, "name")?;
            orch(
                state,
                Method::GET,
                &format!("/cdc/connectors/{}/timeline", urlencoding_simple(name)),
                None,
            )
            .await
        }
        "cdc_connector_templates" => {
            orch(state, Method::GET, "/cdc/connectors/templates", None).await
        }
        "cdc_topics_list" => {
            let path = topics_query(&call.arguments);
            orch(state, Method::GET, &path, None).await
        }
        "cdc_kafka_topics" => orch(state, Method::GET, "/cdc/kafka/topics", None).await,
        "cdc_topic_messages" => {
            let topic = arg_str(&call.arguments, "topic")?;
            let mut q = vec![
                format!(
                    "limit={}",
                    call.arguments
                        .get("limit")
                        .and_then(Value::as_u64)
                        .unwrap_or(20)
                ),
                format!(
                    "mode={}",
                    call.arguments
                        .get("mode")
                        .and_then(Value::as_str)
                        .unwrap_or("latest")
                ),
            ];
            if let Some(p) = call.arguments.get("partition").and_then(Value::as_i64) {
                q.push(format!("partition={p}"));
            }
            if let Some(o) = call.arguments.get("offset").and_then(Value::as_str) {
                q.push(format!("offset={}", urlencoding_simple(o)));
            }
            orch(
                state,
                Method::GET,
                &format!(
                    "/cdc/topics/{}/messages?{}",
                    urlencoding_simple(topic),
                    q.join("&")
                ),
                None,
            )
            .await
        }
        "cdc_topic_metadata" => {
            let topic = arg_str(&call.arguments, "topic")?;
            orch(
                state,
                Method::GET,
                &format!("/cdc/topics/{}/metadata", urlencoding_simple(topic)),
                None,
            )
            .await
        }
        "cdc_consumer_groups" => {
            let mut q = vec![
                format!(
                    "page={}",
                    call.arguments
                        .get("page")
                        .and_then(Value::as_u64)
                        .unwrap_or(1)
                ),
                format!(
                    "page_size={}",
                    call.arguments
                        .get("page_size")
                        .and_then(Value::as_u64)
                        .unwrap_or(50)
                ),
            ];
            if let Some(s) = call
                .arguments
                .get("search")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                q.push(format!("search={}", urlencoding_simple(s)));
            }
            orch(
                state,
                Method::GET,
                &format!("/cdc/consumer-groups?{}", q.join("&")),
                None,
            )
            .await
        }
        "cdc_kafka_lag" => {
            let group = arg_str(&call.arguments, "group")?;
            orch(
                state,
                Method::GET,
                &format!("/cdc/kafka/groups/{}/lag", urlencoding_simple(group)),
                None,
            )
            .await
        }
        "cdc_source_validate" => {
            orch(
                state,
                Method::POST,
                "/cdc/profiles/validate/source",
                Some(db_probe_body(&call.arguments, "source")?),
            )
            .await
        }
        "cdc_target_validate" => {
            orch(
                state,
                Method::POST,
                "/cdc/profiles/validate/target",
                Some(db_probe_body(&call.arguments, "target")?),
            )
            .await
        }
        "cdc_alert_history" => {
            let limit = call
                .arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(100);
            orch(
                state,
                Method::GET,
                &format!("/cdc/notifications/history?limit={limit}"),
                None,
            )
            .await
        }
        "cdc_notification_rules_list" => {
            orch(state, Method::GET, "/cdc/notifications/rules", None).await
        }
        "cdc_logs_tail" => {
            let service = arg_str(&call.arguments, "service")?;
            let tail = call
                .arguments
                .get("tail")
                .and_then(Value::as_u64)
                .unwrap_or(200);
            orch(
                state,
                Method::GET,
                &format!("/cdc/logs/{}?tail={tail}", urlencoding_simple(service)),
                None,
            )
            .await
        }
        "cdc_processing_pipelines_list" => {
            orch(state, Method::GET, "/cdc/processing-pipelines", None).await
        }
        "cdc_processing_pipeline_get" => {
            let id = arg_str(&call.arguments, "id")?;
            orch(
                state,
                Method::GET,
                &format!("/cdc/processing-pipelines/{id}"),
                None,
            )
            .await
        }
        "cdc_engine_up" => {
            guard_write(state)?;
            orch(state, Method::POST, "/cdc/engine/up", None).await
        }
        "cdc_engine_down" => {
            guard_write(state)?;
            orch(state, Method::POST, "/cdc/engine/down", None).await
        }
        "cdc_engine_restart" => {
            guard_write(state)?;
            orch(state, Method::POST, "/cdc/engine/restart", None).await
        }
        "cdc_pipeline_create" => pipeline_create(state, &call.arguments).await,
        "cdc_profile_upsert" => {
            guard_write(state)?;
            let body = call
                .arguments
                .get("body")
                .cloned()
                .ok_or_else(|| ApiError::Message("body required".into()))?;
            orch(state, Method::POST, "/cdc/profiles", Some(body)).await
        }
        "cdc_profile_enable" => {
            guard_write(state)?;
            let id = arg_str(&call.arguments, "id")?;
            let enabled = call
                .arguments
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            orch(
                state,
                Method::POST,
                &format!("/cdc/profiles/{id}/enable"),
                Some(json!({"enabled": enabled})),
            )
            .await
        }
        "cdc_profile_action" => {
            guard_write(state)?;
            let id = arg_str(&call.arguments, "id")?;
            let action = arg_str(&call.arguments, "action")?;
            orch(
                state,
                Method::POST,
                &format!("/cdc/profiles/{id}/actions/{action}"),
                None,
            )
            .await
        }
        "cdc_connector_validate" => {
            let name = arg_str(&call.arguments, "name")?;
            let config = call
                .arguments
                .get("config")
                .cloned()
                .unwrap_or_else(|| json!({}));
            orch(
                state,
                Method::POST,
                "/cdc/connectors/validate",
                Some(json!({"name": name, "config": config})),
            )
            .await
        }
        "cdc_connector_create" => {
            guard_write(state)?;
            let name = arg_str(&call.arguments, "name")?;
            let config = call
                .arguments
                .get("config")
                .cloned()
                .unwrap_or_else(|| json!({}));
            orch(
                state,
                Method::POST,
                "/cdc/connectors",
                Some(json!({"name": name, "config": config})),
            )
            .await
        }
        "cdc_connector_update" => {
            guard_write(state)?;
            let name = arg_str(&call.arguments, "name")?;
            let config = call
                .arguments
                .get("config")
                .cloned()
                .unwrap_or_else(|| json!({}));
            orch(
                state,
                Method::PUT,
                &format!("/cdc/connectors/{}/config", urlencoding_simple(name)),
                Some(json!({"config": config})),
            )
            .await
        }
        "cdc_connector_action" => {
            guard_write(state)?;
            let name = arg_str(&call.arguments, "name")?;
            let action = arg_str(&call.arguments, "action")?;
            orch(
                state,
                Method::POST,
                &format!("/cdc/connectors/{}/{}", urlencoding_simple(name), action),
                None,
            )
            .await
        }
        "cdc_connector_delete" => {
            guard_write(state)?;
            let name = arg_str(&call.arguments, "name")?;
            orch(
                state,
                Method::DELETE,
                &format!("/cdc/connectors/{}", urlencoding_simple(name)),
                None,
            )
            .await
        }
        "cdc_topic_create" => {
            guard_write(state)?;
            orch(
                state,
                Method::POST,
                "/cdc/topics",
                Some(call.arguments.clone()),
            )
            .await
        }
        "cdc_topic_delete" => {
            guard_write(state)?;
            let topic = arg_str(&call.arguments, "topic")?;
            orch(
                state,
                Method::DELETE,
                &format!("/cdc/topics/{}", urlencoding_simple(topic)),
                None,
            )
            .await
        }
        "cdc_topic_recreate" => {
            guard_write(state)?;
            let topic = arg_str(&call.arguments, "topic")?;
            orch(
                state,
                Method::POST,
                &format!("/cdc/topics/{}/recreate", urlencoding_simple(topic)),
                None,
            )
            .await
        }
        "cdc_topic_clear_messages" => {
            guard_write(state)?;
            let topic = arg_str(&call.arguments, "topic")?;
            orch(
                state,
                Method::POST,
                &format!("/cdc/topics/{}/clear-messages", urlencoding_simple(topic)),
                None,
            )
            .await
        }
        "cdc_notification_rule_upsert" => {
            guard_write(state)?;
            let body = call
                .arguments
                .get("body")
                .cloned()
                .ok_or_else(|| ApiError::Message("body required".into()))?;
            if let Some(id) = call.arguments.get("id").and_then(Value::as_str) {
                orch(
                    state,
                    Method::PUT,
                    &format!("/cdc/notifications/rules/{id}"),
                    Some(body),
                )
                .await
            } else {
                orch(state, Method::POST, "/cdc/notifications/rules", Some(body)).await
            }
        }
        "cdc_notification_rule_delete" => {
            guard_write(state)?;
            let id = arg_str(&call.arguments, "id")?;
            orch(
                state,
                Method::DELETE,
                &format!("/cdc/notifications/rules/{id}"),
                None,
            )
            .await
        }
        "cdc_processing_pipeline_upsert" => {
            guard_write(state)?;
            let body = call
                .arguments
                .get("body")
                .cloned()
                .ok_or_else(|| ApiError::Message("body required".into()))?;
            if let Some(id) = call.arguments.get("id").and_then(Value::as_str) {
                orch(
                    state,
                    Method::PUT,
                    &format!("/cdc/processing-pipelines/{id}"),
                    Some(body),
                )
                .await
            } else {
                orch(state, Method::POST, "/cdc/processing-pipelines", Some(body)).await
            }
        }
        "cdc_processing_pipeline_delete" => {
            guard_write(state)?;
            let id = arg_str(&call.arguments, "id")?;
            orch(
                state,
                Method::DELETE,
                &format!("/cdc/processing-pipelines/{id}"),
                None,
            )
            .await
        }
        other => Err(ApiError::Message(format!("unknown tool: {other}"))),
    }
}

fn help_catalog(state: &AppState) -> Value {
    json!({
        "service": "mcp-cdc",
        "write_tools_enabled": state.cfg.allow_write,
        "how_to_prompt": [
            "What's the CDC status?",
            "Which tables are being captured?",
            "Show recent records for bookings.airports_data",
            "Create a replica (source-to-target) pipeline for schema bookings and start it",
            "Check lag for group cdc-connect-cluster"
        ],
        "read_tools": [
            "cdc_overview", "cdc_tables", "cdc_topic_messages", "cdc_topics_list",
            "cdc_profile_list", "cdc_profile_dlq", "cdc_connector_status", "cdc_kafka_lag",
            "cdc_source_validate", "cdc_target_validate", "cdc_logs_tail"
        ],
        "write_tools": [
            "cdc_pipeline_create", "cdc_profile_action", "cdc_connector_action",
            "cdc_topic_create", "cdc_engine_up"
        ]
    })
}

async fn overview(state: &AppState) -> Result<Value, ApiError> {
    let health = orch(state, Method::GET, "/health", None).await.ok();
    let capability = orch(state, Method::GET, "/cdc/capability", None).await.ok();
    let engine = orch(state, Method::GET, "/cdc/engine/status", None)
        .await
        .ok();
    let connectors = orch(state, Method::GET, "/cdc/connectors", None).await.ok();
    let profiles = orch(state, Method::GET, "/cdc/profiles", None).await.ok();
    let topics = orch(
        state,
        Method::GET,
        "/cdc/topics?page_size=50&hide_internal=true",
        None,
    )
    .await
    .ok();
    Ok(json!({
        "health": health,
        "capability": capability,
        "engine": engine,
        "connectors": connectors,
        "profiles": profiles,
        "topics": topics
    }))
}

async fn tables(state: &AppState) -> Result<Value, ApiError> {
    let profiles = orch(state, Method::GET, "/cdc/profiles", None)
        .await
        .unwrap_or(json!({}));
    let topics = orch(state, Method::GET, "/cdc/topics?page_size=200", None)
        .await
        .unwrap_or(json!({}));
    let mut captured = Vec::new();
    if let Some(list) = profiles.get("profiles").and_then(Value::as_array) {
        for profile in list {
            captured.push(json!({
                "profile": profile.get("name"),
                "id": profile.get("id"),
                "status": profile.get("status"),
                "enabled": profile.get("enabled"),
                "schemas": profile.pointer("/source/schemas"),
                "tables": profile.pointer("/source/tables"),
                "connector": profile.get("connector_name"),
                "sink": profile.get("sink_connector_name")
            }));
        }
    }
    Ok(json!({
        "captured": captured,
        "topics": topics.get("items").cloned().unwrap_or(json!([]))
    }))
}

async fn pipeline_create(state: &AppState, args: &Value) -> Result<Value, ApiError> {
    guard_write(state)?;
    let name = arg_str(args, "name")?;
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .unwrap_or("source_to_target");
    let source_type = args
        .get("source_type")
        .and_then(Value::as_str)
        .unwrap_or("postgres");
    let source_conn = connection_from(args, "source")?;
    let schemas = list_arg(args, "schemas");
    let tables = list_arg(args, "tables");
    let schema_mode = if schemas.is_empty() {
        "all_schemas"
    } else {
        "specific_schemas"
    };
    let capture_mode = args
        .get("capture_mode")
        .and_then(Value::as_str)
        .unwrap_or("logical");
    let table_mode =
        if tables.is_empty() && capture_mode != "incremental" && capture_mode != "trigger" {
            "all_tables"
        } else {
            "specific_tables"
        };
    let mut body = json!({
        "name": name,
        "description": args.get("description").and_then(Value::as_str).unwrap_or(""),
        "pattern": pattern,
        "transform_strategy": "none",
        "sink_error_policy": if pattern == "source_to_target" {
            args.get("sink_error_policy").and_then(Value::as_str).unwrap_or("fail_fast")
        } else {
            "fail_fast"
        },
        "source": {
            "source_type": source_type,
            "connection": source_conn,
            "schema_mode": schema_mode,
            "table_mode": table_mode,
            "capture_mode": capture_mode,
            "schemas": schemas,
            "tables": tables,
            "timestamp_column": args.get("timestamp_column").and_then(Value::as_str),
            "incrementing_column": args.get("incrementing_column").and_then(Value::as_str)
        },
        "flow": {
            "transforms": [],
            "schedule": {
                "delivery_mode": args.get("delivery_mode").and_then(Value::as_str).unwrap_or("realtime"),
                "interval_seconds": args.get("interval_seconds")
            },
            "mappings": []
        },
        "governance": {
            "domain": args.get("domain").and_then(Value::as_str).unwrap_or(""),
            "dataset": args.get("dataset").and_then(Value::as_str).unwrap_or(""),
            "sensitivity": args.get("sensitivity").and_then(Value::as_str).unwrap_or("internal"),
            "industry_pack": args.get("industry_pack").and_then(Value::as_str),
            "pii_fields": []
        }
    });
    attach_pipeline_destination(&mut body, args, pattern, name)?;

    let should_validate = args
        .get("validate")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut validation = json!({});
    if should_validate {
        validation["source"] = orch(
            state,
            Method::POST,
            "/cdc/profiles/validate/source",
            Some(body["source"].clone()),
        )
        .await?;
        if pattern == "source_to_target" {
            validation["target"] = orch(
                state,
                Method::POST,
                "/cdc/profiles/validate/target",
                Some(body["target"].clone()),
            )
            .await?;
        } else if pattern == "source_to_lake" {
            validation["lake"] = orch(
                state,
                Method::POST,
                "/cdc/profiles/validate/lake",
                Some(body["lake"].clone()),
            )
            .await?;
        }
    }

    let created = orch(state, Method::POST, "/cdc/profiles", Some(body)).await?;
    let id = created
        .pointer("/profile/id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let started = maybe_start_pipeline(state, args, pattern, id.as_deref()).await?;
    Ok(json!({"validation": validation, "profile": created, "started": started}))
}

fn attach_pipeline_destination(
    body: &mut Value,
    args: &Value,
    pattern: &str,
    name: &str,
) -> Result<(), ApiError> {
    if pattern == "source_to_stream" {
        body["stream_platform"] = json!(args
            .get("stream_platform")
            .and_then(Value::as_str)
            .unwrap_or("redpanda"));
        body["target"] = Value::Null;
        body["lake"] = Value::Null;
        return Ok(());
    }
    if pattern == "source_to_olake" {
        body["target"] = Value::Null;
        body["lake"] = Value::Null;
        return Ok(());
    }
    if pattern == "source_to_lake" {
        body["target"] = Value::Null;
        body["lake"] = lake_body(args, name);
        return Ok(());
    }
    let target_name = args
        .get("target_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| format!("{name}-target"));
    body["target"] = json!({
        "name": target_name,
        "target_type": args.get("target_type").and_then(Value::as_str).unwrap_or("postgres"),
        "connection": connection_from(args, "target")?
    });
    body["lake"] = Value::Null;
    Ok(())
}

fn lake_change_mode<'a>(args: &'a Value, zone: &str) -> &'a str {
    args.get("lake_change_mode")
        .and_then(Value::as_str)
        .unwrap_or(if zone == "silver" {
            "upsert"
        } else {
            "changelog"
        })
}

fn lake_body(args: &Value, name: &str) -> Value {
    let lake_name = args
        .get("lake_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| format!("{name}-lake"));
    let zone = args
        .get("lake_zone")
        .and_then(Value::as_str)
        .unwrap_or("bronze");
    json!({
        "name": lake_name,
        "table_format": args.get("lake_format").and_then(Value::as_str).unwrap_or("iceberg"),
        "zone": zone,
        "change_mode": lake_change_mode(args, zone),
        "catalog_type": args.get("lake_catalog").and_then(Value::as_str).unwrap_or("jdbc"),
        "namespace": args.get("lake_namespace").and_then(Value::as_str).unwrap_or(""),
        "warehouse": args.get("lake_warehouse").and_then(Value::as_str).unwrap_or("s3://lake/warehouse"),
        "connection": {
            "endpoint": args.get("lake_endpoint").and_then(Value::as_str).unwrap_or("http://cdc-lake:9000"),
            "bucket": args.get("lake_bucket").and_then(Value::as_str).unwrap_or("lake"),
            "region": args.get("lake_region").and_then(Value::as_str).unwrap_or("us-east-1"),
            "access_key": args.get("lake_access_key").and_then(Value::as_str).unwrap_or("cdcadmin"),
            "secret_key": args.get("lake_secret_key").and_then(Value::as_str).unwrap_or("cdcadmincdcadmin"),
            "catalog_uri": args.get("lake_catalog_uri").and_then(Value::as_str).unwrap_or("jdbc:postgresql://cdc-postgres-target:5432/demo"),
            "catalog_user": args.get("lake_catalog_user").and_then(Value::as_str).unwrap_or("demo"),
            "catalog_password": args.get("lake_catalog_password").and_then(Value::as_str).unwrap_or("demo")
        }
    })
}

async fn maybe_start_pipeline(
    state: &AppState,
    args: &Value,
    pattern: &str,
    id: Option<&str>,
) -> Result<Value, ApiError> {
    if !args.get("start").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(Value::Null);
    }
    if pattern == "source_to_olake" {
        return Ok(json!({
            "skipped": true,
            "reason": "Analytics-first (source_to_olake) cannot start yet. Draft saved. The OLake stack is Phase 3."
        }));
    }
    let Some(id) = id else {
        return Ok(Value::Null);
    };
    let _ = orch(
        state,
        Method::POST,
        &format!("/cdc/profiles/{id}/enable"),
        Some(json!({"enabled": true})),
    )
    .await?;
    orch(
        state,
        Method::POST,
        &format!("/cdc/profiles/{id}/actions/start"),
        None,
    )
    .await
}

fn connection_from(args: &Value, side: &str) -> Result<Value, ApiError> {
    let host_key = format!("{side}_host");
    let port_key = format!("{side}_port");
    let db_key = format!("{side}_database");
    let user_key = format!("{side}_username");
    let pass_key = format!("{side}_password");
    let default_host = if side == "source" {
        "cdc-postgres-source"
    } else {
        "cdc-postgres-target"
    };
    let host = args
        .get(&host_key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_host);
    let port = args
        .get(&port_key)
        .and_then(|v| match v {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "5432".to_string());
    let database = args
        .get(&db_key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("demo");
    let username = args
        .get(&user_key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("demo");
    let password = args
        .get(&pass_key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("demo");
    Ok(json!({
        "host": host,
        "port": port,
        "database": database,
        "username": username,
        "password": password
    }))
}

fn db_probe_body(args: &Value, side: &str) -> Result<Value, ApiError> {
    let type_key = if side == "source" {
        "source_type"
    } else {
        "target_type"
    };
    let db_type = args
        .get(type_key)
        .and_then(Value::as_str)
        .unwrap_or("postgres");
    let default_host = if side == "source" {
        "cdc-postgres-source"
    } else {
        "cdc-postgres-target"
    };
    let host = args
        .get("host")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_host);
    let port = args
        .get("port")
        .and_then(|v| match v {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "5432".to_string());
    let database = args
        .get("database")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("demo");
    let username = args
        .get("username")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("demo");
    let password = args
        .get("password")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("demo");
    let mut body = json!({
        "connection": {
            "host": host,
            "port": port,
            "database": database,
            "username": username,
            "password": password
        }
    });
    if side == "source" {
        body["source_type"] = json!(db_type);
        body["schema_mode"] = json!("all_schemas");
        body["schemas"] = json!([]);
        body["tables"] = json!([]);
    } else {
        body["target_type"] = json!(db_type);
    }
    Ok(body)
}

fn list_arg(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn topics_query(args: &Value) -> String {
    let mut q = vec![
        format!(
            "page={}",
            args.get("page").and_then(Value::as_u64).unwrap_or(1)
        ),
        format!(
            "page_size={}",
            args.get("page_size").and_then(Value::as_u64).unwrap_or(50)
        ),
    ];
    if let Some(s) = args
        .get("search")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        q.push(format!("search={}", urlencoding_simple(s)));
    }
    if args
        .get("hide_internal")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        q.push("hide_internal=true".into());
    }
    if let Some(s) = args.get("sort_by").and_then(Value::as_str) {
        q.push(format!("sort_by={s}"));
    }
    if let Some(s) = args.get("sort_order").and_then(Value::as_str) {
        q.push(format!("sort_order={s}"));
    }
    format!("/cdc/topics?{}", q.join("&"))
}
