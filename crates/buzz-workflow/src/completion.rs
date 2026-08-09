//! Agent completion-block parser.
//!
//! An `AssignToAgent` step suspends the run and @mentions an agent. The
//! agent's reply is expected to contain a fenced ```` ```completion ```` YAML
//! block describing the outcome:
//!
//! ````text
//! ```completion
//! status: success
//! outputs:
//!   summary: "done"
//! reason: null
//! usage:
//!   input_tokens: 120
//!   output_tokens: 45
//!   cost: 0.002
//! ```
//! ````
//!
//! Parsing never fails outward: a missing or malformed block resumes the
//! workflow with `status: failed` rather than leaving the run stuck forever
//! (see [`parse`]).

use std::collections::HashMap;

use serde::Deserialize;

/// Outcome reported by an agent's completion block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionStatus {
    /// The agent completed its assignment successfully.
    Success,
    /// The agent could not complete its assignment.
    Failed,
}

/// Self-reported token usage and cost for the agent's turn.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub struct UsageInfo {
    /// Input (prompt) tokens consumed.
    #[serde(default)]
    pub input_tokens: u64,
    /// Output (completion) tokens produced.
    #[serde(default)]
    pub output_tokens: u64,
    /// Estimated cost in USD, if the agent reported one.
    #[serde(default)]
    pub cost: Option<f64>,
}

/// Parsed result of an agent's completion reply.
///
/// Always produced — see [`parse`] for the fallback behavior on a
/// missing/malformed block.
#[derive(Debug, Clone)]
pub struct AgentCompletion {
    /// Whether the agent reports success or failure.
    pub status: CompletionStatus,
    /// Structured outputs, made available downstream as
    /// `{{steps.<id>.output.X}}`.
    pub outputs: HashMap<String, serde_json::Value>,
    /// Optional human-readable explanation (required-ish for `failed`, but
    /// not enforced — absent reason on failure just means no explanation).
    pub reason: Option<String>,
    /// Optional self-reported usage/cost.
    pub usage: Option<UsageInfo>,
}

/// Maximum length (bytes) of raw message content retained as the `reason`
/// fallback when no valid completion block is found. Prevents an
/// adversarial or runaway agent reply from bloating the stored step output.
pub const MAX_FALLBACK_REASON_LEN: usize = 2000;

/// Raw shape of a `completion` YAML block, deserialized before being
/// converted into the public [`AgentCompletion`].
#[derive(Debug, Deserialize)]
struct RawCompletion {
    status: CompletionStatus,
    #[serde(default)]
    outputs: HashMap<String, serde_json::Value>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    usage: Option<UsageInfo>,
}

/// Extract the contents of the first ` ```completion ` ... ` ``` ` fenced
/// block in `content`, if any.
fn extract_completion_block(content: &str) -> Option<&str> {
    const FENCE_OPEN: &str = "```completion";
    let start = content.find(FENCE_OPEN)?;
    let after_open = start + FENCE_OPEN.len();
    // Skip the rest of the opening fence line (allows trailing whitespace
    // after the language tag before the newline).
    let body_start = content[after_open..]
        .find('\n')
        .map(|i| after_open + i + 1)?;
    let close_offset = content[body_start..].find("```")?;
    Some(&content[body_start..body_start + close_offset])
}

/// True when `content` contains a complete ```` ```completion ```` fenced
/// block that [`parse`] would extract (used by the relay to distinguish a
/// block-bearing reply from a plain-text harness completion).
pub fn has_completion_block(content: &str) -> bool {
    extract_completion_block(content).is_some()
}

