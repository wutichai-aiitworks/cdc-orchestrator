use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use reqwest::Method;
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use uuid::Uuid;

use crate::audit::log_action;
use crate::command::run_docker;
use crate::connector_catalog;
use crate::connector_meta::{self, ConnectorKind};
use crate::engine;
use crate::error::AppError;
use crate::models::{
    ActionResponse, ConnectorCatalogResponse, ConnectorCatalogSchema, ConnectorCatalogTable,
    ConnectorConfigResponse, ConnectorConnectionPublic, ConnectorCreateRequest,
    ConnectorInspectRequest, ConnectorProbeResponse, ConnectorStatusResponse, ConnectorTemplate,
    ConnectorTemplatesResponse, ConnectorTimelineEntry, ConnectorTimelineResponse,
    ConnectorTimelineTaskError, ConnectorUpdateRequest, ConnectorValidateRequest,
    ConnectorValidateResponse, ConnectorValidationIssue, ConnectorsResponse, ConsumerGroupItem,
    CopyDataRequest, CopyDataResponse, CopyTableResult, DeliveryMode, EngineStatusResponse,
    ExecDdlRequest, GroupLagResponse, GroupsResponse, HealthResponse, HistoryItem, LakeDestination,
    PagedGroupsResponse, PaginationQuery, ProfileCaseRequest, ProfileEnableRequest, ProfilePattern,
    ProfileResponse, ProfileStatus, ProfileUpsertRequest, ProfilesResponse, SchemaCompareProfile,
    SchemaCompareProfileRequest, SchemaCompareProfilesResponse, SchemaCountsRequest,
    SchemaCountsResponse, SchemaSelectionMode, SourceCatalogRequest, SourceCatalogResponse,
    SourceConfig, SourceTableInspectRequest, StreamPlatform, TableCountRequest, TableCountResponse,
    TablePreviewRequest, TablePreviewResponse, TableSelectionMode, TargetCatalogRequest,
    TargetConfig, TargetTableInspectRequest, TopicItem, TopicMessageItem, TopicMessagesResponse,
    TopicMetadataResponse, TopicPartitionMetadata, TopicsResponse, TransformConfig,
    TransformStrategy, TriggerInstallRequest, TriggerInstallResponse, TriggerPreviewResponse,
    TriggerStatusResponse, TriggerStatusTable, ValidationResponse,
};
use crate::reconcile::{self, connect_request};
use crate::AppState;

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        service: "hyper-cdc-orchestrator".to_string(),
        status: "ok".to_string(),
        enabled: state.config.enabled,
        timestamp: Utc::now(),
    })
}

pub async fn engine_status(
    State(state): State<AppState>,
) -> Result<Json<EngineStatusResponse>, AppError> {
    let (engine_target, services) = engine::engine_status(&state.config).await?;
    Ok(Json(EngineStatusResponse {
        engine_target,
        services,
        timestamp: Utc::now(),
    }))
}

