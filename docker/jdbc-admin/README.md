# cdc-jdbc-admin

Narrow JDBC Type 4 HTTP service for CDC control-plane catalog, connection test,
inspect, preview, counts, and one-shot DDL. Uses `DatabaseMetaData`, not
kafka-connect-jdbc connector APIs and not Hyper DataService.

Internal only (`server_zone`). Orchestrator calls `ORCH_JDBC_ADMIN_URL`.
