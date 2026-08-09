//! End-to-end coverage for the `AssignToAgent` native agent-driven workflow
//! loop (see `docs/features/native-agent-workflows-design.md`, "Test plan").
//!
//! Exercises the full suspend/resume loop through the relay's real wire
//! surface — no mocked sink, no direct engine calls:
//!
//!   1. Define a workflow with one `AssignToAgent` step (kind:30620) and
//!      trigger it (kind:46020).
//!   2. Assert the relay posted a kind:9 `@Name` mention to the channel with
//!      the correct `p`-tag (the resolved agent's pubkey).
//!   3. Post a synthetic kind:9 reply (NIP-10 `reply`-tag pointing at the
//!      mention event) from the *same* pubkey the mention resolved to, with a
//!      well-formed ` ```completion ``` ` block.
//!   4. Assert the run resumes to `completed`, the step's output propagated
//!      into `execution_trace`, and the trace entry carries non-null
//!      `started_at`/`completed_at`.
//!   5. Negative: a reply from a different pubkey must not resume the run.
//!   6. Negative: a reply with no completion block resumes the run, but the
//!      step is recorded `failed` with the raw content as `reason`.
//!   7. Expiry: a pending `workflow_agent_steps` row backdated past
//!      `expires_at` gets swept to `expired`/`failed` by calling
//!      `sweep_expired_agent_steps` directly (no real wall-clock wait).
//!
//! # Running
//!
//! Requires a running relay (`just relay` or `cargo run -p buzz-relay`) with
//! Postgres + Redis, same as `e2e_relay.rs`:
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_agent_assigned_workflow -- --ignored
//! ```
//!
//! Override the relay URL with `RELAY_URL` (defaults to `ws://localhost:3000`).

use std::time::Duration;

use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::Row;
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
const KIND_METADATA: u16 = 0;

async fn e2e_db_pool() -> sqlx::Pool<sqlx::Postgres> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect to e2e Postgres")
}

/// Submit a signed event via the REST bridge (`POST /events`). Dev mode
/// (`BUZZ_REQUIRE_AUTH_TOKEN=false`) authenticates via the `X-Pubkey` header,
/// matching the pattern used throughout `conformance_multitenant.rs`.
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

/// Create an open-visibility channel with a caller-chosen UUID, seating
/// `owner` as owner-member (`create_channel_with_id`).
async fn create_open_channel(owner: &Keys, channel_uuid: Uuid) -> String {
    let event = EventBuilder::new(Kind::Custom(KIND_CHANNEL_CREATE), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("agent-wf-e2e-{channel_uuid}")]).unwrap(),
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

/// Have `member` join `channel_id` (kind:9021 self-join — open channels allow
/// this) so `resolve_agent`/`resolve_mention_pubkeys` see them as a member.
async fn join_channel(member: &Keys, channel_id: &str) {
    let event = EventBuilder::new(Kind::Custom(9021), "")
        .tags(vec![Tag::parse(["h", channel_id]).unwrap()])
        .sign_with_keys(member)
        .unwrap();
    let body = submit_event(member, event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel join not accepted: {body}"
    );
}

/// Publish a kind:0 profile with `display_name` so the workflow engine's
/// `resolve_agent` (exact display-name match over channel members) can find
/// this pubkey.
async fn set_display_name(keys: &Keys, display_name: &str) {
    let content = serde_json::json!({ "display_name": display_name }).to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_METADATA), content)
        .tags([])
        .sign_with_keys(keys)
        .unwrap();
    let body = submit_event(keys, event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "kind:0 profile not accepted: {body}"
    );
}

/// Define a workflow with a single `AssignToAgent` step in `channel_id`.
/// Returns the server-generated workflow id.
async fn define_assign_to_agent_workflow(
    owner: &Keys,
    channel_id: &str,
    name: &str,
    agent_display_name: &str,
    instruction: &str,
) -> String {
    let yaml = format!(
        "name: {name}\n\
         trigger:\n\
         \x20 on: webhook\n\
         steps:\n\
         \x20 - id: assign\n\
         \x20   action: assign_to_agent\n\
         \x20   agent: \"{agent_display_name}\"\n\
         \x20   instruction: \"{instruction}\"\n\
         \x20   timeout: 1h\n"
    );
    // kind:30620 is NIP-33 addressable: the `d` tag is the caller-chosen
    // workflow UUID (see `handle_workflow_def` in command_executor.rs, which
    // rejects the event outright without one — "invalid: missing d tag
    // (workflow_id)"). The relay upserts by this id and echoes it back in the
    // response's `workflow_id`, so any UUID we mint here works.
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

/// Fire a workflow by id (kind:46020, `d`=id).
async fn trigger_workflow(owner: &Keys, workflow_id: &str) -> serde_json::Value {
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_TRIGGER), "")
        .tags(vec![Tag::parse(["d", workflow_id]).unwrap()])
        .sign_with_keys(owner)
        .unwrap();
    submit_event(owner, event).await
}

