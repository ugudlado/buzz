//! Sequential workflow executor.
//!
//! Responsibilities:
//! - Template variable resolution (`{{trigger.X}}`, `{{steps.ID.output.X}}`)
//! - Condition evaluation (`if:` expressions via `evalexpr`)
//! - Sequential step dispatch
//! - Execution trace updates in DB
//!
//! Action dispatch uses placeholder implementations that log intent.
//! Real event emission is wired in WF-07/08 (relay integration).

use std::collections::HashMap;

use buzz_core::tenant::CommunityId;
use evalexpr::HashMapContext;
use nostr::ToBech32;
use serde_json::Value as JsonValue;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::error::WorkflowError;
use crate::schema::{ActionDef, Step, WorkflowDef};
use crate::WorkflowEngine;

/// Data extracted from the triggering event, passed to every step.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TriggerContext {
    /// Message content (message_posted trigger).
    pub text: String,
    /// Pubkey of the event author (hex string).
    pub author: String,
    /// Channel UUID as string.
    pub channel_id: String,
    /// Unix timestamp of the triggering event (as string for template use).
    pub timestamp: String,
    /// Emoji name (reaction_added trigger).
    pub emoji: String,
    /// Event ID of the triggering message (hex string).
    pub message_id: String,
    /// Arbitrary webhook body fields (webhook trigger).
    pub webhook_fields: HashMap<String, String>,
}

impl TriggerContext {
    /// Look up a trigger field by name.
    ///
    /// Returns `Some(&str)` for known fields; for webhook triggers, also
    /// checks `webhook_fields`. Returns `None` for unknown names.
    pub fn get_field(&self, name: &str) -> Option<&str> {
        match name {
            "text" => Some(&self.text),
            "author" => Some(&self.author),
            "channel_id" => Some(&self.channel_id),
            "timestamp" => Some(&self.timestamp),
            "emoji" => Some(&self.emoji),
            "message_id" => Some(&self.message_id),
            other => self.webhook_fields.get(other).map(|s| s.as_str()),
        }
    }
}

/// Resolve `{{trigger.X}}` and `{{steps.ID.output.X}}` placeholders in a string.
///
/// Supports filters:
/// - `| truncate(N)` — truncate to N characters
/// - `| npub` — encode a hex pubkey as its full bech32 `npub` (non-pubkey
///   values pass through unchanged); `truncate_pubkey` is a legacy alias
///
/// Unknown `{{keys}}` are left as literal text (no error, no substitution).
pub fn resolve_template(
    template: &str,
    trigger_ctx: &TriggerContext,
    step_outputs: &HashMap<String, JsonValue>,
) -> Result<String, WorkflowError> {
    if !template.contains("{{") {
        return Ok(template.to_owned());
    }

    let mut result = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(start) = remaining.find("{{") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 2..];

        let end = match remaining.find("}}") {
            Some(e) => e,
            None => {
                // Unclosed `{{` — emit literally and stop.
                result.push_str("{{");
                result.push_str(remaining);
                return Ok(result);
            }
        };

        let expr = remaining[..end].trim();
        remaining = &remaining[end + 2..];

        // Split on `|` to extract filters.
        let mut parts = expr.splitn(2, '|');
        let var_path = parts.next().unwrap_or("").trim();
        let filter = parts.next().map(|s| s.trim());

        let raw_value = resolve_variable(var_path, trigger_ctx, step_outputs);

        let value = match (raw_value, filter) {
            (Some(v), Some(f)) => apply_filter(v, f)?,
            (Some(v), None) => v,
            (None, _) => {
                // Unknown variable — emit the original `{{expr}}` literally.
                result.push_str("{{");
                result.push_str(expr);
                result.push_str("}}");
                continue;
            }
        };

        result.push_str(&value);
    }

    result.push_str(remaining);
    Ok(result)
}

/// Resolve a single variable path to its string value.
fn resolve_variable(
    path: &str,
    trigger_ctx: &TriggerContext,
    step_outputs: &HashMap<String, JsonValue>,
) -> Option<String> {
    if let Some(field) = path.strip_prefix("trigger.") {
        return trigger_ctx.get_field(field).map(|s| s.to_owned());
    }

    // Pattern: `steps.STEP_ID.output.FIELD`
    if let Some(rest) = path.strip_prefix("steps.") {
        let mut parts = rest.splitn(3, '.');
        let step_id = parts.next()?;
        let middle = parts.next()?; // must be "output"
        let field = parts.next()?;

        if middle != "output" {
            return None;
        }

        let output = step_outputs.get(step_id)?;
        return json_get_str(output, field);
    }

    None
}

/// Navigate a JSON value by a single key and return it as a string.
fn json_get_str(value: &JsonValue, key: &str) -> Option<String> {
    match value {
        JsonValue::Object(map) => {
            let v = map.get(key)?;
            Some(json_to_string(v))
        }
        _ => None,
    }
}

/// Convert a JSON value to a plain string for template substitution.
fn json_to_string(v: &JsonValue) -> String {
    match v {
        JsonValue::String(s) => s.clone(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Number(n) => n.to_string(),
        JsonValue::Null => String::new(),
        other => other.to_string(),
    }
}

/// Apply a filter expression to a resolved value.
fn apply_filter(value: String, filter: &str) -> Result<String, WorkflowError> {
    let filter = filter.trim();

    if let Some(inner) = filter
        .strip_prefix("truncate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let n: usize = inner.trim().parse().map_err(|_| {
            WorkflowError::TemplateError(format!("truncate() requires a number, got: {inner}"))
        })?;
        let truncated: String = value.chars().take(n).collect();
        return Ok(truncated);
    }

    // `npub` (alias `truncate_pubkey`): full bech32 npub — truncated prefixes are grindable.
    if filter == "npub" || filter == "truncate_pubkey" {
        if let Ok(pk) = nostr::PublicKey::from_hex(&value) {
            return Ok(pk.to_bech32().unwrap_or(value));
        }
        return Ok(value);
    }

    Err(WorkflowError::TemplateError(format!(
        "unknown filter: {filter}"
    )))
}

/// Build an `evalexpr::HashMapContext` from trigger context and step outputs.
///
/// Variable names use underscores (not dots) because `evalexpr` does not
/// support dotted identifiers:
///
/// | YAML reference                    | evalexpr variable         |
/// |-----------------------------------|---------------------------|
/// | `trigger.text`                    | `trigger_text`            |
/// | `trigger.author`                  | `trigger_author`          |
/// | `trigger.channel_id`              | `trigger_channel_id`      |
/// | `trigger.timestamp`               | `trigger_timestamp`       |
/// | `trigger.emoji`                   | `trigger_emoji`           |
/// | `trigger.message_id`              | `trigger_message_id`      |
/// | `steps.STEP_ID.output.FIELD`      | `steps_STEP_ID_output_FIELD` |
///
/// Also registers string helper functions that the `cron` crate's `evalexpr` v11
/// does not include by default:
/// - `str_contains(haystack, needle)` → bool
/// - `str_starts_with(s, prefix)` → bool
/// - `str_ends_with(s, suffix)` → bool
/// - `str_len(s)` → int
pub fn build_eval_context(
    trigger_ctx: &TriggerContext,
    step_outputs: &HashMap<String, JsonValue>,
) -> Result<HashMapContext, WorkflowError> {
    use evalexpr::*;

    let mut ctx = HashMapContext::new();

    // evalexpr v11 does not ship str_contains / str_starts_with / str_ends_with.
    // Register them as custom functions so workflow YAML can use them.

    ctx.set_function(
        "str_contains".into(),
        Function::new(|args| {
            let args = args.as_fixed_len_tuple(2)?;
            let haystack = args[0].as_string()?;
            let needle = args[1].as_string()?;
            Ok(Value::Boolean(haystack.contains(needle.as_str())))
        }),
    )
    .map_err(|e| WorkflowError::ConditionError(e.to_string()))?;

    ctx.set_function(
        "str_starts_with".into(),
        Function::new(|args| {
            let args = args.as_fixed_len_tuple(2)?;
            let s = args[0].as_string()?;
            let prefix = args[1].as_string()?;
            Ok(Value::Boolean(s.starts_with(prefix.as_str())))
        }),
    )
    .map_err(|e| WorkflowError::ConditionError(e.to_string()))?;

    ctx.set_function(
        "str_ends_with".into(),
        Function::new(|args| {
            let args = args.as_fixed_len_tuple(2)?;
            let s = args[0].as_string()?;
            let suffix = args[1].as_string()?;
            Ok(Value::Boolean(s.ends_with(suffix.as_str())))
        }),
    )
    .map_err(|e| WorkflowError::ConditionError(e.to_string()))?;

    ctx.set_function(
        "str_len".into(),
        Function::new(|arg| {
            let s = arg.as_string()?;
            Ok(Value::Int(s.len() as i64))
        }),
    )
    .map_err(|e| WorkflowError::ConditionError(e.to_string()))?;

    // Register webhook fields first as `trigger_FIELD` so that standard trigger
    // fields inserted below always take precedence and cannot be spoofed.
    for (key, val) in &trigger_ctx.webhook_fields {
        // Skip any key that would collide with a standard trigger_ or steps_ variable.
        if key.starts_with("trigger_") || key.starts_with("steps_") {
            continue;
        }
        let var_name = format!("trigger_{key}");
        ctx.set_value(var_name, Value::String(val.clone()))
            .map_err(|e| WorkflowError::ConditionError(e.to_string()))?;
    }

    let trigger_fields = [
        ("trigger_text", trigger_ctx.text.as_str()),
        ("trigger_author", trigger_ctx.author.as_str()),
        ("trigger_channel_id", trigger_ctx.channel_id.as_str()),
        ("trigger_timestamp", trigger_ctx.timestamp.as_str()),
        ("trigger_emoji", trigger_ctx.emoji.as_str()),
        ("trigger_message_id", trigger_ctx.message_id.as_str()),
    ];

    for (name, val) in &trigger_fields {
        ctx.set_value((*name).into(), Value::String((*val).to_owned()))
            .map_err(|e| WorkflowError::ConditionError(e.to_string()))?;
    }

    for (step_id, output) in step_outputs {
        if let JsonValue::Object(map) = output {
            for (field, val) in map {
                let var_name = format!("steps_{step_id}_output_{field}");
                let eval_val = json_value_to_eval(val);
                ctx.set_value(var_name, eval_val)
                    .map_err(|e| WorkflowError::ConditionError(e.to_string()))?;
            }
        }
    }

    Ok(ctx)
}