/// Parse an agent's reply content into an [`AgentCompletion`].
///
/// This function never fails outward. Per design: a missing or malformed
/// completion block must never block the workflow resume path — it instead
/// falls back to `status: failed` with `reason` set to a (possibly
/// truncated) copy of the raw message content, so the run always has a way
/// to proceed instead of getting stuck waiting forever.
pub fn parse(content: &str) -> AgentCompletion {
    let Some(block) = extract_completion_block(content) else {
        return fallback(content);
    };

    match serde_yaml::from_str::<RawCompletion>(block) {
        Ok(raw) => AgentCompletion {
            status: raw.status,
            outputs: raw.outputs,
            reason: raw.reason,
            usage: raw.usage,
        },
        Err(_) => fallback(content),
    }
}

/// Build the fallback `AgentCompletion` for a missing/malformed block.
fn fallback(content: &str) -> AgentCompletion {
    let reason: String = content.chars().take(MAX_FALLBACK_REASON_LEN).collect();
    AgentCompletion {
        status: CompletionStatus::Failed,
        outputs: HashMap::new(),
        reason: Some(reason),
        usage: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_completion_with_all_fields() {
        let content = "Done!\n```completion\nstatus: success\noutputs:\n  summary: \"all good\"\n  count: 3\nreason: \"finished cleanly\"\nusage:\n  input_tokens: 100\n  output_tokens: 50\n  cost: 0.001\n```\n";
        let c = parse(content);
        assert_eq!(c.status, CompletionStatus::Success);
        assert_eq!(
            c.outputs.get("summary").and_then(|v| v.as_str()),
            Some("all good")
        );
        assert_eq!(c.outputs.get("count").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(c.reason.as_deref(), Some("finished cleanly"));
        let usage = c.usage.expect("usage present");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cost, Some(0.001));
    }

    #[test]
    fn well_formed_completion_with_only_required_fields() {
        let content = "```completion\nstatus: failed\n```";
        let c = parse(content);
        assert_eq!(c.status, CompletionStatus::Failed);
        assert!(c.outputs.is_empty());
        assert!(c.reason.is_none());
        assert!(c.usage.is_none());
    }

    #[test]
    fn malformed_yaml_falls_back_to_failed() {
        let content = "```completion\nstatus: [this is not valid: yaml structure\n```";
        let c = parse(content);
        assert_eq!(c.status, CompletionStatus::Failed);
        assert!(c.outputs.is_empty());
        assert!(c.reason.is_some());
    }

    #[test]
    fn no_fence_present_falls_back_to_failed() {
        let content = "Sorry, I couldn't finish this task in time.";
        let c = parse(content);
        assert_eq!(c.status, CompletionStatus::Failed);
        assert_eq!(c.reason.as_deref(), Some(content));
    }

    #[test]
    fn multiple_fenced_blocks_takes_first_completion_tagged() {
        let content = "Here's my code:\n```rust\nfn main() {}\n```\nAnd the result:\n```completion\nstatus: success\noutputs:\n  ok: true\n```\n";
        let c = parse(content);
        assert_eq!(c.status, CompletionStatus::Success);
        assert_eq!(c.outputs.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn ignores_earlier_non_completion_fence_with_completion_word_in_body() {
        // A fence tagged something else that happens to mention "completion"
        // in its body must not be picked up as the completion block.
        let content = "```yaml\n# completion notes here\nfoo: bar\n```\n```completion\nstatus: success\n```\n";
        let c = parse(content);
        assert_eq!(c.status, CompletionStatus::Success);
    }

    #[test]
    fn usage_bearing_reply_parses_cost_as_optional() {
        let content =
            "```completion\nstatus: success\nusage:\n  input_tokens: 10\n  output_tokens: 5\n```";
        let c = parse(content);
        let usage = c.usage.expect("usage present");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.cost, None);
    }

    #[test]
    fn fallback_reason_is_truncated_for_oversized_content() {
        let long = "x".repeat(MAX_FALLBACK_REASON_LEN + 500);
        let c = parse(&long);
        assert_eq!(c.status, CompletionStatus::Failed);
        assert_eq!(
            c.reason.as_ref().map(|s| s.chars().count()),
            Some(MAX_FALLBACK_REASON_LEN)
        );
    }
}
