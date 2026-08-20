//! `GET /api/agents/{agent_pubkey}/jobs` — the provider-side job ledger for an
//! agent's home community (BUZZ-10).
//!
//! Returns the cross-community jobs one of THIS community's listed agents ran
//! for foreign callers: which community called, duration, and estimated cost
//! from our own listing rate. Authorized to the agent's verified owner only.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::bridge::{check_nip98_replay, nip98_expected_url, verify_bridge_auth};
use super::{api_error, internal_error};
use crate::state::AppState;

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 500;

/// Query parameters for the provider job-ledger read.
#[derive(Debug, Deserialize)]
pub struct JobsReadQuery {
    limit: Option<i64>,
}

fn clamp_limit(requested: Option<i64>) -> i64 {
    requested.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// `GET /api/agents/{agent_pubkey}/jobs` — provider job ledger, owner-gated.
pub async fn list_agent_jobs(
    State(state): State<Arc<AppState>>,
    Path(agent_pubkey_hex): Path<String>,
    RawQuery(raw_query): RawQuery,
    Query(q): Query<JobsReadQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let agent_pubkey = hex::decode(&agent_pubkey_hex)
        .ok()
        .filter(|bytes| bytes.len() == 32)
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "invalid agent pubkey"))?;

    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })?;

    let path = format!("/api/agents/{agent_pubkey_hex}/jobs");
    let path_with_query = match raw_query.as_deref() {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path,
    };
    let url = nip98_expected_url(&state.config.relay_url, &tenant, &path_with_query);
    let (caller, event_id_bytes) =
        verify_bridge_auth(&headers, "GET", &url, None, state.config.require_auth_token)?;
    check_nip98_replay(&state, &tenant, event_id_bytes).await?;

    // Only the agent's verified owner may read its earnings ledger.
    let is_owner = state
        .db
        .is_agent_owner(tenant.community(), &agent_pubkey, &caller.to_bytes())
        .await
        .map_err(|e| internal_error(&format!("agent owner lookup: {e}")))?;
    if !is_owner {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: only the agent's owner may read its job ledger",
        ));
    }

    let jobs = state
        .db
        .list_provider_jobs_for_agent(tenant.community(), &agent_pubkey, clamp_limit(q.limit))
        .await
        .map_err(|e| internal_error(&format!("list provider jobs: {e}")))?;

    let items: Vec<Value> = jobs.iter().map(job_json).collect();
    Ok(Json(json!({ "jobs": items, "totals": totals_json(&jobs) })))
}

fn job_json(job: &buzz_db::provider_jobs::ProviderJobRecord) -> Value {
    let estimated =
        job.rate_microunits_per_hour
            .zip(job.duration_ms)
            .and_then(|(rate, duration)| {
                buzz_core::marketplace::estimated_microunits(rate as u64, duration as u64).ok()
            });
    json!({
        "request_event_id": job.request_event_id,
        "request_id": job.request_id,
        "agent_pubkey": hex::encode(&job.agent_pubkey),
        "agent_owner_pubkey": job.agent_owner_pubkey.as_ref().map(hex::encode),
        "caller_relay_pubkey": hex::encode(&job.caller_relay_pubkey),
        "caller_relay_url": job.caller_relay_url,
        "listing_event_id": job.listing_event_id,
        "rate_currency": job.rate_currency,
        "rate_microunits_per_hour": job.rate_microunits_per_hour,
        "requested_at_ms": job.requested_at.timestamp_millis(),
        "terminal_at_ms": job.terminal_at.map(|t| t.timestamp_millis()),
        "duration_ms": job.duration_ms,
        "estimated_microunits": estimated,
        "outcome": job.outcome.clone().unwrap_or_else(|| "pending".into()),
    })
}

/// Per-caller-community and per-currency rollups. Estimates are never summed
/// across currencies — mixed-currency callers each get their own line.
fn totals_json(jobs: &[buzz_db::provider_jobs::ProviderJobRecord]) -> Value {
    use std::collections::BTreeMap;

    // (caller_relay_pubkey_hex, currency) -> (job_count, total_micros, total_ms)
    let mut by_caller: BTreeMap<(String, Option<String>), (u64, u128, i128)> = BTreeMap::new();
    for job in jobs {
        let caller = hex::encode(&job.caller_relay_pubkey);
        let estimated = job
            .rate_microunits_per_hour
            .zip(job.duration_ms)
            .and_then(|(rate, duration)| {
                buzz_core::marketplace::estimated_microunits(rate as u64, duration as u64).ok()
            })
            .unwrap_or(0);
        let entry = by_caller
            .entry((caller, job.rate_currency.clone()))
            .or_insert((0, 0, 0));
        entry.0 += 1;
        entry.1 += estimated as u128;
        entry.2 += job.duration_ms.unwrap_or(0) as i128;
    }

    let rows: Vec<Value> = by_caller
        .into_iter()
        .map(|((caller, currency), (count, micros, ms))| {
            json!({
                "caller_relay_pubkey": caller,
                "currency": currency,
                "job_count": count,
                "estimated_microunits": micros as u64,
                "total_duration_ms": ms as i64,
            })
        })
        .collect();
    json!({ "by_caller": rows, "job_count": jobs.len() })
}