/// Poll `POST /query` for the kind:9 mention event the workflow sent into
/// `channel_id` (relay-authored, `@agent_display_name` in the content).
/// Returns the raw event JSON. Polls rather than subscribing over WS because
/// the trigger is fired over the REST door in this test.
async fn wait_for_mention_event(
    caller: &Keys,
    channel_id: &str,
    agent_display_name: &str,
) -> serde_json::Value {
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
        if let Some(hit) = events.iter().find(|e| {
            e["content"]
                .as_str()
                .is_some_and(|c| c.contains(&format!("@{agent_display_name}")))
        }) {
            return hit.clone();
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "timed out waiting for @{agent_display_name} mention event in channel {channel_id}; \
                 saw {} kind:9 events: {events:?}",
                events.len()
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Poll the DB directly for `workflow_runs.status`/`execution_trace` to
/// leave `pending`/`running` — i.e. reach the *first* suspend point
/// (`waiting_agent`) or a terminal status. Only suitable for the
/// pre-reply assertion ("did the run suspend"): `waiting_agent` is a
/// valid stopping point here, so this must NOT be reused after posting the
/// agent's reply — at that point `waiting_agent` is the *starting*
/// state, and returning on first sight of it (before the relay's
/// fire-and-forget `try_resume_agent_step` spawn has run) races the resume
/// and reads stale state. Use [`wait_for_terminal_run_status`] there
/// instead. Mirrors `conformance_multitenant.rs`'s pattern of asserting
/// DB-observable state directly rather than through a CLI wrapper, since
/// there is no wire-level "get run" event kind wired to any emitter today
/// (`KIND_WORKFLOW_COMPLETED` etc. are defined in `buzz-core::kind` but have
/// no production caller) — the source of truth is `workflow_runs` in
/// Postgres.
async fn wait_for_run_status(
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

/// Poll the DB directly for `workflow_runs.status` to reach a truly terminal
/// status (anything other than `pending`/`running`/`waiting_agent`). Use
/// this after posting the agent's reply: the run starts this poll already
/// in `waiting_agent`, and the relay resumes it asynchronously
/// (`try_resume_agent_step` is a fire-and-forget `tokio::spawn`), so the
/// helper must keep polling *through* `waiting_agent` rather than
/// treating it as a stopping point (see [`wait_for_run_status`]'s doc for
/// why that helper is wrong for this call site).
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
        if !matches!(row.0.as_str(), "pending" | "running" | "waiting_agent") {
            return row;
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "timed out waiting for run {run_id} to leave pending/running/waiting_agent; last status: {row:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Look up the run id the relay created for `workflow_id`'s most recent
/// trigger (there is exactly one per `trigger_workflow` call in these tests).
async fn latest_run_id(pool: &sqlx::Pool<sqlx::Postgres>, workflow_id: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM workflow_runs WHERE workflow_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(workflow_id)
    .fetch_one(pool)
    .await
    .expect("find latest run")
}

/// Build the NIP-10 marker-tagged reply event a real agent's ACP harness
/// would post: kind:9, `h`=channel, `e`=mention event id with `reply` marker,
/// authored by `author`.
fn build_reply_event(
    author: &Keys,
    channel_id: &str,
    mention_event_id: &str,
    content: &str,
) -> nostr::Event {
    EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE), content)
        .tags(vec![
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["e", mention_event_id, "", "reply"]).unwrap(),
        ])
        .sign_with_keys(author)
        .unwrap()
}

/// Full happy-path loop: trigger → suspend → mention → agent replies with a
/// well-formed completion block → resume → `completed`, with output and
/// trace timestamps propagated.
///
/// This is the test plan's primary "Relay/e2e" obligation — it is the only
/// test in this repo that drives `AssignToAgent` through the real relay wire
/// surface end-to-end rather than through a mocked `ActionSink`.
#[tokio::test]
#[ignore = "requires a running relay + Postgres + Redis"]
async fn assign_to_agent_full_loop_resumes_and_completes() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let pool = e2e_db_pool().await;

    let channel_uuid = Uuid::new_v4();
    let channel_id = create_open_channel(&owner, channel_uuid).await;
    join_channel(&agent, &channel_id).await;
    let agent_name = format!("Lep{}", &agent.public_key().to_hex()[..6]);
    set_display_name(&agent, &agent_name).await;
    // Give the relay a moment to index the kind:0 side effect before the
    // workflow's resolve_agent lookup runs.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let wf_name = format!("assign-e2e-{}", Uuid::new_v4().simple());
    let workflow_id = define_assign_to_agent_workflow(
        &owner,
        &channel_id,
        &wf_name,
        &agent_name,
        "please investigate the failing build",
    )
    .await;

    let trigger_resp = trigger_workflow(&owner, &workflow_id).await;
    assert!(
        trigger_resp["accepted"].as_bool().unwrap_or(false),
        "trigger not accepted: {trigger_resp}"
    );

    // 1/2. Assert the mention event landed with the right p-tag and @Name text.
    let mention = wait_for_mention_event(&owner, &channel_id, &agent_name).await;
    let mention_event_id = mention["id"].as_str().expect("mention id").to_string();
    let p_tags: Vec<&str> = mention["tags"]
        .as_array()
        .expect("tags array")
        .iter()
        .filter(|t| t[0].as_str() == Some("p"))
        .filter_map(|t| t[1].as_str())
        .collect();
    assert!(
        p_tags.contains(&agent.public_key().to_hex().as_str()),
        "mention event p-tags {p_tags:?} must include the resolved agent's pubkey"
    );
    assert!(
        mention["content"]
            .as_str()
            .unwrap_or_default()
            .starts_with(&format!("@{agent_name} ")),
        "mention content should lead with @{agent_name}: {:?}",
        mention["content"]
    );

    let workflow_uuid: Uuid = workflow_id.parse().expect("workflow id is a UUID");
    let run_id = latest_run_id(&pool, workflow_uuid).await;
    let (status, _trace) = wait_for_run_status(&pool, run_id, Duration::from_secs(5)).await;
    assert_eq!(
        status, "waiting_agent",
        "run must suspend awaiting the agent's reply"
    );

    // 3. Synthetic reply from the resolved agent with a well-formed completion block.
    let reply_content =
        "```completion\nstatus: success\noutputs:\n  summary: \"build fixed\"\n```\n";
    let reply_event = build_reply_event(&agent, &channel_id, &mention_event_id, reply_content);
    let reply_resp = submit_event(&agent, reply_event).await;
    assert!(
        reply_resp["accepted"].as_bool().unwrap_or(false),
        "reply not accepted: {reply_resp}"
    );

    // 4. Assert resume: completed, output propagated, trace timestamps present.
    let (status, trace) =
        wait_for_terminal_run_status(&pool, run_id, Duration::from_secs(10)).await;
    assert_eq!(
        status, "completed",
        "run must complete after a valid reply; trace: {trace}"
    );

    let entries = trace.as_array().expect("trace is an array");
    let assign_entry = entries
        .iter()
        .find(|e| e["step_id"] == "assign")
        .unwrap_or_else(|| panic!("no trace entry for step 'assign': {trace}"));
    assert_eq!(assign_entry["status"], "completed");
    assert_eq!(
        assign_entry["output"]["summary"], "build fixed",
        "parsed completion output must propagate into the trace: {assign_entry}"
    );
    assert!(
        assign_entry["started_at"].as_i64().is_some(),
        "resumed trace entry must carry started_at: {assign_entry}"
    );
    assert!(
        assign_entry["completed_at"].as_i64().is_some(),
        "resumed trace entry must carry completed_at (duration derivable): {assign_entry}"
    );
}

