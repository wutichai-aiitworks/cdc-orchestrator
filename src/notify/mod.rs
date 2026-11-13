//! Notification channels and delivery (ADR 0015).
//!
//! The watcher and the audit hook call [`fanout`]; it applies each rule's cooldown, resolves
//! the rule's channels and enqueues one job per channel. A worker sends jobs (8 at a time, 4
//! attempts with backoff) and records every final result. Nothing here blocks the watcher.

pub mod channel;
pub mod render;
pub mod send;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use hyper_cdc_orchestrator::persist::{DeliveryRow, KIND_CHANNEL, KIND_RULE};
use tokio::sync::{mpsc, Mutex, Semaphore};
use uuid::Uuid;

use crate::error::AppError;
use crate::models::AlertRule;
use crate::AppState;
use channel::{ChannelSettings, ChannelType, NotificationChannel};
use render::NotificationEvent;

const QUEUE_CAPACITY: usize = 1000;
const CONCURRENCY: usize = 8;
const BACKOFF: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(10),
    Duration::from_secs(30),
];
const MEMORY_LOG: usize = 500;
/// Trim the rqlite table after this many inserts.
const TRIM_EVERY: u64 = 200;

pub struct Job {
    pub channel: NotificationChannel,
    /// Plain secrets for a legacy rule webhook (not migrated: no `ORCH_SECRET_KEY`).
    pub plain_secrets: Option<BTreeMap<String, String>>,
    pub rule_id: Option<String>,
    pub rule_name: Option<String>,
    pub event: NotificationEvent,
}

/// Connector → when this instance first saw it FAILED, and the rules already fired for it.
type Failing = HashMap<String, (DateTime<Utc>, HashSet<String>)>;

#[derive(Clone)]
pub struct Notifier {
    tx: mpsc::Sender<Job>,
    http: reqwest::Client,
    /// `rule|condition|subject` → last sent.
    cooldowns: Arc<Mutex<HashMap<String, Instant>>>,
    failing: Arc<Mutex<Failing>>,
    memory_log: Arc<Mutex<VecDeque<DeliveryRow>>>,
    inserts: Arc<AtomicU64>,
}

impl Notifier {
    pub fn new() -> (Self, mpsc::Receiver<Job>) {
        let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("hyper-cdc-orchestrator")
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .expect("notification http client");
        (
            Self {
                tx,
                http,
                cooldowns: Arc::default(),
                failing: Arc::default(),
                memory_log: Arc::default(),
                inserts: Arc::default(),
            },
            rx,
        )
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

pub fn spawn_worker(state: AppState, mut rx: mpsc::Receiver<Job>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let permits = Arc::new(Semaphore::new(CONCURRENCY));
        while let Some(job) = rx.recv().await {
            let Ok(permit) = permits.clone().acquire_owned().await else {
                break;
            };
            let state = state.clone();
            tokio::spawn(async move {
                deliver(&state, job).await;
                drop(permit);
            });
        }
    })
}

// ── Delivery ────────────────────────────────────────────────────────────────

struct Attempted {
    ok: bool,
    attempts: u32,
    status: Option<u16>,
    error: Option<String>,
    elapsed: Duration,
}

async fn attempt_all(state: &AppState, job: &Job, max_attempts: u32) -> Attempted {
    let started = Instant::now();
    let secrets = match &job.plain_secrets {
        Some(s) => Ok(s.clone()),
        None => job
            .channel
            .decrypt_secrets(state.config.secret_key.as_ref()),
    };
    let secrets = match secrets {
        Ok(s) => s,
        Err(e) => {
            return Attempted {
                ok: false,
                attempts: 0,
                status: None,
                error: Some(e),
                elapsed: started.elapsed(),
            }
        }
    };
    let mut attempts = 0;
    loop {
        attempts += 1;
        match send::send(&state.notifier.http, &job.channel, &secrets, &job.event).await {
            Ok(status) => {
                return Attempted {
                    ok: true,
                    attempts,
                    status,
                    error: None,
                    elapsed: started.elapsed(),
                }
            }
            Err(e) => {
                if !e.retryable || attempts >= max_attempts {
                    return Attempted {
                        ok: false,
                        attempts,
                        status: e.status,
                        error: Some(e.message),
                        elapsed: started.elapsed(),
                    };
                }
                tokio::time::sleep(BACKOFF[(attempts as usize - 1).min(BACKOFF.len() - 1)]).await;
            }
        }
    }
}

