//! Pure latency maths (ADR 0016): delay classification, percentiles, 5-minute rollups.

use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use serde::Serialize;

use hyper_cdc_orchestrator::persist::MetricsRollup;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Heartbeat row written on the source and seen on the target (exact).
    Heartbeat,
    /// Trigger capture inbox: `applied_at − changed_at` on the target (exact, DB clocks).
    TriggerInbox,
    /// Kafka timestamps, source offsets and sink commits (upper bound / approximation).
    Estimate,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Heartbeat => "heartbeat",
            Self::TriggerInbox => "trigger_inbox",
            Self::Estimate => "estimate",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    /// Records are waiting longer than the commit granularity.
    Behind,
    /// No new records recently and nothing waiting.
    Idle,
    /// Pipeline not running.
    Stopped,
    /// Not enough data (first samples, unreachable Kafka/target).
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub ts: DateTime<Utc>,
    pub e2e_ms: Option<i64>,
    pub capture_ms: Option<i64>,
    pub sink_ms: Option<i64>,
    pub lag_records: Option<i64>,
    pub throughput_rps: Option<f64>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub method: Method,
    pub status: Status,
    /// Smallest delay the method can tell from zero: the Connect offset-flush interval for
    /// estimates, the 250 ms poll for the heartbeat. `e2e_ms: null` with status ok means
    /// "below this".
    pub resolution_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Age of the oldest record the sink has not committed, as a delay. A backlog younger than
/// the offset-flush interval may already be applied (the sink commits only every flush), so
/// it counts as caught up.
pub fn sink_delay_ms(
    now_ms: i64,
    lag: i64,
    oldest_unconsumed_ms: Option<i64>,
    flush_ms: i64,
) -> Option<i64> {
    if lag <= 0 {
        return Some(0);
    }
    let age = (now_ms - oldest_unconsumed_ms?).max(0);
    Some(if age <= flush_ms { 0 } else { age })
}

/// Debezium offsets carry the source commit time of the last captured change
/// (`ts_usec` Postgres, `ts_sec` MySQL, `ts_ms` others).
pub fn offset_source_ms(offset: &serde_json::Value) -> Option<i64> {
    let o = offset
        .get("offsets")?
        .as_array()?
        .iter()
        .find_map(|e| e.get("offset"))?;
    if let Some(us) = o.get("ts_usec").and_then(serde_json::Value::as_i64) {
        return Some(us / 1000);
    }
    if let Some(s) = o.get("ts_sec").and_then(serde_json::Value::as_i64) {
        return Some(s * 1000);
    }
    o.get("ts_ms").and_then(serde_json::Value::as_i64)
}

/// Capture delay: newest record's Kafka time minus the source commit time in the connector's
/// offset, when both are known. Connect stores source offsets only every flush interval, so
/// the offset trails the newest record by up to that much even when capture is instant: a gap
/// within `flush_ms` counts as 0, a larger one is an upper bound.
pub fn capture_delay_ms(
    newest_kafka_ms: Option<i64>,
    source_commit_ms: Option<i64>,
    flush_ms: i64,
) -> Option<i64> {
    let d = newest_kafka_ms? - source_commit_ms?;
    // A negative gap is clock skew; a huge one means the offset is from another window.
    if !(-5_000..=24 * 3_600_000).contains(&d) {
        return None;
    }
    Some(if d <= flush_ms { 0 } else { d })
}

