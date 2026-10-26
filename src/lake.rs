//! Lakehouse destination contract: validate landing spec and compile Iceberg/S3 sink config.

use std::collections::HashMap;

use crate::error::AppError;
use crate::models::{
    CdcProfile, ChangeMode, LakeCatalogType, LakeDestination, LakeZone, TableFormat,
};

pub fn validate_lake_destination(lake: &LakeDestination) -> Result<(), AppError> {
    if lake.zone == LakeZone::Gold {
        return Err(AppError::BadRequest(
            "CDC lands bronze (raw changelog) or silver (curated). Gold marts are derived via processing pipelines, not capture".to_string(),
        ));
    }
    if lake.table_format == TableFormat::Delta {
        return Err(AppError::BadRequest(
            "Delta Lake sink is reserved; use iceberg for lakehouse tables or parquet for object-storage files".to_string(),
        ));
    }
    match lake.zone {
        LakeZone::Bronze if lake.change_mode != ChangeMode::Changelog => {
            return Err(AppError::BadRequest(
                "bronze zone requires change_mode=changelog (append-only CDC history)".to_string(),
            ));
        }
        LakeZone::Silver if lake.change_mode == ChangeMode::Changelog => {
            return Err(AppError::BadRequest(
                "silver zone requires change_mode=upsert or scd2".to_string(),
            ));
        }
        _ => {}
    }
    req_conn(&lake.connection, "endpoint")?;
    req_conn(&lake.connection, "bucket")?;
    req_conn(&lake.connection, "access_key")?;
    req_conn(&lake.connection, "secret_key")?;
    lake_endpoint_host_port(req_conn(&lake.connection, "endpoint")?)?;
    match lake.catalog_type {
        LakeCatalogType::Rest | LakeCatalogType::Jdbc | LakeCatalogType::Nessie
            if lake.table_format == TableFormat::Iceberg
                && lake
                    .connection
                    .get("catalog_uri")
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true) =>
        {
            return Err(AppError::BadRequest(format!(
                "lake connection missing catalog_uri for {} catalog",
                lake.catalog_type.as_str()
            )));
        }
        _ => {}
    }
    Ok(())
}

pub fn lake_endpoint_host_port(endpoint: &str) -> Result<(String, u16), AppError> {
    let raw = endpoint.trim();
    if raw.is_empty() {
        return Err(AppError::BadRequest(
            "lake connection missing required field: endpoint".to_string(),
        ));
    }
    let stripped = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .unwrap_or(raw);
    let hostport = stripped.split('/').next().unwrap_or(stripped);
    let (host, port) = if let Some((h, p)) = hostport.rsplit_once(':') {
        let port = p
            .parse::<u16>()
            .map_err(|_| AppError::BadRequest(format!("lake endpoint has invalid port: {p}")))?;
        (h.to_string(), port)
    } else {
        let port = if raw.starts_with("https://") {
            443
        } else {
            9000
        };
        (hostport.to_string(), port)
    };
    if host.is_empty() {
        return Err(AppError::BadRequest(
            "lake endpoint is missing a host".to_string(),
        ));
    }
    Ok((host, port))
}

pub fn build_lake_sink_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    let lake = profile.lake.as_ref().ok_or_else(|| {
        AppError::BadRequest("lake destination is required for source_to_lake".to_string())
    })?;
    validate_lake_destination(lake)?;
    match lake.table_format {
        TableFormat::Iceberg => build_iceberg_sink_config(profile, lake, schema_registry_url),
        TableFormat::Parquet => build_s3_parquet_sink_config(profile, lake, schema_registry_url),
        TableFormat::Delta => Err(AppError::BadRequest(
            "Delta Lake sink is reserved; use iceberg or parquet".to_string(),
        )),
    }
}