/// Convert a `serde_json::Value` to an `evalexpr::Value`.
fn json_value_to_eval(v: &JsonValue) -> evalexpr::Value {
    use evalexpr::Value as EV;
    match v {
        JsonValue::String(s) => EV::String(s.clone()),
        JsonValue::Bool(b) => EV::Boolean(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                EV::Int(i)
            } else if let Some(f) = n.as_f64() {
                EV::Float(f)
            } else {
                EV::String(n.to_string())
            }
        }
        JsonValue::Null => EV::Empty,
        other => EV::String(other.to_string()),
    }
}

/// Maximum wall-clock time allowed for a single `evalexpr` evaluation.
///
/// `evalexpr` is not designed for adversarial input — a deeply nested or
/// recursive expression can spin indefinitely. We run the evaluation on a
/// blocking thread and impose a hard timeout.
const EVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

/// Evaluate a boolean `if:` expression against the current execution context.
///
/// Returns `true` if the step should run, `false` if it should be skipped.
///
/// The evaluation is wrapped in a [`tokio::time::timeout`] to prevent a
/// malicious or pathological expression from blocking a Tokio worker thread.
pub async fn evaluate_condition(
    expr: &str,
    trigger_ctx: &TriggerContext,
    step_outputs: &HashMap<String, JsonValue>,
) -> Result<bool, WorkflowError> {
    let ctx = build_eval_context(trigger_ctx, step_outputs)?;
    let expr_owned = expr.to_owned();

    // Bound expression complexity to prevent pathological evaluation times.
    // The spawn_blocking thread cannot be cancelled by tokio::time::timeout —
    // it will run to completion even after timeout. Length-limiting the expression
    // prevents worst-case O(2^n) evaluation paths.
    const MAX_EXPR_LEN: usize = 4096;
    if expr_owned.len() > MAX_EXPR_LEN {
        return Err(WorkflowError::ConditionError(format!(
            "condition expression exceeds {} byte limit",
            MAX_EXPR_LEN
        )));
    }

    let result = tokio::time::timeout(
        EVAL_TIMEOUT,
        tokio::task::spawn_blocking(move || evalexpr::eval_boolean_with_context(&expr_owned, &ctx)),
    )
    .await
    .map_err(|_| {
        WorkflowError::ConditionError(format!(
            "'{expr}': evaluation timed out after {}ms",
            EVAL_TIMEOUT.as_millis()
        ))
    })?
    .map_err(|e| WorkflowError::ConditionError(format!("'{expr}': eval task panicked: {e}")))?
    .map_err(|e| WorkflowError::ConditionError(format!("'{expr}': {e}")))?;

    Ok(result)
}

/// Resolve all template variables in a step's action fields.
///
/// Returns a new `ActionDef` with all `{{...}}` placeholders substituted.
pub fn resolve_step_templates(
    step: &Step,
    trigger_ctx: &TriggerContext,
    step_outputs: &HashMap<String, JsonValue>,
) -> Result<ActionDef, WorkflowError> {
    use ActionDef::*;

    let t = |s: &str| resolve_template(s, trigger_ctx, step_outputs);
    let t_opt = |s: &Option<String>| -> Result<Option<String>, WorkflowError> {
        match s {
            Some(v) => Ok(Some(t(v)?)),
            None => Ok(None),
        }
    };

    match &step.action {
        SendMessage { text, channel } => Ok(SendMessage {
            text: t(text)?,
            channel: t_opt(channel)?,
        }),
        SendDm { to, text } => Ok(SendDm {
            to: t(to)?,
            text: t(text)?,
        }),
        SetChannelTopic { topic } => Ok(SetChannelTopic { topic: t(topic)? }),
        AddReaction { emoji } => Ok(AddReaction { emoji: t(emoji)? }),
        CallWebhook {
            url,
            method,
            headers,
            body,
        } => {
            let resolved_headers = match headers {
                Some(h) => {
                    let mut out = std::collections::HashMap::new();
                    for (k, v) in h {
                        out.insert(k.clone(), t(v)?);
                    }
                    Some(out)
                }
                None => None,
            };
            Ok(CallWebhook {
                url: t(url)?,
                method: method.clone(),
                headers: resolved_headers,
                body: t_opt(body)?,
            })
        }
        RequestApproval {
            from,
            message,
            timeout,
        } => Ok(RequestApproval {
            from: t(from)?,
            message: t(message)?,
            timeout: timeout.clone(),
        }),
        Delay { duration } => Ok(Delay {
            duration: duration.clone(),
        }),
        AssignToAgent {
            agent,
            agent_pubkey,
            instruction,
            timeout,
        } => Ok(AssignToAgent {
            agent: t(agent)?,
            agent_pubkey: agent_pubkey.clone(),
            instruction: t(instruction)?,
            timeout: timeout.clone(),
        }),
    }
}

/// What a suspended step is waiting on. Both variants pause the run the same
/// way (see [`StepResult::Suspended`]) but persist to different tables and
/// carry different resume identifiers.
#[derive(Debug)]
pub enum SuspendReason {
    /// Waiting on a `RequestApproval` gate to be granted or denied.
    Approval {
        /// Who may approve (user mention or role spec), from the step's `from` field.
        approver_spec: String,
    },
    /// Waiting on an `AssignToAgent` mention to receive a completion reply.
    AgentAssignment {
        /// Hex event id of the `@mention` message sent to the agent.
        prompt_event_id: String,
        /// Unix seconds `created_at` of the prompt event — carried forward so
        /// a later step in the same run can thread its own prompt as a
        /// NIP-10 reply without a DB round-trip.
        prompt_created_at: i64,
        /// Hex event id of this run's thread root — the very first
        /// `assign_to_agent` prompt. Equal to `prompt_event_id` when this is
        /// that first prompt.
        root_event_id: String,
        /// Unix seconds `created_at` of the root event.
        root_created_at: i64,
        /// Compressed public key bytes of the mentioned agent.
        agent_pubkey: Vec<u8>,
    },
}

/// Result of dispatching a single step action.
#[derive(Debug)]
pub enum StepResult {
    /// Step completed normally. Output is stored in `step_outputs`.
    Completed(JsonValue),
    /// Step requests suspension (approval gate or agent assignment).
    /// Execution must pause.
    Suspended {
        /// Token used to resume this suspension. For approvals this is the
        /// raw approval token (stored hashed); for agent assignments this is
        /// the prompt event id (public — there is no secret to hash).
        resume_token: String,
        /// What this step is waiting on.
        reason: SuspendReason,
        /// Duration string (e.g. `"24h"`) after which the suspension expires.
        timeout: String,
    },
    /// Step was skipped due to `if:` condition being false.
    Skipped,
}

fn resolve_send_message_channel(
    explicit_channel: Option<&str>,
    trigger_channel: &str,
    workflow_channel_id: Option<Uuid>,
) -> Result<String, WorkflowError> {
    let explicit_channel = explicit_channel
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(workflow_channel_id) = workflow_channel_id {
        if let Some(explicit_channel) = explicit_channel {
            let override_channel_id = explicit_channel.parse::<Uuid>().map_err(|e| {
                WorkflowError::InvalidDefinition(format!(
                    "SendMessage: invalid channel override UUID: {e}"
                ))
            })?;
            if override_channel_id != workflow_channel_id {
                return Err(WorkflowError::InvalidDefinition(format!(
                    "SendMessage: channel override must match the workflow channel ({workflow_channel_id})"
                )));
            }
        }
        return Ok(workflow_channel_id.to_string());
    }

    if let Some(explicit_channel) = explicit_channel {
        let override_channel_id = explicit_channel.parse::<Uuid>().map_err(|e| {
            WorkflowError::InvalidDefinition(format!(
                "SendMessage: invalid channel override UUID: {e}"
            ))
        })?;
        return Ok(override_channel_id.to_string());
    }

    if trigger_channel.trim().is_empty() {
        return Err(WorkflowError::InvalidDefinition(
            "SendMessage: no channel_id available (trigger has no channel context and no channel override was specified)"
                .into(),
        ));
    }

    Ok(trigger_channel.trim().to_string())
}

