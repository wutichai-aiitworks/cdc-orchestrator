mod audit;
mod clients;
mod command;
mod config;
mod connector_catalog;
mod connector_meta;
mod connector_parse;
mod engine;
mod error;
mod handlers;
mod incremental_cdc;
mod kafka_admin;
mod kafka_topics;
mod lake;
mod migration_tools;
mod migration_tools_handlers;
mod models;
mod mongocopy;
mod mongocopy_handlers;
mod ops;
mod pgcopy;
mod pgcopy_handlers;
mod pgloader;
mod pgloader_handlers;
mod postgres_trigger_cdc;
mod processing_api;
mod reconcile;
mod redisshake;
mod redisshake_handlers;
mod store;
mod trigger_cdc;
mod watcher;

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::AlertHistory;
use crate::store::{
    CompareProfileStore, MongoCopyProfileStore, PgCopyProfileStore, PgLoaderProfileStore,
    ProfileStore, RedisShakeProfileStore,
};
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::Router;
use hyper_cdc_orchestrator::persist::RqliteClient;
use reqwest::Client;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, RwLock};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use crate::models::ConnectorSnapshot;
use hyper_cdc_orchestrator::function_runtime::store::ProcessingPipelineStore;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub http: Client,
    pub profiles: ProfileStore,
    pub compare_profiles: CompareProfileStore,
    pub pgcopy_profiles: PgCopyProfileStore,
    pub redisshake_profiles: RedisShakeProfileStore,
    pub pgloader_profiles: PgLoaderProfileStore,
    pub mongocopy_profiles: MongoCopyProfileStore,
    pub rqlite: Option<RqliteClient>,
    pub history: Arc<RwLock<AlertHistory>>,
    pub alerts_tx: broadcast::Sender<String>,
    pub connector_snapshots: Arc<RwLock<HashMap<String, ConnectorSnapshot>>>,
    pub dlq_high_watermarks: Arc<RwLock<HashMap<String, i64>>>,
    pub watcher_errors: Arc<RwLock<u64>>,
    pub processing_pipelines: ProcessingPipelineStore,
}

async fn open_rqlite(config: &AppConfig) -> Option<RqliteClient> {
    let Some(url) = &config.rqlite_url else {
        if config.require_persistence {
            eprintln!("FATAL: ORCH_RQLITE_URL required but unset");
            std::process::exit(1);
        }
        info!("ORCH_RQLITE_URL unset; configuration stored in memory only");
        return None;
    };
    match RqliteClient::try_new(url, config.rqlite_user.clone(), config.rqlite_pass.clone()) {
        Ok(client) => rqlite_after_connect(config, url, client).await,
        Err(e) => {
            if config.require_persistence {
                eprintln!("FATAL: rqlite client init failed under ORCH_REQUIRE_PERSISTENCE: {e}");
                std::process::exit(1);
            }
            warn!(error = %e, "rqlite client init failed; memory-only configuration");
            None
        }
    }
}

