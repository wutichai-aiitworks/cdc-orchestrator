use crate::audit::log_action;
use crate::command::run_docker;
use crate::error::AppError;
use crate::models::{
    ActionResponse, CdcProfile, CreateTopicRequest, PagedTopicsResponse, PaginationQuery,
    ProfileDlqStatus, TopicItem,
};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

pub async fn profile_dlq_status(
    State(state): State<AppState>,
    Path(profile_id): Path<Uuid>,
) -> Result<Json<ProfileDlqStatus>, AppError> {
    let profile = state
        .profiles
        .get(profile_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {profile_id}")))?;
    let topics = load_dlq_topics(&state, std::slice::from_ref(&profile)).await?;
    Ok(Json(dlq_status_from_topics(&profile, &topics)))
}

pub async fn load_profile_dlq_statuses(
    state: &AppState,
) -> Result<Vec<ProfileDlqStatus>, AppError> {
    let profiles = state.profiles.list().await?;
    let topics = load_dlq_topics(state, &profiles).await?;
    Ok(profiles
        .iter()
        .map(|profile| dlq_status_from_topics(profile, &topics))
        .collect())
}

async fn load_dlq_topics(
    state: &AppState,
    profiles: &[CdcProfile],
) -> Result<Vec<TopicItem>, AppError> {
    let wanted: std::collections::HashSet<String> = profiles
        .iter()
        .filter_map(CdcProfile::dlq_topic_name)
        .collect();
    if wanted.is_empty() {
        return Ok(vec![]);
    }

    let list_output = rpk(state, &["topic", "list", "-i"]).await?;
    let mut topics: Vec<TopicItem> = parse_topic_list(&list_output)
        .into_iter()
        .filter(|topic| wanted.contains(&topic.name))
        .collect();
    if topics.is_empty() {
        return Ok(topics);
    }

    let mut args = vec![
        "topic".to_string(),
        "describe".to_string(),
        "-p".to_string(),
        "--".to_string(),
    ];
    args.extend(topics.iter().map(|topic| topic.name.clone()));
    if let Ok(raw) = rpk_owned(state, &args).await {
        apply_describe(&mut topics, &raw);
    }
    Ok(topics)
}

fn dlq_status_from_topics(profile: &CdcProfile, topics: &[TopicItem]) -> ProfileDlqStatus {
    let topic = profile.dlq_topic_name();
    let item = topic
        .as_deref()
        .and_then(|name| topics.iter().find(|candidate| candidate.name == name));
    ProfileDlqStatus {
        profile_id: profile.id,
        configured_policy: profile.sink_error_policy,
        effective_policy: profile.effective_sink_error_policy(),
        enabled: topic.is_some(),
        topic,
        exists: item.is_some(),
        messages_count: item.and_then(|value| value.messages_count).unwrap_or(0),
        high_watermark: item.and_then(|value| value.high_watermark).unwrap_or(0),
        size_bytes: item.and_then(|value| value.segment_size),
        timestamp: Utc::now(),
    }
}

pub async fn list_topics_paged(
    State(state): State<AppState>,
    Query(query): Query<PaginationQuery>,
) -> Result<Json<PagedTopicsResponse>, AppError> {
    let mut items = load_topics(&state).await?;
    if let Some(search) = query.search.as_ref().map(|s| s.trim().to_ascii_lowercase()) {
        if !search.is_empty() {
            items.retain(|t| t.name.to_ascii_lowercase().contains(&search));
        }
    }
    if query.hide_internal.unwrap_or(false) {
        items.retain(|t| !t.internal);
    }
    sort_topics(
        &mut items,
        query.sort_by.as_deref().unwrap_or("name"),
        query.sort_order.as_deref().unwrap_or("asc"),
    );

    let total = items.len();
    let (page, page_size, paged_items) = apply_pagination(items, query.page, query.page_size);

    Ok(Json(PagedTopicsResponse {
        items: paged_items,
        total,
        page,
        page_size,
        timestamp: Utc::now(),
    }))
}

pub async fn create_topic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateTopicRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let name = validate_topic_name(&payload.name)?;
    reject_internal(&name)?;

    let partitions = payload.partitions.unwrap_or(1).max(1);
    let replication = payload.replication_factor.unwrap_or(1).max(1);
    let configs = topic_configs_from_request(&payload);

    let mut args = vec![
        "topic".to_string(),
        "create".to_string(),
        name.clone(),
        "-p".to_string(),
        partitions.to_string(),
        "-r".to_string(),
        replication.to_string(),
    ];
    for (key, value) in &configs {
        args.push("-c".to_string());
        args.push(format!("{key}={value}"));
    }

    let output = rpk_owned(&state, &args).await?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "topic.create",
        "success",
        &name,
    );
    Ok(Json(ActionResponse {
        action: "topic.create".to_string(),
        status: "success".to_string(),
        message: if output.is_empty() {
            format!("created topic {name}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn delete_topic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(topic): Path<String>,
) -> Result<Json<ActionResponse>, AppError> {
    let name = validate_topic_name(&topic)?;
    reject_internal(&name)?;
    let output = rpk_owned(
        &state,
        &["topic".into(), "delete".into(), "--".into(), name.clone()],
    )
    .await?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "topic.delete",
        "success",
        &name,
    );
    Ok(Json(ActionResponse {
        action: "topic.delete".to_string(),
        status: "success".to_string(),
        message: if output.is_empty() {
            format!("deleted topic {name}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn recreate_topic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(topic): Path<String>,
) -> Result<Json<ActionResponse>, AppError> {
    let name = validate_topic_name(&topic)?;
    reject_internal(&name)?;

    let snapshot = describe_topic_snapshot(&state, &name).await?;
    rpk_owned(
        &state,
        &["topic".into(), "delete".into(), "--".into(), name.clone()],
    )
    .await?;
    sleep(Duration::from_millis(400)).await;

    let mut args = vec![
        "topic".to_string(),
        "create".to_string(),
        name.clone(),
        "-p".to_string(),
        snapshot.partition_count.max(1).to_string(),
        "-r".to_string(),
        snapshot.replication_factor.max(1).to_string(),
    ];
    for (key, value) in &snapshot.configs {
        args.push("-c".to_string());
        args.push(format!("{key}={value}"));
    }
    let output = rpk_owned(&state, &args).await?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "topic.recreate",
        "success",
        &name,
    );
    Ok(Json(ActionResponse {
        action: "topic.recreate".to_string(),
        status: "success".to_string(),
        message: if output.is_empty() {
            format!("recreated topic {name}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

/// Build `rpk topic trim-prefix` args that discard every record on a topic.
///
/// Flags have to come before `--`, because rpk treats everything after it as positional
/// and trim-prefix accepts only one positional (the topic name). The separator still
/// guards topic names that begin with a dash.
fn trim_prefix_args(name: &str) -> Vec<String> {
    vec![
        "topic".into(),
        "trim-prefix".into(),
        "--offset".into(),
        "end".into(),
        "--no-confirm".into(),
        "--".into(),
        name.to_string(),
    ]
}

pub async fn clear_topic_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(topic): Path<String>,
) -> Result<Json<ActionResponse>, AppError> {
    let name = validate_topic_name(&topic)?;
    reject_internal(&name)?;
    let snapshot = describe_topic_snapshot(&state, &name).await?;
    let policy = snapshot
        .cleanup_policy
        .unwrap_or_else(|| "delete".to_string())
        .to_ascii_lowercase();
    if policy != "delete" {
        return Err(AppError::BadRequest(
            "Clearing messages is only allowed for topics with DELETE policy".to_string(),
        ));
    }
    let output = rpk_owned(&state, &trim_prefix_args(&name)).await?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "topic.clear_messages",
        "success",
        &name,
    );
    Ok(Json(ActionResponse {
        action: "topic.clear_messages".to_string(),
        status: "success".to_string(),
        message: if output.is_empty() {
            format!("cleared messages on topic {name}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn copy_topic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(topic): Path<String>,
    Json(payload): Json<crate::models::CopyTopicRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let source = validate_topic_name(&topic)?;
    let dest = validate_topic_name(&payload.name)?;
    reject_internal(&source)?;
    reject_internal(&dest)?;
    if source == dest {
        return Err(AppError::BadRequest(
            "copy destination must differ from the source topic".into(),
        ));
    }
    let snapshot = describe_topic_snapshot(&state, &source).await?;
    let mut args = vec![
        "topic".to_string(),
        "create".to_string(),
        dest.clone(),
        "-p".to_string(),
        snapshot.partition_count.max(1).to_string(),
        "-r".to_string(),
        snapshot.replication_factor.max(1).to_string(),
    ];
    for (key, value) in &snapshot.configs {
        args.push("-c".to_string());
        args.push(format!("{key}={value}"));
    }
    let output = rpk_owned(&state, &args).await?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "topic.copy",
        "success",
        &format!("{source}->{dest}"),
    );
    Ok(Json(ActionResponse {
        action: "topic.copy".to_string(),
        status: "success".to_string(),
        message: if output.is_empty() {
            format!("copied {source} to {dest}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn update_topic_configs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(topic): Path<String>,
    Json(payload): Json<crate::models::TopicConfigUpdateRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let name = validate_topic_name(&topic)?;
    reject_internal(&name)?;
    if payload.configs.is_empty() {
        return Err(AppError::BadRequest("configs cannot be empty".into()));
    }
    let mut args = vec![
        "topic".to_string(),
        "alter-config".to_string(),
        name.clone(),
    ];
    for (key, value) in &payload.configs {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        args.push("--set".to_string());
        args.push(format!("{key}={value}"));
    }
    if args.len() == 3 {
        return Err(AppError::BadRequest("no valid configs provided".into()));
    }
    let output = rpk_owned(&state, &args).await?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "topic.alter_config",
        "success",
        &name,
    );
    Ok(Json(ActionResponse {
        action: "topic.alter_config".to_string(),
        status: "success".to_string(),
        message: if output.is_empty() {
            format!("updated configs for {name}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub fn parse_topic_list(raw: &str) -> Vec<TopicItem> {
    let mut topics = Vec::new();
    for line in raw.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.is_empty() {
            continue;
        }
        let name = cols[0].to_string();
        if name.eq_ignore_ascii_case("NAME") {
            continue;
        }
        topics.push(TopicItem {
            internal: is_internal_topic(&name),
            partition_count: cols.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            replication_factor: cols.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            out_of_sync_replicas: 0,
            messages_count: None,
            high_watermark: None,
            segment_size: None,
            cleanup_policy: None,
            name,
        });
    }
    topics
}

pub fn is_internal_topic(name: &str) -> bool {
    name.starts_with('_')
}

async fn load_topics(state: &AppState) -> Result<Vec<TopicItem>, AppError> {
    let list_output = rpk(state, &["topic", "list", "-i"]).await?;
    let mut items = parse_topic_list(&list_output);
    if items.is_empty() {
        return Ok(items);
    }

    let names: Vec<String> = items.iter().map(|t| t.name.clone()).collect();
    let mut describe_args = vec![
        "topic".to_string(),
        "describe".to_string(),
        "-s".to_string(),
        "-p".to_string(),
        "-c".to_string(),
        "--".to_string(),
    ];
    describe_args.extend(names.iter().cloned());
    if let Ok(described) = rpk_owned(state, &describe_args).await {
        apply_describe(&mut items, &described);
    }

    let mut storage_tasks = Vec::with_capacity(items.len());
    for name in names {
        let state = state.clone();
        storage_tasks.push(async move {
            let args = [
                "topic".to_string(),
                "describe-storage".to_string(),
                "-z".to_string(),
                "--".to_string(),
                name.clone(),
            ];
            match rpk_owned(&state, &args).await {
                Ok(raw) => (name, parse_storage_total_bytes(&raw)),
                Err(_) => (name, None),
            }
        });
    }
    let sizes = futures_util::future::join_all(storage_tasks).await;
    let size_by_name: HashMap<String, i64> = sizes
        .into_iter()
        .filter_map(|(name, size)| size.map(|v| (name, v)))
        .collect();
    for item in &mut items {
        item.segment_size = size_by_name.get(&item.name).copied();
        if item.internal {
            item.messages_count = None;
        }
    }
    Ok(items)
}

fn apply_describe(items: &mut [TopicItem], raw: &str) {
    let by_name: HashMap<String, TopicDescribe> = parse_topic_describe(raw);
    for item in items {
        if let Some(desc) = by_name.get(&item.name) {
            if desc.partition_count > 0 {
                item.partition_count = desc.partition_count;
            }
            if desc.replication_factor > 0 {
                item.replication_factor = desc.replication_factor;
            }
            item.out_of_sync_replicas = desc.out_of_sync_replicas;
            item.cleanup_policy = desc.cleanup_policy.clone();
            if !item.internal {
                item.messages_count = Some(desc.messages_count);
            }
            item.high_watermark = Some(desc.high_watermark);
        }
    }
}

#[derive(Default)]
struct TopicDescribe {
    partition_count: i32,
    replication_factor: i32,
    out_of_sync_replicas: i32,
    messages_count: i64,
    high_watermark: i64,
    cleanup_policy: Option<String>,
    configs: HashMap<String, String>,
}

fn parse_topic_describe(raw: &str) -> HashMap<String, TopicDescribe> {
    let mut out: HashMap<String, TopicDescribe> = HashMap::new();
    let mut current: Option<String> = None;
    let mut section = "";
    let mut partition_header: Vec<String> = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.chars().all(|c| c == '=' || c == '-') {
            continue;
        }
        let upper = trimmed.to_ascii_uppercase();
        if let Some(next) = describe_section(&upper) {
            section = next;
            continue;
        }

        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if cols.is_empty() {
            continue;
        }
        if note_summary_name(&mut out, &mut current, section, &cols) {
            continue;
        }
        let Some(name) = current.clone() else {
            continue;
        };
        let entry = out.entry(name).or_default();
        apply_describe_line(entry, section, &cols, &mut partition_header);
    }
    out
}

fn describe_section(upper: &str) -> Option<&'static str> {
    match upper {
        "SUMMARY" => Some("summary"),
        "CONFIGS" => Some("configs"),
        "PARTITIONS" => Some("partitions"),
        _ => None,
    }
}

fn note_summary_name(
    out: &mut HashMap<String, TopicDescribe>,
    current: &mut Option<String>,
    section: &str,
    cols: &[&str],
) -> bool {
    if section == "summary" && cols[0].eq_ignore_ascii_case("NAME") && cols.len() >= 2 {
        let name = cols[1..].join(" ");
        *current = Some(name.clone());
        out.entry(name).or_default();
        return true;
    }
    false
}

fn apply_describe_line(
    entry: &mut TopicDescribe,
    section: &str,
    cols: &[&str],
    partition_header: &mut Vec<String>,
) {
    match section {
        "summary" => apply_summary_line(entry, cols),
        "configs" => apply_config_line(entry, cols),
        "partitions" => apply_partition_line(entry, cols, partition_header),
        _ => {}
    }
}

fn apply_summary_line(entry: &mut TopicDescribe, cols: &[&str]) {
    if cols[0].eq_ignore_ascii_case("PARTITIONS") {
        entry.partition_count = cols.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    } else if cols[0].eq_ignore_ascii_case("REPLICAS") {
        entry.replication_factor = cols.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    }
}

fn apply_config_line(entry: &mut TopicDescribe, cols: &[&str]) {
    if cols[0].eq_ignore_ascii_case("KEY") || cols.len() < 2 {
        return;
    }
    let key = cols[0].to_string();
    let value = cols[1].to_string();
    if key == "cleanup.policy" {
        entry.cleanup_policy = Some(value.clone());
    }
    entry.configs.insert(key, value);
}

fn apply_partition_line(
    entry: &mut TopicDescribe,
    cols: &[&str],
    partition_header: &mut Vec<String>,
) {
    if cols[0].eq_ignore_ascii_case("PARTITION") {
        *partition_header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
        return;
    }
    if cols[0].parse::<i32>().is_err() {
        return;
    }
    let replicas = header_value(partition_header, cols, "REPLICAS")
        .map(parse_id_list)
        .unwrap_or_default();
    let isr = header_value(partition_header, cols, "IN-SYNC-REPLICAS")
        .or_else(|| header_value(partition_header, cols, "ISR"))
        .map(parse_id_list)
        .unwrap_or_else(|| replicas.clone());
    let start = header_value(partition_header, cols, "LOG-START-OFFSET")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let hw = header_value(partition_header, cols, "HIGH-WATERMARK")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let oos = replicas.iter().filter(|id| !isr.contains(id)).count() as i32;
    entry.out_of_sync_replicas += oos;
    entry.messages_count += (hw - start).max(0);
    entry.high_watermark += hw.max(0);
}

fn parse_storage_total_bytes(raw: &str) -> Option<i64> {
    let mut total = 0i64;
    let mut seen = false;
    let mut header: Vec<String> = Vec::new();
    for line in raw.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.is_empty() {
            continue;
        }
        if cols.iter().any(|c| c.eq_ignore_ascii_case("TOTAL-BYTES")) {
            header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
            continue;
        }
        if header.is_empty() {
            continue;
        }
        if cols[0].parse::<i32>().is_err() {
            continue;
        }
        if let Some(value) =
            header_value(&header, &cols, "TOTAL-BYTES").and_then(|v| v.parse::<i64>().ok())
        {
            total += value;
            seen = true;
        }
    }
    seen.then_some(total)
}

fn header_value<'a>(header: &[String], cols: &[&'a str], name: &str) -> Option<&'a str> {
    header
        .iter()
        .position(|h| h == name)
        .and_then(|idx| cols.get(idx).copied())
}

fn parse_id_list(raw: &str) -> Vec<i32> {
    raw.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .filter_map(|p| p.trim().parse().ok())
        .collect()
}

struct TopicSnapshot {
    partition_count: i32,
    replication_factor: i32,
    cleanup_policy: Option<String>,
    configs: HashMap<String, String>,
}

async fn describe_topic_snapshot(state: &AppState, name: &str) -> Result<TopicSnapshot, AppError> {
    let raw = rpk_owned(
        state,
        &[
            "topic".into(),
            "describe".into(),
            "-s".into(),
            "-c".into(),
            "--".into(),
            name.to_string(),
        ],
    )
    .await?;
    let parsed = parse_topic_describe(&raw);
    let desc = parsed
        .get(name)
        .ok_or_else(|| AppError::NotFound(format!("topic not found: {name}")))?;
    let mut configs = HashMap::new();
    for key in [
        "cleanup.policy",
        "min.insync.replicas",
        "retention.ms",
        "retention.bytes",
        "max.message.bytes",
    ] {
        if let Some(value) = desc.configs.get(key) {
            configs.insert(key.to_string(), value.clone());
        }
    }
    Ok(TopicSnapshot {
        partition_count: desc.partition_count,
        replication_factor: desc.replication_factor,
        cleanup_policy: desc.cleanup_policy.clone(),
        configs,
    })
}

fn topic_configs_from_request(payload: &CreateTopicRequest) -> Vec<(String, String)> {
    let mut configs: Vec<(String, String)> = Vec::new();
    if let Some(policy) = payload
        .cleanup_policy
        .as_ref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
    {
        configs.push(("cleanup.policy".into(), policy.replace('_', ",")));
    }
    if let Some(min_isr) = payload.min_insync_replicas {
        configs.push(("min.insync.replicas".into(), min_isr.to_string()));
    }
    if let Some(retention_ms) = payload.retention_ms {
        configs.push(("retention.ms".into(), retention_ms.to_string()));
    }
    if let Some(retention_bytes) = payload.retention_bytes.filter(|v| *v >= 0) {
        configs.push(("retention.bytes".into(), retention_bytes.to_string()));
    }
    if let Some(max_bytes) = payload.max_message_bytes.filter(|v| *v > 0) {
        configs.push(("max.message.bytes".into(), max_bytes.to_string()));
    }
    if let Some(custom) = &payload.custom_params {
        for param in custom {
            let key = param.name.trim();
            let value = param.value.trim();
            if key.is_empty() || value.is_empty() {
                continue;
            }
            if configs.iter().any(|(k, _)| k == key) {
                continue;
            }
            configs.push((key.to_string(), value.to_string()));
        }
    }
    configs
}

fn sort_topics(items: &mut [TopicItem], sort_by: &str, sort_order: &str) {
    let desc = sort_order.eq_ignore_ascii_case("desc");
    items.sort_by(|a, b| {
        let ord = match sort_by {
            "partitions" | "partition_count" | "TOTAL_PARTITIONS" => {
                a.partition_count.cmp(&b.partition_count)
            }
            "out_of_sync_replicas" | "OUT_OF_SYNC_REPLICAS" => {
                a.out_of_sync_replicas.cmp(&b.out_of_sync_replicas)
            }
            "replication_factor" | "REPLICATION_FACTOR" => {
                a.replication_factor.cmp(&b.replication_factor)
            }
            "messages_count" | "MESSAGES_COUNT" => a
                .messages_count
                .unwrap_or(-1)
                .cmp(&b.messages_count.unwrap_or(-1)),
            "size" | "segment_size" | "SIZE" => a
                .segment_size
                .unwrap_or(-1)
                .cmp(&b.segment_size.unwrap_or(-1)),
            _ => a
                .name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase()),
        };
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

fn validate_topic_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("topic name is required".to_string()));
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    if !ok || name.len() > 249 {
        return Err(AppError::BadRequest(
            "topic name may only contain letters, numbers, '.', '_', and '-'".to_string(),
        ));
    }
    Ok(name.to_string())
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

fn reject_internal(name: &str) -> Result<(), AppError> {
    if is_internal_topic(name) {
        return Err(AppError::BadRequest(
            "internal topics cannot be modified from this UI".to_string(),
        ));
    }
    Ok(())
}

fn actor_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-user")
        .or_else(|| headers.get("x-actor"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system")
        .to_string()
}

async fn rpk(state: &AppState, args: &[&str]) -> Result<String, AppError> {
    let extra: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    rpk_owned(state, &extra).await
}

async fn rpk_owned(state: &AppState, extra: &[String]) -> Result<String, AppError> {
    let mut args: Vec<String> = vec![
        "exec".to_string(),
        state.config.redpanda_container.clone(),
        "rpk".to_string(),
    ];
    args.extend(extra.iter().cloned());
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_docker(&state.config, &refs).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_prefix_passes_exactly_one_positional() {
        let args = trim_prefix_args("test01.G_BOOKS");
        let sep = args.iter().position(|a| a == "--").expect("missing --");
        // rpk rejects the call with "accepts at most 1 arg(s)" if any flag lands after `--`.
        assert_eq!(&args[sep + 1..], ["test01.G_BOOKS"]);
        assert!(args[..sep].contains(&"--offset".to_string()));
        assert!(args[..sep].contains(&"--no-confirm".to_string()));
    }

    #[test]
    fn trim_prefix_still_guards_dash_leading_names() {
        let args = trim_prefix_args("-weird-topic");
        let sep = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(&args[sep + 1..], ["-weird-topic"]);
    }

    #[test]
    fn parses_topic_list() {
        let raw =
            "NAME      PARTITIONS  REPLICAS\nfoo       3           1\n_bar      1           1\n";
        let topics = parse_topic_list(raw);
        assert_eq!(topics.len(), 2);
        assert_eq!(topics[0].name, "foo");
        assert!(!topics[0].internal);
        assert_eq!(topics[0].partition_count, 3);
        assert!(topics[1].internal);
    }

    #[test]
    fn parses_describe_and_storage() {
        let raw = r#"
SUMMARY
=======
NAME        demo.bookings.airports_data
PARTITIONS  1
REPLICAS    1

CONFIGS
=======
KEY             VALUE   SOURCE
cleanup.policy  delete  DEFAULT_CONFIG

PARTITIONS
==========
PARTITION  LEADER  EPOCH  REPLICAS  LOG-START-OFFSET  HIGH-WATERMARK
0          0       1      [0]       0                 5501
"#;
        let parsed = parse_topic_describe(raw);
        let desc = parsed.get("demo.bookings.airports_data").unwrap();
        assert_eq!(desc.messages_count, 5501);
        assert_eq!(desc.high_watermark, 5501);
        assert_eq!(desc.cleanup_policy.as_deref(), Some("delete"));
        assert_eq!(desc.out_of_sync_replicas, 0);

        let size = parse_storage_total_bytes(
            "PARTITION  CLOUD-BYTES  LOCAL-BYTES  TOTAL-BYTES  CLOUD-SEGMENTS  LOCAL-SEGMENTS\n0          0            23088585     23088585     0               1\n",
        );
        assert_eq!(size, Some(23088585));
    }
}
