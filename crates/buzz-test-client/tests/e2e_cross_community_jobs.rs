//! Relay-level E2E tests for the cross-community agent-job event family
//! (kinds 43001–43006, BUZZ-10).
//!
//! These cover the relay's *boundary* obligations from
//! `docs/features/cross-community-agent-invocation.md`:
//!
//! - forged / expired / malformed job requests are rejected at ingest;
//! - a member cannot submit a 43001/43005 without verified cross-community
//!   relay authorization (WS traffic is never cross-community);
//! - a non-member's HTTP-submitted request, terminal response, or
//!   cancellation is rejected unless it matches an admitted pending
//!   assignment;
//! - job events are readable only by their signed author or `p` recipient,
//!   including through kindless event-id queries.
//!
//! The *acceptance* half of the flow (a listed remote-enabled agent's home
//! relay admitting a request from community B, duplicate-result idempotency,
//! cancel-beats-late-result) requires two live relays with NIP-11
//! identities the harness can verify. It is deliberately not faked here;
//! see the doc's acceptance scenarios 1, 3, 4, and 6.
//!
//! # Running
//!
//! Start the relay (`just relay`), then:
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_cross_community_jobs -- --ignored
//! ```

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use buzz_core::agent_job::{
    build_cancellation_event, build_request_event, build_terminal_event, JobRequestPayload,
    JobResultPayload,
};
use buzz_core::kind::{KIND_JOB_ACCEPTED, KIND_JOB_REQUEST};
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, EventId, Filter, Keys, Kind, Tag};
use sha2::{Digest, Sha256};

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn sub_id(name: &str) -> String {
    format!("e2e-jobs-{name}-{}", uuid::Uuid::new_v4())
}

fn future_expiry() -> u64 {
    (chrono::Utc::now().timestamp() as u64) + 600
}

fn request_payload() -> JobRequestPayload {
    JobRequestPayload {
        instruction: "review the release notes".into(),
        caller_pubkey: Keys::generate().public_key().to_hex(),
        workflow_id: uuid::Uuid::new_v4().to_string(),
        run_id: uuid::Uuid::new_v4().to_string(),
        step_id: "step-e2e".into(),
        channel_id: uuid::Uuid::new_v4().to_string(),
        listing_event_id: EventId::all_zeros().to_hex(),
    }
}

/// NIP-98 Authorization header for `POST /events`, mirroring the desktop's
/// cross-community submit path.
fn nip98_post_header(keys: &Keys, url: &str, body: &str) -> String {
    let payload_hash = hex::encode(Sha256::digest(body.as_bytes()));
    let event = EventBuilder::new(Kind::Custom(27_235), "")
        .tags(vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", "POST"]).expect("method tag"),
            Tag::parse(["payload", &payload_hash]).expect("payload tag"),
        ])
        .sign_with_keys(keys)
        .expect("sign nip98");
    let json = serde_json::to_vec(&event).expect("serialize nip98");
    format!("Nostr {}", BASE64.encode(json))
}

async fn post_event(keys: &Keys, event: &nostr::Event) -> (u16, String) {
    let url = format!("{}/events", relay_http_url());
    let body = serde_json::to_string(event).expect("serialize event");
    let response = reqwest::Client::new()
        .post(&url)
        .header("Authorization", nip98_post_header(keys, &url, &body))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST /events");
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    (status, text)
}

/// A member's WebSocket-submitted job request must be refused: WS sessions
/// are never cross-community-authorized, whatever the envelope claims.
#[tokio::test]
#[ignore]
async fn ws_job_request_requires_cross_community_authorization() {
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");

    let agent = Keys::generate().public_key();
    let event = build_request_event(
        &keys,
        &agent,
        "req-ws-gate",
        future_expiry(),
        "wss://other-community.example",
        &request_payload(),
    )
    .expect("build request");

    let ok = client.send_event(event).await.expect("OK frame");
    assert!(!ok.accepted, "WS job request must be rejected");
    assert!(
        ok.message.to_lowercase().contains("restricted"),
        "expected cross-community auth refusal, got: {}",
        ok.message
    );
    client.disconnect().await.ok();
}