async fn deliver(state: &AppState, job: Job) {
    let result = attempt_all(state, &job, 4).await;
    if !result.ok {
        tracing::warn!(
            channel = %job.channel.name,
            channel_type = job.channel.kind.as_str(),
            rule = ?job.rule_name,
            attempts = result.attempts,
            error = ?result.error,
            "notification delivery failed"
        );
    }
    record(
        state,
        row(
            &job,
            if result.ok { "delivered" } else { "failed" },
            &result,
        ),
    )
    .await;
}

fn row(job: &Job, outcome: &str, r: &Attempted) -> DeliveryRow {
    DeliveryRow {
        delivery_id: Uuid::new_v4().to_string(),
        timestamp: Utc::now().to_rfc3339(),
        channel_id: Some(job.channel.id.clone()).filter(|s| !s.is_empty()),
        channel_name: job.channel.name.clone(),
        channel_type: job.channel.kind.as_str().to_string(),
        rule_id: job.rule_id.clone(),
        rule_name: job.rule_name.clone(),
        condition: job.event.condition.clone(),
        subject: job.event.subject.clone(),
        outcome: outcome.to_string(),
        attempts: r.attempts,
        status_code: r.status,
        error: r.error.clone(),
        duration_ms: r.elapsed.as_millis() as u64,
        test: job.event.test,
    }
}

async fn record(state: &AppState, d: DeliveryRow) {
    if let Some(rq) = &state.rqlite {
        if let Err(e) = rq.insert_delivery(&d).await {
            tracing::warn!(error = %e, "notification delivery log write failed");
        } else if state.notifier.inserts.fetch_add(1, Ordering::Relaxed) % TRIM_EVERY
            == TRIM_EVERY - 1
        {
            let _ = rq.trim_deliveries(state.config.audit_rqlite_max_rows).await;
        }
        return;
    }
    let mut log = state.notifier.memory_log.lock().await;
    log.push_front(d);
    log.truncate(MEMORY_LOG);
}

pub async fn list_deliveries(
    state: &AppState,
    limit: usize,
    channel_id: Option<&str>,
    rule_id: Option<&str>,
    outcome: Option<&str>,
) -> Result<Vec<DeliveryRow>, AppError> {
    if let Some(rq) = &state.rqlite {
        return rq
            .list_deliveries(limit, channel_id, rule_id, outcome)
            .await
            .map_err(|e| AppError::Upstream(e.to_string()));
    }
    let log = state.notifier.memory_log.lock().await;
    Ok(log
        .iter()
        .filter(|d| channel_id.is_none_or(|c| d.channel_id.as_deref() == Some(c)))
        .filter(|d| rule_id.is_none_or(|r| d.rule_id.as_deref() == Some(r)))
        .filter(|d| outcome.is_none_or(|o| d.outcome == o))
        .take(limit.clamp(1, 2000))
        .cloned()
        .collect())
}

/// One attempt with a sample event, for "Send test". Recorded like any delivery.
pub async fn send_test(state: &AppState, channel: NotificationChannel) -> DeliveryRow {
    let job = Job {
        channel,
        plain_secrets: None,
        rule_id: None,
        rule_name: None,
        event: NotificationEvent {
            link: state.config.public_ui_url.clone(),
            ..NotificationEvent::sample()
        },
    };
    let result = attempt_all(state, &job, 1).await;
    let d = row(
        &job,
        if result.ok { "delivered" } else { "failed" },
        &result,
    );
    record(state, d.clone()).await;
    d
}

