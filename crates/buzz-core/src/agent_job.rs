//! Cross-community encrypted agent-job wire contract.

use nostr::{Event, EventBuilder, EventId, Keys, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::kind::{
    event_kind_u32, KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_ERROR, KIND_JOB_PROGRESS,
    KIND_JOB_REQUEST, KIND_JOB_RESULT,
};
use crate::observer::{
    content_looks_like_nip44, decrypt_observer_payload, encrypt_observer_payload,
    ObserverPayloadError, OBSERVER_MAX_PLAINTEXT_LEN,
};

/// Maximum encrypted request or response plaintext size in UTF-8 bytes.
pub const MAX_JOB_PLAINTEXT_BYTES: usize = OBSERVER_MAX_PLAINTEXT_LEN;
/// Maximum error text retained in a failure payload.
pub const MAX_JOB_ERROR_BYTES: usize = 4_096;
/// Maximum instruction text accepted in a request.
pub const MAX_JOB_INSTRUCTION_BYTES: usize = 60_000;

/// Cleartext routing fields on a job request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRequestEnvelope {
    /// Target agent identity.
    pub agent_pubkey: PublicKey,
    /// Random caller-generated correlation id.
    pub request_id: String,
    /// Unix-second request expiry.
    pub expiration: u64,
    /// Caller community relay URL.
    pub relay_url: String,
    /// Caller community NIP-11 `self` pubkey.
    pub relay_pubkey: PublicKey,
}

/// Cleartext routing fields on a job response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobResponseEnvelope {
    /// Target caller relay identity.
    pub relay_pubkey: PublicKey,
    /// Request event being answered.
    pub request_event_id: EventId,
    /// Caller-generated correlation id copied from the request.
    pub request_id: String,
}

/// Cleartext routing fields on a caller-relay cancellation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobCancellationEnvelope {
    /// Target agent identity.
    pub agent_pubkey: PublicKey,
    /// Request event being cancelled.
    pub request_event_id: EventId,
    /// Caller-generated request id.
    pub request_id: String,
}

/// NIP-44 request body visible only to the target agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRequestPayload {
    /// Instruction to execute.
    pub instruction: String,
    /// Human who initiated the workflow or direct-use request.
    pub caller_pubkey: String,
    /// Workflow definition id.
    pub workflow_id: String,
    /// Workflow run id.
    pub run_id: String,
    /// Workflow step id.
    pub step_id: String,
    /// Destination channel in the caller community.
    pub channel_id: String,
    /// Exact marketplace listing used at dispatch.
    pub listing_event_id: String,
}

/// Terminal result body visible only to the caller relay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobResultPayload {
    /// Terminal outcome (`completed`, `failed`, or `cancelled`).
    pub outcome: String,
    /// Agent output on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Bounded failure text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Unix-second completion time.
    pub completed_at: u64,
}

