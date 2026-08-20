//! Cross-community agent-job validation and relay-to-relay HTTP transport.

use std::time::Duration;
use std::{
    sync::atomic::{AtomicU32, Ordering},
    sync::Arc,
};

use buzz_core::agent_job::{
    build_cancellation_event, decrypt_terminal_event, validate_cancellation_event,
    validate_request_envelope, validate_response_envelope, JobCancellationEnvelope,
    JobRequestEnvelope, JobResultPayload,
};
use buzz_core::kind::{KIND_JOB_ERROR, KIND_JOB_RESULT, KIND_MANAGED_AGENT};
use buzz_core::marketplace::AgentMarketplace;
use buzz_core::tenant::CommunityId;
use nostr::{Alphabet, Event, Filter, Keys, Kind, PublicKey, SingleLetterTag};
use reqwest::header::ACCEPT;

use crate::state::AppState;

const MAX_HTTP_BODY_BYTES: usize = 1_048_576;
const REQUESTS_PER_WINDOW: u32 = 60;
pub(crate) const RATE_CACHE_CAPACITY: u64 = 50_000;
pub(crate) const RATE_WINDOW: Duration = Duration::from_secs(60);

/// Verified remote marketplace listing used to snapshot an assignment.
#[derive(Debug, Clone)]
pub struct RemoteListing {
    /// Exact listing event id.
    pub event_id: String,
    /// Verified listing author / managed-agent owner.
    pub owner_pubkey: Vec<u8>,
    /// Sanitized marketplace projection.
    pub marketplace: AgentMarketplace,
}

/// Match a non-member cancellation to the exact request already admitted here.
pub async fn validate_incoming_cancellation(
    state: &AppState,
    community_id: CommunityId,
    event: &Event,
) -> Result<JobCancellationEnvelope, String> {
    buzz_core::verify_event(event).map_err(|error| format!("invalid event: {error}"))?;
    let cancellation = validate_cancellation_event(event).map_err(|error| error.to_string())?;
    let request = state
        .db
        .get_event_by_id(community_id, &cancellation.request_event_id.to_bytes())
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "cancellation request event was not found".to_string())?
        .event;
    let request_envelope = validate_request_envelope(&request, request.created_at.as_secs())
        .map_err(|error| error.to_string())?;
    if request.pubkey != event.pubkey
        || request_envelope.agent_pubkey != cancellation.agent_pubkey
        || request_envelope.request_id != cancellation.request_id
    {
        return Err("cancellation correlation check failed".into());
    }
    verify_relay_identity(
        &request_envelope.relay_url,
        &request_envelope.relay_pubkey,
        &state.config.relay_url,
    )
    .await?;
    Ok(cancellation)
}

/// A terminal response matched to a pending local assignment.
pub struct ValidatedTerminalJob {
    /// Durable step that authorized the non-member response.
    pub step: buzz_db::workflow::AgentStepRecord,
    /// Decrypted and validated terminal payload.
    pub payload: JobResultPayload,
}

/// Persist and best-effort deliver a cancellation for a remote assignment.
pub async fn publish_cancellation(
    state: &AppState,
    community_id: CommunityId,
    step: &buzz_db::workflow::AgentStepRecord,
) -> Result<bool, String> {
    let (Some(request_id), Some(relay_pubkey), Some(relay_url)) = (
        step.request_id.as_deref(),
        step.agent_relay_pubkey.as_deref(),
        step.agent_relay_url.as_deref(),
    ) else {
        return Ok(false);
    };
    let agent_pubkey =
        PublicKey::from_slice(&step.agent_pubkey).map_err(|error| error.to_string())?;
    let target_relay = PublicKey::from_slice(relay_pubkey).map_err(|error| error.to_string())?;
    let request_event_id = step
        .prompt_event_id
        .parse()
        .map_err(|error| format!("invalid stored request event id: {error}"))?;
    let event = build_cancellation_event(
        &state.relay_keypair,
        &agent_pubkey,
        &request_event_id,
        request_id,
    )
    .map_err(|error| error.to_string())?;
    state
        .db
        .insert_event(community_id, &event, None)
        .await
        .map_err(|error| error.to_string())?;
    submit_remote_event(&state.relay_keypair, relay_url, &target_relay, &event).await?;
    Ok(true)
}

