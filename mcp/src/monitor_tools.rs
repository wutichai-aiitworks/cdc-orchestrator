//! Production monitoring tools for chat clients (docs/production-rollout-guideline.md §5).
//!
//! Answers the questions an operator asks during and after a rollout, without reading raw JSON:
//! "is the source OK?", "is the replication slot holding WAL?", "is the SSH tunnel up?",
//! "is the Redis real-time sync still running?". Everything here is read-only except
//! `cdc_ssh_tunnel_reconnect`. Secrets never appear in the output: tunnel summaries are built
//! field by field from the (already redacted) orchestrator responses.

use reqwest::Method;
use serde_json::{json, Value};

use crate::ops_tools::resolve_pipeline;
use crate::{
    arg_str, guard_write, orch, td, urlencoding_simple, ApiError, AppState, ToolCall, ToolDef,
};

const PIPELINE_ARG: &str = "Pipeline id (UUID) or name, e.g. \"CollabX Sync\".";
const TUNNEL_ARG: &str = "SSH tunnel id (UUID) or name.";
/// Readiness checks about the replication slot, WAL retention and senders (ADR 0019).
const SLOT_CHECKS: &[&str] = &[
    "pg.wal_level",
    "pg.own_slot",
    "pg.slot_lag",
    "pg.slots",
    "pg.wal_keep",
    "pg.senders",
];

pub fn definitions() -> Vec<ToolDef> {
    vec![
        td(
            "cdc_source_health",
            "One-call health of a pipeline's SOURCE for production monitoring: pipeline status and last error, \
             current latency/backlog, committed log position, replication-slot and WAL-retention checks \
             (slot active, WAL held by the slot, max_slot_wal_keep_size, free senders) and the SSH tunnel state \
             when the source goes through one. Returns `warnings[]` in plain words. Read-only: probes run SELECTs only.",
            json!({"properties":{"pipeline":{"type":"string","description":PIPELINE_ARG}},"required":["pipeline"]}),
            true,
        ),
        td(
            "cdc_source_readiness",
            "Live CDC readiness check of a pipeline's source (ADR 0019): wal_level, slots, WAL senders, REPLICATION \
             login, publication coverage, replica identity, table SELECT (and MySQL/SQL Server/Oracle equivalents). \
             Returns ready, summary counts and only the problems with their platform-specific fix. Read-only.",
            json!({"properties":{
                "pipeline":{"type":"string","description":PIPELINE_ARG},
                "include_fix_script":{"type":"boolean","default":false,"description":"Also return the full DBA fix script."}
            },"required":["pipeline"]}),
            true,
        ),
        td(
            "cdc_ssh_tunnels",
            "SSH tunnels (ADR 0020) with live state: connected or not, last error, the stable forward ports \
             (cdc-sshtunnel:22xxx → database as seen from the SSH server), open connections and which pipelines use \
             each tunnel. Also says whether the gateway is configured/reachable. No secrets are returned.",
            json!({"properties":{"name":{"type":"string","description":"Optional: only tunnels whose name contains this text."}}}),
            true,
        ),
        td(
            "cdc_ssh_tunnel_probe",
            "Dial a database address through a running SSH tunnel (host/port as seen from the SSH server, e.g. \
             127.0.0.1:5432). Answers \"is the path to the database open?\". Read-only diagnostic.",
            json!({"properties":{
                "tunnel":{"type":"string","description":TUNNEL_ARG},
                "host":{"type":"string"},
                "port":{"type":"integer"}
            },"required":["tunnel","host","port"]}),
            true,
        ),
        td(
            "cdc_ssh_tunnel_reconnect",
            "Reconnect an SSH tunnel now instead of waiting for the backoff (up to 30 s). Safe: pipelines resume \
             from their offsets. Write-gated.",
            json!({"properties":{"tunnel":{"type":"string","description":TUNNEL_ARG}},"required":["tunnel"]}),
            false,
        ),
        td(
            "cdc_redis_sync_status",
            "Redis → Redis sync (RedisShake) status per profile: whether it is configured for real time (live AOF \
             stream) or one-time, the latest run's status, start/finish time and error. Flags real-time profiles \
             that are NOT running, with the auto-restart policy (off / night window / any time) and the supervisor \
             state (down, restarts in the last 24 h, next attempt, gave up). A restart is a full snapshot (BGSAVE) on \
             the source. Read-only.",
            json!({"properties":{"name":{"type":"string","description":"Optional: only profiles whose name contains this text."}}}),
            true,
        ),
    ]
}