pub async fn engine_up(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ActionResponse>, AppError> {
    mutate_engine_action(state, headers, "engine.up").await
}

pub async fn engine_down(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ActionResponse>, AppError> {
    mutate_engine_action(state, headers, "engine.down").await
}

pub async fn engine_restart(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ActionResponse>, AppError> {
    mutate_engine_action(state, headers, "engine.restart").await
}

pub async fn list_connectors(
    State(state): State<AppState>,
) -> Result<Json<ConnectorsResponse>, AppError> {
    let v = connect_request(
        &state,
        Method::GET,
        "/connectors?expand=status&expand=info",
        None,
    )
    .await?;
    Ok(Json(ConnectorsResponse {
        connectors: v,
        timestamp: Utc::now(),
    }))
}

pub async fn connector_status(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<ConnectorStatusResponse>, AppError> {
    let path = format!("/connectors/{name}/status");
    let status = connect_request(&state, Method::GET, &path, None).await?;
    Ok(Json(ConnectorStatusResponse {
        name,
        status,
        timestamp: Utc::now(),
    }))
}

pub async fn connector_timeline(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<ConnectorTimelineResponse> {
    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let name_norm = name.trim().to_ascii_lowercase();
    let mut items = Vec::<ConnectorTimelineEntry>::new();
    for entry in state.history.read().await.list_recent_owned().into_iter() {
        if entry.payload.connector.to_ascii_lowercase() != name_norm {
            continue;
        }
        items.push(ConnectorTimelineEntry {
            ts: entry.payload.ts,
            connector: entry.payload.connector,
            from: entry.payload.from,
            to: entry.payload.to,
            rule: entry.payload.rule,
            task_errors: entry
                .payload
                .task_errors
                .into_iter()
                .map(|te| ConnectorTimelineTaskError {
                    task_id: te.task_id,
                    trace: te.trace,
                })
                .collect(),
            resolved: entry.resolved,
        });
        if items.len() >= limit {
            break;
        }
    }
    Json(ConnectorTimelineResponse {
        connector: name,
        total: items.len(),
        items,
        timestamp: Utc::now(),
    })
}

pub async fn create_connector(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ConnectorCreateRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest(
            "connector name is required".to_string(),
        ));
    }
    if payload.config.is_empty() {
        return Err(AppError::BadRequest(
            "connector config must not be empty".to_string(),
        ));
    }

    let actor = actor_from_headers(&headers);
    let body = json!({
        "name": payload.name,
        "config": payload.config,
    });

    connect_request(&state, Method::POST, "/connectors", Some(body)).await?;
    let audit_id = log_action(&actor, "connect.create", "success", "connector created");

    Ok(Json(ActionResponse {
        action: "connect.create".to_string(),
        status: "success".to_string(),
        message: "connector created".to_string(),
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn update_connector(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<ConnectorUpdateRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    if payload.config.is_empty() {
        return Err(AppError::BadRequest(
            "connector config must not be empty".to_string(),
        ));
    }
    let actor = actor_from_headers(&headers);

    let path = format!("/connectors/{name}/config");
    connect_request(
        &state,
        Method::PUT,
        &path,
        Some(serde_json::to_value(payload.config).map_err(|e| AppError::Internal(e.to_string()))?),
    )
    .await?;
    let audit_id = log_action(
        &actor,
        "connect.update",
        "success",
        &format!("connector={name}"),
    );

    Ok(Json(ActionResponse {
        action: "connect.update".to_string(),
        status: "success".to_string(),
        message: format!("connector {name} updated"),
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn get_connector_config(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<ConnectorConfigResponse>, AppError> {
    let path = format!("/connectors/{}/config", urlencoding::encode(&name));
    let config = connect_request(&state, Method::GET, &path, None).await?;
    Ok(Json(ConnectorConfigResponse {
        name,
        config,
        timestamp: Utc::now(),
    }))
}

pub async fn delete_connector(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ActionResponse>, AppError> {
    let actor = actor_from_headers(&headers);
    let path = format!("/connectors/{name}");
    connect_request(&state, Method::DELETE, &path, None).await?;
    let audit_id = log_action(
        &actor,
        "connect.delete",
        "success",
        &format!("connector={name}"),
    );

    Ok(Json(ActionResponse {
        action: "connect.delete".to_string(),
        status: "success".to_string(),
        message: format!("connector {name} deleted"),
        audit_id,
        timestamp: Utc::now(),
    }))
}

/// Rewrite Kafka Connect's rejection of an offset reset into a UI-actionable hint.
///
/// Connect answers with a curl command, which is unhelpful from a web UI where the
/// equivalent is a button.
fn explain_connector_action_error(name: &str, action: &str, err: AppError) -> AppError {
    let raw = err.to_string_message();
    if action == "reset-offsets" && raw.contains("STOPPED state") {
        return AppError::BadRequest(format!(
            "connector '{name}' must be stopped before its offsets can be reset — \
             press Stop first, or use the pipeline's Resync action to do the whole sequence"
        ));
    }
    err
}

pub async fn connector_action(
    State(state): State<AppState>,
    Path((name, action)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<ActionResponse>, AppError> {
    let connect_action = match action.as_str() {
        "pause" => ("pause", Method::PUT),
        "resume" => ("resume", Method::PUT),
        "restart" => ("restart", Method::POST),
        "stop" => ("stop", Method::PUT),
        // Clears the connector's stored position so a source re-reads from the start.
        "reset-offsets" => ("offsets", Method::DELETE),
        _ => return Err(AppError::BadRequest("unsupported action".to_string())),
    };

    let actor = actor_from_headers(&headers);
    let path = format!("/connectors/{name}/{}", connect_action.0);
    connect_request(&state, connect_action.1, &path, None)
        .await
        .map_err(|e| explain_connector_action_error(&name, &action, e))?;
    let audit_id = log_action(
        &actor,
        &format!("connect.{action}"),
        "success",
        &format!("connector={name}"),
    );

    Ok(Json(ActionResponse {
        action: format!("connect.{action}"),
        status: "success".to_string(),
        message: format!("connector {name} {action} request accepted"),
        audit_id,
        timestamp: Utc::now(),
    }))
}

/// GET /connectors/plugins — lists all available connector plugins from Kafka Connect
pub async fn list_connector_plugins(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let body = connect_request(&state, Method::GET, "/connector-plugins", None).await?;
    Ok(Json(body))
}

/// GET /connectors/:name/topics — subscribed topics for a connector
pub async fn connector_topics(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let path = format!("/connectors/{name}/topics");
    let body = connect_request(&state, Method::GET, &path, None).await?;
    Ok(Json(body))
}

pub async fn connector_templates() -> Json<ConnectorTemplatesResponse> {
    let templates = vec![
        ConnectorTemplate {
            id: "postgres-source".to_string(),
            name: "Postgres Source (Debezium)".to_string(),
            kind: "source".to_string(),
            database_type: "postgres".to_string(),
            description: "Capture CDC changes from PostgreSQL into Kafka/Redpanda topics."
                .to_string(),
            config: HashMap::from([
                (
                    "connector.class".to_string(),
                    "io.debezium.connector.postgresql.PostgresConnector".to_string(),
                ),
                ("tasks.max".to_string(), "1".to_string()),
                ("plugin.name".to_string(), "pgoutput".to_string()),
                ("slot.name".to_string(), "debezium_slot".to_string()),
                (
                    "publication.name".to_string(),
                    "dbz_publication".to_string(),
                ),
                ("topic.prefix".to_string(), "demo".to_string()),
                (
                    "database.hostname".to_string(),
                    "cdc-postgres-source".to_string(),
                ),
                ("database.port".to_string(), "5432".to_string()),
                ("database.user".to_string(), "demo".to_string()),
                ("database.password".to_string(), "demo".to_string()),
                ("database.dbname".to_string(), "demo".to_string()),
                ("schema.include.list".to_string(), "bookings".to_string()),
            ]),
        },
        ConnectorTemplate {
            id: "mysql-source".to_string(),
            name: "MySQL Source (Debezium)".to_string(),
            kind: "source".to_string(),
            database_type: "mysql".to_string(),
            description: "Capture CDC changes from MySQL into Kafka/Redpanda topics.".to_string(),
            config: HashMap::from([
                (
                    "connector.class".to_string(),
                    "io.debezium.connector.mysql.MySqlConnector".to_string(),
                ),
                ("tasks.max".to_string(), "1".to_string()),
                ("topic.prefix".to_string(), "demo".to_string()),
                ("database.hostname".to_string(), "mysql".to_string()),
                ("database.port".to_string(), "3306".to_string()),
                ("database.user".to_string(), "debezium".to_string()),
                ("database.password".to_string(), "dbz".to_string()),
                ("database.include.list".to_string(), "inventory".to_string()),
                (
                    "schema.history.internal.kafka.bootstrap.servers".to_string(),
                    "redpanda:9092".to_string(),
                ),
                (
                    "schema.history.internal.kafka.topic".to_string(),
                    "schema-changes.inventory".to_string(),
                ),
            ]),
        },
        ConnectorTemplate {
            id: "jdbc-source".to_string(),
            name: "JDBC Source (trigger changelog)".to_string(),
            kind: "source".to_string(),
            database_type: "postgres".to_string(),
            description: "Poll hyper_cdc.change_log when WAL/binlog CDC is not available. Used for capture_mode=trigger."
                .to_string(),
            config: HashMap::from([
                (
                    "connector.class".to_string(),
                    "io.confluent.connect.jdbc.JdbcSourceConnector".to_string(),
                ),
                ("tasks.max".to_string(), "1".to_string()),
                (
                    "connection.url".to_string(),
                    "jdbc:postgresql://cdc-postgres-source:5432/demo".to_string(),
                ),
                ("connection.user".to_string(), "demo".to_string()),
                ("connection.password".to_string(), "demo".to_string()),
                ("mode".to_string(), "incrementing".to_string()),
                ("incrementing.column.name".to_string(), "id".to_string()),
                ("table.include.list".to_string(), "hyper_cdc.change_log".to_string()),
                ("topic.prefix".to_string(), "demo_".to_string()),
                ("numeric.mapping".to_string(), "best_fit".to_string()),
                ("validate.non.null".to_string(), "false".to_string()),
                ("poll.interval.ms".to_string(), "1000".to_string()),
            ]),
        },
        ConnectorTemplate {
            id: "jdbc-source-incremental".to_string(),
            name: "JDBC Source (incremental)".to_string(),
            kind: "source".to_string(),
            database_type: "postgres".to_string(),
            description: "Poll source tables on a timestamp/id watermark when WAL is unavailable and triggers are too expensive. Used for capture_mode=incremental. Hard deletes are not captured."
                .to_string(),
            config: HashMap::from([
                (
                    "connector.class".to_string(),
                    "io.confluent.connect.jdbc.JdbcSourceConnector".to_string(),
                ),
                ("tasks.max".to_string(), "1".to_string()),
                (
                    "connection.url".to_string(),
                    "jdbc:postgresql://cdc-postgres-source:5432/demo".to_string(),
                ),
                ("connection.user".to_string(), "demo".to_string()),
                ("connection.password".to_string(), "demo".to_string()),
                ("mode".to_string(), "timestamp+incrementing".to_string()),
                ("table.include.list".to_string(), "bookings\\.tickets".to_string()),
                ("timestamp.columns.mapping".to_string(), ".*:[updated_at]".to_string()),
                ("incrementing.column.mapping".to_string(), ".*:id".to_string()),
                ("topic.prefix".to_string(), "demo.".to_string()),
                ("numeric.mapping".to_string(), "best_fit".to_string()),
                ("validate.non.null".to_string(), "true".to_string()),
                ("poll.interval.ms".to_string(), "300000".to_string()),
            ]),
        },
        ConnectorTemplate {
            id: "jdbc-sink".to_string(),
            name: "JDBC Sink (Debezium)".to_string(),
            kind: "sink".to_string(),
            database_type: "postgres".to_string(),
            description: "Apply CDC topic events into a target relational database.".to_string(),
            config: HashMap::from([
                (
                    "connector.class".to_string(),
                    "io.debezium.connector.jdbc.JdbcSinkConnector".to_string(),
                ),
                ("tasks.max".to_string(), "1".to_string()),
                (
                    "topics.regex".to_string(),
                    "demo\\.bookings\\..*".to_string(),
                ),
                (
                    "connection.url".to_string(),
                    "jdbc:postgresql://cdc-postgres-target:5432/demo?currentSchema=bookings"
                        .to_string(),
                ),
                ("connection.username".to_string(), "demo".to_string()),
                ("connection.password".to_string(), "demo".to_string()),
                ("insert.mode".to_string(), "upsert".to_string()),
                ("delete.enabled".to_string(), "true".to_string()),
                ("primary.key.mode".to_string(), "record_key".to_string()),
                ("schema.evolution".to_string(), "basic".to_string()),
            ]),
        },
    ];
    Json(ConnectorTemplatesResponse {
        templates,
        timestamp: Utc::now(),
    })
}

pub async fn connector_validate(
    State(state): State<AppState>,
    Json(payload): Json<ConnectorValidateRequest>,
) -> Result<Json<ConnectorValidateResponse>, AppError> {
    let mut errors = Vec::<ConnectorValidationIssue>::new();
    let mut warnings = Vec::<String>::new();
    let mut upstream = None;

    if payload.config.is_empty() {
        errors.push(ConnectorValidationIssue {
            field: "config".to_string(),
            message: "connector config must not be empty".to_string(),
        });
    }
    let class_name = payload
        .config
        .get("connector.class")
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    if class_name.is_empty() {
        errors.push(ConnectorValidationIssue {
            field: "connector.class".to_string(),
            message: "connector.class is required".to_string(),
        });
    }
    let tasks_max = payload
        .config
        .get("tasks.max")
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    if tasks_max.is_empty() {
        warnings.push("tasks.max is missing; recommended to set explicit task count".to_string());
    } else if tasks_max.parse::<u32>().ok().filter(|v| *v > 0).is_none() {
        errors.push(ConnectorValidationIssue {
            field: "tasks.max".to_string(),
            message: "tasks.max must be a positive integer".to_string(),
        });
    }

    let normalized_name = payload
        .name
        .as_ref()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| {
            payload
                .config
                .get("name")
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
        });
    if normalized_name.is_none() {
        warnings
            .push("name is not provided; UI should set connector name before create".to_string());
    }

    if !class_name.is_empty() {
        let path = format!(
            "/connector-plugins/{}/config/validate",
            urlencoding::encode(&class_name)
        );
        match connect_request(
            &state,
            Method::PUT,
            &path,
            Some(
                serde_json::to_value(&payload.config)
                    .map_err(|e| AppError::Internal(e.to_string()))?,
            ),
        )
        .await
        {
            Ok(v) => {
                if v.get("error_count").and_then(|n| n.as_u64()).unwrap_or(0) > 0 {
                    warnings.push(
                        "Kafka Connect plugin validation reported configuration issues".to_string(),
                    );
                }
                upstream = Some(v);
            }
            Err(e) => {
                warnings.push(format!(
                    "Kafka Connect plugin validation unavailable: {}",
                    e.to_string_message()
                ));
            }
        }
    }

    let valid = errors.is_empty();
    Ok(Json(ConnectorValidateResponse {
        valid,
        normalized_name,
        errors,
        warnings,
        upstream,
        timestamp: Utc::now(),
    }))
}

pub async fn connector_probe(
    State(state): State<AppState>,
    Json(payload): Json<ConnectorInspectRequest>,
) -> Result<Json<ConnectorProbeResponse>, AppError> {
    let config = resolve_connector_config(&state, &payload).await?;
    let insight = connector_meta::insight_from_config(&config);
    if insight.connection.host.is_empty() {
        return Err(AppError::BadRequest(
            "could not determine database host from connector config".to_string(),
        ));
    }
    let fields = connector_meta::connection_fields(&insight);
    let location = kind_label(insight.kind);
    let (reachable, message) = match probe_connection(&state, &fields, location).await {
        Ok(message) => (true, message),
        Err(e) => (false, e.to_string_message()),
    };
    Ok(Json(ConnectorProbeResponse {
        valid: reachable,
        reachable,
        kind: kind_label(insight.kind).to_string(),
        database_type: insight.database_type.clone(),
        database_label: insight.database_label.clone(),
        class_name: insight.class_name.clone(),
        connection: public_connection(&insight),
        configured_schemas: insight.configured_schemas.clone(),
        configured_tables: insight.configured_tables.clone(),
        message,
        timestamp: Utc::now(),
    }))
}

pub async fn connector_catalog(
    State(state): State<AppState>,
    Json(payload): Json<ConnectorInspectRequest>,
) -> Result<Json<ConnectorCatalogResponse>, AppError> {
    let config = resolve_connector_config(&state, &payload).await?;
    let insight = connector_meta::insight_from_config(&config);
    let schema_filter = payload
        .schema
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let fields = connector_meta::connection_fields(&insight);
    let location = kind_label(insight.kind);

    let mut reachable = false;
    let mut live = false;
    let mut message = if insight.connection.host.is_empty() {
        "could not determine database host from connector config".to_string()
    } else {
        match probe_connection(&state, &fields, location).await {
            Ok(ok) => {
                reachable = true;
                ok
            }
            Err(e) => e.to_string_message(),
        }
    };

    let mut schemas: Vec<ConnectorCatalogSchema> = Vec::new();
    let mut tables: Vec<ConnectorCatalogTable> = Vec::new();

    if reachable {
        match connector_catalog::load_live_catalog(&insight).await {
            Ok(catalog) => {
                live = true;
                message = format!(
                    "{message}. Live catalog: {} schemas, {} tables.",
                    catalog.schemas.len(),
                    catalog.tables.len()
                );
                push_live_catalog(&insight, &schema_filter, catalog, &mut schemas, &mut tables);
            }
            Err(e) => {
                message = format!(
                    "{message}. Live catalog unavailable: {}",
                    e.to_string_message()
                );
            }
        }
    }

    if schemas.is_empty() && tables.is_empty() {
        push_configured_catalog(&insight, &schema_filter, &mut schemas, &mut tables);
    }

    Ok(Json(ConnectorCatalogResponse {
        kind: kind_label(insight.kind).to_string(),
        database_type: insight.database_type.clone(),
        database_label: insight.database_label.clone(),
        class_name: insight.class_name.clone(),
        connection: public_connection(&insight),
        reachable,
        live,
        message,
        configured_schemas: insight.configured_schemas.clone(),
        configured_tables: insight.configured_tables.clone(),
        schemas,
        tables,
        timestamp: Utc::now(),
    }))
}

fn schema_skipped(filter: &Option<String>, name: &str) -> bool {
    filter
        .as_ref()
        .is_some_and(|wanted| !name.eq_ignore_ascii_case(wanted))
}

fn push_live_catalog(
    insight: &connector_meta::ConnectorInsight,
    schema_filter: &Option<String>,
    catalog: connector_catalog::LiveCatalog,
    schemas: &mut Vec<ConnectorCatalogSchema>,
    tables: &mut Vec<ConnectorCatalogTable>,
) {
    for name in catalog.schemas {
        if schema_skipped(schema_filter, &name) {
            continue;
        }
        let table_count = catalog
            .tables
            .iter()
            .filter(|table| table.schema == name)
            .count();
        schemas.push(ConnectorCatalogSchema {
            captured: is_schema_captured(insight, &name),
            name,
            table_count,
        });
    }
    for table in catalog.tables {
        if schema_skipped(schema_filter, &table.schema) {
            continue;
        }
        tables.push(ConnectorCatalogTable {
            captured: is_table_captured(insight, &table.schema, &table.name),
            schema: table.schema,
            name: table.name,
            row_count: table.row_count,
            primary_key: table.primary_key,
        });
    }
}

fn table_belongs_to_schema(qualified: &str, name: &str, schema_filter: &Option<String>) -> bool {
    qualified.starts_with(&format!("{name}."))
        || (!qualified.contains('.') && schema_filter.as_deref() == Some(name))
}

fn split_configured_table(
    qualified: &str,
    insight: &connector_meta::ConnectorInsight,
) -> (String, String) {
    match qualified.split_once('.') {
        Some((schema, name)) => (schema.to_string(), name.to_string()),
        None => (
            insight
                .configured_schemas
                .first()
                .cloned()
                .unwrap_or_default(),
            qualified.to_string(),
        ),
    }
}

fn push_configured_catalog(
    insight: &connector_meta::ConnectorInsight,
    schema_filter: &Option<String>,
    schemas: &mut Vec<ConnectorCatalogSchema>,
    tables: &mut Vec<ConnectorCatalogTable>,
) {
    for name in &insight.configured_schemas {
        if schema_skipped(schema_filter, name) {
            continue;
        }
        let table_count = insight
            .configured_tables
            .iter()
            .filter(|t| table_belongs_to_schema(t, name, schema_filter))
            .count();
        schemas.push(ConnectorCatalogSchema {
            name: name.clone(),
            table_count,
            captured: true,
        });
    }
    for qualified in &insight.configured_tables {
        let (schema, name) = split_configured_table(qualified, insight);
        if schema_skipped(schema_filter, &schema) {
            continue;
        }
        tables.push(ConnectorCatalogTable {
            schema,
            name,
            captured: true,
            row_count: None,
            primary_key: Vec::new(),
        });
    }
}

async fn resolve_connector_config(
    state: &AppState,
    payload: &ConnectorInspectRequest,
) -> Result<HashMap<String, String>, AppError> {
    let name = payload
        .name
        .as_ref()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let mut config = payload.config.clone().unwrap_or_default();
    if let Some(name) = name {
        let path = format!("/connectors/{}/config", urlencoding::encode(&name));
        let remote = connect_request(state, Method::GET, &path, None).await?;
        let mut merged = connector_meta::config_from_json(&remote);
        for (key, value) in config {
            if !value.trim().is_empty() && value != "***" {
                merged.insert(key, value);
            }
        }
        config = merged;
    }
    if config.is_empty() {
        return Err(AppError::BadRequest(
            "connector name or config is required".to_string(),
        ));
    }
    Ok(config)
}

fn public_connection(insight: &connector_meta::ConnectorInsight) -> ConnectorConnectionPublic {
    ConnectorConnectionPublic {
        host: insight.connection.host.clone(),
        port: insight.connection.port.clone(),
        database: insight.connection.database.clone(),
        username: insight.connection.username.clone(),
        jdbc_url: insight.connection.jdbc_url.clone(),
    }
}

fn kind_label(kind: ConnectorKind) -> &'static str {
    match kind {
        ConnectorKind::Source => "source",
        ConnectorKind::Sink => "sink",
    }
}

fn is_schema_captured(insight: &connector_meta::ConnectorInsight, schema: &str) -> bool {
    insight.configured_schemas.is_empty()
        || insight
            .configured_schemas
            .iter()
            .any(|item| item.eq_ignore_ascii_case(schema))
}

fn is_table_captured(
    insight: &connector_meta::ConnectorInsight,
    schema: &str,
    table: &str,
) -> bool {
    if insight.configured_tables.is_empty() {
        return is_schema_captured(insight, schema);
    }
    let qualified = format!("{schema}.{table}");
    insight
        .configured_tables
        .iter()
        .any(|item| item.eq_ignore_ascii_case(&qualified) || item.eq_ignore_ascii_case(table))
}

pub async fn kafka_topics(State(state): State<AppState>) -> Result<Json<TopicsResponse>, AppError> {
    let output = run_docker(
        &state.config,
        &[
            "exec",
            state.config.redpanda_container.as_str(),
            "rpk",
            "topic",
            "list",
            "-i",
        ],
    )
    .await?;
    let topics = parse_rpk_topic_list(&output);

    Ok(Json(TopicsResponse {
        topics,
        timestamp: Utc::now(),
    }))
}

pub async fn kafka_groups(State(state): State<AppState>) -> Result<Json<GroupsResponse>, AppError> {
    let output = run_docker(
        &state.config,
        &[
            "exec",
            state.config.redpanda_container.as_str(),
            "rpk",
            "group",
            "list",
        ],
    )
    .await?;
    let groups = parse_rpk_group_list(&output);

    Ok(Json(GroupsResponse {
        groups,
        timestamp: Utc::now(),
    }))
}

pub async fn kafka_group_lag(
    State(state): State<AppState>,
    Path(group): Path<String>,
) -> Result<Json<GroupLagResponse>, AppError> {
    let output = run_docker(
        &state.config,
        &[
            "exec",
            state.config.redpanda_container.as_str(),
            "rpk",
            "group",
            "describe",
            group.as_str(),
        ],
    )
    .await?;
    let mut parsed = crate::kafka_admin::parse_group_describe(&output);
    crate::kafka_admin::augment_connect_group(
        &state,
        &group,
        &mut parsed.entries,
        &mut parsed.total_lag,
    )
    .await;
    Ok(Json(GroupLagResponse {
        group,
        total_lag: parsed.total_lag,
        entries: parsed.entries,
        timestamp: Utc::now(),
    }))
}

pub async fn list_consumer_groups_paged(
    State(state): State<AppState>,
    Query(query): Query<PaginationQuery>,
) -> Result<Json<PagedGroupsResponse>, AppError> {
    let output = run_docker(
        &state.config,
        &[
            "exec",
            state.config.redpanda_container.as_str(),
            "rpk",
            "group",
            "list",
        ],
    )
    .await?;

    let mut items: Vec<ConsumerGroupItem> = parse_rpk_group_list(&output)
        .into_iter()
        .map(|group| ConsumerGroupItem {
            group,
            state: None,
            members: None,
            topic_count: None,
            coordinator: None,
            total_lag: None,
        })
        .collect();
    if let Some(search) = query.search.as_ref().map(|s| s.trim().to_ascii_lowercase()) {
        if !search.is_empty() {
            items.retain(|g| g.group.to_ascii_lowercase().contains(&search));
        }
    }

    let total = items.len();
    let (page, page_size, mut paged_items) = apply_pagination(items, query.page, query.page_size);
    for item in &mut paged_items {
        if let Ok(raw) = run_docker(
            &state.config,
            &[
                "exec",
                state.config.redpanda_container.as_str(),
                "rpk",
                "group",
                "describe",
                item.group.as_str(),
            ],
        )
        .await
        {
            let mut parsed = crate::kafka_admin::parse_group_describe(&raw);
            crate::kafka_admin::augment_connect_group(
                &state,
                &item.group,
                &mut parsed.entries,
                &mut parsed.total_lag,
            )
            .await;
            item.state = parsed.state;
            item.members = parsed.members;
            item.coordinator = parsed.coordinator;
            item.total_lag = Some(parsed.total_lag);
            let unique_topics: std::collections::HashSet<&str> =
                parsed.entries.iter().map(|e| e.topic.as_str()).collect();
            item.topic_count = Some(unique_topics.len());
        }
    }
    if let Some(state_filter) = query.state.as_ref().map(|s| s.trim().to_ascii_lowercase()) {
        if !state_filter.is_empty() {
            paged_items.retain(|g| {
                g.state
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&state_filter)
            });
        }
    }

    Ok(Json(PagedGroupsResponse {
        items: paged_items,
        total,
        page,
        page_size,
        timestamp: Utc::now(),
    }))
}

pub async fn topic_messages(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<TopicMessagesResponse>, AppError> {
    let mode = query
        .get("mode")
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "latest".to_string());
    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(20)
        .clamp(1, 200);
    let requested_offset = query.get("offset").map(|s| s.trim().to_ascii_lowercase());
    let specific_partition = query.get("partition").map(|s| s.trim().to_string());

    let include_meta = query
        .get("include_meta")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(true);

    // rpk topic consume --offset end/start never exits on its own when fewer messages
    // exist than the requested -n limit.  We always:
    //   1. Fetch per-partition high-watermarks
    //   2. Derive a *numeric* start offset (never the literal "end")
    //   3. Clamp -n to the number of actually available messages so rpk exits promptly.
    let hw_map = partition_high_watermarks(&state, &topic, specific_partition.as_deref()).await;
    // total high-watermark and log-start across relevant partitions
    let total_hw: i64 = hw_map.values().copied().sum();
    // For simplicity we treat log_start_offset = 0 when unknown (safe under-estimate)
    let total_log_start: i64 = 0;
    let available: i64 = (total_hw - total_log_start).max(0);

    let (effective_offset, effective_limit): (String, usize) = {
        let wants_latest = requested_offset.as_deref() == Some("end")
            || mode == "latest"
            || requested_offset.is_none();
        if wants_latest {
            let start = (total_hw - limit as i64).max(0);
            let actual_n = (total_hw - start) as usize;
            (start.to_string(), actual_n.max(1).min(limit))
        } else if requested_offset.as_deref() == Some("start") {
            let actual_n = available.min(limit as i64) as usize;
            ("start".to_string(), actual_n.max(1).min(limit))
        } else {
            // Numeric explicit offset — still clamp to avoid hanging
            let start: i64 = requested_offset
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let actual_n = (total_hw - start).max(0).min(limit as i64) as usize;
            (start.to_string(), actual_n.max(1).min(limit))
        }
    };
    let limit_s = effective_limit.to_string();

    let mut args = vec![
        "exec".to_string(),
        state.config.redpanda_container.clone(),
        "rpk".to_string(),
        "topic".to_string(),
        "consume".to_string(),
        topic.clone(),
        "-n".to_string(),
        limit_s.clone(),
        "--offset".to_string(),
        effective_offset.clone(),
    ];
    if let Some(ref p) = specific_partition {
        args.push("-p".to_string());
        args.push(p.clone());
    }
    // NOTE: --use-schema-registry routes rpk output to stderr, not stdout.
    // The messages are Debezium JSON (not Avro-encoded on the wire), so no
    // schema-registry deserialization is needed here.

    let output = if include_meta {
        let mut with_meta = args.clone();
        with_meta.push("--format".to_string());
        // Each record on its own line; rpk interprets \n as newline
        with_meta.push("%k\t%p\t%o\t%d\t%v\n".to_string());
        let meta_refs: Vec<&str> = with_meta.iter().map(String::as_str).collect();
        match run_docker(&state.config, &meta_refs).await {
            Ok(v) => v,
            Err(_) => {
                let mut value_only = args.clone();
                value_only.push("--format".to_string());
                value_only.push("%v\n".to_string());
                let val_refs: Vec<&str> = value_only.iter().map(String::as_str).collect();
                run_docker(&state.config, &val_refs).await?
            }
        }
    } else {
        let mut value_only = args.clone();
        value_only.push("--format".to_string());
        value_only.push("%v\n".to_string());
        let val_refs: Vec<&str> = value_only.iter().map(String::as_str).collect();
        run_docker(&state.config, &val_refs).await?
    };

    let items = output
        .lines()
        .filter_map(parse_topic_message_line)
        .filter(|item| {
            let Some(q) = query.get("q").map(|s| s.trim().to_ascii_lowercase()) else {
                return true;
            };
            if q.is_empty() {
                return true;
            }
            item.payload.to_ascii_lowercase().contains(&q)
                || item
                    .key
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&q)
        })
        .collect::<Vec<_>>();

    Ok(Json(TopicMessagesResponse {
        topic,
        total: items.len(),
        limit,
        offset: effective_offset,
        mode,
        items,
        timestamp: Utc::now(),
    }))
}

/// Fetch per-partition high-watermarks using rpk describe.
/// Returns map of partition → high_watermark (0 if unknown).
async fn partition_high_watermarks(
    state: &crate::AppState,
    topic: &str,
    specific_partition: Option<&str>,
) -> std::collections::HashMap<i32, i64> {
    let raw = run_docker(
        &state.config,
        &[
            "exec",
            state.config.redpanda_container.as_str(),
            "rpk",
            "topic",
            "describe",
            topic,
            "--print-partitions",
        ],
    )
    .await
    .unwrap_or_default();

    let mut map = std::collections::HashMap::new();
    let mut header: Vec<String> = Vec::new();
    for line in raw.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.is_empty() {
            continue;
        }
        if cols[0].eq_ignore_ascii_case("PARTITION") {
            header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
            continue;
        }
        let Ok(p) = cols[0].parse::<i32>() else {
            continue;
        };
        if let Some(sp) = specific_partition {
            if sp != p.to_string() {
                continue;
            }
        }
        let hw = header
            .iter()
            .position(|h| h == "HIGH-WATERMARK")
            .and_then(|i| cols.get(i).and_then(|v| v.parse::<i64>().ok()))
            .unwrap_or(0);
        map.insert(p, hw);
    }
    map
}

pub async fn topic_metadata(
    State(state): State<AppState>,
    Path(topic): Path<String>,
) -> Result<Json<TopicMetadataResponse>, AppError> {
    // Use -p (partitions) and -s (summary) to get full detail
    let output = run_docker(
        &state.config,
        &[
            "exec",
            state.config.redpanda_container.as_str(),
            "rpk",
            "topic",
            "describe",
            topic.as_str(),
            "--print-partitions",
            "--print-summary",
        ],
    )
    .await?;
    let raw = output.lines().map(|s| s.to_string()).collect::<Vec<_>>();
    let partitions = parse_rpk_topic_describe_partitions(&output);
    // Prefer SUMMARY PARTITIONS line; fall back to parsed partition count
    let partition_count = parse_summary_partition_count(&output).unwrap_or(partitions.len());
    let configs = match run_docker(
        &state.config,
        &[
            "exec",
            state.config.redpanda_container.as_str(),
            "rpk",
            "topic",
            "describe",
            topic.as_str(),
            "--print-configs",
        ],
    )
    .await
    {
        Ok(cfg_output) => parse_rpk_topic_configs(&cfg_output),
        Err(_) => HashMap::new(),
    };
    let urp = partitions
        .iter()
        .filter(|p| p.in_sync_replicas.len() < p.replicas.len())
        .count();
    Ok(Json(TopicMetadataResponse {
        topic,
        partition_count,
        urp,
        partitions,
        configs,
        raw,
        timestamp: Utc::now(),
    }))
}

pub async fn list_profiles(
    State(state): State<AppState>,
) -> Result<Json<ProfilesResponse>, AppError> {
    let mut profiles = state.profiles.list().await?;
    for p in &mut profiles {
        p.redact_secrets();
    }
    Ok(Json(ProfilesResponse {
        profiles,
        timestamp: Utc::now(),
    }))
}

pub async fn get_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ProfileResponse>, AppError> {
    let mut profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;
    profile.redact_secrets();
    Ok(Json(ProfileResponse {
        profile,
        timestamp: Utc::now(),
    }))
}

pub async fn delete_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let mut profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

    if profile.connector_name.is_some() || profile.sink_connector_name.is_some() {
        let _ = reconcile::apply_profile_action(&state, &mut profile, "stop").await;
    }

    let name = profile.name.clone();
    state.profiles.delete(id).await?;
    let actor = actor_from_headers(&headers);
    log_action(
        &actor,
        "profile.delete",
        "success",
        &format!("profile={name}"),
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ProfileUpsertRequest>,
) -> Result<Json<ProfileResponse>, AppError> {
    validate_profile_payload(&payload, false)?;
    reject_dual_source_capture(&state, None, &payload).await?;
    let mut profile = build_profile(None, payload)?;
    state.profiles.upsert(&profile).await?;
    let actor = actor_from_headers(&headers);
    log_action(
        &actor,
        "profile.create",
        "success",
        &format!("profile={}", profile.name),
    );
    profile.redact_secrets();

    Ok(Json(ProfileResponse {
        profile,
        timestamp: Utc::now(),
    }))
}

pub async fn create_profile_from_case(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ProfileCaseRequest>,
) -> Result<Json<ProfileResponse>, AppError> {
    let mode = payload.case_id.trim().to_ascii_lowercase();
    let (pattern, target, transform_strategy, transforms, stream_platform) = match mode.as_str() {
        "case1" | "case1_source_to_stream" | "source_to_stream" => (
            ProfilePattern::SourceToStream,
            None,
            TransformStrategy::None,
            Vec::new(),
            Some(payload.stream_platform.unwrap_or(StreamPlatform::Redpanda)),
        ),
        "case2" | "case2_source_target_sync" | "source_to_target_basic" => (
            ProfilePattern::SourceToTarget,
            payload.target,
            TransformStrategy::None,
            Vec::new(),
            None,
        ),
        "case3" | "case3_source_target_transform" | "source_to_target_transform" => (
            ProfilePattern::SourceToTarget,
            payload.target,
            TransformStrategy::Predefined,
            vec![build_predefined_transform(
                payload
                    .transform_preset
                    .as_deref()
                    .unwrap_or("mask_pii_standard"),
            )?],
            None,
        ),
        _ => {
            return Err(AppError::BadRequest(format!(
                "unsupported case_id: {}",
                payload.case_id
            )));
        }
    };

    let mut target = target;
    if let Some(ref mut t) = target {
        if t.name.trim().is_empty() {
            t.name = format!("{}-target", payload.name.trim());
        }
    }

    let request = ProfileUpsertRequest {
        name: payload.name,
        description: None,
        pattern: Some(pattern),
        stream_platform,
        transform_strategy: Some(transform_strategy),
        source: payload.source,
        target,
        lake: None,
        governance: None,
        sink_error_policy: Default::default(),
        flow: crate::models::FlowConfig {
            transforms,
            mappings: vec![],
            schedule: crate::models::ScheduleConfig {
                delivery_mode: payload.delivery_mode.unwrap_or(DeliveryMode::Realtime),
                interval_seconds: payload.interval_seconds,
                full_refresh_minutes: None,
            },
        },
    };
    validate_profile_payload(&request, true)?;
    reject_dual_source_capture(&state, None, &request).await?;
    let mut profile = build_profile(None, request)?;
    state.profiles.upsert(&profile).await?;
    let actor = actor_from_headers(&headers);
    log_action(
        &actor,
        "profile.create.case",
        "success",
        &format!("profile={} case={mode}", profile.name),
    );
    profile.redact_secrets();

    Ok(Json(ProfileResponse {
        profile,
        timestamp: Utc::now(),
    }))
}

pub async fn update_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<ProfileUpsertRequest>,
) -> Result<Json<ProfileResponse>, AppError> {
    validate_profile_payload(&payload, false)?;
    reject_dual_source_capture(&state, Some(id), &payload).await?;
    let mut profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

    profile.name = if payload.name.trim().is_empty() {
        format!("Draft · {}", payload.source.source_type)
    } else {
        payload.name.trim().to_string()
    };
    if let Some(description) = payload.description {
        profile.description = description.trim().to_string();
    }
    profile.pattern = payload
        .pattern
        .unwrap_or_else(|| infer_profile_pattern(payload.target.as_ref(), payload.lake.as_ref()));
    profile.stream_platform = if profile.pattern == ProfilePattern::SourceToStream {
        Some(payload.stream_platform.unwrap_or(StreamPlatform::Redpanda))
    } else {
        None
    };
    profile.transform_strategy = payload
        .transform_strategy
        .unwrap_or_else(|| infer_transform_strategy(&payload.flow.transforms));

    let old_source_conn = profile.source.connection.clone();
    let old_target_conn = profile.target.as_ref().map(|t| t.connection.clone());
    let old_lake_conn = profile.lake.as_ref().map(|l| l.connection.clone());
    let mut new_source = payload.source;
    preserve_secret_fields(&mut new_source.connection, &old_source_conn);
    profile.source = new_source;

    profile.target = merge_profile_target(&profile.name, payload.target, old_target_conn.as_ref());
    profile.lake = merge_profile_lake(&profile.name, payload.lake, old_lake_conn.as_ref());
    profile.governance = payload.governance.unwrap_or_default();
    if let Some(policy) = payload.sink_error_policy {
        profile.sink_error_policy = policy;
    }

    profile.flow = payload.flow;
    profile.updated_at = Utc::now();
    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_updated".to_string(),
        detail: "Profile configuration updated".to_string(),
    });
    if matches!(
        profile.status,
        ProfileStatus::Draft | ProfileStatus::Validated
    ) {
        profile.status = ProfileStatus::Draft;
    }
    state.profiles.upsert(&profile).await?;
    let actor = actor_from_headers(&headers);
    log_action(
        &actor,
        "profile.update",
        "success",
        &format!("profile={}", profile.name),
    );
    profile.redact_secrets();

    Ok(Json(ProfileResponse {
        profile,
        timestamp: Utc::now(),
    }))
}

pub async fn set_profile_enabled(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<ProfileEnableRequest>,
) -> Result<Json<ProfileResponse>, AppError> {
    let mut profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

    if payload.enabled {
        validate_profile_payload(&upsert_from_profile(&profile), true)?;
    }

    profile.enabled = payload.enabled;
    profile.updated_at = Utc::now();
    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_enable_changed".to_string(),
        detail: format!("Profile enabled set to {}", payload.enabled),
    });

    if payload.enabled {
        if let Err(e) = reconcile::apply_profile_action(&state, &mut profile, "start").await {
            if profile.status != ProfileStatus::Draft {
                profile.status = ProfileStatus::Failed;
            }
            profile.last_error = Some(e.to_string_message());
            state.profiles.upsert(&profile).await?;
            return Err(e);
        }
    } else if profile.status == ProfileStatus::Running {
        if let Err(e) = reconcile::apply_profile_action(&state, &mut profile, "pause").await {
            profile.last_error = Some(e.to_string_message());
            state.profiles.upsert(&profile).await?;
            return Err(e);
        }
    }

    state.profiles.upsert(&profile).await?;
    let actor = actor_from_headers(&headers);
    log_action(
        &actor,
        "profile.enable",
        "success",
        &format!("profile={} enabled={}", profile.name, payload.enabled),
    );
    profile.redact_secrets();

    Ok(Json(ProfileResponse {
        profile,
        timestamp: Utc::now(),
    }))
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct ProfileActionParams {
    /// Only honoured by `resync`: empty the target tables before replaying.
    #[serde(default)]
    pub clear_target: bool,
}

pub async fn profile_action(
    State(state): State<AppState>,
    Path((id, action)): Path<(Uuid, String)>,
    Query(params): Query<ProfileActionParams>,
    headers: HeaderMap,
) -> Result<Json<ProfileResponse>, AppError> {
    let mut profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

    if matches!(action.as_str(), "start" | "restart") {
        validate_profile_payload(&upsert_from_profile(&profile), true)?;
    }

    let outcome = if action == "resync" {
        reconcile::reconcile_resync(&state, &mut profile, params.clear_target).await
    } else {
        reconcile::apply_profile_action(&state, &mut profile, &action).await
    };

    if let Err(e) = outcome {
        if profile.status != ProfileStatus::Draft {
            profile.status = ProfileStatus::Failed;
        }
        profile.last_error = Some(e.to_string_message());
        let _ = state.profiles.upsert(&profile).await;
        return Err(e);
    }

    state.profiles.upsert(&profile).await?;
    let actor = actor_from_headers(&headers);
    log_action(
        &actor,
        &format!("profile.{action}"),
        "success",
        &format!("profile={}", profile.name),
    );
    profile.redact_secrets();

    Ok(Json(ProfileResponse {
        profile,
        timestamp: Utc::now(),
    }))
}

pub async fn validate_source(
    State(state): State<AppState>,
    Json(mut req): Json<crate::models::SourceCatalogRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    // Restore stored credentials when the caller omits the password (saved profile).
    if let Some(pid) = req.profile_id {
        if let Ok(Some(profile)) = state.profiles.get(pid).await {
            preserve_secret_fields(&mut req.source.connection, &profile.source.connection);
        }
    }
    let payload = req.source;
    if payload.source_type.trim().is_empty() {
        return Err(AppError::BadRequest("source_type is required".to_string()));
    }
    if !is_supported_database_type(&payload.source_type) {
        return Err(AppError::BadRequest(format!(
            "unsupported source_type: {} (database connectors only)",
            payload.source_type
        )));
    }
    validate_connection_fields(&payload.connection, "source")?;

    let host = payload
        .connection
        .get("host")
        .map(|s| s.as_str())
        .unwrap_or("");
    let port = payload
        .connection
        .get("port")
        .map(|s| s.as_str())
        .unwrap_or("");

    let message = if crate::clients::JdbcAdminClient::from_env().is_some() {
        connector_catalog::test_connection(&payload.source_type, &payload.connection).await?
    } else {
        match payload.source_type.trim().to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => {
                let insight = connector_catalog::insight_from_connection(
                    &payload.source_type,
                    &payload.connection,
                );
                connector_catalog::postgres_client(&insight)
                    .await
                    .map(|_| format!("source connection successful: {host}:{port}"))
                    .map_err(|e| {
                        let detail = e.to_string();
                        let detail = detail
                            .strip_prefix("catalog connection failed to ")
                            .unwrap_or(&detail);
                        AppError::BadRequest(format!("source connection failed: {detail}"))
                    })?
            }
            _ => probe_connection(&state, &payload.connection, "source").await?,
        }
    };

    Ok(Json(ValidationResponse {
        valid: true,
        message,
        timestamp: Utc::now(),
    }))
}

fn catalog_response(
    catalog: connector_catalog::LiveCatalog,
    selected_schemas: &[String],
    selected_tables: &[String],
) -> SourceCatalogResponse {
    let schemas = catalog
        .schemas
        .into_iter()
        .map(|name| {
            let table_count = catalog
                .tables
                .iter()
                .filter(|table| table.schema == name)
                .count();
            let captured = selected_schemas
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(&name));
            ConnectorCatalogSchema {
                name,
                table_count,
                captured,
            }
        })
        .collect();
    let tables = catalog
        .tables
        .into_iter()
        .map(|table| {
            let qualified = format!("{}.{}", table.schema, table.name);
            let captured = selected_tables.iter().any(|wanted| {
                wanted.eq_ignore_ascii_case(&qualified) || wanted.eq_ignore_ascii_case(&table.name)
            });
            ConnectorCatalogTable {
                schema: table.schema,
                name: table.name,
                captured,
                row_count: table.row_count,
                primary_key: table.primary_key,
            }
        })
        .collect();
    SourceCatalogResponse {
        reachable: true,
        message: "Live catalog loaded".to_string(),
        schemas,
        tables,
        timestamp: Utc::now(),
    }
}

fn source_config_from_target(target: TargetConfig) -> SourceConfig {
    SourceConfig {
        source_type: target.target_type,
        connection: target.connection,
        capture_mode: Default::default(),
        schema_mode: SchemaSelectionMode::AllSchemas,
        table_mode: TableSelectionMode::AllTables,
        schemas: vec![],
        tables: vec![],
        changelog_retention_hours: None,
        timestamp_column: None,
        incrementing_column: None,
    }
}

pub async fn browse_source_catalog(
    State(state): State<AppState>,
    Json(mut payload): Json<SourceCatalogRequest>,
) -> Result<Json<SourceCatalogResponse>, AppError> {
    if let Some(pid) = payload.profile_id {
        if let Ok(Some(profile)) = state.profiles.get(pid).await {
            preserve_secret_fields(&mut payload.source.connection, &profile.source.connection);
        }
    }
    if payload.source.source_type.trim().is_empty() {
        return Err(AppError::BadRequest("source_type is required".to_string()));
    }
    if !is_supported_database_type(&payload.source.source_type) {
        return Err(AppError::BadRequest(format!(
            "unsupported source_type: {} (database connectors only)",
            payload.source.source_type
        )));
    }
    validate_connection_fields(&payload.source.connection, "source")?;

    let catalog = connector_catalog::load_live_catalog_from_connection(
        &payload.source.source_type,
        &payload.source.connection,
        &payload.source.schemas,
    )
    .await?;
    let selected_schemas: Vec<String> = payload
        .source
        .schemas
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let selected_tables: Vec<String> = payload
        .source
        .tables
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(Json(catalog_response(
        catalog,
        &selected_schemas,
        &selected_tables,
    )))
}

pub async fn browse_target_catalog(
    State(state): State<AppState>,
    Json(mut payload): Json<TargetCatalogRequest>,
) -> Result<Json<SourceCatalogResponse>, AppError> {
    if let Some(pid) = payload.profile_id {
        if let Ok(Some(profile)) = state.profiles.get(pid).await {
            if let Some(stored_target) = &profile.target {
                preserve_secret_fields(&mut payload.target.connection, &stored_target.connection);
            }
        }
    }
    if payload.target.target_type.trim().is_empty() {
        return Err(AppError::BadRequest("target_type is required".to_string()));
    }
    if !is_supported_database_type(&payload.target.target_type) {
        return Err(AppError::BadRequest(format!(
            "unsupported target_type: {} (database connectors only)",
            payload.target.target_type
        )));
    }
    validate_connection_fields(&payload.target.connection, "target")?;

    let schema_filter: Vec<String> = payload
        .target
        .schemas
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let catalog = connector_catalog::load_live_catalog_from_connection(
        &payload.target.target_type,
        &payload.target.connection,
        &schema_filter,
    )
    .await?;
    Ok(Json(catalog_response(catalog, &schema_filter, &[])))
}

pub async fn inspect_source_table(
    State(state): State<AppState>,
    Json(mut payload): Json<SourceTableInspectRequest>,
) -> Result<Json<crate::models::SourceTableInspectResponse>, AppError> {
    // When a saved profile_id is supplied, merge stored credentials so the caller
    // does not need to re-supply a password (e.g. when editing a saved pipeline).
    if let Some(pid) = payload.profile_id {
        if let Ok(Some(profile)) = state.profiles.get(pid).await {
            preserve_secret_fields(&mut payload.source.connection, &profile.source.connection);
        }
    }
    if payload.source.source_type.trim().is_empty() {
        return Err(AppError::BadRequest("source_type is required".to_string()));
    }
    if !is_supported_database_type(&payload.source.source_type) {
        return Err(AppError::BadRequest(format!(
            "unsupported source_type: {} (database connectors only)",
            payload.source.source_type
        )));
    }
    validate_connection_fields(&payload.source.connection, "source")?;
    if payload.schema.trim().is_empty() || payload.name.trim().is_empty() {
        return Err(AppError::BadRequest(
            "schema and table name are required".to_string(),
        ));
    }
    let inspected = connector_catalog::inspect_source_table(&payload).await?;
    Ok(Json(inspected))
}

fn trigger_table_names(source: &SourceConfig, extra: &[String]) -> Vec<String> {
    if extra.iter().any(|item| !item.trim().is_empty()) {
        extra.to_vec()
    } else {
        source.tables.clone()
    }
}

fn prepare_trigger_request(
    payload: &TriggerInstallRequest,
) -> Result<Vec<crate::postgres_trigger_cdc::CapturedTable>, AppError> {
    let source_type = payload.source.source_type.trim().to_ascii_lowercase();
    crate::trigger_cdc::Vendor::parse(&source_type)?;
    if source_type == "sqlite" {
        if payload
            .source
            .connection
            .get("database")
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
        {
            return Err(AppError::BadRequest(
                "SQLite trigger install needs a database file path".to_string(),
            ));
        }
    } else {
        validate_connection_fields(&payload.source.connection, "source")?;
    }
    let names = trigger_table_names(&payload.source, &payload.tables);
    crate::trigger_cdc::tables_from_names(&names)
}

/// Merge stored source credentials into a trigger request when `profile_id` is supplied,
/// so callers editing a saved profile do not have to re-enter the password.
async fn restore_trigger_secrets(state: &AppState, payload: &mut TriggerInstallRequest) {
    if let Some(pid) = payload.profile_id {
        if let Ok(Some(profile)) = state.profiles.get(pid).await {
            preserve_secret_fields(&mut payload.source.connection, &profile.source.connection);
        }
    }
}

pub async fn preview_source_triggers(
    State(state): State<AppState>,
    Json(mut payload): Json<TriggerInstallRequest>,
) -> Result<Json<TriggerPreviewResponse>, AppError> {
    restore_trigger_secrets(&state, &mut payload).await;
    let tables = prepare_trigger_request(&payload)?;
    let result = crate::trigger_cdc::preview_source_triggers(
        &payload.source.source_type,
        &payload.source.connection,
        &tables,
    )
    .await?;
    Ok(Json(TriggerPreviewResponse {
        tables: result.tables,
        change_log: result.change_log,
        sql: result.sql,
        will_execute: result.will_execute,
        vendor: result.vendor,
        message: result.message,
        timestamp: Utc::now(),
    }))
}

pub async fn install_source_triggers(
    State(state): State<AppState>,
    Json(mut payload): Json<TriggerInstallRequest>,
) -> Result<Json<TriggerInstallResponse>, AppError> {
    restore_trigger_secrets(&state, &mut payload).await;
    let tables = prepare_trigger_request(&payload)?;
    let result = crate::trigger_cdc::install_source_triggers(
        &payload.source.source_type,
        &payload.source.connection,
        &tables,
    )
    .await?;
    Ok(Json(TriggerInstallResponse {
        installed: result.installed,
        already_present: result.already_present,
        change_log: result.change_log,
        sql: result.sql,
        executed: result.executed,
        message: result.message,
        timestamp: Utc::now(),
    }))
}

pub async fn source_trigger_status(
    State(state): State<AppState>,
    Json(mut payload): Json<TriggerInstallRequest>,
) -> Result<Json<TriggerStatusResponse>, AppError> {
    restore_trigger_secrets(&state, &mut payload).await;
    let source_type = payload.source.source_type.trim().to_ascii_lowercase();
    crate::trigger_cdc::Vendor::parse(&source_type)?;
    if source_type != "sqlite" {
        validate_connection_fields(&payload.source.connection, "source")?;
    }
    let names = trigger_table_names(&payload.source, &payload.tables);
    let (change_log_present, tables) = crate::trigger_cdc::source_trigger_status(
        &payload.source.source_type,
        &payload.source.connection,
        &names,
    )
    .await?;
    Ok(Json(TriggerStatusResponse {
        change_log_present,
        tables: tables
            .into_iter()
            .map(|(schema, name, installed)| TriggerStatusTable {
                schema,
                name,
                installed,
            })
            .collect(),
        timestamp: Utc::now(),
    }))
}

pub async fn inspect_target_table(
    State(state): State<AppState>,
    Json(mut payload): Json<TargetTableInspectRequest>,
) -> Result<Json<crate::models::SourceTableInspectResponse>, AppError> {
    // Merge stored target credentials from a saved profile when profile_id is provided.
    if let Some(pid) = payload.profile_id {
        if let Ok(Some(profile)) = state.profiles.get(pid).await {
            if let Some(stored_target) = &profile.target {
                preserve_secret_fields(&mut payload.target.connection, &stored_target.connection);
            }
        }
    }
    inspect_source_table(
        State(state),
        Json(SourceTableInspectRequest {
            source: source_config_from_target(payload.target),
            schema: payload.schema,
            name: payload.name,
            limit: payload.limit,
            profile_id: None,
        }),
    )
    .await
}

pub async fn validate_target(
    State(state): State<AppState>,
    Json(mut req): Json<crate::models::TargetCatalogRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    // Restore stored credentials when the caller omits the password (saved profile).
    if let Some(pid) = req.profile_id {
        if let Ok(Some(profile)) = state.profiles.get(pid).await {
            if let Some(stored_target) = &profile.target {
                preserve_secret_fields(&mut req.target.connection, &stored_target.connection);
            }
        }
    }
    let payload = req.target;
    if payload.target_type.trim().is_empty() {
        return Err(AppError::BadRequest("target_type is required".to_string()));
    }
    if !is_supported_database_type(&payload.target_type) {
        return Err(AppError::BadRequest(format!(
            "unsupported target_type: {} (database connectors only)",
            payload.target_type
        )));
    }
    validate_connection_fields(&payload.connection, "target")?;

    let host = payload
        .connection
        .get("host")
        .map(|s| s.as_str())
        .unwrap_or("");
    let port = payload
        .connection
        .get("port")
        .map(|s| s.as_str())
        .unwrap_or("");

    let message = if crate::clients::JdbcAdminClient::from_env().is_some() {
        connector_catalog::test_connection(&payload.target_type, &payload.connection).await?
    } else {
        match payload.target_type.trim().to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => {
                let insight = connector_catalog::insight_from_connection(
                    &payload.target_type,
                    &payload.connection,
                );
                connector_catalog::postgres_client(&insight)
                    .await
                    .map(|_| format!("target connection successful: {host}:{port}"))
                    .map_err(|e| {
                        let detail = e.to_string();
                        let detail = detail
                            .strip_prefix("catalog connection failed to ")
                            .unwrap_or(&detail);
                        AppError::BadRequest(format!("target connection failed: {detail}"))
                    })?
            }
            _ => probe_connection(&state, &payload.connection, "target").await?,
        }
    };

    Ok(Json(ValidationResponse {
        valid: true,
        message,
        timestamp: Utc::now(),
    }))
}

pub async fn validate_lake(
    State(state): State<AppState>,
    Json(payload): Json<LakeDestination>,
) -> Result<Json<ValidationResponse>, AppError> {
    crate::lake::validate_lake_destination(&payload)?;
    let endpoint = payload
        .connection
        .get("endpoint")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::BadRequest("lake connection missing required field: endpoint".to_string())
        })?;
    let (host, port) = crate::lake::lake_endpoint_host_port(endpoint)?;
    let mut probe = payload.connection.clone();
    probe.insert("host".into(), host);
    probe.insert("port".into(), port.to_string());
    let message = probe_connection(&state, &probe, "lake").await?;
    Ok(Json(ValidationResponse {
        valid: true,
        message,
        timestamp: Utc::now(),
    }))
}

async fn mutate_engine_action(
    state: AppState,
    headers: HeaderMap,
    action: &str,
) -> Result<Json<ActionResponse>, AppError> {
    let actor = actor_from_headers(&headers);
    let message = match action {
        "engine.up" => engine::engine_up(&state.config).await?,
        "engine.down" => engine::engine_down(&state.config).await?,
        "engine.restart" => engine::engine_restart(&state.config).await?,
        _ => {
            return Err(AppError::BadRequest(
                "unsupported engine action".to_string(),
            ))
        }
    };
    let audit_id = log_action(&actor, action, "success", &message);
    Ok(Json(ActionResponse {
        action: action.to_string(),
        status: "success".to_string(),
        message,
        audit_id,
        timestamp: Utc::now(),
    }))
}

fn actor_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-user")
        .or_else(|| headers.get("x-actor"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system")
        .to_string()
}

fn parse_rpk_topic_list(raw: &str) -> Vec<TopicItem> {
    crate::kafka_topics::parse_topic_list(raw)
}

fn parse_rpk_group_list(raw: &str) -> Vec<String> {
    // rpk group list output: BROKER  GROUP  STATE
    // Find the GROUP column index from the header row, fall back to index 1.
    let mut group_col: usize = 1;
    let mut header_found = false;
    let mut groups = Vec::new();
    for line in raw.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.is_empty() {
            continue;
        }
        if !header_found {
            // Detect header row by looking for "GROUP"
            if let Some(pos) = cols.iter().position(|c| c.eq_ignore_ascii_case("GROUP")) {
                group_col = pos;
                header_found = true;
            }
            continue;
        }
        if let Some(group) = cols.get(group_col) {
            if !group.is_empty() {
                groups.push((*group).to_string());
            }
        }
    }
    groups
}

/// Parse `rpk topic describe --print-partitions` output.
///
/// Expected header (may vary by rpk version):
///   PARTITION  LEADER  EPOCH  REPLICAS  LOG-START-OFFSET  HIGH-WATERMARK
/// Replicas are printed as `[0]` or `[0,1,2]` — strip the brackets.
fn parse_rpk_topic_describe_partitions(raw: &str) -> Vec<TopicPartitionMetadata> {
    let mut header: Vec<String> = Vec::new();
    let mut out = Vec::new();

    for line in raw.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.is_empty() {
            continue;
        }
        // Detect header row
        if cols[0].eq_ignore_ascii_case("PARTITION") {
            header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
            continue;
        }
        if header.is_empty() {
            continue;
        }
        // First column must parse as a partition number
        let Ok(partition) = cols[0].parse::<i32>() else {
            continue;
        };

        let get = |name: &str| -> Option<&str> {
            header
                .iter()
                .position(|h| h == name)
                .and_then(|i| cols.get(i).copied())
        };

        let leader = get("LEADER").and_then(|v| v.parse::<i32>().ok());

        // REPLICAS column looks like "[0]" or "[0,1,2]"
        let parse_id_list = |v: &str| -> Vec<i32> {
            v.trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .filter_map(|p| p.trim().parse::<i32>().ok())
                .collect()
        };

        let replicas = get("REPLICAS").map(parse_id_list).unwrap_or_default();

        // ISR column present in some rpk versions; default to replicas if absent
        let in_sync_replicas = get("IN-SYNC-REPLICAS")
            .or_else(|| get("ISR"))
            .map(parse_id_list)
            .unwrap_or_else(|| replicas.clone());

        let log_start_offset = get("LOG-START-OFFSET").and_then(|v| v.parse::<i64>().ok());
        let high_watermark = get("HIGH-WATERMARK").and_then(|v| v.parse::<i64>().ok());
        let message_count = match (log_start_offset, high_watermark) {
            (Some(start), Some(hw)) if hw >= start => Some(hw - start),
            _ => None,
        };

        out.push(TopicPartitionMetadata {
            partition,
            leader,
            replicas,
            in_sync_replicas,
            log_start_offset,
            high_watermark,
            message_count,
        });
    }
    out
}

