use crate::function_proto::{CdcRecord, FunctionError, RecordBatch, RecordHeader};
use crate::function_runtime::executor::{ProcessingPipeline, StageError};
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{Header, Headers, OwnedHeaders, OwnedMessage};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::util::Timeout;
use rdkafka::{ClientConfig, Message, Offset, TopicPartitionList};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct KafkaRuntimeConfig {
    pub bootstrap_servers: String,
    pub producer_timeout: Duration,
}

impl KafkaRuntimeConfig {
    pub fn from_env() -> Result<Self, StageError> {
        let bootstrap_servers = std::env::var("CDC_BOOTSTRAP_SERVERS")
            .map_err(|_| StageError::terminal("CDC_BOOTSTRAP_SERVERS is required"))?;
        Ok(Self {
            bootstrap_servers,
            producer_timeout: Duration::from_secs(30),
        })
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
struct PartitionKey {
    topic: String,
    partition: i32,
}

pub struct KafkaProcessor {
    pipeline: Arc<ProcessingPipeline>,
    consumer: StreamConsumer,
    producer: FutureProducer,
    producer_timeout: Duration,
}

impl KafkaProcessor {
    pub fn new(
        runtime: KafkaRuntimeConfig,
        pipeline: Arc<ProcessingPipeline>,
    ) -> Result<Self, StageError> {
        let consumer: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", &runtime.bootstrap_servers)
            .set("group.id", &pipeline.spec().consumer_group)
            .set("enable.auto.commit", "false")
            .set("enable.auto.offset.store", "false")
            .set("auto.offset.reset", "earliest")
            .set("isolation.level", "read_committed")
            .create()
            .map_err(|error| StageError::terminal(format!("Kafka consumer: {error}")))?;
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", &runtime.bootstrap_servers)
            .set("enable.idempotence", "true")
            .set("acks", "all")
            .set("message.timeout.ms", "30000")
            .create()
            .map_err(|error| StageError::terminal(format!("Kafka producer: {error}")))?;
        Ok(Self {
            pipeline,
            consumer,
            producer,
            producer_timeout: runtime.producer_timeout,
        })
    }

    pub async fn run(&self, cancellation: CancellationToken) -> Result<(), StageError> {
        self.consumer
            .subscribe(&[&self.pipeline.spec().input_topic])
            .map_err(|error| StageError::terminal(format!("Kafka subscribe: {error}")))?;

        let mut buffers: HashMap<PartitionKey, Vec<OwnedMessage>> = HashMap::new();
        let max_wait = Duration::from_millis(self.pipeline.spec().batch.max_wait_ms);
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    self.flush_all(&mut buffers).await?;
                    return Ok(());
                }
                received = tokio::time::timeout(max_wait, self.consumer.recv()) => {
                    match received {
                        Ok(Ok(message)) => {
                            let key = PartitionKey {
                                topic: message.topic().to_string(),
                                partition: message.partition(),
                            };
                            let buffer = buffers.entry(key.clone()).or_default();
                            buffer.push(message.detach());
                            if buffer.len() >= self.pipeline.spec().batch.max_records
                                || batch_bytes(buffer) >= self.pipeline.spec().batch.max_bytes
                            {
                                if let Some(messages) = buffers.remove(&key) {
                                    self.process_partition(key, messages).await?;
                                }
                            }
                        }
                        Ok(Err(error)) => {
                            return Err(StageError::retryable(format!("Kafka receive: {error}")));
                        }
                        Err(_) => self.flush_all(&mut buffers).await?,
                    }
                }
            }
        }
    }

    async fn flush_all(
        &self,
        buffers: &mut HashMap<PartitionKey, Vec<OwnedMessage>>,
    ) -> Result<(), StageError> {
        let drained = buffers.drain().collect::<Vec<_>>();
        for (key, messages) in drained {
            if !messages.is_empty() {
                self.process_partition(key, messages).await?;
            }
        }
        Ok(())
    }

    async fn process_partition(
        &self,
        key: PartitionKey,
        messages: Vec<OwnedMessage>,
    ) -> Result<(), StageError> {
        let last_offset = messages
            .last()
            .map(Message::offset)
            .ok_or_else(|| StageError::terminal("empty Kafka batch"))?;
        let records = messages.iter().map(to_cdc_record).collect::<Vec<_>>();
        let batch = RecordBatch {
            pipeline_id: self.pipeline.spec().id.to_string(),
            stage_id: String::new(),
            invocation_id: Uuid::new_v4().to_string(),
            attempt: 0,
            topic: key.topic.clone(),
            partition: key.partition,
            records,
            attributes: HashMap::from([
                ("first_offset".into(), messages[0].offset().to_string()),
                ("last_offset".into(), last_offset.to_string()),
            ]),
        };
        let outcome = self.pipeline.execute(batch).await?;

        for record in outcome.emitted {
            self.publish(
                &self.pipeline.spec().output_topic,
                key.partition,
                &record,
                None,
            )
            .await?;
        }
        for (record, error) in outcome.dead_letter {
            self.publish(
                &self.pipeline.spec().dead_letter_topic,
                key.partition,
                &record,
                Some(&error),
            )
            .await?;
        }

        let mut offsets = TopicPartitionList::new();
        offsets
            .add_partition_offset(&key.topic, key.partition, Offset::Offset(last_offset + 1))
            .map_err(|error| StageError::terminal(format!("Kafka offset: {error}")))?;
        self.consumer
            .commit(&offsets, CommitMode::Sync)
            .map_err(|error| StageError::retryable(format!("Kafka commit: {error}")))?;
        Ok(())
    }

    async fn publish(
        &self,
        topic: &str,
        partition: i32,
        record: &CdcRecord,
        error: Option<&FunctionError>,
    ) -> Result<(), StageError> {
        let payload = record.value.as_deref().unwrap_or_default();
        let mut headers = OwnedHeaders::new();
        for header in &record.headers {
            headers = headers.insert(Header {
                key: &header.key,
                value: header.value.as_deref(),
            });
        }
        if let Some(error) = error {
            let encoded = serde_json::to_vec(&DlqError::from(error))
                .map_err(|e| StageError::terminal(format!("DLQ error encode: {e}")))?;
            headers = headers.insert(Header {
                key: "hyper-cdc-error",
                value: Some(encoded.as_slice()),
            });
        }
        let mut output = FutureRecord::to(topic)
            .partition(partition)
            .payload(payload)
            .headers(headers);
        if let Some(key) = record.key.as_deref() {
            output = output.key(key);
        }
        self.producer
            .send(output, Timeout::After(self.producer_timeout))
            .await
            .map_err(|(error, _)| StageError::retryable(format!("Kafka publish: {error}")))?;
        Ok(())
    }
}

