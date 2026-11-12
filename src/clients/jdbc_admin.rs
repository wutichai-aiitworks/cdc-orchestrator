//! HTTP client for the cdc-jdbc-admin Type 4 control-plane service.

use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;
use crate::models::{
    SchemaCountsResponse, SourceTableColumn, SourceTableInspectResponse, TableCountResponse,
    TableIndex, TablePreviewResponse,
};

#[derive(Clone)]
pub struct JdbcAdminClient {
    base: String,
    token: Option<String>,
    http: Client,
    /// For `/v1/stream`: no total-request timeout (a full-table copy can run for many
    /// minutes), only connect and per-read timeouts so a stalled stream still fails.
    stream_http: Client,
}

/// Seconds without any bytes from a `/v1/stream` response before it is treated as stalled.
/// jdbc-admin flushes every 1000 rows, so this only trips on a stuck source query.
pub const STREAM_READ_TIMEOUT_SECS: u64 = 600;

#[derive(Debug, Serialize)]
struct AdminRequest<'a> {
    db_type: &'a str,
    connection: &'a HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    table: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    statement: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    schema_filter: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_size: Option<u64>,
    /// `/v1/stream` only: cap on rows read (None = whole table).
    #[serde(skip_serializing_if = "Option::is_none")]
    max_rows: Option<u64>,
    /// `/v1/stream` only: rows per cursor round trip on the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    fetch_size: Option<u32>,
    /// `/v1/stream`, `/v1/lookup`: columns to read, in order (empty = every column).
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    columns: &'a [String],
    /// `/v1/stream`: canonical, engine-neutral string values (record diff).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    canonical: bool,
    /// `/v1/lookup`: key column names.
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    key_columns: &'a [String],
    /// `/v1/lookup`: key tuples (canonical strings, `key_columns` order).
    #[serde(skip_serializing_if = "<[Vec<Option<String>>]>::is_empty")]
    keys: &'a [Vec<Option<String>>],
}

impl<'a> AdminRequest<'a> {
    fn new(db_type: &'a str, connection: &'a HashMap<String, String>) -> Self {
        Self {
            db_type,
            connection,
            schema: None,
            table: None,
            statement: None,
            search: None,
            schema_filter: Vec::new(),
            page: None,
            page_size: None,
            max_rows: None,
            fetch_size: None,
            columns: &[],
            canonical: false,
            key_columns: &[],
            keys: &[],
        }
    }
}

/// `POST /v1/lookup` result: canonical rows found by key.
#[derive(Debug, Deserialize)]
pub struct LookupRows {
    #[serde(default)]
    #[allow(dead_code)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<Option<String>>>,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TestBody {
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CatalogBody {
    #[serde(default)]
    schemas: Vec<String>,
    #[serde(default)]
    tables: Vec<CatalogTable>,
}

#[derive(Debug, Deserialize)]
struct CatalogTable {
    schema: String,
    name: String,
    #[serde(default)]
    primary_key: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct InspectBody {
    schema: String,
    name: String,
    #[serde(default)]
    primary_key: Vec<String>,
    #[serde(default)]
    columns: Vec<SourceTableColumn>,
    #[serde(default)]
    indexes: Option<Vec<TableIndex>>,
}

#[derive(Debug, Deserialize)]
struct CountBody {
    #[serde(default)]
    schema: String,
    count: Option<i64>,
    #[serde(default)]
    estimated: bool,
    #[serde(default)]
    counts: HashMap<String, i64>,
}

#[derive(Debug, Deserialize)]
pub struct QueryResult {
    #[serde(default)]
    #[allow(dead_code)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
struct DdlBody {
    #[serde(default)]
    update_count: i64,
}

impl JdbcAdminClient {
    pub fn try_new(base: &str, token: Option<String>) -> Result<Self, AppError> {
        let base = base.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err(AppError::Internal("jdbc-admin url is empty".into()));
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AppError::Internal(format!("jdbc-admin http client: {e}")))?;
        let stream_http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(STREAM_READ_TIMEOUT_SECS))
            .build()
            .map_err(|e| AppError::Internal(format!("jdbc-admin stream client: {e}")))?;
        Ok(Self {
            base,
            token,
            http,
            stream_http,
        })
    }

