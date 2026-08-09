//! Workflow run + approval reads — `GET /api/workflows/{id}/runs` and
//! `GET /api/workflows/{id}/runs/{run_id}/approvals`.
//!
//! NIP-98 signed, outside the Nostr event data plane (mirrors the moderation
//! read endpoints in `bridge.rs`). Access control mirrors `handle_workflow_def`
//! in `handlers/command_executor.rs`: the caller must be a member of the
//! workflow's channel. A workflow with no channel (`channel_id IS NULL`) has
//! no membership set to check against, so reads are restricted to the
//! workflow's owner in that case.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde_json::Value;
use uuid::Uuid;

use buzz_core::TenantContext;
use buzz_db::workflow::{ApprovalRecord, WorkflowRunRecord};

use super::bridge::{check_nip98_replay, nip98_expected_url, verify_bridge_auth};
use super::{api_error, internal_error};
use crate::state::AppState;

/// Cap on rows returned by a single runs read.
const RUNS_READ_LIMIT: i64 = 1000;
/// Default rows returned when `?limit=` is omitted.
const RUNS_DEFAULT_LIMIT: i64 = 100;

fn clamp_limit(requested: Option<i64>) -> i64 {
    requested
        .filter(|n| *n > 0)
        .map(|n| n.min(RUNS_READ_LIMIT))
        .unwrap_or(RUNS_DEFAULT_LIMIT)
}

/// Shared prelude for a workflow-run read: bind tenant, verify NIP-98 GET
/// auth, replay-check, load the workflow, and confirm the caller may view its
/// runs. Returns the resolved tenant, the caller's pubkey bytes, and the
/// workflow record.
///
/// Mirrors `authorize_moderation_read` in `bridge.rs` for the auth/tenant
/// steps; the access-control step below mirrors `handle_workflow_def`'s
/// membership check in `handlers/command_executor.rs` so run/approval reads
/// carry the same gate as the workflow definition itself.
async fn authorize_workflow_read(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    raw_query: Option<&str>,
    workflow_id: Uuid,
) -> Result<(TenantContext, buzz_db::workflow::WorkflowRecord), (StatusCode, Json<Value>)> {
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

    let path_with_query = match raw_query {
        Some(q) if !q.is_empty() => format!("{path}?{q}"),
        _ => path.to_string(),
    };
    let url = nip98_expected_url(&state.config.relay_url, &tenant, &path_with_query);
    let (pubkey, event_id_bytes) =
        verify_bridge_auth(headers, "GET", &url, None, state.config.require_auth_token)?;
    check_nip98_replay(state, &tenant, event_id_bytes).await?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let community_id = tenant.community();
    let workflow = state
        .db
        .get_workflow(community_id, workflow_id)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "workflow not found"))?;

    let authorized = match workflow.channel_id {
        Some(channel_id) => state
            .is_member_cached(community_id, channel_id, &pubkey_bytes)
            .await
            .map_err(|e| internal_error(&format!("membership check: {e}")))?,
        // No channel to check membership against — fall back to ownership.
        None => workflow.owner_pubkey == pubkey_bytes,
    };
    if !authorized {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "forbidden: not a member of this workflow's channel",
        ));
    }

    Ok((tenant, workflow))
}

/// Query parameters for `GET /api/workflows/{id}/runs`.
#[derive(Debug, Default, serde::Deserialize)]
pub struct RunsReadQuery {
    limit: Option<i64>,
}

/// `GET /api/workflows/{workflow_id}/runs` — list runs for a workflow, newest
/// first. NIP-98 auth; caller must be a member of the workflow's channel (or
/// its owner, for channel-less workflows).
pub async fn list_workflow_runs(
    State(state): State<Arc<AppState>>,
    Path(workflow_id_str): Path<String>,
    RawQuery(raw_query): RawQuery,
    Query(q): Query<RunsReadQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let workflow_id = Uuid::parse_str(&workflow_id_str)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid workflow UUID"))?;

    let path = format!("/api/workflows/{workflow_id_str}/runs");
    let (tenant, _workflow) =
        authorize_workflow_read(&state, &headers, &path, raw_query.as_deref(), workflow_id).await?;

    let runs = state
        .db
        .list_workflow_runs(tenant.community(), workflow_id, clamp_limit(q.limit))
        .await
        .map_err(|e| internal_error(&format!("list workflow runs: {e}")))?;

    Ok(Json(Value::Array(runs.iter().map(run_json).collect())))
}