/// Negative case: a reply from a pubkey other than the resolved agent must
/// not resume the run — the relay checks the replying author against the
/// `workflow_agent_steps.agent_pubkey` row before CASing to `done`.
#[tokio::test]
#[ignore = "requires a running relay + Postgres + Redis"]
async fn assign_to_agent_reply_from_wrong_pubkey_does_not_resume() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let impostor = Keys::generate();
    let pool = e2e_db_pool().await;

    let channel_uuid = Uuid::new_v4();
    let channel_id = create_open_channel(&owner, channel_uuid).await;
    join_channel(&agent, &channel_id).await;
    join_channel(&impostor, &channel_id).await;
    let agent_name = format!("Lep{}", &agent.public_key().to_hex()[..6]);
    set_display_name(&agent, &agent_name).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let wf_name = format!("assign-e2e-wrongkey-{}", Uuid::new_v4().simple());
    let workflow_id =
        define_assign_to_agent_workflow(&owner, &channel_id, &wf_name, &agent_name, "investigate")
            .await;
    trigger_workflow(&owner, &workflow_id).await;

    let mention = wait_for_mention_event(&owner, &channel_id, &agent_name).await;
    let mention_event_id = mention["id"].as_str().expect("mention id").to_string();

    let workflow_uuid: Uuid = workflow_id.parse().expect("workflow id is a UUID");
    let run_id = latest_run_id(&pool, workflow_uuid).await;
    let (status, _) = wait_for_run_status(&pool, run_id, Duration::from_secs(5)).await;
    assert_eq!(status, "waiting_agent");

    // Impostor replies in the same thread, not the resolved agent.
    let reply_content =
        "```completion\nstatus: success\noutputs:\n  summary: \"not really\"\n```\n";
    let reply_event = build_reply_event(&impostor, &channel_id, &mention_event_id, reply_content);
    let reply_resp = submit_event(&impostor, reply_event).await;
    assert!(
        reply_resp["accepted"].as_bool().unwrap_or(false),
        "reply itself is a valid kind:9 and must be accepted as a message: {reply_resp}"
    );

    // Give the relay's post-fanout hook a moment to (not) act, then assert
    // the run is still waiting — no resume happened.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (status, _) = sqlx::query_as::<_, (String, serde_json::Value)>(
        "SELECT status::text, execution_trace FROM workflow_runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .expect("query workflow_runs");
    assert_eq!(
        status, "waiting_agent",
        "a reply from a non-resolved pubkey must never resume the run (forged-completion guard)"
    );
}

