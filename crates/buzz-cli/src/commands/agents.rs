use buzz_agent_record::{
    validate_respond_to_allowlist, ManagedAgentRecord, RespondTo, DEFAULT_ACP_COMMAND,
    DEFAULT_AGENT_PARALLELISM,
};
use buzz_core::kind::{KIND_IA_ARCHIVED_LIST, KIND_MANAGED_AGENT};
use buzz_core::marketplace::{AgentDeployment, AgentMarketplace, HourlyRate};
use buzz_sdk::builders::{build_archive_identity_request, build_unarchive_identity_request};
use nostr::{EventBuilder, Kind, PublicKey, Tag, ToBech32};
use serde_json::json;

use crate::agent_management::{build_create, build_update, CreateAgentDraft, UpdateAgentDraft};
use crate::client::{extract_d_tag, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{read_or_stdin, validate_hex64};
use crate::{AgentAccessArg, AgentsCmd, AgentsMarketplaceCmd, MarketplaceDeployment, RespondToArg};

pub async fn dispatch(command: AgentsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match command {
        AgentsCmd::Marketplace { command } => match command {
            AgentsMarketplaceCmd::List => cmd_marketplace_list(client).await,
            AgentsMarketplaceCmd::Publish {
                agent_pubkey,
                description,
                capabilities,
                deployment,
                rate,
                currency,
            } => {
                cmd_marketplace_publish(
                    client,
                    &agent_pubkey,
                    description,
                    capabilities,
                    deployment,
                    rate,
                    currency,
                )
                .await
            }
            AgentsMarketplaceCmd::Unpublish { agent_pubkey } => {
                cmd_marketplace_unpublish(client, &agent_pubkey).await
            }
        },
        AgentsCmd::Import {
            file,
            store_dir,
            identifier,
            dry_run,
            replace_pubkey,
            prune_unreferenced_definitions,
        } => import_agent(
            client,
            &file,
            store_dir.as_deref(),
            &identifier,
            dry_run,
            replace_pubkey.as_deref(),
            prune_unreferenced_definitions,
        ),

        AgentsCmd::Remove {
            pubkey,
            dry_run,
            store_dir,
            identifier,
        } => {
            let output = remove_agent(&pubkey, dry_run, store_dir.as_deref(), &identifier)?;
            println!("{output}");
            Ok(())
        }

        AgentsCmd::SetAccess {
            pubkey,
            respond_to,
            allowlist,
            store_dir,
            identifier,
        } => set_agent_access(
            &pubkey,
            respond_to,
            &allowlist,
            store_dir.as_deref(),
            &identifier,
        ),

        AgentsCmd::DraftCreate {
            channel,
            display_name,
            system_prompt,
        } => {
            let owner = require_owner(client)?;
            let built = build_create(
                client.keys(),
                &owner,
                CreateAgentDraft {
                    channel_id: channel,
                    display_name,
                    system_prompt: read_or_stdin(&system_prompt)?,
                },
            )?;
            let response = client.publish_ephemeral_event(built.event).await?;
            let mut output: serde_json::Value = serde_json::from_str(&response)
                .map_err(|e| CliError::Other(format!("invalid relay response: {e}")))?;
            if let Some(obj) = output.as_object_mut() {
                obj.insert("request_id".into(), built.request_id.into());
                obj.insert("action".into(), built.action.into());
                obj.insert("saved".into(), false.into());
                obj.insert(
                    "message".into(),
                    "Draft sent to Buzz Desktop for owner review. Nothing changes until the owner saves it."
                        .into(),
                );
            }
            println!("{output}");
            Ok(())
        }

        AgentsCmd::DraftUpdate {
            channel,
            agent_name,
            display_name,
            system_prompt,
            runtime,
            provider,
            model,
            respond_to,
        } => {
            let owner = require_owner(client)?;
            let built = build_update(
                client.keys(),
                &owner,
                UpdateAgentDraft {
                    channel_id: channel,
                    agent_name,
                    display_name,
                    system_prompt: system_prompt.map(|v| read_or_stdin(&v)).transpose()?,
                    runtime,
                    provider,
                    model,
                    respond_to: respond_to.map(RespondToArg::to_wire),
                },
            )?;
            let response = client.publish_ephemeral_event(built.event).await?;
            let mut output: serde_json::Value = serde_json::from_str(&response)
                .map_err(|e| CliError::Other(format!("invalid relay response: {e}")))?;
            if let Some(obj) = output.as_object_mut() {
                obj.insert("request_id".into(), built.request_id.into());
                obj.insert("action".into(), built.action.into());
                obj.insert("saved".into(), false.into());
                obj.insert(
                    "message".into(),
                    "Draft sent to Buzz Desktop for owner review. Nothing changes until the owner saves it."
                        .into(),
                );
            }
            println!("{output}");
            Ok(())
        }

        AgentsCmd::Archive {
            target_pubkey,
            reason,
            replaced_by,
            content,
            admin,
        } => {
            validate_hex64(&target_pubkey)?;
            let signer_hex = client.keys().public_key().to_hex();
            let auth = resolve_auth(
                client,
                &target_pubkey,
                &signer_hex,
                admin,
                &mut std::io::stderr(),
            )
            .await?;
            let builder = build_archive_identity_request(
                &target_pubkey,
                &content,
                reason.as_deref(),
                replaced_by.as_deref(),
                auth.as_ref(),
            )
            .map_err(|e| CliError::Usage(format!("invalid archive request: {e}")))?;
            let event = client.sign_event_unchecked(builder)?;
            let event_id = event.id.to_hex();
            client.submit_event(event).await?;
            println!(
                "{}",
                json!({
                    "ok": true,
                    "event_id": event_id,
                    "action": "archive",
                    "target": target_pubkey,
                })
            );
            Ok(())
        }

        AgentsCmd::Unarchive {
            target_pubkey,
            reason,
            content,
            admin,
        } => {
            validate_hex64(&target_pubkey)?;
            let signer_hex = client.keys().public_key().to_hex();
            let auth = resolve_auth(
                client,
                &target_pubkey,
                &signer_hex,
                admin,
                &mut std::io::stderr(),
            )
            .await?;
            let builder = build_unarchive_identity_request(
                &target_pubkey,
                &content,
                reason.as_deref(),
                auth.as_ref(),
            )
            .map_err(|e| CliError::Usage(format!("invalid unarchive request: {e}")))?;
            let event = client.sign_event_unchecked(builder)?;
            let event_id = event.id.to_hex();
            client.submit_event(event).await?;
            println!(
                "{}",
                json!({
                    "ok": true,
                    "event_id": event_id,
                    "action": "unarchive",
                    "target": target_pubkey,
                })
            );
            Ok(())
        }

        AgentsCmd::Archived => cmd_archived(client).await,
    }
}

const PUBLIC_AGENT_FIELDS: &[&str] = &[
    "name",
    "persona_id",
    "system_prompt",
    "model",
    "provider",
    "persona_source_version",
    "parallelism",
    "respond_to",
    "respond_to_allowlist",
    "marketplace",
];

fn sanitized_agent_content(
    content: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, CliError> {
    let mut content = serde_json::from_str::<serde_json::Value>(content)
        .map_err(|e| CliError::Other(format!("managed-agent content is not valid JSON: {e}")))?
        .as_object()
        .cloned()
        .ok_or_else(|| CliError::Other("managed-agent content must be a JSON object".into()))?;
    content.retain(|key, _| PUBLIC_AGENT_FIELDS.contains(&key.as_str()));
    Ok(content)
}

fn merge_agent_marketplace(
    content: &str,
    marketplace: AgentMarketplace,
) -> Result<String, CliError> {
    let mut content = sanitized_agent_content(content)?;
    content.insert(
        "marketplace".into(),
        serde_json::to_value(marketplace).map_err(|e| {
            CliError::Other(format!("failed to serialize marketplace listing: {e}"))
        })?,
    );
    serde_json::to_string(&content)
        .map_err(|e| CliError::Other(format!("failed to serialize managed-agent content: {e}")))
}

fn existing_agent_marketplace(content: &str) -> Result<Option<AgentMarketplace>, CliError> {
    sanitized_agent_content(content)?
        .remove("marketplace")
        .map(serde_json::from_value::<AgentMarketplace>)
        .transpose()
        .map_err(|e| CliError::Other(format!("invalid existing marketplace listing: {e}")))?
        .map(AgentMarketplace::normalized)
        .transpose()
        .map_err(|e| CliError::Other(format!("invalid existing marketplace listing: {e}")))
}

async fn owned_agent_event(
    client: &BuzzClient,
    agent_pubkey: &str,
) -> Result<serde_json::Value, CliError> {
    validate_hex64(agent_pubkey)?;
    let signer = client.keys().public_key().to_hex();
    let filter = json!({
        "kinds": [KIND_MANAGED_AGENT],
        "authors": [signer],
        "#d": [agent_pubkey],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid managed-agent query response: {e}")))?;
    let event = events.into_iter().next().ok_or_else(|| {
        CliError::NotFound(format!(
            "managed agent {agent_pubkey} was not found with the current signer as author"
        ))
    })?;
    if event.get("pubkey").and_then(serde_json::Value::as_str) != Some(signer.as_str())
        || extract_d_tag(&event) != agent_pubkey
    {
        return Err(CliError::Auth(
            "current signer is not the managed-agent event author".into(),
        ));
    }
    Ok(event)
}

fn agent_listing_event(
    client: &BuzzClient,
    event: &serde_json::Value,
    agent_pubkey: &str,
    marketplace: AgentMarketplace,
) -> Result<nostr::Event, CliError> {
    let content = event
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::Other("managed-agent event is missing content".into()))?;
    let content = merge_agent_marketplace(content, marketplace)?;
    let d_tag = Tag::parse(["d", agent_pubkey])
        .map_err(|e| CliError::Other(format!("invalid managed-agent d-tag: {e}")))?;
    client
        .sign_event(EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), content).tag(d_tag))
}

async fn cmd_marketplace_list(client: &BuzzClient) -> Result<(), CliError> {
    let events = client
        .query_all(json!({"kinds": [KIND_MANAGED_AGENT]}))
        .await?;
    let mut listings = Vec::new();
    for event in events {
        let Some(raw_content) = event.get("content").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let mut content = sanitized_agent_content(raw_content)?;
        let Some(marketplace) = content.remove("marketplace") else {
            continue;
        };
        let marketplace: AgentMarketplace = serde_json::from_value(marketplace)
            .map_err(|e| CliError::Other(format!("invalid marketplace listing: {e}")))?;
        let marketplace = marketplace
            .normalized()
            .map_err(|e| CliError::Other(format!("invalid marketplace listing: {e}")))?;
        if !marketplace.listed {
            continue;
        }
        content.insert("marketplace".into(), serde_json::json!(marketplace));
        content.insert("agent_pubkey".into(), extract_d_tag(&event).into());
        content.insert(
            "owner_pubkey".into(),
            event
                .get("pubkey")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
        content.insert(
            "created_at".into(),
            event
                .get("created_at")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
        listings.push(serde_json::Value::Object(content));
    }
    println!("{}", serde_json::Value::Array(listings));
    Ok(())
}

async fn cmd_marketplace_publish(
    client: &BuzzClient,
    agent_pubkey: &str,
    description: Option<String>,
    capabilities: Vec<String>,
    deployment: Option<MarketplaceDeployment>,
    rate: Option<u64>,
    currency: Option<String>,
) -> Result<(), CliError> {
    let event = owned_agent_event(client, agent_pubkey).await?;
    let content = event
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::Other("managed-agent event is missing content".into()))?;
    let mut listing = existing_agent_marketplace(content)?.unwrap_or(AgentMarketplace {
        listed: false,
        description: String::new(),
        capabilities: Vec::new(),
        deployment: AgentDeployment::Local,
        pricing: None,
    });
    listing.listed = true;
    if let Some(description) = description {
        listing.description = description;
    }
    if !capabilities.is_empty() {
        listing.capabilities = capabilities;
    }
    if let Some(deployment) = deployment {
        listing.deployment = match deployment {
            MarketplaceDeployment::Local => AgentDeployment::Local,
            MarketplaceDeployment::Remote => AgentDeployment::Remote,
            MarketplaceDeployment::Kubernetes => AgentDeployment::Kubernetes,
        };
    }
    if let (Some(rate), Some(currency)) = (rate, currency) {
        listing.pricing = Some(HourlyRate {
            currency,
            microunits_per_hour: rate,
        });
    }
    let listing = listing
        .normalized()
        .map_err(|e| CliError::Usage(format!("invalid marketplace listing: {e}")))?;
    let event = agent_listing_event(client, &event, agent_pubkey, listing)?;
    let response = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&response));
    Ok(())
}

