//! Cross-community job polling, independent validation, and result delivery.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use buzz_core::agent_job::{
    build_accepted_event, build_progress_event, build_terminal_event, decrypt_request_event,
    validate_cancellation_event, validate_request_envelope, validate_response_envelope,
    JobRequestPayload, JobResultPayload, MAX_JOB_ERROR_BYTES, MAX_JOB_PLAINTEXT_BYTES,
};
use buzz_core::kind::{
    KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_ERROR, KIND_JOB_PROGRESS, KIND_JOB_REQUEST,
    KIND_JOB_RESULT, KIND_MANAGED_AGENT, KIND_STREAM_MESSAGE,
};
use buzz_core::marketplace::{AgentMarketplace, ReportedUsage};
use nostr::{
    Alphabet, Event, EventBuilder, EventId, Filter, Keys, Kind, PublicKey, SingleLetterTag, Tag,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::relay::RestClient;

const REMOTE_JOB_TAG: &str = "buzz:remote-job";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_LIMIT: usize = 500;
const REQUESTS_PER_RELAY_PER_MINUTE: usize = 60;

#[derive(Default)]
struct RemoteJobRateLimiter {
    callers: HashMap<PublicKey, (Instant, HashSet<String>)>,
}

impl RemoteJobRateLimiter {
    fn allow(&mut self, caller: PublicKey, request_id: &str, now: Instant) -> bool {
        self.callers
            .retain(|_, (started, _)| now.duration_since(*started) < Duration::from_secs(60));
        let (_, requests) = self
            .callers
            .entry(caller)
            .or_insert_with(|| (now, HashSet::new()));
        requests.len() < REQUESTS_PER_RELAY_PER_MINUTE && requests.insert(request_id.to_owned())
    }
}

/// Decrypted job converted into the existing channel queue contract.
pub struct PreparedRemoteJob {
    pub channel_id: Uuid,
    pub event: Event,
}

/// Durable poller updates consumed by the existing queue/control loop.
pub enum RemoteJobUpdate {
    /// Execute a newly accepted request.
    Execute(Box<PreparedRemoteJob>),
    /// Cancel queued or in-flight work for the request.
    Cancel { channel_id: Uuid },
}

/// Clear metadata carried only by an in-memory synthetic queue event.
pub struct RemoteJobContext {
    pub request_event_id: EventId,
    pub request_id: String,
    pub relay_url: String,
    pub relay_pubkey: PublicKey,
}

/// Poll the home relay for globally-scoped encrypted job requests.
pub fn spawn_poller(rest: RestClient) -> mpsc::Receiver<RemoteJobUpdate> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        let mut cancelled_requests = HashSet::new();
        let mut rate_limiter = RemoteJobRateLimiter::default();
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) =
                poll_once(&rest, &tx, &mut cancelled_requests, &mut rate_limiter).await
            {
                tracing::warn!(%error, "cross-community agent-job poll failed");
            }
            if tx.is_closed() {
                return;
            }
        }
    });
    rx
}