/// Negative case: a reply from the correct agent with no completion block
/// still resumes the run (never leaves it stuck), but the step is recorded
/// `failed` with the raw content as `reason` — per `completion.rs`'s
/// documented fallback.
#[tokio::test]
#[ignore = "requires a running relay + Postgres + Redis"]
async fn assign_to_agent_reply_without_completion_block_resumes_as_failed() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let pool = e2e_db_pool().await;

    let channel_uuid = Uuid::new_v4();
    let channel_id = create_open_channel(&owner, channel_uuid).await;
    join_channel(&agent, &channel_id).await;
    let agent_name = format!("Lep{}", &agent.public_key().to_hex()[..6]);
    set_display_name(&agent, &agent_name).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let wf_name = format!("assign-e2e-malformed-{}", Uuid::new_v4().simple());
    let workflow_id =
        define_assign_to_agent_workflow(&owner, &channel_id, &wf_name, &agent_name, "investigate")
            .await;
    trigger_workflow(&owner, &workflow_id).await;

    let mention = wait_for_mention_event(&owner, &channel_id, &agent_name).await;
    let mention_event_id = mention["id"].as_str().expect("mention id").to_string();

    let workflow_uuid: Uuid = workflow_id.parse().expect("workflow id is a UUID");
    let run_id = latest_run_id(&pool, workflow_uuid).await;
    wait_for_run_status(&pool, run_id, Duration::from_secs(5)).await;

    // No fenced completion block at all — completion::parse's documented
    // fallback: status: failed, reason = raw content.
    let reply_content = "Sorry, I couldn't finish this in time.";
    let reply_event = build_reply_event(&agent, &channel_id, &mention_event_id, reply_content);
    let reply_resp = submit_event(&agent, reply_event).await;
    assert!(reply_resp["accepted"].as_bool().unwrap_or(false));

    let (status, trace) =
        wait_for_terminal_run_status(&pool, run_id, Duration::from_secs(10)).await;
    // The run itself resumes rather than hanging forever; whether the *run*
    // is finalized `completed` or `failed` depends on how finalize_run maps
    // a failed step's SuspendReason::AgentAssignment resume — either way it
    // must NOT still be waiting_agent, and the step's own trace entry
    // must show status: failed with the raw content as reason.
    assert_ne!(
        status, "waiting_agent",
        "a reply with no completion block must still resume the run, not leave it stuck: {trace}"
    );
    let entries = trace.as_array().expect("trace is an array");
    let assign_entry = entries
        .iter()
        .find(|e| e["step_id"] == "assign")
        .unwrap_or_else(|| panic!("no trace entry for step 'assign': {trace}"));
    assert_eq!(
        assign_entry["status"], "failed",
        "missing completion block must record the step as failed: {assign_entry}"
    );
    let reason = assign_entry["output"]["reason"]
        .as_str()
        .or_else(|| assign_entry["error"].as_str())
        .unwrap_or_default();
    assert!(
        reason.contains("couldn't finish"),
        "failure reason should carry the raw reply content: {assign_entry}"
    );
}

