//! `ManagedAgentRecord` and its serde-only supporting types, shared between
//! Buzz Desktop (which owns `managed-agents.json`) and `buzz-cli` (which can
//! append to that store directly via `agents import`). Desktop-internal
//! concerns — process handles, persona-definition folding, keyring I/O,
//! relay publishing — stay in `desktop/src-tauri`; this crate is the pure
//! data shape both sides serialize.

use buzz_core::marketplace::AgentMarketplace;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const DEFAULT_ACP_COMMAND: &str = "buzz-acp";
/// ~5 min (320s) — matches the CLI harness default (BUZZ_ACP_IDLE_TIMEOUT).
pub const DEFAULT_AGENT_TURN_TIMEOUT_SECONDS: u64 = 320;
pub const DEFAULT_AGENT_PARALLELISM: u32 = 10;

pub fn default_agent_parallelism() -> u32 {
    DEFAULT_AGENT_PARALLELISM
}

pub fn default_start_on_app_launch() -> bool {
    true
}

pub fn default_auto_restart_on_config_change() -> bool {
    true
}

pub fn default_record_active() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackendKind {
    #[default]
    Local,
    Provider {
        id: String,
        config: serde_json::Value,
    },
}

// ── Inbound author gate ──────────────────────────────────────────────────────
//
// Mirrors `buzz-acp`'s `--respond-to` CLI flag and the related
// `--respond-to-allowlist` option. Persisted per agent so the desktop can
// translate the user's choice into `BUZZ_ACP_RESPOND_TO` /
// `BUZZ_ACP_RESPOND_TO_ALLOWLIST` env vars at spawn time.
//
// Wire format is kebab-case (`owner-only`, `allowlist`, `anyone`) to match
// the harness CLI vocabulary and the strings the GUI emits.
//
// `nobody` is intentionally NOT exposed here. The harness supports it, but
// it's a heartbeat-only mode and the desktop has no surface for it.

/// Who the agent should respond to. Defaults to `OwnerOnly`, which matches
/// the harness default → existing agents behave identically.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RespondTo {
    #[default]
    OwnerOnly,
    Allowlist,
    Anyone,
}

impl RespondTo {
    /// CLI/env wire string (matches `buzz-acp`'s `--respond-to`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OwnerOnly => "owner-only",
            Self::Allowlist => "allowlist",
            Self::Anyone => "anyone",
        }
    }

    /// Parse the NIP-AP wire string. Definitions carry `respond_to` as
    /// opaque data everywhere else; this is the single parse boundary
    /// (instance mint), and an unrecognized mode fails LOUDLY here rather
    /// than silently defaulting — a typo'd definition must not mint an
    /// agent with a different audience than its author intended.
    pub fn parse_wire(value: &str) -> Result<Self, String> {
        match value {
            "owner-only" => Ok(Self::OwnerOnly),
            "allowlist" => Ok(Self::Allowlist),
            "anyone" => Ok(Self::Anyone),
            other => Err(format!(
                "definition respond_to '{other}' is not a recognized mode (expected 'owner-only', 'allowlist', or 'anyone')"
            )),
        }
    }
}

/// Validate and normalize a respond-to allowlist.
///
/// Rules mirror `buzz-acp/src/config.rs::validate_allowlist`:
/// - Each entry is exactly 64 hex chars (any case in, lowercase out).
/// - Duplicates removed, insertion order preserved.
///
/// Empty input is allowed here — the boundary check (allowlist mode requires
/// at least one entry) is the caller's job, because an `UpdateManagedAgentRequest`
/// may want to validate a list without yet knowing the final mode.
pub fn validate_respond_to_allowlist(input: &[String]) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(input.len());
    for entry in input {
        let trimmed = entry.trim();
        if trimmed.len() != 64 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "invalid pubkey in respond-to allowlist: '{trimmed}' (must be 64 hex chars)"
            ));
        }
        let lower = trimmed.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            out.push(lower);
        }
    }
    Ok(out)
}

/// Where a persona copy came from in another owner's shared catalog.
///
/// The pair is the publication's NIP-AP coordinate minus the kind: the owner
/// who published it and the `d`-tag identifying the persona within that
/// owner's catalog. A copy carries a fresh local `id`, so this pair is the
/// only thing that can answer "is this catalog entry already added".
///
/// Field casing follows [`RelayMeshConfig`]: persisted records use snake_case
/// and the camelCase `alias`es accept the create payload the frontend sends
/// (`rename_all` on the request does not recurse into nested structs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogSource {
    #[serde(alias = "ownerPubkey")]
    pub owner_pubkey: String,
    #[serde(alias = "personaId")]
    pub persona_id: String,
}