/// Dispatch a resolved action and return its output.
///
/// For MVP, most actions log their intent and return a success output.
/// Real event emission is wired in WF-07/08 (relay integration).
///
/// `RequestApproval` returns `StepResult::Suspended` — the caller must
/// persist state and stop the execution loop.
pub async fn dispatch_action(
    step_id: &str,
    action: &ActionDef,
    engine: &WorkflowEngine,
    community_id: CommunityId,
    run_id: Uuid,
    trigger_ctx: &TriggerContext,
    thread_anchor: Option<&crate::action_sink::ThreadAnchor>,
) -> Result<StepResult, WorkflowError> {
    use ActionDef::*;

    // The workflow engine can outlive the serving request that spawned it.
    // Revalidate the durable community fence immediately before every external
    // side effect (message publish, webhook, delay/resume). A storage failure is
    // a denial, never permission to continue.
    let serving_write =
        buzz_deletion::acquire_serving_write(&engine.db, community_id, "workflow_action")
            .await
            .map_err(|error| {
                WorkflowError::WebhookError(format!(
                    "community write fence rejected workflow side effect: {error}"
                ))
            })?;

    serving_write.verify().await.map_err(|error| {
        WorkflowError::WebhookError(format!("community write lease lost: {error}"))
    })?;

    let result = serving_write.protect(async {
        match action {
        SendMessage { text, channel } => {
            // Look up workflow metadata for destination validation and
            // attribution, scoped to the run's community — the same run/workflow
            // UUID may exist in another community, so a bare-id lookup could
            // load the wrong row and drive a side effect under it.
            let wf_run = engine
                .db
                .get_workflow_run(community_id, run_id)
                .await
                .map_err(|e| {
                    WorkflowError::WebhookError(format!(
                        "SendMessage: failed to load workflow run {run_id}: {e}"
                    ))
                })?;
            let workflow = engine
                .db
                .get_workflow(community_id, wf_run.workflow_id)
                .await
                .map_err(|e| {
                    WorkflowError::WebhookError(format!(
                        "SendMessage: failed to load workflow {}: {e}",
                        wf_run.workflow_id
                    ))
                })?;
            let channel_id = resolve_send_message_channel(
                channel.as_deref(),
                &trigger_ctx.channel_id,
                workflow.channel_id,
            )?;
            let owner_pubkey_hex = hex::encode(&workflow.owner_pubkey);

            info!(
                run_id = %run_id,
                step = step_id,
                channel = %channel_id,
                "SendMessage → {channel_id}: {text}"
            );

            // Thread into the run's conversation (as a direct reply to the
            // root, same flat shape as agent-step prompts) when the run has
            // one; a run whose first step is send_message has no thread yet
            // and posts top-level.
            let event_id = engine
                .action_sink()?
                .send_message(
                    community_id,
                    &channel_id,
                    text,
                    &owner_pubkey_hex,
                    thread_anchor,
                )
                .await
                .map_err(WorkflowError::from)?;

            Ok(StepResult::Completed(serde_json::json!({
                "sent": true,
                "event_id": event_id,
            })))
        }

        SendDm { to, text: _ } => {
            warn!(run_id = %run_id, step = step_id, "SendDm not yet implemented (to={to})");
            // TODO (WF-07): emit DM event.
            Err(WorkflowError::NotImplemented("SendDm".into()))
        }

        SetChannelTopic { topic: _ } => {
            warn!(run_id = %run_id, step = step_id, "SetChannelTopic not yet implemented");
            // TODO (WF-07): update channel topic via DB.
            Err(WorkflowError::NotImplemented("SetChannelTopic".into()))
        }

        AddReaction { emoji } => {
            info!(run_id = %run_id, step = step_id, "AddReaction → :{emoji}:");
            if trigger_ctx.message_id.is_empty() {
                return Err(WorkflowError::InvalidDefinition(
                    "AddReaction: no trigger.message_id available".into(),
                ));
            }

            #[cfg(feature = "reqwest")]
            {
                let result = add_reaction_impl(&trigger_ctx.message_id, emoji).await?;
                Ok(StepResult::Completed(result))
            }

            #[cfg(not(feature = "reqwest"))]
            {
                warn!(
                    run_id = %run_id,
                    step = step_id,
                    "AddReaction: reqwest feature not enabled, skipping HTTP call"
                );
                Ok(StepResult::Completed(
                    serde_json::json!({ "added": false, "skipped": true }),
                ))
            }
        }

        CallWebhook {
            url,
            method,
            headers,
            body,
        } => {
            let method_str = method.as_deref().unwrap_or("POST");
            info!(run_id = %run_id, step = step_id, "CallWebhook → {method_str} {url}");

            #[cfg(feature = "reqwest")]
            {
                let result = call_webhook_impl(url, method_str, headers, body).await?;
                Ok(StepResult::Completed(result))
            }

            #[cfg(not(feature = "reqwest"))]
            {
                // reqwest not enabled — log and return placeholder.
                warn!(
                    run_id = %run_id, step = step_id,
                    "CallWebhook: reqwest feature not enabled, skipping HTTP call"
                );
                let _ = (headers, body); // suppress unused warnings
                Ok(StepResult::Completed(serde_json::json!({
                    "status": 0,
                    "body": null,
                    "skipped": true
                })))
            }
        }

        RequestApproval {
            from,
            message,
            timeout,
        } => {
            let timeout_str = timeout.as_deref().unwrap_or("24h");
            info!(
                run_id = %run_id, step = step_id,
                "RequestApproval from={from} timeout={timeout_str}: {message}"
            );

            let token = generate_approval_token(run_id, step_id);

            // The approval record is created by `WorkflowEngine::finalize_run`
            // once this suspension bubbles up through `execute_steps` — it has
            // the DB handle and run/workflow context needed for the insert.

            Ok(StepResult::Suspended {
                resume_token: token,
                reason: SuspendReason::Approval {
                    approver_spec: from.clone(),
                },
                timeout: timeout_str.to_owned(),
            })
        }

        AssignToAgent {
            agent,
            agent_pubkey: agent_pubkey_hint,
            instruction,
            timeout,
        } => {
            let timeout_str = timeout.as_deref().unwrap_or("24h");

            let wf_run = engine
                .db
                .get_workflow_run(community_id, run_id)
                .await
                .map_err(|e| {
                    WorkflowError::WebhookError(format!(
                        "AssignToAgent: failed to load workflow run {run_id}: {e}"
                    ))
                })?;
            let workflow = engine
                .db
                .get_workflow(community_id, wf_run.workflow_id)
                .await
                .map_err(|e| {
                    WorkflowError::WebhookError(format!(
                        "AssignToAgent: failed to load workflow {}: {e}",
                        wf_run.workflow_id
                    ))
                })?;
            let channel_id =
                resolve_send_message_channel(None, &trigger_ctx.channel_id, workflow.channel_id)?;
            let owner_pubkey_hex = hex::encode(&workflow.owner_pubkey);

            let sink = engine.action_sink()?;

            // When the step carries an explicit `agent_pubkey`, that's the
            // authoritative identity: verify it directly against channel
            // membership rather than resolving `agent` by name. This
            // disambiguates same-named agents and survives a rename. When
            // absent, fall back to case-insensitive exact-name resolution —
            // `send_message`'s own `@Name` mention resolution silently omits
            // the `p` tag for an unresolved or ambiguous name (it "sends to
            // nobody"), which would otherwise let this step appear to
            // succeed while no agent was actually woken. Surface either
            // failure mode as a typed step failure instead.
            let agent_pubkey = match agent_pubkey_hint {
                Some(pk) => sink
                    .verify_agent_membership(community_id, &channel_id, pk)
                    .await
                    .map_err(WorkflowError::from)?
                    .ok_or_else(|| {
                        WorkflowError::InvalidDefinition(format!(
                            "AssignToAgent: agent_pubkey '{pk}' is not a member of channel {channel_id}"
                        ))
                    })?,
                None => sink
                    .resolve_agent(community_id, &channel_id, agent)
                    .await
                    .map_err(WorkflowError::from)?
                    .ok_or_else(|| {
                        WorkflowError::InvalidDefinition(format!(
                            "AssignToAgent: agent '{agent}' does not resolve to exactly one member of channel {channel_id}"
                        ))
                    })?,
            };

            let text = format!("@{agent} {instruction}");

            info!(
                run_id = %run_id,
                step = step_id,
                channel = %channel_id,
                "AssignToAgent → {channel_id}: {text}"
            );

            // Anchor every step's prompt as a direct reply to the run's
            // thread root, not to the previous step's agent reply — keeps
            // the whole run flat (root, prompt1, reply1, prompt2, reply2,
            // ...) instead of a growing root→reply→prompt→reply chain that
            // gets harder to read as more steps are added.
            let prompt_event_id = sink
                .send_message(
                    community_id,
                    &channel_id,
                    &text,
                    &owner_pubkey_hex,
                    thread_anchor,
                )
                .await
                .map_err(WorkflowError::from)?;
            // Matches the relay's own `Timestamp::now()` at signing time
            // (same clock, moments apart) — avoids a broader ActionSink
            // return-type change just to carry this back from `send_message`.
            let prompt_created_at = unix_now();
            // The very first assign_to_agent prompt in a run is its own
            // thread root; later ones inherit the root from the anchor
            // carried forward by the prior step's suspend/resume.
            let (root_event_id, root_created_at) = match thread_anchor {
                Some(a) => (a.root_event_id.clone(), a.root_created_at),
                None => (prompt_event_id.clone(), prompt_created_at),
            };

            Ok(StepResult::Suspended {
                resume_token: prompt_event_id.clone(),
                reason: SuspendReason::AgentAssignment {
                    prompt_event_id,
                    prompt_created_at,
                    root_event_id,
                    root_created_at,
                    agent_pubkey,
                },
                timeout: timeout_str.to_owned(),
            })
        }

        Delay { duration } => {
            let secs = parse_duration_secs(duration)?;
            // Cap delay at 270 seconds (4.5 minutes) — must be less than default_timeout_secs (300s)
            // to avoid non-deterministic StepTimeout. Long delays (hours/days)
            // should use the scheduled resume pattern (future work: WF-09).
            const MAX_DELAY_SECS: u64 = 270;
            if secs > MAX_DELAY_SECS {
                return Err(WorkflowError::InvalidDefinition(format!(
                    "delay exceeds maximum of {MAX_DELAY_SECS} seconds (got {secs}s); \
                     use the scheduled resume pattern for long delays"
                )));
            }
            info!(run_id = %run_id, step = step_id, "Delay {duration} ({secs}s)");
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            Ok(StepResult::Completed(
                serde_json::json!({ "slept_secs": secs }),
            ))
        }
    }
    })
    .await
    .map_err(|error| {
        WorkflowError::WebhookError(format!("community write lease lost: {error}"))
    })?;
    let release = serving_write.finish().await.map_err(|error| {
        WorkflowError::WebhookError(format!("community write lease release failed: {error}"))
    });
    match result {
        Ok(value) => {
            release?;
            Ok(value)
        }
        Err(error) => {
            let _ = release;
            Err(error)
        }
    }
}

/// Generate a cryptographically random approval token.
///
/// Uses `Uuid::new_v4()` which draws from the OS CSPRNG (via the `getrandom`
/// crate). The `run_id` and `step_id` parameters are accepted for logging
/// context but are not mixed into the token — the UUID's own randomness is
/// sufficient and avoids the predictability of time-based entropy.
fn generate_approval_token(_run_id: Uuid, _step_id: &str) -> String {
    Uuid::new_v4().to_string()
}

/// Parse a duration string like "5m", "1h", "30s" into seconds.
///
/// Exposed as `pub(crate)` so `schema.rs` can use it for interval validation.
pub(crate) fn parse_duration_secs(duration: &str) -> Result<u64, WorkflowError> {
    let duration = duration.trim();
    if let Some(n) = duration.strip_suffix('h') {
        let hours: u64 = n.trim().parse().map_err(|_| {
            WorkflowError::InvalidDefinition(format!("invalid duration: {duration}"))
        })?;
        return hours.checked_mul(3600).ok_or_else(|| {
            WorkflowError::InvalidDefinition(format!("duration overflow: {duration}"))
        });
    }
    if let Some(n) = duration.strip_suffix('m') {
        let mins: u64 = n.trim().parse().map_err(|_| {
            WorkflowError::InvalidDefinition(format!("invalid duration: {duration}"))
        })?;
        return mins.checked_mul(60).ok_or_else(|| {
            WorkflowError::InvalidDefinition(format!("duration overflow: {duration}"))
        });
    }
    if let Some(n) = duration.strip_suffix('s') {
        let secs: u64 = n.trim().parse().map_err(|_| {
            WorkflowError::InvalidDefinition(format!("invalid duration: {duration}"))
        })?;
        return Ok(secs);
    }
    // Plain number — assume seconds.
    duration
        .parse()
        .map_err(|_| WorkflowError::InvalidDefinition(format!("invalid duration: {duration}")))
}