fn to_cdc_record(message: &OwnedMessage) -> CdcRecord {
    let headers = message
        .headers()
        .map(|headers| {
            headers
                .iter()
                .map(|header| RecordHeader {
                    key: header.key.to_string(),
                    value: header.value.map(ToOwned::to_owned),
                })
                .collect()
        })
        .unwrap_or_default();
    CdcRecord {
        record_id: format!(
            "{}:{}:{}",
            message.topic(),
            message.partition(),
            message.offset()
        ),
        key: message.key().map(ToOwned::to_owned),
        value: message.payload().map(ToOwned::to_owned),
        headers,
        timestamp_ms: message.timestamp().to_millis().unwrap_or_default(),
        offset: message.offset(),
    }
}

fn batch_bytes(messages: &[OwnedMessage]) -> usize {
    messages
        .iter()
        .map(|message| {
            message.payload().map_or(0, <[u8]>::len) + message.key().map_or(0, <[u8]>::len)
        })
        .sum()
}

#[derive(Serialize)]
struct DlqError<'a> {
    code: &'a str,
    message: &'a str,
    retryable: bool,
}

impl<'a> From<&'a FunctionError> for DlqError<'a> {
    fn from(value: &'a FunctionError) -> Self {
        Self {
            code: &value.code,
            message: &value.message,
            retryable: value.retryable,
        }
    }
}