    pub fn from_env() -> Option<Self> {
        let url = std::env::var("ORCH_JDBC_ADMIN_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty())?;
        let token = std::env::var("ORCH_JDBC_ADMIN_TOKEN")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty());
        Self::try_new(&url, token).ok()
    }

    pub async fn test(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
    ) -> Result<String, AppError> {
        let body: TestBody = self
            .post(
                "/v1/test",
                &AdminRequest {
                    db_type,
                    connection,
                    schema: None,
                    table: None,
                    statement: None,
                    search: None,
                    schema_filter: Vec::new(),
                    page: None,
                    page_size: None,
                    max_rows: None,
                    fetch_size: None,
                    ..AdminRequest::new(db_type, connection)
                },
            )
            .await?;
        Ok(body
            .message
            .unwrap_or_else(|| "connection successful via jdbc-admin".into()))
    }

    pub async fn catalog(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        schema_filter: &[String],
    ) -> Result<(Vec<String>, Vec<crate::connector_catalog::LiveTable>), AppError> {
        let body: CatalogBody = self
            .post(
                "/v1/catalog",
                &AdminRequest {
                    db_type,
                    connection,
                    schema: None,
                    table: None,
                    statement: None,
                    search: None,
                    schema_filter: schema_filter.to_vec(),
                    page: None,
                    page_size: None,
                    max_rows: None,
                    fetch_size: None,
                    ..AdminRequest::new(db_type, connection)
                },
            )
            .await?;
        let tables = body
            .tables
            .into_iter()
            .map(|t| crate::connector_catalog::LiveTable {
                schema: t.schema,
                name: t.name,
                row_count: None,
                primary_key: t.primary_key,
            })
            .collect();
        Ok((body.schemas, tables))
    }