async fn cmd_marketplace_unpublish(
    client: &BuzzClient,
    agent_pubkey: &str,
) -> Result<(), CliError> {
    let event = owned_agent_event(client, agent_pubkey).await?;
    let content = event
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::Other("managed-agent event is missing content".into()))?;
    let mut listing = existing_agent_marketplace(content)?
        .ok_or_else(|| CliError::Usage("managed agent is not published".into()))?;
    listing.listed = false;
    let event = agent_listing_event(client, &event, agent_pubkey, listing)?;
    let response = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&response));
    Ok(())
}

// ── `agents import` ──────────────────────────────────────────────────────────

/// Import a `buzz-agent-snapshot v1` JSON manifest directly into Buzz
/// Desktop's local `managed-agents.json`, mirroring what the desktop app's
/// Import dialog does for a `.agent.json` file, minus the pieces that need a
/// live desktop process (relay publish, avatar upload, memory restore — see
/// the command's `after_help`).
///
/// No owner attestation is written: the CLI cannot prove it holds the
/// desktop owner's key, so the record is stored without an NIP-OA auth tag
/// and the desktop applies its owner fallback on spawn. No event is signed
/// or sent to any relay.
///
/// With `dry_run`, the snapshot is validated and the resolved record is
/// printed as pretty JSON (private key redacted) without resolving or
/// touching the store — so dry-run works on machines with no desktop install.
fn import_agent(
    _client: &BuzzClient,
    file: &std::path::Path,
    store_dir: Option<&std::path::Path>,
    identifier: &str,
    dry_run: bool,
    replace_pubkey: Option<&str>,
    prune_unreferenced_definitions: bool,
) -> Result<(), CliError> {
    let bytes = std::fs::read(file)
        .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", file.display())))?;

    if file
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(|ext| !ext.eq_ignore_ascii_case("json"))
        .unwrap_or(true)
    {
        return Err(CliError::Usage(format!(
            "{} does not look like a .agent.json file — only the JSON snapshot format is \
             supported by `agents import` (not .agent.png)",
            file.display()
        )));
    }

    let snapshot: buzz_agent_record::AgentSnapshot = serde_json::from_slice(&bytes)
        .map_err(|e| CliError::Usage(format!("not a valid agent snapshot manifest: {e}")))?;

    if snapshot.format != buzz_agent_record::FORMAT_DISCRIMINATOR {
        return Err(CliError::Usage(format!(
            "unrecognized snapshot format '{}' (expected '{}')",
            snapshot.format,
            buzz_agent_record::FORMAT_DISCRIMINATOR
        )));
    }
    if snapshot.version != buzz_agent_record::FORMAT_VERSION {
        return Err(CliError::Usage(format!(
            "unsupported snapshot version {} (this build supports v{})",
            snapshot.version,
            buzz_agent_record::FORMAT_VERSION
        )));
    }
    if snapshot.memory.level == buzz_agent_record::MemoryLevel::None
        && !snapshot.memory.entries.is_empty()
    {
        return Err(CliError::Usage(
            "snapshot is malformed: memory.level is 'none' but entries are present".into(),
        ));
    }
    if !snapshot.memory.entries.is_empty() {
        eprintln!(
            "buzz agents import: warning: snapshot carries {} memory entries — \
             `agents import` does not restore memory; the agent starts with none.",
            snapshot.memory.entries.len()
        );
    }

    let display_name = snapshot.profile.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(CliError::Usage("snapshot display name is empty".into()));
    }

    let replace_pubkey = replace_pubkey
        .map(|value| {
            validate_hex64(value)?;
            Ok::<_, CliError>(value.to_ascii_lowercase())
        })
        .transpose()?;
    let needs_store = !dry_run || replace_pubkey.is_some();
    let (store_path, existing): (Option<std::path::PathBuf>, Vec<ManagedAgentRecord>) =
        if !needs_store {
            (None, Vec::new())
        } else {
            let store_path = resolve_store_path(store_dir, identifier)?;
            if !dry_run {
                refuse_if_desktop_running()?;
            }

            let existing = if store_path.exists() {
                let content = std::fs::read_to_string(&store_path).map_err(|e| {
                    CliError::Other(format!("failed to read {}: {e}", store_path.display()))
                })?;
                serde_json::from_str(&content).map_err(|e| {
                    CliError::Other(format!(
                        "{} is not valid JSON — refusing to write over a store this build cannot \
                     parse (desktop's own malformed-store guard did not fire because this is the \
                     CLI path): {e}",
                        store_path.display()
                    ))
                })?
            } else {
                Vec::new()
            };
            (Some(store_path), existing)
        };

    let respond_to = match snapshot.definition.respond_to.as_deref() {
        Some(wire) => Some(
            RespondTo::parse_wire(wire)
                .map_err(|e| CliError::Usage(format!("snapshot respond_to: {e}")))?,
        ),
        None => None,
    };
    let allowlist = validate_respond_to_allowlist(&snapshot.definition.respond_to_allowlist)
        .map_err(|e| CliError::Usage(format!("snapshot respond_to_allowlist: {e}")))?;
    if respond_to == Some(RespondTo::Allowlist) && allowlist.is_empty() {
        return Err(CliError::Usage(
            "snapshot respond-to mode is 'allowlist' but the allowlist is empty — cannot \
             import: no pubkeys to grant access to"
                .into(),
        ));
    }
    if !allowlist.is_empty() {
        eprintln!(
            "buzz agents import: warning: snapshot's respond-to allowlist came from a \
             different environment and is meaningless here; importing it unchanged. Edit the \
             agent's allowlist in Buzz Desktop after import if needed."
        );
    }
    let parallelism = snapshot
        .definition
        .parallelism
        .filter(|count| (1..=32).contains(count))
        .unwrap_or(DEFAULT_AGENT_PARALLELISM);

    let (runtime, model) = import_runtime_and_model(
        snapshot.definition.runtime.as_deref(),
        snapshot.definition.model.as_deref(),
    );

    if let Some(target_pubkey) = replace_pubkey.as_deref() {
        let store_path = store_path.as_ref().ok_or_else(|| {
            CliError::Other("store path unresolved for replacement import — this is a bug".into())
        })?;
        let result = replace_agent_from_snapshot(
            existing,
            &snapshot,
            &display_name,
            target_pubkey,
            runtime,
            model,
            parallelism,
            respond_to.unwrap_or_default(),
            allowlist,
            store_path,
            dry_run,
            prune_unreferenced_definitions,
        )?;
        println!("{result}");
        return Ok(());
    }

    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    if !dry_run && existing.iter().any(|r| r.pubkey == pubkey) {
        return Err(CliError::Other(format!(
            "generated pubkey {pubkey} already exists in the store — retry"
        )));
    }
    let private_key_nsec = agent_keys
        .secret_key()
        .to_bech32()
        .map_err(|e| CliError::Other(format!("failed to encode agent private key: {e}")))?;

    let now = chrono::Utc::now().to_rfc3339();
    let record = ManagedAgentRecord {
        pubkey: pubkey.clone(),
        name: display_name.clone(),
        persona_id: None,
        team_id: None,
        // Inline for now — desktop's `hydrate_keys` migrates this into the OS
        // keyring and blanks it here on the agent's first load, exactly as it
        // does for any record whose key arrives via the keyring-unreachable
        // fallback path. No keyring access is required from the CLI.
        private_key_nsec,
        // No auth tag: the CLI cannot prove it holds the desktop owner's key,
        // and a tag signed by the wrong key breaks relay auth. `None` makes
        // the desktop use its legacy owner fallback (`BUZZ_ACP_AGENT_OWNER`
        // from the workspace owner) on spawn — see `access_policy.rs`.
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: snapshot
            .profile
            .avatar_url
            .clone()
            .or(snapshot.profile.avatar_data_url.clone()),
        acp_command: DEFAULT_ACP_COMMAND.to_string(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: Vec::new(),
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: snapshot.definition.idle_timeout_seconds,
        max_turn_duration_seconds: snapshot.definition.max_turn_duration_seconds,
        parallelism,
        system_prompt: snapshot.definition.system_prompt.clone(),
        model,
        provider: snapshot.definition.provider.clone(),
        persona_source_version: None,
        env_vars: std::collections::BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: now.clone(),
        updated_at: now,
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: respond_to.unwrap_or_default(),
        respond_to_allowlist: allowlist,
        marketplace: None,
        display_name: None,
        slug: None,
        runtime,
        name_pool: snapshot.definition.name_pool.clone(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        definition_respond_to: snapshot.definition.respond_to.clone(),
        definition_respond_to_allowlist: snapshot.definition.respond_to_allowlist.clone(),
        definition_parallelism: snapshot.definition.parallelism,
        relay_mesh: None,
    };

    if dry_run {
        println!("{}", dry_run_record_json(&record)?);
        return Ok(());
    }
    let store_path = store_path.ok_or_else(|| {
        CliError::Other("store path unresolved outside dry-run — this is a bug".into())
    })?;

    let mut records = existing;
    records.push(record);
    if let Some(parent) = store_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CliError::Other(format!("failed to create {}: {e}", parent.display())))?;
    }
    let json = serde_json::to_string_pretty(&records)
        .map_err(|e| CliError::Other(format!("failed to serialize agent store: {e}")))?;
    std::fs::write(&store_path, json)
        .map_err(|e| CliError::Other(format!("failed to write {}: {e}", store_path.display())))?;

    println!(
        "{}",
        json!({
            "pubkey": pubkey,
            "name": display_name,
            "store_path": store_path.display().to_string(),
            "message": "Imported. Start Buzz Desktop to publish this agent's identity and \
                        profile, then add it to a channel.",
        })
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn replace_agent_from_snapshot(
    mut records: Vec<ManagedAgentRecord>,
    snapshot: &buzz_agent_record::AgentSnapshot,
    display_name: &str,
    target_pubkey: &str,
    runtime: Option<String>,
    model: Option<String>,
    parallelism: u32,
    respond_to: RespondTo,
    allowlist: Vec<String>,
    store_path: &std::path::Path,
    dry_run: bool,
    prune_unreferenced_definitions: bool,
) -> Result<serde_json::Value, CliError> {
    let target_index = records
        .iter()
        .position(|record| record.pubkey.eq_ignore_ascii_case(target_pubkey))
        .ok_or_else(|| {
            CliError::Usage(format!(
                "no managed agent with pubkey {target_pubkey} in {}",
                store_path.display()
            ))
        })?;
    if records[target_index].name != display_name {
        return Err(CliError::Usage(format!(
            "snapshot name '{display_name}' does not match agent {} named '{}'",
            target_pubkey, records[target_index].name
        )));
    }

    let same_name_instances = records
        .iter()
        .filter(|record| !record.pubkey.is_empty() && record.name == display_name)
        .count();
    if prune_unreferenced_definitions && same_name_instances > 1 {
        return Err(CliError::Usage(format!(
            "{same_name_instances} keyed agents are named '{display_name}'; refusing to guess which identities are duplicates. Remove unwanted pubkeys explicitly first"
        )));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let persona_id = records[target_index].persona_id.clone();
    apply_snapshot_config(
        &mut records[target_index],
        snapshot,
        display_name,
        runtime.clone(),
        model.clone(),
        parallelism,
        respond_to,
        allowlist.clone(),
        false,
        &now,
    );

    if let Some(persona_id) = persona_id.as_deref() {
        if let Some(definition) = records
            .iter_mut()
            .find(|record| record.pubkey.is_empty() && record.slug.as_deref() == Some(persona_id))
        {
            apply_snapshot_config(
                definition,
                snapshot,
                display_name,
                runtime,
                model,
                parallelism,
                respond_to,
                allowlist,
                true,
                &now,
            );
        }
    }

    let referenced_definitions: std::collections::HashSet<String> = records
        .iter()
        .filter(|record| !record.pubkey.is_empty())
        .filter_map(|record| record.persona_id.clone())
        .collect();
    let before = records.len();
    if prune_unreferenced_definitions {
        records.retain(|record| {
            !record.pubkey.is_empty()
                || record.name != display_name
                || record
                    .slug
                    .as_ref()
                    .is_some_and(|slug| referenced_definitions.contains(slug))
        });
    }
    let pruned_definitions = before - records.len();

    let backup_path = store_path.with_file_name("managed-agents.json.pre-roster-import.bak");
    if !dry_run {
        if !backup_path.exists() {
            std::fs::copy(store_path, &backup_path).map_err(|e| {
                CliError::Other(format!(
                    "failed to back up {} to {}: {e}",
                    store_path.display(),
                    backup_path.display()
                ))
            })?;
        }
        let json = serde_json::to_string_pretty(&records)
            .map_err(|e| CliError::Other(format!("failed to serialize agent store: {e}")))?;
        std::fs::write(store_path, json).map_err(|e| {
            CliError::Other(format!("failed to write {}: {e}", store_path.display()))
        })?;
    }

    Ok(json!({
        "pubkey": target_pubkey,
        "name": display_name,
        "updated": true,
        "dry_run": dry_run,
        "pruned_definitions": pruned_definitions,
        "store_path": store_path.display().to_string(),
        "backup_path": backup_path.display().to_string(),
        "message": "Updated the pinned agent identity. Start Buzz Desktop to publish the refreshed profile and managed-agent event."
    }))
}

#[allow(clippy::too_many_arguments)]
fn apply_snapshot_config(
    record: &mut ManagedAgentRecord,
    snapshot: &buzz_agent_record::AgentSnapshot,
    display_name: &str,
    runtime: Option<String>,
    model: Option<String>,
    parallelism: u32,
    respond_to: RespondTo,
    allowlist: Vec<String>,
    is_definition: bool,
    now: &str,
) {
    record.name = display_name.to_string();
    record.display_name = is_definition.then(|| display_name.to_string());
    record.system_prompt = snapshot.definition.system_prompt.clone();
    record.runtime = runtime;
    record.model = model;
    record.provider = snapshot.definition.provider.clone();
    record.parallelism = parallelism;
    record.respond_to = respond_to;
    record.respond_to_allowlist = allowlist;
    record.definition_respond_to = snapshot.definition.respond_to.clone();
    record.definition_respond_to_allowlist = snapshot.definition.respond_to_allowlist.clone();
    record.definition_parallelism = snapshot.definition.parallelism;
    record.name_pool = snapshot.definition.name_pool.clone();
    record.idle_timeout_seconds = snapshot.definition.idle_timeout_seconds;
    record.max_turn_duration_seconds = snapshot.definition.max_turn_duration_seconds;
    record.avatar_url = snapshot
        .profile
        .avatar_url
        .clone()
        .or(snapshot.profile.avatar_data_url.clone());
    record.updated_at = now.to_string();
}

/// Render `record` as the `--dry-run` output: pretty JSON with the generated
/// agent private key blanked. The record's serde attribute
/// (`skip_serializing_if = "String::is_empty"`) then omits the field from the
/// output entirely, so the secret never reaches stdout or CI logs.
fn dry_run_record_json(record: &ManagedAgentRecord) -> Result<String, CliError> {
    let mut redacted = record.clone();
    redacted.private_key_nsec = String::new();
    serde_json::to_string_pretty(&redacted)
        .map_err(|e| CliError::Other(format!("failed to serialize agent record: {e}")))
}

/// Harness + model an imported agent record gets.
///
/// ponytail: temporary default — CLI-imported agents run on the Cursor
/// harness ("cursor" preset) instead of Claude for now. A snapshot pinning
/// any runtime other than claude keeps it (and its model); claude/unset
/// becomes cursor with the model cleared so Cursor's own default model
/// applies (a Claude model id is meaningless to another harness). Drop this
/// override when Claude harness is the desired import default again.
fn import_runtime_and_model(
    runtime: Option<&str>,
    model: Option<&str>,
) -> (Option<String>, Option<String>) {
    match runtime {
        None | Some("claude") => (Some("cursor".to_string()), None),
        Some(other) => (Some(other.to_string()), model.map(str::to_string)),
    }
}

/// Remove a managed agent record by pubkey from the local store. The inverse
/// of `import_agent` for CLI-managed stores — keeps store maintenance in the
/// CLI instead of hand-editing the JSON.
///
/// `--dry-run` shares the real path's pubkey/store validation (including the
/// Desktop-running guard), returns a preview JSON value, and never writes.
fn remove_agent(
    pubkey: &str,
    dry_run: bool,
    store_dir: Option<&std::path::Path>,
    identifier: &str,
) -> Result<serde_json::Value, CliError> {
    validate_hex64(pubkey)?;
    let store_path = resolve_store_path(store_dir, identifier)?;
    refuse_if_desktop_running()?;

    let content = std::fs::read_to_string(&store_path)
        .map_err(|e| CliError::Other(format!("failed to read {}: {e}", store_path.display())))?;
    let records: Vec<ManagedAgentRecord> = serde_json::from_str(&content).map_err(|e| {
        CliError::Other(format!(
            "{} is not valid JSON — refusing to rewrite a store this build cannot parse: {e}",
            store_path.display()
        ))
    })?;

    let name = records
        .iter()
        .find(|r| r.pubkey == pubkey)
        .map(|r| r.name.clone())
        .ok_or_else(|| {
            CliError::Usage(format!(
                "no record with pubkey {pubkey} in {}",
                store_path.display()
            ))
        })?;

    if dry_run {
        return Ok(json!({
            "pubkey": pubkey,
            "name": name,
            "store_path": store_path.display().to_string(),
            "dry_run": true,
            "message": "Would remove agent from store; managed-agents.json unchanged.",
        }));
    }

    let before = records.len();
    let remaining: Vec<ManagedAgentRecord> =
        records.into_iter().filter(|r| r.pubkey != pubkey).collect();
    let json = serde_json::to_string_pretty(&remaining)
        .map_err(|e| CliError::Other(format!("failed to serialize agent store: {e}")))?;
    std::fs::write(&store_path, json)
        .map_err(|e| CliError::Other(format!("failed to write {}: {e}", store_path.display())))?;

    Ok(json!({
        "pubkey": pubkey,
        "removed": before - remaining.len(),
        "store_path": store_path.display().to_string(),
    }))
}

fn set_agent_access(
    pubkey: &str,
    respond_to: AgentAccessArg,
    allowlist: &[String],
    store_dir: Option<&std::path::Path>,
    identifier: &str,
) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    let store_path = resolve_store_path(store_dir, identifier)?;
    refuse_if_desktop_running()?;
    let content = std::fs::read_to_string(&store_path)
        .map_err(|e| CliError::Other(format!("failed to read {}: {e}", store_path.display())))?;
    let mut records: Vec<ManagedAgentRecord> = serde_json::from_str(&content).map_err(|e| {
        CliError::Other(format!(
            "{} is not valid JSON — refusing to rewrite a store this build cannot parse: {e}",
            store_path.display()
        ))
    })?;
    let normalized = validate_respond_to_allowlist(allowlist)
        .map_err(|e| CliError::Usage(format!("invalid allowlist: {e}")))?;
    let mode = match respond_to {
        AgentAccessArg::OwnerOnly => RespondTo::OwnerOnly,
        AgentAccessArg::Allowlist => RespondTo::Allowlist,
        AgentAccessArg::Anyone => RespondTo::Anyone,
    };
    if mode == RespondTo::Allowlist && normalized.is_empty() {
        return Err(CliError::Usage(
            "--respond-to allowlist requires at least one --allow pubkey".into(),
        ));
    }
    if mode != RespondTo::Allowlist && !normalized.is_empty() {
        return Err(CliError::Usage(
            "--allow is only valid with --respond-to allowlist".into(),
        ));
    }
    let record = records
        .iter_mut()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| {
            CliError::Usage(format!(
                "no record with pubkey {pubkey} in {}",
                store_path.display()
            ))
        })?;
    record.respond_to = mode;
    record.respond_to_allowlist = normalized;
    let saved_mode = record.respond_to;
    let allowlist_count = record.respond_to_allowlist.len();
    let json = serde_json::to_string_pretty(&records)
        .map_err(|e| CliError::Other(format!("failed to serialize agent store: {e}")))?;
    std::fs::write(&store_path, json)
        .map_err(|e| CliError::Other(format!("failed to write {}: {e}", store_path.display())))?;
    println!(
        "{}",
        json!({
            "pubkey": pubkey,
            "respond_to": saved_mode,
            "allowlist_count": allowlist_count,
            "store_path": store_path.display().to_string(),
        })
    );
    Ok(())
}

