//! Copy & Sync (one-shot migration tool) MCP tools: pgcopy (pgcopydb), pgdump, pgloader,
//! mongocopy, redisshake, rsync. Configure-only is the default: saving a profile never starts a
//! run; `cdc_copy_run` is a separate, explicit write tool. Results are redacted centrally in
//! `main.rs` (`redact::redact`) before they reach the client.

use axum::http::Method;
use serde_json::{json, Map, Value};

use crate::{
    arg_str, guard_write, orch, td, urlencoding_simple, ApiError, AppState, ToolCall, ToolDef,
};

const TOOLS: [&str; 6] = [
    "pgcopy",
    "pgdump",
    "pgloader",
    "mongocopy",
    "redisshake",
    "rsync",
];
/// Placeholder that makes the orchestrator keep a saved password (same login only).
const REDACTED: &str = crate::redact::MASK;

/// Keys of `PgCopyCloneOptions` (orchestrator `src/models.rs`). The orchestrator silently drops
/// unknown keys, so a typo would widen or change a clone without any error — reject it here.
const PGCOPY_OPTION_KEYS: &[&str] = &[
    "table_jobs",
    "index_jobs",
    "restore_jobs",
    "large_objects_jobs",
    "split_tables_larger_than",
    "split_max_parts",
    "drop_if_exists",
    "roles",
    "no_role_passwords",
    "no_owner",
    "no_acl",
    "no_comments",
    "no_tablespaces",
    "skip_large_objects",
    "skip_extensions",
    "skip_ext_comments",
    "skip_collations",
    "skip_vacuum",
    "skip_analyze",
    "skip_db_properties",
    "skip_split_by_ctid",
    "fail_fast",
    "restart",
    "resume",
    "not_consistent",
    "follow",
    "create_slot",
    "wal2json_numeric_as_string",
    "use_copy_binary",
    "all_databases",
    "estimate_table_sizes",
    "plugin",
    "slot_name",
    "origin",
    "endpos",
    "snapshot",
    "requirements",
    "filters_file",
];

/// Prompt-friendly top-level flags copied into `options` (subset of PGCOPY_OPTION_KEYS).
const PGCOPY_FRIENDLY_FLAGS: &[&str] = &[
    "drop_if_exists",
    "no_owner",
    "no_acl",
    "skip_extensions",
    "table_jobs",
    "index_jobs",
];

fn tool_prop() -> Value {
    json!({"type":"string","enum": TOOLS, "description":"Copy & Sync tool id"})
}