    pub async fn inspect(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        schema: &str,
        table: &str,
    ) -> Result<SourceTableInspectResponse, AppError> {
        let body: InspectBody = self
            .post(
                "/v1/inspect",
                &AdminRequest {
                    db_type,
                    connection,
                    schema: Some(schema),
                    table: Some(table),
                    statement: None,
                    search: None,
                    schema_filter: Vec::new(),
                    page: None,
                    page_size: None,
                    max_rows: None,
                    fetch_size: None,
                    ..AdminRequest::new(db_type, connection)
                },
            )
            .await?;
        Ok(SourceTableInspectResponse {
            schema: body.schema,
            name: body.name,
            row_count: None,
            primary_key: body.primary_key,
            columns: body.columns,
            indexes: body.indexes,
            rows: Vec::new(),
            limit: 0,
            timestamp: chrono::Utc::now(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn preview(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        schema: &str,
        table: &str,
        page: u64,
        page_size: u64,
        search: Option<&str>,
    ) -> Result<TablePreviewResponse, AppError> {
        self.post(
            "/v1/preview",
            &AdminRequest {
                db_type,
                connection,
                schema: Some(schema),
                table: Some(table),
                statement: None,
                search,
                schema_filter: Vec::new(),
                page: Some(page),
                page_size: Some(page_size),
                max_rows: None,
                fetch_size: None,
                ..AdminRequest::new(db_type, connection)
            },
        )
        .await
    }

    pub async fn count_schema(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        schema: &str,
    ) -> Result<SchemaCountsResponse, AppError> {
        let body: CountBody = self
            .post(
                "/v1/count",
                &AdminRequest {
                    db_type,
                    connection,
                    schema: Some(schema),
                    table: None,
                    statement: None,
                    search: None,
                    schema_filter: Vec::new(),
                    page: None,
                    page_size: None,
                    max_rows: None,
                    fetch_size: None,
                    ..AdminRequest::new(db_type, connection)
                },
            )
            .await?;
        Ok(SchemaCountsResponse {
            schema: if body.schema.is_empty() {
                schema.to_string()
            } else {
                body.schema
            },
            estimated: body.estimated,
            counts: body.counts,
        })
    }

    pub async fn count_table(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        schema: &str,
        table: &str,
    ) -> Result<TableCountResponse, AppError> {
        let body: CountBody = self
            .post(
                "/v1/count",
                &AdminRequest {
                    db_type,
                    connection,
                    schema: Some(schema),
                    table: Some(table),
                    statement: None,
                    search: None,
                    schema_filter: Vec::new(),
                    page: None,
                    page_size: None,
                    max_rows: None,
                    fetch_size: None,
                    ..AdminRequest::new(db_type, connection)
                },
            )
            .await?;
        Ok(TableCountResponse {
            schema: schema.to_string(),
            table: table.to_string(),
            count: body.count.unwrap_or(0),
        })
    }

    pub async fn query(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        statement: &str,
    ) -> Result<QueryResult, AppError> {
        self.post(
            "/v1/query",
            &AdminRequest {
                db_type,
                connection,
                schema: None,
                table: None,
                statement: Some(statement),
                search: None,
                schema_filter: Vec::new(),
                page: None,
                page_size: None,
                max_rows: None,
                fetch_size: None,
                ..AdminRequest::new(db_type, connection)
            },
        )
        .await
    }

    pub async fn ddl(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        statement: &str,
        schema: Option<&str>,
    ) -> Result<i64, AppError> {
        let body: DdlBody = self
            .post(
                "/v1/ddl",
                &AdminRequest {
                    db_type,
                    connection,
                    schema,
                    table: None,
                    statement: Some(statement),
                    search: None,
                    schema_filter: Vec::new(),
                    page: None,
                    page_size: None,
                    max_rows: None,
                    fetch_size: None,
                    ..AdminRequest::new(db_type, connection)
                },
            )
            .await?;
        Ok(body.update_count)
    }

    /// `POST /v1/stream`: one `SELECT *` over `schema.table`, read as it arrives.
    ///
    /// The source sees a single forward-only query (never the per-page re-reads of
    /// `preview`). Pull rows with [`TableRowStream::next_batch`].
    pub async fn open_stream(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        schema: &str,
        table: &str,
        max_rows: Option<u64>,
    ) -> Result<TableRowStream, AppError> {
        self.open_stream_with(db_type, connection, schema, table, max_rows, &[], false)
            .await
    }

    /// `POST /v1/lookup`: rows whose `key_columns` equal one of `keys`, as canonical
    /// values of `columns` (empty = the key columns). At most 5000 keys per call.
    #[allow(clippy::too_many_arguments)]
    pub async fn lookup(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        schema: &str,
        table: &str,
        key_columns: &[String],
        columns: &[String],
        keys: &[Vec<Option<String>>],
    ) -> Result<LookupRows, AppError> {
        self.post(
            "/v1/lookup",
            &AdminRequest {
                schema: Some(schema),
                table: Some(table),
                columns,
                key_columns,
                keys,
                ..AdminRequest::new(db_type, connection)
            },
        )
        .await
    }

    /// [`Self::open_stream`] reading only `columns` (in order; empty = all), optionally as
    /// canonical strings that compare equal across engines (`/v1/stream` `canonical`).
    #[allow(clippy::too_many_arguments)]
    pub async fn open_stream_with(
        &self,
        db_type: &str,
        connection: &HashMap<String, String>,
        schema: &str,
        table: &str,
        max_rows: Option<u64>,
        columns: &[String],
        canonical: bool,
    ) -> Result<TableRowStream, AppError> {
        let path = "/v1/stream";
        let mut req = self.stream_http.post(format!("{}{path}", self.base));
        if let Some(token) = &self.token {
            req = req
                .header("X-Admin-Token", token)
                .header("Authorization", format!("Bearer {token}"));
        }
        let body = AdminRequest {
            db_type,
            connection,
            schema: Some(schema),
            table: Some(table),
            statement: None,
            search: None,
            schema_filter: Vec::new(),
            page: None,
            page_size: None,
            max_rows,
            fetch_size: None,
            columns,
            canonical,
            ..AdminRequest::new(db_type, connection)
        };
        let res = req
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("jdbc-admin {path}: {e}")))?;
        let status = res.status();
        if !status.is_success() {
            let bytes = res.bytes().await.unwrap_or_default();
            let err = serde_json::from_slice::<ErrorBody>(&bytes)
                .ok()
                .and_then(|b| b.error)
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
            return Err(AppError::BadRequest(format!(
                "jdbc-admin {path} failed [{status}]: {err}"
            )));
        }
        let mut stream = TableRowStream {
            res,
            parser: NdjsonRowParser::default(),
            label: format!("{schema}.{table}"),
        };
        // Line 1 carries the column list; read it before handing the stream out.
        while stream.parser.columns.is_none() && !stream.parser.finished() {
            if !stream.fill().await? {
                break;
            }
        }
        if stream.parser.columns.is_none() {
            return Err(AppError::Upstream(format!(
                "jdbc-admin {path} for {} ended before sending columns",
                stream.label
            )));
        }
        Ok(stream)
    }

    async fn post<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &AdminRequest<'_>,
    ) -> Result<T, AppError> {
        let mut req = self.http.post(format!("{}{path}", self.base));
        if let Some(token) = &self.token {
            req = req
                .header("X-Admin-Token", token)
                .header("Authorization", format!("Bearer {token}"));
        }
        let res = req
            .json(body)
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("jdbc-admin {path}: {e}")))?;
        let status = res.status();
        let bytes = res
            .bytes()
            .await
            .map_err(|e| AppError::Upstream(format!("jdbc-admin {path} body: {e}")))?;
        if !status.is_success() {
            let err = serde_json::from_slice::<ErrorBody>(&bytes)
                .ok()
                .and_then(|b| b.error)
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
            return Err(AppError::BadRequest(format!(
                "jdbc-admin {path} failed [{status}]: {err}"
            )));
        }
        serde_json::from_slice(&bytes)
            .map_err(|e| AppError::Internal(format!("jdbc-admin {path} JSON: {e}")))
    }
}