async fn rqlite_after_connect(
    config: &AppConfig,
    url: &str,
    client: RqliteClient,
) -> Option<RqliteClient> {
    if let Err(e) = client.ensure_schema().await {
        if config.require_persistence {
            eprintln!("FATAL: rqlite schema init failed under ORCH_REQUIRE_PERSISTENCE: {e}");
            std::process::exit(1);
        }
        warn!(error = %e, "rqlite schema init failed; memory-only configuration");
        return None;
    }
    info!(rqlite = %url, "rqlite configuration store enabled");
    Some(client)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "hyper_cdc_orchestrator=info,tower_http=info".to_string()),
        )
        .init();

    let config = AppConfig::from_env();
    config.validate_or_die();

    let rqlite = open_rqlite(&config).await;

    if let Some(url) = &config.jdbc_admin_url {
        info!(
            jdbc_admin = %url,
            token_set = config.jdbc_admin_token.is_some(),
            "cdc-jdbc-admin configured for Type 4 catalog/DDL/preview"
        );
    } else {
        info!("ORCH_JDBC_ADMIN_URL unset; catalog falls back to native drivers where available");
    }

    let (alerts_tx, _) = broadcast::channel(256);
    let state = AppState {
        config: config.clone(),
        http: Client::builder()
            .timeout(Duration::from_secs(config.command_timeout_secs))
            .build()
            .expect("build reqwest client"),
        profiles: ProfileStore::new(rqlite.clone(), config.strict_persistence),
        compare_profiles: CompareProfileStore::new(rqlite.clone()),
        pgcopy_profiles: PgCopyProfileStore::new(rqlite.clone()),
        redisshake_profiles: RedisShakeProfileStore::new(rqlite.clone()),
        pgloader_profiles: PgLoaderProfileStore::new(rqlite.clone()),
        mongocopy_profiles: MongoCopyProfileStore::new(rqlite.clone()),
        rqlite: rqlite.clone(),
        history: Arc::new(RwLock::new(AlertHistory::new(config.history_capacity))),
        alerts_tx,
        connector_snapshots: Arc::new(RwLock::new(HashMap::new())),
        dlq_high_watermarks: Arc::new(RwLock::new(HashMap::new())),
        watcher_errors: Arc::new(RwLock::new(0)),
        processing_pipelines: ProcessingPipelineStore::new(rqlite),
    };

    let legacy = Router::new()
        .route("/engine/status", get(handlers::engine_status))
        .route("/engine/up", post(handlers::engine_up))
        .route("/engine/down", post(handlers::engine_down))
        .route("/engine/restart", post(handlers::engine_restart))
        .route("/connect/connectors", get(handlers::list_connectors))
        .route("/connect/connectors", post(handlers::create_connector))
        .route(
            "/connect/connectors/templates",
            get(handlers::connector_templates),
        )
        .route(
            "/connect/connectors/validate",
            post(handlers::connector_validate),
        )
        .route("/connect/connectors/probe", post(handlers::connector_probe))
        .route(
            "/connect/connectors/catalog",
            post(handlers::connector_catalog),
        )
        .route(
            "/connect/connectors/:name/status",
            get(handlers::connector_status),
        )
        .route(
            "/connect/connectors/:name/timeline",
            get(handlers::connector_timeline),
        )
        .route(
            "/connect/connectors/:name/config",
            get(handlers::get_connector_config).put(handlers::update_connector),
        )
        .route(
            "/connect/connectors/:name/:action",
            post(handlers::connector_action),
        )
        .route(
            "/connect/connectors/:name",
            delete(handlers::delete_connector),
        )
        .route("/kafka/topics", get(handlers::kafka_topics))
        .route(
            "/topics",
            get(kafka_topics::list_topics_paged).post(kafka_topics::create_topic),
        )
        .route("/topics/:topic", delete(kafka_topics::delete_topic))
        .route(
            "/topics/:topic/recreate",
            post(kafka_topics::recreate_topic),
        )
        .route(
            "/topics/:topic/clear-messages",
            post(kafka_topics::clear_topic_messages),
        )
        .route("/topics/:topic/metadata", get(handlers::topic_metadata))
        .route("/topics/:topic/messages", get(handlers::topic_messages))
        .route("/kafka/groups", get(handlers::kafka_groups))
        .route(
            "/consumer-groups",
            get(handlers::list_consumer_groups_paged),
        )
        .route("/kafka/groups/:group/lag", get(handlers::kafka_group_lag))
        .route("/profiles", get(handlers::list_profiles))
        .route("/profiles", post(handlers::create_profile))
        .route("/profiles/cases", post(handlers::create_profile_from_case))
        .route("/profiles/validate/source", post(handlers::validate_source))
        .route(
            "/profiles/catalog/source",
            post(handlers::browse_source_catalog),
        )
        .route(
            "/profiles/catalog/source/table",
            post(handlers::inspect_source_table),
        )
        .route(
            "/profiles/catalog/source/triggers",
            post(handlers::install_source_triggers),
        )
        .route(
            "/profiles/catalog/source/triggers/preview",
            post(handlers::preview_source_triggers),
        )
        .route(
            "/profiles/catalog/source/triggers/status",
            post(handlers::source_trigger_status),
        )
        .route(
            "/profiles/catalog/target",
            post(handlers::browse_target_catalog),
        )
        .route(
            "/profiles/catalog/target/table",
            post(handlers::inspect_target_table),
        )
        .route("/profiles/validate/target", post(handlers::validate_target))
        .route("/profiles/validate/lake", post(handlers::validate_lake))
        .route("/profiles/:id", get(handlers::get_profile))
        .route("/profiles/:id", put(handlers::update_profile))
        .route("/profiles/:id", delete(handlers::delete_profile))
        .route("/profiles/:id/dlq", get(kafka_topics::profile_dlq_status))
        .route("/profiles/:id/enable", post(handlers::set_profile_enabled))
        .route(
            "/profiles/:id/actions/:action",
            post(handlers::profile_action),
        )
        .route(
            "/processing-pipelines",
            get(processing_api::list_pipelines).post(processing_api::create_pipeline),
        )
        .route(
            "/processing-pipelines/validate",
            post(processing_api::validate_pipeline),
        )
        .route(
            "/processing-pipelines/:id",
            get(processing_api::get_pipeline)
                .put(processing_api::update_pipeline)
                .delete(processing_api::delete_pipeline),
        );

    let cdc = Router::new()
        .route("/capability", get(ops::capability))
        .route("/engine/status", get(handlers::engine_status))
        .route("/engine/up", post(handlers::engine_up))
        .route("/engine/down", post(handlers::engine_down))
        .route("/engine/restart", post(handlers::engine_restart))
        .route("/cluster", get(ops::cluster_info))
        .route("/connectors", get(handlers::list_connectors))
        .route("/connectors", post(handlers::create_connector))
        .route("/connectors/templates", get(handlers::connector_templates))
        .route("/connectors/validate", post(handlers::connector_validate))
        .route("/connectors/probe", post(handlers::connector_probe))
        .route("/connectors/catalog", post(handlers::connector_catalog))
        .route("/connectors/:name/status", get(handlers::connector_status))
        .route(
            "/connectors/:name/timeline",
            get(handlers::connector_timeline),
        )
        .route(
            "/connectors/:name/config",
            get(handlers::get_connector_config).put(handlers::update_connector),
        )
        .route(
            "/connectors/:name/tasks/:task/restart",
            post(ops::restart_connector_task),
        )
        .route(
            "/connectors/:name/:action",
            post(handlers::connector_action),
        )
        .route("/connectors/:name", delete(handlers::delete_connector))
        .route("/connectors/plugins", get(handlers::list_connector_plugins))
        .route("/connectors/:name/topics", get(handlers::connector_topics))
        .route("/kafka/topics", get(handlers::kafka_topics))
        .route(
            "/topics",
            get(kafka_topics::list_topics_paged).post(kafka_topics::create_topic),
        )
        .route("/topics/:topic", delete(kafka_topics::delete_topic))
        .route(
            "/topics/:topic/recreate",
            post(kafka_topics::recreate_topic),
        )
        .route(
            "/topics/:topic/clear-messages",
            post(kafka_topics::clear_topic_messages),
        )
        .route("/topics/:topic/copy", post(kafka_topics::copy_topic))
        .route(
            "/topics/:topic/configs",
            put(kafka_topics::update_topic_configs),
        )
        .route("/topics/:topic/metadata", get(handlers::topic_metadata))
        .route(
            "/topics/:topic/messages",
            get(handlers::topic_messages).post(kafka_admin::produce_message),
        )
        .route("/kafka/groups", get(handlers::kafka_groups))
        .route(
            "/consumer-groups",
            get(handlers::list_consumer_groups_paged),
        )
        .route("/kafka/groups/:group/lag", get(handlers::kafka_group_lag))
        .route(
            "/consumer-groups/:group",
            get(kafka_admin::get_consumer_group).delete(kafka_admin::delete_consumer_group),
        )
        .route(
            "/consumer-groups/:group/lag",
            get(handlers::kafka_group_lag),
        )
        .route(
            "/consumer-groups/:group/reset",
            post(kafka_admin::reset_consumer_group),
        )
        .route(
            "/consumer-groups/:group/topics/:topic/offsets",
            delete(kafka_admin::delete_topic_offsets),
        )
        .route("/brokers", get(kafka_admin::list_brokers))
        .route("/brokers/:id", get(kafka_admin::get_broker))
        .route("/brokers/:id/logdirs", get(kafka_admin::get_broker_logdirs))
        .route(
            "/brokers/:id/configs",
            get(kafka_admin::get_broker_configs).put(kafka_admin::update_broker_config),
        )
        .route("/brokers/:id/metrics", get(kafka_admin::get_broker_metrics))
        .route(
            "/acls",
            get(kafka_admin::list_acls)
                .post(kafka_admin::create_acl)
                .delete(kafka_admin::delete_acl),
        )
        .route(
            "/schemas",
            get(kafka_admin::list_schemas).post(kafka_admin::create_schema),
        )
        .route(
            "/schemas/:subject",
            get(kafka_admin::get_schema)
                .put(kafka_admin::update_schema)
                .delete(kafka_admin::delete_schema),
        )
        .route(
            "/schemas/:subject/compatibility",
            post(kafka_admin::check_schema_compatibility)
                .put(kafka_admin::set_schema_compatibility),
        )
        .route(
            "/schemas/:subject/versions/:version",
            delete(kafka_admin::delete_schema_version),
        )
        .route("/ksqldb/tables", get(kafka_admin::ksql_tables))
        .route("/ksqldb/streams", get(kafka_admin::ksql_streams))
        .route("/ksqldb/queries", get(kafka_admin::ksql_queries))
        .route("/ksqldb/info", get(kafka_admin::ksql_info))
        .route("/ksqldb/query", post(kafka_admin::ksql_query))
        .route("/ksqldb/terminate", post(kafka_admin::ksql_terminate))
        .route("/ksqldb/drop", post(kafka_admin::ksql_drop))
        .route("/ksqldb/explain/:query_id", get(kafka_admin::ksql_explain))
        .route("/profiles", get(handlers::list_profiles))
        .route("/profiles", post(handlers::create_profile))
        .route("/profiles/cases", post(handlers::create_profile_from_case))
        .route("/profiles/validate/source", post(handlers::validate_source))
        .route(
            "/profiles/catalog/source",
            post(handlers::browse_source_catalog),
        )
        .route(
            "/profiles/catalog/source/table",
            post(handlers::inspect_source_table),
        )
        .route(
            "/profiles/catalog/source/triggers",
            post(handlers::install_source_triggers),
        )
        .route(
            "/profiles/catalog/source/triggers/preview",
            post(handlers::preview_source_triggers),
        )
        .route(
            "/profiles/catalog/source/triggers/status",
            post(handlers::source_trigger_status),
        )
        .route(
            "/profiles/catalog/target",
            post(handlers::browse_target_catalog),
        )
        .route(
            "/profiles/catalog/target/table",
            post(handlers::inspect_target_table),
        )
        .route("/profiles/validate/target", post(handlers::validate_target))
        .route("/profiles/validate/lake", post(handlers::validate_lake))
        .route("/profiles/:id", get(handlers::get_profile))
        .route("/profiles/:id", put(handlers::update_profile))
        .route("/profiles/:id", delete(handlers::delete_profile))
        .route("/profiles/:id/dlq", get(kafka_topics::profile_dlq_status))
        .route("/profiles/:id/enable", post(handlers::set_profile_enabled))
        .route(
            "/profiles/:id/actions/:action",
            post(handlers::profile_action),
        )
        .route(
            "/processing-pipelines",
            get(processing_api::list_pipelines).post(processing_api::create_pipeline),
        )
        .route(
            "/processing-pipelines/validate",
            post(processing_api::validate_pipeline),
        )
        .route(
            "/processing-pipelines/:id",
            get(processing_api::get_pipeline)
                .put(processing_api::update_pipeline)
                .delete(processing_api::delete_pipeline),
        )
        .route("/logs", get(ops::log_services))
        .route("/logs/loggers", get(ops::get_loggers))
        .route("/logs/loggers/:logger", put(ops::put_logger))
        .route("/logs/:service", get(ops::snapshot_logs))
        .route("/logs/:service/stream", get(ops::stream_logs))
        .route(
            "/notifications/rules",
            get(ops::list_rules).post(ops::create_rule),
        )
        .route(
            "/notifications/rules/:id",
            get(ops::get_rule)
                .put(ops::update_rule)
                .delete(ops::delete_rule),
        )
        .route("/notifications/history", get(ops::alert_history))
        .route("/ws/alerts", get(ops::ws_alerts))
        .route("/metrics", get(ops::metrics))
        .route("/compare/profiles", get(handlers::list_compare_profiles))
        .route("/compare/profiles", post(handlers::create_compare_profile))
        .route("/compare/profiles/:id", get(handlers::get_compare_profile))
        .route(
            "/compare/profiles/:id",
            put(handlers::update_compare_profile),
        )
        .route(
            "/compare/profiles/:id",
            delete(handlers::delete_compare_profile),
        )
        .route("/compare/target/exec-ddl", post(handlers::exec_compare_ddl))
        .route("/compare/data/copy", post(handlers::copy_compare_data))
        .route(
            "/compare/table/preview",
            post(handlers::preview_compare_table),
        )
        .route("/compare/table/count", post(handlers::count_compare_table))
        .route(
            "/compare/schema/counts",
            post(handlers::count_compare_schema),
        )
        .route(
            "/migration-tools/discover",
            get(migration_tools_handlers::migration_tools_discover),
        )
        .route(
            "/migration-tools/:tool_id/discover",
            get(migration_tools_handlers::migration_tool_discover),
        )
        .route(
            "/migration-tools/:tool_id/contract",
            get(migration_tools_handlers::migration_tool_contract),
        )
        .route(
            "/pgcopy/discover",
            get(migration_tools_handlers::pgcopy_discover),
        )
        .route(
            "/pgcopy/contract",
            get(migration_tools_handlers::pgcopy_contract),
        )
        .route(
            "/pgcopy/capability",
            get(pgcopy_handlers::pgcopy_capability),
        )
        .route("/pgcopy/jobs", get(pgcopy_handlers::list_pgcopy_jobs))
        .route("/pgcopy/jobs/:job_id", get(pgcopy_handlers::get_pgcopy_job))
        .route(
            "/pgcopy/jobs/:job_id/logs",
            get(pgcopy_handlers::pgcopy_job_logs),
        )
        .route(
            "/pgcopy/jobs/:job_id/stop",
            post(pgcopy_handlers::stop_pgcopy_job),
        )
        .route(
            "/pgcopy/profiles",
            get(pgcopy_handlers::list_pgcopy_profiles),
        )
        .route(
            "/pgcopy/profiles",
            post(pgcopy_handlers::create_pgcopy_profile),
        )
        .route(
            "/pgcopy/profiles/:id",
            get(pgcopy_handlers::get_pgcopy_profile)
                .put(pgcopy_handlers::update_pgcopy_profile)
                .delete(pgcopy_handlers::delete_pgcopy_profile),
        )
        .route(
            "/pgcopy/profiles/:id/run",
            post(pgcopy_handlers::run_pgcopy_profile),
        )
        .route(
            "/pgcopy/validate",
            post(pgcopy_handlers::validate_pgcopy_connection),
        )
        .route(
            "/redisshake/discover",
            get(migration_tools_handlers::redisshake_discover),
        )
        .route(
            "/redisshake/contract",
            get(migration_tools_handlers::redisshake_contract),
        )
        .route(
            "/redisshake/capability",
            get(redisshake_handlers::redisshake_capability),
        )
        .route(
            "/redisshake/jobs",
            get(redisshake_handlers::list_redisshake_jobs),
        )
        .route(
            "/redisshake/jobs/:job_id",
            get(redisshake_handlers::get_redisshake_job),
        )
        .route(
            "/redisshake/jobs/:job_id/logs",
            get(redisshake_handlers::redisshake_job_logs),
        )
        .route(
            "/redisshake/jobs/:job_id/stop",
            post(redisshake_handlers::stop_redisshake_job),
        )
        .route(
            "/redisshake/profiles",
            get(redisshake_handlers::list_redisshake_profiles)
                .post(redisshake_handlers::create_redisshake_profile),
        )
        .route(
            "/redisshake/profiles/:id",
            get(redisshake_handlers::get_redisshake_profile)
                .put(redisshake_handlers::update_redisshake_profile)
                .delete(redisshake_handlers::delete_redisshake_profile),
        )
        .route(
            "/redisshake/profiles/:id/run",
            post(redisshake_handlers::run_redisshake_profile),
        )
        .route(
            "/redisshake/validate",
            post(redisshake_handlers::validate_redisshake_connection),
        )
        .route(
            "/pgloader/discover",
            get(migration_tools_handlers::pgloader_discover),
        )
        .route(
            "/pgloader/contract",
            get(migration_tools_handlers::pgloader_contract),
        )
        .route(
            "/pgloader/capability",
            get(pgloader_handlers::pgloader_capability),
        )
        .route("/pgloader/jobs", get(pgloader_handlers::list_pgloader_jobs))
        .route(
            "/pgloader/jobs/:job_id",
            get(pgloader_handlers::get_pgloader_job),
        )
        .route(
            "/pgloader/jobs/:job_id/logs",
            get(pgloader_handlers::pgloader_job_logs),
        )
        .route(
            "/pgloader/jobs/:job_id/stop",
            post(pgloader_handlers::stop_pgloader_job),
        )
        .route(
            "/pgloader/profiles",
            get(pgloader_handlers::list_pgloader_profiles),
        )
        .route(
            "/pgloader/profiles",
            post(pgloader_handlers::create_pgloader_profile),
        )
        .route(
            "/pgloader/profiles/:id",
            get(pgloader_handlers::get_pgloader_profile)
                .put(pgloader_handlers::update_pgloader_profile)
                .delete(pgloader_handlers::delete_pgloader_profile),
        )
        .route(
            "/pgloader/profiles/:id/run",
            post(pgloader_handlers::run_pgloader_profile),
        )
        .route(
            "/pgloader/validate",
            post(pgloader_handlers::validate_pgloader_connection),
        )
        .route(
            "/mongocopy/discover",
            get(migration_tools_handlers::mongocopy_discover),
        )
        .route(
            "/mongocopy/contract",
            get(migration_tools_handlers::mongocopy_contract),
        )
        .route(
            "/mongocopy/capability",
            get(mongocopy_handlers::mongocopy_capability),
        )
        .route(
            "/mongocopy/jobs",
            get(mongocopy_handlers::list_mongocopy_jobs),
        )
        .route(
            "/mongocopy/jobs/:job_id",
            get(mongocopy_handlers::get_mongocopy_job),
        )
        .route(
            "/mongocopy/jobs/:job_id/logs",
            get(mongocopy_handlers::mongocopy_job_logs),
        )
        .route(
            "/mongocopy/jobs/:job_id/stop",
            post(mongocopy_handlers::stop_mongocopy_job),
        )
        .route(
            "/mongocopy/profiles",
            get(mongocopy_handlers::list_mongocopy_profiles),
        )
        .route(
            "/mongocopy/profiles",
            post(mongocopy_handlers::create_mongocopy_profile),
        )
        .route(
            "/mongocopy/profiles/:id",
            get(mongocopy_handlers::get_mongocopy_profile)
                .put(mongocopy_handlers::update_mongocopy_profile)
                .delete(mongocopy_handlers::delete_mongocopy_profile),
        )
        .route(
            "/mongocopy/profiles/:id/run",
            post(mongocopy_handlers::run_mongocopy_profile),
        )
        .route(
            "/mongocopy/validate",
            post(mongocopy_handlers::validate_mongocopy_connection),
        );

    let protected = Router::new()
        .merge(legacy)
        .nest("/cdc", cdc)
        .layer(from_fn_with_state(state.clone(), auth_middleware));

    let cors = build_cors_layer(&config.cors_origins);

    let app = Router::new()
        .route("/health", get(handlers::health))
        .merge(protected)
        .layer(cors)
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("cache-control"),
            HeaderValue::from_static("no-store"),
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let _watcher = watcher::spawn_watcher(Arc::new(state.clone()));
    let _pruner = watcher::spawn_changelog_pruner(Arc::new(state.clone()));
    let _refresher = watcher::spawn_refresh_scheduler(Arc::new(state.clone()));

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("parse bind address");

    info!(
        host = %config.host,
        port = config.port,
        env = %config.env_mode.as_str(),
        require_auth = config.require_auth,
        require_persistence = config.require_persistence,
        strict_persistence = config.strict_persistence,
        engine_provider = %config.engine_provider.as_str(),
        container_project = %config.container_project_name,
        connect_base_url = %config.connect_base_url,
        "hyper-cdc-orchestrator starting (unified /cdc API)"
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    axum::serve(listener, app).await.expect("run server");
}

fn build_cors_layer(origins: &[String]) -> CorsLayer {
    if origins.is_empty() {
        // Default deny browser cross-origin (WebUI uses same-origin Nitro proxy).
        return CorsLayer::new()
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT]);
    }
    if origins.iter().any(|o| o == "*") {
        warn!("ORCH_CORS_ORIGINS=* is permissive — prefer an explicit allow-list");
        return CorsLayer::permissive();
    }
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(parsed))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
}

fn bearer_matches(expected: &str, actual: Option<&str>) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    // Constant-time compare; length mismatch still compared via padded path.
    if actual.len() != expected.len() {
        let _ = actual.as_bytes().ct_eq(expected.as_bytes());
        return false;
    }
    bool::from(actual.as_bytes().ct_eq(expected.as_bytes()))
}

async fn auth_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    if let Some(expected_token) = &state.config.api_token {
        let actual = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim);

        if !bearer_matches(expected_token, actual) {
            warn!("request rejected: invalid or missing bearer token");
            return Err(AppError::Unauthorized(
                "missing or invalid authorization token".to_string(),
            ));
        }
    }

    let response = next.run(req).await;
    if response.status() == StatusCode::UNAUTHORIZED {
        warn!("request returned unauthorized");
    }
    Ok(response)
}
