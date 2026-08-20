//! Action sink trait — interface for workflow side-effects.
//!
//! The relay implements [`ActionSink`] to provide direct DB access to the
//! executor, replacing the HTTP loopback pattern.

use std::future::Future;
use std::pin::Pin;

use buzz_core::tenant::CommunityId;

/// Errors from action sink operations.
#[derive(Debug, thiserror::Error)]
pub enum ActionSinkError {
    /// An input parameter is malformed (e.g. invalid UUID).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The target channel does not exist.
    #[error("channel not found: {0}")]
    ChannelNotFound(String),
    /// The target channel is archived.
    #[error("channel is archived: {0}")]
    ChannelArchived(String),
    /// Nostr event construction or signing failed.
    #[error("event construction failed: {0}")]
    EventBuild(String),
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(String),
    /// Message content is empty or whitespace-only.
    #[error("empty message content")]
    EmptyContent,
}

impl From<ActionSinkError> for crate::WorkflowError {
    fn from(e: ActionSinkError) -> Self {
        crate::WorkflowError::WebhookError(e.to_string())
    }
}

/// Boxed future returned by [`ActionSink::resolve_agent`].
type ResolveAgentFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, ActionSinkError>> + Send + 'a>>;

/// Marketplace attribution snapshotted when an assignment is dispatched.
#[derive(Debug, Clone)]
pub struct AgentMarketplaceSnapshot {
    /// Verified owner of the managed-agent identity.
    pub owner_pubkey: Vec<u8>,
    /// Whether the current owner has an active marketplace listing.
    pub listed: bool,
    /// Three-letter currency code, absent for an unpriced listing.
    pub rate_currency: Option<String>,
    /// Integer micro-units per hour, absent for an unpriced listing.
    pub rate_microunits_per_hour: Option<u64>,
    /// Exact managed-agent listing used for a remote dispatch.
    pub listing_event_id: Option<String>,
}

/// Immutable home-relay coordinate for a remote managed agent.
#[derive(Debug, Clone)]
pub struct AgentRelayCoordinate {
    /// NIP-11 `self` pubkey of the agent's home relay.
    pub relay_pubkey: Vec<u8>,
    /// Verified absolute `wss://` URL of the agent's home relay.
    pub relay_url: String,
}

/// Extra wire data needed when an assignment leaves this community.
#[derive(Debug, Clone)]
pub struct RemoteAgentAssignment {
    /// Agent home-relay coordinate.
    pub coordinate: AgentRelayCoordinate,
    /// Exact listing accepted at dispatch.
    pub listing_event_id: String,
    /// Plaintext instruction that will be encrypted for the target agent.
    pub instruction: String,
}

/// Durable assignment data armed immediately before its prompt is published.
#[derive(Debug, Clone)]
pub struct AgentAssignmentArm {
    /// Workflow containing the assignment.
    pub workflow_id: uuid::Uuid,
    /// Run waiting on the assignment.
    pub run_id: uuid::Uuid,
    /// Stable step identifier.
    pub step_id: String,
    /// Zero-based step index.
    pub step_index: i32,
    /// Assigned managed-agent identity.
    pub agent_pubkey: Vec<u8>,
    /// Verified owner at dispatch.
    pub agent_owner_pubkey: Vec<u8>,
    /// Snapshotted rate currency.
    pub rate_currency: Option<String>,
    /// Snapshotted integer micro-units per hour.
    pub rate_microunits_per_hour: Option<u64>,
    /// Assignment timeout in seconds.
    pub timeout_secs: u64,
    /// Trace entries completed before this assignment began.
    pub trace_prefix: Vec<serde_json::Value>,
    /// Unix-second timestamp captured when this step began.
    pub step_started_at: i64,
    /// Present only for a cross-community job request.
    pub remote: Option<RemoteAgentAssignment>,
}

