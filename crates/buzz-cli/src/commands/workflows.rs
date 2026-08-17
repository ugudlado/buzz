use sha2::{Digest, Sha256};

use buzz_core::kind::KIND_WORKFLOW_DEF;
use buzz_core::marketplace::{FixedPrice, WorkflowMarketplace};
use buzz_workflow::{ActionDef, WorkflowDef, WorkflowEngine};

use crate::client::{
    extract_d_tag, extract_relay_response_field, normalize_write_response, print_create_response,
    BuzzClient,
};
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin, sdk_err, validate_uuid};
use crate::{WorkflowsCmd, WorkflowsMarketplaceCmd};

// TODO(phase-4): Replace raw nostr::EventBuilder usage with buzz-sdk builder functions

/// List workflows in a channel — query kind:30620 workflow definition events.
pub async fn cmd_list_workflows(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [30620],
        "#h": [channel_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let workflows: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "workflow_id": extract_d_tag(e),
                "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                "pubkey": e.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""),
            })
        })
        .collect();
    let output = serde_json::to_string(&workflows).unwrap_or_default();
    println!("{output}");
    Ok(())
}

/// Get a single workflow definition.
pub async fn cmd_get_workflow(client: &BuzzClient, workflow_id: &str) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    let filter = serde_json::json!({
        "kinds": [30620],
        "#d": [workflow_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    if let Some(e) = events.first() {
        let normalized = serde_json::json!({
            "workflow_id": extract_d_tag(e),
            "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
            "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
            "pubkey": e.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""),
        });
        println!("{normalized}");
    } else {
        println!("null");
    }
    Ok(())
}

/// Get workflow run history from the relay's existing authenticated run endpoint.
pub async fn cmd_get_workflow_runs(
    client: &BuzzClient,
    workflow_id: &str,
    run_id: Option<&str>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    if let Some(run_id) = run_id {
        validate_uuid(run_id)?;
    }
    let limit = if run_id.is_some() {
        1000
    } else {
        limit.unwrap_or(20).min(100)
    };
    let resp = client
        .get_authed(&format!("/api/workflows/{workflow_id}/runs?limit={limit}"))
        .await?;
    let runs: Vec<serde_json::Value> = serde_json::from_str(&resp)
        .map_err(|e| CliError::Other(format!("invalid workflow runs response: {e}")))?;
    if let Some(run_id) = run_id {
        let run = runs
            .into_iter()
            .find(|run| run.get("id").and_then(serde_json::Value::as_str) == Some(run_id))
            .ok_or_else(|| CliError::NotFound(format!("workflow run {run_id} not found")))?;
        println!("{run}");
    } else {
        println!("{}", serde_json::Value::Array(runs));
    }
    Ok(())
}

fn parse_workflow(content: &str) -> Result<WorkflowDef, CliError> {
    WorkflowEngine::parse_yaml(content)
        .map(|(definition, _)| definition)
        .map_err(|e| CliError::Other(format!("invalid workflow definition: {e}")))
}

fn workflow_content_with_marketplace(
    content: &str,
    marketplace: WorkflowMarketplace,
) -> Result<String, CliError> {
    let mut definition = parse_workflow(content)?;
    definition.marketplace = Some(
        marketplace
            .normalized()
            .map_err(|e| CliError::Usage(format!("invalid marketplace listing: {e}")))?,
    );
    definition
        .validate()
        .map_err(|e| CliError::Usage(format!("invalid workflow definition: {e}")))?;
    serde_json::to_string(&definition)
        .map_err(|e| CliError::Other(format!("failed to serialize workflow definition: {e}")))
}

fn event_tag<'a>(event: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    event
        .get("tags")?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_array)
        .find(|tag| tag.first().and_then(serde_json::Value::as_str) == Some(name))?
        .get(1)?
        .as_str()
}