fn build_iceberg_sink_config(
    profile: &CdcProfile,
    lake: &LakeDestination,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    let endpoint = req_conn(&lake.connection, "endpoint")?;
    let bucket = req_conn(&lake.connection, "bucket")?;
    let access_key = req_conn(&lake.connection, "access_key")?;
    let secret_key = req_conn(&lake.connection, "secret_key")?;
    let region = lake
        .connection
        .get("region")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("us-east-1");
    let prefix = crate::reconcile::topic_prefix_for(profile);
    let namespace = lake.namespace_or_zone();
    let control_topic = format!("_cdc_iceberg_{}", slug(&profile.name));

    let mut cfg = HashMap::new();
    cfg.insert(
        "connector.class".into(),
        "org.apache.iceberg.connect.IcebergSinkConnector".into(),
    );
    cfg.insert("tasks.max".into(), "1".into());
    cfg.insert("topics.regex".into(), topics_regex(profile, &prefix));
    cfg.insert("iceberg.tables.auto-create-enabled".into(), "true".into());
    cfg.insert("iceberg.tables.evolve-schema-enabled".into(), "true".into());
    let tables = iceberg_table_list(profile, &namespace);
    if tables.is_empty() {
        cfg.insert("iceberg.tables.dynamic-enabled".into(), "true".into());
        cfg.insert("iceberg.tables.route-regex".into(), r"^.*\.([^.]+)$".into());
        cfg.insert(
            "iceberg.tables.route-regex-replacement".into(),
            format!("{namespace}.$1"),
        );
    } else {
        cfg.insert("iceberg.tables".into(), tables);
    }
    cfg.insert("iceberg.control.topic".into(), control_topic);
    cfg.insert("iceberg.control.commit.interval-ms".into(), "10000".into());
    cfg.insert("iceberg.catalog".into(), "cdc".into());
    cfg.insert(
        "iceberg.catalog.type".into(),
        lake.catalog_type.as_str().into(),
    );
    cfg.insert("iceberg.catalog.warehouse".into(), lake.warehouse_uri());
    cfg.insert(
        "iceberg.catalog.io-impl".into(),
        "org.apache.iceberg.aws.s3.S3FileIO".into(),
    );
    cfg.insert("iceberg.catalog.s3.endpoint".into(), endpoint.into());
    cfg.insert("iceberg.catalog.s3.path-style-access".into(), "true".into());
    cfg.insert("iceberg.catalog.client.region".into(), region.into());
    cfg.insert("iceberg.catalog.s3.access-key-id".into(), access_key.into());
    cfg.insert(
        "iceberg.catalog.s3.secret-access-key".into(),
        secret_key.into(),
    );
    cfg.insert("iceberg.catalog.s3.bucket".into(), bucket.into());

    match lake.catalog_type {
        LakeCatalogType::Rest | LakeCatalogType::Nessie => {
            cfg.insert(
                "iceberg.catalog.uri".into(),
                req_conn(&lake.connection, "catalog_uri")?.into(),
            );
        }
        LakeCatalogType::Jdbc => {
            cfg.insert(
                "iceberg.catalog.uri".into(),
                req_conn(&lake.connection, "catalog_uri")?.into(),
            );
            if let Some(user) = nonempty(&lake.connection, "catalog_user") {
                cfg.insert("iceberg.catalog.jdbc.user".into(), user.into());
            }
            if let Some(password) = nonempty(&lake.connection, "catalog_password") {
                cfg.insert("iceberg.catalog.jdbc.password".into(), password.into());
            }
        }
        LakeCatalogType::Glue => {
            cfg.insert("iceberg.catalog.warehouse".into(), lake.warehouse_uri());
        }
        LakeCatalogType::Hadoop => {}
    }

    let upsert = matches!(lake.change_mode, ChangeMode::Upsert | ChangeMode::Scd2);
    cfg.insert(
        "iceberg.tables.upsert-mode-enabled".into(),
        if upsert { "true" } else { "false" }.into(),
    );
    if lake.change_mode == ChangeMode::Scd2 {
        cfg.insert("iceberg.tables.cdc-field".into(), "op".into());
    }

    apply_lake_transforms(&mut cfg, lake);
    crate::reconcile::apply_avro_converters(&mut cfg, schema_registry_url);
    Ok(cfg)
}