/// Extract PARTITIONS count from the SUMMARY section printed by `rpk topic describe -s`.
fn parse_summary_partition_count(raw: &str) -> Option<usize> {
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("PARTITIONS") {
            if let Ok(n) = rest.trim().parse::<usize>() {
                return Some(n);
            }
        }
    }
    None
}

/// Parse `rpk topic describe --print-configs` output.
///
/// Format: `KEY  VALUE  SOURCE`  (whitespace-separated, header skipped)
/// May also appear as `key=value` from other sources.
fn parse_rpk_topic_configs(raw: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        absorb_topic_config_line(&mut out, line);
    }
    out
}

fn absorb_topic_config_line(out: &mut HashMap<String, String>, line: &str) {
    let trimmed = line.trim();
    if topic_config_noise(trimmed) {
        return;
    }
    if absorb_topic_config_kv(out, trimmed) {
        return;
    }
    absorb_topic_config_row(out, trimmed);
}

fn topic_config_noise(trimmed: &str) -> bool {
    trimmed.is_empty() || trimmed.starts_with('=') || trimmed.starts_with('#')
}

fn absorb_topic_config_kv(out: &mut HashMap<String, String>, trimmed: &str) -> bool {
    let Some((k, v)) = trimmed.split_once('=') else {
        return false;
    };
    let key = k.trim().to_string();
    if !key.is_empty() && !key.contains(' ') {
        out.insert(key, v.trim().to_string());
    }
    true
}