/// Errors returned by job event helpers.
#[derive(Debug, Error)]
pub enum JobError {
    /// Event kind is not valid for this operation.
    #[error("invalid job kind: {0}")]
    InvalidKind(u32),
    /// A required tag is missing, duplicated, or malformed.
    #[error("invalid job tag: {0}")]
    InvalidTag(String),
    /// A payload field is invalid.
    #[error("invalid job payload: {0}")]
    InvalidPayload(String),
    /// NIP-44 or JSON processing failed.
    #[error(transparent)]
    Payload(#[from] ObserverPayloadError),
    /// Event signing failed.
    #[error("failed to sign job event: {0}")]
    Sign(String),
}

/// Return whether `kind` belongs to the agent-job event family.
pub fn is_job_kind(kind: u32) -> bool {
    (KIND_JOB_REQUEST..=KIND_JOB_ERROR).contains(&kind)
}

/// Validate a request envelope without decrypting its payload.
pub fn validate_request_envelope(event: &Event, now: u64) -> Result<JobRequestEnvelope, JobError> {
    if event_kind_u32(event) != KIND_JOB_REQUEST {
        return Err(JobError::InvalidKind(event_kind_u32(event)));
    }
    if !content_looks_like_nip44(&event.content) {
        return Err(JobError::InvalidPayload(
            "content must be NIP-44 v2 ciphertext".into(),
        ));
    }
    let agent_pubkey = parse_pubkey_tag(event, "p")?;
    let request_id = single_tag(event, "request")?;
    validate_request_id(&request_id)?;
    let expiration = single_tag(event, "expiration")?
        .parse::<u64>()
        .map_err(|_| JobError::InvalidTag("expiration must be unix seconds".into()))?;
    if expiration <= now {
        return Err(JobError::InvalidPayload("request is expired".into()));
    }
    let relay_url = single_tag(event, "relay")?;
    validate_wss_url(&relay_url)?;
    let relay_pubkey = parse_pubkey_tag(event, "relay-pubkey")?;
    if relay_pubkey != event.pubkey {
        return Err(JobError::InvalidTag(
            "relay-pubkey must equal the request author".into(),
        ));
    }
    Ok(JobRequestEnvelope {
        agent_pubkey,
        request_id,
        expiration,
        relay_url,
        relay_pubkey,
    })
}

/// Validate a response envelope without decrypting its payload.
pub fn validate_response_envelope(event: &Event) -> Result<JobResponseEnvelope, JobError> {
    let kind = event_kind_u32(event);
    if !matches!(
        kind,
        KIND_JOB_ACCEPTED | KIND_JOB_PROGRESS | KIND_JOB_RESULT | KIND_JOB_ERROR
    ) {
        return Err(JobError::InvalidKind(kind));
    }
    if matches!(kind, KIND_JOB_RESULT | KIND_JOB_ERROR) && !content_looks_like_nip44(&event.content)
    {
        return Err(JobError::InvalidPayload(
            "terminal response content must be NIP-44 v2 ciphertext".into(),
        ));
    }
    let relay_pubkey = parse_pubkey_tag(event, "p")?;
    let request_event_id = single_tag(event, "e")?
        .parse::<EventId>()
        .map_err(|_| JobError::InvalidTag("e must be a 64-character event id".into()))?;
    let request_id = single_tag(event, "request")?;
    validate_request_id(&request_id)?;
    Ok(JobResponseEnvelope {
        relay_pubkey,
        request_event_id,
        request_id,
    })
}

/// Validate a caller-relay cancellation without loading its request.
pub fn validate_cancellation_event(event: &Event) -> Result<JobCancellationEnvelope, JobError> {
    if event_kind_u32(event) != KIND_JOB_CANCEL {
        return Err(JobError::InvalidKind(event_kind_u32(event)));
    }
    if !event.content.is_empty() {
        return Err(JobError::InvalidPayload(
            "cancellation content must be empty".into(),
        ));
    }
    let agent_pubkey = parse_pubkey_tag(event, "p")?;
    let request_event_id = single_tag(event, "e")?
        .parse::<EventId>()
        .map_err(|_| JobError::InvalidTag("e must be a 64-character event id".into()))?;
    let request_id = single_tag(event, "request")?;
    validate_request_id(&request_id)?;
    Ok(JobCancellationEnvelope {
        agent_pubkey,
        request_event_id,
        request_id,
    })
}

/// Encrypt and sign a job request.
pub fn build_request_event(
    relay_keys: &Keys,
    agent_pubkey: &PublicKey,
    request_id: &str,
    expiration: u64,
    relay_url: &str,
    payload: &JobRequestPayload,
) -> Result<Event, JobError> {
    validate_request_id(request_id)?;
    validate_wss_url(relay_url)?;
    validate_request_payload(payload)?;
    let content = encrypt_observer_payload(relay_keys, agent_pubkey, payload)?;
    let expiration = expiration.to_string();
    EventBuilder::new(Kind::Custom(KIND_JOB_REQUEST as u16), content)
        .tags([
            Tag::public_key(*agent_pubkey),
            parse_tag(["request", request_id])?,
            parse_tag(["expiration", &expiration])?,
            parse_tag(["relay", relay_url])?,
            parse_tag(["relay-pubkey", &relay_keys.public_key().to_hex()])?,
        ])
        .sign_with_keys(relay_keys)
        .map_err(|error| JobError::Sign(error.to_string()))
}

/// Validate and decrypt a request for `agent_keys`.
pub fn decrypt_request_event(
    event: &Event,
    agent_keys: &Keys,
    now: u64,
) -> Result<(JobRequestEnvelope, JobRequestPayload), JobError> {
    let envelope = validate_request_envelope(event, now)?;
    if envelope.agent_pubkey != agent_keys.public_key() {
        return Err(JobError::InvalidPayload(
            "request recipient does not match agent".into(),
        ));
    }
    let payload = decrypt_observer_payload(agent_keys, event)?;
    validate_request_payload(&payload)?;
    Ok((envelope, payload))
}

/// Sign an empty acceptance receipt for durable at-most-once execution.
pub fn build_accepted_event(
    agent_keys: &Keys,
    relay_pubkey: &PublicKey,
    request_event_id: &EventId,
    request_id: &str,
) -> Result<Event, JobError> {
    build_status_event(
        agent_keys,
        KIND_JOB_ACCEPTED,
        relay_pubkey,
        request_event_id,
        request_id,
        "",
    )
}

/// Sign a bounded progress receipt such as local result-delivery state.
pub fn build_progress_event(
    agent_keys: &Keys,
    relay_pubkey: &PublicKey,
    request_event_id: &EventId,
    request_id: &str,
    progress: &str,
) -> Result<Event, JobError> {
    if progress.len() > 256 {
        return Err(JobError::InvalidPayload(
            "progress text exceeds 256 bytes".into(),
        ));
    }
    build_status_event(
        agent_keys,
        crate::kind::KIND_JOB_PROGRESS,
        relay_pubkey,
        request_event_id,
        request_id,
        progress,
    )
}

/// Sign a cancellation addressed to the original target agent.
pub fn build_cancellation_event(
    relay_keys: &Keys,
    agent_pubkey: &PublicKey,
    request_event_id: &EventId,
    request_id: &str,
) -> Result<Event, JobError> {
    validate_request_id(request_id)?;
    EventBuilder::new(Kind::Custom(KIND_JOB_CANCEL as u16), "")
        .tags([
            Tag::public_key(*agent_pubkey),
            Tag::event(*request_event_id),
            parse_tag(["request", request_id])?,
        ])
        .sign_with_keys(relay_keys)
        .map_err(|error| JobError::Sign(error.to_string()))
}

fn build_status_event(
    agent_keys: &Keys,
    kind: u32,
    relay_pubkey: &PublicKey,
    request_event_id: &EventId,
    request_id: &str,
    content: &str,
) -> Result<Event, JobError> {
    validate_request_id(request_id)?;
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags([
            Tag::public_key(*relay_pubkey),
            Tag::event(*request_event_id),
            parse_tag(["request", request_id])?,
        ])
        .sign_with_keys(agent_keys)
        .map_err(|error| JobError::Sign(error.to_string()))
}

/// Encrypt and sign a terminal job result or failure.
pub fn build_terminal_event(
    agent_keys: &Keys,
    relay_pubkey: &PublicKey,
    request_event_id: &EventId,
    request_id: &str,
    payload: &JobResultPayload,
) -> Result<Event, JobError> {
    validate_request_id(request_id)?;
    validate_result_payload(payload)?;
    let kind = match payload.outcome.as_str() {
        "completed" => KIND_JOB_RESULT,
        "failed" | "cancelled" => KIND_JOB_ERROR,
        _ => {
            return Err(JobError::InvalidPayload(
                "outcome must be completed, failed, or cancelled".into(),
            ));
        }
    };
    let content = encrypt_observer_payload(agent_keys, relay_pubkey, payload)?;
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags([
            Tag::public_key(*relay_pubkey),
            Tag::event(*request_event_id),
            parse_tag(["request", request_id])?,
        ])
        .sign_with_keys(agent_keys)
        .map_err(|error| JobError::Sign(error.to_string()))
}

/// Validate and decrypt a terminal response for `relay_keys`.
pub fn decrypt_terminal_event(
    event: &Event,
    relay_keys: &Keys,
) -> Result<(JobResponseEnvelope, JobResultPayload), JobError> {
    let envelope = validate_response_envelope(event)?;
    if envelope.relay_pubkey != relay_keys.public_key() {
        return Err(JobError::InvalidPayload(
            "response recipient does not match relay".into(),
        ));
    }
    let payload = decrypt_observer_payload(relay_keys, event)?;
    validate_result_payload(&payload)?;
    Ok((envelope, payload))
}

fn validate_request_payload(payload: &JobRequestPayload) -> Result<(), JobError> {
    if payload.instruction.trim().is_empty()
        || payload.instruction.len() > MAX_JOB_INSTRUCTION_BYTES
    {
        return Err(JobError::InvalidPayload(format!(
            "instruction must contain 1–{MAX_JOB_INSTRUCTION_BYTES} bytes"
        )));
    }
    validate_hex(&payload.caller_pubkey, "caller_pubkey")?;
    validate_uuid(&payload.workflow_id, "workflow_id")?;
    validate_uuid(&payload.run_id, "run_id")?;
    validate_uuid(&payload.channel_id, "channel_id")?;
    if payload.step_id.is_empty() || payload.step_id.len() > 64 {
        return Err(JobError::InvalidPayload(
            "step_id must contain 1–64 bytes".into(),
        ));
    }
    validate_hex(&payload.listing_event_id, "listing_event_id")?;
    validate_serialized_size(payload)
}

fn validate_result_payload(payload: &JobResultPayload) -> Result<(), JobError> {
    match payload.outcome.as_str() {
        "completed" if payload.output.is_some() && payload.error.is_none() => {}
        "failed" | "cancelled" if payload.output.is_none() && payload.error.is_some() => {}
        _ => {
            return Err(JobError::InvalidPayload(
                "completed requires output; failed/cancelled require error".into(),
            ));
        }
    }
    if payload
        .error
        .as_ref()
        .is_some_and(|error| error.len() > MAX_JOB_ERROR_BYTES)
    {
        return Err(JobError::InvalidPayload(format!(
            "error exceeds {MAX_JOB_ERROR_BYTES} bytes"
        )));
    }
    validate_serialized_size(payload)
}

fn validate_serialized_size<T: Serialize>(value: &T) -> Result<(), JobError> {
    let size = serde_json::to_vec(value)
        .map_err(|error| JobError::InvalidPayload(error.to_string()))?
        .len();
    if size > MAX_JOB_PLAINTEXT_BYTES {
        return Err(JobError::InvalidPayload(format!(
            "plaintext exceeds {MAX_JOB_PLAINTEXT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_request_id(value: &str) -> Result<(), JobError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(JobError::InvalidTag(
            "request must contain 1–64 URL-safe characters".into(),
        ));
    }
    Ok(())
}

fn validate_wss_url(value: &str) -> Result<(), JobError> {
    let url = url::Url::parse(value)
        .map_err(|_| JobError::InvalidTag("relay must be an absolute wss URL".into()))?;
    if url.scheme() != "wss"
        || url.host_str().is_none()
        || url.username() != ""
        || url.password().is_some()
    {
        return Err(JobError::InvalidTag(
            "relay must be an absolute wss URL without userinfo".into(),
        ));
    }
    Ok(())
}

fn validate_hex(value: &str, field: &str) -> Result<(), JobError> {
    if value.len() != 64
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(JobError::InvalidPayload(format!(
            "{field} must be 64 lowercase hex characters"
        )));
    }
    Ok(())
}

fn validate_uuid(value: &str, field: &str) -> Result<(), JobError> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| JobError::InvalidPayload(format!("{field} must be a UUID")))
}

