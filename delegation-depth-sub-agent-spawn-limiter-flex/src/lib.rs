// Copyright 2026 Salesforce, Inc. All rights reserved.
//! Delegation Depth & Sub-Agent Spawn Limiter — MCP-native, inbound Omni/Flex policy.
//!
//! Reads a delegation header (`{"root","depth","agent","parent"}`) on MCP
//! `tools/call` and enforces two ceilings:
//!   * depth  — reject when header depth > maxDepth (STATELESS, deterministic);
//!   * fan-out — reject when a root has spawned more than maxFanout distinct
//!               sub-agents within windowMillis (per-replica shared storage).
//! Guards against runaway agent subcontracting (OWASP ASI-T07 Authority Drift).

mod delegation;
mod generated;

use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use pdk::data_storage::{DataStorage, DataStorageBuilder, LocalDataStorage, StoreMode};
use pdk::hl::*;
use pdk::logger;
use serde_json::{json, Value};

use crate::delegation::{
    depth_exceeded, hash_id, parse, record_distinct, FanoutState,
};
use crate::generated::config::Config;

const TOOLS_CALL: &str = "tools/call";
const STORE_NAMESPACE: &str = "delegation-fanout";
const FANOUT_CAP: usize = 512;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// HTTP 403 with an in-band JSON-RPC error (server-defined code -32005).
fn deny(id: Value, message: String) -> Response {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32005, "message": message }
    })
    .to_string();
    Response::new(403)
        .with_headers(vec![("content-type".to_string(), "application/json".to_string())])
        .with_body(body.into_bytes())
}

async fn request_filter(
    request_state: RequestState,
    storage: Rc<LocalDataStorage>,
    config: Rc<Config>,
) -> Flow<()> {
    let headers_state = request_state.into_headers_state().await;

    if headers_state.method().as_str() != "POST" {
        return Flow::Continue(());
    }
    match headers_state.handler().header("content-type") {
        Some(ct) if ct.starts_with("application/json") => {}
        _ => return Flow::Continue(()),
    }

    let header_name = config.header_name.as_deref().unwrap_or("x-agent-delegation");
    let raw = headers_state.handler().header(header_name);

    let body_state = headers_state.into_body_state().await;
    let body = body_state.handler().body();
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Flow::Continue(()),
    };
    if req.get("method").and_then(Value::as_str) != Some(TOOLS_CALL) {
        return Flow::Continue(());
    }

    let del = match parse(raw.as_deref()) {
        Some(d) => d,
        None => {
            // malformed delegation header — fail closed (can't trust the chain)
            logger::warn!("delegation-limiter: malformed delegation header; rejecting");
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            return maybe_break(&config, deny(id, "malformed delegation header".to_string()));
        }
    };

    let max_depth = config.max_depth.unwrap_or(5);
    let id = req.get("id").cloned().unwrap_or(Value::Null);

    // --- depth ceiling (stateless) ---
    if depth_exceeded(del.depth, max_depth) {
        logger::warn!(
            "delegation-limiter: depth {} exceeds max {} (root '{}')",
            del.depth, max_depth, del.root
        );
        return maybe_break(
            &config,
            deny(
                id,
                format!("delegation depth {} exceeds limit {}", del.depth, max_depth),
            ),
        );
    }

    // --- fan-out ceiling (stateful, per root) ---
    let max_fanout = config.max_fanout.unwrap_or(10);
    if max_fanout > 0 && !del.root.is_empty() && !del.agent.is_empty() {
        let window = config.window_millis.unwrap_or(60_000).max(0) as u64;
        let now = now_ms();
        let agent_id = hash_id(&del.agent);

        let existing = match storage.get::<FanoutState>(&del.root).await {
            Ok(v) => v,
            Err(e) => {
                logger::warn!("delegation-limiter: storage read failed ({e}); allowing");
                return Flow::Continue(());
            }
        };
        let st = existing.map(|(s, _cas)| s).unwrap_or_default();
        let (updated, distinct) = record_distinct(st, agent_id, now, window, FANOUT_CAP);
        if let Err(e) = storage.store(&del.root, &StoreMode::Always, &updated).await {
            logger::warn!("delegation-limiter: storage write failed ({e})");
        }

        if distinct as i64 > max_fanout {
            logger::warn!(
                "delegation-limiter: root '{}' fan-out {} exceeds max {}",
                del.root, distinct, max_fanout
            );
            return maybe_break(
                &config,
                deny(
                    id,
                    format!(
                        "sub-agent fan-out {} for root '{}' exceeds limit {}",
                        distinct, del.root, max_fanout
                    ),
                ),
            );
        }
    }

    Flow::Continue(())
}

/// Break with the deny response, unless onExceed=report (then log-only continue).
fn maybe_break(config: &Config, response: Response) -> Flow<()> {
    if config.on_exceed.as_deref() == Some("report") {
        logger::warn!("delegation-limiter: (report mode) limit exceeded — allowing");
        Flow::Continue(())
    } else {
        Flow::Break(response)
    }
}

#[entrypoint]
async fn configure(
    launcher: Launcher,
    store_builder: DataStorageBuilder,
    Configuration(bytes): Configuration,
) -> Result<()> {
    let config: Config = serde_json::from_slice(&bytes).map_err(|err| {
        anyhow!(
            "Failed to parse configuration '{}'. Cause: {}",
            String::from_utf8_lossy(&bytes),
            err
        )
    })?;
    let config = Rc::new(config);
    let storage = Rc::new(store_builder.local(STORE_NAMESPACE));

    let filter = on_request(move |rs| {
        let config = config.clone();
        let storage = storage.clone();
        async move { request_filter(rs, storage, config).await }
    });

    launcher.launch(filter).await?;
    Ok(())
}