// is_private_ip is provided by buzz_core::network::is_private_ip

/// Resolve `host` to IP addresses and reject if any are private/reserved.
///
/// Uses the OS resolver (blocking, run on a threadpool via `spawn_blocking`).
/// Rejects the request if DNS resolution fails or returns zero addresses.
///
/// Returns the first validated IP address so the caller can pin DNS resolution
/// in the HTTP client, preventing DNS rebinding TOCTOU attacks.
#[cfg(feature = "reqwest")]
async fn check_ssrf(host: &str, port: u16) -> Result<std::net::IpAddr, WorkflowError> {
    let addr_str = format!("{host}:{port}");
    let addrs: Vec<std::net::IpAddr> = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        addr_str
            .to_socket_addrs()
            .map(|iter| iter.map(|sa| sa.ip()).collect::<Vec<_>>())
    })
    .await
    .map_err(|e| WorkflowError::WebhookError(format!("SSRF check task failed: {e}")))?
    .map_err(|e| WorkflowError::WebhookError(format!("DNS resolution failed: {e}")))?;

    if addrs.is_empty() {
        return Err(WorkflowError::WebhookError(
            "DNS resolution returned no addresses".into(),
        ));
    }

    debug!("Resolved webhook host '{}' → {:?}", host, addrs);

    for ip in &addrs {
        if buzz_core::network::is_private_ip(ip) {
            return Err(WorkflowError::WebhookError(format!(
                "SSRF blocked: '{host}' resolved to private/reserved address {ip}"
            )));
        }
    }

    Ok(addrs[0])
}

/// Maximum response body size for webhook calls (1 MiB).
#[cfg(feature = "reqwest")]
const WEBHOOK_MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[cfg(feature = "reqwest")]
async fn call_webhook_impl(
    url: &str,
    method: &str,
    headers: &Option<std::collections::HashMap<String, String>>,
    body: &Option<String>,
) -> Result<JsonValue, WorkflowError> {
    use reqwest::Client;
    use std::time::Duration;

    let parsed_url = reqwest::Url::parse(url)
        .map_err(|e| WorkflowError::WebhookError(format!("invalid URL: {e}")))?;

    let host = parsed_url
        .host_str()
        .ok_or_else(|| WorkflowError::WebhookError("URL has no host".into()))?;

    // Default ports: 443 for https, 80 for http.
    let port = parsed_url.port_or_known_default().unwrap_or(80);

    let safe_ip = check_ssrf(host, port).await?;

    // Client is built per-request because `resolve()` pins DNS for a specific host.
    // This disables connection pooling but is required for SSRF safety: without
    // pinning, reqwest performs its own DNS resolution which could return a
    // different address than the one validated above (DNS rebinding TOCTOU).
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        // A system proxy would resolve the original hostname itself, bypassing
        // the validated and pinned address above.
        .no_proxy()
        // Disable redirects — a redirect to an internal host bypasses the SSRF check.
        .redirect(reqwest::redirect::Policy::none())
        .resolve(host, std::net::SocketAddr::new(safe_ip, port))
        .build()
        .map_err(|e| WorkflowError::WebhookError(e.to_string()))?;

    let method_parsed = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|e| WorkflowError::WebhookError(e.to_string()))?;

    let mut req = client.request(method_parsed, url);

    if let Some(hdrs) = headers {
        for (k, v) in hdrs {
            req = req.header(k, v);
        }
    }

    if let Some(b) = body {
        req = req.body(b.clone());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| WorkflowError::WebhookError(e.to_string()))?;

    let status = resp.status().as_u16();

    // Read incrementally to prevent OOM from a malicious server returning a
    // multi-GB payload. `resp.bytes()` would buffer the entire body before we
    // could check the size; chunked reading lets us abort early.
    let mut body_bytes = Vec::new();
    let mut resp = resp;
    loop {
        let chunk = resp
            .chunk()
            .await
            .map_err(|e| WorkflowError::WebhookError(format!("reading response body: {e}")))?;
        match chunk {
            Some(bytes) => {
                body_bytes.extend_from_slice(&bytes);
                if body_bytes.len() > WEBHOOK_MAX_RESPONSE_BYTES {
                    return Err(WorkflowError::WebhookError(format!(
                        "response body exceeds {} byte limit",
                        WEBHOOK_MAX_RESPONSE_BYTES
                    )));
                }
            }
            None => break,
        }
    }

    let body_text = String::from_utf8_lossy(&body_bytes).into_owned();

    Ok(serde_json::json!({
        "status": status,
        "body": body_text,
    }))
}

/// Returns a shared `reqwest::Client` reused across all workflow HTTP calls.
/// Sharing a single client reuses the underlying connection pool.
#[cfg(feature = "reqwest")]
fn shared_http_client() -> &'static reqwest::Client {
    use std::sync::LazyLock;
    use std::time::Duration;
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client build must succeed")
    });
    &CLIENT
}

/// POST `{"emoji": emoji}` to `POST /api/messages/{message_id}/reactions`.
#[cfg(feature = "reqwest")]
async fn add_reaction_impl(message_id: &str, emoji: &str) -> Result<JsonValue, WorkflowError> {
    let base_url =
        std::env::var("BUZZ_RELAY_BASE_URL").unwrap_or_else(|_| "http://localhost:3000".to_owned());

    let url = format!("{base_url}/api/messages/{message_id}/reactions");

    let client = shared_http_client();

    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "emoji": emoji }));

    if let Ok(token) = std::env::var("BUZZ_API_TOKEN") {
        req = req.header("Authorization", format!("Bearer {token}"));
    } else if let Ok(pubkey) = std::env::var("BUZZ_RELAY_PUBKEY") {
        req = req.header("X-Pubkey", pubkey);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| WorkflowError::WebhookError(format!("AddReaction HTTP error: {e}")))?;

    let status = resp.status();

    if !status.is_success() {
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable>".to_owned());
        return Err(WorkflowError::WebhookError(format!(
            "AddReaction: relay returned {status} for message {message_id}: {body}"
        )));
    }

    let body_text = resp.text().await.unwrap_or_else(|_| String::new());
    let body_json: JsonValue = serde_json::from_str(&body_text)
        .unwrap_or_else(|_| serde_json::json!({ "raw": body_text }));

    Ok(serde_json::json!({
        "added": true,
        "status": status.as_u16(),
        "response": body_json,
    }))
}

/// A suspension pending persistence by the caller (`WorkflowEngine::finalize_run`).
///
/// Carries everything needed to write either a `workflow_approvals` row (for
/// `RequestApproval`) or a `workflow_agent_steps` row (for `AssignToAgent`),
/// plus the step context common to both.
#[derive(Debug)]
pub struct PendingSuspend {
    /// Resume token: the raw approval token (approvals) or the prompt event
    /// id (agent assignments).
    pub resume_token: String,
    /// What the run is waiting on.
    pub reason: SuspendReason,
    /// The suspended step's `id`.
    pub step_id: String,
    /// The suspended step's timeout duration string (e.g. `"24h"`).
    pub timeout: String,
}

/// Rich return type from `execute_run` / `execute_from_step`.
///
/// Carries enough information for the caller to:
/// - Persist the approval/agent-step record when suspended.
/// - Update the run's execution trace and current step in the DB.
/// - Resume execution from the correct step after the suspension resolves.
#[derive(Debug)]
pub struct ExecutionResult {
    /// Set when execution suspended at a `RequestApproval` or `AssignToAgent`
    /// step. `None` means the run completed normally.
    pub suspend: Option<PendingSuspend>,
    /// Index of the step that suspended (or the total step count on completion).
    pub step_index: usize,
    /// Accumulated step outputs at the point of suspension or completion.
    pub step_outputs: HashMap<String, JsonValue>,
    /// Execution trace: one entry per completed/skipped/waiting/failed step.
    pub trace: Vec<JsonValue>,
}

/// Execute a workflow run sequentially.
///
/// Steps run in order. Each step:
/// 1. Evaluates `if:` condition (skip if false).
/// 2. Resolves template variables in action fields.
/// 3. Dispatches the action.
/// 4. Stores the step output for use by later steps.
///
/// On `RequestApproval`: returns `ExecutionResult` with `approval_token = Some(token)`.
/// Caller must persist the approval record and update the run status.
///
/// Returns `ExecutionResult` with `approval_token = None` on normal completion.
///
/// Enforces `engine.config.max_concurrent` via a semaphore — returns
/// [`WorkflowError::CapacityExceeded`] immediately if all permits are taken.
/// Transitions the run to `Running` after acquiring a permit.
pub async fn execute_run(
    engine: &WorkflowEngine,
    community_id: CommunityId,
    run_id: Uuid,
    def: &WorkflowDef,
    trigger_ctx: &TriggerContext,
) -> Result<ExecutionResult, (WorkflowError, crate::error::PartialProgress)> {
    // Fail fast if all concurrency permits are in use — no queuing.
    let _permit = engine.run_semaphore.try_acquire().map_err(|_| {
        (
            WorkflowError::CapacityExceeded,
            crate::error::PartialProgress::default(),
        )
    })?;

    engine
        .db
        .update_workflow_run(
            community_id,
            run_id,
            buzz_db::workflow::RunStatus::Running,
            0,
            &serde_json::json!([]),
            None,
        )
        .await
        .map_err(|e| {
            (
                WorkflowError::from(e),
                crate::error::PartialProgress::default(),
            )
        })?;

    execute_steps(engine, community_id, run_id, def, trigger_ctx, 0, None).await
}