async fn poll_once(
    rest: &RestClient,
    tx: &mpsc::Sender<RemoteJobUpdate>,
    cancelled_requests: &mut HashSet<String>,
    rate_limiter: &mut RemoteJobRateLimiter,
) -> Result<(), String> {
    let agent_pubkey = rest.keys.public_key();
    let p = SingleLetterTag::lowercase(Alphabet::P);
    let requests = Filter::new()
        .kinds([
            Kind::Custom(KIND_JOB_REQUEST as u16),
            Kind::Custom(KIND_JOB_CANCEL as u16),
        ])
        .custom_tags(p, [agent_pubkey.to_hex()])
        .limit(POLL_LIMIT);
    let receipts = Filter::new()
        .kinds([
            Kind::Custom(KIND_JOB_ACCEPTED as u16),
            Kind::Custom(KIND_JOB_PROGRESS as u16),
            Kind::Custom(KIND_JOB_RESULT as u16),
            Kind::Custom(KIND_JOB_ERROR as u16),
        ])
        .author(agent_pubkey)
        .limit(POLL_LIMIT);
    let mut events: Vec<Event> = serde_json::from_value(
        rest.query(&[requests, receipts])
            .await
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let present_requests = events
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == KIND_JOB_REQUEST)
        .map(|event| event.id)
        .collect::<HashSet<_>>();
    let missing_requests = events
        .iter()
        .filter_map(referenced_request_event_id)
        .filter(|event_id| !present_requests.contains(event_id))
        .collect::<HashSet<_>>();
    if !missing_requests.is_empty() {
        let referenced_requests = Filter::new()
            .kind(Kind::Custom(KIND_JOB_REQUEST as u16))
            .ids(missing_requests.iter().copied())
            .custom_tags(p, [agent_pubkey.to_hex()])
            .limit(missing_requests.len());
        let older: Vec<Event> = serde_json::from_value(
            rest.query(&[referenced_requests])
                .await
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        events.extend(older);
    }

    let request_events: HashMap<String, Event> = events
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == KIND_JOB_REQUEST)
        .map(|event| (event.id.to_hex(), event.clone()))
        .collect();
    let mut accepted = HashSet::new();
    let mut delivered = HashSet::new();
    let mut terminal_requests = HashSet::new();
    let mut terminals = Vec::new();
    for event in &events {
        if event.kind.as_u16() as u32 != KIND_JOB_CANCEL {
            continue;
        }
        let Ok(cancellation) = validate_cancellation_event(event) else {
            continue;
        };
        let request_hex = cancellation.request_event_id.to_hex();
        let Some(request) = request_events.get(&request_hex) else {
            continue;
        };
        let Ok(envelope) = validate_request_envelope(request, request.created_at.as_secs()) else {
            continue;
        };
        if request.pubkey != event.pubkey
            || envelope.agent_pubkey != agent_pubkey
            || envelope.request_id != cancellation.request_id
        {
            continue;
        }
        if !cancelled_requests.insert(request_hex) {
            continue;
        }
        tx.send(RemoteJobUpdate::Cancel {
            channel_id: channel_id_for_request(request, &envelope.request_id),
        })
        .await
        .map_err(|_| "remote job queue is closed".to_string())?;
    }
    for event in events {
        match event.kind.as_u16() as u32 {
            KIND_JOB_ACCEPTED => {
                if let Ok(envelope) = validate_response_envelope(&event) {
                    accepted.insert(envelope.request_event_id.to_hex());
                }
            }
            KIND_JOB_PROGRESS if event.content == "delivered" => {
                if let Ok(envelope) = validate_response_envelope(&event) {
                    delivered.insert(envelope.request_event_id.to_hex());
                }
            }
            KIND_JOB_RESULT | KIND_JOB_ERROR => {
                if let Some(request_event_id) = referenced_request_event_id(&event) {
                    terminal_requests.insert(request_event_id.to_hex());
                }
                terminals.push(event);
            }
            _ => {}
        }
    }

    for terminal in terminals {
        let Ok(response) = validate_response_envelope(&terminal) else {
            continue;
        };
        let request_hex = response.request_event_id.to_hex();
        if delivered.contains(&request_hex) {
            continue;
        }
        if let Some(request) = request_events.get(&request_hex) {
            deliver_terminal(rest, request, &terminal).await?;
        }
    }

    for (request_hex, request) in request_events {
        if accepted.contains(&request_hex)
            || delivered.contains(&request_hex)
            || terminal_requests.contains(&request_hex)
            || cancelled_requests.contains(&request_hex)
        {
            continue;
        }
        let now = chrono::Utc::now().timestamp() as u64;
        let (envelope, payload) = match decrypt_request_event(&request, &rest.keys, now) {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!(event_id = %request.id, %error, "ignoring invalid or expired remote job");
                continue;
            }
        };
        if !rate_limiter.allow(envelope.relay_pubkey, &envelope.request_id, Instant::now()) {
            tracing::warn!(
                caller_relay = %envelope.relay_pubkey,
                request_id = %envelope.request_id,
                "remote agent-job request rate-limited"
            );
            continue;
        }
        verify_listing(rest, &envelope.relay_pubkey, &payload).await?;
        let _ = safe_remote_rest(&envelope.relay_url, &envelope.relay_pubkey, &rest.keys).await?;
        let accepted_event = build_accepted_event(
            &rest.keys,
            &envelope.relay_pubkey,
            &request.id,
            &envelope.request_id,
        )
        .map_err(|error| error.to_string())?;
        rest.submit_event(&accepted_event)
            .await
            .map_err(|error| error.to_string())?;
        accepted.insert(request_hex);
        tx.send(RemoteJobUpdate::Execute(Box::new(synthetic_job(
            &rest.keys, &request, &envelope, &payload,
        )?)))
        .await
        .map_err(|_| "remote job queue is closed".to_string())?;
    }
    Ok(())
}

