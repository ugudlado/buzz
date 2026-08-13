use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    Info,
    Deploy(Box<DeployRequest>),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeployRequest {
    pub agent: AgentPayload,
    #[serde(default)]
    pub provider_config: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentPayload {
    pub relay_url: String,
    pub private_key_nsec: String,
    #[serde(default)]
    pub auth_tag: Option<String>,
    #[serde(default)]
    pub respond_to: Option<String>,
    #[serde(default)]
    pub respond_to_allowlist: Option<Vec<String>>,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default)]
    pub launch: Option<LaunchBlock>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct LaunchBlock {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub policy_env: BTreeMap<String, String>,
    #[serde(default)]
    pub owner_pubkey: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Info(InfoResponse),
    Deploy(DeployResponse),
    Error(ErrorResponse),
}

#[derive(Debug, Serialize)]
pub struct InfoResponse {
    pub ok: bool,
    pub name: &'static str,
    pub version: &'static str,
    pub protocol_version: u32,
    pub description: &'static str,
    pub config_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct DeployResponse {
    pub ok: bool,
    pub agent_id: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub ok: bool,
    pub error: String,
}

impl Response {
    pub fn info() -> Self {
        Self::Info(InfoResponse {
            ok: true,
            name: "host",
            version: env!("CARGO_PKG_VERSION"),
            protocol_version: PROTOCOL_VERSION,
            description: "Remote setup required: install the matching Buzz host provider, ACP harness, developer MCP, Buzz CLI, Git 2.46+, git-credential-nostr, and the selected runtime before adding this agent. See https://github.com/block/buzz/blob/main/docs/host-agents.md.",
            config_schema: crate::config::schema(),
        })
    }

    pub fn deployed(agent_id: impl Into<String>) -> Self {
        Self::Deploy(DeployResponse {
            ok: true,
            agent_id: agent_id.into(),
        })
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self::Error(ErrorResponse {
            ok: false,
            error: error.into(),
        })
    }
}