/// Verify a remote request's relay identity and current local listing policy.
pub async fn validate_incoming_request(
    state: &AppState,
    community_id: CommunityId,
    event: &Event,
) -> Result<JobRequestEnvelope, String> {
    buzz_core::verify_event(event).map_err(|error| format!("invalid event: {error}"))?;
    let envelope = validate_request_envelope(event, chrono::Utc::now().timestamp() as u64)
        .map_err(|error| error.to_string())?;
    verify_relay_identity(
        &envelope.relay_url,
        &envelope.relay_pubkey,
        &state.config.relay_url,
    )
    .await?;
    let listing = local_listing(state, community_id, &envelope.agent_pubkey).await?;
    let policy = listing
        .marketplace
        .remote_invocation
        .as_ref()
        .ok_or_else(|| "remote invocation is not enabled".to_string())?;
    if !policy.allows(&envelope.relay_pubkey.to_hex()) {
        return Err("caller relay is not allowed by the listing policy".into());
    }
    if incoming_request_rate_limited(state, community_id, &envelope) {
        return Err("rate-limited: too many remote requests for this agent".into());
    }
    // Provider-side ledger: record that a caller community dispatched to one of
    // our listed agents, snapshotting our own rate. Best-effort — a ledger
    // write failure must not reject an otherwise-valid job.
    let (rate_currency, rate_microunits) = listing
        .marketplace
        .pricing
        .as_ref()
        .map(|rate| {
            (
                Some(rate.currency.clone()),
                Some(rate.microunits_per_hour as i64),
            )
        })
        .unwrap_or((None, None));
    if let Err(error) = state
        .db
        .record_provider_job_accepted(buzz_db::provider_jobs::RecordJobAcceptedParams {
            community_id,
            request_event_id: &event.id.to_hex(),
            request_id: &envelope.request_id,
            agent_pubkey: &envelope.agent_pubkey.to_bytes(),
            agent_owner_pubkey: Some(&listing.owner_pubkey),
            caller_relay_pubkey: &envelope.relay_pubkey.to_bytes(),
            caller_relay_url: &envelope.relay_url,
            listing_event_id: &listing.event_id,
            rate_currency: rate_currency.as_deref(),
            rate_microunits_per_hour: rate_microunits,
            requested_at: chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
                .unwrap_or_else(chrono::Utc::now),
        })
        .await
    {
        tracing::warn!(%error, request_id = %envelope.request_id, "provider job-ledger accept write failed");
    }
    Ok(envelope)
}

fn incoming_request_rate_limited(
    state: &AppState,
    community_id: CommunityId,
    envelope: &JobRequestEnvelope,
) -> bool {
    let counter = state.remote_job_rate_limiter.get_with(
        (
            community_id,
            envelope.relay_pubkey.to_bytes(),
            envelope.agent_pubkey.to_bytes(),
        ),
        || Arc::new(AtomicU32::new(0)),
    );
    counter_is_rate_limited(&counter)
}

fn counter_is_rate_limited(counter: &AtomicU32) -> bool {
    counter.fetch_add(1, Ordering::Relaxed) >= REQUESTS_PER_WINDOW
}