pub fn classify(
    running: bool,
    sink_ms: Option<i64>,
    lag: Option<i64>,
    last_event_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Status {
    if !running {
        return Status::Stopped;
    }
    match (sink_ms, lag) {
        (Some(s), _) if s > 0 => Status::Behind,
        (Some(_), Some(0)) => match last_event_at {
            Some(t) if now - t < TimeDelta::minutes(5) => Status::Ok,
            _ => Status::Idle,
        },
        (Some(_), _) => Status::Ok,
        _ => Status::Unknown,
    }
}

/// Nearest-rank percentile of sorted values.
pub fn percentile(sorted: &[i64], p: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

pub const BUCKET: TimeDelta = TimeDelta::minutes(5);

pub fn bucket_of(t: DateTime<Utc>) -> DateTime<Utc> {
    t.duration_trunc(BUCKET).unwrap_or(t)
}

pub fn rollup(profile_id: &str, bucket: DateTime<Utc>, samples: &[Sample]) -> MetricsRollup {
    let mut e2e: Vec<i64> = samples.iter().filter_map(|s| s.e2e_ms).collect();
    e2e.sort_unstable();
    let max_of = |f: fn(&Sample) -> Option<i64>| samples.iter().filter_map(f).max();
    let tps: Vec<f64> = samples.iter().filter_map(|s| s.throughput_rps).collect();
    // The most exact method seen in the bucket names it.
    let method = if samples.iter().any(|s| s.method == Method::Heartbeat) {
        Method::Heartbeat
    } else if samples.iter().any(|s| s.method == Method::TriggerInbox) {
        Method::TriggerInbox
    } else {
        Method::Estimate
    };
    MetricsRollup {
        profile_id: profile_id.to_string(),
        timestamp: bucket.to_rfc3339(),
        e2e_p50_ms: percentile(&e2e, 50.0),
        e2e_p95_ms: percentile(&e2e, 95.0),
        e2e_max_ms: e2e.last().copied(),
        capture_max_ms: max_of(|s| s.capture_ms),
        sink_max_ms: max_of(|s| s.sink_ms),
        lag_max: max_of(|s| s.lag_records),
        throughput_avg: (!tps.is_empty()).then(|| tps.iter().sum::<f64>() / tps.len() as f64),
        samples: samples.len() as u32,
        method: method.as_str().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_backlog_younger_than_the_commit_interval_is_caught_up() {
        assert_eq!(sink_delay_ms(100_000, 0, None, 60_000), Some(0));
        assert_eq!(sink_delay_ms(100_000, 5, Some(50_000), 60_000), Some(0));
        assert_eq!(
            sink_delay_ms(200_000, 5, Some(50_000), 60_000),
            Some(150_000)
        );
        assert_eq!(
            sink_delay_ms(200_000, 5, None, 60_000),
            None,
            "backlog but unreadable"
        );
    }

    #[test]
    fn source_commit_time_comes_from_each_dialects_offset_field() {
        let pg = json!({"offsets":[{"partition":{"server":"x"},"offset":{"lsn":1,"ts_usec":1794496443840441i64}}]});
        assert_eq!(offset_source_ms(&pg), Some(1794496443840));
        let my = json!({"offsets":[{"offset":{"file":"b.1","pos":4,"ts_sec":1794496443}}]});
        assert_eq!(offset_source_ms(&my), Some(1794496443000));
        assert_eq!(offset_source_ms(&json!({"offsets":[]})), None);
    }

    #[test]
    fn capture_delay_ignores_skew_and_stale_offsets() {
        assert_eq!(
            capture_delay_ms(Some(30_500), Some(10_000), 60_000),
            Some(0),
            "within the offset flush interval"
        );
        assert_eq!(
            capture_delay_ms(Some(100_000), Some(10_000), 60_000),
            Some(90_000)
        );
        assert_eq!(
            capture_delay_ms(Some(10_000), Some(12_000), 60_000),
            Some(0),
            "small negative = skew"
        );
        assert_eq!(capture_delay_ms(Some(10_000), Some(20_000), 60_000), None);
        assert_eq!(capture_delay_ms(None, Some(1), 60_000), None);
    }

    #[test]
    fn status_reflects_backlog_and_recent_traffic() {
        let now: DateTime<Utc> = "2026-11-12T10:00:00Z".parse().unwrap();
        assert_eq!(
            classify(false, Some(0), Some(0), None, now),
            Status::Stopped
        );
        assert_eq!(
            classify(true, Some(90_000), Some(10), None, now),
            Status::Behind
        );
        assert_eq!(
            classify(
                true,
                Some(0),
                Some(0),
                Some(now - TimeDelta::minutes(1)),
                now
            ),
            Status::Ok
        );
        assert_eq!(
            classify(true, Some(0), Some(0), Some(now - TimeDelta::hours(1)), now),
            Status::Idle
        );
        assert_eq!(classify(true, None, None, None, now), Status::Unknown);
    }

    #[test]
    fn rollup_percentiles_and_best_method() {
        let t: DateTime<Utc> = "2026-11-12T10:03:20Z".parse().unwrap();
        assert_eq!(bucket_of(t).to_rfc3339(), "2026-11-12T10:00:00+00:00");
        let s = |e2e: i64, m: Method| Sample {
            ts: t,
            e2e_ms: Some(e2e),
            capture_ms: Some(1),
            sink_ms: Some(0),
            lag_records: Some(e2e / 100),
            throughput_rps: Some(2.0),
            last_event_at: None,
            method: m,
            status: Status::Ok,
            resolution_ms: None,
            note: None,
        };
        let samples: Vec<Sample> = (1..=20)
            .map(|i| {
                s(
                    i * 100,
                    if i == 20 {
                        Method::Heartbeat
                    } else {
                        Method::Estimate
                    },
                )
            })
            .collect();
        let r = rollup("p", bucket_of(t), &samples);
        assert_eq!(r.e2e_p50_ms, Some(1000));
        assert_eq!(r.e2e_p95_ms, Some(1900));
        assert_eq!(r.e2e_max_ms, Some(2000));
        assert_eq!(r.lag_max, Some(20));
        assert_eq!(r.throughput_avg, Some(2.0));
        assert_eq!(r.method, "heartbeat");
        assert_eq!(percentile(&[], 50.0), None);
    }
}
