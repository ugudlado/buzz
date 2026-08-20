//! Probe: POST a job-result (43004) authored by the agent, p-tagged to a
//! foreign caller relay, to the agent's HOME relay — reproduces the 403 that
//! blocks the agent from persisting its own cross-community result.
//!
//! Usage: probe_terminal
//!   env: BUMBLE_KEY (agent key), CALLER_PUBKEY (foreign relay B pubkey hex),
//!        SUBMIT_HTTP_URL (home relay A http base)

use nostr::{Keys, PublicKey};
use std::str::FromStr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let agent = Keys::from_str(&std::env::var("BUMBLE_KEY")?)?;
    let caller = PublicKey::from_hex(&std::env::var("CALLER_PUBKEY")?)?;
    let submit = std::env::var("SUBMIT_HTTP_URL")?;

    // A minimal valid terminal: NIP-44 ciphertext content, p=caller, e=req, request tag.
    let payload = buzz_core::agent_job::JobResultPayload {
        outcome: "completed".into(),
        output: Some("VERIFY OK".into()),
        error: None,
        completed_at: 1_700_000_000,
        usage: None,
    };
    let fake_req = nostr::EventId::all_zeros();
    let request_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    let terminal = buzz_core::agent_job::build_terminal_event(
        &agent, &caller, &fake_req, request_id, &payload,
    )?;
    println!(
        "terminal id: {} p->{}",
        terminal.id.to_hex(),
        caller.to_hex()
    );

    let mut url = url::Url::parse(&submit)?;
    url.set_path("/events");
    let body = serde_json::to_vec(&terminal)?;
    let authz =
        buzz_core::http_auth::authorization_header(&agent, "POST", url.as_str(), Some(&body))
            .map_err(|e| anyhow::anyhow!(e))?;
    let resp = reqwest::Client::new()
        .post(url)
        .header("Authorization", authz)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await?;
    println!(
        "HTTP {} — {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    Ok(())
}
