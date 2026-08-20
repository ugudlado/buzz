use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    relay::{parse_command_response, query_relay, submit_event},
    relay_http::get_relay_json,
};

// ── Wire shapes (snake_case, consumed by tauriWorkflows.ts) ──────────────────

/// A workflow definition as the desktop frontend expects it. Mirrors the
/// `RawWorkflow` type in `desktop/src/shared/api/tauriWorkflows.ts`.
///
/// The relay stores a workflow as a single kind:30620 event whose content is
/// the raw YAML. Everything the UI needs is derived from that event:
/// - `id` / `channel_id` from the `d` / `h` tags,
/// - `definition` from parsing the YAML body into a free-form object,
/// - `name` from `definition.name`,
/// - `owner_pubkey` / timestamps from the event itself.
///
/// `status` is always `"active"` here: the relay's disable/archive lifecycle is
/// not reflected back into the kind:30620 event, and the UI derives a
/// "disabled" display state from `definition.enabled` on its own
/// (`getWorkflowDisplayStatus`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowWire {
    pub id: String,
    pub name: String,
    pub owner_pubkey: String,
    pub channel_id: Option<String>,
    pub definition: Value,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Response shape for create/update. Mirrors `RawWorkflowSaveResponse` in the
/// frontend: a full workflow record plus an optional webhook secret (only
/// present for webhook-triggered workflows on creation).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowSaveWire {
    #[serde(flatten)]
    pub workflow: WorkflowWire,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
}

// ── Reads ────────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_channel_workflows(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowWire>, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#h": [channel_id],
        })],
    )
    .await?;

    Ok(events.iter().map(workflow_from_event).collect())
}

/// Fetch workflows across many channels in a single relay round-trip.
///
/// The Workflows overview screen previously issued one `get_channel_workflows`
/// query per member channel (`Promise.all` fanout in `WorkflowsView`), i.e. N
/// relay POSTs. A nostr `#h` filter matches ANY of its listed values, so one
/// query with all channel ids returns the same set. Each `WorkflowWire` carries
/// its own `channel_id` (from the event's `h` tag), so the frontend can still
/// group results by channel. Neither this nor the per-channel command sets a
/// `limit`, so batching does not change result completeness.
#[tauri::command]
pub async fn get_channels_workflows(
    channel_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowWire>, String> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#h": channel_ids,
        })],
    )
    .await?;

    Ok(events.iter().map(workflow_from_event).collect())
}

#[tauri::command]
pub async fn get_workflow(
    workflow_id: String,
    state: State<'_, AppState>,
) -> Result<WorkflowWire, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#d": [workflow_id],
            "limit": 1
        })],
    )
    .await?;

    events
        .first()
        .map(workflow_from_event)
        .ok_or_else(|| "workflow not found".to_string())
}

/// Fetch runs for a workflow via the relay's `GET /api/workflows/{id}/runs`
/// bridge (NIP-98 signed). Returns a bare array — the frontend wrapper
/// (`getWorkflowRuns`) does `raw.map(fromRawWorkflowRun)`, so a wrapped
/// `{ runs: [...] }` shape would make `.map()` throw and crash the detail
/// panel.
#[tauri::command]
pub async fn get_workflow_runs(
    workflow_id: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<Value>, String> {
    let mut path = format!("/api/workflows/{workflow_id}/runs");
    if let Some(limit) = limit {
        path.push_str(&format!("?limit={limit}"));
    }
    get_relay_json(&state, &path).await
}

/// Provider-side job ledger for one of this community's agents: which caller
/// communities invoked it, duration, and estimated cost. Owner-gated by the
/// relay. Returns `{ jobs: [...], totals: {...} }`.
#[tauri::command]
pub async fn get_agent_jobs(
    agent_pubkey: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut path = format!("/api/agents/{agent_pubkey}/jobs");
    if let Some(limit) = limit {
        path.push_str(&format!("?limit={limit}"));
    }
    let values: Value = get_relay_json(&state, &path).await?;
    Ok(values)
}

// ── Writes ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn create_workflow(
    channel_id: String,
    yaml_definition: String,
    state: State<'_, AppState>,
) -> Result<WorkflowSaveWire, String> {
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let builder = events::build_workflow_definition(&workflow_id, &channel_id, &yaml_definition)?;
    let result = submit_event(builder, &state).await?;

    // The relay returns `webhook_secret` in the OK response message for
    // webhook-triggered workflows. Everything else in the save record is built
    // locally from the inputs we already hold — the relay's create response
    // only carries `{ workflow_id, webhook_secret? }`.
    let webhook_secret = parse_command_response::<Value>(&result.message)
        .ok()
        .and_then(|v| {
            v.get("webhook_secret")
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    let now = now_secs();
    let workflow = workflow_record(
        workflow_id,
        Some(channel_id),
        current_pubkey_hex(&state)?,
        &yaml_definition,
        now,
        now,
    );

    Ok(WorkflowSaveWire {
        workflow,
        webhook_secret,
    })
}

#[tauri::command]
pub async fn update_workflow(
    workflow_id: String,
    yaml_definition: String,
    state: State<'_, AppState>,
) -> Result<WorkflowSaveWire, String> {
    // Find the channel id (and creation time) from the existing workflow event
    // so the new event carries the same `h` tag — kind:30620 is replaceable by
    // (pubkey, d-tag).
    let prior = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#d": [workflow_id.clone()],
            "limit": 1
        })],
    )
    .await?;

    let prior_event = prior
        .first()
        .ok_or_else(|| "workflow not found".to_string())?;
    let channel_id = tag_value(prior_event, "h").ok_or_else(|| "workflow not found".to_string())?;
    let created_at = prior_event.created_at.as_secs() as i64;

    let builder = events::build_workflow_definition(&workflow_id, &channel_id, &yaml_definition)?;
    submit_event(builder, &state).await?;

    let updated_at = now_secs();
    let workflow = workflow_record(
        workflow_id,
        Some(channel_id),
        current_pubkey_hex(&state)?,
        &yaml_definition,
        created_at,
        updated_at,
    );

    Ok(WorkflowSaveWire {
        workflow,
        // Updates never rotate the webhook secret.
        webhook_secret: None,
    })
}

