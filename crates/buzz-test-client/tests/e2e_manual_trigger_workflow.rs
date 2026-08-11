//! End-to-end coverage for `TriggerDef::Manual` (BUZZ-1, AC-3): a workflow
//! declared `on: manual` must run to completion via `buzz workflows trigger`
//! (kind:46020) exactly like the pre-existing `on: webhook`-as-workaround
//! convention, since `handle_workflow_trigger` never inspects `trigger` at
//! all — see `crates/buzz-relay/src/handlers/command_executor.rs`.
//!
//! # Running
//!
//! Requires a running relay (`just relay` or `cargo run -p buzz-relay`) with
//! Postgres + Redis, same as `e2e_relay.rs`:
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_manual_trigger_workflow -- --ignored
//! ```
//!
//! Override the relay URL with `RELAY_URL` (defaults to `ws://localhost:3000`).

use std::time::Duration;

use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

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

const KIND_WORKFLOW_DEF: u16 = 30620;
const KIND_WORKFLOW_TRIGGER: u16 = 46020;
const KIND_CHANNEL_CREATE: u16 = 9007;
const KIND_STREAM_MESSAGE: u16 = 9;

async fn e2e_db_pool() -> sqlx::Pool<sqlx::Postgres> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect to e2e Postgres")
}

async fn submit_event(keys: &Keys, event: nostr::Event) -> serde_json::Value {
    let client = reqwest::Client::new();
    let http_base = relay_http_url();
    let resp = client
        .post(format!("{http_base}/events"))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).expect("serialize event"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST /events failed: {e}"));
    let status = resp.status();
    let body = resp.text().await.expect("read /events body");
    assert!(
        status.is_success(),
        "POST /events returned HTTP {status}: {body}"
    );
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("parse /events JSON: {e} (body: {body})"))
}

async fn create_open_channel(owner: &Keys, channel_uuid: Uuid) -> String {
    let event = EventBuilder::new(Kind::Custom(KIND_CHANNEL_CREATE), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("manual-wf-e2e-{channel_uuid}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let body = submit_event(owner, event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "create-channel not accepted: {body}"
    );
    channel_uuid.to_string()
}

/// Define a workflow with `on: manual` and a single `send_message` step.
/// Returns the server-generated workflow id.
async fn define_manual_workflow(owner: &Keys, channel_id: &str, name: &str, text: &str) -> String {
    let yaml = format!(
        "name: {name}\n\
         trigger:\n\
         \x20 on: manual\n\
         steps:\n\
         \x20 - id: notify\n\
         \x20   action: send_message\n\
         \x20   text: \"{text}\"\n"
    );
    let workflow_uuid = Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF), yaml)
        .tags(vec![
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["d", &workflow_uuid.to_string()]).unwrap(),
            Tag::parse(["name", name]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let body = submit_event(owner, event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "workflow def not accepted: {body}"
    );
    let msg = body["message"].as_str().unwrap_or_default();
    let json_part = msg
        .strip_prefix("response:")
        .unwrap_or_else(|| panic!("workflow def OK message missing `response:` prefix: {msg:?}"));
    let resp: serde_json::Value = serde_json::from_str(json_part)
        .unwrap_or_else(|e| panic!("parse workflow def response json: {e} ({json_part:?})"));
    resp["workflow_id"]
        .as_str()
        .unwrap_or_else(|| panic!("workflow def response missing workflow_id: {resp}"))
        .to_string()
}

/// Fire a workflow by id (kind:46020, `d`=id) — same door `buzz workflows
/// trigger` uses (`cmd_trigger_workflow` in `crates/buzz-cli`).
async fn trigger_workflow(owner: &Keys, workflow_id: &str) -> serde_json::Value {
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_TRIGGER), "")
        .tags(vec![Tag::parse(["d", workflow_id]).unwrap()])
        .sign_with_keys(owner)
        .unwrap();
    submit_event(owner, event).await
}

async fn latest_run_id(pool: &sqlx::Pool<sqlx::Postgres>, workflow_id: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM workflow_runs WHERE workflow_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(workflow_id)
    .fetch_one(pool)
    .await
    .expect("find latest run")
}

async fn wait_for_terminal_run_status(
    pool: &sqlx::Pool<sqlx::Postgres>,
    run_id: Uuid,
    timeout: Duration,
) -> (String, serde_json::Value) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let row: (String, serde_json::Value) =
            sqlx::query_as("SELECT status::text, execution_trace FROM workflow_runs WHERE id = $1")
                .bind(run_id)
                .fetch_one(pool)
                .await
                .expect("query workflow_runs");
        if !matches!(row.0.as_str(), "pending" | "running") {
            return row;
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "timed out waiting for run {run_id} to leave pending/running; last status: {row:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_message_event(caller: &Keys, channel_id: &str, text: &str) -> serde_json::Value {
    let client = reqwest::Client::new();
    let http_base = relay_http_url();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let filters = serde_json::json!([{
            "kinds": [KIND_STREAM_MESSAGE],
            "#h": [channel_id],
            "limit": 20,
        }]);
        let resp = client
            .post(format!("{http_base}/query"))
            .header("X-Pubkey", caller.public_key().to_hex())
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&filters).unwrap())
            .send()
            .await
            .expect("POST /query");
        let events: Vec<serde_json::Value> = resp.json().await.expect("query JSON");
        if let Some(hit) = events.iter().find(|e| e["content"].as_str() == Some(text)) {
            return hit.clone();
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "timed out waiting for \"{text}\" message event in channel {channel_id}; \
                 saw {} kind:9 events: {events:?}",
                events.len()
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// AC-3: `buzz workflows trigger --workflow <UUID>` runs a Manual-trigger
/// workflow end-to-end, unmodified — a stored `on: manual` workflow reaches
/// `completed` and its `send_message` step's output lands in the channel.
#[tokio::test]
#[ignore = "requires a running relay + Postgres + Redis"]
async fn manual_trigger_workflow_runs_end_to_end() {
    let owner = Keys::generate();
    let pool = e2e_db_pool().await;

    let channel_uuid = Uuid::new_v4();
    let channel_id = create_open_channel(&owner, channel_uuid).await;

    let wf_name = format!("manual-e2e-{}", Uuid::new_v4().simple());
    let notify_text = format!("manual trigger fired {}", Uuid::new_v4().simple());
    let workflow_id = define_manual_workflow(&owner, &channel_id, &wf_name, &notify_text).await;

    let trigger_resp = trigger_workflow(&owner, &workflow_id).await;
    assert!(
        trigger_resp["accepted"].as_bool().unwrap_or(false),
        "trigger not accepted: {trigger_resp}"
    );

    let workflow_uuid: Uuid = workflow_id.parse().expect("workflow id is a UUID");
    let run_id = latest_run_id(&pool, workflow_uuid).await;
    let (status, trace) =
        wait_for_terminal_run_status(&pool, run_id, Duration::from_secs(10)).await;
    assert_eq!(
        status, "completed",
        "manual-trigger run must complete; trace: {trace}"
    );

    let entries = trace.as_array().expect("trace is an array");
    let notify_entry = entries
        .iter()
        .find(|e| e["step_id"] == "notify")
        .unwrap_or_else(|| panic!("no trace entry for step 'notify': {trace}"));
    assert_eq!(notify_entry["status"], "completed");

    wait_for_message_event(&owner, &channel_id, &notify_text).await;
}