// ── Fan-out ─────────────────────────────────────────────────────────────────

pub async fn load_channels(
    state: &AppState,
) -> Result<HashMap<String, NotificationChannel>, AppError> {
    let Some(rq) = &state.rqlite else {
        return Ok(HashMap::new());
    };
    Ok(rq
        .list_json::<NotificationChannel>(KIND_CHANNEL)
        .await?
        .into_iter()
        .map(|c| (c.id.clone(), c))
        .collect())
}

/// Sends `event` through every channel of every rule (cooldown applied per rule and subject).
pub async fn fanout(state: &AppState, rules: &[&AlertRule], event: NotificationEvent) {
    if rules.is_empty() {
        return;
    }
    let channels = match load_channels(state).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "notification channels could not be loaded");
            return;
        }
    };
    for rule in rules {
        if rule.cooldown_minutes > 0 {
            let key = format!("{}|{}|{}", rule.id, event.condition, event.subject);
            let mut cd = state.notifier.cooldowns.lock().await;
            let window = Duration::from_secs(u64::from(rule.cooldown_minutes) * 60);
            if cd.get(&key).is_some_and(|at| at.elapsed() < window) {
                tracing::debug!(rule = %rule.name, subject = %event.subject, "notification suppressed by cooldown");
                continue;
            }
            cd.insert(key, Instant::now());
        }
        let mut ev = event.clone();
        ev.rule = Some(rule.name.clone());
        for (channel, plain) in rule_channels(rule, &channels) {
            enqueue(
                state,
                Job {
                    channel,
                    plain_secrets: plain,
                    rule_id: Some(rule.id.clone()),
                    rule_name: Some(rule.name.clone()),
                    event: ev.clone(),
                },
            )
            .await;
        }
    }
}

/// The rule's enabled channels, or its legacy inline webhook when it has none.
fn rule_channels(
    rule: &AlertRule,
    channels: &HashMap<String, NotificationChannel>,
) -> Vec<(NotificationChannel, Option<BTreeMap<String, String>>)> {
    if !rule.channel_ids.is_empty() {
        return rule
            .channel_ids
            .iter()
            .filter_map(|id| channels.get(id))
            .filter(|c| c.enabled)
            .map(|c| (c.clone(), None))
            .collect();
    }
    match rule.webhook_url.as_deref().filter(|u| !u.trim().is_empty()) {
        Some(url) if rule.channels.iter().any(|c| c == "webhook") || rule.channels.is_empty() => {
            let secrets: BTreeMap<String, String> = rule
                .webhook_secret
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| ("signingSecret".to_string(), s.clone()))
                .collect();
            vec![(legacy_webhook(rule, url), Some(secrets))]
        }
        _ => Vec::new(),
    }
}

fn legacy_webhook(rule: &AlertRule, url: &str) -> NotificationChannel {
    NotificationChannel {
        id: migrated_channel_id(&rule.id),
        name: format!("Webhook — {}", rule.name),
        kind: ChannelType::Webhook,
        enabled: true,
        settings: ChannelSettings {
            url: Some(url.trim().to_string()),
            legacy_secret_header: true,
            ..Default::default()
        },
        secrets: BTreeMap::new(),
        created_at: rule.created_at,
        updated_at: None,
        migrated_from_rule: Some(rule.id.clone()),
    }
}

async fn enqueue(state: &AppState, job: Job) {
    if let Err(mpsc::error::TrySendError::Full(job) | mpsc::error::TrySendError::Closed(job)) =
        state.notifier.tx.try_send(job)
    {
        tracing::warn!(channel = %job.channel.name, "notification queue full; dropped");
        let r = Attempted {
            ok: false,
            attempts: 0,
            status: None,
            error: Some("queue full".into()),
            elapsed: Duration::ZERO,
        };
        record(state, row(&job, "dropped", &r)).await;
    }
}

pub fn link(state: &AppState, path: &str) -> Option<String> {
    state
        .config
        .public_ui_url
        .as_ref()
        .map(|base| format!("{base}{path}"))
}