fn referenced_request_event_id(event: &Event) -> Option<EventId> {
    match event.kind.as_u16() as u32 {
        KIND_JOB_CANCEL => validate_cancellation_event(event)
            .ok()
            .map(|envelope| envelope.request_event_id),
        KIND_JOB_ACCEPTED | KIND_JOB_PROGRESS | KIND_JOB_RESULT | KIND_JOB_ERROR => {
            validate_response_envelope(event)
                .ok()
                .map(|envelope| envelope.request_event_id)
        }
        _ => None,
    }
}

fn synthetic_job(
    keys: &Keys,
    request: &Event,
    envelope: &buzz_core::agent_job::JobRequestEnvelope,
    payload: &JobRequestPayload,
) -> Result<PreparedRemoteJob, String> {
    let channel_id = channel_id_for_request(request, &envelope.request_id);
    let tag = Tag::parse([
        REMOTE_JOB_TAG,
        &request.id.to_hex(),
        &envelope.request_id,
        &envelope.relay_url,
        &envelope.relay_pubkey.to_hex(),
    ])
    .map_err(|error| error.to_string())?;
    let content = format!(
        "[Cross-community job from {} for caller {}]\n\n{}",
        envelope.relay_pubkey.to_hex(),
        payload.caller_pubkey,
        payload.instruction
    );
    let event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), content)
        .tags([tag, Tag::public_key(keys.public_key())])
        .sign_with_keys(keys)
        .map_err(|error| error.to_string())?;
    Ok(PreparedRemoteJob { channel_id, event })
}

fn channel_id_for_request(request: &Event, request_id: &str) -> Uuid {
    Uuid::parse_str(request_id).unwrap_or_else(|_| {
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&request.id.to_bytes()[..16]);
        Uuid::from_bytes(bytes)
    })
}

pub fn context(event: &Event) -> Option<RemoteJobContext> {
    let values = event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some(REMOTE_JOB_TAG) && values.len() == 5)
            .then_some(values)
    })?;
    Some(RemoteJobContext {
        request_event_id: values[1].parse().ok()?,
        request_id: values[2].clone(),
        relay_url: values[3].clone(),
        relay_pubkey: PublicKey::from_hex(&values[4]).ok()?,
    })
}

/// Persist a terminal receipt at home, deliver it remotely, then mark delivery at home.
pub async fn post_result(
    rest: &RestClient,
    synthetic: &Event,
    output: String,
    usage: Option<ReportedUsage>,
) -> Result<(), String> {
    post_terminal(rest, synthetic, "completed", Some(output), None, usage).await
}

/// Persist and deliver a terminal failure without re-executing the accepted job.
pub async fn post_error(
    rest: &RestClient,
    synthetic: &Event,
    outcome: &str,
    error: String,
) -> Result<(), String> {
    post_terminal(rest, synthetic, outcome, None, Some(error), None).await
}

async fn post_terminal(
    rest: &RestClient,
    synthetic: &Event,
    outcome: &str,
    output: Option<String>,
    error: Option<String>,
    usage: Option<ReportedUsage>,
) -> Result<(), String> {
    let context = context(synthetic).ok_or_else(|| "missing remote job context".to_string())?;
    let payload = fit_result_payload(JobResultPayload {
        outcome: outcome.into(),
        output: output.map(|value| {
            if value.trim().is_empty() {
                "Done.".into()
            } else {
                value
            }
        }),
        error: error.map(|mut value| {
            truncate_utf8(&mut value, MAX_JOB_ERROR_BYTES);
            value
        }),
        completed_at: chrono::Utc::now().timestamp() as u64,
        // Advisory: an invalid self-report is dropped rather than failing an
        // otherwise-deliverable terminal result.
        usage: usage.and_then(|usage| match usage.normalized() {
            Ok(usage) => Some(usage),
            Err(error) => {
                tracing::warn!(%error, "dropping invalid self-reported usage from remote job result");
                None
            }
        }),
    })?;
    let terminal = build_terminal_event(
        &rest.keys,
        &context.relay_pubkey,
        &context.request_event_id,
        &context.request_id,
        &payload,
    )
    .map_err(|error| error.to_string())?;
    rest.submit_event(&terminal)
        .await
        .map_err(|error| error.to_string())?;
    deliver_to(
        rest,
        &context.relay_url,
        &context.relay_pubkey,
        &context.request_event_id,
        &context.request_id,
        &terminal,
    )
    .await
}