fn absorb_topic_config_row(out: &mut HashMap<String, String>, trimmed: &str) {
    let cols: Vec<&str> = trimmed.split_whitespace().collect();
    if cols.len() < 2 || cols[0].eq_ignore_ascii_case("KEY") || cols[0].is_empty() {
        return;
    }
    out.insert(cols[0].to_string(), cols[1].to_string());
}

fn parse_topic_message_line(line: &str) -> Option<TopicMessageItem> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cols: Vec<&str> = trimmed.splitn(5, '\t').collect();
    if cols.len() == 5 {
        return Some(TopicMessageItem {
            key: Some(cols[0].to_string()).filter(|s| !s.is_empty()),
            partition: cols[1].parse::<i32>().ok(),
            offset: cols[2].parse::<i64>().ok(),
            timestamp: Some(cols[3].to_string()),
            payload: cols[4].to_string(),
        });
    }
    let cols4: Vec<&str> = trimmed.splitn(4, '\t').collect();
    if cols4.len() == 4 && cols4[0].parse::<i32>().is_ok() {
        return Some(TopicMessageItem {
            partition: cols4[0].parse::<i32>().ok(),
            offset: cols4[1].parse::<i64>().ok(),
            timestamp: Some(cols4[2].to_string()),
            key: None,
            payload: cols4[3].to_string(),
        });
    }
    Some(TopicMessageItem {
        partition: None,
        offset: None,
        timestamp: None,
        key: None,
        payload: trimmed.to_string(),
    })
}