/// Resolve `managed-agents.json`'s directory the same way Tauri's
/// `app_data_dir()` would for `identifier` on this platform, unless
/// overridden by `--store-dir`.
fn resolve_store_path(
    store_dir: Option<&std::path::Path>,
    identifier: &str,
) -> Result<std::path::PathBuf, CliError> {
    let base = match store_dir {
        Some(dir) => dir.to_path_buf(),
        None => {
            let data_dir = dirs::data_dir().ok_or_else(|| {
                CliError::Other("could not resolve this platform's app-data directory".into())
            })?;
            data_dir.join(identifier)
        }
    };
    Ok(base.join("agents").join("managed-agents.json"))
}

/// Refuse to run while Buzz Desktop is alive: it holds `managed-agents.json`
/// in memory and rewrites it wholesale on several code paths (including
/// boot-time reconcile), which would silently discard this import. There is
/// no cross-process lock to share, so "the app must be quit" is the contract.
#[cfg(target_os = "macos")]
fn refuse_if_desktop_running() -> Result<(), CliError> {
    // Unit tests exercise the store path with tempdirs; skip the live process
    // probe so a running desktop on the developer machine doesn't fail them.
    if cfg!(test) {
        return Ok(());
    }
    let output = std::process::Command::new("pgrep")
        .arg("-x")
        .arg("buzz-desktop")
        .output()
        .map_err(|e| CliError::Other(format!("failed to check for a running desktop app: {e}")))?;
    if output.status.success() && !output.stdout.is_empty() {
        return Err(CliError::Usage(
            "Buzz Desktop is running — quit it first. It rewrites managed-agents.json on \
             launch and on several save paths, which would discard this import."
                .into(),
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn refuse_if_desktop_running() -> Result<(), CliError> {
    // TODO: pgrep-equivalent process check for Linux/Windows when the desktop
    // ships there under `agents import`. Until then this is a soft no-op —
    // the store-corruption risk is unchanged from before this command existed
    // (hand-editing the file while the app runs was always unsafe).
    Ok(())
}

/// Require `BUZZ_AUTH_TAG` and parse the owner pubkey from it. Used only by
/// the `draft-create` and `draft-update` paths.
fn require_owner(client: &BuzzClient) -> Result<PublicKey, CliError> {
    let hex = client
        .auth_tag_owner_hex()
        .ok_or_else(|| CliError::Auth("agent draft requests require BUZZ_AUTH_TAG".into()))?;
    PublicKey::parse(&hex).map_err(|e| CliError::Auth(format!("invalid owner attestation: {e}")))
}

/// Typed reason why NIP-OA owner-auth could not be extracted.
///
/// Covers all distinguishable failure causes from profile fetch through tag
/// validation so the diagnostic in [`resolve_auth`] is always precise and
/// never duplicates classification logic.
#[derive(Debug, PartialEq)]
enum AuthFailure {
    /// No kind:0 profile was found for the target; target pubkey included.
    NoProfile(String),
    /// kind:0 was found but has no `tags` array; target pubkey included.
    NoTagsArray(String),
    /// `tags` array has no `auth`-labelled entries.
    NoAuthTag,
    /// `tags` array has more than one `auth`-labelled entry; count included.
    AmbiguousAuthTag(usize),
    /// Sole `auth` tag has wrong element count; actual count included.
    WrongArity(usize),
    /// Sole `auth` tag contains a non-string element.
    NonStringElement,
    /// Sole `auth` tag owner field is not a valid 64-hex pubkey; value included.
    InvalidOwnerHex(String),
    /// Sole `auth` tag sig field is not a valid 128-hex signature.
    InvalidSigHex,
    /// Tag is structurally valid but names a different owner; actual owner included.
    OwnerMismatch(String),
}

impl AuthFailure {
    /// Human-readable description suitable for the `"warning"` JSON field.
    fn message(&self) -> String {
        match self {
            AuthFailure::NoProfile(target) => {
                format!("no kind:0 profile found for target {target}")
            }
            AuthFailure::NoTagsArray(target) => {
                format!("target {target} kind:0 has no tags array")
            }
            AuthFailure::NoAuthTag => "target kind:0 has no \"auth\" tag".to_owned(),
            AuthFailure::AmbiguousAuthTag(n) => format!(
                "target kind:0 has {n} \"auth\" tags (expected exactly 1) — ambiguous ownership"
            ),
            AuthFailure::WrongArity(n) => format!(
                "sole \"auth\" tag has {n} element(s) (expected 4: label, owner, conditions, sig)"
            ),
            AuthFailure::NonStringElement => {
                "sole \"auth\" tag contains a non-string element".to_owned()
            }
            AuthFailure::InvalidOwnerHex(v) => {
                format!("sole \"auth\" tag owner field is not a valid 64-hex pubkey: {v}")
            }
            AuthFailure::InvalidSigHex => {
                "sole \"auth\" tag sig field is not a valid 128-hex signature".to_owned()
            }
            AuthFailure::OwnerMismatch(actual) => {
                format!("sole \"auth\" tag names owner {actual} which does not match your key")
            }
        }
    }
}

/// Single classifier: either extract the auth tag or return the typed reason
/// for failure. [`extract_owner_auth_tag`] is a thin `.ok()` wrapper kept for
/// the existing tests that assert on `Option`.
fn classify_owner_auth_tag(
    tags: &[serde_json::Value],
    signer_hex: &str,
) -> Result<[String; 4], AuthFailure> {
    let auth_tags: Vec<&serde_json::Value> = tags
        .iter()
        .filter(|tag| {
            tag.as_array()
                .and_then(|elems| elems.first())
                .and_then(|v| v.as_str())
                == Some("auth")
        })
        .collect();
    match auth_tags.len() {
        0 => return Err(AuthFailure::NoAuthTag),
        n if n > 1 => return Err(AuthFailure::AmbiguousAuthTag(n)),
        _ => {}
    }

    // Exactly one auth tag.
    let elems = auth_tags[0]
        .as_array()
        .ok_or(AuthFailure::NonStringElement)?;
    if elems.len() != 4 {
        return Err(AuthFailure::WrongArity(elems.len()));
    }
    let label = elems[0].as_str().ok_or(AuthFailure::NonStringElement)?;
    let owner = elems[1].as_str().ok_or(AuthFailure::NonStringElement)?;
    let conditions = elems[2].as_str().ok_or(AuthFailure::NonStringElement)?;
    let sig = elems[3].as_str().ok_or(AuthFailure::NonStringElement)?;
    if owner.len() != 64 || !owner.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AuthFailure::InvalidOwnerHex(owner.to_owned()));
    }
    if sig.len() != 128 || !sig.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AuthFailure::InvalidSigHex);
    }
    if !owner.eq_ignore_ascii_case(signer_hex) {
        return Err(AuthFailure::OwnerMismatch(owner.to_owned()));
    }
    Ok([
        label.to_owned(),
        owner.to_owned(),
        conditions.to_owned(),
        sig.to_owned(),
    ])
}

