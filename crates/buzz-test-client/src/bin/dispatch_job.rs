//! Dispatch a cross-community agent-job request — dev harness reproducing
//! exactly what a caller relay B sends to an agent's home relay A.
//!
//! Usage: dispatch_job <agent_pubkey_hex> <instruction>
//!   env: RELAY_B_KEY (caller relay signing key, hex)
//!        DISPATCH_TO_URL   (home relay A wss URL — where the agent listens)
//!        DISPATCH_RETURN_URL (caller relay B wss URL — result return address)
//!        SUBMIT_HTTP_URL   (http(s) base of home relay A for POST /events)

use nostr::{Keys, PublicKey};
use std::str::FromStr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: dispatch_job <agent_pubkey_hex> <instruction>");
        std::process::exit(1);
    }
    let agent = PublicKey::from_hex(&args[1])?;
    let instruction = args[2..].join(" ");

    let relay_b_key = std::env::var("RELAY_B_KEY").expect("RELAY_B_KEY");
    let return_url = std::env::var("DISPATCH_RETURN_URL").expect("DISPATCH_RETURN_URL");
    let submit_url = std::env::var("SUBMIT_HTTP_URL").expect("SUBMIT_HTTP_URL");
    let keys = Keys::from_str(&relay_b_key)?;

    let request_id = uuid_like();
    let expiration = 4_102_444_800; // year 2100, plenty of headroom
    let payload = buzz_core::agent_job::JobRequestPayload {
        instruction,
        caller_pubkey: keys.public_key().to_hex(),
        workflow_id: uuid_like(),
        run_id: uuid_like(),
        step_id: "ask".into(),
        channel_id: uuid_like(),
        listing_event_id: std::env::var("LISTING_EVENT_ID").unwrap_or_else(|_| "0".repeat(64)),
    };
    let event = buzz_core::agent_job::build_request_event(
        &keys,
        &agent,
        &request_id,
        expiration,
        &return_url,
        &payload,
    )?;
    println!("request event id: {}", event.id.to_hex());

    // POST /events over HTTP with NIP-98 auth signed by relay B — this is the
    // path that trips A's cross-community trigger (signer == event author).
    let mut post_url = url::Url::parse(&submit_url)?;
    post_url.set_path("/events");
    let body = serde_json::to_vec(&event)?;
    let authorization =
        buzz_core::http_auth::authorization_header(&keys, "POST", post_url.as_str(), Some(&body))
            .map_err(|e| anyhow::anyhow!(e))?;
    let client = reqwest::Client::new();
    let resp = client
        .post(post_url)
        .header("Authorization", authorization)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    println!("dispatched to A: HTTP {status} — {text}");
    Ok(())
}

// Deterministic-enough unique id in UUID shape (no rand crate needed here).
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let h = format!("{n:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}