fn fit_result_payload(mut payload: JobResultPayload) -> Result<JobResultPayload, String> {
    loop {
        let size = serde_json::to_vec(&payload)
            .map_err(|error| error.to_string())?
            .len();
        if size <= MAX_JOB_PLAINTEXT_BYTES {
            return Ok(payload);
        }
        let value = payload
            .output
            .as_mut()
            .or(payload.error.as_mut())
            .ok_or_else(|| "remote result has no bounded text field".to_string())?;
        let next_len = value
            .len()
            .saturating_sub(size - MAX_JOB_PLAINTEXT_BYTES)
            .saturating_sub(16);
        if next_len >= value.len() {
            return Err("remote result cannot fit the plaintext limit".into());
        }
        truncate_utf8(value, next_len);
    }
}

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

async fn deliver_terminal(
    rest: &RestClient,
    request: &Event,
    terminal: &Event,
) -> Result<(), String> {
    let envelope = validate_request_envelope(request, request.created_at.as_secs())
        .map_err(|error| error.to_string())?;
    deliver_to(
        rest,
        &envelope.relay_url,
        &envelope.relay_pubkey,
        &request.id,
        &envelope.request_id,
        terminal,
    )
    .await
}

async fn deliver_to(
    rest: &RestClient,
    relay_url: &str,
    relay_pubkey: &PublicKey,
    request_event_id: &EventId,
    request_id: &str,
    terminal: &Event,
) -> Result<(), String> {
    let remote = safe_remote_rest(relay_url, relay_pubkey, &rest.keys).await?;
    remote
        .submit_event(terminal)
        .await
        .map_err(|error| error.to_string())?;
    let delivered = build_progress_event(
        &rest.keys,
        relay_pubkey,
        request_event_id,
        request_id,
        "delivered",
    )
    .map_err(|error| error.to_string())?;
    rest.submit_event(&delivered)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn verify_listing(
    rest: &RestClient,
    source_relay: &PublicKey,
    payload: &JobRequestPayload,
) -> Result<(), String> {
    let d = SingleLetterTag::lowercase(Alphabet::D);
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_MANAGED_AGENT as u16))
        .custom_tags(d, [rest.keys.public_key().to_hex()])
        .limit(1);
    let events: Vec<Event> = serde_json::from_value(
        rest.query(&[filter])
            .await
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let listing = events
        .into_iter()
        .find(|event| event.id.to_hex() == payload.listing_event_id)
        .ok_or_else(|| "remote request listing is no longer current".to_string())?;
    let marketplace = validate_listing(&listing, &rest.keys.public_key())?;
    let policy = marketplace
        .remote_invocation
        .ok_or_else(|| "remote invocation is disabled".to_string())?;
    if !policy.allows(&source_relay.to_hex()) {
        return Err("source relay is not allowed by the listing".into());
    }
    Ok(())
}

fn validate_listing(listing: &Event, agent_pubkey: &PublicKey) -> Result<AgentMarketplace, String> {
    buzz_core::verify_event(listing).map_err(|error| format!("invalid listing: {error}"))?;
    let expected_d = agent_pubkey.to_hex();
    let d_tags = listing
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("d") && values.len() == 2)
                .then(|| values[1].as_str())
        })
        .collect::<Vec<_>>();
    if listing.kind.as_u16() as u32 != KIND_MANAGED_AGENT
        || d_tags.as_slice() != [expected_d.as_str()]
    {
        return Err("listing coordinate does not match this agent".into());
    }
    let content: serde_json::Value =
        serde_json::from_str(&listing.content).map_err(|error| error.to_string())?;
    let marketplace = serde_json::from_value::<AgentMarketplace>(
        content
            .get("marketplace")
            .cloned()
            .ok_or_else(|| "listing has no marketplace policy".to_string())?,
    )
    .map_err(|error| error.to_string())?
    .normalized()?;
    if !marketplace.listed {
        return Err("agent is not currently listed".into());
    }
    Ok(marketplace)
}