fn apply_pagination<T>(
    mut items: Vec<T>,
    page: Option<usize>,
    page_size: Option<usize>,
) -> (usize, usize, Vec<T>) {
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(50).clamp(1, 200);
    let start = (page - 1) * page_size;
    if start >= items.len() {
        return (page, page_size, Vec::new());
    }
    let tail = items.split_off(start);
    let paged = tail.into_iter().take(page_size).collect();
    (page, page_size, paged)
}

async fn reject_dual_source_capture(
    state: &AppState,
    ignore_id: Option<Uuid>,
    payload: &ProfileUpsertRequest,
) -> Result<(), AppError> {
    let pattern = payload
        .pattern
        .clone()
        .unwrap_or_else(|| infer_profile_pattern(payload.target.as_ref(), payload.lake.as_ref()));
    if !payload.source.capture_identity_ready() {
        return Ok(());
    }
    if !pattern.is_olake_capture() && !pattern.is_debezium_capture() {
        return Ok(());
    }
    let identity = payload.source.capture_identity();
    for existing in state.profiles.list().await? {
        if ignore_id == Some(existing.id) {
            continue;
        }
        if existing.source.capture_identity() != identity {
            continue;
        }
        let conflict = (pattern.is_olake_capture() && existing.pattern.is_debezium_capture())
            || (pattern.is_debezium_capture() && existing.pattern.is_olake_capture());
        if conflict {
            return Err(AppError::BadRequest(format!(
                "This source is already captured by '{}'. One reader per database: Analytics-first (OLake) cannot share a source with replica or Event-first (Debezium).",
                existing.name
            )));
        }
    }
    Ok(())
}