/// `GET /api/workflows/{workflow_id}/runs/{run_id}/approvals` — list approval
/// gates for a run. NIP-98 auth; same access gate as `list_workflow_runs`.
pub async fn list_run_approvals(
    State(state): State<Arc<AppState>>,
    Path((workflow_id_str, run_id_str)): Path<(String, String)>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let workflow_id = Uuid::parse_str(&workflow_id_str)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid workflow UUID"))?;
    let run_id = Uuid::parse_str(&run_id_str)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid run UUID"))?;

    let path = format!("/api/workflows/{workflow_id_str}/runs/{run_id_str}/approvals");
    let (tenant, _workflow) =
        authorize_workflow_read(&state, &headers, &path, raw_query.as_deref(), workflow_id).await?;

    let approvals = state
        .db
        .get_run_approvals(tenant.community(), workflow_id, run_id)
        .await
        .map_err(|e| internal_error(&format!("list run approvals: {e}")))?;

    Ok(Json(Value::Array(
        approvals.iter().map(approval_json).collect(),
    )))
}

/// Serialize a [`WorkflowRunRecord`] into the desktop's `RawWorkflowRun`
/// shape (`desktop/src/shared/api/tauriWorkflows.ts`). Timestamps are unix
/// seconds; `status` serializes via `RunStatus`'s `snake_case` derive, which
/// already matches the frontend's `WorkflowRunStatus` union.
fn run_json(run: &WorkflowRunRecord) -> Value {
    serde_json::json!({
        "id": run.id,
        "workflow_id": run.workflow_id,
        "status": run.status,
        "current_step": run.current_step,
        "execution_trace": run.execution_trace,
        "started_at": run.started_at.map(|t| t.timestamp()),
        "completed_at": run.completed_at.map(|t| t.timestamp()),
        "error_message": run.error_message,
        "created_at": run.created_at.timestamp(),
    })
}