fn build_s3_parquet_sink_config(
    profile: &CdcProfile,
    lake: &LakeDestination,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    let endpoint = req_conn(&lake.connection, "endpoint")?;
    let bucket = req_conn(&lake.connection, "bucket")?;
    let access_key = req_conn(&lake.connection, "access_key")?;
    let secret_key = req_conn(&lake.connection, "secret_key")?;
    let region = lake
        .connection
        .get("region")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("us-east-1");
    let prefix = crate::reconcile::topic_prefix_for(profile);
    let domain = profile.governance.domain.trim();
    let dataset = profile.governance.dataset_or(&profile.name);
    let topics_dir = if domain.is_empty() {
        format!("{}/{dataset}", lake.zone.as_str())
    } else {
        format!("{}/{domain}/{dataset}", lake.zone.as_str())
    };

    let mut cfg = HashMap::new();
    cfg.insert(
        "connector.class".into(),
        "io.confluent.connect.s3.S3SinkConnector".into(),
    );
    cfg.insert("tasks.max".into(), "1".into());
    cfg.insert("topics.regex".into(), topics_regex(profile, &prefix));
    cfg.insert("s3.bucket.name".into(), bucket.into());
    cfg.insert("s3.region".into(), region.into());
    cfg.insert("store.url".into(), endpoint.into());
    cfg.insert("aws.access.key.id".into(), access_key.into());
    cfg.insert("aws.secret.access.key".into(), secret_key.into());
    cfg.insert(
        "storage.class".into(),
        "io.confluent.connect.s3.storage.S3Storage".into(),
    );
    cfg.insert(
        "format.class".into(),
        "io.confluent.connect.s3.format.parquet.ParquetFormat".into(),
    );
    cfg.insert(
        "partitioner.class".into(),
        "io.confluent.connect.storage.partitioner.TimeBasedPartitioner".into(),
    );
    cfg.insert(
        "path.format".into(),
        "'year'=YYYY/'month'=MM/'day'=dd".into(),
    );
    cfg.insert("locale".into(), "en-US".into());
    cfg.insert("timezone".into(), "UTC".into());
    cfg.insert("flush.size".into(), "1000".into());
    cfg.insert("topics.dir".into(), topics_dir);
    cfg.insert("s3.path.style.access.enabled".into(), "true".into());

    apply_lake_transforms(&mut cfg, lake);
    crate::reconcile::apply_avro_converters(&mut cfg, schema_registry_url);
    Ok(cfg)
}

fn apply_lake_transforms(cfg: &mut HashMap<String, String>, lake: &LakeDestination) {
    if lake.zone == LakeZone::Bronze && lake.change_mode == ChangeMode::Changelog {
        return;
    }
    cfg.insert("transforms".into(), "unwrap".into());
    cfg.insert(
        "transforms.unwrap.type".into(),
        "io.debezium.transforms.ExtractNewRecordState".into(),
    );
    cfg.insert("transforms.unwrap.drop.tombstones".into(), "false".into());
    cfg.insert(
        "transforms.unwrap.delete.handling.mode".into(),
        "rewrite".into(),
    );
}

