//! Kafka reads for the latency sampler: high watermarks and record timestamps.
//! Blocking (rdkafka `BaseConsumer`); call through `tokio::task::spawn_blocking`.
//! The consumer never commits and uses its own group, so it cannot disturb the sink.

use std::collections::HashMap;
use std::time::Duration;

use rdkafka::consumer::{BaseConsumer, Consumer};
use rdkafka::{ClientConfig, Message, Offset, TopicPartitionList};

const TIMEOUT: Duration = Duration::from_secs(3);
/// Record reads per scan, each for the oldest backlog and the newest arrivals.
const MAX_READS: usize = 5;

pub type Tp = (String, i32);

#[derive(Debug, Default, Clone)]
pub struct Scan {
    /// High watermark per partition.
    pub highs: HashMap<Tp, i64>,
    /// Records not yet committed by the sink, summed over partitions.
    pub lag: i64,
    /// Kafka timestamp (ms) of the oldest record the sink has not committed.
    pub oldest_unconsumed_ms: Option<i64>,
    /// Kafka timestamp (ms) of the newest record among partitions that grew since `prev_highs`.
    pub newest_ms: Option<i64>,
}

pub fn consumer(bootstrap: &str) -> Result<BaseConsumer, String> {
    ClientConfig::new()
        .set("bootstrap.servers", bootstrap)
        .set("group.id", "cdc-orchestrator-latency")
        .set("enable.auto.commit", "false")
        .set("enable.partition.eof", "false")
        .set("socket.timeout.ms", "5000")
        .set("session.timeout.ms", "10000")
        .create()
        .map_err(|e| format!("kafka client: {e}"))
}

fn read_ts(c: &BaseConsumer, topic: &str, partition: i32, offset: i64) -> Option<i64> {
    let mut tpl = TopicPartitionList::new();
    tpl.add_partition_offset(topic, partition, Offset::Offset(offset))
        .ok()?;
    c.assign(&tpl).ok()?;
    let deadline = std::time::Instant::now() + TIMEOUT;
    while std::time::Instant::now() < deadline {
        match c.poll(Duration::from_millis(500)) {
            Some(Ok(m)) if m.topic() == topic && m.partition() == partition => {
                return m.timestamp().to_millis();
            }
            Some(_) => continue,
            None => continue,
        }
    }
    None
}

/// Scans `topics`. `committed` holds the sink's committed offset per partition (a missing
/// partition counts from the log start). `prev_highs` is the previous scan's watermarks.
pub fn scan(
    c: &BaseConsumer,
    topics: &[String],
    committed: &HashMap<Tp, i64>,
    prev_highs: &HashMap<Tp, i64>,
) -> Scan {
    let mut out = Scan::default();
    let mut backlog: Vec<(Tp, i64, i64)> = Vec::new(); // (tp, from offset, lag)
    let mut grown: Vec<(Tp, i64, i64)> = Vec::new(); // (tp, high, growth)
    for topic in topics {
        let Ok(md) = c.fetch_metadata(Some(topic), TIMEOUT) else {
            continue;
        };
        let Some(t) = md.topics().iter().find(|t| t.name() == topic) else {
            continue;
        };
        for p in t.partitions() {
            let Ok((low, high)) = c.fetch_watermarks(topic, p.id(), TIMEOUT) else {
                continue;
            };
            let tp = (topic.clone(), p.id());
            out.highs.insert(tp.clone(), high);
            let from = committed.get(&tp).copied().unwrap_or(low).max(low);
            let lag = (high - from).max(0);
            out.lag += lag;
            if lag > 0 {
                backlog.push((tp.clone(), from, lag));
            }
            if let Some(prev) = prev_highs.get(&tp) {
                if high > *prev {
                    grown.push((tp, high, high - prev));
                }
            }
        }
    }
    backlog.sort_by_key(|b| std::cmp::Reverse(b.2));
    out.oldest_unconsumed_ms = backlog
        .iter()
        .take(MAX_READS)
        .filter_map(|((t, p), from, _)| read_ts(c, t, *p, *from))
        .min();
    grown.sort_by_key(|g| std::cmp::Reverse(g.2));
    out.newest_ms = grown
        .iter()
        .take(MAX_READS)
        .filter_map(|((t, p), high, _)| read_ts(c, t, *p, high - 1))
        .max();
    let _ = c.unassign();
    out
}