async fn owned_workflow_event(
    client: &BuzzClient,
    workflow_id: &str,
) -> Result<serde_json::Value, CliError> {
    validate_uuid(workflow_id)?;
    let signer = client.keys().public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [KIND_WORKFLOW_DEF],
        "authors": [signer],
        "#d": [workflow_id],
        "limit": 1,
    });
    let response = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&response)
        .map_err(|e| CliError::Other(format!("invalid workflow query response: {e}")))?;
    let event = events.into_iter().next().ok_or_else(|| {
        CliError::NotFound(format!(
            "workflow {workflow_id} was not found with the current signer as author"
        ))
    })?;
    if event.get("pubkey").and_then(serde_json::Value::as_str) != Some(signer.as_str())
        || extract_d_tag(&event) != workflow_id
    {
        return Err(CliError::Auth(
            "current signer is not the workflow event author".into(),
        ));
    }
    Ok(event)
}

async fn cmd_marketplace_list(client: &BuzzClient) -> Result<(), CliError> {
    let events = client
        .query_all(serde_json::json!({"kinds": [KIND_WORKFLOW_DEF]}))
        .await?;
    let mut listings = Vec::new();
    for event in events {
        let Some(content) = event.get("content").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let definition = parse_workflow(content)?;
        let Some(marketplace) = definition.marketplace.as_ref() else {
            continue;
        };
        if !marketplace.listed {
            continue;
        }
        let agents: Vec<serde_json::Value> = definition
            .steps
            .iter()
            .filter_map(|step| match &step.action {
                ActionDef::AssignToAgent {
                    agent,
                    agent_pubkey,
                    ..
                } => Some(serde_json::json!({
                    "name": agent,
                    "pubkey": agent_pubkey,
                })),
                _ => None,
            })
            .collect();
        listings.push(serde_json::json!({
            "workflow_id": extract_d_tag(&event),
            "channel_id": event_tag(&event, "h"),
            "author_pubkey": event.get("pubkey").and_then(serde_json::Value::as_str),
            "name": definition.name,
            "description": definition.description,
            "marketplace": marketplace,
            "agents": agents,
            "created_at": event.get("created_at"),
        }));
    }
    println!("{}", serde_json::Value::Array(listings));
    Ok(())
}

async fn publish_workflow_listing(
    client: &BuzzClient,
    workflow_id: &str,
    marketplace: WorkflowMarketplace,
    event: &serde_json::Value,
) -> Result<(), CliError> {
    let channel_id = event_tag(event, "h")
        .ok_or_else(|| CliError::Other("workflow event is missing its h-tag".into()))?;
    let channel_id = parse_uuid(channel_id)?;
    let workflow_id = parse_uuid(workflow_id)?;
    let content = event
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::Other("workflow event is missing content".into()))?;
    let content = workflow_content_with_marketplace(content, marketplace)?;
    let builder =
        buzz_sdk::build_workflow_update(channel_id, workflow_id, &content).map_err(sdk_err)?;
    let response = client.submit_event(client.sign_event(builder)?).await?;
    println!("{}", normalize_write_response(&response));
    Ok(())
}

async fn cmd_marketplace_publish(
    client: &BuzzClient,
    workflow_id: &str,
    summary: Option<String>,
    fixed_price: Option<u64>,
    currency: Option<String>,
) -> Result<(), CliError> {
    let event = owned_workflow_event(client, workflow_id).await?;
    let content = event
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::Other("workflow event is missing content".into()))?;
    let definition = parse_workflow(content)?;
    let default_summary = definition
        .description
        .clone()
        .unwrap_or_else(|| definition.name.clone());
    let mut marketplace = definition.marketplace.unwrap_or(WorkflowMarketplace {
        listed: false,
        summary: default_summary,
        fixed_price: None,
        origin_event_id: None,
    });
    marketplace.listed = true;
    if let Some(summary) = summary {
        marketplace.summary = summary;
    }
    if let (Some(microunits), Some(currency)) = (fixed_price, currency) {
        marketplace.fixed_price = Some(FixedPrice {
            currency,
            microunits,
        });
    }
    publish_workflow_listing(client, workflow_id, marketplace, &event).await
}