pub fn definitions() -> Vec<ToolDef> {
    let conn_side = |side: &str, default_host: &str| {
        json!({
            format!("{side}_host"): {"type":"string","description": format!("{side} host as seen from the sidecar container. Host-machine DBs: host.docker.internal. In-stack: {default_host}.")},
            format!("{side}_port"): {"type":["string","integer"],"default":"5432"},
            format!("{side}_database"): {"type":"string"},
            format!("{side}_username"): {"type":"string"},
            format!("{side}_password"): {"type":"string","description":"Required on create. On update, omit to keep the saved password (same host/port/user only)."}
        })
    };
    let mut create_props = Map::new();
    create_props.insert(
        "name".into(),
        json!({"type":"string","description":"Profile name, e.g. supertokens-copy"}),
    );
    create_props.insert("id".into(), json!({"type":"string","description":"Existing pgcopy profile id to replace (update). Omit to create."}));
    for (k, v) in conn_side("source", "cdc-postgres-source")
        .as_object()
        .unwrap()
    {
        create_props.insert(k.clone(), v.clone());
    }
    for (k, v) in conn_side("target", "cdc-postgres-target")
        .as_object()
        .unwrap()
    {
        create_props.insert(k.clone(), v.clone());
    }
    create_props.insert("include_tables".into(), json!({"type":["string","array"],"description":"schema.table list (CSV or array). Empty = whole database."}));
    create_props.insert("drop_if_exists".into(), json!({"type":"boolean","default":false,"description":"DROP target objects before restore (destructive on run)."}));
    create_props.insert("no_owner".into(), json!({"type":"boolean","default":false,"description":"Skip ALTER OWNER (use when source owner roles do not exist on target)."}));
    create_props.insert(
        "no_acl".into(),
        json!({"type":"boolean","default":false,"description":"Skip GRANT/REVOKE."}),
    );
    create_props.insert(
        "skip_extensions".into(),
        json!({"type":"boolean","default":false}),
    );
    create_props.insert(
        "table_jobs".into(),
        json!({"type":"integer","description":"Parallel table copy jobs"}),
    );
    create_props.insert(
        "index_jobs".into(),
        json!({"type":"integer","description":"Parallel index build jobs"}),
    );
    create_props.insert("options".into(), json!({"type":"object","description":"Advanced pgcopydb options (keys of PgCopyCloneOptions; unknown keys are rejected). See cdc_migration_tool_contract tool_id=pgcopy."}));
    create_props.insert("test_connection".into(), json!({"type":"boolean","default":true,"description":"After saving, test source and target from the pgcopydb sidecar (read-only). Never starts a copy."}));

    vec![
        td(
            "cdc_copy_profile_list",
            "List saved Copy & Sync profiles for a tool (passwords redacted).",
            json!({"required":["tool"],"properties":{"tool": tool_prop()}}),
            true,
        ),
        td(
            "cdc_copy_profile_get",
            "Get one saved Copy & Sync profile (passwords redacted).",
            json!({"required":["tool","id"],"properties":{"tool": tool_prop(),"id":{"type":"string"}}}),
            true,
        ),
        td(
            "cdc_copy_test_connection",
            "Test a database/Redis/SSH connection from the tool's sidecar (where the copy runs) — proves network path, login, server version. Read-only. Pass profile_id and omit password to reuse a saved one.",
            json!({
                "required":["tool"],
                "properties":{
                    "tool": tool_prop(),
                    "side":{"type":"string","enum":["source","target"],"default":"source"},
                    "type":{"type":"string","default":"postgres","description":"postgres, mysql, mssql, sqlite, mongodb, redis, ..."},
                    "host":{"type":"string"},
                    "port":{"type":["string","integer"]},
                    "database":{"type":"string"},
                    "username":{"type":"string"},
                    "password":{"type":"string"},
                    "profile_id":{"type":"string","description":"Saved profile: fills a missing password (same login only)"},
                    "options":{"type":"object"}
                }
            }),
            true,
        ),
        td(
            "cdc_copy_jobs",
            "List runs for a Copy & Sync tool, or one run when job_id is given (status, progress, errors).",
            json!({"required":["tool"],"properties":{"tool": tool_prop(),"job_id":{"type":"string"}}}),
            true,
        ),
        td(
            "cdc_copy_job_logs",
            "Log output of one Copy & Sync run.",
            json!({"required":["tool","job_id"],"properties":{"tool": tool_prop(),"job_id":{"type":"string"}}}),
            true,
        ),
        td(
            "cdc_pg_copy_create",
            "Configure (save) a PostgreSQL → PostgreSQL copy profile (pgcopydb clone) from prompt-friendly fields. Does NOT start the copy — use cdc_copy_run only when the user explicitly asks to run it (write).",
            json!({
                "required":["name","source_host","source_database","source_username","target_host","target_database","target_username"],
                "properties": Value::Object(create_props)
            }),
            false,
        ),
        td(
            "cdc_copy_run",
            "START a copy/sync run from a saved profile (write, can overwrite target data). Only call when the user explicitly asks to start/run the copy.",
            json!({"required":["tool","id"],"properties":{"tool": tool_prop(),"id":{"type":"string","description":"Profile id"}}}),
            false,
        ),
        td(
            "cdc_copy_job_stop",
            "Stop a running Copy & Sync job (write).",
            json!({"required":["tool","job_id"],"properties":{"tool": tool_prop(),"job_id":{"type":"string"}}}),
            false,
        ),
        td(
            "cdc_copy_profile_delete",
            "Delete a saved Copy & Sync profile (write). Does not touch databases or past run files.",
            json!({"required":["tool","id"],"properties":{"tool": tool_prop(),"id":{"type":"string"}}}),
            false,
        ),
    ]
}