// ── connector_failed_for ────────────────────────────────────────────────────

/// Call every watcher tick with the connectors currently FAILED (and their errors). Fires
/// each `connector_failed_for` rule once per failure, after its minutes.
pub async fn check_failed_for(
    state: &AppState,
    rules: &[AlertRule],
    failed: &[(String, Vec<String>)],
) {
    let now = Utc::now();
    let due: Vec<(String, Vec<String>, i64, Vec<AlertRule>)> = {
        let mut map = state.notifier.failing.lock().await;
        let current: HashSet<&str> = failed.iter().map(|(n, _)| n.as_str()).collect();
        map.retain(|name, _| current.contains(name.as_str()));
        let mut due = Vec::new();
        for (name, errors) in failed {
            let (since, fired) = map
                .entry(name.clone())
                .or_insert_with(|| (now, HashSet::new()));
            let mins = (now - *since).num_minutes();
            let hit: Vec<AlertRule> =
                crate::watcher::rules::rules_for(rules, "connector_failed_for", name)
                    .into_iter()
                    .filter(|r| {
                        !fired.contains(&r.id)
                            && mins >= i64::from(crate::watcher::rules::failed_for_minutes(r))
                    })
                    .cloned()
                    .collect();
            for r in &hit {
                fired.insert(r.id.clone());
            }
            if !hit.is_empty() {
                due.push((name.clone(), errors.clone(), mins, hit));
            }
        }
        due
    };
    for (name, errors, mins, hit) in due {
        let refs: Vec<&AlertRule> = hit.iter().collect();
        let ev = NotificationEvent {
            condition: "connector_failed_for".into(),
            severity: render::Severity::Critical,
            title: format!("Connector still failed: {name}"),
            summary: format!("FAILED for {mins} min"),
            subject: name.clone(),
            subject_kind: "connector".into(),
            from: None,
            to: "FAILED".into(),
            errors,
            ts: now,
            rule: None,
            link: link(state, "/connectors"),
            test: false,
        };
        fanout(state, &refs, ev).await;
    }
}

// ── pipeline_failed (from the audit trail) ──────────────────────────────────

/// Audit hook: a failed operation on a pipeline (other than connector state changes, which
/// the watcher reports) notifies `pipeline_failed` rules.
pub fn on_audit(state: &AppState, e: &crate::audit::AuditEvent) {
    if e.outcome != "failure"
        || e.profile_id.is_none()
        || e.action.starts_with("connector.")
        || state.rqlite.is_none()
    {
        return;
    }
    let state = state.clone();
    let e = e.clone();
    tokio::spawn(async move {
        let Ok(rules) = crate::watcher::rules::load_enabled_rules(&state).await else {
            return;
        };
        let name = e
            .profile_name
            .clone()
            .unwrap_or_else(|| e.profile_id.clone().unwrap_or_default());
        let matched = crate::watcher::rules::rules_for(&rules, "pipeline_failed", &name);
        if matched.is_empty() {
            return;
        }
        let detail: serde_json::Value = serde_json::from_str(&e.detail).unwrap_or_default();
        let error = ["error", "message", "reason"]
            .iter()
            .find_map(|k| detail.get(*k).and_then(|v| v.as_str()))
            .map(str::to_string);
        let ev = NotificationEvent {
            condition: "pipeline_failed".into(),
            severity: render::Severity::Critical,
            title: format!("Pipeline failed: {name}"),
            summary: format!("{} failed", e.action),
            subject: name,
            subject_kind: "pipeline".into(),
            from: None,
            to: "failed".into(),
            errors: error.into_iter().collect(),
            ts: Utc::now(),
            rule: None,
            link: link(&state, "/pipelines"),
            test: false,
        };
        fanout(&state, &matched, ev).await;
    });
}

// ── Startup migration ───────────────────────────────────────────────────────