fn validate_trigger_capture(payload: &ProfileUpsertRequest) -> Result<(), AppError> {
    crate::trigger_cdc::Vendor::parse(&payload.source.source_type)?;
    if payload
        .source
        .tables
        .iter()
        .all(|item| item.trim().is_empty())
    {
        return Err(AppError::BadRequest(
            "trigger capture needs an explicit table list (schema.table)".to_string(),
        ));
    }
    match payload.target.as_ref() {
        None => {
            return Err(AppError::BadRequest(
                "trigger capture requires a PostgreSQL target (source_to_target); stream/lake patterns are not supported in trigger mode".to_string(),
            ));
        }
        Some(target)
            if !matches!(
                target.target_type.trim().to_ascii_lowercase().as_str(),
                "postgres" | "postgresql"
            ) =>
        {
            return Err(AppError::BadRequest(format!(
                "trigger capture currently supports only a PostgreSQL target (got '{}')",
                target.target_type
            )));
        }
        _ => {}
    }
    if payload.flow.schedule.delivery_mode == DeliveryMode::MicroBatch
        && payload.flow.schedule.interval_seconds.unwrap_or(0) == 0
    {
        return Err(AppError::BadRequest(
            "trigger capture in micro_batch mode requires interval_seconds > 0".to_string(),
        ));
    }
    Ok(())
}