/// Match, authenticate, and decrypt a remote terminal response.
pub async fn validate_incoming_terminal(
    state: &AppState,
    community_id: CommunityId,
    event: &Event,
) -> Result<ValidatedTerminalJob, String> {
    buzz_core::verify_event(event).map_err(|error| format!("invalid event: {error}"))?;
    let kind = buzz_core::kind::event_kind_u32(event);
    if !matches!(kind, KIND_JOB_RESULT | KIND_JOB_ERROR) {
        return Err("event is not a terminal agent-job response".into());
    }
    let envelope = validate_response_envelope(event).map_err(|error| error.to_string())?;
    if envelope.relay_pubkey != state.relay_keypair.public_key() {
        return Err("job response targets a different relay".into());
    }
    let step = state
        .db
        .get_agent_step(community_id, &envelope.request_event_id.to_hex())
        .await
        .map_err(|_| "job response does not match a pending request".to_string())?;
    if step.status != buzz_db::workflow::AgentStepStatus::Pending
        || step.expires_at <= chrono::Utc::now()
        || step.request_id.as_deref() != Some(envelope.request_id.as_str())
        || step.agent_pubkey != event.pubkey.to_bytes()
        || step.origin_relay_pubkey.as_deref() != Some(state.relay_keypair.public_key().as_bytes())
    {
        return Err("job response correlation check failed".into());
    }
    let (_, payload) =
        decrypt_terminal_event(event, &state.relay_keypair).map_err(|error| error.to_string())?;
    Ok(ValidatedTerminalJob { step, payload })
}

/// Authorize a terminal that one of THIS relay's own agents is posting back to
/// a foreign caller relay (the `p` recipient is not us).
///
/// This is the origin-relay side of result delivery: when a local agent
/// finishes a cross-community job, it persists a home receipt whose `p` tag is
/// the caller relay. That event must be admitted even though its recipient is
/// not a member here — but only when the author is genuinely one of our listed,
/// remote-invocable agents, so a stray non-member cannot inject a terminal
/// addressed to some third-party relay. Structural envelope validity is checked
/// too; correlation against a pending assignment is the *caller's* job, not the
/// origin's.
pub async fn validate_outgoing_terminal(
    state: &AppState,
    community_id: CommunityId,
    event: &Event,
) -> Result<(), String> {
    buzz_core::verify_event(event).map_err(|error| format!("invalid event: {error}"))?;
    let kind = buzz_core::kind::event_kind_u32(event);
    if !matches!(kind, KIND_JOB_RESULT | KIND_JOB_ERROR) {
        return Err("event is not a terminal agent-job response".into());
    }
    validate_response_envelope(event).map_err(|error| error.to_string())?;
    // The author must be one of our own listed, remote-invocable agents.
    let listing = local_listing(state, community_id, &event.pubkey).await?;
    listing
        .marketplace
        .remote_invocation
        .as_ref()
        .ok_or_else(|| "author agent is not remote-invocable in this community".to_string())?;
    Ok(())
}

/// Fetch and verify an agent listing from its home relay.
pub async fn query_remote_listing(
    source_keys: &Keys,
    source_relay_url: &str,
    target_relay_url: &str,
    target_relay_pubkey: &PublicKey,
    agent_pubkey: &PublicKey,
) -> Result<RemoteListing, String> {
    let (client, mut http_url) = safe_http_client(target_relay_url).await?;
    verify_relay_identity_with_client(&client, &http_url, target_relay_pubkey).await?;
    http_url.set_path("/query");
    http_url.set_query(None);
    let d = SingleLetterTag::lowercase(Alphabet::D);
    let filters = [Filter::new()
        .kind(Kind::Custom(KIND_MANAGED_AGENT as u16))
        .custom_tags(d, [agent_pubkey.to_hex()])
        .limit(10)];
    let body = serde_json::to_vec(&filters).map_err(|error| error.to_string())?;
    let authorization = buzz_core::http_auth::authorization_header(
        source_keys,
        "POST",
        http_url.as_str(),
        Some(&body),
    )?;
    let response = client
        .post(http_url)
        .header("Authorization", authorization)
        .header("Content-Type", "application/json")
        .header("x-buzz-relay-url", source_relay_url)
        .body(body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "remote marketplace query returned {}",
            response.status()
        ));
    }
    let bytes = bounded_body(response).await?;
    let events: Vec<Event> = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let event = events
        .into_iter()
        .max_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| right.id.cmp(&left.id))
        })
        .ok_or_else(|| "remote agent listing was not found".to_string())?;
    parse_listing_event(event, agent_pubkey)
}

