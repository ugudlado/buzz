//! Shared data shapes for Buzz's managed-agent store (`managed-agents.json`)
//! and the `buzz-agent-snapshot v1` (`.agent.json`) portable format.
//!
//! Exists so `buzz-cli` can construct and serialize a real
//! [`record::ManagedAgentRecord`] instead of hand-rolling JSON that drifts
//! from the desktop app's definition. Desktop-internal concerns (process
//! handles, persona-definition folding, keyring I/O, relay publishing, PNG
//! snapshot encoding) stay in `desktop/src-tauri`.

pub mod record;
pub mod snapshot;

pub use record::{
    default_agent_parallelism, default_auto_restart_on_config_change, default_record_active,
    default_start_on_app_launch, validate_respond_to_allowlist, BackendKind, CatalogSource,
    ManagedAgentRecord, RelayMeshConfig, RespondTo, DEFAULT_ACP_COMMAND, DEFAULT_AGENT_PARALLELISM,
    DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
};
pub use snapshot::{
    AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory, AgentSnapshotMemoryEntry,
    AgentSnapshotProfile, MemoryLevel, FORMAT_DISCRIMINATOR, FORMAT_VERSION,
};