fn validate_pattern_shape(
    payload: &ProfileUpsertRequest,
    pattern: &ProfilePattern,
) -> Result<(), AppError> {
    let message = match (pattern, payload.target.as_ref(), payload.lake.as_ref()) {
        (ProfilePattern::SourceToTarget, None, _) => {
            "target is required for source_to_target pattern"
        }
        (ProfilePattern::SourceToStream, Some(_), _) => {
            "target must be empty for source_to_stream pattern"
        }
        (ProfilePattern::SourceToStream, _, Some(_)) => {
            "lake must be empty for source_to_stream pattern"
        }
        (ProfilePattern::SourceToLake, _, None) => {
            "lake destination is required for source_to_lake pattern"
        }
        (ProfilePattern::SourceToLake, Some(_), _) => {
            "target must be empty for source_to_lake pattern; use a lake destination"
        }
        (ProfilePattern::SourceToTarget, _, Some(_)) => {
            "lake must be empty for source_to_target pattern; JDBC replica and lake are separate patterns"
        }
        (ProfilePattern::SourceToOlake, Some(_), _) => {
            "target must be empty for source_to_olake; Analytics-first uses the OLake stack, not a JDBC replica"
        }
        (ProfilePattern::SourceToOlake, _, Some(_)) => {
            "lake must be empty for source_to_olake; Iceberg from Kafka is Event-first (source_to_lake), not OLake"
        }
        _ => return Ok(()),
    };
    Err(AppError::BadRequest(message.to_string()))
}

fn merge_profile_target(
    profile_name: &str,
    new_target: Option<TargetConfig>,
    old_conn: Option<&std::collections::HashMap<String, String>>,
) -> Option<TargetConfig> {
    let mut new_target = new_target?;
    if let Some(old_t) = old_conn {
        preserve_secret_fields(&mut new_target.connection, old_t);
    }
    if new_target.name.trim().is_empty() {
        new_target.name = format!("{profile_name}-target");
    }
    Some(new_target)
}

fn merge_profile_lake(
    profile_name: &str,
    new_lake: Option<LakeDestination>,
    old_conn: Option<&std::collections::HashMap<String, String>>,
) -> Option<LakeDestination> {
    let mut new_lake = new_lake?;
    if let Some(old_l) = old_conn {
        preserve_secret_fields(&mut new_lake.connection, old_l);
    }
    if new_lake.name.trim().is_empty() {
        new_lake.name = format!("{profile_name}-lake");
    }
    Some(new_lake)
}

fn validate_profile_payload(payload: &ProfileUpsertRequest, strict: bool) -> Result<(), AppError> {
    if strict && payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("profile name is required".to_string()));
    }
    if !is_supported_database_type(&payload.source.source_type) {
        return Err(AppError::BadRequest(format!(
            "unsupported source_type: {} (database connectors only)",
            payload.source.source_type
        )));
    }
    let pattern = payload
        .pattern
        .clone()
        .unwrap_or_else(|| infer_profile_pattern(payload.target.as_ref(), payload.lake.as_ref()));
    if payload.sink_error_policy == Some(crate::models::SinkErrorPolicy::Dlq)
        && pattern != ProfilePattern::SourceToTarget
    {
        return Err(AppError::BadRequest(
            "sink_error_policy=dlq is supported only for source_to_target profiles".to_string(),
        ));
    }
    if !strict {
        return Ok(());
    }
    if payload.source.capture_mode == crate::models::CaptureMode::Incremental {
        crate::incremental_cdc::validate_profile(payload)?;
    }
    if payload.source.capture_mode == crate::models::CaptureMode::Trigger {
        validate_trigger_capture(payload)?;
    }
    validate_pattern_shape(payload, &pattern)?;
    validate_strict_connections(payload)?;
    Ok(())
}

fn validate_strict_connections(payload: &ProfileUpsertRequest) -> Result<(), AppError> {
    if let Some(target) = payload.target.as_ref() {
        if !is_supported_database_type(&target.target_type) {
            return Err(AppError::BadRequest(format!(
                "unsupported target_type: {} (database connectors only)",
                target.target_type
            )));
        }
        validate_connection_fields(&target.connection, "target")?;
    }
    if let Some(lake) = payload.lake.as_ref() {
        crate::lake::validate_lake_destination(lake)?;
    }
    validate_connection_fields(&payload.source.connection, "source")?;
    if payload.source.schema_mode == SchemaSelectionMode::SpecificSchemas
        && payload.source.schemas.is_empty()
    {
        return Err(AppError::BadRequest(
            "schemas are required when schema_mode is specific_schemas".to_string(),
        ));
    }
    if payload.source.table_mode == TableSelectionMode::SpecificTables
        && payload.source.tables.is_empty()
        && payload.flow.mappings.is_empty()
    {
        return Err(AppError::BadRequest(
            "tables are required when table_mode is specific_tables".to_string(),
        ));
    }
    crate::models::validate_table_mappings(&payload.flow.mappings).map_err(AppError::BadRequest)?;
    if payload.flow.schedule.delivery_mode == DeliveryMode::MicroBatch
        && payload.flow.schedule.interval_seconds.unwrap_or(0) == 0
    {
        return Err(AppError::BadRequest(
            "interval_seconds is required for micro_batch mode".to_string(),
        ));
    }
    Ok(())
}

fn upsert_from_profile(profile: &crate::models::CdcProfile) -> ProfileUpsertRequest {
    ProfileUpsertRequest {
        name: profile.name.clone(),
        description: Some(profile.description.clone()),
        pattern: Some(profile.pattern.clone()),
        stream_platform: profile.stream_platform,
        transform_strategy: Some(profile.transform_strategy.clone()),
        source: profile.source.clone(),
        target: profile.target.clone(),
        lake: profile.lake.clone(),
        governance: Some(profile.governance.clone()),
        sink_error_policy: Some(profile.sink_error_policy),
        flow: profile.flow.clone(),
    }
}

fn build_profile(
    id: Option<Uuid>,
    payload: ProfileUpsertRequest,
) -> Result<crate::models::CdcProfile, AppError> {
    let now = Utc::now();
    let pattern = payload
        .pattern
        .clone()
        .unwrap_or_else(|| infer_profile_pattern(payload.target.as_ref(), payload.lake.as_ref()));
    let stream_platform = if pattern == ProfilePattern::SourceToStream {
        Some(payload.stream_platform.unwrap_or(StreamPlatform::Redpanda))
    } else {
        None
    };
    let transform_strategy = payload
        .transform_strategy
        .unwrap_or_else(|| infer_transform_strategy(&payload.flow.transforms));
    let pipeline_name = if payload.name.trim().is_empty() {
        format!("Draft · {}", payload.source.source_type)
    } else {
        payload.name.trim().to_string()
    };
    let mut target = payload.target;
    if let Some(ref mut t) = target {
        if t.name.trim().is_empty() {
            t.name = format!("{pipeline_name}-target");
        }
    }
    let mut lake = payload.lake;
    if let Some(ref mut l) = lake {
        if l.name.trim().is_empty() {
            l.name = format!("{pipeline_name}-lake");
        }
    }
    Ok(crate::models::CdcProfile {
        id: id.unwrap_or_else(Uuid::new_v4),
        name: pipeline_name,
        description: payload.description.unwrap_or_default().trim().to_string(),
        pattern,
        stream_platform,
        transform_strategy,
        source: payload.source,
        target,
        lake,
        governance: payload.governance.unwrap_or_default(),
        sink_error_policy: payload.sink_error_policy.unwrap_or_default(),
        flow: payload.flow,
        status: ProfileStatus::Draft,
        enabled: true,
        last_error: None,
        history: vec![HistoryItem {
            timestamp: now,
            level: "info".to_string(),
            event: "profile_created".to_string(),
            detail: "Draft saved. Finish configuration to start capture.".to_string(),
        }],
        created_at: now,
        updated_at: now,
        connector_name: None,
        sink_connector_name: None,
    })
}

fn infer_profile_pattern(
    target: Option<&TargetConfig>,
    lake: Option<&LakeDestination>,
) -> ProfilePattern {
    if lake.is_some() {
        ProfilePattern::SourceToLake
    } else if target.is_some() {
        ProfilePattern::SourceToTarget
    } else {
        ProfilePattern::SourceToStream
    }
}

fn infer_transform_strategy(transforms: &[TransformConfig]) -> TransformStrategy {
    if transforms.is_empty() {
        TransformStrategy::None
    } else if transforms
        .iter()
        .all(|t| t.name.starts_with("std_") || t.name == "mask_pii")
    {
        TransformStrategy::Predefined
    } else {
        TransformStrategy::Custom
    }
}

fn build_predefined_transform(preset: &str) -> Result<TransformConfig, AppError> {
    match preset.trim().to_ascii_lowercase().as_str() {
        "mask_pii_standard" | "mask_pii" => Ok(TransformConfig {
            name: "std_mask_pii".to_string(),
            enabled: true,
            config: HashMap::from([
                ("strategy".to_string(), "partial_mask".to_string()),
                ("fields".to_string(), "email,phone".to_string()),
            ]),
        }),
        "normalize_names" | "normalize_name" => Ok(TransformConfig {
            name: "std_normalize_name".to_string(),
            enabled: true,
            config: HashMap::from([("mode".to_string(), "title_case".to_string())]),
        }),
        other => Err(AppError::BadRequest(format!(
            "unsupported transform_preset: {other}"
        ))),
    }
}

fn validate_connection_fields(
    fields: &HashMap<String, String>,
    location: &str,
) -> Result<(), AppError> {
    for key in ["host", "port", "database", "username", "password"] {
        required_connection_field(fields, key, location)?;
    }
    Ok(())
}

fn required_connection_field<'a>(
    fields: &'a HashMap<String, String>,
    key: &str,
    location: &str,
) -> Result<&'a str, AppError> {
    let value = fields.get(key).map(|v| v.trim()).unwrap_or("");
    if value.is_empty() {
        return Err(AppError::BadRequest(format!(
            "{location} connection missing required field: {key}"
        )));
    }
    Ok(value)
}

/// TCP reachability on host:port when jdbc-admin is not configured.
async fn probe_connection(
    _state: &AppState,
    fields: &HashMap<String, String>,
    location: &str,
) -> Result<String, AppError> {
    let host = required_connection_field(fields, "host", location)?;
    let port = required_connection_field(fields, "port", location)?;
    test_database_connection(fields, location).await?;
    Ok(format!("{location} connection successful: {host}:{port}"))
}

async fn test_database_connection(
    fields: &HashMap<String, String>,
    location: &str,
) -> Result<(), AppError> {
    let host = required_connection_field(fields, "host", location)?;
    let port_raw = required_connection_field(fields, "port", location)?;
    let port = port_raw.parse::<u16>().map_err(|_| {
        AppError::BadRequest(format!(
            "{location} connection invalid port: {port_raw} (expected 1-65535)"
        ))
    })?;

    let endpoint = format!("{host}:{port}");
    let connect = timeout(Duration::from_secs(5), TcpStream::connect((host, port))).await;
    match connect {
        Ok(Ok(stream)) => {
            drop(stream);
            Ok(())
        }
        Ok(Err(err)) => Err(AppError::BadRequest(format!(
            "{location} connection failed to {endpoint}: {err}"
        ))),
        Err(_) => Err(AppError::BadRequest(format!(
            "{location} connection timed out after 5 seconds to {endpoint}"
        ))),
    }
}

fn preserve_secret_fields(
    incoming: &mut HashMap<String, String>,
    existing: &HashMap<String, String>,
) {
    for key in [
        "password",
        "secret",
        "token",
        "secret_key",
        "access_key",
        "catalog_password",
    ] {
        let incoming_val = incoming.get(key).map(|s| s.as_str()).unwrap_or("");
        if incoming_val.is_empty() || incoming_val == "***" {
            if let Some(old) = existing.get(key) {
                if !old.is_empty() && old != "***" {
                    incoming.insert(key.to_string(), old.clone());
                }
            }
        }
    }
}

