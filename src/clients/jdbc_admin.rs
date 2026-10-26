//! HTTP client for the cdc-jdbc-admin Type 4 control-plane service.

use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;
use crate::models::{
    SchemaCountsResponse, SourceTableColumn, SourceTableInspectResponse, TableCountResponse,
    TablePreviewResponse,
};

#[derive(Clone)]
pub struct JdbcAdminClient {
    base: String,
    token: Option<String>,
    http: Client,
}

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
        Ok(Self { base, token, http })
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
                },
            )
            .await?;
        Ok(SourceTableInspectResponse {
            schema: body.schema,
            name: body.name,
            row_count: None,
            primary_key: body.primary_key,
            columns: body.columns,
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
                },
            )
            .await?;
        Ok(body.update_count)
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