/// The NIP-10 thread root a new `assign_to_agent` prompt should reply to, so
/// a multi-step run reads as one flat conversation (every prompt/announce
/// replies directly to the root, depth 1) instead of independent top-level
/// (prompt, reply) pairs per step.
///
/// Both fields are known to the caller without a DB round-trip: the executor
/// carries them forward in the run's own trace/step-output state (set by the
/// run's first `assign_to_agent` prompt).
#[derive(Debug, Clone)]
pub struct ThreadAnchor {
    /// Hex event id of the run's first `assign_to_agent` prompt (thread root).
    pub root_event_id: String,
    /// Unix seconds `created_at` of the root event.
    pub root_created_at: i64,
}

/// Interface for workflow actions that produce side effects.
///
/// Implemented by the relay to provide direct DB/event access to the executor.
/// This replaces the HTTP loopback where the executor POSTed to the relay's
/// REST API (which failed with 401 auth errors).
///
/// Returns `Pin<Box<dyn Future>>` for dyn-compatibility — required because
/// `WorkflowEngine` stores `Arc<dyn ActionSink>`.
pub trait ActionSink: Send + Sync {
    /// Post a message to a channel on behalf of a workflow owner.
    ///
    /// - `community_id`: the server-resolved community that owns the workflow
    ///   run driving this side effect. The relay-signed message is published
    ///   under *this* community, never the deployment/default tenant — the run
    ///   carries its owning community so a workflow in community B posts into B
    ///   even though the side effect has no inbound connection to bind.
    /// - `channel_id`: UUID string of the target channel
    /// - `text`: message body (must not be empty/whitespace-only)
    /// - `author_pubkey`: hex-encoded pubkey of the workflow owner (used for
    ///   the `p` attribution tag; the relay keypair signs the event)
    /// - `reply_to`: when `Some(anchor)`, the message is published as a
    ///   NIP-10 threaded reply (`e` tags with `root`/`reply` markers)
    ///   instead of a top-level message. Used to chain a multi-step
    ///   `assign_to_agent` run into one conversation. `None` posts
    ///   top-level, as before.
    ///
    /// Returns the event ID hex string on success.
    fn send_message(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        text: &str,
        author_pubkey: &str,
        reply_to: Option<&ThreadAnchor>,
        assignment: Option<AgentAssignmentArm>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;

    /// Resolve a display name to the hex pubkey of exactly one active member
    /// of `channel_id`.
    ///
    /// Used by `AssignToAgent` to validate the target agent *before* sending
    /// the assignment message — `send_message`'s own `@Name` mention
    /// resolution silently omits the `p` tag for unknown or ambiguous names
    /// (see `resolve_mention_pubkeys`), which would otherwise let an
    /// unresolvable agent name "succeed" while waking no one. Matching is
    /// case-insensitive exact-name (no substring/fuzzy matching), mirroring
    /// `send_message`'s own mention resolver.
    ///
    /// Returns `Ok(None)` when the name matches zero or more than one member
    /// (ambiguous); callers should treat that as a step failure, not a
    /// silent no-op.
    fn resolve_agent<'a>(
        &'a self,
        community_id: CommunityId,
        channel_id: &'a str,
        name: &'a str,
    ) -> ResolveAgentFuture<'a>;

    /// Verify that `pubkey_hex` is an active member of `channel_id`.
    ///
    /// Used by `AssignToAgent` when the step definition carries an explicit
    /// `agent_pubkey` — the authoritative identity, bypassing name-based
    /// resolution entirely (so a duplicate/renamed display name can't cause
    /// ambiguity or misdirection). Returns `Ok(None)` when the pubkey is not
    /// a member of the channel; callers should treat that as a step failure,
    /// not a silent no-op — same contract as [`ActionSink::resolve_agent`].
    fn verify_agent_membership<'a>(
        &'a self,
        community_id: CommunityId,
        channel_id: &'a str,
        pubkey_hex: &'a str,
    ) -> ResolveAgentFuture<'a>;

    /// Load the current listed marketplace projection for an agent.
    ///
    /// Returns `Ok(None)` when the identity is missing or unlisted. The
    /// executor refuses new marketplace workflow dispatches in that case.
    fn agent_marketplace_snapshot<'a>(
        &'a self,
        community_id: CommunityId,
        agent_pubkey: &'a [u8],
        coordinate: Option<&'a AgentRelayCoordinate>,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<AgentMarketplaceSnapshot>, ActionSinkError>>
                + Send
                + 'a,
        >,
    >;
}