fn is_supported_database_type(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "postgres" | "mysql" | "mariadb" | "sqlserver" | "oracle"
    )
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::models::{
        CaptureMode, DeliveryMode, FlowConfig, ScheduleConfig, SchemaSelectionMode,
        TableSelectionMode, TargetConfig,
    };

    fn empty_draft() -> ProfileUpsertRequest {
        ProfileUpsertRequest {
            name: String::new(),
            description: None,
            pattern: Some(ProfilePattern::SourceToTarget),
            stream_platform: None,
            transform_strategy: Some(TransformStrategy::None),
            source: SourceConfig {
                source_type: "postgres".into(),
                connection: HashMap::new(),
                capture_mode: CaptureMode::Logical,
                schema_mode: SchemaSelectionMode::SpecificSchemas,
                table_mode: TableSelectionMode::AllTables,
                schemas: vec![],
                tables: vec![],
                changelog_retention_hours: None,
                timestamp_column: None,
                incrementing_column: None,
            },
            target: Some(TargetConfig {
                name: String::new(),
                target_type: "postgres".into(),
                connection: HashMap::new(),
                schemas: vec![],
            }),
            lake: None,
            governance: None,
            sink_error_policy: Default::default(),
            flow: FlowConfig {
                transforms: vec![],
                mappings: vec![],
                schedule: ScheduleConfig {
                    delivery_mode: DeliveryMode::Realtime,
                    interval_seconds: None,
                    full_refresh_minutes: None,
                },
            },
        }
    }

    #[test]
    fn draft_payload_skips_connection_rules() {
        assert!(validate_profile_payload(&empty_draft(), false).is_ok());
        assert!(validate_profile_payload(&empty_draft(), true).is_err());
    }

    #[test]
    fn draft_profile_gets_a_name() {
        let profile = build_profile(None, empty_draft()).unwrap();
        assert_eq!(profile.name, "Draft · postgres");
        assert_eq!(profile.status, ProfileStatus::Draft);
    }

    #[test]
    fn incremental_requires_tables_and_watermark() {
        let mut payload = empty_draft();
        payload.name = "incr".into();
        payload.source.capture_mode = CaptureMode::Incremental;
        payload.source.connection = HashMap::from([
            ("host".into(), "src".into()),
            ("port".into(), "5432".into()),
            ("database".into(), "demo".into()),
            ("username".into(), "demo".into()),
            ("password".into(), "demo".into()),
        ]);
        payload.target.as_mut().unwrap().connection = payload.source.connection.clone();
        assert!(validate_profile_payload(&payload, true).is_err());
        payload.source.tables = vec!["bookings.tickets".into()];
        payload.source.schemas = vec!["bookings".into()];
        assert!(validate_profile_payload(&payload, true).is_err());
        payload.source.timestamp_column = Some("updated_at".into());
        assert!(validate_profile_payload(&payload, true).is_err());
        payload.source.incrementing_column = Some("id".into());
        assert!(validate_profile_payload(&payload, true).is_ok());
    }
}

// ── Schema Compare Profile CRUD ───────────────────────────────────────────────

pub async fn list_compare_profiles(
    State(state): State<AppState>,
) -> Result<Json<SchemaCompareProfilesResponse>, AppError> {
    let profiles = state.compare_profiles.list().await?;
    Ok(Json(SchemaCompareProfilesResponse { profiles }))
}

pub async fn create_compare_profile(
    State(state): State<AppState>,
    Json(req): Json<SchemaCompareProfileRequest>,
) -> Result<Json<SchemaCompareProfile>, AppError> {
    let now = Utc::now();
    let profile = SchemaCompareProfile {
        id: Uuid::new_v4(),
        name: req.name.trim().to_string(),
        source: req.source,
        target: req.target,
        created_at: now,
        updated_at: now,
    };
    if profile.name.is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }
    state.compare_profiles.upsert(&profile).await?;
    Ok(Json(profile))
}

pub async fn get_compare_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SchemaCompareProfile>, AppError> {
    let profile = state
        .compare_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("compare profile {id} not found")))?;
    Ok(Json(profile))
}

pub async fn update_compare_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<SchemaCompareProfileRequest>,
) -> Result<Json<SchemaCompareProfile>, AppError> {
    let existing = state
        .compare_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("compare profile {id} not found")))?;

    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }

    let profile = SchemaCompareProfile {
        id,
        name: req.name.trim().to_string(),
        source: req.source,
        target: req.target,
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    state.compare_profiles.upsert(&profile).await?;
    Ok(Json(profile))
}

pub async fn delete_compare_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.compare_profiles.delete(id).await?;
    Ok(Json(json!({ "status": true, "id": id })))
}

/// Execute a DDL statement (CREATE SCHEMA, CREATE TABLE, ALTER TABLE …)
/// against the target database specified in the request body.
pub async fn exec_compare_ddl(
    State(_state): State<AppState>,
    Json(req): Json<ExecDdlRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let stmt = req.statement.trim();
    if stmt.is_empty() {
        return Err(AppError::BadRequest("statement is required".into()));
    }
    let mut connection = std::collections::HashMap::new();
    connection.insert("host".to_string(), req.target.host.clone());
    connection.insert("port".to_string(), req.target.port.clone());
    connection.insert("database".to_string(), req.target.database.clone());
    connection.insert("username".to_string(), req.target.username.clone());
    connection.insert("password".to_string(), req.target.password.clone());

    crate::connector_catalog::exec_ddl_on_connection(&req.target.db_type, &connection, stmt)
        .await?;
    Ok(Json(json!({ "status": true, "statement": stmt })))
}

/// Copy rows from source tables to matching target tables.
///
/// Runs SELECT (source) → batch INSERT (target) for each requested table.
/// Returns per-table row counts and any errors encountered.  Tables that fail
/// are reported individually so that a single error does not abort the whole
/// migration run.
pub async fn copy_compare_data(
    State(_state): State<AppState>,
    Json(req): Json<CopyDataRequest>,
) -> Result<Json<CopyDataResponse>, AppError> {
    if req.tables.is_empty() {
        return Err(AppError::BadRequest("tables list must not be empty".into()));
    }

    let mut src_conn = HashMap::new();
    src_conn.insert("host".to_string(), req.source.host.clone());
    src_conn.insert("port".to_string(), req.source.port.clone());
    src_conn.insert("database".to_string(), req.source.database.clone());
    src_conn.insert("username".to_string(), req.source.username.clone());
    src_conn.insert("password".to_string(), req.source.password.clone());

    let mut tgt_conn = HashMap::new();
    tgt_conn.insert("host".to_string(), req.target.host.clone());
    tgt_conn.insert("port".to_string(), req.target.port.clone());
    tgt_conn.insert("database".to_string(), req.target.database.clone());
    tgt_conn.insert("username".to_string(), req.target.username.clone());
    tgt_conn.insert("password".to_string(), req.target.password.clone());

    let mut table_results: Vec<CopyTableResult> = Vec::new();
    let mut total_rows_copied: i64 = 0;
    let mut errors: usize = 0;

    let copy_timeout = if req.force_direct_copy {
        Duration::from_secs(1800) // 30 minutes when force flag is set
    } else {
        Duration::from_secs(300) // 5 minutes default
    };

    for spec in &req.tables {
        match timeout(
            copy_timeout,
            crate::connector_catalog::copy_table_data(
                &req.source.db_type,
                &src_conn,
                &req.target.db_type,
                &tgt_conn,
                &spec.schema,
                &spec.name,
                req.batch_size,
                req.truncate_before_copy,
            ),
        )
        .await
        {
            Ok(Ok(rows)) => {
                total_rows_copied += rows;
                table_results.push(CopyTableResult {
                    schema: spec.schema.clone(),
                    name: spec.name.clone(),
                    rows_copied: rows,
                    status: "ok".into(),
                    error: None,
                });
            }
            Ok(Err(e)) => {
                errors += 1;
                table_results.push(CopyTableResult {
                    schema: spec.schema.clone(),
                    name: spec.name.clone(),
                    rows_copied: 0,
                    status: "error".into(),
                    error: Some(e.to_string()),
                });
            }
            Err(_elapsed) => {
                errors += 1;
                let hint = if req.force_direct_copy {
                    "copy timed out after 30 minutes — table is too large for direct copy; use CDC SyncOnce".into()
                } else {
                    "copy timed out after 5 minutes — enable 'Force direct copy' for a longer timeout, or use CDC SyncOnce for very large tables".into()
                };
                table_results.push(CopyTableResult {
                    schema: spec.schema.clone(),
                    name: spec.name.clone(),
                    rows_copied: 0,
                    status: "error".into(),
                    error: Some(hint),
                });
            }
        }
    }

    Ok(Json(CopyDataResponse {
        tables: table_results,
        total_rows_copied,
        errors,
    }))
}

// ── Table Data Preview ────────────────────────────────────────────────────────

pub async fn count_compare_schema(
    State(_state): State<AppState>,
    Json(req): Json<SchemaCountsRequest>,
) -> Result<Json<SchemaCountsResponse>, AppError> {
    let mut conn = HashMap::new();
    conn.insert("host".to_string(), req.connection.host.clone());
    conn.insert("port".to_string(), req.connection.port.clone());
    conn.insert("database".to_string(), req.connection.database.clone());
    conn.insert("username".to_string(), req.connection.username.clone());
    conn.insert("password".to_string(), req.connection.password.clone());

    let resp = timeout(
        Duration::from_secs(30),
        crate::connector_catalog::count_schema_tables(&req.connection.db_type, &conn, &req.schema),
    )
    .await
    .map_err(|_| AppError::BadRequest("schema counts timed out after 30 seconds".into()))??;

    Ok(Json(resp))
}

pub async fn count_compare_table(
    State(_state): State<AppState>,
    Json(req): Json<TableCountRequest>,
) -> Result<Json<TableCountResponse>, AppError> {
    let mut conn = HashMap::new();
    conn.insert("host".to_string(), req.connection.host.clone());
    conn.insert("port".to_string(), req.connection.port.clone());
    conn.insert("database".to_string(), req.connection.database.clone());
    conn.insert("username".to_string(), req.connection.username.clone());
    conn.insert("password".to_string(), req.connection.password.clone());

    let resp = timeout(
        Duration::from_secs(30),
        crate::connector_catalog::count_table_rows(
            &req.connection.db_type,
            &conn,
            &req.schema,
            &req.table,
        ),
    )
    .await
    .map_err(|_| AppError::BadRequest("row count timed out after 30 seconds".into()))??;

    Ok(Json(resp))
}

pub async fn preview_compare_table(
    State(_state): State<AppState>,
    Json(req): Json<TablePreviewRequest>,
) -> Result<Json<TablePreviewResponse>, AppError> {
    let mut conn = HashMap::new();
    conn.insert("host".to_string(), req.connection.host.clone());
    conn.insert("port".to_string(), req.connection.port.clone());
    conn.insert("database".to_string(), req.connection.database.clone());
    conn.insert("username".to_string(), req.connection.username.clone());
    conn.insert("password".to_string(), req.connection.password.clone());

    let page_size = req.page_size.clamp(1, 200);
    let resp = timeout(
        Duration::from_secs(30),
        crate::connector_catalog::preview_table_data(
            &req.connection.db_type,
            &conn,
            &req.schema,
            &req.table,
            req.page,
            page_size,
            req.search.as_deref(),
        ),
    )
    .await
    .map_err(|_| AppError::BadRequest("preview timed out after 30 seconds".into()))??;

    Ok(Json(resp))
}