fn iceberg_table_list(profile: &CdcProfile, namespace: &str) -> String {
    crate::reconcile::source_table_include_list(profile)
        .iter()
        .map(|qualified| {
            let table = qualified.rsplit('.').next().unwrap_or(qualified.as_str());
            format!("{namespace}.{table}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn topics_regex(profile: &CdcProfile, prefix: &str) -> String {
    let tables = crate::reconcile::source_table_include_list(profile);
    if tables.is_empty() {
        format!("{prefix}\\..*")
    } else {
        let parts: Vec<String> = tables.iter().map(|t| regex_escape(t)).collect();
        format!("{prefix}\\.({})", parts.join("|"))
    }
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn regex_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(
            c,
            '.' | '+' | '*' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn req_conn<'a>(fields: &'a HashMap<String, String>, key: &str) -> Result<&'a str, AppError> {
    fields
        .get(key)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::BadRequest(format!("lake connection missing required field: {key}"))
        })
}

fn nonempty<'a>(fields: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    fields.get(key).map(|s| s.trim()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        DeliveryMode, FlowConfig, GovernanceConfig, ProfilePattern, ProfileStatus, ScheduleConfig,
        SchemaSelectionMode, Sensitivity, SourceConfig, TableSelectionMode, TransformStrategy,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn lake_conn() -> HashMap<String, String> {
        HashMap::from([
            ("endpoint".into(), "http://cdc-lake:9000".into()),
            ("bucket".into(), "lake".into()),
            ("region".into(), "us-east-1".into()),
            ("access_key".into(), "cdcadmin".into()),
            ("secret_key".into(), "cdcadmincdcadmin".into()),
            (
                "catalog_uri".into(),
                "jdbc:postgresql://cdc-postgres-target:5432/demo".into(),
            ),
            ("catalog_user".into(), "demo".into()),
            ("catalog_password".into(), "demo".into()),
        ])
    }

    fn lake_profile(zone: LakeZone, mode: ChangeMode, format: TableFormat) -> CdcProfile {
        CdcProfile {
            id: Uuid::new_v4(),
            name: "Commerce Orders".into(),
            description: "lake landing".into(),
            pattern: ProfilePattern::SourceToLake,
            stream_platform: None,
            transform_strategy: TransformStrategy::None,
            source: SourceConfig {
                source_type: "postgres".into(),
                connection: HashMap::from([
                    ("host".into(), "cdc-postgres-source".into()),
                    ("port".into(), "5432".into()),
                    ("database".into(), "demo".into()),
                    ("username".into(), "demo".into()),
                    ("password".into(), "demo".into()),
                ]),
                capture_mode: crate::models::CaptureMode::Logical,
                schema_mode: SchemaSelectionMode::SpecificSchemas,
                table_mode: TableSelectionMode::SpecificTables,
                schemas: vec!["bookings".into()],
                tables: vec!["bookings.tickets".into()],
                changelog_retention_hours: None,
                timestamp_column: None,
                incrementing_column: None,
            },
            target: None,
            lake: Some(LakeDestination {
                name: "analytics-lake".into(),
                table_format: format,
                zone,
                change_mode: mode,
                catalog_type: LakeCatalogType::Jdbc,
                namespace: String::new(),
                warehouse: String::new(),
                connection: lake_conn(),
            }),
            governance: GovernanceConfig {
                domain: "commerce".into(),
                dataset: "orders".into(),
                sensitivity: Sensitivity::Confidential,
                industry_pack: Some("retail".into()),
                pii_fields: vec!["email".into()],
            },
            sink_error_policy: crate::models::SinkErrorPolicy::FailFast,
            flow: FlowConfig {
                transforms: vec![],
                schedule: ScheduleConfig {
                    delivery_mode: DeliveryMode::Realtime,
                    interval_seconds: None,
                    full_refresh_minutes: None,
                },
                mappings: vec![],
            },
            status: ProfileStatus::Draft,
            enabled: true,
            last_error: None,
            history: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            connector_name: None,
            sink_connector_name: None,
        }
    }

    #[test]
    fn bronze_iceberg_keeps_cdc_envelope() {
        let profile = lake_profile(
            LakeZone::Bronze,
            ChangeMode::Changelog,
            TableFormat::Iceberg,
        );
        let cfg = build_lake_sink_config(&profile, "http://cdc-message:8081").unwrap();
        assert_eq!(
            cfg.get("connector.class").unwrap(),
            "org.apache.iceberg.connect.IcebergSinkConnector"
        );
        assert_eq!(cfg.get("iceberg.tables").unwrap(), "bronze.tickets");
        assert_eq!(
            cfg.get("iceberg.tables.upsert-mode-enabled").unwrap(),
            "false"
        );
        assert!(!cfg.contains_key("transforms"));
        assert_eq!(
            cfg.get("iceberg.catalog.warehouse").unwrap(),
            "s3://lake/warehouse"
        );
        assert_eq!(
            cfg.get("topics.regex").unwrap(),
            "commerce_orders\\.(bookings\\.tickets)"
        );
        assert_eq!(
            cfg.get("value.converter").unwrap(),
            "io.confluent.connect.avro.AvroConverter"
        );
    }

    #[test]
    fn silver_upsert_unwraps_debezium_envelope() {
        let profile = lake_profile(LakeZone::Silver, ChangeMode::Upsert, TableFormat::Iceberg);
        let cfg = build_lake_sink_config(&profile, "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("iceberg.tables").unwrap(), "silver.tickets");
        assert_eq!(
            cfg.get("iceberg.tables.upsert-mode-enabled").unwrap(),
            "true"
        );
        assert_eq!(cfg.get("transforms").unwrap(), "unwrap");
        assert!(cfg
            .get("transforms.unwrap.type")
            .unwrap()
            .contains("ExtractNewRecordState"));
    }

    #[test]
    fn parquet_uses_s3_path_convention() {
        let profile = lake_profile(
            LakeZone::Bronze,
            ChangeMode::Changelog,
            TableFormat::Parquet,
        );
        let cfg = build_lake_sink_config(&profile, "http://cdc-message:8081").unwrap();
        assert_eq!(
            cfg.get("connector.class").unwrap(),
            "io.confluent.connect.s3.S3SinkConnector"
        );
        assert_eq!(cfg.get("topics.dir").unwrap(), "bronze/commerce/orders");
        assert_eq!(cfg.get("s3.bucket.name").unwrap(), "lake");
    }

    #[test]
    fn gold_and_delta_are_rejected() {
        let gold = lake_profile(LakeZone::Gold, ChangeMode::Upsert, TableFormat::Iceberg);
        assert!(validate_lake_destination(gold.lake.as_ref().unwrap()).is_err());
        let delta = lake_profile(LakeZone::Bronze, ChangeMode::Changelog, TableFormat::Delta);
        assert!(validate_lake_destination(delta.lake.as_ref().unwrap()).is_err());
    }

    #[test]
    fn parses_minio_endpoint() {
        let (host, port) = lake_endpoint_host_port("http://cdc-lake:9000").unwrap();
        assert_eq!(host, "cdc-lake");
        assert_eq!(port, 9000);
    }
}
