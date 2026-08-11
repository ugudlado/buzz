//! `buzz-agent-snapshot v1` manifest types — the portable `.agent.json`
//! representation of an agent definition, shared between Buzz Desktop
//! (encoder/decoder, including the `.agent.png` variant) and `buzz-cli`
//! (`agents import`, JSON-only).
//!
//! See `desktop/src-tauri/src/managed_agents/agent_snapshot.rs` for the full
//! format doc (secret-field exclusion list, PNG embedding) — that module
//! re-exports these types and owns everything encoding-specific.

use serde::{Deserialize, Serialize};

/// Format discriminator — used for sniffing and validation.
pub const FORMAT_DISCRIMINATOR: &str = "buzz-agent-snapshot";

/// Version of the manifest format produced by this module.
pub const FORMAT_VERSION: u32 = 1;

/// How much memory to bundle in the snapshot.
///
/// The default is `None` — config-only export, safest for sharing. Memory
/// entries are plaintext in the output file; users must opt in explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLevel {
    /// Export definition + profile only. No memory. (Default)
    #[default]
    None,
    /// Export definition + profile + `core` memory only.
    Core,
    /// Export definition + profile + `core` + all `mem/*` entries.
    Everything,
}

/// Behavioral definition — what makes the agent do what it does.
///
/// Fields mirror `ManagedAgentRecord` definition-level fields. Only the subset
/// meaningful across environments is included; machine-local / secret fields
/// are deliberately absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotDefinition {
    pub name: String,
    /// Portable source classification for import-preview metadata. Imported
    /// definitions are still created as custom agents with fresh identities.
    #[serde(default)]
    pub source_is_builtin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    /// Allowlist entries. These are flagged during import — they come from the
    /// source environment and are meaningless on the importer's relay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub respond_to_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turn_duration_seconds: Option<u64>,
}

/// kind:0 presentation fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotProfile {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Avatar inlined as a `data:image/...;base64,…` URI (≤ 2 MB),
    /// or a URL fallback if the image exceeds the size limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_data_url: Option<String>,
    /// Present when the avatar exceeds the inline size limit and is stored
    /// by reference rather than inlined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// A single decrypted memory entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotMemoryEntry {
    pub slug: String,
    pub body: String,
}

/// Memory section of the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotMemory {
    /// Indicates what was included at export time.
    pub level: MemoryLevel,
    /// Decrypted memory entries. Empty when `level == None`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<AgentSnapshotMemoryEntry>,
}

/// The top-level `buzz-agent-snapshot v1` manifest.
///
/// Serializes to / from JSON. Embedded in `.agent.json` directly, or (desktop
/// only) in the `buzz_agent_snapshot` tEXt chunk of a `.agent.png`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshot {
    /// Fixed discriminator for format sniffing.
    pub format: String,
    /// Schema version. This module produces version 1.
    pub version: u32,
    pub definition: AgentSnapshotDefinition,
    pub profile: AgentSnapshotProfile,
    pub memory: AgentSnapshotMemory,
}