/// `Some(result)` when `name` is a copy tool, `None` to let the caller report "unknown tool".
pub async fn call(state: &AppState, call: &ToolCall) -> Option<Result<Value, ApiError>> {
    let args = &call.arguments;
    let out = match call.name.as_str() {
        "cdc_copy_profile_list" => {
            async {
                let tool = tool_arg(args)?;
                orch(state, Method::GET, &format!("/cdc/{tool}/profiles"), None).await
            }
            .await
        }
        "cdc_copy_profile_get" => {
            async {
                let tool = tool_arg(args)?;
                let id = seg(arg_str(args, "id")?);
                orch(
                    state,
                    Method::GET,
                    &format!("/cdc/{tool}/profiles/{id}"),
                    None,
                )
                .await
            }
            .await
        }
        "cdc_copy_test_connection" => {
            async {
                let tool = tool_arg(args)?;
                let body = test_connection_body(args)?;
                orch(
                    state,
                    Method::POST,
                    &format!("/cdc/{tool}/test-connection"),
                    Some(body),
                )
                .await
            }
            .await
        }
        "cdc_copy_jobs" => {
            async {
                let tool = tool_arg(args)?;
                let path = match args
                    .get("job_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    Some(job) => format!("/cdc/{tool}/jobs/{}", seg(job)),
                    None => format!("/cdc/{tool}/jobs"),
                };
                orch(state, Method::GET, &path, None).await
            }
            .await
        }
        "cdc_copy_job_logs" => {
            async {
                let tool = tool_arg(args)?;
                let job = seg(arg_str(args, "job_id")?);
                orch(
                    state,
                    Method::GET,
                    &format!("/cdc/{tool}/jobs/{job}/logs"),
                    None,
                )
                .await
            }
            .await
        }
        "cdc_pg_copy_create" => pg_copy_create(state, args).await,
        "cdc_copy_run" => {
            async {
                guard_write(state)?;
                let tool = tool_arg(args)?;
                let id = seg(arg_str(args, "id")?);
                orch(
                    state,
                    Method::POST,
                    &format!("/cdc/{tool}/profiles/{id}/run"),
                    None,
                )
                .await
            }
            .await
        }
        "cdc_copy_job_stop" => {
            async {
                guard_write(state)?;
                let tool = tool_arg(args)?;
                let job = seg(arg_str(args, "job_id")?);
                orch(
                    state,
                    Method::POST,
                    &format!("/cdc/{tool}/jobs/{job}/stop"),
                    None,
                )
                .await
            }
            .await
        }
        "cdc_copy_profile_delete" => {
            async {
                guard_write(state)?;
                let tool = tool_arg(args)?;
                let id = seg(arg_str(args, "id")?);
                orch(
                    state,
                    Method::DELETE,
                    &format!("/cdc/{tool}/profiles/{id}"),
                    None,
                )
                .await
            }
            .await
        }
        _ => return None,
    };
    Some(out)
}