/// Submit an already-signed event after pinning the target relay's NIP-11 identity.
pub async fn submit_remote_event(
    source_keys: &Keys,
    target_relay_url: &str,
    target_relay_pubkey: &PublicKey,
    event: &Event,
) -> Result<(), String> {
    let (client, mut http_url) = safe_http_client(target_relay_url).await?;
    verify_relay_identity_with_client(&client, &http_url, target_relay_pubkey).await?;
    http_url.set_path("/events");
    http_url.set_query(None);
    let body = serde_json::to_vec(event).map_err(|error| error.to_string())?;
    let authorization = buzz_core::http_auth::authorization_header(
        source_keys,
        "POST",
        http_url.as_str(),
        Some(&body),
    )?;
    let response = client
        .post(http_url)
        .header("Authorization", authorization)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "remote event submission returned {}",
            response.status()
        ));
    }
    Ok(())
}

/// Retry due durable request deliveries. Re-sending the same signed event is idempotent.
pub async fn retry_due_deliveries(state: &AppState) -> Result<usize, String> {
    let due = state
        .db
        .list_due_remote_agent_deliveries(100)
        .await
        .map_err(|error| error.to_string())?;
    for delivery in &due {
        let event_id = hex::decode(&delivery.prompt_event_id)
            .map_err(|error| format!("invalid stored request id: {error}"))?;
        let stored = state
            .db
            .get_event_by_id(delivery.community_id, &event_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "stored remote request event is missing".to_string())?;
        let relay_pubkey = PublicKey::from_slice(&delivery.agent_relay_pubkey)
            .map_err(|error| error.to_string())?;
        let result = submit_remote_event(
            &state.relay_keypair,
            &delivery.agent_relay_url,
            &relay_pubkey,
            &stored.event,
        )
        .await;
        state
            .db
            .record_remote_agent_delivery(
                delivery.community_id,
                &delivery.prompt_event_id,
                result.as_ref().err().map(String::as_str),
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(due.len())
}

/// Verify that `relay_url` serves a NIP-11 document whose `self` is `expected`.
pub async fn verify_relay_identity(
    relay_url: &str,
    expected: &PublicKey,
    local_relay_url: &str,
) -> Result<(), String> {
    if relay_url == local_relay_url {
        return Err("remote relay URL must not point to this relay".into());
    }
    let (client, http_url) = safe_http_client(relay_url).await?;
    verify_relay_identity_with_client(&client, &http_url, expected).await
}

async fn local_listing(
    state: &AppState,
    community_id: CommunityId,
    agent_pubkey: &PublicKey,
) -> Result<RemoteListing, String> {
    let (_, owner) = state
        .db
        .get_agent_channel_policy(community_id, agent_pubkey.as_bytes())
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "target agent is not registered in this community".to_string())?;
    let owner = owner.ok_or_else(|| "target agent has no verified owner".to_string())?;
    let mut query = buzz_db::event::EventQuery::for_community(community_id);
    query.kinds = Some(vec![KIND_MANAGED_AGENT as i32]);
    query.pubkey = Some(owner);
    query.d_tag = Some(agent_pubkey.to_hex());
    query.limit = Some(1);
    query.global_only = true;
    let event = state
        .db
        .query_events(&query)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| "target agent is not listed".to_string())?
        .event;
    parse_listing_event(event, agent_pubkey)
}