#[tauri::command]
pub async fn delete_workflow(
    workflow_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let builder = events::build_workflow_delete(&workflow_id, &current_pubkey_hex(&state)?)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn trigger_workflow(
    workflow_id: String,
    fields: Option<Value>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let content = match &fields {
        Some(value) if value.is_object() => Some(value.to_string()),
        Some(_) => return Err("trigger fields must be a JSON object".to_string()),
        None => None,
    };
    let builder = events::build_workflow_trigger(&workflow_id, content.as_deref())?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({ "event_id": result.event_id }))
}

// ── Approvals ────────────────────────────────────────────────────────────────

/// Fetch approval gates for a run via the relay's
/// `GET /api/workflows/{id}/runs/{run_id}/approvals` bridge (NIP-98 signed).
/// Returns a bare array — the frontend's `getRunApprovals` does
/// `raw.map(fromRawApproval)`.
#[tauri::command]
pub async fn get_run_approvals(
    workflow_id: String,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Value>, String> {
    let path = format!("/api/workflows/{workflow_id}/runs/{run_id}/approvals");
    get_relay_json(&state, &path).await
}

#[tauri::command]
pub async fn grant_approval(
    token: String,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let builder = events::build_approval_grant(&token, note.as_deref())?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({ "event_id": result.event_id }))
}

#[tauri::command]
pub async fn deny_approval(
    token: String,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let builder = events::build_approval_deny(&token, note.as_deref())?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({ "event_id": result.event_id }))
}

// ── Helpers (pure, unit-tested in workflows_tests.rs) ─────────────────────────

fn current_pubkey_hex(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// First value of the tag whose name matches `name` (e.g. `d`, `h`).
fn tag_value(ev: &nostr::Event, name: &str) -> Option<String> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.len() >= 2 && s[0] == name).then(|| s[1].clone())
    })
}

/// Parse a workflow's YAML body into a free-form JSON object. The frontend
/// consumes `definition` as `Record<string, unknown>`, so we preserve the full
/// document. On parse failure (or a non-object document) we fall back to an
/// empty object rather than failing the whole list query — a single malformed
/// workflow must not break the page.
fn parse_definition(yaml: &str) -> Value {
    match serde_yaml::from_str::<Value>(yaml) {
        Ok(v @ Value::Object(_)) => v,
        _ => Value::Object(serde_json::Map::new()),
    }
}

/// Build a [`WorkflowWire`] record from its parts. Shared by the read path
/// (from a relay event) and the write path (from local inputs).
fn workflow_record(
    id: String,
    channel_id: Option<String>,
    owner_pubkey: String,
    yaml_definition: &str,
    created_at: i64,
    updated_at: i64,
) -> WorkflowWire {
    let definition = parse_definition(yaml_definition);
    let name = definition
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());

    WorkflowWire {
        id,
        name,
        owner_pubkey,
        channel_id,
        definition,
        status: "active".to_string(),
        created_at,
        updated_at,
    }
}

/// Convert a kind:30620 workflow definition event into a [`WorkflowWire`].
fn workflow_from_event(ev: &nostr::Event) -> WorkflowWire {
    let id = tag_value(ev, "d").unwrap_or_default();
    let channel_id = tag_value(ev, "h");
    let ts = ev.created_at.as_secs() as i64;
    workflow_record(id, channel_id, ev.pubkey.to_hex(), &ev.content, ts, ts)
}

#[cfg(test)]
#[path = "workflows_tests.rs"]
mod tests;