/// Pure typed extractor: given a fetched kind:0 profile (or `None` when no
/// event was found), return the auth tag or the typed failure reason.
///
/// Separated from [`resolve_auth`] so pure-unit tests can exercise all
/// failure cases without a live `BuzzClient` or async runtime.
fn extract_auth(
    profile: Option<&serde_json::Value>,
    target_hex: &str,
    signer_hex: &str,
) -> Result<[String; 4], AuthFailure> {
    let event = profile.ok_or_else(|| AuthFailure::NoProfile(target_hex.to_owned()))?;
    let tags = event
        .get("tags")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AuthFailure::NoTagsArray(target_hex.to_owned()))?;
    classify_owner_auth_tag(tags, signer_hex)
}

/// Resolve the optional NIP-OA `auth` tag for archive/unarchive requests,
/// with one automatic retry on extraction failure.
///
/// Resolution logic (linear state machine):
/// - `target == signer`: self path — no auth needed → `Ok(None)`, silent, zero fetches.
/// - Otherwise: fetch target's kind:0 and attempt extraction.
///   - Success (attempt 1) → `Ok(Some(tag))`, one fetch.
///   - Failure (attempt 1) → fetch again once (transient republish is the
///     dominant cause), then attempt extraction again.
///     - Success (attempt 2) → `Ok(Some(tag))`, two fetches.
///     - Failure (attempt 2), `allow_bare == false` → `Err(CliError::Usage)`
///       with an actionable message naming the reason. Request is NOT sent.
///     - Failure (attempt 2), `allow_bare == true` → emit one
///       `{"warning":"..."}` line to `warn_sink`, return `Ok(None)` (bare
///       send) for relay-admin callers.
/// - Network/parse failures surface as `Err` regardless of `allow_bare`.
async fn resolve_auth(
    client: &BuzzClient,
    target_hex: &str,
    signer_hex: &str,
    allow_bare: bool,
    warn_sink: &mut dyn std::io::Write,
) -> Result<Option<[String; 4]>, CliError> {
    if target_hex.eq_ignore_ascii_case(signer_hex) {
        return Ok(None);
    }

    // Attempt 1.
    let profile = fetch_kind0(client, target_hex).await?;
    if let Ok(tag) = extract_auth(profile.as_ref(), target_hex, signer_hex) {
        return Ok(Some(tag));
    }

    // Attempt 2 — one retry for transient republish churn.
    let profile = fetch_kind0(client, target_hex).await?;
    match extract_auth(profile.as_ref(), target_hex, signer_hex) {
        Ok(tag) => Ok(Some(tag)),
        Err(failure) => {
            let detail = failure.message();
            if allow_bare {
                let msg = format!(
                    "{detail}; proceeding without owner attestation (--admin) — \
                     this succeeds only if your key is a relay admin"
                );
                let _ = writeln!(warn_sink, "{}", serde_json::json!({"warning": msg}));
                Ok(None)
            } else {
                Err(CliError::Usage(format!(
                    "{detail}; refusing to send a bare request that the relay will reject — \
                     re-run once the target's profile has finished publishing, or pass --admin \
                     if your key is a relay admin"
                )))
            }
        }
    }
}