/// Rows of one `/v1/stream` response, pulled in batches.
pub struct TableRowStream {
    res: reqwest::Response,
    parser: NdjsonRowParser,
    label: String,
}

impl TableRowStream {
    /// Column names in source order (as the source reports them, e.g. Oracle UPPERCASE).
    pub fn columns(&self) -> &[String] {
        self.parser.columns.as_deref().unwrap_or(&[])
    }

    /// Up to `max` rows. An empty batch means the stream finished and its trailer
    /// confirmed every row arrived; a stream that ends any other way is an error.
    pub async fn next_batch(&mut self, max: usize) -> Result<Vec<Vec<Value>>, AppError> {
        loop {
            if self.parser.rows.len() >= max || self.parser.finished() {
                break;
            }
            if !self.fill().await? {
                break;
            }
        }
        if let Some(err) = self.parser.error.take() {
            return Err(AppError::Upstream(format!(
                "source read of {} failed mid-stream: {err}",
                self.label
            )));
        }
        let take = self.parser.rows.len().min(max);
        let batch: Vec<Vec<Value>> = self.parser.rows.drain(..take).collect();
        if batch.is_empty() {
            self.parser.check_complete(&self.label)?;
        }
        Ok(batch)
    }

    /// Read one more chunk into the parser. `false` once the HTTP body is exhausted.
    async fn fill(&mut self) -> Result<bool, AppError> {
        match self.res.chunk().await {
            Ok(Some(bytes)) => {
                self.parser.push(&bytes)?;
                Ok(true)
            }
            Ok(None) => {
                self.parser.end()?;
                Ok(false)
            }
            Err(e) => Err(AppError::Upstream(format!(
                "jdbc-admin stream of {} broke: {e}",
                self.label
            ))),
        }
    }
}

/// Incremental parser for the `/v1/stream` NDJSON body (columns line, row arrays, trailer).
#[derive(Default)]
pub(crate) struct NdjsonRowParser {
    buf: Vec<u8>,
    pub(crate) columns: Option<Vec<String>>,
    pub(crate) rows: std::collections::VecDeque<Vec<Value>>,
    rows_seen: u64,
    /// Row count from the `{"done":true,"rows":N}` trailer.
    done_rows: Option<u64>,
    pub(crate) error: Option<String>,
    ended: bool,
}