/// Envelope validation runs before the authorization gate: expired requests,
/// author/relay-pubkey mismatches, and non-wss relay hints are all invalid.
#[tokio::test]
#[ignore]
async fn ws_job_request_envelope_validation_rejects_forgeries() {
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");
    let agent = Keys::generate().public_key();

    // Expired request.
    let expired = build_request_event(
        &keys,
        &agent,
        "req-expired",
        (chrono::Utc::now().timestamp() as u64).saturating_sub(60),
        "wss://other-community.example",
        &request_payload(),
    )
    .expect("build expired request");
    let ok = client.send_event(expired).await.expect("OK frame");
    assert!(!ok.accepted, "expired request must be rejected");
    assert!(
        ok.message.to_lowercase().contains("invalid"),
        "expected invalid, got: {}",
        ok.message
    );

    // relay-pubkey that does not match the signing author (a forged origin).
    let other = Keys::generate();
    let forged = EventBuilder::new(
        Kind::Custom(KIND_JOB_REQUEST as u16),
        // Envelope shape check only requires NIP-44-looking content; reuse a
        // genuinely encrypted body from the builder for realism.
        build_request_event(
            &keys,
            &agent,
            "req-forged",
            future_expiry(),
            "wss://other-community.example",
            &request_payload(),
        )
        .expect("inner request")
        .content
        .clone(),
    )
    .tags([
        Tag::public_key(agent),
        Tag::parse(["request", "req-forged"]).expect("request tag"),
        Tag::parse(["expiration", &future_expiry().to_string()]).expect("expiration tag"),
        Tag::parse(["relay", "wss://other-community.example"]).expect("relay tag"),
        Tag::parse(["relay-pubkey", &other.public_key().to_hex()]).expect("relay-pubkey tag"),
    ])
    .sign_with_keys(&keys)
    .expect("sign forged request");
    let ok = client.send_event(forged).await.expect("OK frame");
    assert!(!ok.accepted, "forged relay-pubkey must be rejected");

    // Non-wss relay routing hint.
    let plaintext_relay = EventBuilder::new(
        Kind::Custom(KIND_JOB_REQUEST as u16),
        build_request_event(
            &keys,
            &agent,
            "req-plain",
            future_expiry(),
            "wss://other-community.example",
            &request_payload(),
        )
        .expect("inner request")
        .content
        .clone(),
    )
    .tags([
        Tag::public_key(agent),
        Tag::parse(["request", "req-plain"]).expect("request tag"),
        Tag::parse(["expiration", &future_expiry().to_string()]).expect("expiration tag"),
        Tag::parse(["relay", "ws://other-community.example"]).expect("relay tag"),
        Tag::parse(["relay-pubkey", &keys.public_key().to_hex()]).expect("relay-pubkey tag"),
    ])
    .sign_with_keys(&keys)
    .expect("sign plaintext-relay request");
    let ok = client.send_event(plaintext_relay).await.expect("OK frame");
    assert!(!ok.accepted, "non-wss relay hint must be rejected");

    client.disconnect().await.ok();
}

/// A non-member relay identity POSTing a job request for an agent that is
/// not listed here must get 403, not membership-bypass acceptance.
#[tokio::test]
#[ignore]
async fn http_job_request_for_unlisted_agent_rejected() {
    let foreign_relay = Keys::generate();
    let unlisted_agent = Keys::generate().public_key();
    let event = build_request_event(
        &foreign_relay,
        &unlisted_agent,
        "req-unlisted",
        future_expiry(),
        "wss://caller-community.example",
        &request_payload(),
    )
    .expect("build request");

    let (status, body) = post_event(&foreign_relay, &event).await;
    assert_eq!(status, 403, "expected 403, got {status}: {body}");
}

/// A non-member terminal result that matches no pending assignment must be
/// rejected — this is the doc's "wrong-origin responses stay forbidden".
#[tokio::test]
#[ignore]
async fn http_terminal_response_without_pending_assignment_rejected() {
    let agent = Keys::generate();
    let payload = JobResultPayload {
        outcome: "completed".into(),
        output: Some("done".into()),
        error: None,
        completed_at: chrono::Utc::now().timestamp() as u64,
        usage: None,
    };
    let event = build_terminal_event(
        &agent,
        &Keys::generate().public_key(), // arbitrary target relay identity
        &EventId::all_zeros(),
        "req-orphan-result",
        &payload,
    )
    .expect("build terminal");

    let (status, body) = post_event(&agent, &event).await;
    assert_eq!(status, 403, "expected 403, got {status}: {body}");
}