/// Fetch the most-recent kind:0 for `target_hex` from the relay.
/// Returns `None` when no event was found, `Err` on network/parse failure.
async fn fetch_kind0(
    client: &BuzzClient,
    target_hex: &str,
) -> Result<Option<serde_json::Value>, CliError> {
    let filter = json!({"kinds": [0], "authors": [target_hex], "limit": 1});
    let raw = client
        .query(&filter)
        .await
        .map_err(|e| CliError::Other(format!("failed to fetch target kind:0: {e}")))?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid kind:0 query response: {e}")))?;
    Ok(events.into_iter().next())
}

/// Pure extraction helper: require exactly one kind:0 tag whose first
/// element is `"auth"` (a set-level rule — a valid tag alongside a second
/// malformed or duplicate `auth`-labeled tag is bare, not the valid one),
/// then structurally validate that sole tag as
/// `["auth", owner, conditions, sig]` matching `signer_hex`.
///
/// Thin wrapper around [`classify_owner_auth_tag`] that collapses the typed
/// failure reason to `None`. Malformed tags → `None`; valid tag → `Some`.
#[cfg(test)]
fn extract_owner_auth_tag(tags: &[serde_json::Value], signer_hex: &str) -> Option<[String; 4]> {
    classify_owner_auth_tag(tags, signer_hex).ok()
}

/// Validate the NIP-11 relay-info `self` field is a 64-hex pubkey and
/// normalize it to lowercase, so the archived-identities query filter and
/// the author comparison in [`verify_archived_event`] agree regardless of
/// the case the relay published `self` in.
fn normalize_relay_self_hex(self_hex: &str) -> Result<String, CliError> {
    if self_hex.len() != 64 || !self_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CliError::Other(format!(
            "relay 'self' field is not a valid 64-hex pubkey: {self_hex}"
        )));
    }
    Ok(self_hex.to_ascii_lowercase())
}

/// Fetch and verify the relay's NIP-IA archived-identities snapshot (kind
/// 13535). Shared by `cmd_archived` (trust failures are fatal — verifying
/// repair state is the command's whole purpose) and the `--template`
/// resolver's archive filter, which fails open on a trust failure instead
/// (see `channels::resolve_roster_with_archive_filter`'s doc comment for
/// why).
///
/// Three trust states:
/// - State 1: no events — `Ok(vec![])`
/// - State 2: event passes all checks — `Ok(<pubkeys>)`
/// - State 3: trust failure — `Err`, naming the specific failure
pub(crate) async fn fetch_archived_snapshot(client: &BuzzClient) -> Result<Vec<String>, CliError> {
    // Fetch NIP-11 info to get the relay's self pubkey.
    let nip11_raw = client
        .get_public("/")
        .await
        .map_err(|e| CliError::Other(format!("failed to fetch relay info document: {e}")))?;
    let nip11: serde_json::Value = serde_json::from_str(&nip11_raw)
        .map_err(|e| CliError::Other(format!("relay info document is not valid JSON: {e}")))?;
    let self_hex = nip11
        .get("self")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CliError::Other("relay info document missing 'self' field".into()))?;
    let self_hex = normalize_relay_self_hex(self_hex)?;

    // Query for the archived-identities list.
    let filter = json!({"kinds": [KIND_IA_ARCHIVED_LIST], "authors": [self_hex], "limit": 1});
    let raw = client
        .query(&filter)
        .await
        .map_err(|e| CliError::Other(format!("failed to query archived-identities list: {e}")))?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid query response: {e}")))?;

    // State 1: no events.
    if events.is_empty() {
        return Ok(Vec::new());
    }

    // State 2 or 3: verify then collect.
    let raw_event = events.into_iter().next().unwrap();
    let event: nostr::Event = serde_json::from_value(raw_event)
        .map_err(|e| CliError::Other(format!("archived-identities event is malformed: {e}")))?;
    let archived = verify_archived_event(&event, &self_hex)?;

    Ok(archived.into_iter().map(str::to_string).collect())
}

/// `buzz agents archived`: read path over [`fetch_archived_snapshot`] for
/// direct invocation — a trust failure (state 3) is fatal here so a
/// verification command can never look like success.
async fn cmd_archived(client: &BuzzClient) -> Result<(), CliError> {
    let archived = fetch_archived_snapshot(client).await?;
    println!("{}", json!({"archived": archived}));
    Ok(())
}