/// Resume execution from a specific step index (used for approval resume).
///
/// Acquires a concurrency permit from `engine.run_semaphore` before executing —
/// returns [`WorkflowError::CapacityExceeded`] immediately if all permits are
/// taken.
///
/// Transitions the run to `Running` after acquiring a permit, so that
/// approval-resumed runs correctly reflect their active state.
///
/// `initial_outputs` should be reconstructed from the execution trace before
/// calling this function on resume, so that steps after the resume point can
/// reference `{{steps.PREV_STEP.output.X}}` correctly.
pub async fn execute_from_step(
    engine: &WorkflowEngine,
    community_id: CommunityId,
    run_id: Uuid,
    def: &WorkflowDef,
    trigger_ctx: &TriggerContext,
    start_index: usize,
    initial_outputs: Option<HashMap<String, JsonValue>>,
) -> Result<ExecutionResult, (WorkflowError, crate::error::PartialProgress)> {
    // Fail fast if all concurrency permits are in use — no queuing.
    let _permit = engine.run_semaphore.try_acquire().map_err(|_| {
        (
            WorkflowError::CapacityExceeded,
            crate::error::PartialProgress::default(),
        )
    })?;

    // Mark run as Running now that we have a permit (resume from approval).
    // Preserve the existing execution trace from pre-approval steps.
    let existing_trace = match engine.db.get_workflow_run(community_id, run_id).await {
        Ok(r) => r.execution_trace,
        Err(e) => {
            warn!(
                run_id = %run_id,
                "Failed to read existing trace for resume — pre-approval trace will be lost: {e}"
            );
            serde_json::json!([])
        }
    };
    engine
        .db
        .update_workflow_run(
            community_id,
            run_id,
            buzz_db::workflow::RunStatus::Running,
            start_index as i32,
            &existing_trace,
            None,
        )
        .await
        .map_err(|e| {
            (
                WorkflowError::from(e),
                crate::error::PartialProgress::default(),
            )
        })?;

    execute_steps(
        engine,
        community_id,
        run_id,
        def,
        trigger_ctx,
        start_index,
        initial_outputs,
    )
    .await
}

/// Extract a [`crate::action_sink::ThreadAnchor`] from a completed
/// `assign_to_agent` step's `step_outputs` entry, if it carries a
/// `__reply_thread` blob (embedded by the relay's resume path — see
/// `try_resume_agent_step`/`agent_completion_to_output_json` in
/// `buzz-relay`'s `command_executor.rs`).
///
/// Returns `None` for non-agent-step outputs (no `__reply_thread` key) —
/// harmless, since `execute_steps` only uses this to *seed* the running
/// anchor for a resumed run, falling back to no anchor (top-level messages)
/// if nothing is found.
fn thread_anchor_from_step_output(output: &JsonValue) -> Option<crate::action_sink::ThreadAnchor> {
    let t = output.get("__reply_thread")?;
    Some(crate::action_sink::ThreadAnchor {
        root_event_id: t.get("root_event_id")?.as_str()?.to_owned(),
        root_created_at: t.get("root_created_at")?.as_i64()?,
    })
}

/// Internal: execute workflow steps starting from `start_index`, without
/// acquiring the semaphore. Called by both [`execute_run`] and
/// [`execute_from_step`] after they have already acquired a permit.
///
/// On error, returns `(WorkflowError, PartialProgress)` so callers can persist
/// the trace of steps completed before the failure.
async fn execute_steps(
    engine: &WorkflowEngine,
    community_id: CommunityId,
    run_id: Uuid,
    def: &WorkflowDef,
    trigger_ctx: &TriggerContext,
    start_index: usize,
    initial_outputs: Option<HashMap<String, JsonValue>>,
) -> Result<ExecutionResult, (WorkflowError, crate::error::PartialProgress)> {
    let mut step_outputs: HashMap<String, JsonValue> = initial_outputs.unwrap_or_default();
    let mut trace: Vec<JsonValue> = Vec::new();

    // Running NIP-10 thread anchor for `assign_to_agent` steps, so a
    // multi-step run threads into one conversation instead of independent
    // top-level (prompt, reply) pairs. Seeded once from the most recent
    // already-completed `assign_to_agent` step carried in via `step_outputs`
    // (i.e. this call is a resume) — see `thread_anchor_from_step_output`;
    // fresh anchors only appear when execution re-enters here on the next
    // resume. `None` until the run's first prompt exists, so early steps
    // post top-level.
    let thread_anchor: Option<crate::action_sink::ThreadAnchor> =
        def.steps.iter().take(start_index).rev().find_map(|s| {
            step_outputs
                .get(&s.id)
                .and_then(thread_anchor_from_step_output)
        });

    for (i, step) in def.steps.iter().enumerate() {
        if i < start_index {
            debug!(run_id = %run_id, step = %step.id, "Skipping already-executed step");
            continue;
        }

        // Captured once per step so every trace entry emitted below (skip,
        // failure, suspend, completion) reports the same `started_at` —
        // when the executor first picked this step up, not when a
        // particular sub-phase (condition eval vs dispatch) ran.
        let step_started_at = unix_now();

        if let Some(expr) = &step.if_expr {
            match evaluate_condition(expr, trigger_ctx, &step_outputs).await {
                Ok(true) => {
                    debug!(run_id = %run_id, step = %step.id, "Condition true — running step");
                }
                Ok(false) => {
                    info!(run_id = %run_id, step = %step.id, "Condition false — skipping step");
                    trace.push(serde_json::json!({
                        "step_id": step.id,
                        "status": "skipped",
                        "started_at": step_started_at,
                        "completed_at": unix_now(),
                    }));
                    continue;
                }
                Err(e) => {
                    warn!(run_id = %run_id, step = %step.id, "Condition error: {e}");
                    return Err(fail_step(e, &step.id, step_started_at, i, trace));
                }
            }
        }

        let resolved_action = match resolve_step_templates(step, trigger_ctx, &step_outputs) {
            Ok(a) => a,
            Err(e) => return Err(fail_step(e, &step.id, step_started_at, i, trace)),
        };

        let timeout_secs = step
            .timeout_secs
            .unwrap_or(engine.config.default_timeout_secs);
        let dispatch_result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            dispatch_action(
                &step.id,
                &resolved_action,
                engine,
                community_id,
                run_id,
                trigger_ctx,
                thread_anchor.as_ref(),
            ),
        )
        .await;

        let result = match dispatch_result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(fail_step(e, &step.id, step_started_at, i, trace)),
            Err(_timeout) => {
                let timeout_err = WorkflowError::StepTimeout {
                    step_id: step.id.clone(),
                    timeout_secs,
                };
                return Err(fail_step(timeout_err, &step.id, step_started_at, i, trace));
            }
        };

        match result {
            StepResult::Completed(output) => {
                debug!(run_id = %run_id, step = %step.id, "Step completed");
                trace.push(serde_json::json!({
                    "step_id": step.id,
                    "status": "completed",
                    "output": output,
                    "started_at": step_started_at,
                    "completed_at": unix_now(),
                }));
                step_outputs.insert(step.id.clone(), output);
            }
            StepResult::Suspended {
                resume_token,
                reason,
                timeout,
            } => {
                info!(
                    run_id = %run_id, step = %step.id,
                    "Step suspended — awaiting external resume (token: <redacted>)"
                );
                // For agent assignments, carry the prompt's own thread
                // identity in the trace entry — the resume path (relay's
                // `try_resume_agent_step`) reads it back out to build the
                // next step's `ThreadAnchor` without a DB round-trip.
                let thread = if let SuspendReason::AgentAssignment {
                    prompt_event_id,
                    prompt_created_at,
                    root_event_id,
                    root_created_at,
                    ..
                } = &reason
                {
                    serde_json::json!({
                        "event_id": prompt_event_id,
                        "created_at": prompt_created_at,
                        "root_event_id": root_event_id,
                        "root_created_at": root_created_at,
                    })
                } else {
                    JsonValue::Null
                };
                trace.push(serde_json::json!({
                    "step_id": step.id,
                    "status": "waiting",
                    "started_at": step_started_at,
                    "completed_at": null,
                    "__prompt_thread": thread,
                }));
                // Return the token and current state so the caller can persist the
                // approval/agent-step record and update the run's execution trace.
                return Ok(ExecutionResult {
                    suspend: Some(PendingSuspend {
                        resume_token,
                        reason,
                        step_id: step.id.clone(),
                        timeout,
                    }),
                    step_index: i,
                    step_outputs,
                    trace,
                });
            }
            StepResult::Skipped => {
                debug!(run_id = %run_id, step = %step.id, "Step skipped");
                trace.push(serde_json::json!({
                    "step_id": step.id,
                    "status": "skipped",
                    "started_at": step_started_at,
                    "completed_at": unix_now(),
                }));
            }
        }
    }

    info!(run_id = %run_id, "Workflow run completed");
    Ok(ExecutionResult {
        suspend: None,
        step_index: def.steps.len(),
        step_outputs,
        trace,
    })
}