async fn pg_copy_create(state: &AppState, args: &Value) -> Result<Value, ApiError> {
    guard_write(state)?;
    let update_id = args
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let body = pg_copy_body(args, update_id.is_some())?;
    let saved = match update_id {
        Some(id) => {
            orch(
                state,
                Method::PUT,
                &format!("/cdc/pgcopy/profiles/{}", seg(id)),
                Some(body.clone()),
            )
            .await?
        }
        None => {
            orch(
                state,
                Method::POST,
                "/cdc/pgcopy/profiles",
                Some(body.clone()),
            )
            .await?
        }
    };
    let profile_id = saved
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let mut tests = Value::Null;
    if args
        .get("test_connection")
        .and_then(Value::as_bool)
        .unwrap_or(true)
        && !profile_id.is_empty()
    {
        let mut out = Map::new();
        for side in ["source", "target"] {
            // Redacted password + profile_id: the orchestrator fills the saved one (same login only).
            let mut conn = body[side].clone();
            conn["password"] = json!(REDACTED);
            let req = json!({"connection": conn, "side": side, "profile_id": profile_id});
            let res = orch(
                state,
                Method::POST,
                "/cdc/pgcopy/test-connection",
                Some(req),
            )
            .await
            .unwrap_or_else(|e| json!({"ok": false, "error": e.to_string()}));
            out.insert(side.into(), res);
        }
        tests = Value::Object(out);
    }

    Ok(json!({
        "saved": true,
        "started": false,
        "profile": saved,
        "connection_tests": tests,
        "next_step": format!("Not started. To copy later: cdc_copy_run tool=pgcopy id={profile_id}"),
    }))
}

/// Build the orchestrator `PgCopyProfileRequest` body.
fn pg_copy_body(args: &Value, is_update: bool) -> Result<Value, ApiError> {
    let name = arg_str(args, "name")?.trim().to_string();
    let source = pg_conn(args, "source", is_update)?;
    let target = pg_conn(args, "target", is_update)?;
    if source["host"] == target["host"]
        && source["port"] == target["port"]
        && source["database"] == target["database"]
    {
        return Err(ApiError::Message(
            "source and target are the same database".into(),
        ));
    }

    let mut options = match args.get("options") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(ApiError::Message("options must be an object".into())),
    };
    for key in PGCOPY_FRIENDLY_FLAGS {
        if let Some(v) = args.get(*key).filter(|v| !v.is_null()) {
            options.insert((*key).into(), v.clone());
        }
    }
    let unknown: Vec<&String> = options
        .keys()
        .filter(|k| !PGCOPY_OPTION_KEYS.contains(&k.as_str()))
        .collect();
    if !unknown.is_empty() {
        return Err(ApiError::Message(format!(
            "unknown pgcopy option(s): {unknown:?}; see cdc_migration_tool_contract tool_id=pgcopy"
        )));
    }

    Ok(json!({
        "name": name,
        "source": source,
        "target": target,
        "include_tables": list_arg(args, "include_tables"),
        "options": options,
    }))
}

fn pg_conn(args: &Value, side: &str, is_update: bool) -> Result<Value, ApiError> {
    let get = |k: &str| -> Option<String> {
        match args.get(format!("{side}_{k}")) {
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    };
    let need = |k: &str| get(k).ok_or_else(|| ApiError::Message(format!("missing {side}_{k}")));
    let password = match (
        args.get(format!("{side}_password")).and_then(Value::as_str),
        is_update,
    ) {
        // Passwords are not trimmed: whitespace can be part of a password.
        (Some(p), _) if !p.is_empty() => p.to_string(),
        (_, true) => REDACTED.to_string(),
        (_, false) => return Err(ApiError::Message(format!("missing {side}_password"))),
    };
    Ok(json!({
        "type": "postgres",
        "host": need("host")?,
        "port": get("port").unwrap_or_else(|| "5432".into()),
        "database": need("database")?,
        "username": need("username")?,
        "password": password,
        "schema": "",
    }))
}

fn test_connection_body(args: &Value) -> Result<Value, ApiError> {
    let s = |k: &str| match args.get(k) {
        Some(Value::String(v)) => v.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let side = match s("side").as_str() {
        "" => "source".to_string(),
        "source" | "target" => s("side"),
        other => {
            return Err(ApiError::Message(format!(
                "side must be source or target, got {other}"
            )))
        }
    };
    let profile_id = args
        .get("profile_id")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty());
    let password = match args.get("password").and_then(Value::as_str) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ if profile_id.is_some() => REDACTED.to_string(),
        _ => String::new(),
    };
    let db_type = if s("type").is_empty() {
        "postgres".to_string()
    } else {
        s("type")
    };
    let mut body = json!({
        "connection": {
            "type": db_type, "host": s("host"), "port": s("port"), "database": s("database"),
            "username": s("username"), "password": password, "schema": ""
        },
        "side": side,
    });
    if let Some(id) = profile_id {
        body["profile_id"] = json!(id);
    }
    if let Some(opts @ Value::Object(_)) = args.get("options") {
        body["options"] = opts.clone();
    }
    Ok(body)
}