/// Pure verification of a kind:13535 archived-identities event.
///
/// Returns the list of valid hex64 pubkeys from `p` tags on success, or a
/// named trust-failure error (State 3).
fn verify_archived_event<'a>(
    event: &'a nostr::Event,
    relay_self_hex: &str,
) -> Result<Vec<&'a str>, CliError> {
    if event.kind != nostr::Kind::Custom(KIND_IA_ARCHIVED_LIST as u16) {
        return Err(CliError::Other(format!(
            "archived-identities event has wrong kind: {}",
            event.kind.as_u16()
        )));
    }

    if event.pubkey.to_hex() != relay_self_hex {
        return Err(CliError::Other(format!(
            "archived-identities event author {} does not match relay self {}",
            event.pubkey.to_hex(),
            relay_self_hex
        )));
    }

    let mut nip70_count = 0usize;
    for t in event.tags.iter() {
        let s = t.as_slice();
        if s.first().map(String::as_str) != Some("-") {
            continue;
        }
        if s.len() != 1 {
            return Err(CliError::Other(
                "archived-identities event has a malformed NIP-70 '-' tag (expected arity 1)"
                    .into(),
            ));
        }
        nip70_count += 1;
    }
    if nip70_count != 1 {
        return Err(CliError::Other(format!(
            "archived-identities event must have exactly one NIP-70 '-' tag, found {nip70_count}"
        )));
    }

    event.verify().map_err(|e| {
        CliError::Other(format!(
            "archived-identities event failed cryptographic verification: {e}"
        ))
    })?;

    let archived: Vec<&str> = event
        .tags
        .iter()
        .filter_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) == Some("p") {
                let pk = s.get(1).map(String::as_str)?;
                if pk.len() == 64 && pk.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some(pk);
                }
            }
            None
        })
        .collect();

    Ok(archived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::KIND_IA_ARCHIVED_LIST;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use serde_json::json;

    fn hex64(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    #[test]
    fn marketplace_merge_preserves_public_projection_and_drops_private_fields() {
        let content = json!({
            "name": "Reviewer",
            "persona_id": "reviewer",
            "parallelism": 2,
            "respond_to": "anyone",
            "private_key_nsec": "nsec-secret",
            "env_vars": {"TOKEN": "secret"},
            "marketplace": {"listed": false, "description": "old", "capabilities": [], "deployment": "local"}
        });
        let listing = AgentMarketplace {
            listed: true,
            description: "Reviews Rust".into(),
            capabilities: vec!["rust".into()],
            deployment: AgentDeployment::Remote,
            pricing: Some(HourlyRate {
                currency: "USD".into(),
                microunits_per_hour: 12_000_000,
            }),
        };

        let merged: serde_json::Value =
            serde_json::from_str(&merge_agent_marketplace(&content.to_string(), listing).unwrap())
                .unwrap();

        assert_eq!(merged["name"], "Reviewer");
        assert_eq!(merged["persona_id"], "reviewer");
        assert_eq!(merged["parallelism"], 2);
        assert_eq!(merged["marketplace"]["description"], "Reviews Rust");
        assert!(merged.get("private_key_nsec").is_none());
        assert!(merged.get("env_vars").is_none());
    }

    // --- `agents import --dry-run` ---

    fn offline_client() -> BuzzClient {
        BuzzClient::new("http://localhost".into(), Keys::generate(), None, None)
            .expect("offline client construction cannot fail")
    }

    fn valid_snapshot_json() -> serde_json::Value {
        json!({
            "format": "buzz-agent-snapshot",
            "version": 1,
            "definition": { "name": "test-agent" },
            "profile": { "displayName": "Test Agent" },
            "memory": { "level": "none" }
        })
    }

    fn write_snapshot(dir: &std::path::Path, snapshot: &serde_json::Value) -> std::path::PathBuf {
        let path = dir.join("test.agent.json");
        std::fs::write(&path, snapshot.to_string()).expect("write snapshot");
        path
    }

    #[test]
    fn dry_run_valid_snapshot_ok_and_no_store_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write_snapshot(dir.path(), &valid_snapshot_json());
        let store_dir = dir.path().join("store");

        let result = import_agent(
            &offline_client(),
            &file,
            Some(&store_dir),
            "test.id",
            true,
            None,
            false,
        );

        assert!(result.is_ok(), "dry-run should succeed: {result:?}");
        assert!(
            !store_dir.exists(),
            "dry-run must not create the store directory"
        );
        assert!(!store_dir
            .join("agents")
            .join("managed-agents.json")
            .exists());
    }

    #[test]
    fn dry_run_wrong_format_returns_existing_usage_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut snapshot = valid_snapshot_json();
        snapshot["format"] = json!("not-a-buzz-snapshot");
        let file = write_snapshot(dir.path(), &snapshot);

        let err = import_agent(
            &offline_client(),
            &file,
            Some(dir.path()),
            "test.id",
            true,
            None,
            false,
        )
        .expect_err("wrong format must fail");
        match err {
            CliError::Usage(msg) => assert_eq!(
                msg,
                "unrecognized snapshot format 'not-a-buzz-snapshot' (expected 'buzz-agent-snapshot')"
            ),
            other => panic!("expected CliError::Usage, got {other:?}"),
        }
    }

    #[test]
    fn dry_run_empty_display_name_returns_existing_usage_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut snapshot = valid_snapshot_json();
        snapshot["profile"]["displayName"] = json!("   ");
        let file = write_snapshot(dir.path(), &snapshot);

        let err = import_agent(
            &offline_client(),
            &file,
            Some(dir.path()),
            "test.id",
            true,
            None,
            false,
        )
        .expect_err("empty display name must fail");
        match err {
            CliError::Usage(msg) => assert_eq!(msg, "snapshot display name is empty"),
            other => panic!("expected CliError::Usage, got {other:?}"),
        }
    }

    #[test]
    fn dry_run_record_json_redacts_private_key() {
        let record: ManagedAgentRecord = serde_json::from_value(json!({
            "pubkey": hex64('a'),
            "name": "Test Agent",
            "private_key_nsec": "nsec1exampleexampleexample",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 0,
            "system_prompt": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }))
        .expect("minimal record deserializes");

        let output = dry_run_record_json(&record).expect("serialization succeeds");
        assert!(
            !output.contains("private_key_nsec"),
            "dry-run output must not contain the private key field: {output}"
        );
        assert!(output.contains("\"pubkey\""));
    }

    #[test]
    fn import_defaults_claude_and_unset_runtimes_to_cursor() {
        assert_eq!(
            import_runtime_and_model(None, Some("claude-sonnet-5")),
            (Some("cursor".into()), None)
        );
        assert_eq!(
            import_runtime_and_model(Some("claude"), Some("claude-sonnet-5")),
            (Some("cursor".into()), None)
        );
        // Any other pinned harness keeps both runtime and model.
        assert_eq!(
            import_runtime_and_model(Some("goose"), Some("gpt-5")),
            (Some("goose".into()), Some("gpt-5".into()))
        );
    }

    fn hex128(c: char) -> String {
        std::iter::repeat_n(c, 128).collect()
    }

    // --- (b) auth-selection matrix: extract_owner_auth_tag ---

    #[test]
    fn auth_selection_owner_match_returns_tag() {
        let signer = hex64('a');
        let sig = hex128('b');
        let tags = vec![json!(["auth", signer, "conditions", sig])];
        let result = extract_owner_auth_tag(&tags, &signer);
        assert!(result.is_some());
        let tag = result.unwrap();
        assert_eq!(tag[0], "auth");
        assert_eq!(tag[1], signer);
        assert_eq!(tag[2], "conditions");
        assert_eq!(tag[3], sig);
    }

    #[test]
    fn auth_selection_non_owner_returns_none() {
        let signer = hex64('a');
        let other_owner = hex64('b');
        let tags = vec![json!(["auth", other_owner, "", hex128('c')])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_three_elements_returns_none() {
        let signer = hex64('a');
        let tags = vec![json!(["auth", signer, "conditions"])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_five_elements_returns_none() {
        let signer = hex64('a');
        let tags = vec![json!(["auth", signer, "conditions", hex128('b'), "extra"])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_non_hex_owner_returns_none() {
        let signer = "z".repeat(64);
        let tags = vec![json!(["auth", signer, "", hex128('a')])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_non_hex_sig_returns_none() {
        let signer = hex64('a');
        let bad_sig = "z".repeat(128);
        let tags = vec![json!(["auth", signer, "", bad_sig])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_malformed_short_sig_returns_none() {
        let signer = hex64('a');
        let short_sig = hex128('a')[..64].to_string();
        let tags = vec![json!(["auth", signer, "", short_sig])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_case_insensitive_owner_match() {
        let signer_lower = hex64('a');
        let signer_upper = signer_lower.to_uppercase();
        let sig = hex128('b');
        let tags = vec![json!(["auth", signer_upper, "cond", sig])];
        let result = extract_owner_auth_tag(&tags, &signer_lower);
        assert!(result.is_some());
    }

    #[test]
    fn auth_selection_non_string_elements_returns_none() {
        let signer = hex64('a');
        let tags = vec![json!(["auth", signer, 42, hex128('b')])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_non_array_tag_skipped() {
        let signer = hex64('a');
        let tags = vec![
            json!("not an array"),
            json!(["auth", signer, "", hex128('b')]),
        ];
        let result = extract_owner_auth_tag(&tags, &signer);
        assert!(result.is_some());
    }

    #[test]
    fn auth_selection_no_tags_returns_none() {
        assert!(extract_owner_auth_tag(&[], &hex64('a')).is_none());
    }

    #[test]
    fn auth_selection_wrong_label_returns_none() {
        let signer = hex64('a');
        let tags = vec![json!(["delegation", signer, "", hex128('b')])];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_valid_plus_duplicate_auth_tag_returns_none() {
        // Set-level rule (F6): a structurally valid, owner-matching `auth`
        // tag alongside a second `auth`-labeled tag (malformed or a
        // duplicate) must not be selected — the whole kind:0 is bare.
        let signer = hex64('a');
        let sig = hex128('b');
        let tags = vec![
            json!(["auth", signer, "conditions", sig]),
            json!(["auth", signer, "conditions", sig]),
        ];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    #[test]
    fn auth_selection_valid_plus_malformed_second_auth_tag_returns_none() {
        let signer = hex64('a');
        let sig = hex128('b');
        let tags = vec![
            json!(["auth", signer, "conditions", sig]),
            json!(["auth", "not-hex", "conditions"]),
        ];
        assert!(extract_owner_auth_tag(&tags, &signer).is_none());
    }

    // --- (c) auth-failure classifier: classify_owner_auth_tag ---
    //
    // Tests the typed failure taxonomy. Each case asserts the exact
    // AuthFailure variant so a wrong classification causes a compile-time or
    // assertion failure — not just a message-substring miss.

    #[test]
    fn classify_no_auth_tag_returns_no_auth_tag() {
        // Case 3 (zero auth tags): tags array has entries but none labelled "auth".
        let signer = hex64('a');
        let tags = vec![json!(["p", hex64('b')]), json!(["e", hex64('c')])];
        assert_eq!(
            classify_owner_auth_tag(&tags, &signer),
            Err(AuthFailure::NoAuthTag)
        );
    }

    #[test]
    fn classify_empty_tags_returns_no_auth_tag() {
        assert_eq!(
            classify_owner_auth_tag(&[], &hex64('a')),
            Err(AuthFailure::NoAuthTag)
        );
    }

    #[test]
    fn classify_duplicate_auth_tags_returns_ambiguous() {
        let signer = hex64('a');
        let sig = hex128('b');
        let tags = vec![
            json!(["auth", signer, "conditions", sig]),
            json!(["auth", signer, "conditions", sig]),
        ];
        assert_eq!(
            classify_owner_auth_tag(&tags, &signer),
            Err(AuthFailure::AmbiguousAuthTag(2))
        );
    }

    #[test]
    fn classify_wrong_arity_returns_wrong_arity() {
        let signer = hex64('a');
        let tags = vec![json!(["auth", signer, "conditions"])];
        assert_eq!(
            classify_owner_auth_tag(&tags, &signer),
            Err(AuthFailure::WrongArity(3))
        );
    }

    #[test]
    fn classify_non_string_element_returns_non_string() {
        let signer = hex64('a');
        let tags = vec![json!(["auth", signer, 42, hex128('b')])];
        assert_eq!(
            classify_owner_auth_tag(&tags, &signer),
            Err(AuthFailure::NonStringElement)
        );
    }

    #[test]
    fn classify_invalid_owner_hex_returns_invalid_owner_hex() {
        let bad_owner = "z".repeat(64);
        let tags = vec![json!(["auth", bad_owner, "", hex128('a')])];
        assert_eq!(
            classify_owner_auth_tag(&tags, &bad_owner),
            Err(AuthFailure::InvalidOwnerHex(bad_owner))
        );
    }

    #[test]
    fn classify_invalid_sig_hex_returns_invalid_sig_hex() {
        let signer = hex64('a');
        let bad_sig = "z".repeat(128);
        let tags = vec![json!(["auth", signer, "", bad_sig])];
        assert_eq!(
            classify_owner_auth_tag(&tags, &signer),
            Err(AuthFailure::InvalidSigHex)
        );
    }

    #[test]
    fn classify_owner_mismatch_returns_owner_mismatch_with_actual_owner() {
        // Case 4: structurally valid tag but owner ≠ signer. The failure must
        // carry the actual owner so resolve_auth can print it in the warning.
        let actual_owner = hex64('a');
        let signer = hex64('b');
        let sig = hex128('c');
        let tags = vec![json!(["auth", actual_owner, "conditions", sig])];
        assert_eq!(
            classify_owner_auth_tag(&tags, &signer),
            Err(AuthFailure::OwnerMismatch(actual_owner.clone()))
        );
        // Message must include the actual owner for actionability.
        let msg = AuthFailure::OwnerMismatch(actual_owner.clone()).message();
        assert!(
            msg.contains(&actual_owner),
            "OwnerMismatch message must include actual owner, got: {msg}"
        );
    }

    // --- (c2) extract_auth: profile-level failure taxonomy ---
    //
    // Pure-unit tests: exercise `extract_auth` directly with pre-built
    // profiles. No relay, no async runtime. These guard all failure paths
    // that the async production resolver depends on.

    #[test]
    fn extract_auth_no_profile_returns_no_profile_failure() {
        let target = hex64('t');
        let signer = hex64('s');
        assert_eq!(
            extract_auth(None, &target, &signer),
            Err(AuthFailure::NoProfile(target.clone()))
        );
    }

    #[test]
    fn extract_auth_no_tags_array_returns_no_tags_array_failure() {
        let target = hex64('t');
        let signer = hex64('s');
        let profile = json!({"kind": 0, "content": "{}"});
        assert_eq!(
            extract_auth(Some(&profile), &target, &signer),
            Err(AuthFailure::NoTagsArray(target.clone()))
        );
    }

    #[test]
    fn extract_auth_no_auth_tag_returns_no_auth_tag_failure() {
        let target = hex64('t');
        let signer = hex64('s');
        let profile = json!({"tags": [["p", hex64('b')]]});
        assert_eq!(
            extract_auth(Some(&profile), &target, &signer),
            Err(AuthFailure::NoAuthTag)
        );
    }

    #[test]
    fn extract_auth_valid_tag_returns_ok() {
        let signer = hex64('a');
        let sig = hex128('b');
        let profile = json!({"tags": [["auth", signer, "conditions", sig]]});
        let result = extract_auth(Some(&profile), &hex64('t'), &signer);
        assert!(result.is_ok(), "must succeed with a valid tag");
        let tag = result.unwrap();
        assert_eq!(tag[0], "auth");
        assert_eq!(tag[1], signer);
    }

    #[test]
    fn extract_auth_owner_mismatch_returns_owner_mismatch_failure() {
        let actual_owner = hex64('a');
        let signer = hex64('b');
        let sig = hex128('c');
        let profile = json!({"tags": [["auth", actual_owner, "conditions", sig]]});
        assert_eq!(
            extract_auth(Some(&profile), &hex64('t'), &signer),
            Err(AuthFailure::OwnerMismatch(actual_owner.clone()))
        );
        // Message must include the actual owner for actionability.
        let msg = AuthFailure::OwnerMismatch(actual_owner.clone()).message();
        assert!(
            msg.contains(&actual_owner),
            "OwnerMismatch message must include actual owner, got: {msg}"
        );
    }

    // --- (c3) resolve_auth: production async resolver via counted test server ---
    //
    // Each test spins up a local Axum server that handles POST /query, counts
    // calls, and returns a canned kind:0 (or empty array) based on the
    // attempt number. The test drives the production `resolve_auth` function
    // and asserts on both return value and exact fetch count.

    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{HeaderMap, Response, StatusCode};
    use axum::Router;
    use tokio::net::TcpListener;

    /// Spin up an Axum server handling POST /query.
    /// `f(n)` is called with the 1-based attempt number and returns the JSON body.
    async fn query_server<F>(f: F) -> (String, Arc<AtomicU32>)
    where
        F: Fn(u32) -> String + Send + Sync + 'static,
    {
        let counter = Arc::new(AtomicU32::new(0));
        let handler: Arc<dyn Fn(u32) -> String + Send + Sync> = Arc::new(f);
        let state = (handler, counter.clone());

        type S = (Arc<dyn Fn(u32) -> String + Send + Sync>, Arc<AtomicU32>);
        let app = Router::new()
            .route(
                "/query",
                axum::routing::post(
                    |State((handler, ctr)): State<S>, _headers: HeaderMap, _body: Body| async move {
                        let n = ctr.fetch_add(1, Ordering::SeqCst) + 1;
                        let body = handler(n);
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "application/json")
                            .body(Body::from(body))
                            .unwrap()
                    },
                ),
            )
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), counter)
    }

    fn test_client(base_url: &str) -> crate::client::BuzzClient {
        let keys = nostr::Keys::generate();
        crate::client::BuzzClient::new(base_url.to_string(), keys, None, None).unwrap()
    }

    fn kind0_response(signer_hex: &str) -> String {
        let sig = hex128('b');
        serde_json::json!([{
            "kind": 0,
            "tags": [["auth", signer_hex, "conditions", sig]],
            "content": "{}"
        }])
        .to_string()
    }

    fn empty_response() -> String {
        "[]".to_string()
    }

    fn no_auth_response() -> String {
        serde_json::json!([{"kind": 0, "tags": [["p", "xx"]], "content": "{}"}]).to_string()
    }

    /// First attempt succeeds — exactly 1 fetch, Ok(Some(tag)).
    #[tokio::test]
    async fn resolve_auth_first_success_one_fetch() {
        let signer = hex64('a');
        let signer_clone = signer.clone();
        let (url, counter) = query_server(move |_n| kind0_response(&signer_clone)).await;
        let client = test_client(&url);
        let target = hex64('b'); // target ≠ signer
        let mut sink: Vec<u8> = Vec::new();

        let result = resolve_auth(&client, &target, &signer, false, &mut sink).await;

        assert!(result.is_ok(), "first success must return Ok: {result:?}");
        assert!(result.unwrap().is_some(), "must return the extracted tag");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "first-success path must issue exactly 1 query"
        );
        assert!(sink.is_empty(), "no warning on success");
    }

    /// First fails (no auth tag), retry succeeds — exactly 2 fetches, Ok(Some(tag)).
    #[tokio::test]
    async fn resolve_auth_retry_success_two_fetches() {
        let signer = hex64('a');
        let signer_clone = signer.clone();
        let (url, counter) = query_server(move |n| {
            if n == 1 {
                no_auth_response()
            } else {
                kind0_response(&signer_clone)
            }
        })
        .await;
        let client = test_client(&url);
        let target = hex64('b');
        let mut sink: Vec<u8> = Vec::new();

        let result = resolve_auth(&client, &target, &signer, false, &mut sink).await;

        assert!(result.is_ok(), "retry success must return Ok: {result:?}");
        assert!(result.unwrap().is_some(), "must return the extracted tag");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "retry-success path must issue exactly 2 queries"
        );
        assert!(sink.is_empty(), "no warning on success");
    }

    /// Both attempts fail, allow_bare == false — exactly 2 fetches, Err (fail closed).
    #[tokio::test]
    async fn resolve_auth_double_failure_no_admin_fail_closed() {
        let (url, counter) = query_server(|_n| no_auth_response()).await;
        let client = test_client(&url);
        let signer = hex64('s');
        let target = hex64('t');
        let mut sink: Vec<u8> = Vec::new();

        let result = resolve_auth(&client, &target, &signer, false, &mut sink).await;

        assert!(result.is_err(), "double failure must fail closed");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("refusing to send"),
            "error must name the refusal: {err}"
        );
        assert!(
            err.contains("--admin"),
            "error must mention --admin escape: {err}"
        );
        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "double-failure path must issue exactly 2 queries"
        );
        assert!(sink.is_empty(), "no warning on fail-closed path");
    }

    /// Both attempts fail, allow_bare == true (--admin) — exactly 2 fetches,
    /// Ok(None) + exactly one warning line.
    #[tokio::test]
    async fn resolve_auth_double_failure_admin_allows_bare_with_warning() {
        let (url, counter) = query_server(|_n| no_auth_response()).await;
        let client = test_client(&url);
        let signer = hex64('s');
        let target = hex64('t');
        let mut sink: Vec<u8> = Vec::new();

        let result = resolve_auth(&client, &target, &signer, true, &mut sink).await;

        assert!(result.is_ok(), "--admin must allow bare send: {result:?}");
        assert!(result.unwrap().is_none(), "bare path returns None");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "--admin double-failure path must issue exactly 2 queries"
        );
        // Exactly one warning line.
        let text = std::str::from_utf8(&sink).expect("UTF-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "expected exactly one warning, got: {text:?}"
        );
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON warning");
        let warning = parsed["warning"].as_str().expect("string warning field");
        assert!(
            warning.contains("proceeding without owner attestation"),
            "warning must name the bare-send: {warning}"
        );
    }

    /// Self path (target == signer, case-insensitive) — zero fetches, Ok(None).
    #[tokio::test]
    async fn resolve_auth_self_path_zero_fetches() {
        // Server counts every /query call; we assert 0.
        let (url, counter) = query_server(|_n| empty_response()).await;
        let client = test_client(&url);
        let signer = hex64('a');
        let target_upper = signer.to_uppercase(); // case-insensitive self
        let mut sink: Vec<u8> = Vec::new();

        let result = resolve_auth(&client, &target_upper, &signer, false, &mut sink).await;

        assert!(result.is_ok(), "self path must return Ok: {result:?}");
        assert!(result.unwrap().is_none(), "self path returns None");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "self path must issue zero queries"
        );
        assert!(sink.is_empty(), "no warning on self path");
    }

    // --- (c4) --admin flag parser: both archive and unarchive ---
    //
    // These tests confirm the flag is declared and parsed on both subcommands
    // so it can't silently disappear from one of them.

    #[test]
    fn archive_admin_flag_is_parsed() {
        use crate::Cli;
        use clap::Parser;
        let cli = Cli::try_parse_from(["buzz", "agents", "archive", &hex64('a'), "--admin"])
            .expect("--admin must be accepted by agents archive");
        match cli.command {
            crate::Cmd::Agents(crate::AgentsCmd::Archive { admin, .. }) => {
                assert!(admin, "--admin must be true when flag is present");
            }
            _ => panic!("unexpected command variant"),
        }
    }

    #[test]
    fn unarchive_admin_flag_is_parsed() {
        use crate::Cli;
        use clap::Parser;
        let cli = Cli::try_parse_from(["buzz", "agents", "unarchive", &hex64('a'), "--admin"])
            .expect("--admin must be accepted by agents unarchive");
        match cli.command {
            crate::Cmd::Agents(crate::AgentsCmd::Unarchive { admin, .. }) => {
                assert!(admin, "--admin must be true when flag is present");
            }
            _ => panic!("unexpected command variant"),
        }
    }

    // --- (d) NIP-11 self normalization: normalize_relay_self_hex ---

    #[test]
    fn normalize_self_lowercases_uppercase_hex() {
        let upper = hex64('A');
        let result = normalize_relay_self_hex(&upper).expect("should pass");
        assert_eq!(result, hex64('a'));
    }

    #[test]
    fn normalize_self_rejects_wrong_length() {
        assert!(normalize_relay_self_hex(&hex64('a')[..63]).is_err());
    }

    #[test]
    fn normalize_self_rejects_non_hex() {
        assert!(normalize_relay_self_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn archived_uppercase_self_matches_lowercase_event_author() {
        // F7: an uppercase NIP-11 `self` must still resolve to the same
        // relay identity as the event's (always-lowercase) author hex once
        // normalized — before the fix this was a case-sensitive mismatch.
        let keys = Keys::generate();
        let self_hex_lower = keys.public_key().to_hex();
        let self_hex_upper = self_hex_lower.to_uppercase();
        let normalized = normalize_relay_self_hex(&self_hex_upper).expect("valid hex");
        let event = build_archived_event(&keys, KIND_IA_ARCHIVED_LIST as u16, &[], true);
        let result = verify_archived_event(&event, &normalized).expect("should pass");
        assert!(result.is_empty());
    }

    // --- (c) snapshot tri-state: verify_archived_event ---

    fn build_archived_event(
        keys: &Keys,
        kind: u16,
        p_tags: &[&str],
        include_nip70: bool,
    ) -> nostr::Event {
        let mut tags: Vec<Tag> = Vec::new();
        if include_nip70 {
            tags.push(Tag::parse(["-"]).unwrap());
        }
        for pk in p_tags {
            tags.push(Tag::parse(["p", pk]).unwrap());
        }
        EventBuilder::new(Kind::Custom(kind), "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign")
    }

    #[test]
    fn archived_state2_valid_event_returns_pubkeys() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let pk1 = hex64('a');
        let pk2 = hex64('b');
        let event = build_archived_event(&keys, KIND_IA_ARCHIVED_LIST as u16, &[&pk1, &pk2], true);
        let result = verify_archived_event(&event, &self_hex).expect("should pass");
        assert_eq!(result, vec![pk1.as_str(), pk2.as_str()]);
    }

    #[test]
    fn archived_state2_empty_p_tags_returns_empty() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = build_archived_event(&keys, KIND_IA_ARCHIVED_LIST as u16, &[], true);
        let result = verify_archived_event(&event, &self_hex).expect("should pass");
        assert!(result.is_empty());
    }

    #[test]
    fn archived_state3_wrong_kind_errors() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = build_archived_event(&keys, 9999, &[], true);
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("wrong kind"),
            "error should name wrong kind: {err}"
        );
    }

    #[test]
    fn archived_state3_wrong_author_errors() {
        let event_keys = Keys::generate();
        let other_self = hex64('f');
        let event = build_archived_event(&event_keys, KIND_IA_ARCHIVED_LIST as u16, &[], true);
        let err = verify_archived_event(&event, &other_self).unwrap_err();
        assert!(
            err.to_string().contains("does not match relay self"),
            "error should name author mismatch: {err}"
        );
    }

    #[test]
    fn archived_state3_no_nip70_tag_errors() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = build_archived_event(&keys, KIND_IA_ARCHIVED_LIST as u16, &[], false);
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("NIP-70"),
            "error should name missing NIP-70 tag: {err}"
        );
    }

    #[test]
    fn archived_state3_duplicate_nip70_tags_errors() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([Tag::parse(["-"]).unwrap(), Tag::parse(["-"]).unwrap()])
            .sign_with_keys(&keys)
            .expect("sign");
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("found 2"),
            "error should report 2 NIP-70 tags: {err}"
        );
    }

    #[test]
    fn archived_state3_lone_malformed_nip70_tag_errors() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([Tag::parse(["-", "extra"]).unwrap()])
            .sign_with_keys(&keys)
            .expect("sign");
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("malformed NIP-70"),
            "error should name the malformed NIP-70 tag: {err}"
        );
    }

    #[test]
    fn archived_state3_exact_marker_plus_malformed_marker_errors() {
        // F5 (IMPORTANT, discriminating): a valid `["-"]` alongside a
        // malformed `["-", "extra"]` must still poison the snapshot — the
        // old count-of-exact-shape-only check let this bypass through with
        // nip70_count == 1.
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["-", "extra"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign");
        let err = verify_archived_event(&event, &self_hex).unwrap_err();
        assert!(
            err.to_string().contains("malformed NIP-70"),
            "error should name the malformed NIP-70 tag: {err}"
        );
    }

    #[test]
    fn archived_non_hex_p_tag_dropped() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let valid_pk = hex64('a');
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["p", &valid_pk]).unwrap(),
                Tag::parse(["p", "not-hex-at-all"]).unwrap(),
                Tag::parse(["p", &"z".repeat(64)]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign");
        let result = verify_archived_event(&event, &self_hex).expect("should pass");
        assert_eq!(result, vec![valid_pk.as_str()]);
    }

    #[test]
    fn archived_short_p_tag_dropped() {
        let keys = Keys::generate();
        let self_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
            .tags([
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["p", &hex64('a')[..32]]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign");
        let result = verify_archived_event(&event, &self_hex).expect("should pass");
        assert!(result.is_empty());
    }

    // --- agents remove / --dry-run ---

    fn sample_managed_record(pubkey: &str, name: &str) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: pubkey.to_string(),
            name: name.to_string(),
            persona_id: None,
            team_id: None,
            private_key_nsec: "nsec1testsecretmaterial".into(),
            auth_tag: None,
            relay_url: String::new(),
            avatar_url: None,
            acp_command: DEFAULT_ACP_COMMAND.to_string(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: Vec::new(),
            mcp_command: String::new(),
            turn_timeout_seconds: 0,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: DEFAULT_AGENT_PARALLELISM,
            system_prompt: Some("prompt".into()),
            model: None,
            provider: None,
            persona_source_version: None,
            env_vars: std::collections::BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: Default::default(),
            backend_agent_id: None,
            provider_binary_path: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: Default::default(),
            respond_to_allowlist: Vec::new(),
            marketplace: None,
            display_name: None,
            slug: None,
            runtime: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: Vec::new(),
            definition_parallelism: None,
            relay_mesh: None,
        }
    }

    fn write_temp_store(
        dir: &std::path::Path,
        records: &[ManagedAgentRecord],
    ) -> std::path::PathBuf {
        let agents_dir = dir.join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        let path = agents_dir.join("managed-agents.json");
        let json = serde_json::to_string_pretty(records).unwrap();
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn remove_dry_run_leaves_store_bytes_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let pubkey = hex64('a');
        let path = write_temp_store(
            dir.path(),
            &[sample_managed_record(&pubkey, "PreviewAgent")],
        );
        let before = std::fs::read(&path).unwrap();

        let out = remove_agent(&pubkey, true, Some(dir.path()), "xyz.block.buzz.app")
            .expect("dry-run should succeed");

        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "dry-run must not mutate managed-agents.json");
        assert_eq!(out["pubkey"], pubkey);
        assert_eq!(out["name"], "PreviewAgent");
        assert_eq!(out["dry_run"], true);
        assert!(
            out.get("store_path").and_then(|v| v.as_str()).is_some(),
            "stdout JSON must include store_path: {out}"
        );
        assert!(
            out.get("private_key_nsec").is_none(),
            "must omit private_key_nsec: {out}"
        );
        assert!(
            !serde_json::to_string(&out)
                .unwrap()
                .contains("nsec1testsecretmaterial"),
            "must not leak nsec material: {out}"
        );
    }

    #[test]
    fn remove_without_dry_run_drops_matched_record() {
        let dir = tempfile::tempdir().unwrap();
        let keep = hex64('b');
        let drop = hex64('c');
        let path = write_temp_store(
            dir.path(),
            &[
                sample_managed_record(&keep, "KeepMe"),
                sample_managed_record(&drop, "DropMe"),
            ],
        );

        let out = remove_agent(&drop, false, Some(dir.path()), "xyz.block.buzz.app")
            .expect("real remove should succeed");

        // Non-dry-run keeps the pre-existing JSON shape (pubkey/removed/store_path).
        assert_eq!(out["pubkey"], drop);
        assert_eq!(out["removed"], 1);
        assert!(out.get("dry_run").is_none());
        assert!(out.get("private_key_nsec").is_none());

        let remaining: Vec<ManagedAgentRecord> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].pubkey, keep);
        assert_eq!(remaining[0].name, "KeepMe");
    }

    #[test]
    fn remove_unknown_pubkey_fails_without_write() {
        let dir = tempfile::tempdir().unwrap();
        let present = hex64('d');
        let missing = hex64('e');
        let path = write_temp_store(dir.path(), &[sample_managed_record(&present, "StillHere")]);
        let before = std::fs::read(&path).unwrap();

        let err = remove_agent(&missing, true, Some(dir.path()), "xyz.block.buzz.app")
            .expect_err("unknown pubkey must fail");
        assert!(
            err.to_string().contains(&missing),
            "error should name the missing pubkey: {err}"
        );
        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "failed remove must not mutate the store");
    }

    #[test]
    fn set_access_updates_only_the_matching_agent() {
        let dir = tempfile::tempdir().unwrap();
        let target = hex64('a');
        let untouched = hex64('b');
        let path = write_temp_store(
            dir.path(),
            &[
                sample_managed_record(&target, "Target"),
                sample_managed_record(&untouched, "Untouched"),
            ],
        );

        set_agent_access(
            &target,
            AgentAccessArg::Allowlist,
            std::slice::from_ref(&untouched),
            Some(dir.path()),
            "xyz.block.buzz.app",
        )
        .expect("access update should succeed");

        let records: Vec<ManagedAgentRecord> =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(records[0].respond_to, RespondTo::Allowlist);
        assert_eq!(records[0].respond_to_allowlist, vec![untouched]);
        assert_eq!(records[1].respond_to, RespondTo::OwnerOnly);
    }

    #[test]
    fn replacement_import_preserves_identity_and_prunes_only_orphan_definitions() {
        let dir = tempfile::tempdir().unwrap();
        let pubkey = hex64('a');
        let mut instance = sample_managed_record(&pubkey, "Explorer");
        instance.persona_id = Some("explorer-linked".into());
        let original_secret = instance.private_key_nsec.clone();

        let mut linked = sample_managed_record("", "Explorer");
        linked.slug = Some("explorer-linked".into());
        let mut orphan = sample_managed_record("", "Explorer");
        orphan.slug = Some("explorer-old".into());
        let mut unrelated = sample_managed_record("", "Reviewer");
        unrelated.slug = Some("reviewer".into());
        let path = write_temp_store(dir.path(), &[instance, linked, orphan, unrelated]);

        let mut snapshot = valid_snapshot_json();
        snapshot["definition"]["name"] = json!("Explorer");
        snapshot["definition"]["systemPrompt"] = json!("focused explorer prompt");
        snapshot["definition"]["runtime"] = json!("cursor");
        snapshot["definition"]["parallelism"] = json!(7);
        snapshot["profile"]["displayName"] = json!("Explorer");
        let file = write_snapshot(dir.path(), &snapshot);

        import_agent(
            &offline_client(),
            &file,
            Some(dir.path()),
            "test.id",
            false,
            Some(&pubkey),
            true,
        )
        .expect("replacement import");

        let records: Vec<ManagedAgentRecord> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(records.len(), 3);
        let updated = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .unwrap();
        assert_eq!(updated.private_key_nsec, original_secret);
        assert_eq!(updated.persona_id.as_deref(), Some("explorer-linked"));
        assert_eq!(
            updated.system_prompt.as_deref(),
            Some("focused explorer prompt")
        );
        assert_eq!(updated.parallelism, 7);

        let linked = records
            .iter()
            .find(|record| record.slug.as_deref() == Some("explorer-linked"))
            .unwrap();
        assert_eq!(
            linked.system_prompt.as_deref(),
            Some("focused explorer prompt")
        );
        assert!(records
            .iter()
            .all(|record| record.slug.as_deref() != Some("explorer-old")));
        assert!(records
            .iter()
            .any(|record| record.slug.as_deref() == Some("reviewer")));
        assert!(dir
            .path()
            .join("agents/managed-agents.json.pre-roster-import.bak")
            .is_file());
    }

    #[test]
    fn replacement_import_refuses_ambiguous_keyed_name_before_pruning() {
        let dir = tempfile::tempdir().unwrap();
        let keep = hex64('a');
        let other = hex64('b');
        let path = write_temp_store(
            dir.path(),
            &[
                sample_managed_record(&keep, "Explorer"),
                sample_managed_record(&other, "Explorer"),
            ],
        );
        let before = std::fs::read(&path).unwrap();

        let mut snapshot = valid_snapshot_json();
        snapshot["definition"]["name"] = json!("Explorer");
        snapshot["profile"]["displayName"] = json!("Explorer");
        let file = write_snapshot(dir.path(), &snapshot);
        let error = import_agent(
            &offline_client(),
            &file,
            Some(dir.path()),
            "test.id",
            false,
            Some(&keep),
            true,
        )
        .expect_err("ambiguous keyed names must not be pruned");

        assert!(error.to_string().contains("refusing to guess"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