fn parse_listing_event(event: Event, agent_pubkey: &PublicKey) -> Result<RemoteListing, String> {
    let d_tags = event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("d") && values.len() == 2)
                .then(|| values[1].as_str())
        })
        .collect::<Vec<_>>();
    let expected_d = agent_pubkey.to_hex();
    if event.kind.as_u16() as u32 != KIND_MANAGED_AGENT
        || d_tags.as_slice() != [expected_d.as_str()]
    {
        return Err("remote listing coordinate does not match the target agent".into());
    }
    buzz_core::verify_event(&event)
        .map_err(|error| format!("invalid listing signature: {error}"))?;
    let content: serde_json::Value =
        serde_json::from_str(&event.content).map_err(|error| error.to_string())?;
    let marketplace = content
        .get("marketplace")
        .cloned()
        .ok_or_else(|| "listing is missing marketplace metadata".to_string())?;
    let marketplace = serde_json::from_value::<AgentMarketplace>(marketplace)
        .map_err(|error| error.to_string())?
        .normalized()?;
    if !marketplace.listed {
        return Err("agent is not currently listed".into());
    }
    Ok(RemoteListing {
        event_id: event.id.to_hex(),
        owner_pubkey: event.pubkey.to_bytes().to_vec(),
        marketplace,
    })
}

async fn safe_http_client(relay_url: &str) -> Result<(reqwest::Client, url::Url), String> {
    let mut url = url::Url::parse(relay_url).map_err(|error| error.to_string())?;
    if url.scheme() != "wss" || url.host_str().is_none() {
        return Err("relay URL must be an absolute wss URL".into());
    }
    url.set_scheme("https")
        .map_err(|_| "failed to convert relay URL to HTTPS".to_string())?;
    let host = url
        .host_str()
        .ok_or_else(|| "relay URL has no host".to_string())?
        .to_owned();
    let port = url.port_or_known_default().unwrap_or(443);
    let lookup_host = host.clone();
    let safe_ip = tokio::task::spawn_blocking(move || {
        buzz_core::network::resolve_public_host(&lookup_host, port)
    })
    .await
    .map_err(|error| error.to_string())??;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&host, std::net::SocketAddr::new(safe_ip, port))
        .build()
        .map_err(|error| error.to_string())?;
    Ok((client, url))
}

async fn verify_relay_identity_with_client(
    client: &reqwest::Client,
    http_url: &url::Url,
    expected: &PublicKey,
) -> Result<(), String> {
    let response = client
        .get(http_url.clone())
        .header(ACCEPT, "application/nostr+json")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("NIP-11 request returned {}", response.status()));
    }
    let bytes = bounded_body(response).await?;
    let info: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let actual = info
        .get("self")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "NIP-11 document is missing self".to_string())?;
    if actual != expected.to_hex() {
        return Err("NIP-11 self does not match the expected relay pubkey".into());
    }
    Ok(())
}

async fn bounded_body(mut response: reqwest::Response) -> Result<bytes::Bytes, String> {
    let mut body = bytes::BytesMut::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if body.len().saturating_add(chunk.len()) > MAX_HTTP_BODY_BYTES {
            return Err(format!(
                "remote relay response exceeds {MAX_HTTP_BODY_BYTES} bytes"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::marketplace::{AgentDeployment, RemoteInvocationPolicy};
    use nostr::{EventBuilder, Tag};

    #[test]
    fn listing_parser_keeps_exact_event_and_remote_policy() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let content = serde_json::json!({
            "name": "Reviewer",
            "marketplace": {
                "listed": true,
                "description": "Review",
                "capabilities": ["rust"],
                "deployment": "remote",
                "remote_invocation": { "policy": "any_community" }
            }
        });
        let event = EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), content.to_string())
            .tags([Tag::parse(["d", &agent.public_key().to_hex()]).expect("d tag")])
            .sign_with_keys(&owner)
            .expect("sign");
        let event_id = event.id.to_hex();
        let listing = parse_listing_event(event, &agent.public_key()).expect("parse");
        assert_eq!(listing.event_id, event_id);
        assert_eq!(listing.marketplace.deployment, AgentDeployment::Remote);
        assert_eq!(
            listing.marketplace.remote_invocation,
            Some(RemoteInvocationPolicy::AnyCommunity)
        );
    }

    #[test]
    fn request_rate_limit_rejects_the_sixty_first_request() {
        let counter = AtomicU32::new(0);
        for _ in 0..REQUESTS_PER_WINDOW {
            assert!(!counter_is_rate_limited(&counter));
        }
        assert!(counter_is_rate_limited(&counter));
    }
}