fn tool_arg(args: &Value) -> Result<&str, ApiError> {
    let tool = arg_str(args, "tool")?;
    if TOOLS.contains(&tool) {
        Ok(tool)
    } else {
        Err(ApiError::Message(format!(
            "unknown copy tool {tool}; expected one of {TOOLS:?}"
        )))
    }
}

fn seg(s: &str) -> String {
    urlencoding_simple(s.trim())
}

fn list_arg(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supertokens_args() -> Value {
        json!({
            "name": "supertokens-copy",
            "source_host": "host.docker.internal", "source_port": 5438,
            "source_database": "supertokens", "source_username": "supertokens", "source_password": "s3cret ",
            "target_host": "cdc-postgres-target",
            "target_database": "supertokens", "target_username": "supertokens", "target_password": "s3cret",
        })
    }

    #[test]
    fn pg_copy_body_maps_prompt_fields() {
        let b = pg_copy_body(&supertokens_args(), false).unwrap();
        assert_eq!(b["source"]["type"], "postgres");
        assert_eq!(b["source"]["port"], "5438");
        assert_eq!(
            b["source"]["password"], "s3cret ",
            "password must not be trimmed"
        );
        assert_eq!(b["target"]["port"], "5432");
        assert_eq!(b["include_tables"], json!([]));
        assert_eq!(b["options"], json!({}));
    }

    #[test]
    fn pg_copy_body_requires_password_on_create_only() {
        let mut a = supertokens_args();
        a.as_object_mut().unwrap().remove("target_password");
        assert!(pg_copy_body(&a, false).is_err());
        assert_eq!(
            pg_copy_body(&a, true).unwrap()["target"]["password"],
            REDACTED
        );
    }

    #[test]
    fn pg_copy_body_rejects_unknown_options_and_same_db() {
        let mut a = supertokens_args();
        a["options"] = json!({"drop_if_exist": true});
        assert!(pg_copy_body(&a, false)
            .unwrap_err()
            .to_string()
            .contains("drop_if_exist"));

        let mut a = supertokens_args();
        a["drop_if_exists"] = json!(true);
        a["include_tables"] = json!("public.users, public.roles");
        let b = pg_copy_body(&a, false).unwrap();
        assert_eq!(b["options"]["drop_if_exists"], true);
        assert_eq!(b["include_tables"], json!(["public.users", "public.roles"]));

        let mut a = supertokens_args();
        a["target_host"] = json!("host.docker.internal");
        a["target_port"] = json!("5438");
        assert!(pg_copy_body(&a, false).is_err());
    }

    #[test]
    fn test_connection_body_reuses_saved_password_via_profile() {
        let b = test_connection_body(
            &json!({"tool":"pgcopy","host":"h","port":5438,"profile_id":"abc"}),
        )
        .unwrap();
        assert_eq!(b["connection"]["password"], REDACTED);
        assert_eq!(b["connection"]["port"], "5438");
        assert_eq!(b["side"], "source");
        assert!(test_connection_body(&json!({"side":"middle"})).is_err());
    }

    #[test]
    fn tool_arg_is_allow_listed() {
        assert!(tool_arg(&json!({"tool":"pgcopy"})).is_ok());
        assert!(tool_arg(&json!({"tool":"../profiles"})).is_err());
    }
}