async fn cmd_marketplace_unpublish(client: &BuzzClient, workflow_id: &str) -> Result<(), CliError> {
    let event = owned_workflow_event(client, workflow_id).await?;
    let content = event
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::Other("workflow event is missing content".into()))?;
    let mut marketplace = parse_workflow(content)?
        .marketplace
        .ok_or_else(|| CliError::Usage("workflow is not published".into()))?;
    marketplace.listed = false;
    publish_workflow_listing(client, workflow_id, marketplace, &event).await
}

/// Create a workflow — sign and submit a kind:30620 event.
pub async fn cmd_create_workflow(
    client: &BuzzClient,
    channel_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    let yaml_definition = read_or_stdin(yaml)?;

    let workflow_id = uuid::Uuid::new_v4();
    let builder = buzz_sdk::build_workflow_def(channel_uuid, workflow_id, &yaml_definition)
        .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    let final_workflow_id = extract_relay_response_field(&resp, "workflow_id")
        .unwrap_or_else(|| workflow_id.to_string());
    print_create_response(&resp, "workflow_id", &final_workflow_id);
    Ok(())
}

/// Update a workflow — sign and submit an updated kind:30620 event with same d-tag.
pub async fn cmd_update_workflow(
    client: &BuzzClient,
    channel_id: &str,
    workflow_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    let wf_uuid = parse_uuid(workflow_id)?;
    let yaml_definition = read_or_stdin(yaml)?;

    let builder = buzz_sdk::build_workflow_update(channel_uuid, wf_uuid, &yaml_definition)
        .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Delete a workflow — sign and submit a kind:5 deletion event.
pub async fn cmd_delete_workflow(client: &BuzzClient, workflow_id: &str) -> Result<(), CliError> {
    let wf_uuid = parse_uuid(workflow_id)?;
    let keys = client.keys();

    let builder =
        buzz_sdk::build_workflow_delete(&keys.public_key().to_hex(), wf_uuid).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Trigger a workflow — sign and submit a kind:46020 event.
///
/// When `inputs` is provided, it is parsed as a JSON object and used as the
/// event content (MCP parity). When omitted, the event content is `{}`.
pub async fn cmd_trigger_workflow(
    client: &BuzzClient,
    workflow_id: &str,
    inputs: Option<&str>,
) -> Result<(), CliError> {
    let wf_uuid = parse_uuid(workflow_id)?;

    if let Some(raw) = inputs {
        // Parse and validate it is a JSON object, then build the event manually
        // so we can embed the inputs as the event content.
        let parsed: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| CliError::Usage(format!("--inputs is not valid JSON: {e}")))?;
        if !parsed.is_object() {
            return Err(CliError::Usage("--inputs must be a JSON object".into()));
        }
        let content = serde_json::to_string(&parsed).unwrap_or_default();
        use nostr::{EventBuilder, Kind, Tag};
        let tags = vec![Tag::parse(["d", &wf_uuid.to_string()])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?];
        let builder = EventBuilder::new(
            Kind::Custom(buzz_sdk::kind::KIND_WORKFLOW_TRIGGER as u16),
            &content,
        )
        .tags(tags);
        let event = client.sign_event(builder)?;
        let resp = client.submit_event(event).await?;
        println!("{}", normalize_write_response(&resp));
    } else {
        let builder = buzz_sdk::build_workflow_trigger(wf_uuid).map_err(sdk_err)?;
        let event = client.sign_event(builder)?;
        let resp = client.submit_event(event).await?;
        println!("{}", normalize_write_response(&resp));
    }
    Ok(())
}

/// Cancel a workflow run that is currently waiting on an agent.
pub async fn cmd_cancel_workflow_run(client: &BuzzClient, run_id: &str) -> Result<(), CliError> {
    let run_id = parse_uuid(run_id)?;
    use nostr::{EventBuilder, Kind, Tag};
    let tags = vec![Tag::parse(["d", &run_id.to_string()])
        .map_err(|e| CliError::Other(format!("tag error: {e}")))?];
    let event = client.sign_event(
        EventBuilder::new(
            Kind::Custom(buzz_sdk::kind::KIND_WORKFLOW_CANCELLED as u16),
            "",
        )
        .tags(tags),
    )?;
    let response = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&response));
    Ok(())
}