impl CatalogSource {
    /// Normalize a coordinate arriving from the frontend.
    ///
    /// "Already added" is decided by comparing this pair against a
    /// publication's author and `d`-tag, so an un-normalized value silently
    /// fails to match and mints another copy — the exact duplicate the field
    /// exists to prevent. Owner pubkey: 64 hex, any case in, lowercase out
    /// (same contract as [`validate_respond_to_allowlist`]). Persona id: the
    /// publication's `d`-tag, trimmed and required.
    pub fn normalized(self) -> Result<Self, String> {
        let owner_pubkey = self.owner_pubkey.trim().to_ascii_lowercase();
        if owner_pubkey.len() != 64 || !owner_pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "invalid catalog source owner pubkey: '{owner_pubkey}' (must be 64 hex chars)"
            ));
        }
        let persona_id = self.persona_id.trim().to_string();
        if persona_id.is_empty() {
            return Err("catalog source persona id is required".to_string());
        }
        Ok(Self {
            owner_pubkey,
            persona_id,
        })
    }
}

/// Typed relay-mesh configuration carried on a [`ManagedAgentRecord`].
///
/// Feature-independent on purpose: the field is always present in the record
/// schema so saved agents round-trip identically whether or not the `mesh-llm`
/// feature is compiled in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayMeshConfig {
    /// The served model id this agent routes to (e.g. "Qwen3").
    ///
    /// `alias` because this struct crosses two boundaries with different
    /// casing conventions: the TS create request sends camelCase
    /// (`relayMesh: { modelRef }` — `rename_all` on the request does not
    /// recurse into nested structs), while persisted records use snake_case.
    /// Serialization stays `model_ref` so saved records are stable.
    #[serde(alias = "modelRef")]
    pub model_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManagedAgentRecord {
    pub pubkey: String,
    pub name: String,
    #[serde(default)]
    pub persona_id: Option<String>,
    /// Team this instance was deployed from. Resolves runtime team instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// nsec private key. Held in memory but persisted to the OS keyring (keyed
    /// by `pubkey`) rather than serialized to `managed-agents.json`. The
    /// storage layer blanks this before writing JSON once the key is safely in
    /// the keyring, and re-hydrates it from the keyring on load.
    ///
    /// It is only serialized inline (the `0o600` JSON fallback) when the
    /// keyring is unreachable — `skip_serializing_if` keeps it out of JSON in
    /// the normal keyring-backed case. `default` also lets an old build parse a
    /// store whose inline key was already migrated out and blanked.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub private_key_nsec: String,
    /// NIP-OA auth tag JSON. Computed at agent creation time.
    ///
    /// Pre-existing agents created before NIP-OA will have `None` here.
    /// This is intentional — they continue to work without attestation.
    /// Re-attestation requires agent recreation (v2 migration scope).
    #[serde(default)]
    pub auth_tag: Option<String>,
    pub relay_url: String,
    /// Avatar URL resolved at creation time (user-supplied input, else the
    /// command-based fallback). Persisted so startup reconciliation compares
    /// against what was actually published rather than re-deriving it from
    /// persona config — which would silently overwrite user intent on restart.
    /// `#[serde(default)]` so pre-existing records deserialize as `None`.
    #[serde(default)]
    pub avatar_url: Option<String>,
    pub acp_command: String,
    pub agent_command: String,
    /// Explicit per-instance harness pin. `None` (the default) means inherit
    /// the harness from the linked persona's `runtime`, so persona harness
    /// edits propagate on the next spawn — mirroring the opt-in `model`
    /// override. `Some` is set only when the user deliberately picks a harness
    /// that diverges from the persona. Resolved via `effective_agent_command`;
    /// `agent_command` above is the create-time snapshot kept for avatar/legacy
    /// derivations and is not authoritative for spawn.
    #[serde(default)]
    pub agent_command_override: Option<String>,
    pub agent_args: Vec<String>,
    /// Create-time snapshot of the catalog MCP command. Never read at spawn —
    /// the effective MCP command is always re-derived from the runtime catalog
    /// (`known_acp_runtime`) — and no longer written by updates. Kept for
    /// serde compatibility with existing stores.
    pub mcp_command: String,
    /// Deprecated: `BUZZ_ACP_TURN_TIMEOUT` is ignored by the harness and the
    /// desktop no longer emits or edits it. Kept for serde compatibility with
    /// existing stores; use `idle_timeout_seconds` or
    /// `max_turn_duration_seconds` for turn-length control.
    pub turn_timeout_seconds: u64,
    /// Idle timeout in seconds (`BUZZ_ACP_IDLE_TIMEOUT`): how long the agent
    /// may stay silent on its ACP channel mid-turn before the harness times
    /// the turn out.
    #[serde(default)]
    pub idle_timeout_seconds: Option<u64>,
    /// Absolute wall-clock cap per turn.
    #[serde(default)]
    pub max_turn_duration_seconds: Option<u64>,
    #[serde(default = "default_agent_parallelism")]
    pub parallelism: u32,
    pub system_prompt: Option<String>,
    /// Desired LLM model ID. Matches AgentModelInfo.id from discovery.
    /// The harness re-discovers the correct ACP switching metadata at session
    /// creation by matching this ID against the fresh session/new response.
    /// For a linked instance this is a legacy/display snapshot only — spawn
    /// and deploy resolve the effective model from the definition, never
    /// from this field (see `effective_config::resolve_effective_config`).
    /// For a definition-less instance this field is authoritative.
    #[serde(default)]
    pub model: Option<String>,
    /// LLM inference provider. For a linked instance this is a legacy/display
    /// snapshot only — spawn and deploy resolve the effective provider from
    /// the definition, never from this field (see
    /// `effective_config::resolve_effective_config`). For a definition-less
    /// instance this field is authoritative. `#[serde(default)]` so
    /// pre-existing records deserialize as `None` and get backfilled on
    /// first load.
    #[serde(default)]
    pub provider: Option<String>,
    /// Content hash of the persona at the time this agent was created — the
    /// `persona_content_hash` of the snapshot in `system_prompt` / `model` /
    /// `provider` / `env_vars`. The Agents menu compares it against the linked
    /// persona's current hash to flag a stale (out-of-date) instance. `None`
    /// for non-persona agents and for pre-existing records pending backfill.
    #[serde(default)]
    pub persona_source_version: Option<String>,
    /// Environment variables injected at spawn time. Layered as: desktop
    /// parent env < persona `env_vars` < this agent's `env_vars` (last wins).
    ///
    /// To "override" a persona env var: set the same key here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default = "default_start_on_app_launch")]
    pub start_on_app_launch: bool,
    /// Auto-restart this agent when its effective spawn config drifts from
    /// the running process (Chunk F). Default ON; the policy loop in the
    /// frontend only fires when the agent is idle, connected, and local.
    #[serde(default = "default_auto_restart_on_config_change")]
    pub auto_restart_on_config_change: bool,
    #[serde(default)]
    pub runtime_pid: Option<u32>,
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(default)]
    pub backend_agent_id: Option<String>,
    #[serde(default)]
    pub provider_binary_path: Option<String>,
    /// Installed team directory path (absolute). Set when agent was created from a team persona.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "persona_pack_path"
    )]
    pub persona_team_dir: Option<PathBuf>,
    /// Persona name within the team.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "persona_name_in_pack"
    )]
    pub persona_name_in_team: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_started_at: Option<String>,
    pub last_stopped_at: Option<String>,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_error_code: Option<i64>,
    /// Inbound author gate mode. Translates to `BUZZ_ACP_RESPOND_TO`.
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Allowlist used when `respond_to == Allowlist`. Stored normalized
    /// (64-char lowercase hex, deduped). Empty when mode is not Allowlist.
    /// Preserved across mode toggles so users don't lose state.
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
    /// Optional public marketplace metadata. This is deliberately separate
    /// from runtime/backend configuration and is safe to project publicly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<AgentMarketplace>,
    /// Optional display name distinct from the unique `name` handle. Absorbed
    /// from `AgentDefinition.display_name` (unified agent model, Phase 1A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Stable definition slug — the former `AgentDefinition.id`. Key-less
    /// records (definitions not yet instantiated) publish kind:30175 at
    /// `d_tag = slug`, preserving the pre-merge event coordinates. `None` for
    /// agents created directly (never persona-backed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Absorbed from `AgentDefinition.runtime` — the preferred ACP runtime ID
    /// (e.g. 'goose', 'claude'). Record-first command resolution reads this
    /// before falling back to legacy persona lookup; populated by the store
    /// migration and at create time, and re-mirrored from the linked
    /// definition at every snapshot apply (`apply_persona_snapshot`).
    ///
    /// `None` means "inherit from the linked definition" (the Inherit sentinel
    /// clears it). Serialization then omits the key, so boot-time
    /// `materialize_agent_runtimes` re-inserts a mirror of the definition's
    /// current runtime on the next launch — behaviorally identical, because
    /// every apply site re-mirrors the live definition anyway. A literal
    /// `"runtime": null` in the store (key present, e.g. hand-edited) is
    /// honored: materialization skips it and it deserializes to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Pool of short thematic names for clones of this agent. Absorbed from
    /// `AgentDefinition.name_pool`; feeds clone naming.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    /// Absorbed from `AgentDefinition.is_builtin`.
    #[serde(default)]
    pub is_builtin: bool,
    /// Absorbed from `AgentDefinition.is_active` — `false` means an archived
    /// definition hidden from pickers. Defaults `true` for existing records.
    #[serde(default = "default_record_active")]
    pub is_active: bool,
    /// Legacy process-global catalog visibility field.
    ///
    /// New writes omit it and definition views ignore it. It remains
    /// deserializable for branch-era stores, but active visibility is projected
    /// from the relay+owner-scoped retention database instead.
    #[serde(default, skip_serializing)]
    pub shared: bool,
    /// Absorbed from `AgentDefinition.source_team` — team ID when this
    /// definition was imported from a team directory (team definitions are
    /// non-editable). Distinct from `persona_team_dir`/`persona_name_in_team`,
    /// which are the instance-side spawn plumbing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_team: Option<String>,
    /// Absorbed from `AgentDefinition.source_team_persona_slug` — the
    /// definition's slug within its source team.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_team_persona_slug: Option<String>,
    /// Absorbed from `AgentDefinition.catalog_source` — the publication this
    /// definition was copied from, when it came from another owner's catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_source: Option<CatalogSource>,
    /// NIP-AP definition-level behavioral defaults, absorbed from
    /// `AgentDefinition` in WIRE shape (kebab-case string / optional u32),
    /// distinct from the instance-side `respond_to`/`respond_to_allowlist`/
    /// `parallelism` fields above: these are what a *definition* advertises
    /// and are copied onto instances at mint time only. Wire shape (not the
    /// `RespondTo` enum) so absent-ness and unknown future mode strings
    /// round-trip byte-identically through the store — parsed/validated
    /// solely at the mint boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_respond_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub definition_respond_to_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_parallelism: Option<u32>,
    /// Typed marker for relay-mesh agents. `Some(_)` means this agent runs its
    /// inference through Buzz's relay-mesh local endpoint; the `model_ref` is
    /// the served model id to route to. `None` is a normal agent.
    ///
    /// Not the source of truth. `provider == "relay-mesh"` is, resolved through
    /// `effective_config::resolve_effective_config`; spawn-time env vars are
    /// derived from that resolution. This field is retained solely as a
    /// backward-compatibility signal for records written before the record had
    /// a `provider` field, and is consulted only for definition-less records
    /// that carry no provider — after which the env-var preset is the last
    /// fallback. A linked instance's marker is never read: its definition is
    /// authoritative. `#[serde(default)]` so records predating the field
    /// deserialize as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_mesh: Option<RelayMeshConfig>,
}