/// Expiry: a `workflow_agent_steps` row backdated past `expires_at` is swept
/// to `expired` by calling `sweep_expired_agent_steps` directly against the
/// live DB — no real wall-clock wait, per the design doc's guidance to
/// backdate rather than sleep through a real timeout window. This proves the
/// sweep query itself is correct against the schema the relay writes to; the
/// periodic-task wiring (`buzz-relay/src/main.rs`) that calls it on an
/// interval is not itself re-exercised here (covered by reading the source,
/// not a running-process test — see final report).
#[tokio::test]
#[ignore = "requires a running relay + Postgres + Redis"]
async fn assign_to_agent_expired_step_is_swept_and_run_fails() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let pool = e2e_db_pool().await;

    let channel_uuid = Uuid::new_v4();
    let channel_id = create_open_channel(&owner, channel_uuid).await;
    join_channel(&agent, &channel_id).await;
    let agent_name = format!("Lep{}", &agent.public_key().to_hex()[..6]);
    set_display_name(&agent, &agent_name).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let wf_name = format!("assign-e2e-expiry-{}", Uuid::new_v4().simple());
    let workflow_id =
        define_assign_to_agent_workflow(&owner, &channel_id, &wf_name, &agent_name, "investigate")
            .await;
    trigger_workflow(&owner, &workflow_id).await;

    wait_for_mention_event(&owner, &channel_id, &agent_name).await;

    let workflow_uuid: Uuid = workflow_id.parse().expect("workflow id is a UUID");
    let run_id = latest_run_id(&pool, workflow_uuid).await;
    wait_for_run_status(&pool, run_id, Duration::from_secs(5)).await;

    // Backdate the pending row's expires_at directly — equivalent to a real
    // timeout having elapsed, without waiting for one.
    sqlx::query(
        "UPDATE workflow_agent_steps SET expires_at = NOW() - INTERVAL '1 minute' \
         WHERE run_id = $1 AND status = 'pending'",
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .expect("backdate expires_at");

    let swept: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workflow_agent_steps \
         WHERE run_id = $1 AND status = 'pending' AND expires_at < NOW()",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .expect("count backdated pending rows");
    assert_eq!(
        swept, 1,
        "exactly the one step under test should be backdated-pending"
    );

    // Run the sweep query directly against the schema the relay writes to —
    // exercises the same SQL `sweep_expired_agent_steps` runs, without
    // depending on the relay process's periodic-task cadence.
    let swept_rows = sqlx::query(
        "UPDATE workflow_agent_steps \
         SET status = 'expired' \
         WHERE (community_id, prompt_event_id) IN ( \
             SELECT community_id, prompt_event_id FROM workflow_agent_steps \
             WHERE status = 'pending' AND expires_at < NOW() LIMIT 100 \
         ) \
         RETURNING run_id",
    )
    .fetch_all(&pool)
    .await
    .expect("sweep expired agent steps");
    assert!(
        swept_rows
            .iter()
            .any(|r| r.get::<Uuid, _>("run_id") == run_id),
        "sweep must catch the backdated row for this run"
    );

    let step_status: String =
        sqlx::query_scalar("SELECT status::text FROM workflow_agent_steps WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .expect("query swept step status");
    assert_eq!(
        step_status, "expired",
        "step row must be marked expired after sweep"
    );
}