pub async fn call(state: &AppState, call: &ToolCall) -> Option<Result<Value, ApiError>> {
    let a = &call.arguments;
    let out = match call.name.as_str() {
        "cdc_source_health" => source_health(state, a).await,
        "cdc_source_readiness" => {
            async {
                let id = resolve_pipeline(state, arg_str(a, "pipeline")?).await?;
                let report = readiness(state, &id).await?;
                let full = a
                    .get("include_fix_script")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                Ok(summarize_readiness(&report, full))
            }
            .await
        }
        "cdc_ssh_tunnels" => {
            async {
                let v = orch(state, Method::GET, "/cdc/ssh-tunnels", None).await?;
                let filter = a.get("name").and_then(Value::as_str).map(str::to_lowercase);
                let tunnels: Vec<Value> = v
                    .get("tunnels")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter(|t| match &filter {
                        Some(f) => t
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(f),
                        None => true,
                    })
                    .map(summarize_tunnel)
                    .collect();
                Ok(json!({
                    "configured": v.get("configured"),
                    "gateway_reachable": v.get("gateway_reachable"),
                    "count": tunnels.len(),
                    "tunnels": tunnels,
                }))
            }
            .await
        }
        "cdc_ssh_tunnel_probe" => {
            async {
                let id = resolve_tunnel(state, arg_str(a, "tunnel")?).await?;
                let host = arg_str(a, "host")?;
                let port = a
                    .get("port")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| ApiError::Message("missing integer arg: port".into()))?;
                orch(
                    state,
                    Method::POST,
                    &format!("/cdc/ssh-tunnels/{}/probe", urlencoding_simple(&id)),
                    Some(json!({"host": host, "port": port})),
                )
                .await
            }
            .await
        }
        "cdc_ssh_tunnel_reconnect" => {
            async {
                guard_write(state)?;
                let id = resolve_tunnel(state, arg_str(a, "tunnel")?).await?;
                orch(
                    state,
                    Method::POST,
                    &format!("/cdc/ssh-tunnels/{}/reconnect", urlencoding_simple(&id)),
                    None,
                )
                .await
            }
            .await
        }
        "cdc_redis_sync_status" => redis_sync_status(state, a).await,
        _ => return None,
    };
    Some(out)
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn list<'a>(v: &'a Value, key: &str) -> Vec<&'a Value> {
    v.get(key)
        .and_then(Value::as_array)
        .or_else(|| v.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

async fn resolve_tunnel(state: &AppState, raw: &str) -> Result<String, ApiError> {
    let raw = raw.trim();
    let v = orch(state, Method::GET, "/cdc/ssh-tunnels", None).await?;
    let tunnels = list(&v, "tunnels");
    if let Some(t) = tunnels.iter().find(|t| s(t, "id") == raw) {
        return Ok(s(t, "id").to_string());
    }
    let want = raw.to_lowercase();
    let matches: Vec<_> = tunnels
        .iter()
        .filter(|t| {
            s(t, "name").to_lowercase() == want || s(t, "name").to_lowercase().contains(&want)
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok(s(one, "id").to_string()),
        [] => Err(ApiError::Message(format!(
            "no SSH tunnel named {raw:?}; list them with cdc_ssh_tunnels"
        ))),
        many => Err(ApiError::Message(format!(
            "{} SSH tunnels match {raw:?}: {}; use the exact name or id",
            many.len(),
            many.iter()
                .map(|t| s(t, "name"))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Tunnel view without any secret field (built explicitly, not copied).
fn summarize_tunnel(t: &Value) -> Value {
    let st = t.get("status").cloned().unwrap_or(Value::Null);
    let proxy = t.get("proxy").cloned().unwrap_or(Value::Null);
    let via_cloudflare = s(&proxy, "kind") == "cloudflare_access";
    let fh = t
        .get("forward_host")
        .and_then(Value::as_str)
        .unwrap_or("cdc-sshtunnel");
    let live: Vec<Value> = st
        .get("forwards")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let forwards: Vec<Value> = list(t, "forwards")
        .into_iter()
        .map(|f| {
            let lp = f.get("listen_port").and_then(Value::as_u64).unwrap_or(0);
            let l = live.iter().find(|x| x.get("listen_port").and_then(Value::as_u64) == Some(lp));
            json!({
                "address": format!("{fh}:{lp}"),
                "database": format!("{}:{}", s(f, "remote_host"), f.get("remote_port").and_then(Value::as_u64).unwrap_or(0)),
                "listening": l.and_then(|x| x.get("listening")).and_then(Value::as_bool).unwrap_or(false),
                "open_connections": l.and_then(|x| x.get("active_connections")).cloned().unwrap_or(json!(0)),
            })
        })
        .collect();
    let pinned = !s(t, "host_key_fingerprint").is_empty();
    let state = if !pinned {
        "setup incomplete (host key not pinned)"
    } else if st.is_null() {
        "not on the gateway (gateway down or starting)"
    } else if st.get("connected").and_then(Value::as_bool) == Some(true) {
        "connected"
    } else {
        "not connected"
    };
    json!({
        "id": t.get("id"),
        "name": t.get("name"),
        "state": state,
        "ssh": if via_cloudflare {
            format!("{}@{} (Cloudflare Access)", s(t, "username"), s(&proxy, "hostname"))
        } else {
            format!("{}@{}:{}", s(t, "username"), s(t, "host"), t.get("port").and_then(Value::as_u64).unwrap_or(22))
        },
        "connected_since": st.get("connected_since"),
        "last_error": st.get("last_error"),
        "forwards": forwards,
        "used_by": t.get("used_by").cloned().unwrap_or(json!([])),
    })
}

async fn readiness(state: &AppState, id: &str) -> Result<Value, ApiError> {
    let p = orch(state, Method::GET, &format!("/cdc/profiles/{id}"), None).await?;
    let profile = p.get("profile").unwrap_or(&p);
    let mut body = profile
        .get("source")
        .cloned()
        .ok_or_else(|| ApiError::Message("pipeline has no source".into()))?;
    // The orchestrator restores the stored password and uses this pipeline's slot name.
    body["profile_id"] = json!(id);
    orch(
        state,
        Method::POST,
        "/cdc/profiles/catalog/source/readiness",
        Some(body),
    )
    .await
}

fn summarize_readiness(r: &Value, full: bool) -> Value {
    let problems: Vec<Value> = list(r, "checks")
        .into_iter()
        .filter(|c| s(c, "status") != "pass")
        .map(|c| json!({"id": c.get("id"), "status": c.get("status"), "title": c.get("title"), "detail": c.get("detail"), "fix": c.get("fix")}))
        .collect();
    let mut out = json!({
        "ready": r.get("ready"),
        "applicable": r.get("applicable"),
        "engine": r.get("engine"),
        "platform": r.get("platform"),
        "server": r.get("server").and_then(|x| x.get("version")),
        "summary": r.get("summary"),
        "problems": problems,
        "note": r.get("note"),
    });
    if full {
        out["fix_script"] = r.get("fix_script").cloned().unwrap_or(Value::Null);
    }
    out
}

async fn source_health(state: &AppState, a: &Value) -> Result<Value, ApiError> {
    let id = resolve_pipeline(state, arg_str(a, "pipeline")?).await?;
    let p = orch(state, Method::GET, &format!("/cdc/profiles/{id}"), None).await?;
    let profile = p.get("profile").unwrap_or(&p).clone();
    let mut warnings: Vec<String> = Vec::new();

    let status = s(&profile, "status").to_string();
    warnings.extend(status_warning(&status));
    let latency = latency_of(state, &id, &mut warnings).await;
    // Committed position (log-based only)
    let position = orch(
        state,
        Method::GET,
        &format!("/cdc/profiles/{id}/position"),
        None,
    )
    .await
    .ok();
    let capture = profile
        .get("source")
        .map(|x| s(x, "capture_mode"))
        .unwrap_or("");
    let slot = if capture.is_empty() || capture == "logical" {
        slot_checks(state, &id, &mut warnings).await
    } else {
        Value::Null
    };
    let tunnel = tunnel_of(state, &profile, &mut warnings).await;

    Ok(json!({
        "pipeline": profile.get("name"),
        "id": id,
        "status": status,
        "last_error": profile.get("last_error"),
        "capture_mode": capture,
        "latency": latency,
        "position": position,
        "slot_and_wal": slot,
        "ssh_tunnel": tunnel,
        "warnings": warnings,
        "healthy": warnings.is_empty(),
    }))
}

fn status_warning(status: &str) -> Option<String> {
    match status {
        "failed" => Some("Pipeline is FAILED: see last_error and cdc_pipeline_logs.".into()),
        "stopped" | "paused" => Some(format!(
            "Pipeline is {status}: changes are not being synced."
        )),
        "draft" => Some("Pipeline is a draft: it has never been started.".into()),
        _ => None,
    }
}

/// Current latency/backlog of the pipeline; warns when it is behind.
async fn latency_of(state: &AppState, id: &str, warnings: &mut Vec<String>) -> Option<Value> {
    let v = orch(state, Method::GET, "/cdc/latency", None).await.ok()?;
    let item = list(&v, "items")
        .into_iter()
        .find(|i| s(i, "profile_id") == id)
        .cloned()?;
    if s(&item, "status") == "behind" {
        warnings.push(
            "Pipeline is behind (backlog growing): check the target and the connector.".into(),
        );
    }
    Some(item)
}

/// Replication slot / WAL checks from the read-only readiness probes.
async fn slot_checks(state: &AppState, id: &str, warnings: &mut Vec<String>) -> Value {
    let r = match readiness(state, id).await {
        Ok(r) => r,
        Err(e) => {
            warnings.push(format!("Could not run the slot/WAL checks: {e}"));
            return Value::Null;
        }
    };
    let checks: Vec<Value> = list(&r, "checks")
        .into_iter()
        .filter(|c| SLOT_CHECKS.contains(&s(c, "id")))
        .map(|c| json!({"id": c.get("id"), "status": c.get("status"), "detail": c.get("detail"), "value": c.get("value")}))
        .collect();
    warnings.extend(checks.iter().filter_map(check_warning));
    json!({"ready": r.get("ready"), "checks": checks})
}

fn check_warning(c: &Value) -> Option<String> {
    match s(c, "status") {
        "fail" => Some(format!("{}: {}", s(c, "id"), s(c, "detail"))),
        "warn" => Some(format!("{} (warning): {}", s(c, "id"), s(c, "detail"))),
        _ => None,
    }
}

/// The SSH tunnel the source goes through, if any; warns when it is not connected or missing.
async fn tunnel_of(state: &AppState, profile: &Value, warnings: &mut Vec<String>) -> Value {
    let tunnel_id = profile
        .get("source")
        .and_then(|x| x.get("connection"))
        .map(|c| s(c, "ssh_tunnel_id").to_string())
        .unwrap_or_default();
    if tunnel_id.is_empty() {
        return Value::Null;
    }
    let Ok(v) = orch(state, Method::GET, "/cdc/ssh-tunnels", None).await else {
        return Value::Null;
    };
    let Some(t) = list(&v, "tunnels")
        .into_iter()
        .find(|t| s(t, "id") == tunnel_id)
    else {
        warnings.push("The pipeline points at an SSH tunnel that no longer exists.".into());
        return Value::Null;
    };
    let tunnel = summarize_tunnel(t);
    if s(&tunnel, "state") != "connected" {
        warnings.push(format!(
            "SSH tunnel {} is {}.",
            s(&tunnel, "name"),
            s(&tunnel, "state")
        ));
    }
    tunnel
}

async fn redis_sync_status(state: &AppState, a: &Value) -> Result<Value, ApiError> {
    let profiles = orch(state, Method::GET, "/cdc/redisshake/profiles", None).await?;
    let jobs = orch(state, Method::GET, "/cdc/redisshake/jobs", None).await?;
    let jobs = list(&jobs, "jobs");
    let filter = a.get("name").and_then(Value::as_str).map(str::to_lowercase);
    let mut warnings: Vec<String> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for p in list(&profiles, "profiles") {
        if filter
            .as_ref()
            .is_some_and(|f| !s(p, "name").to_lowercase().contains(f))
        {
            continue;
        }
        let (view, warning) = redis_profile_view(p, &jobs);
        warnings.extend(warning);
        out.push(view);
    }
    Ok(json!({"profiles": out, "warnings": warnings, "healthy": warnings.is_empty()}))
}

fn restart_policy(opts: &Value) -> String {
    match s(opts, "auto_restart") {
        "always" => "any time".to_string(),
        "window" => {
            let w = s(opts, "restart_window");
            format!(
                "night window {}",
                if w.is_empty() { "00:00-05:00" } else { w }
            )
        }
        _ => "off".to_string(),
    }
}

fn host_port(side: Option<&Value>) -> String {
    side.map(|x| format!("{}:{}", s(x, "host"), s(x, "port")))
        .unwrap_or_default()
}

/// One RedisShake profile's status, plus a warning when a real-time sync is not running.
fn redis_profile_view(p: &Value, jobs: &[&Value]) -> (Value, Option<String>) {
    let name = s(p, "name");
    let pid = s(p, "id");
    let opts = p.get("options").cloned().unwrap_or(Value::Null);
    let realtime = opts
        .get("sync_aof")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let latest = jobs
        .iter()
        .filter(|j| s(j, "profile_id") == pid)
        .max_by(|x, y| s(x, "started_at").cmp(s(y, "started_at")));
    let run_status = latest.map(|j| s(j, "status")).unwrap_or("never run");
    let running = matches!(run_status, "running" | "starting" | "queued");
    let policy = restart_policy(&opts);
    let sup = p.get("supervisor").cloned().unwrap_or(Value::Null);
    let down = realtime && !running && run_status != "stopped" && run_status != "never run";
    let warning = down.then(|| {
        let next = if sup.get("gave_up").and_then(Value::as_bool) == Some(true) {
            "auto-restart gave up after 3 tries in 24 h — start it manually".to_string()
        } else if policy == "off" {
            "auto-restart is off — restart it at night (full snapshot on the source)".to_string()
        } else {
            format!("auto-restart ({policy}) will restart it with a full snapshot")
        };
        format!("{name}: real-time sync is NOT running (latest run: {run_status}); {next}.")
    });
    let view = json!({
        "profile": name,
        "id": pid,
        "mode": if realtime { "real-time (snapshot + live stream)" } else { "one-time copy" },
        "source": host_port(p.get("source")),
        "target": host_port(p.get("target")),
        "auto_restart": if realtime { json!(policy) } else { Value::Null },
        "supervisor": if realtime { sup } else { Value::Null },
        "latest_run": latest.map(|j| json!({
            "id": j.get("id"),
            "status": j.get("status"),
            "started_at": j.get("started_at"),
            "finished_at": j.get("finished_at"),
            "exit_code": j.get("exit_code"),
            "error": j.get("error"),
        })),
        "running": running,
    });
    (view, warning)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_summary_never_copies_secrets() {
        let t = json!({
            "id":"t1","name":"branch","host":"ssh.x","username":"cdc","private_key":"********","password":"",
            "host_key_fingerprint":"SHA256:abc","forward_host":"cdc-sshtunnel",
            "proxy":{"kind":"cloudflare_access","hostname":"ssh.x","service_token_id":"id","service_token_secret":"********"},
            "forwards":[{"remote_host":"127.0.0.1","remote_port":5432,"listen_port":22000}],
            "status":{"connected":true,"forwards":[{"listen_port":22000,"listening":true,"active_connections":2}]},
            "used_by":["Orders"]
        });
        let out = summarize_tunnel(&t);
        let text = out.to_string();
        assert!(
            !text.contains("private_key")
                && !text.contains("service_token")
                && !text.contains("********")
        );
        assert_eq!(out["state"], "connected");
        assert_eq!(out["forwards"][0]["address"], "cdc-sshtunnel:22000");
        assert_eq!(out["forwards"][0]["database"], "127.0.0.1:5432");
        assert_eq!(out["forwards"][0]["open_connections"], 2);
        assert_eq!(out["ssh"], "cdc@ssh.x (Cloudflare Access)");
    }

    #[test]
    fn unpinned_tunnel_reports_setup_incomplete() {
        let out = summarize_tunnel(
            &json!({"name":"x","host":"h","username":"u","proxy":{"kind":"none"}}),
        );
        assert_eq!(out["state"], "setup incomplete (host key not pinned)");
    }

    #[test]
    fn redis_view_warns_only_for_a_down_realtime_sync() {
        let p = json!({"id":"p1","name":"orders","options":{"sync_aof":true,"auto_restart":"window"},
            "source":{"host":"s","port":"6379"},"target":{"host":"t","port":"6379"},"supervisor":{"gave_up":false}});
        let failed =
            json!({"profile_id":"p1","status":"failed","started_at":"2026-11-15T01:00:00Z"});
        let (v, w) = redis_profile_view(&p, &[&failed]);
        assert_eq!(v["auto_restart"], "night window 00:00-05:00");
        assert_eq!(v["source"], "s:6379");
        assert!(w.unwrap().contains("will restart it with a full snapshot"));
        let stopped =
            json!({"profile_id":"p1","status":"stopped","started_at":"2026-11-15T02:00:00Z"});
        assert!(
            redis_profile_view(&p, &[&failed, &stopped]).1.is_none(),
            "latest run wins; a stop is not a failure"
        );
        let once = json!({"id":"p2","name":"copy","options":{"sync_aof":false}});
        let (v, w) = redis_profile_view(&once, &[]);
        assert!(w.is_none() && v["auto_restart"].is_null());
    }

    #[test]
    fn status_and_check_warnings() {
        assert!(status_warning("running").is_none());
        assert!(status_warning("stopped")
            .unwrap()
            .contains("not being synced"));
        assert!(
            check_warning(&json!({"id":"pg.slot_lag","status":"warn","detail":"5 GB"}))
                .unwrap()
                .contains("(warning)")
        );
        assert!(check_warning(&json!({"id":"pg.wal_level","status":"pass"})).is_none());
    }

    #[test]
    fn readiness_summary_keeps_only_problems() {
        let r = json!({"ready":false,"applicable":true,"engine":"postgres","summary":{"pass":1,"fail":1},
            "checks":[{"id":"pg.wal_level","status":"pass"},{"id":"pg.own_slot","status":"fail","detail":"missing","fix":"SELECT 1"}],
            "fix_script":"-- script"});
        let out = summarize_readiness(&r, false);
        assert_eq!(out["problems"].as_array().unwrap().len(), 1);
        assert_eq!(out["problems"][0]["id"], "pg.own_slot");
        assert!(out.get("fix_script").is_none());
        assert_eq!(summarize_readiness(&r, true)["fix_script"], "-- script");
    }
}