#[cfg(test)]
mod tests {
    use super::CatalogSource;

    fn source(owner_pubkey: &str, persona_id: &str) -> CatalogSource {
        CatalogSource {
            owner_pubkey: owner_pubkey.to_string(),
            persona_id: persona_id.to_string(),
        }
    }

    #[test]
    fn normalized_lowercases_and_trims_the_owner_pubkey() {
        // "Already added" compares this against a publication's author hex, which
        // is always lowercase — a mixed-case value from the UI must not miss.
        let normalized = source(&format!("  {}  ", "A".repeat(64)), " helper ")
            .normalized()
            .expect("64 hex chars with surrounding space is valid");
        assert_eq!(normalized.owner_pubkey, "a".repeat(64));
        assert_eq!(normalized.persona_id, "helper");
    }

    #[test]
    fn normalized_rejects_a_short_owner_pubkey() {
        let err = source("abc123", "helper").normalized().unwrap_err();
        assert!(err.contains("64 hex"), "error must name the rule: {err}");
    }

    #[test]
    fn normalized_rejects_a_non_hex_owner_pubkey() {
        let err = source(&"z".repeat(64), "helper").normalized().unwrap_err();
        assert!(err.contains("64 hex"), "error must name the rule: {err}");
    }

    #[test]
    fn normalized_rejects_a_blank_persona_id() {
        let err = source(&"a".repeat(64), "   ").normalized().unwrap_err();
        assert!(
            err.contains("persona id"),
            "error must name the field: {err}"
        );
    }

    #[test]
    fn deserializes_the_camel_case_payload_the_frontend_sends() {
        // `rename_all` on CreatePersonaRequest does not recurse into this struct,
        // so without the aliases the copy request fails at the Tauri boundary.
        let parsed: CatalogSource =
            serde_json::from_str(r#"{"ownerPubkey":"abc","personaId":"helper"}"#)
                .expect("camelCase payload from TS should deserialize");
        assert_eq!(parsed, source("abc", "helper"));
    }

    #[test]
    fn round_trips_persisted_snake_case() {
        let value = source(&"a".repeat(64), "helper");
        let json = serde_json::to_string(&value).unwrap();
        assert!(json.contains("owner_pubkey"), "persisted shape: {json}");
        assert_eq!(
            serde_json::from_str::<CatalogSource>(&json).unwrap(),
            value,
            "the camelCase alias must not break the stored-record round trip"
        );
    }
}
