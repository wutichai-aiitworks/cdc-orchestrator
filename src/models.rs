use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub service: String,
    pub status: String,
    pub enabled: bool,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ServiceStatus {
    pub name: String,
    pub state: String,
    pub health: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EngineStatusResponse {
    pub engine_target: String,
    pub services: Vec<ServiceStatus>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub action: String,
    pub status: String,
    pub message: String,
    pub audit_id: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ConnectorCreateRequest {
    pub name: String,
    pub config: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct ConnectorUpdateRequest {
    pub config: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorsResponse {
    pub connectors: Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorStatusResponse {
    pub name: String,
    pub status: Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ConnectorValidateRequest {
    pub name: Option<String>,
    pub config: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorValidationIssue {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ConnectorValidateResponse {
    pub valid: bool,
    pub normalized_name: Option<String>,
    pub errors: Vec<ConnectorValidationIssue>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<Value>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub database_type: String,
    pub config: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct ConnectorInspectRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub config: Option<HashMap<String, String>>,
    #[serde(default)]
    pub schema: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorConnectionPublic {
    pub host: String,
    pub port: String,
    pub database: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jdbc_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorProbeResponse {
    pub valid: bool,
    pub reachable: bool,
    pub kind: String,
    pub database_type: String,
    pub database_label: String,
    pub class_name: String,
    pub connection: ConnectorConnectionPublic,
    pub configured_schemas: Vec<String>,
    pub configured_tables: Vec<String>,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorCatalogSchema {
    pub name: String,
    pub table_count: usize,
    pub captured: bool,
}

#[derive(Debug, Serialize)]
pub struct ConnectorCatalogTable {
    pub schema: String,
    pub name: String,
    pub captured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_key: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorCatalogResponse {
    pub kind: String,
    pub database_type: String,
    pub database_label: String,
    pub class_name: String,
    pub connection: ConnectorConnectionPublic,
    pub reachable: bool,
    pub live: bool,
    pub message: String,
    pub configured_schemas: Vec<String>,
    pub configured_tables: Vec<String>,
    pub schemas: Vec<ConnectorCatalogSchema>,
    pub tables: Vec<ConnectorCatalogTable>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct SourceCatalogResponse {
    pub reachable: bool,
    pub message: String,
    pub schemas: Vec<ConnectorCatalogSchema>,
    pub tables: Vec<ConnectorCatalogTable>,
    pub timestamp: DateTime<Utc>,
}

/// Browse-catalog request — wraps SourceConfig and adds an optional profile_id so the server
/// can merge stored credentials when the caller does not re-supply a password.
#[derive(Debug, Deserialize)]
pub struct SourceCatalogRequest {
    #[serde(flatten)]
    pub source: SourceConfig,
    /// When supplied, stored credentials are merged before connecting.
    #[serde(default)]
    pub profile_id: Option<uuid::Uuid>,
}

/// Same for the target catalog.
#[derive(Debug, Deserialize)]
pub struct TargetCatalogRequest {
    #[serde(flatten)]
    pub target: TargetConfig,
    #[serde(default)]
    pub profile_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct SourceTableInspectRequest {
    pub source: SourceConfig,
    pub schema: String,
    pub name: String,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Optional: when supplied the server merges stored credentials before connecting,
    /// so the caller does not have to re-supply a password for saved profiles.
    #[serde(default)]
    pub profile_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct TargetTableInspectRequest {
    pub target: TargetConfig,
    pub schema: String,
    pub name: String,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Optional: when supplied the server merges stored credentials before connecting.
    #[serde(default)]
    pub profile_id: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SourceTableColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub position: i32,
    pub primary_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pk_position: Option<i32>,
    pub unique: bool,
}

#[derive(Debug, Serialize)]
pub struct SourceTableInspectResponse {
    pub schema: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_count: Option<i64>,
    pub primary_key: Vec<String>,
    pub columns: Vec<SourceTableColumn>,
    pub rows: Vec<Value>,
    pub limit: u32,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorConfigResponse {
    pub name: String,
    pub config: Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorTemplatesResponse {
    pub templates: Vec<ConnectorTemplate>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorTimelineTaskError {
    #[serde(rename = "taskId")]
    pub task_id: u32,
    pub trace: String,
}

#[derive(Debug, Serialize)]
pub struct ConnectorTimelineEntry {
    pub ts: DateTime<Utc>,
    pub connector: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(default)]
    pub task_errors: Vec<ConnectorTimelineTaskError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorTimelineResponse {
    pub connector: String,
    pub total: usize,
    pub items: Vec<ConnectorTimelineEntry>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TopicItem {
    pub name: String,
    pub internal: bool,
    pub partition_count: i32,
    pub replication_factor: i32,
    pub out_of_sync_replicas: i32,
    pub messages_count: Option<i64>,
    pub high_watermark: Option<i64>,
    pub segment_size: Option<i64>,
    pub cleanup_policy: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TopicConfigParam {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTopicRequest {
    pub name: String,
    pub partitions: Option<i32>,
    pub replication_factor: Option<i32>,
    pub cleanup_policy: Option<String>,
    pub min_insync_replicas: Option<i32>,
    pub retention_ms: Option<i64>,
    pub retention_bytes: Option<i64>,
    pub max_message_bytes: Option<i64>,
    pub custom_params: Option<Vec<TopicConfigParam>>,
}

#[derive(Debug, Serialize)]
pub struct TopicsResponse {
    pub topics: Vec<TopicItem>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct GroupsResponse {
    pub groups: Vec<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct GroupLagItem {
    pub topic: String,
    pub partition: i32,
    pub current_offset: i64,
    pub log_end_offset: i64,
    pub lag: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GroupLagResponse {
    pub group: String,
    pub total_lag: i64,
    pub entries: Vec<GroupLagItem>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ProduceMessageRequest {
    pub value: String,
    pub key: Option<String>,
    pub partition: Option<i32>,
    pub headers: Option<Vec<TopicConfigParam>>,
}

#[derive(Debug, Deserialize)]
pub struct TopicConfigUpdateRequest {
    pub configs: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct CopyTopicRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct AclUpsertRequest {
    pub principal: String,
    pub host: Option<String>,
    pub resource_type: String,
    pub resource_name: String,
    pub pattern_type: Option<String>,
    pub operation: String,
    pub permission: String,
}

#[derive(Debug, Deserialize)]
pub struct SchemaCreateRequest {
    pub subject: String,
    pub schema: String,
    pub schema_type: Option<String>,
    pub compatibility: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SchemaUpdateRequest {
    pub schema: String,
    pub schema_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SchemaCompatibilityRequest {
    pub schema: String,
    pub schema_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetCompatibilityRequest {
    pub compatibility: String,
}

#[derive(Debug, Deserialize)]
pub struct KsqlQueryRequest {
    pub ksql: String,
    pub streams_properties: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
pub struct KsqlTerminateRequest {
    pub query_id: String,
}

#[derive(Debug, Deserialize)]
pub struct KsqlDropRequest {
    pub entity_type: String,
    pub name: String,
    pub delete_topic: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ResetOffsetsRequest {
    pub reset_type: String,
    pub topic: Option<String>,
    pub partitions: Option<Vec<i32>>,
    pub timestamp_ms: Option<i64>,
    pub offsets: Option<Vec<PartitionOffset>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PartitionOffset {
    pub partition: i32,
    pub offset: i64,
}

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub search: Option<String>,
    pub hide_internal: Option<bool>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PagedTopicsResponse {
    pub items: Vec<TopicItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ConsumerGroupItem {
    pub group: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lag: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PagedGroupsResponse {
    pub items: Vec<ConsumerGroupItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TopicMessageItem {
    pub partition: Option<i32>,
    pub offset: Option<i64>,
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub payload: String,
}

#[derive(Debug, Serialize)]
pub struct TopicMessagesResponse {
    pub topic: String,
    pub items: Vec<TopicMessageItem>,
    pub total: usize,
    pub limit: usize,
    pub offset: String,
    pub mode: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TopicPartitionMetadata {
    pub partition: i32,
    pub leader: Option<i32>,
    pub replicas: Vec<i32>,
    /// In-sync replicas — defaults to replicas when ISR column absent from rpk output
    pub in_sync_replicas: Vec<i32>,
    /// First (log-start) offset — equivalent to Kafbat "First Offset"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_start_offset: Option<i64>,
    /// Latest (high-watermark) offset — equivalent to Kafbat "Next Offset"
    pub high_watermark: Option<i64>,
    /// high_watermark - log_start_offset — equivalent to Kafbat "Message Count"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_count: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct TopicMetadataResponse {
    pub topic: String,
    pub partition_count: usize,
    /// Under-replicated partition count (partitions where ISR < replicas)
    pub urp: usize,
    pub partitions: Vec<TopicPartitionMetadata>,
    pub configs: HashMap<String, String>,
    pub raw: Vec<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SchemaSelectionMode {
    #[default]
    AllSchemas,
    SpecificSchemas,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TableSelectionMode {
    #[default]
    AllTables,
    SpecificTables,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// PostgreSQL logical decoding (wal_level=logical, pgoutput). Default ongoing CDC.
    #[default]
    Logical,
    /// Trigger-written changelog when WAL logical decoding is unavailable.
    Trigger,
    /// Copy current table rows once, then stop. No ongoing change capture.
    SyncOnce,
    /// JDBC poll on a timestamp / incrementing watermark. No WAL, no triggers.
    /// Hard deletes are not captured.
    Incremental,
}

impl SourceConfig {
    /// Host/port/database identity used to detect two capture engines on one source.
    pub fn capture_identity(&self) -> String {
        let typ = self.source_type.trim().to_ascii_lowercase();
        let host = self
            .connection
            .get("host")
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default();
        let port = self
            .connection
            .get("port")
            .map(|s| s.trim())
            .unwrap_or_default();
        let database = self
            .connection
            .get("database")
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default();
        format!("{typ}|{host}|{port}|{database}")
    }

    pub fn capture_identity_ready(&self) -> bool {
        let host = self
            .connection
            .get("host")
            .map(|s| s.trim())
            .unwrap_or_default();
        let database = self
            .connection
            .get("database")
            .map(|s| s.trim())
            .unwrap_or_default();
        !host.is_empty() || !database.is_empty()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SourceConfig {
    pub source_type: String,
    pub connection: HashMap<String, String>,
    #[serde(default)]
    pub capture_mode: CaptureMode,
    pub schema_mode: SchemaSelectionMode,
    #[serde(default)]
    pub table_mode: TableSelectionMode,
    pub schemas: Vec<String>,
    pub tables: Vec<String>,
    /// Trigger capture only: prune `hyper_cdc.change_log` rows older than this many
    /// hours so the changelog on a production source does not grow unbounded.
    /// `None`/`0` disables pruning. Choose a window comfortably larger than the
    /// maximum expected replication lag to avoid dropping unconsumed rows.
    #[serde(default)]
    pub changelog_retention_hours: Option<u32>,
    /// Incremental capture: timestamp/watermark column (e.g. `updated_at`).
    /// Combined with `incrementing_column` this uses JDBC `timestamp+incrementing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_column: Option<String>,
    /// Incremental capture: strictly incrementing column, usually the primary key
    /// (e.g. `id`). Required for replica upsert. Append-only tables can use this
    /// without a timestamp column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incrementing_column: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TargetConfig {
    #[serde(default)]
    pub name: String,
    pub target_type: String,
    pub connection: HashMap<String, String>,
    /// Optional schema filter — when non-empty only these schemas are returned by the catalog.
    #[serde(default)]
    pub schemas: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ColumnMapping {
    #[serde(default)]
    pub source: String,
    /// Multiple source columns (for merge / concat / add etc.).
    /// When non-empty this supersedes `source`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    pub target: String,
    #[serde(default)]
    pub data_type: String,
    /// Transform function ID (see transformFunctions.ts for the full catalogue).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<String>,
    /// Free-form arguments for the transform function.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform_args: Option<serde_json::Value>,
}

impl ColumnMapping {
    /// Returns the effective list of source columns:
    /// uses `sources` when non-empty, otherwise wraps `source` in a vec.
    pub fn effective_sources(&self) -> Vec<&str> {
        if !self.sources.is_empty() {
            self.sources.iter().map(|s| s.as_str()).collect()
        } else {
            vec![self.source.as_str()]
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TableMapping {
    pub source_schema: String,
    pub source_table: String,
    pub target_schema: String,
    pub target_table: String,
    /// When true, source and target table/column names match — no remap is applied.
    #[serde(default)]
    pub identical: bool,
    #[serde(default)]
    pub columns: Vec<ColumnMapping>,
    /// Logical primary-key columns on the target used by the trigger apply function for
    /// upsert/delete. Leave empty to let the orchestrator discover the real PK from the
    /// target catalog. Required when the target table has no PRIMARY KEY constraint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pk_columns: Vec<String>,
}

impl TableMapping {
    pub fn source_qualified(&self) -> String {
        qualified_table(&self.source_schema, &self.source_table)
    }

    pub fn is_remap(&self) -> bool {
        !self.identical
            && !self.source_table.trim().is_empty()
            && !self.target_table.trim().is_empty()
            && (self.source_table.trim() != self.target_table.trim()
                || self.source_schema.trim() != self.target_schema.trim()
                || !self.columns.is_empty())
    }
}

pub fn qualified_table(schema: &str, table: &str) -> String {
    let schema = schema.trim();
    let table = table.trim();
    if schema.is_empty() {
        table.to_string()
    } else {
        format!("{schema}.{table}")
    }
}

#[cfg(test)]
pub fn normalize_sql_type(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .split('(')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn validate_table_mappings(mappings: &[TableMapping]) -> Result<(), String> {
    let mut seen_source = std::collections::HashSet::new();
    for mapping in mappings {
        let source_table = mapping.source_table.trim();
        if source_table.is_empty() {
            return Err("each table mapping needs a source table".to_string());
        }
        let source_key = mapping.source_qualified().to_ascii_lowercase();
        if !seen_source.insert(source_key) {
            return Err(format!(
                "duplicate source table mapping: {}",
                mapping.source_qualified()
            ));
        }
        if mapping.identical {
            continue;
        }
        if mapping.target_table.trim().is_empty() {
            return Err(format!(
                "target table is required when mapping {}",
                mapping.source_qualified()
            ));
        }
        if mapping.columns.len() > 64 {
            return Err(format!(
                "too many column mappings for {}",
                mapping.source_qualified()
            ));
        }
        let (seen_source_col, seen_target_col) = validate_mapping_columns(mapping)?;
        if !mapping.columns.is_empty() && seen_target_col > seen_source_col {
            return Err(format!(
                "target {} cannot have more mapped columns than source {}",
                mapping.target_table.trim(),
                mapping.source_qualified()
            ));
        }
    }
    Ok(())
}

fn validate_mapping_columns(mapping: &TableMapping) -> Result<(usize, usize), String> {
    let mut seen_source_col = std::collections::HashSet::new();
    let mut seen_target_col = std::collections::HashSet::new();
    for column in &mapping.columns {
        let effective: Vec<&str> = column.effective_sources();
        let primary_source = effective.first().copied().unwrap_or(column.source.trim());
        let target = column.target.trim();
        if primary_source.is_empty() || target.is_empty() {
            return Err(format!(
                "column mappings for {} need both source and target column names",
                mapping.source_qualified()
            ));
        }
        for src in &effective {
            if src.is_empty() {
                return Err(format!(
                    "column mappings for {} contain an empty source column name",
                    mapping.source_qualified()
                ));
            }
            seen_source_col.insert(src.to_ascii_lowercase());
        }
        if !seen_target_col.insert(target.to_ascii_lowercase()) {
            return Err(format!(
                "duplicate target column {target} on {}",
                mapping.source_qualified()
            ));
        }
        if column.data_type.trim().is_empty() && column.transform.is_none() && effective.len() == 1
        {
            return Err(format!(
                "data type is required for {primary_source} → {target} on {}",
                mapping.source_qualified()
            ));
        }
    }
    Ok((seen_source_col.len(), seen_target_col.len()))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TransformConfig {
    pub name: String,
    pub enabled: bool,
    pub config: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Realtime,
    MicroBatch,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScheduleConfig {
    pub delivery_mode: DeliveryMode,
    pub interval_seconds: Option<u32>,
    /// Periodically empty the target and replay the whole source, in minutes.
    ///
    /// This is how updates and deletes are picked up from sources that cannot stream
    /// them — a legacy Oracle with no LogMiner, for example. `None` or `0` disables it.
    #[serde(default)]
    pub full_refresh_minutes: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FlowConfig {
    pub transforms: Vec<TransformConfig>,
    pub schedule: ScheduleConfig,
    #[serde(default)]
    pub mappings: Vec<TableMapping>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TableFormat {
    #[default]
    Iceberg,
    Parquet,
    Delta,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LakeZone {
    #[default]
    Bronze,
    Silver,
    Gold,
}

impl LakeZone {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bronze => "bronze",
            Self::Silver => "silver",
            Self::Gold => "gold",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChangeMode {
    #[default]
    Changelog,
    Upsert,
    Scd2,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LakeCatalogType {
    #[default]
    Jdbc,
    Rest,
    Hadoop,
    Glue,
    Nessie,
}

impl LakeCatalogType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jdbc => "jdbc",
            Self::Rest => "rest",
            Self::Hadoop => "hadoop",
            Self::Glue => "glue",
            Self::Nessie => "nessie",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Public,
    #[default]
    Internal,
    Confidential,
    Restricted,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GovernanceConfig {
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub dataset: String,
    #[serde(default)]
    pub sensitivity: Sensitivity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub industry_pack: Option<String>,
    #[serde(default)]
    pub pii_fields: Vec<String>,
}

impl GovernanceConfig {
    pub fn dataset_or(&self, fallback: &str) -> String {
        let dataset = self.dataset.trim();
        if dataset.is_empty() {
            fallback.trim().to_string()
        } else {
            dataset.to_string()
        }
    }

    pub fn identity(&self, pipeline_name: &str) -> String {
        let dataset = self.dataset_or(pipeline_name);
        let domain = self.domain.trim();
        if domain.is_empty() {
            dataset
        } else {
            format!("{domain}.{dataset}")
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LakeDestination {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub table_format: TableFormat,
    #[serde(default)]
    pub zone: LakeZone,
    #[serde(default)]
    pub change_mode: ChangeMode,
    #[serde(default)]
    pub catalog_type: LakeCatalogType,
    /// Iceberg/Hive namespace. Defaults to the zone name when empty.
    #[serde(default)]
    pub namespace: String,
    /// Warehouse URI, e.g. `s3://lake/warehouse`.
    #[serde(default)]
    pub warehouse: String,
    #[serde(default)]
    pub connection: HashMap<String, String>,
}

impl LakeDestination {
    pub fn namespace_or_zone(&self) -> String {
        let ns = self.namespace.trim();
        if ns.is_empty() {
            self.zone.as_str().to_string()
        } else {
            ns.to_string()
        }
    }

    pub fn warehouse_uri(&self) -> String {
        let warehouse = self.warehouse.trim();
        if !warehouse.is_empty() {
            return warehouse.to_string();
        }
        let bucket = self
            .connection
            .get("bucket")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("lake");
        format!("s3://{bucket}/warehouse")
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)]
pub enum ProfilePattern {
    SourceToStream,
    SourceToTarget,
    SourceToLake,
    /// Analytics-first: OLake reads the source (separate stack). Not Debezium.
    SourceToOlake,
}

impl ProfilePattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SourceToStream => "source_to_stream",
            Self::SourceToTarget => "source_to_target",
            Self::SourceToLake => "source_to_lake",
            Self::SourceToOlake => "source_to_olake",
        }
    }

    /// Debezium / JDBC source on the OLTP database (replica, event stream, Kafka→Iceberg).
    pub fn is_debezium_capture(&self) -> bool {
        matches!(
            self,
            Self::SourceToTarget | Self::SourceToStream | Self::SourceToLake
        )
    }

    pub fn is_olake_capture(&self) -> bool {
        matches!(self, Self::SourceToOlake)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransformStrategy {
    None,
    Predefined,
    Custom,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamPlatform {
    Kafka,
    Redpanda,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Draft,
    Validated,
    Running,
    Paused,
    Failed,
    Stopped,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SinkErrorPolicy {
    #[default]
    FailFast,
    Dlq,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HistoryItem {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub event: String,
    pub detail: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CdcProfile {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub pattern: ProfilePattern,
    pub stream_platform: Option<StreamPlatform>,
    pub transform_strategy: TransformStrategy,
    pub source: SourceConfig,
    pub target: Option<TargetConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lake: Option<LakeDestination>,
    #[serde(default)]
    pub governance: GovernanceConfig,
    #[serde(default)]
    pub sink_error_policy: SinkErrorPolicy,
    pub flow: FlowConfig,
    pub status: ProfileStatus,
    pub enabled: bool,
    pub last_error: Option<String>,
    pub history: Vec<HistoryItem>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Kafka Connect source connector name managed by this profile (set on start/enable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_name: Option<String>,
    /// Kafka Connect JDBC or lake sink connector name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sink_connector_name: Option<String>,
}

/// Upper bound on retained history entries per profile.
///
/// A profile on a short full-refresh cadence appends an entry every cycle, and the whole
/// profile record is rewritten on each save, so unbounded history would slowly bloat every
/// read and write.
pub const MAX_HISTORY_ITEMS: usize = 200;

/// Append a lifecycle event, dropping the oldest entries past `MAX_HISTORY_ITEMS`.
pub fn push_capped_history(history: &mut Vec<HistoryItem>, item: HistoryItem) {
    history.push(item);
    let len = history.len();
    if len > MAX_HISTORY_ITEMS {
        history.drain(..len - MAX_HISTORY_ITEMS);
    }
}

impl CdcProfile {
    /// Append a lifecycle event, keeping history bounded.
    pub fn push_history(&mut self, item: HistoryItem) {
        push_capped_history(&mut self.history, item);
    }

    pub fn has_managed_sink(&self) -> bool {
        matches!(
            self.pattern,
            ProfilePattern::SourceToTarget | ProfilePattern::SourceToLake
        )
    }

    pub fn dlq_topic_name(&self) -> Option<String> {
        if !self.has_managed_sink() {
            return None;
        }
        if self.effective_sink_error_policy() != SinkErrorPolicy::Dlq {
            return None;
        }
        let connector = self
            .sink_connector_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("cdc-{}-sink", profile_slug(&self.name)));
        Some(format!("{connector}-dlq"))
    }

    pub fn effective_sink_error_policy(&self) -> SinkErrorPolicy {
        if self.source.capture_mode == CaptureMode::Trigger {
            SinkErrorPolicy::Dlq
        } else {
            self.sink_error_policy
        }
    }

    /// Redact password fields before returning to clients.
    pub fn redact_secrets(&mut self) {
        redact_secret_map(&mut self.source.connection);
        if let Some(target) = self.target.as_mut() {
            redact_secret_map(&mut target.connection);
        }
        if let Some(lake) = self.lake.as_mut() {
            redact_secret_map(&mut lake.connection);
        }
    }
}

fn profile_slug(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase()
}

fn redact_secret_map(fields: &mut HashMap<String, String>) {
    for key in [
        "password",
        "secret",
        "token",
        "secret_key",
        "access_key",
        "catalog_password",
    ] {
        if let Some(v) = fields.get_mut(key) {
            if !v.is_empty() {
                *v = "***".to_string();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_identity_is_host_port_database() {
        let source = SourceConfig {
            source_type: "Postgres".into(),
            connection: HashMap::from([
                ("host".into(), "CDC-Postgres-Source".into()),
                ("port".into(), "5432".into()),
                ("database".into(), "Demo".into()),
            ]),
            capture_mode: CaptureMode::Logical,
            schema_mode: SchemaSelectionMode::AllSchemas,
            table_mode: TableSelectionMode::AllTables,
            schemas: vec![],
            tables: vec![],
            changelog_retention_hours: None,
            timestamp_column: None,
            incrementing_column: None,
        };
        assert_eq!(
            source.capture_identity(),
            "postgres|cdc-postgres-source|5432|demo"
        );
        assert!(source.capture_identity_ready());
        assert!(ProfilePattern::SourceToTarget.is_debezium_capture());
        assert!(ProfilePattern::SourceToOlake.is_olake_capture());
        assert!(!ProfilePattern::SourceToOlake.is_debezium_capture());
    }

    #[test]
    fn sink_error_policy_defaults_to_fail_fast() {
        assert_eq!(SinkErrorPolicy::default(), SinkErrorPolicy::FailFast);

        #[derive(Deserialize)]
        struct LegacyProfile {
            #[serde(default)]
            sink_error_policy: SinkErrorPolicy,
        }
        let legacy: LegacyProfile = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.sink_error_policy, SinkErrorPolicy::FailFast);

        #[derive(Deserialize)]
        struct LegacyUpdate {
            #[serde(default)]
            sink_error_policy: Option<SinkErrorPolicy>,
        }
        let legacy_update: LegacyUpdate = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy_update.sink_error_policy, None);
    }

    #[test]
    fn identical_mappings_skip_column_rules() {
        let mappings = vec![TableMapping {
            source_schema: "bookings".into(),
            source_table: "tickets".into(),
            target_schema: "bookings".into(),
            target_table: "tickets".into(),
            identical: true,
            columns: vec![],
            pk_columns: vec![],
        }];
        assert!(validate_table_mappings(&mappings).is_ok());
    }

    #[test]
    fn remap_requires_matching_named_types() {
        let mappings = vec![TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "public".into(),
            target_table: "b".into(),
            identical: false,
            columns: vec![
                ColumnMapping {
                    source: "id".into(),
                    sources: vec![],
                    target: "ticket_id".into(),
                    data_type: "int".into(),
                    transform: None,
                    transform_args: None,
                },
                ColumnMapping {
                    source: "email".into(),
                    sources: vec![],
                    target: "contact_email".into(),
                    data_type: "varchar(255)".into(),
                    transform: None,
                    transform_args: None,
                },
            ],
            pk_columns: vec![],
        }];
        assert!(validate_table_mappings(&mappings).is_ok());
        assert_eq!(normalize_sql_type("VARCHAR(255)"), "varchar");
    }

    #[test]
    fn remap_rejects_missing_target_table() {
        let mappings = vec![TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "public".into(),
            target_table: "".into(),
            identical: false,
            columns: vec![],
            pk_columns: vec![],
        }];
        assert!(validate_table_mappings(&mappings).is_err());
    }

    #[test]
    fn governance_identity_is_industry_agnostic() {
        let gov = GovernanceConfig {
            domain: "commerce".into(),
            dataset: "orders".into(),
            ..GovernanceConfig::default()
        };
        assert_eq!(gov.identity("pipeline"), "commerce.orders");
        let unnamed = GovernanceConfig::default();
        assert_eq!(unnamed.identity("Payments CDC"), "Payments CDC");
    }
}

#[derive(Debug, Serialize)]
pub struct CapabilityResponse {
    pub service: String,
    pub version: String,
    pub api_prefix: String,
    pub features: CapabilityFeatures,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct CapabilityFeatures {
    pub engine: bool,
    pub profiles: bool,
    pub connectors: bool,
    pub kafka: bool,
    pub logs: bool,
    pub notifications: bool,
    pub alerts_ws: bool,
    pub metrics: bool,
    pub persistence: bool,
    pub profile_reconcile: bool,
    pub function_processing: bool,
    pub datalake: bool,
    pub brokers: bool,
    pub schema_registry: bool,
    pub ksql: bool,
    pub acls: bool,
    pub pgcopy: bool,
    pub redisshake: bool,
    pub pgloader: bool,
    pub mongocopy: bool,
}

/// Alert rule persisted in rqlite (`cdc_config` kind `rule`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertRule {
    pub id: String,
    pub name: String,
    pub condition_type: String,
    #[serde(default)]
    pub condition_value: Option<Value>,
    pub connector_filter: String,
    pub channels: Vec<String>,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub webhook_secret: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskErrorBrief {
    #[serde(rename = "taskId")]
    pub task_id: u32,
    pub trace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CdcAlertPayload {
    #[serde(rename = "type")]
    pub type_: String,
    pub ts: DateTime<Utc>,
    pub connector: String,
    pub from: Option<String>,
    pub to: String,
    #[serde(default)]
    pub rule: Option<String>,
    #[serde(default)]
    pub task_errors: Vec<TaskErrorBrief>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    #[serde(flatten)]
    pub payload: CdcAlertPayload,
    #[serde(default)]
    pub resolved: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookBody {
    pub event: String,
    pub connector: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    pub ts: DateTime<Utc>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AlertHistory {
    deque: std::collections::VecDeque<HistoryEntry>,
    cap: usize,
}

impl AlertHistory {
    pub fn new(cap: usize) -> Self {
        Self {
            deque: std::collections::VecDeque::new(),
            cap,
        }
    }

    pub fn push(&mut self, entry: HistoryEntry) {
        self.deque.push_back(entry);
        while self.deque.len() > self.cap {
            self.deque.pop_front();
        }
    }

    pub fn list_recent_owned(&self) -> Vec<HistoryEntry> {
        self.deque.iter().rev().cloned().collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectorSnapshot {
    pub name: String,
    pub state: String,
    #[serde(default)]
    pub task_errors: Vec<TaskErrorBrief>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransition {
    pub connector: String,
    pub from_state: Option<String>,
    pub to_state: String,
    pub snapshot: ConnectorSnapshot,
}

#[derive(Debug, Deserialize)]
pub struct ProfileUpsertRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub pattern: Option<ProfilePattern>,
    pub stream_platform: Option<StreamPlatform>,
    pub transform_strategy: Option<TransformStrategy>,
    pub source: SourceConfig,
    pub target: Option<TargetConfig>,
    #[serde(default)]
    pub lake: Option<LakeDestination>,
    #[serde(default)]
    pub governance: Option<GovernanceConfig>,
    #[serde(default)]
    pub sink_error_policy: Option<SinkErrorPolicy>,
    pub flow: FlowConfig,
}

#[derive(Debug, Deserialize)]
pub struct ProfileCaseRequest {
    pub name: String,
    pub case_id: String,
    pub source: SourceConfig,
    pub target: Option<TargetConfig>,
    pub transform_preset: Option<String>,
    pub stream_platform: Option<StreamPlatform>,
    pub delivery_mode: Option<DeliveryMode>,
    pub interval_seconds: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ProfileEnableRequest {
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct ProfileResponse {
    pub profile: CdcProfile,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ProfilesResponse {
    pub profiles: Vec<CdcProfile>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProfileDlqStatus {
    pub profile_id: Uuid,
    pub configured_policy: SinkErrorPolicy,
    pub effective_policy: SinkErrorPolicy,
    pub enabled: bool,
    pub topic: Option<String>,
    pub exists: bool,
    pub messages_count: i64,
    pub high_watermark: i64,
    pub size_bytes: Option<i64>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ValidationResponse {
    pub valid: bool,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct CaptureInspectResponse {
    pub source_type: String,
    pub wal_level: Option<String>,
    pub logical_available: bool,
    pub trigger_objects_present: bool,
    pub recommended_mode: CaptureMode,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct TriggerInstallRequest {
    pub source: SourceConfig,
    #[serde(default)]
    pub tables: Vec<String>,
    /// When supplied, stored credentials are merged before connecting, so the caller
    /// does not have to re-supply a password for a saved profile.
    #[serde(default)]
    pub profile_id: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize)]
pub struct TriggerInstallResponse {
    pub installed: Vec<String>,
    pub already_present: Vec<String>,
    pub change_log: String,
    #[serde(default)]
    pub sql: String,
    #[serde(default)]
    pub executed: bool,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

// ── Schema Compare Profiles ───────────────────────────────────────────────────

/// Connection details for one side of a schema comparison.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SchemaCompareConn {
    #[serde(rename = "type")]
    pub db_type: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// Optional schema filter (blank = all schemas).
    #[serde(default)]
    pub schema: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SchemaCompareProfile {
    pub id: Uuid,
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SchemaCompareProfile {
    /// Redact passwords before returning to API clients.
    pub fn redact_secrets(&mut self) {
        if !self.source.password.is_empty() {
            self.source.password = "********".to_string();
        }
        if !self.target.password.is_empty() {
            self.target.password = "********".to_string();
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SchemaCompareProfileRequest {
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SchemaCompareProfilesResponse {
    pub profiles: Vec<SchemaCompareProfile>,
}

/// Request body for executing a DDL statement against a compare target connection.
#[derive(Debug, Deserialize)]
pub struct ExecDdlRequest {
    pub target: SchemaCompareConn,
    /// Raw DDL statement (e.g. CREATE SCHEMA … or CREATE TABLE …).
    pub statement: String,
}

// ── Data Copy (Schema Compare) ────────────────────────────────────────────────

/// Request body for copying rows from source tables to matching target tables.
#[derive(Debug, Deserialize)]
pub struct CopyDataRequest {
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    /// List of tables to copy.
    pub tables: Vec<CopyTableSpec>,
    /// Rows per INSERT batch (default: 500, max: 5000).
    #[serde(default = "default_copy_batch_size")]
    pub batch_size: usize,
    /// If true, TRUNCATE the target table before copying.
    #[serde(default)]
    pub truncate_before_copy: bool,
    /// If true, extend the per-table timeout to 30 minutes instead of 5.
    /// Use when copying large tables directly instead of via CDC SyncOnce.
    #[serde(default)]
    pub force_direct_copy: bool,
}

fn default_copy_batch_size() -> usize {
    500
}

#[derive(Debug, Deserialize, Clone)]
pub struct CopyTableSpec {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CopyTableResult {
    pub schema: String,
    pub name: String,
    pub rows_copied: i64,
    /// "ok" or "error"
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CopyDataResponse {
    pub tables: Vec<CopyTableResult>,
    pub total_rows_copied: i64,
    pub errors: usize,
}

// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TriggerPreviewResponse {
    pub tables: Vec<String>,
    pub change_log: String,
    pub sql: String,
    pub will_execute: bool,
    pub vendor: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TriggerStatusTable {
    pub schema: String,
    pub name: String,
    pub installed: bool,
}

#[derive(Debug, Serialize)]
pub struct TriggerStatusResponse {
    pub change_log_present: bool,
    pub tables: Vec<TriggerStatusTable>,
    pub timestamp: DateTime<Utc>,
}

// ── Table Data Preview (Schema Compare) ──────────────────────────────────────

/// Request body for previewing live table data from either the source or target
/// connection in a schema comparison session.
#[derive(Debug, Deserialize)]
pub struct TablePreviewRequest {
    pub connection: SchemaCompareConn,
    pub schema: String,
    pub table: String,
    /// Zero-based page number.
    #[serde(default)]
    pub page: u64,
    /// Rows per page (default 50, max 200).
    #[serde(default = "default_preview_page_size")]
    pub page_size: u64,
    /// Optional text search — applied server-side when the driver supports it.
    #[serde(default)]
    pub search: Option<String>,
}

fn default_preview_page_size() -> u64 {
    50
}

/// Per-table count request (single table).
#[derive(Debug, Deserialize)]
pub struct TableCountRequest {
    pub connection: SchemaCompareConn,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Serialize)]
pub struct TableCountResponse {
    pub schema: String,
    pub table: String,
    pub count: i64,
}

/// Batch schema-level counts request — returns counts for every table in a schema
/// using statistics/catalog tables (fast, single round-trip).
#[derive(Debug, Deserialize)]
pub struct SchemaCountsRequest {
    pub connection: SchemaCompareConn,
    pub schema: String,
}

/// counts: map of table_name → row count (-1 = unknown/error).
#[derive(Debug, Serialize)]
pub struct SchemaCountsResponse {
    pub schema: String,
    /// true when values are catalog estimates (may be stale), false when exact.
    pub estimated: bool,
    pub counts: std::collections::HashMap<String, i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TablePreviewResponse {
    /// Ordered column names.
    pub columns: Vec<String>,
    /// Each element is a row; values are ordered to match `columns`.
    pub rows: Vec<Vec<serde_json::Value>>,
    pub page: u64,
    pub page_size: u64,
    /// True when there is at least one more page of data.
    pub has_more: bool,
}

// ── pgcopydb (Postgres → Postgres bulk copy sidecar) ─────────────────────────

/// Mirrors `pgcopydb clone` flags exposed by the sidecar and WebUI.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub struct PgCopyCloneOptions {
    #[serde(default)]
    pub table_jobs: Option<u32>,
    #[serde(default)]
    pub index_jobs: Option<u32>,
    #[serde(default)]
    pub restore_jobs: Option<u32>,
    #[serde(default)]
    pub large_objects_jobs: Option<u32>,
    #[serde(default)]
    pub split_tables_larger_than: Option<u64>,
    #[serde(default)]
    pub split_max_parts: Option<u32>,
    #[serde(default)]
    pub drop_if_exists: bool,
    #[serde(default)]
    pub roles: bool,
    #[serde(default)]
    pub no_role_passwords: bool,
    #[serde(default)]
    pub no_owner: bool,
    #[serde(default)]
    pub no_acl: bool,
    #[serde(default)]
    pub no_comments: bool,
    #[serde(default)]
    pub no_tablespaces: bool,
    #[serde(default)]
    pub skip_large_objects: bool,
    #[serde(default)]
    pub skip_extensions: bool,
    #[serde(default)]
    pub skip_ext_comments: bool,
    #[serde(default)]
    pub skip_collations: bool,
    #[serde(default)]
    pub skip_vacuum: bool,
    #[serde(default)]
    pub skip_analyze: bool,
    #[serde(default)]
    pub skip_db_properties: bool,
    #[serde(default)]
    pub skip_split_by_ctid: bool,
    #[serde(default)]
    pub fail_fast: bool,
    #[serde(default)]
    pub restart: bool,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub not_consistent: bool,
    #[serde(default)]
    pub follow: bool,
    #[serde(default)]
    pub create_slot: bool,
    #[serde(default)]
    pub wal2json_numeric_as_string: bool,
    #[serde(default)]
    pub use_copy_binary: bool,
    #[serde(default)]
    pub all_databases: bool,
    #[serde(default)]
    pub estimate_table_sizes: bool,
    #[serde(default)]
    pub plugin: Option<String>,
    #[serde(default)]
    pub slot_name: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub endpos: Option<String>,
    #[serde(default)]
    pub snapshot: Option<String>,
    #[serde(default)]
    pub requirements: Option<String>,
    #[serde(default)]
    pub filters_file: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PgCopyProfile {
    pub id: Uuid,
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    #[serde(default)]
    pub include_tables: Vec<String>,
    #[serde(default)]
    pub options: PgCopyCloneOptions,
    #[serde(default)]
    pub last_job_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PgCopyProfile {
    pub fn redact_secrets(&mut self) {
        if !self.source.password.is_empty() {
            self.source.password = "********".to_string();
        }
        if !self.target.password.is_empty() {
            self.target.password = "********".to_string();
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PgCopyProfileRequest {
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    #[serde(default)]
    pub include_tables: Vec<String>,
    #[serde(default)]
    pub options: PgCopyCloneOptions,
}

#[derive(Debug, Serialize)]
pub struct PgCopyProfilesResponse {
    pub profiles: Vec<PgCopyProfile>,
}

// ── RedisShake (Redis → Redis sync sidecar) ──────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub struct RedisShakeSyncOptions {
    #[serde(default)]
    pub source_cluster: bool,
    #[serde(default)]
    pub target_cluster: bool,
    #[serde(default)]
    pub source_tls: bool,
    #[serde(default)]
    pub target_tls: bool,
    #[serde(default = "default_true")]
    pub sync_rdb: bool,
    #[serde(default = "default_true")]
    pub sync_aof: bool,
    #[serde(default)]
    pub prefer_replica: bool,
    #[serde(default)]
    pub try_diskless: bool,
    #[serde(default)]
    pub off_reply: bool,
    #[serde(default)]
    pub rdb_restore_command_behavior: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RedisShakeProfile {
    pub id: Uuid,
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    #[serde(default)]
    pub options: RedisShakeSyncOptions,
    #[serde(default)]
    pub last_job_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl RedisShakeProfile {
    pub fn redact_secrets(&mut self) {
        if !self.source.password.is_empty() {
            self.source.password = "********".to_string();
        }
        if !self.target.password.is_empty() {
            self.target.password = "********".to_string();
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RedisShakeProfileRequest {
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    #[serde(default)]
    pub options: RedisShakeSyncOptions,
}

#[derive(Debug, Serialize)]
pub struct RedisShakeProfilesResponse {
    pub profiles: Vec<RedisShakeProfile>,
}

// ── pgloader (MySQL / SQL Server / SQLite / CSV → Postgres) ──────────────────

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub struct PgLoaderOptions {
    #[serde(default)]
    pub drop_if_exists: bool,
    #[serde(default = "default_true")]
    pub create_tables: bool,
    #[serde(default = "default_true")]
    pub create_indexes: bool,
    #[serde(default = "default_true")]
    pub reset_sequences: bool,
    #[serde(default)]
    pub truncate: bool,
    #[serde(default)]
    pub workers: Option<u32>,
    #[serde(default)]
    pub concurrency: Option<u32>,
    #[serde(default)]
    pub target_table: Option<String>,
    #[serde(default)]
    pub csv_delimiter: Option<String>,
    #[serde(default)]
    pub csv_skip_header: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PgLoaderProfile {
    pub id: Uuid,
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    #[serde(default)]
    pub include_tables: Vec<String>,
    #[serde(default)]
    pub options: PgLoaderOptions,
    #[serde(default)]
    pub last_job_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PgLoaderProfile {
    pub fn redact_secrets(&mut self) {
        if !self.source.password.is_empty() {
            self.source.password = "********".to_string();
        }
        if !self.target.password.is_empty() {
            self.target.password = "********".to_string();
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PgLoaderProfileRequest {
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    #[serde(default)]
    pub include_tables: Vec<String>,
    #[serde(default)]
    pub options: PgLoaderOptions,
}

#[derive(Debug, Serialize)]
pub struct PgLoaderProfilesResponse {
    pub profiles: Vec<PgLoaderProfile>,
}

// ── mongocopy (Mongo → Mongo via mongodump/mongorestore) ─────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub struct MongoCopyOptions {
    #[serde(default)]
    pub drop: bool,
    #[serde(default = "default_true")]
    pub gzip: bool,
    #[serde(default)]
    pub auth_source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MongoCopyProfile {
    pub id: Uuid,
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    #[serde(default)]
    pub include_collections: Vec<String>,
    #[serde(default)]
    pub options: MongoCopyOptions,
    #[serde(default)]
    pub last_job_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MongoCopyProfile {
    pub fn redact_secrets(&mut self) {
        if !self.source.password.is_empty() {
            self.source.password = "********".to_string();
        }
        if !self.target.password.is_empty() {
            self.target.password = "********".to_string();
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct MongoCopyProfileRequest {
    pub name: String,
    pub source: SchemaCompareConn,
    pub target: SchemaCompareConn,
    #[serde(default)]
    pub include_collections: Vec<String>,
    #[serde(default)]
    pub options: MongoCopyOptions,
}

#[derive(Debug, Serialize)]
pub struct MongoCopyProfilesResponse {
    pub profiles: Vec<MongoCopyProfile>,
}