/// Approve or deny a workflow step — sign and submit a kind:46030 (grant) or 46031 (deny) event.
pub async fn cmd_approve_step(
    client: &BuzzClient,
    approval_token: &str,
    approved: bool,
    note: Option<&str>,
) -> Result<(), CliError> {
    validate_uuid(approval_token)?;

    let content = note.unwrap_or("");

    // The relay expects d-tag = hex(SHA256(token)), not the raw token UUID.
    let token_hash = hex::encode(Sha256::digest(approval_token.as_bytes()));
    let builder =
        buzz_sdk::build_workflow_approval(&token_hash, approved, content).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn dispatch(cmd: crate::WorkflowsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        WorkflowsCmd::Marketplace { command } => match command {
            WorkflowsMarketplaceCmd::List => cmd_marketplace_list(client).await,
            WorkflowsMarketplaceCmd::Publish {
                workflow_id,
                summary,
                fixed_price,
                currency,
            } => {
                cmd_marketplace_publish(client, &workflow_id, summary, fixed_price, currency).await
            }
            WorkflowsMarketplaceCmd::Unpublish { workflow_id } => {
                cmd_marketplace_unpublish(client, &workflow_id).await
            }
        },
        WorkflowsCmd::List { channel } => cmd_list_workflows(client, &channel).await,
        WorkflowsCmd::Get { workflow } => cmd_get_workflow(client, &workflow).await,
        WorkflowsCmd::Create { channel, yaml } => {
            cmd_create_workflow(client, &channel, &yaml).await
        }
        WorkflowsCmd::Update {
            channel,
            workflow,
            yaml,
        } => cmd_update_workflow(client, &channel, &workflow, &yaml).await,
        WorkflowsCmd::Delete { workflow } => cmd_delete_workflow(client, &workflow).await,
        WorkflowsCmd::Trigger { workflow, inputs } => {
            cmd_trigger_workflow(client, &workflow, inputs.as_deref()).await
        }
        WorkflowsCmd::Runs {
            workflow,
            run,
            limit,
        } => cmd_get_workflow_runs(client, &workflow, run.as_deref(), limit).await,
        WorkflowsCmd::Cancel { run } => cmd_cancel_workflow_run(client, &run).await,
        WorkflowsCmd::Approve {
            token,
            approved,
            note,
        } => {
            // approved is already a bool — no parse_bool_flag needed
            cmd_approve_step(client, &token, approved, note.as_deref()).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKFLOW: &str = "name: Review\ndescription: Review changes\ntrigger:\n  on: manual\nsteps:\n  - id: review\n    action: assign_to_agent\n    agent: Reviewer\n    agent_pubkey: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n    instruction: Review it\n";

    #[test]
    fn marketplace_merge_changes_only_listing() {
        let merged = workflow_content_with_marketplace(
            WORKFLOW,
            WorkflowMarketplace {
                listed: true,
                summary: "Rust review".into(),
                fixed_price: Some(FixedPrice {
                    currency: "USD".into(),
                    microunits: 5_000_000,
                }),
                origin_event_id: None,
            },
        )
        .unwrap();
        let definition = parse_workflow(&merged).unwrap();

        assert_eq!(definition.name, "Review");
        assert_eq!(definition.description.as_deref(), Some("Review changes"));
        assert_eq!(definition.steps.len(), 1);
        let listing = definition.marketplace.unwrap();
        assert!(listing.listed);
        assert_eq!(listing.summary, "Rust review");
        assert_eq!(listing.fixed_price.unwrap().microunits, 5_000_000);
    }

    #[test]
    fn event_tag_reads_preserved_coordinates() {
        let event = serde_json::json!({
            "tags": [
                ["d", "11111111-1111-1111-1111-111111111111"],
                ["h", "22222222-2222-2222-2222-222222222222"]
            ]
        });
        assert_eq!(
            event_tag(&event, "d"),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(
            event_tag(&event, "h"),
            Some("22222222-2222-2222-2222-222222222222")
        );
    }
}