fn single_tag(event: &Event, name: &str) -> Result<String, JobError> {
    let mut values = event.tags.iter().filter_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some(name) && values.len() == 2)
            .then(|| values[1].clone())
    });
    let value = values
        .next()
        .ok_or_else(|| JobError::InvalidTag(format!("missing {name}")))?;
    if values.next().is_some() {
        return Err(JobError::InvalidTag(format!("duplicate {name}")));
    }
    Ok(value)
}

fn parse_pubkey_tag(event: &Event, name: &str) -> Result<PublicKey, JobError> {
    let value = single_tag(event, name)?;
    PublicKey::from_hex(&value)
        .map_err(|_| JobError::InvalidTag(format!("{name} must be a 64-character pubkey")))
}

fn parse_tag<const N: usize>(values: [&str; N]) -> Result<Tag, JobError> {
    Tag::parse(values).map_err(|error| JobError::InvalidTag(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> JobRequestPayload {
        JobRequestPayload {
            instruction: "Review the patch".into(),
            caller_pubkey: "a".repeat(64),
            workflow_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            step_id: "review".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
            listing_event_id: "b".repeat(64),
        }
    }

    #[test]
    fn request_and_result_round_trip() {
        let relay = Keys::generate();
        let agent = Keys::generate();
        let now = nostr::Timestamp::now().as_secs();
        let request = build_request_event(
            &relay,
            &agent.public_key(),
            "req-1",
            now + 60,
            "wss://caller.example",
            &payload(),
        )
        .expect("build request");
        let (envelope, decoded) =
            decrypt_request_event(&request, &agent, now).expect("decrypt request");
        assert_eq!(decoded.instruction, "Review the patch");
        assert_eq!(envelope.relay_pubkey, relay.public_key());

        let result = build_terminal_event(
            &agent,
            &relay.public_key(),
            &request.id,
            &envelope.request_id,
            &JobResultPayload {
                outcome: "completed".into(),
                output: Some("Looks good".into()),
                error: None,
                completed_at: now + 1,
            },
        )
        .expect("build result");
        let (_, decoded) = decrypt_terminal_event(&result, &relay).expect("decrypt result");
        assert_eq!(decoded.output.as_deref(), Some("Looks good"));
    }

    #[test]
    fn request_rejects_expiry_wrong_recipient_and_insecure_relay() {
        let relay = Keys::generate();
        let agent = Keys::generate();
        let other = Keys::generate();
        let now = nostr::Timestamp::now().as_secs();
        assert!(build_request_event(
            &relay,
            &agent.public_key(),
            "req-1",
            now + 60,
            "ws://caller.example",
            &payload(),
        )
        .is_err());
        let request = build_request_event(
            &relay,
            &agent.public_key(),
            "req-1",
            now + 1,
            "wss://caller.example",
            &payload(),
        )
        .expect("build request");
        assert!(decrypt_request_event(&request, &agent, now + 1).is_err());
        assert!(decrypt_request_event(&request, &other, now).is_err());
    }

    #[test]
    fn cancellation_round_trip_targets_the_original_request() {
        let relay = Keys::generate();
        let agent = Keys::generate();
        let request_id = "request-1";
        let request_event_id = EventId::all_zeros();
        let event =
            build_cancellation_event(&relay, &agent.public_key(), &request_event_id, request_id)
                .expect("build cancellation");
        let envelope = validate_cancellation_event(&event).expect("validate cancellation");
        assert_eq!(envelope.agent_pubkey, agent.public_key());
        assert_eq!(envelope.request_event_id, request_event_id);
        assert_eq!(envelope.request_id, request_id);
        assert!(validate_response_envelope(&event).is_err());
    }
}