pub fn migrated_channel_id(rule_id: &str) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("cdc-rule-webhook:{rule_id}").as_bytes(),
    )
    .to_string()
}

/// Moves each pre-ADR-0015 rule webhook into a channel. Idempotent.
pub async fn migrate_legacy_rules(state: &AppState) {
    let Some(rq) = &state.rqlite else { return };
    let rules = match rq.list_json::<AlertRule>(KIND_RULE).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "notification migration: rules could not be read");
            return;
        }
    };
    for mut rule in rules {
        let Some(url) = rule.webhook_url.clone().filter(|u| !u.trim().is_empty()) else {
            continue;
        };
        if !rule.channel_ids.is_empty() {
            continue;
        }
        let secret = rule.webhook_secret.clone().filter(|s| !s.is_empty());
        let mut ch = legacy_webhook(&rule, &url);
        if let Some(sec) = secret {
            let Some(key) = state.config.secret_key.as_ref() else {
                tracing::warn!(rule = %rule.name, "rule webhook has a secret but ORCH_SECRET_KEY is unset; left on the legacy path until a key is set");
                continue;
            };
            ch.secrets.insert("signingSecret".into(), key.encrypt(&sec));
        }
        ch.updated_at = Some(Utc::now());
        if let Err(e) = rq.put_json(KIND_CHANNEL, &ch.id, &ch).await {
            tracing::warn!(rule = %rule.name, error = %e, "notification migration: channel write failed");
            continue;
        }
        rule.channel_ids = vec![ch.id.clone()];
        rule.webhook_url = None;
        rule.webhook_secret = None;
        if let Err(e) = rq.put_json(KIND_RULE, &rule.id, &rule).await {
            tracing::warn!(rule = %rule.name, error = %e, "notification migration: rule write failed");
            continue;
        }
        tracing::info!(rule = %rule.name, channel = %ch.id, "moved rule webhook to a notification channel");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(json: serde_json::Value) -> AlertRule {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn a_legacy_rule_still_delivers_through_its_inline_webhook() {
        let r = rule(serde_json::json!({
            "id": "r1", "name": "Old", "conditionType": "connector_failed", "connectorFilter": "*",
            "channels": ["webhook"], "webhookUrl": "https://hooks.example.com/x", "webhookSecret": "s", "enabled": true
        }));
        let out = rule_channels(&r, &HashMap::new());
        assert_eq!(out.len(), 1);
        let (ch, plain) = &out[0];
        assert!(
            ch.settings.legacy_secret_header,
            "old receivers expect X-Webhook-Secret"
        );
        assert_eq!(plain.as_ref().unwrap()["signingSecret"], "s");
        assert_eq!(
            ch.id,
            migrated_channel_id("r1"),
            "same id the migration will use"
        );
    }

    #[test]
    fn channel_ids_win_and_disabled_or_missing_channels_are_skipped() {
        let mut channels = HashMap::new();
        let mut on = legacy_webhook(
            &rule(
                serde_json::json!({"id":"x","name":"x","conditionType":"c","connectorFilter":"*","channels":[]}),
            ),
            "https://a.example.com",
        );
        on.id = "on".into();
        let mut off = on.clone();
        off.id = "off".into();
        off.enabled = false;
        channels.insert("on".to_string(), on);
        channels.insert("off".to_string(), off);
        let r = rule(serde_json::json!({
            "id": "r", "name": "r", "conditionType": "connector_failed", "connectorFilter": "*", "channels": ["webhook"],
            "channelIds": ["on", "off", "gone"], "webhookUrl": "https://ignored.example.com"
        }));
        let ids: Vec<_> = rule_channels(&r, &channels)
            .into_iter()
            .map(|(c, _)| c.id)
            .collect();
        assert_eq!(ids, vec!["on"]);
    }

    #[test]
    fn migrated_ids_are_stable() {
        assert_eq!(migrated_channel_id("abc"), migrated_channel_id("abc"));
        assert_ne!(migrated_channel_id("abc"), migrated_channel_id("abd"));
    }
}