async fn safe_remote_rest(
    url: &str,
    expected: &PublicKey,
    keys: &Keys,
) -> Result<RestClient, String> {
    let mut parsed = url::Url::parse(url).map_err(|error| error.to_string())?;
    if parsed.scheme() != "wss"
        || parsed.host_str().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
    {
        return Err("remote relay must be an absolute wss URL without userinfo".into());
    }
    parsed
        .set_scheme("https")
        .map_err(|_| "invalid relay scheme".to_string())?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "missing relay host".to_string())?
        .to_owned();
    let port = parsed.port_or_known_default().unwrap_or(443);
    let lookup_host = host.clone();
    let ip = tokio::task::spawn_blocking(move || {
        buzz_core::network::resolve_public_host(&lookup_host, port)
    })
    .await
    .map_err(|error| error.to_string())??;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&host, std::net::SocketAddr::new(ip, port))
        .build()
        .map_err(|error| error.to_string())?;
    let client = RestClient {
        http,
        base_url: parsed.as_str().trim_end_matches('/').to_string(),
        keys: keys.clone(),
        auth_tag_json: None,
    };
    if client
        .fetch_relay_pubkey()
        .await
        .map_err(|error| error.to_string())?
        != *expected
    {
        return Err("remote NIP-11 self does not match its relay coordinate".into());
    }
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_utf8_result_is_bounded_without_splitting_characters() {
        let payload = fit_result_payload(JobResultPayload {
            outcome: "completed".into(),
            output: Some("🐝\n".repeat(MAX_JOB_PLAINTEXT_BYTES)),
            error: None,
            completed_at: 1,
            usage: None,
        })
        .expect("fit result");
        assert!(serde_json::to_vec(&payload).expect("serialize").len() <= MAX_JOB_PLAINTEXT_BYTES);
        assert!(payload
            .output
            .expect("output")
            .chars()
            .all(|value| value == '🐝' || value == '\n'));
    }

    #[test]
    fn rate_limit_counts_unique_request_ids_per_caller_relay() {
        let caller = Keys::generate().public_key();
        let started = Instant::now();
        let mut limiter = RemoteJobRateLimiter::default();
        assert!(limiter.allow(caller, "request-0", started));
        assert!(!limiter.allow(caller, "request-0", started));
        for index in 1..REQUESTS_PER_RELAY_PER_MINUTE {
            assert!(limiter.allow(caller, &format!("request-{index}"), started));
        }
        assert!(!limiter.allow(caller, "request-over-limit", started));
        assert!(limiter.allow(
            caller,
            "request-next-window",
            started + Duration::from_secs(60)
        ));
    }

    #[test]
    fn harness_rejects_an_unlisted_remote_policy() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let listing = EventBuilder::new(
            Kind::Custom(KIND_MANAGED_AGENT as u16),
            serde_json::json!({
                "marketplace": {
                    "listed": false,
                    "description": "Reviewer",
                    "capabilities": [],
                    "deployment": "remote",
                    "remote_invocation": { "policy": "any_community" }
                }
            })
            .to_string(),
        )
        .tags([Tag::parse(["d", &agent.public_key().to_hex()]).expect("d tag")])
        .sign_with_keys(&owner)
        .expect("sign listing");
        assert!(validate_listing(&listing, &agent.public_key()).is_err());
    }

    #[test]
    fn terminal_and_cancellation_receipts_recover_their_request_id() {
        let relay = Keys::generate();
        let agent = Keys::generate();
        let request_event_id = EventId::all_zeros();
        let terminal =
            build_accepted_event(&agent, &relay.public_key(), &request_event_id, "request-1")
                .expect("accepted receipt");
        let cancellation = buzz_core::agent_job::build_cancellation_event(
            &relay,
            &agent.public_key(),
            &request_event_id,
            "request-1",
        )
        .expect("cancellation");
        assert_eq!(
            referenced_request_event_id(&terminal),
            Some(request_event_id)
        );
        assert_eq!(
            referenced_request_event_id(&cancellation),
            Some(request_event_id)
        );
    }
}