/// Serialize an [`ApprovalRecord`] into the desktop's `RawWorkflowApproval`
/// shape. `token` and `approver_pubkey` are `bytea` in Postgres (the token is
/// the SHA-256 hash, never the plaintext) and are hex-encoded for JSON.
/// `expires_at` is an RFC 3339 string, matching the TS `expiresAt: string`
/// field (unlike the numeric-timestamp run fields).
fn approval_json(approval: &ApprovalRecord) -> Value {
    serde_json::json!({
        "token": hex::encode(&approval.token),
        "workflow_id": approval.workflow_id,
        "run_id": approval.run_id,
        "step_id": approval.step_id,
        "step_index": approval.step_index,
        "approver_spec": approval.approver_spec,
        "status": approval.status,
        "approver_pubkey": approval.approver_pubkey.as_ref().map(hex::encode),
        "note": approval.note,
        "expires_at": approval.expires_at.to_rfc3339(),
        "created_at": approval.created_at.timestamp(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn sample_run() -> WorkflowRunRecord {
        WorkflowRunRecord {
            id: Uuid::nil(),
            community_id: buzz_core::CommunityId::from_uuid(Uuid::nil()),
            workflow_id: Uuid::nil(),
            status: buzz_db::workflow::RunStatus::WaitingApproval,
            trigger_event_id: None,
            current_step: 2,
            execution_trace: serde_json::json!([]),
            trigger_context: None,
            started_at: Some(Utc.timestamp_opt(1_700_000_000, 0).unwrap()),
            completed_at: None,
            error_message: None,
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        }
    }

    fn sample_approval() -> ApprovalRecord {
        ApprovalRecord {
            token: vec![0xab, 0xcd],
            workflow_id: Uuid::nil(),
            run_id: Uuid::nil(),
            step_id: "approve-deploy".to_string(),
            step_index: 1,
            approver_spec: "@alice".to_string(),
            status: buzz_db::workflow::ApprovalStatus::Pending,
            approver_pubkey: Some(vec![0x01, 0x02]),
            note: None,
            expires_at: Utc.timestamp_opt(1_700_003_600, 0).unwrap(),
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        }
    }

    /// `run_json` must produce every field `RawWorkflowRun`
    /// (`desktop/src/shared/api/tauriWorkflows.ts`) expects, snake_case, with
    /// the run status serialized as the `snake_case` string the TS
    /// `WorkflowRunStatus` union expects (not the workflow-status `lowercase`
    /// convention).
    #[test]
    fn run_json_matches_raw_workflow_run_contract() {
        let value = run_json(&sample_run());
        let obj = value.as_object().expect("run_json returns an object");

        for field in [
            "id",
            "workflow_id",
            "status",
            "current_step",
            "execution_trace",
            "started_at",
            "completed_at",
            "error_message",
            "created_at",
        ] {
            assert!(obj.contains_key(field), "missing field: {field}");
        }

        assert_eq!(obj["status"], "waiting_approval");
        assert_eq!(obj["current_step"], 2);
        assert_eq!(obj["completed_at"], Value::Null);
        assert_eq!(obj["started_at"], 1_700_000_000);
        assert_eq!(obj["created_at"], 1_700_000_000);
    }

    /// `approval_json` must produce every field `RawWorkflowApproval` expects,
    /// with `token`/`approver_pubkey` hex-encoded (they are `bytea` in
    /// Postgres) and `expires_at` as an RFC 3339 string (the TS type is
    /// `expiresAt: string`, unlike the numeric run timestamps).
    #[test]
    fn approval_json_matches_raw_workflow_approval_contract() {
        let value = approval_json(&sample_approval());
        let obj = value.as_object().expect("approval_json returns an object");

        for field in [
            "token",
            "workflow_id",
            "run_id",
            "step_id",
            "step_index",
            "approver_spec",
            "status",
            "approver_pubkey",
            "note",
            "expires_at",
            "created_at",
        ] {
            assert!(obj.contains_key(field), "missing field: {field}");
        }

        assert_eq!(obj["token"], "abcd");
        assert_eq!(obj["approver_pubkey"], "0102");
        assert_eq!(obj["status"], "pending");
        assert!(
            obj["expires_at"].as_str().unwrap().contains('T'),
            "expires_at must be an RFC 3339 string, got {:?}",
            obj["expires_at"]
        );
        assert_eq!(obj["note"], Value::Null);
    }

    #[test]
    fn clamp_limit_defaults_and_caps() {
        assert_eq!(clamp_limit(None), RUNS_DEFAULT_LIMIT);
        assert_eq!(clamp_limit(Some(0)), RUNS_DEFAULT_LIMIT);
        assert_eq!(clamp_limit(Some(-5)), RUNS_DEFAULT_LIMIT);
        assert_eq!(clamp_limit(Some(5)), 5);
        assert_eq!(clamp_limit(Some(RUNS_READ_LIMIT + 500)), RUNS_READ_LIMIT);
    }

    // ──────────────────────────────────────────────────────────────────────
    // Router-level tests: real Postgres, real HTTP request through the axum
    // router, X-Pubkey dev-mode auth. Mirrors `bridge_handler_test_state` /
    // `post_events` in `bridge.rs`.
    //
    // `#[ignore = "requires Postgres"]`: needs a real `communities`/`channels`/
    // `workflows`/`workflow_runs` row set. Run explicitly:
    //   cargo test -p buzz-relay --lib api::workflows::tests::router_tests -- --ignored
    // ──────────────────────────────────────────────────────────────────────
    mod router_tests {
        use super::*;
        use axum::body::Body;
        use axum::http::{header, Request};
        use buzz_db::channel::{ChannelType, ChannelVisibility};
        use nostr::Keys;
        use tower::ServiceExt;

        const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

        /// Build an AppState suitable for router-level workflow-read tests.
        /// Returns `None` when local Postgres/Redis is not reachable.
        async fn test_state() -> Option<Arc<crate::state::AppState>> {
            let mut config = crate::config::Config::from_env().ok()?;
            config.database_url = TEST_DB_URL.to_string();
            config.redis_url =
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
            config.relay_url = "wss://workflows-test.local".to_string();
            // NIP-98 auth is still required (X-Pubkey is the dev-mode
            // fallback path exercised here); membership is enforced by this
            // module's own channel-membership check, independent of the
            // relay-wide membership gate.
            config.require_auth_token = false;

            let pool = sqlx::PgPool::connect(TEST_DB_URL).await.ok()?;
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .ok()?;
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .ok()?,
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;

            let (state, _audit_shutdown) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                Keys::generate(),
                media_storage,
            );
            Some(Arc::new(state))
        }

        async fn get(
            state: Arc<crate::state::AppState>,
            host: &str,
            pubkey_hex: &str,
            path: &str,
        ) -> (axum::http::StatusCode, Value) {
            let response = crate::router::build_router(state)
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(path)
                        .header(header::HOST, host)
                        .header("x-pubkey", pubkey_hex)
                        .body(Body::empty())
                        .expect("build request"),
                )
                .await
                .expect("router oneshot");
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read body");
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            (status, body)
        }

        /// A channel member can list runs for a workflow scoped to their
        /// channel, and the response round-trips the fields
        /// `RawWorkflowRun` expects.
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn list_workflow_runs_returns_runs_for_channel_member() {
            let Some(state) = test_state().await else {
                panic!(
                    "local Postgres not reachable — start Postgres before running ignored tests"
                );
            };

            let host = format!("workflows-test-{}.local", Uuid::new_v4().simple());
            let community = state
                .db
                .ensure_configured_community(&host)
                .await
                .expect("ensure community");
            let community_id = community.id;

            let owner = Keys::generate();
            let owner_bytes = owner.public_key().to_bytes().to_vec();
            state
                .db
                .ensure_user(community_id, &owner_bytes)
                .await
                .expect("ensure user");

            let channel = state
                .db
                .create_channel(
                    community_id,
                    &format!("wf-runs-{}", Uuid::new_v4().simple()),
                    ChannelType::Stream,
                    ChannelVisibility::Open,
                    None,
                    &owner_bytes,
                    None,
                )
                .await
                .expect("create channel");

            let workflow_id = state
                .db
                .create_workflow(
                    community_id,
                    Some(channel.id),
                    &owner_bytes,
                    "test workflow",
                    "{}",
                    &[0u8; 32],
                )
                .await
                .expect("create workflow");

            let run_id = state
                .db
                .create_workflow_run(community_id, workflow_id, None, None)
                .await
                .expect("create workflow run");

            let (status, body) = get(
                state.clone(),
                &host,
                &owner.public_key().to_hex(),
                &format!("/api/workflows/{workflow_id}/runs"),
            )
            .await;

            assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
            let runs = body.as_array().expect("runs must be a bare JSON array");
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0]["id"], run_id.to_string());
            assert_eq!(runs[0]["workflow_id"], workflow_id.to_string());
            assert_eq!(runs[0]["status"], "pending");
        }

        /// A caller who is not a member of the workflow's channel is
        /// forbidden from reading its runs — mirrors the membership gate on
        /// `handle_workflow_def`.
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn list_workflow_runs_forbidden_for_non_member() {
            let Some(state) = test_state().await else {
                panic!(
                    "local Postgres not reachable — start Postgres before running ignored tests"
                );
            };

            let host = format!("workflows-test-{}.local", Uuid::new_v4().simple());
            let community = state
                .db
                .ensure_configured_community(&host)
                .await
                .expect("ensure community");
            let community_id = community.id;

            let owner = Keys::generate();
            let owner_bytes = owner.public_key().to_bytes().to_vec();
            let outsider = Keys::generate();
            state
                .db
                .ensure_user(community_id, &owner_bytes)
                .await
                .expect("ensure user");

            let channel = state
                .db
                .create_channel(
                    community_id,
                    &format!("wf-runs-forbidden-{}", Uuid::new_v4().simple()),
                    ChannelType::Stream,
                    ChannelVisibility::Open,
                    None,
                    &owner_bytes,
                    None,
                )
                .await
                .expect("create channel");

            let workflow_id = state
                .db
                .create_workflow(
                    community_id,
                    Some(channel.id),
                    &owner_bytes,
                    "test workflow",
                    "{}",
                    &[0u8; 32],
                )
                .await
                .expect("create workflow");

            let (status, _body) = get(
                state.clone(),
                &host,
                &outsider.public_key().to_hex(),
                &format!("/api/workflows/{workflow_id}/runs"),
            )
            .await;

            assert_eq!(status, axum::http::StatusCode::FORBIDDEN);
        }

        /// Approvals for a run round-trip token/pubkey hex-encoding and the
        /// RFC 3339 `expires_at` string format.
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn list_run_approvals_round_trips_hex_and_timestamp_shapes() {
            let Some(state) = test_state().await else {
                panic!(
                    "local Postgres not reachable — start Postgres before running ignored tests"
                );
            };

            let host = format!("workflows-test-{}.local", Uuid::new_v4().simple());
            let community = state
                .db
                .ensure_configured_community(&host)
                .await
                .expect("ensure community");
            let community_id = community.id;

            let owner = Keys::generate();
            let owner_bytes = owner.public_key().to_bytes().to_vec();
            state
                .db
                .ensure_user(community_id, &owner_bytes)
                .await
                .expect("ensure user");

            let channel = state
                .db
                .create_channel(
                    community_id,
                    &format!("wf-approvals-{}", Uuid::new_v4().simple()),
                    ChannelType::Stream,
                    ChannelVisibility::Open,
                    None,
                    &owner_bytes,
                    None,
                )
                .await
                .expect("create channel");

            let workflow_id = state
                .db
                .create_workflow(
                    community_id,
                    Some(channel.id),
                    &owner_bytes,
                    "test workflow",
                    "{}",
                    &[0u8; 32],
                )
                .await
                .expect("create workflow");

            let run_id = state
                .db
                .create_workflow_run(community_id, workflow_id, None, None)
                .await
                .expect("create workflow run");

            state
                .db
                .create_approval(buzz_db::workflow::CreateApprovalParams {
                    community_id,
                    token: "raw-test-token",
                    workflow_id,
                    run_id,
                    step_id: "approve-deploy",
                    step_index: 0,
                    approver_spec: "@alice",
                    expires_at: Utc::now() + chrono::Duration::hours(1),
                })
                .await
                .expect("create approval");

            let (status, body) = get(
                state.clone(),
                &host,
                &owner.public_key().to_hex(),
                &format!("/api/workflows/{workflow_id}/runs/{run_id}/approvals"),
            )
            .await;

            assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
            let approvals = body.as_array().expect("approvals must be a bare array");
            assert_eq!(approvals.len(), 1);
            assert_eq!(approvals[0]["status"], "pending");
            assert_eq!(approvals[0]["step_id"], "approve-deploy");
            // token is the hashed value, hex-encoded — never the plaintext.
            let token_hex = approvals[0]["token"].as_str().expect("token is a string");
            assert_ne!(token_hex, "raw-test-token");
            assert!(hex::decode(token_hex).is_ok());
            assert!(approvals[0]["expires_at"]
                .as_str()
                .expect("expires_at is a string")
                .contains('T'));
        }
    }
}