impl NdjsonRowParser {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<(), AppError> {
        self.buf.extend_from_slice(bytes);
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            self.line(&line[..line.len() - 1])?;
        }
        Ok(())
    }

    /// The HTTP body ended; flush a final unterminated line.
    pub(crate) fn end(&mut self) -> Result<(), AppError> {
        if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            self.line(&line)?;
        }
        self.ended = true;
        Ok(())
    }

    /// Trailer seen (success or error) or body ended.
    pub(crate) fn finished(&self) -> bool {
        self.done_rows.is_some() || self.error.is_some() || self.ended
    }

    /// Succeeds only when a `done` trailer arrived and its count matches what was parsed.
    pub(crate) fn check_complete(&self, label: &str) -> Result<(), AppError> {
        match self.done_rows {
            Some(n) if n == self.rows_seen => Ok(()),
            Some(n) => Err(AppError::Upstream(format!(
                "stream of {label} reported {n} row(s) but {} arrived",
                self.rows_seen
            ))),
            None => Err(AppError::Upstream(format!(
                "stream of {label} was cut off after {} row(s) (no completion trailer)",
                self.rows_seen
            ))),
        }
    }

    fn line(&mut self, raw: &[u8]) -> Result<(), AppError> {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.iter().all(u8::is_ascii_whitespace) {
            return Ok(());
        }
        let value: Value = serde_json::from_slice(raw)
            .map_err(|e| AppError::Upstream(format!("invalid stream line: {e}")))?;
        match value {
            Value::Array(row) => {
                if self.columns.is_none() {
                    return Err(AppError::Upstream(
                        "stream sent a row before its columns".into(),
                    ));
                }
                self.rows_seen += 1;
                self.rows.push_back(row);
            }
            Value::Object(obj) => {
                if let Some(cols) = obj.get("columns").and_then(Value::as_array) {
                    self.columns = Some(
                        cols.iter()
                            .map(|c| c.as_str().unwrap_or_default().to_string())
                            .collect(),
                    );
                } else if obj.get("done").and_then(Value::as_bool) == Some(true) {
                    self.done_rows = Some(obj.get("rows").and_then(Value::as_u64).unwrap_or(0));
                } else if let Some(err) = obj.get("error") {
                    self.error = Some(
                        err.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| err.to_string()),
                    );
                }
            }
            _ => return Err(AppError::Upstream("unexpected stream line".into())),
        }
        Ok(())
    }
}

#[cfg(test)]
mod stream_tests {
    use super::NdjsonRowParser;

    #[test]
    fn rows_split_across_chunks_are_reassembled_and_trailer_checked() {
        let mut p = NdjsonRowParser::default();
        p.push(b"{\"columns\":[\"ID\",\"NAME\"]}\n[1,\"a\"]\n[2,")
            .unwrap();
        assert_eq!(p.rows.len(), 1);
        p.push(b"\"b\"]\n{\"done\":true,\"rows\":2}\n").unwrap();
        p.end().unwrap();
        assert_eq!(p.columns.as_deref().unwrap(), ["ID", "NAME"]);
        assert_eq!(p.rows.len(), 2);
        assert!(p.check_complete("s.t").is_ok());
    }

    #[test]
    fn a_stream_without_trailer_is_rejected() {
        let mut p = NdjsonRowParser::default();
        p.push(b"{\"columns\":[\"ID\"]}\n[1]\n[2]").unwrap();
        p.end().unwrap();
        let err = p.check_complete("s.t").unwrap_err().to_string_message();
        assert!(err.contains("cut off after 2 row(s)"), "{err}");
    }

    #[test]
    fn a_trailer_count_mismatch_is_rejected() {
        let mut p = NdjsonRowParser::default();
        p.push(b"{\"columns\":[\"ID\"]}\n[1]\n{\"done\":true,\"rows\":5}\n")
            .unwrap();
        assert!(p.check_complete("s.t").is_err());
    }

    #[test]
    fn a_mid_stream_error_is_surfaced() {
        let mut p = NdjsonRowParser::default();
        p.push(b"{\"columns\":[\"ID\"]}\n[1]\n{\"error\":\"ORA-01555: snapshot too old\"}\n")
            .unwrap();
        assert_eq!(p.error.as_deref(), Some("ORA-01555: snapshot too old"));
        assert!(p.finished());
    }
}

#[cfg(test)]
mod tests {
    use super::JdbcAdminClient;

    #[test]
    fn from_env_absent_when_unset() {
        // Safety: tests run single-threaded for this env key in CI unit suite.
        std::env::remove_var("ORCH_JDBC_ADMIN_URL");
        assert!(JdbcAdminClient::from_env().is_none());
    }
}