/// A non-member cancellation for a request this relay never admitted must be
/// rejected.
#[tokio::test]
#[ignore]
async fn http_cancellation_without_admitted_request_rejected() {
    let foreign_relay = Keys::generate();
    let event = build_cancellation_event(
        &foreign_relay,
        &Keys::generate().public_key(),
        &EventId::all_zeros(),
        "req-orphan-cancel",
    )
    .expect("build cancellation");

    let (status, body) = post_event(&foreign_relay, &event).await;
    assert_eq!(status, 403, "expected 403, got {status}: {body}");
}

/// Job receipts are p-gated: the author and the `p` recipient can read them,
/// a third party cannot — including via a kindless event-id query.
#[tokio::test]
#[ignore]
async fn job_receipts_readable_only_by_author_or_recipient() {
    let author = Keys::generate();
    let recipient = Keys::generate();
    let outsider = Keys::generate();

    let mut author_client = BuzzTestClient::connect(&relay_url(), &author)
        .await
        .expect("connect author");

    // A member-published 43002 receipt (agents accept jobs from inside their
    // own community session; only shape is validated at ingest).
    let receipt = buzz_core::agent_job::build_accepted_event(
        &author,
        &recipient.public_key(),
        &EventId::all_zeros(),
        "req-receipt-gate",
    )
    .expect("build accepted receipt");
    let receipt_id = receipt.id;
    let ok = author_client.send_event(receipt).await.expect("OK frame");
    assert!(ok.accepted, "member receipt publish failed: {}", ok.message);

    // Author reads their own receipts (kinds + authors=self).
    let sid = sub_id("author-read");
    author_client
        .subscribe(
            &sid,
            vec![Filter::new()
                .kinds([Kind::Custom(KIND_JOB_ACCEPTED as u16)])
                .author(author.public_key())],
        )
        .await
        .expect("author REQ");
    let events = author_client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("author EOSE");
    assert!(
        events.iter().any(|event| event.id == receipt_id),
        "author must see their own receipt"
    );
    author_client.disconnect().await.ok();

    // Recipient reads via the #p gate.
    let mut recipient_client = BuzzTestClient::connect(&relay_url(), &recipient)
        .await
        .expect("connect recipient");
    let sid = sub_id("recipient-read");
    recipient_client
        .subscribe(
            &sid,
            vec![Filter::new()
                .kinds([Kind::Custom(KIND_JOB_ACCEPTED as u16)])
                .pubkey(recipient.public_key())],
        )
        .await
        .expect("recipient REQ");
    let events = recipient_client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("recipient EOSE");
    assert!(
        events.iter().any(|event| event.id == receipt_id),
        "recipient must see the receipt addressed to them"
    );
    recipient_client.disconnect().await.ok();

    // An outsider cannot read it by kind+author…
    let mut outsider_client = BuzzTestClient::connect(&relay_url(), &outsider)
        .await
        .expect("connect outsider");
    let sid = sub_id("outsider-kind");
    outsider_client
        .subscribe(
            &sid,
            vec![Filter::new()
                .kinds([Kind::Custom(KIND_JOB_ACCEPTED as u16)])
                .author(author.public_key())],
        )
        .await
        .expect("outsider REQ");
    let leaked = match outsider_client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
    {
        Ok(events) => events.iter().any(|event| event.id == receipt_id),
        // A CLOSED/restricted refusal is an acceptable gate too.
        Err(_) => false,
    };
    assert!(!leaked, "outsider must not read another party's receipt");

    // …and a kindless event-id query obeys the same result gate.
    let sid = sub_id("outsider-id");
    outsider_client
        .subscribe(&sid, vec![Filter::new().id(receipt_id)])
        .await
        .expect("outsider id REQ");
    let leaked = match outsider_client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
    {
        Ok(events) => events.iter().any(|event| event.id == receipt_id),
        Err(_) => false,
    };
    assert!(
        !leaked,
        "kindless id query must not bypass the job result gate"
    );
    outsider_client.disconnect().await.ok();

    // Drain guard: give the relay a beat before the socket teardown races the
    // last CLOSED frame in slow CI.
    tokio::time::sleep(Duration::from_millis(100)).await;
}
