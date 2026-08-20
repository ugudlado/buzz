//! Provider-side cross-community job ledger (BUZZ-10).
//!
//! Records, on an agent's HOME community, each job a foreign caller community
//! dispatched to one of our listed agents: who called, for how long, and the
//! cost estimated from our own listing rate. Populated entirely from cleartext
//! job-event tags and timestamps — the caller-encrypted result is never read.
//!
//! The symmetric caller-side record lives in [`crate::workflow`]'s
//! `workflow_agent_steps`.

use sqlx::{PgPool, Row};

use buzz_core::CommunityId;

use crate::error::Result;

/// One provider-side job record.
#[derive(Debug, Clone)]
pub struct ProviderJobRecord {
    /// kind:43001 request event id — our per-job correlation key.
    pub request_event_id: String,
    /// Cleartext correlation id from the request `request` tag.
    pub request_id: String,
    /// Our listed agent that ran the job.
    pub agent_pubkey: Vec<u8>,
    /// Verified owner of that agent, when known.
    pub agent_owner_pubkey: Option<Vec<u8>>,
    /// Caller community's relay identity.
    pub caller_relay_pubkey: Vec<u8>,
    /// Caller community's relay routing hint.
    pub caller_relay_url: String,
    /// Our kind:30177 listing snapshot at accept time.
    pub listing_event_id: String,
    /// Snapshotted listing currency.
    pub rate_currency: Option<String>,
    /// Snapshotted listing rate.
    pub rate_microunits_per_hour: Option<i64>,
    /// When the request arrived (from event timestamp).
    pub requested_at: chrono::DateTime<chrono::Utc>,
    /// When the terminal was observed, if completed.
    pub terminal_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Duration derived from `terminal_at - requested_at`.
    pub duration_ms: Option<i64>,
    /// Coarse outcome from the terminal event kind: `completed` or `failed`.
    pub outcome: Option<String>,
    /// The terminal (43004/43003) event id, when completed.
    pub completion_event_id: Option<String>,
}

/// Fields captured when our agent accepts a cross-community request.
pub struct RecordJobAcceptedParams<'a> {
    /// Home community that owns this ledger.
    pub community_id: CommunityId,
    /// kind:43001 request event id.
    pub request_event_id: &'a str,
    /// Cleartext correlation id.
    pub request_id: &'a str,
    /// Our listed agent.
    pub agent_pubkey: &'a [u8],
    /// Verified agent owner, when known.
    pub agent_owner_pubkey: Option<&'a [u8]>,
    /// Caller community relay identity.
    pub caller_relay_pubkey: &'a [u8],
    /// Caller community relay routing hint.
    pub caller_relay_url: &'a str,
    /// Our listing snapshot event id.
    pub listing_event_id: &'a str,
    /// Snapshotted currency.
    pub rate_currency: Option<&'a str>,
    /// Snapshotted rate.
    pub rate_microunits_per_hour: Option<i64>,
    /// Request-arrival timestamp.
    pub requested_at: chrono::DateTime<chrono::Utc>,
}

/// Insert a provider job row at accept time. Idempotent per request: a
/// duplicate accept (re-poll) leaves the existing row untouched.
pub async fn record_job_accepted(pool: &PgPool, params: RecordJobAcceptedParams<'_>) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO provider_agent_jobs
            (community_id, request_event_id, request_id, agent_pubkey,
             agent_owner_pubkey, caller_relay_pubkey, caller_relay_url,
             listing_event_id, rate_currency, rate_microunits_per_hour, requested_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (community_id, request_event_id) DO NOTHING
        "#,
    )
    .bind(params.community_id.as_uuid())
    .bind(params.request_event_id)
    .bind(params.request_id)
    .bind(params.agent_pubkey)
    .bind(params.agent_owner_pubkey)
    .bind(params.caller_relay_pubkey)
    .bind(params.caller_relay_url)
    .bind(params.listing_event_id)
    .bind(params.rate_currency)
    .bind(params.rate_microunits_per_hour)
    .bind(params.requested_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Stamp a provider job terminal. `duration_ms` is derived in SQL from the
/// stored `requested_at`, mirroring the caller-side receipt. First terminal
/// wins; a later duplicate leaves the recorded values unchanged.
pub async fn record_job_terminal(
    pool: &PgPool,
    community_id: CommunityId,
    request_event_id: &str,
    outcome: &str,
    completion_event_id: &str,
    terminal_at: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE provider_agent_jobs
        SET terminal_at = $3,
            duration_ms = GREATEST(
                0,
                (EXTRACT(EPOCH FROM ($3 - requested_at)) * 1000)::BIGINT
            ),
            outcome = $4,
            completion_event_id = $5
        WHERE community_id = $1
          AND request_event_id = $2
          AND terminal_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(request_event_id)
    .bind(terminal_at)
    .bind(outcome)
    .bind(completion_event_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// List provider jobs for one of our agents, newest first.
pub async fn list_jobs_for_agent(
    pool: &PgPool,
    community_id: CommunityId,
    agent_pubkey: &[u8],
    limit: i64,
) -> Result<Vec<ProviderJobRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT request_event_id, request_id, agent_pubkey, agent_owner_pubkey,
               caller_relay_pubkey, caller_relay_url, listing_event_id,
               rate_currency, rate_microunits_per_hour, requested_at,
               terminal_at, duration_ms, outcome, completion_event_id
        FROM provider_agent_jobs
        WHERE community_id = $1 AND agent_pubkey = $2
        ORDER BY requested_at DESC
        LIMIT $3
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(agent_pubkey)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_record).collect()
}

fn row_to_record(row: sqlx::postgres::PgRow) -> Result<ProviderJobRecord> {
    Ok(ProviderJobRecord {
        request_event_id: row.try_get("request_event_id")?,
        request_id: row.try_get("request_id")?,
        agent_pubkey: row.try_get("agent_pubkey")?,
        agent_owner_pubkey: row.try_get("agent_owner_pubkey")?,
        caller_relay_pubkey: row.try_get("caller_relay_pubkey")?,
        caller_relay_url: row.try_get("caller_relay_url")?,
        listing_event_id: row.try_get("listing_event_id")?,
        rate_currency: row.try_get("rate_currency")?,
        rate_microunits_per_hour: row.try_get("rate_microunits_per_hour")?,
        requested_at: row.try_get("requested_at")?,
        terminal_at: row.try_get("terminal_at")?,
        duration_ms: row.try_get("duration_ms")?,
        outcome: row.try_get("outcome")?,
        completion_event_id: row.try_get("completion_event_id")?,
    })
}