/// Current Unix time in seconds, for `started_at`/`completed_at` trace
/// fields. Matches the `i64` Unix-seconds convention used elsewhere in the
/// relay (e.g. `event.created_at.as_secs() as i64`) rather than RFC3339, so
/// it round-trips through the desktop's `number | null` trace-entry contract
/// (`RawTraceEntry.started_at`/`completed_at` in
/// `desktop/src/shared/api/tauriWorkflows.ts`) without a format conversion.
fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Build the `(error, progress)` pair for a failed step, appending its
/// `failed` trace entry — shared tail of every failure exit in
/// [`execute_steps`].
fn fail_step(
    err: WorkflowError,
    step_id: &str,
    started_at: i64,
    step_index: usize,
    mut trace: Vec<JsonValue>,
) -> (WorkflowError, crate::error::PartialProgress) {
    trace.push(serde_json::json!({
        "step_id": step_id,
        "status": "failed",
        "error": err.to_string(),
        "started_at": started_at,
        "completed_at": unix_now(),
    }));
    (err, crate::error::PartialProgress { step_index, trace })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_trigger() -> TriggerContext {
        TriggerContext {
            text: "P1 incident in production".to_owned(),
            author: "abc123def456".to_owned(),
            channel_id: "channel-uuid-here".to_owned(),
            timestamp: "1700000000".to_owned(),
            emoji: "fire".to_owned(),
            message_id: "event-id-hex".to_owned(),
            webhook_fields: HashMap::new(),
        }
    }

    #[test]
    fn resolve_trigger_text() {
        let ctx = make_trigger();
        let out = resolve_template("Alert: {{trigger.text}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "Alert: P1 incident in production");
    }

    #[test]
    fn resolve_trigger_author() {
        let ctx = make_trigger();
        let out = resolve_template("By {{trigger.author}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "By abc123def456");
    }

    #[test]
    fn resolve_step_output() {
        let ctx = make_trigger();
        let mut outputs = HashMap::new();
        outputs.insert("ask".to_owned(), json!({ "replied": "yes" }));
        let out = resolve_template("Reply: {{steps.ask.output.replied}}", &ctx, &outputs).unwrap();
        assert_eq!(out, "Reply: yes");
    }

    #[test]
    fn resolve_unknown_variable_left_literal() {
        let ctx = make_trigger();
        let out = resolve_template("{{unknown.var}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "{{unknown.var}}");
    }

    #[test]
    fn resolve_truncate_filter() {
        let ctx = make_trigger();
        let out =
            resolve_template("{{trigger.text | truncate(5)}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "P1 in");
        assert_eq!(out.chars().count(), 5);
    }

    #[test]
    fn resolve_npub_filter_encodes_hex_pubkey() {
        let mut ctx = make_trigger();
        ctx.author = "e17e5abf7b1dbd363f0ed6fbda2455609727b2555428dea251388c542cd2f03f".to_owned();
        let out = resolve_template("{{trigger.author | npub}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(
            out,
            "npub1u9l940mmrk7nv0cw6maa5fz4vztj0vj42s5dagj38zx9gtxj7qls94fpux"
        );
    }

    #[test]
    fn resolve_truncate_pubkey_is_alias_for_npub() {
        let mut ctx = make_trigger();
        ctx.author = "e17e5abf7b1dbd363f0ed6fbda2455609727b2555428dea251388c542cd2f03f".to_owned();
        let out = resolve_template(
            "{{trigger.author | truncate_pubkey}}",
            &ctx,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(
            out,
            "npub1u9l940mmrk7nv0cw6maa5fz4vztj0vj42s5dagj38zx9gtxj7qls94fpux"
        );
    }

    #[test]
    fn resolve_no_templates_fast_path() {
        let ctx = make_trigger();
        let out = resolve_template("no templates here", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "no templates here");
    }

    #[test]
    fn resolve_multiple_templates_in_one_string() {
        let ctx = make_trigger();
        let out = resolve_template(
            "{{trigger.author}} said: {{trigger.text}}",
            &ctx,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(out, "abc123def456 said: P1 incident in production");
    }

    #[test]
    fn resolve_webhook_field() {
        let mut ctx = make_trigger();
        ctx.webhook_fields
            .insert("service".to_owned(), "api-gateway".to_owned());
        let out = resolve_template("Service: {{trigger.service}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "Service: api-gateway");
    }

    #[tokio::test]
    async fn condition_true_when_text_contains_p1() {
        let ctx = make_trigger(); // text = "P1 incident in production"
        let result =
            evaluate_condition("str_contains(trigger_text, \"P1\")", &ctx, &HashMap::new())
                .await
                .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_false_when_text_does_not_contain_p1() {
        let mut ctx = make_trigger();
        ctx.text = "normal message".to_owned();
        let result =
            evaluate_condition("str_contains(trigger_text, \"P1\")", &ctx, &HashMap::new())
                .await
                .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn condition_or_expression() {
        let ctx = make_trigger(); // text contains "P1"
        let result = evaluate_condition(
            "str_contains(trigger_text, \"P1\") || str_contains(trigger_text, \"SEV1\")",
            &ctx,
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_step_output_bool() {
        let ctx = make_trigger();
        let mut outputs = HashMap::new();
        outputs.insert("request".to_owned(), json!({ "approved": true }));
        let result = evaluate_condition("steps_request_output_approved == true", &ctx, &outputs)
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_step_output_bool_false() {
        let ctx = make_trigger();
        let mut outputs = HashMap::new();
        outputs.insert("request".to_owned(), json!({ "approved": false }));
        let result = evaluate_condition("steps_request_output_approved == false", &ctx, &outputs)
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_invalid_expression_returns_error() {
        let ctx = make_trigger();
        let err = evaluate_condition("this is not valid evalexpr @@@@", &ctx, &HashMap::new())
            .await
            .unwrap_err();
        assert!(matches!(err, WorkflowError::ConditionError(_)));
    }

    #[tokio::test]
    async fn condition_exceeding_max_expr_len_is_rejected() {
        let ctx = make_trigger();
        // Construct an expression that exceeds MAX_EXPR_LEN (4096 bytes).
        let long_expr = "true || ".repeat(625); // 8 * 625 = 5000 bytes
        let err = evaluate_condition(&long_expr, &ctx, &HashMap::new())
            .await
            .unwrap_err();
        match &err {
            WorkflowError::ConditionError(msg) => {
                assert!(
                    msg.contains("exceeds") || msg.contains("limit"),
                    "expected 'exceeds' or 'limit' in error message, got: {msg}"
                );
            }
            other => panic!("expected ConditionError, got: {other:?}"),
        }
    }

    #[test]
    fn parse_duration_hours() {
        assert_eq!(parse_duration_secs("1h").unwrap(), 3600);
        assert_eq!(parse_duration_secs("2h").unwrap(), 7200);
    }

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(parse_duration_secs("5m").unwrap(), 300);
        assert_eq!(parse_duration_secs("30m").unwrap(), 1800);
    }

    #[test]
    fn parse_duration_seconds() {
        assert_eq!(parse_duration_secs("10s").unwrap(), 10);
        assert_eq!(parse_duration_secs("60s").unwrap(), 60);
    }

    #[test]
    fn parse_duration_plain_number() {
        assert_eq!(parse_duration_secs("42").unwrap(), 42);
    }

    #[test]
    fn parse_duration_invalid() {
        assert!(parse_duration_secs("not-a-duration").is_err());
    }

    #[test]
    fn resolve_unclosed_template_emits_literally() {
        // An unclosed `{{` should be emitted literally without panicking.
        let ctx = make_trigger();
        let out = resolve_template("Hello {{trigger.text", &ctx, &HashMap::new()).unwrap();
        // The unclosed `{{` and remaining text are emitted as-is.
        assert!(
            out.contains("{{"),
            "unclosed {{ should appear literally in output"
        );
    }

    #[test]
    fn resolve_empty_template_string() {
        let ctx = make_trigger();
        let out = resolve_template("", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "");
    }

    #[test]
    fn resolve_template_with_only_literal_text() {
        let ctx = make_trigger();
        let out = resolve_template("no placeholders at all", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "no placeholders at all");
    }

    #[test]
    fn resolve_multiple_different_trigger_fields() {
        let ctx = make_trigger();
        let out = resolve_template(
            "channel={{trigger.channel_id}} ts={{trigger.timestamp}} emoji={{trigger.emoji}}",
            &ctx,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(out, "channel=channel-uuid-here ts=1700000000 emoji=fire");
    }

    #[test]
    fn resolve_trigger_message_id() {
        let ctx = make_trigger();
        let out = resolve_template("msg={{trigger.message_id}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "msg=event-id-hex");
    }

    #[test]
    fn resolve_step_output_boolean_value() {
        let ctx = make_trigger();
        let mut outputs = HashMap::new();
        outputs.insert("gate".to_owned(), json!({ "approved": true }));
        let out =
            resolve_template("Approved: {{steps.gate.output.approved}}", &ctx, &outputs).unwrap();
        assert_eq!(out, "Approved: true");
    }

    #[test]
    fn resolve_step_output_number_value() {
        let ctx = make_trigger();
        let mut outputs = HashMap::new();
        outputs.insert("count".to_owned(), json!({ "total": 42 }));
        let out = resolve_template("Total: {{steps.count.output.total}}", &ctx, &outputs).unwrap();
        assert_eq!(out, "Total: 42");
    }

    #[test]
    fn resolve_step_output_null_value_is_empty_string() {
        let ctx = make_trigger();
        let mut outputs = HashMap::new();
        outputs.insert("step".to_owned(), json!({ "val": null }));
        let out = resolve_template("Val: {{steps.step.output.val}}", &ctx, &outputs).unwrap();
        assert_eq!(out, "Val: ");
    }

    #[test]
    fn resolve_unknown_step_id_left_literal() {
        let ctx = make_trigger();
        let out =
            resolve_template("{{steps.nonexistent.output.field}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "{{steps.nonexistent.output.field}}");
    }

    #[test]
    fn resolve_step_output_missing_field_left_literal() {
        let ctx = make_trigger();
        let mut outputs = HashMap::new();
        outputs.insert("step".to_owned(), json!({ "other": "value" }));
        let out = resolve_template("{{steps.step.output.missing}}", &ctx, &outputs).unwrap();
        assert_eq!(out, "{{steps.step.output.missing}}");
    }

    #[test]
    fn resolve_truncate_zero_chars() {
        let ctx = make_trigger();
        let out =
            resolve_template("{{trigger.text | truncate(0)}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "");
    }

    #[test]
    fn resolve_truncate_longer_than_string() {
        let ctx = make_trigger(); // text = "P1 incident in production" (25 chars)
        let out =
            resolve_template("{{trigger.text | truncate(1000)}}", &ctx, &HashMap::new()).unwrap();
        // Truncating to more than the string length returns the full string.
        assert_eq!(out, "P1 incident in production");
    }

    #[test]
    fn resolve_pubkey_filter_non_pubkey_passes_through() {
        // Values that are not valid hex pubkeys are returned unchanged.
        let mut ctx = make_trigger();
        ctx.author = "short".to_owned();
        let out = resolve_template(
            "{{trigger.author | truncate_pubkey}}",
            &ctx,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(out, "short");
    }

    #[test]
    fn resolve_npub_filter_passes_npub_through() {
        // Already-encoded npubs are not valid hex, so they pass through intact.
        let mut ctx = make_trigger();
        ctx.author = "npub1u9l940mmrk7nv0cw6maa5fz4vztj0vj42s5dagj38zx9gtxj7qls94fpux".to_owned();
        let out = resolve_template("{{trigger.author | npub}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, ctx.author);
    }

    #[test]
    fn resolve_unknown_filter_returns_error() {
        let ctx = make_trigger();
        let err = resolve_template(
            "{{trigger.text | nonexistent_filter}}",
            &ctx,
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(matches!(err, WorkflowError::TemplateError(_)));
    }

    #[test]
    fn resolve_truncate_invalid_number_returns_error() {
        let ctx = make_trigger();
        let err = resolve_template("{{trigger.text | truncate(abc)}}", &ctx, &HashMap::new())
            .unwrap_err();
        assert!(matches!(err, WorkflowError::TemplateError(_)));
    }

    #[test]
    fn resolve_adjacent_templates_no_separator() {
        let ctx = make_trigger();
        let out =
            resolve_template("{{trigger.author}}{{trigger.emoji}}", &ctx, &HashMap::new()).unwrap();
        assert_eq!(out, "abc123def456fire");
    }

    #[tokio::test]
    async fn condition_and_expression_both_true() {
        let ctx = make_trigger(); // text = "P1 incident in production"
        let result = evaluate_condition(
            "str_contains(trigger_text, \"P1\") && str_contains(trigger_text, \"production\")",
            &ctx,
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_and_expression_one_false() {
        let ctx = make_trigger(); // text = "P1 incident in production"
        let result = evaluate_condition(
            "str_contains(trigger_text, \"P1\") && str_contains(trigger_text, \"staging\")",
            &ctx,
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn condition_not_expression() {
        let ctx = make_trigger(); // text = "P1 incident in production"
        let result =
            evaluate_condition("!str_contains(trigger_text, \"P2\")", &ctx, &HashMap::new())
                .await
                .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_str_starts_with() {
        let ctx = make_trigger(); // text = "P1 incident in production"
        let result = evaluate_condition(
            "str_starts_with(trigger_text, \"P1\")",
            &ctx,
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_str_ends_with() {
        let ctx = make_trigger(); // text = "P1 incident in production"
        let result = evaluate_condition(
            "str_ends_with(trigger_text, \"production\")",
            &ctx,
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_str_len() {
        let ctx = make_trigger(); // text = "P1 incident in production" (25 chars)
        let result = evaluate_condition("str_len(trigger_text) > 10", &ctx, &HashMap::new())
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_str_len_exact() {
        let mut ctx = make_trigger();
        ctx.text = "hello".to_owned(); // exactly 5 chars
        let result = evaluate_condition("str_len(trigger_text) == 5", &ctx, &HashMap::new())
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_emoji_field() {
        let ctx = make_trigger(); // emoji = "fire"
        let result = evaluate_condition("trigger_emoji == \"fire\"", &ctx, &HashMap::new())
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_author_field() {
        let ctx = make_trigger(); // author = "abc123def456"
        let result = evaluate_condition(
            "str_starts_with(trigger_author, \"abc\")",
            &ctx,
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_webhook_field_registered() {
        let mut ctx = make_trigger();
        ctx.webhook_fields
            .insert("severity".to_owned(), "critical".to_owned());
        let result = evaluate_condition("trigger_severity == \"critical\"", &ctx, &HashMap::new())
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_step_output_string_comparison() {
        let ctx = make_trigger();
        let mut outputs = HashMap::new();
        outputs.insert("fetch".to_owned(), json!({ "status": "ok" }));
        let result = evaluate_condition("steps_fetch_output_status == \"ok\"", &ctx, &outputs)
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_step_output_integer_comparison() {
        let ctx = make_trigger();
        let mut outputs = HashMap::new();
        outputs.insert("count".to_owned(), json!({ "n": 5 }));
        let result = evaluate_condition("steps_count_output_n >= 5", &ctx, &outputs)
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_complex_nested_boolean() {
        let ctx = make_trigger(); // text = "P1 incident in production"
        let result = evaluate_condition(
            "(str_contains(trigger_text, \"P1\") || str_contains(trigger_text, \"P2\")) && str_contains(trigger_text, \"production\")",
            &ctx,
            &HashMap::new(),
        )
        .await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn condition_false_literal() {
        let ctx = make_trigger();
        let result = evaluate_condition("false", &ctx, &HashMap::new())
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn condition_true_literal() {
        let ctx = make_trigger();
        let result = evaluate_condition("true", &ctx, &HashMap::new())
            .await
            .unwrap();
        assert!(result);
    }

    #[test]
    fn trigger_context_get_field_known_fields() {
        let ctx = make_trigger();
        assert_eq!(ctx.get_field("text"), Some("P1 incident in production"));
        assert_eq!(ctx.get_field("author"), Some("abc123def456"));
        assert_eq!(ctx.get_field("channel_id"), Some("channel-uuid-here"));
        assert_eq!(ctx.get_field("timestamp"), Some("1700000000"));
        assert_eq!(ctx.get_field("emoji"), Some("fire"));
        assert_eq!(ctx.get_field("message_id"), Some("event-id-hex"));
    }

    #[test]
    fn trigger_context_get_field_unknown_returns_none() {
        let ctx = make_trigger();
        assert!(ctx.get_field("nonexistent").is_none());
        assert!(ctx.get_field("").is_none());
    }

    #[test]
    fn trigger_context_get_field_webhook_field() {
        let mut ctx = make_trigger();
        ctx.webhook_fields
            .insert("repo".to_owned(), "buzz".to_owned());
        assert_eq!(ctx.get_field("repo"), Some("buzz"));
    }

    #[test]
    fn trigger_context_default_has_empty_fields() {
        let ctx = TriggerContext::default();
        assert_eq!(ctx.text, "");
        assert_eq!(ctx.author, "");
        assert_eq!(ctx.channel_id, "");
        assert_eq!(ctx.timestamp, "");
        assert_eq!(ctx.emoji, "");
        assert_eq!(ctx.message_id, "");
        assert!(ctx.webhook_fields.is_empty());
    }

    #[test]
    fn send_message_uses_bound_workflow_channel_by_default() {
        let workflow_channel_id = Uuid::new_v4();
        let resolved = resolve_send_message_channel(None, "", Some(workflow_channel_id))
            .expect("bound channel should be used");
        assert_eq!(resolved, workflow_channel_id.to_string());
    }

    #[test]
    fn send_message_rejects_cross_channel_override_for_bound_workflow() {
        let workflow_channel_id = Uuid::new_v4();
        let other_channel_id = Uuid::new_v4();
        let err = resolve_send_message_channel(
            Some(&other_channel_id.to_string()),
            "",
            Some(workflow_channel_id),
        )
        .unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidDefinition(_)));
        assert!(
            err.to_string().contains("channel override must match"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn send_message_canonicalizes_valid_explicit_override_for_global_workflow() {
        let override_channel_id = Uuid::new_v4();
        let resolved =
            resolve_send_message_channel(Some(&override_channel_id.to_string()), "", None)
                .expect("override should be accepted");
        assert_eq!(resolved, override_channel_id.to_string());
    }

    // -- AssignToAgent dispatch (requires Postgres) -------------------------
    //
    // `dispatch_action` loads the run's workflow via `engine.db`, so exercising
    // it end-to-end needs a real DB even though the interesting behavior is in
    // the sink. A `MockAgentSink` stands in for the relay's `RelayActionSink`
    // so these tests don't need a running relay.

    use crate::action_sink::ActionSinkError;
    use std::sync::Mutex;

    /// Test double for [`crate::ActionSink`]. `agents` maps a display name to
    /// `Some(pubkey_bytes)` (resolves) or `None` (unresolved/ambiguous),
    /// mirroring the real sink's `resolve_agent` contract. `members` is the
    /// set of pubkeys (hex) considered channel members, for
    /// `verify_agent_membership`.
    struct MockAgentSink {
        agents: HashMap<String, Option<Vec<u8>>>,
        sent: Mutex<Vec<(String, String)>>, // (channel_id, text)
        members: HashMap<String, Vec<u8>>,  // pubkey_hex -> pubkey_bytes
    }

    impl crate::ActionSink for MockAgentSink {
        fn send_message(
            &self,
            _community_id: CommunityId,
            channel_id: &str,
            text: &str,
            _author_pubkey: &str,
            _reply_to: Option<&crate::action_sink::ThreadAnchor>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, ActionSinkError>> + Send + '_>,
        > {
            let channel_id = channel_id.to_owned();
            let text = text.to_owned();
            Box::pin(async move {
                self.sent
                    .lock()
                    .expect("lock")
                    .push((channel_id, text.clone()));
                Ok(format!("{:064x}", text.len())) // fake but distinct event id
            })
        }

        fn resolve_agent<'a>(
            &'a self,
            _community_id: CommunityId,
            _channel_id: &'a str,
            name: &'a str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<Option<Vec<u8>>, ActionSinkError>>
                    + Send
                    + 'a,
            >,
        > {
            let result = self.agents.get(name).cloned().unwrap_or(None);
            Box::pin(async move { Ok(result) })
        }

        fn verify_agent_membership<'a>(
            &'a self,
            _community_id: CommunityId,
            _channel_id: &'a str,
            pubkey_hex: &'a str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<Option<Vec<u8>>, ActionSinkError>>
                    + Send
                    + 'a,
            >,
        > {
            let result = self.members.get(pubkey_hex).cloned();
            Box::pin(async move { Ok(result) })
        }
    }

    async fn setup_engine_with_sink(
        sink: MockAgentSink,
    ) -> (
        std::sync::Arc<crate::WorkflowEngine>,
        buzz_db::Db,
        CommunityId,
        Uuid,
    ) {
        let db = crate::tests_support::setup_db().await;
        let creator = nostr::Keys::generate().public_key().to_bytes().to_vec();
        let (community, channel_id) =
            crate::tests_support::setup_channel(&db, &creator, &creator).await;
        let engine = std::sync::Arc::new(crate::WorkflowEngine::new(
            db.clone(),
            crate::WorkflowConfig::default(),
        ));
        engine.set_action_sink(std::sync::Arc::new(sink));
        (engine, db, community, channel_id)
    }

    /// Shared ritual for the Postgres-backed dispatch/execute tests: create
    /// an owner, a webhook-triggered workflow with the given `steps` JSON,
    /// and a pending run. Returns the run id, a channel-scoped trigger
    /// context, and the raw definition JSON (for tests that re-parse it).
    async fn make_assign_run(
        db: &buzz_db::Db,
        community: CommunityId,
        channel_id: Uuid,
        steps: serde_json::Value,
    ) -> (Uuid, TriggerContext, String) {
        let owner = nostr::Keys::generate().public_key().to_bytes().to_vec();
        db.ensure_user(community, &owner).await.expect("owner user");
        let def_json = serde_json::json!({
            "name": "assign-agent",
            "trigger": {"on": "webhook"},
            "steps": steps,
        })
        .to_string();
        let workflow_id = db
            .create_workflow(
                community,
                Some(channel_id),
                &owner,
                "assign-agent",
                &def_json,
                &[2u8; 32],
            )
            .await
            .expect("create workflow");
        let run_id = db
            .create_workflow_run(community, workflow_id, None, None)
            .await
            .expect("create run");
        let trigger_ctx = TriggerContext {
            channel_id: channel_id.to_string(),
            ..Default::default()
        };
        (run_id, trigger_ctx, def_json)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn dispatch_assign_to_agent_resolved_suspends_with_prompt_event_id() {
        let agent_pubkey = vec![7u8; 32];
        let mut agents = HashMap::new();
        agents.insert("Lep".to_owned(), Some(agent_pubkey.clone()));
        let sink = MockAgentSink {
            agents,
            sent: Mutex::new(Vec::new()),
            members: HashMap::new(),
        };
        let (engine, db, community, channel_id) = setup_engine_with_sink(sink).await;

        let (run_id, trigger_ctx, _def_json) = make_assign_run(
            &db,
            community,
            channel_id,
            serde_json::json!([{
                "id": "assign", "action": "assign_to_agent",
                "agent": "Lep", "instruction": "please investigate"
            }]),
        )
        .await;

        let action = ActionDef::AssignToAgent {
            agent: "Lep".to_owned(),
            agent_pubkey: None,
            instruction: "please investigate".to_owned(),
            timeout: None,
        };

        let result = dispatch_action(
            "assign",
            &action,
            &engine,
            community,
            run_id,
            &trigger_ctx,
            None,
        )
        .await
        .expect("dispatch should succeed");

        match result {
            StepResult::Suspended {
                resume_token,
                reason,
                timeout,
            } => {
                assert!(!resume_token.is_empty());
                assert_eq!(timeout, "24h", "default timeout should be 24h");
                match reason {
                    SuspendReason::AgentAssignment {
                        prompt_event_id,
                        agent_pubkey: got_pubkey,
                        ..
                    } => {
                        assert_eq!(prompt_event_id, resume_token);
                        assert_eq!(got_pubkey, agent_pubkey);
                    }
                    other => panic!("expected AgentAssignment, got {other:?}"),
                }
            }
            other => panic!("expected Suspended, got {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn dispatch_assign_to_agent_with_pubkey_bypasses_name_resolution() {
        let agent_pubkey = vec![7u8; 32];
        let pubkey_hex = hex::encode(&agent_pubkey);
        // Deliberately do NOT register "Lep" in `agents` — if dispatch fell
        // back to name resolution despite the explicit `agent_pubkey`, this
        // step would fail with "does not resolve to exactly one member".
        let mut members = HashMap::new();
        members.insert(pubkey_hex.clone(), agent_pubkey.clone());
        let sink = MockAgentSink {
            agents: HashMap::new(),
            sent: Mutex::new(Vec::new()),
            members,
        };
        let (engine, db, community, channel_id) = setup_engine_with_sink(sink).await;

        let (run_id, trigger_ctx, _def_json) = make_assign_run(
            &db,
            community,
            channel_id,
            serde_json::json!([{
                "id": "assign", "action": "assign_to_agent",
                "agent": "Lep", "agent_pubkey": pubkey_hex, "instruction": "please investigate"
            }]),
        )
        .await;

        let action = ActionDef::AssignToAgent {
            agent: "Lep".to_owned(),
            agent_pubkey: Some(pubkey_hex),
            instruction: "please investigate".to_owned(),
            timeout: None,
        };

        let result = dispatch_action(
            "assign",
            &action,
            &engine,
            community,
            run_id,
            &trigger_ctx,
            None,
        )
        .await
        .expect("dispatch should succeed via pubkey verification");

        match result {
            StepResult::Suspended {
                reason:
                    SuspendReason::AgentAssignment {
                        agent_pubkey: got, ..
                    },
                ..
            } => assert_eq!(got, agent_pubkey),
            other => panic!("expected Suspended/AgentAssignment, got {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn dispatch_assign_to_agent_pubkey_not_a_member_is_typed_failure() {
        let sink = MockAgentSink {
            agents: HashMap::new(),
            sent: Mutex::new(Vec::new()),
            members: HashMap::new(), // pubkey below matches no member
        };
        let (engine, db, community, channel_id) = setup_engine_with_sink(sink).await;

        let pubkey_hex = hex::encode([9u8; 32]);
        let (run_id, trigger_ctx, _def_json) = make_assign_run(
            &db,
            community,
            channel_id,
            serde_json::json!([{
                "id": "assign", "action": "assign_to_agent",
                "agent": "Lep", "agent_pubkey": pubkey_hex, "instruction": "please investigate"
            }]),
        )
        .await;

        let action = ActionDef::AssignToAgent {
            agent: "Lep".to_owned(),
            agent_pubkey: Some(pubkey_hex),
            instruction: "please investigate".to_owned(),
            timeout: None,
        };

        let err = dispatch_action(
            "assign",
            &action,
            &engine,
            community,
            run_id,
            &trigger_ctx,
            None,
        )
        .await
        .expect_err("dispatch should fail: pubkey is not a channel member");
        assert!(
            err.to_string().contains("is not a member of channel"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn dispatch_assign_to_agent_unresolved_name_is_typed_failure_not_silent_success() {
        // Agent name matches no member (or is ambiguous) — the sink's
        // resolve_agent returns Ok(None). Dispatch must surface this as a
        // step failure, not a "successful" suspend with no one actually
        // mentioned (which is send_message's own silent behavior for
        // unresolved @mentions).
        let sink = MockAgentSink {
            agents: HashMap::new(), // "Nobody" resolves to None
            sent: Mutex::new(Vec::new()),
            members: HashMap::new(),
        };
        let (engine, db, community, channel_id) = setup_engine_with_sink(sink).await;

        let (run_id, trigger_ctx, _def_json) = make_assign_run(
            &db,
            community,
            channel_id,
            serde_json::json!([{
                "id": "assign", "action": "assign_to_agent",
                "agent": "Nobody", "instruction": "please investigate"
            }]),
        )
        .await;

        let action = ActionDef::AssignToAgent {
            agent: "Nobody".to_owned(),
            agent_pubkey: None,
            instruction: "please investigate".to_owned(),
            timeout: None,
        };

        let err = dispatch_action(
            "assign",
            &action,
            &engine,
            community,
            run_id,
            &trigger_ctx,
            None,
        )
        .await
        .expect_err("unresolved agent name must be a typed failure");

        assert!(
            matches!(err, WorkflowError::InvalidDefinition(_)),
            "expected InvalidDefinition, got: {err}"
        );
        assert!(
            err.to_string().contains("Nobody"),
            "error should name the unresolved agent: {err}"
        );
    }

    // -- Trace timestamps across step outcomes (requires Postgres) ---------
    //
    // Design doc "Test plan" bullet: trace entries carry started_at/
    // completed_at for every action type, including a `waiting` entry on
    // suspend and a `failed` entry on failure. `execute_run` is exercised
    // directly (rather than unit-testing `execute_steps` in isolation, which
    // is private) so the trace shape asserted here is exactly what a real
    // caller (the relay) receives.

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn trace_entries_carry_started_and_completed_at_for_completed_skipped_and_waiting() {
        let agent_pubkey = vec![9u8; 32];
        let mut agents = HashMap::new();
        agents.insert("Lep".to_owned(), Some(agent_pubkey));
        let sink = MockAgentSink {
            agents,
            sent: Mutex::new(Vec::new()),
            members: HashMap::new(),
        };
        let (engine, db, community, channel_id) = setup_engine_with_sink(sink).await;

        let (run_id, trigger_ctx, def_json) = make_assign_run(
            &db,
            community,
            channel_id,
            serde_json::json!([
                {
                    "id": "notify",
                    "action": "send_message",
                    "text": "starting up"
                },
                {
                    "id": "maybe-skip",
                    "action": "send_message",
                    "text": "should not run",
                    "if": "false"
                },
                {
                    "id": "assign",
                    "action": "assign_to_agent",
                    "agent": "Lep",
                    "instruction": "please investigate"
                }
            ]),
        )
        .await;

        let def: WorkflowDef = serde_json::from_str(&def_json).expect("parse def");

        let result = execute_run(&engine, community, run_id, &def, &trigger_ctx)
            .await
            .expect("execute_run should suspend, not error");

        assert_eq!(result.trace.len(), 3, "one trace entry per step");

        let completed = &result.trace[0];
        assert_eq!(completed["step_id"], "notify");
        assert_eq!(completed["status"], "completed");
        assert!(
            completed["started_at"].as_i64().is_some(),
            "completed entry must have started_at: {completed}"
        );
        assert!(
            completed["completed_at"].as_i64().is_some(),
            "completed entry must have completed_at: {completed}"
        );

        let skipped = &result.trace[1];
        assert_eq!(skipped["step_id"], "maybe-skip");
        assert_eq!(skipped["status"], "skipped");
        assert!(
            skipped["started_at"].as_i64().is_some(),
            "skipped entry must have started_at: {skipped}"
        );
        assert!(
            skipped["completed_at"].as_i64().is_some(),
            "skipped entry must have completed_at: {skipped}"
        );

        let waiting = &result.trace[2];
        assert_eq!(waiting["step_id"], "assign");
        assert_eq!(waiting["status"], "waiting");
        assert!(
            waiting["started_at"].as_i64().is_some(),
            "waiting entry must have started_at: {waiting}"
        );
        assert!(
            waiting["completed_at"].is_null(),
            "waiting entry's completed_at must be null until resume: {waiting}"
        );

        assert!(
            result.suspend.is_some(),
            "run must suspend at the AssignToAgent step"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn trace_entry_carries_timestamps_and_failed_status_on_dispatch_error() {
        // Unresolved agent name → dispatch_action returns Err — execute_run
        // must surface a `failed` trace entry with both timestamps, not an
        // entry missing started_at/completed_at.
        let sink = MockAgentSink {
            agents: HashMap::new(),
            sent: Mutex::new(Vec::new()),
            members: HashMap::new(),
        };
        let (engine, db, community, channel_id) = setup_engine_with_sink(sink).await;

        let (run_id, trigger_ctx, def_json) = make_assign_run(
            &db,
            community,
            channel_id,
            serde_json::json!([{
                "id": "assign",
                "action": "assign_to_agent",
                "agent": "Nobody",
                "instruction": "please investigate"
            }]),
        )
        .await;

        let def: WorkflowDef = serde_json::from_str(&def_json).expect("parse def");

        let (_err, progress) = execute_run(&engine, community, run_id, &def, &trigger_ctx)
            .await
            .expect_err("unresolved agent must fail the run");

        assert_eq!(progress.trace.len(), 1);
        let failed = &progress.trace[0];
        assert_eq!(failed["step_id"], "assign");
        assert_eq!(failed["status"], "failed");
        assert!(
            failed["error"]
                .as_str()
                .is_some_and(|s| s.contains("Nobody")),
            "failed entry should carry the dispatch error: {failed}"
        );
        assert!(
            failed["started_at"].as_i64().is_some(),
            "failed entry must have started_at: {failed}"
        );
        assert!(
            failed["completed_at"].as_i64().is_some(),
            "failed entry must have completed_at: {failed}"
        );
    }
}
