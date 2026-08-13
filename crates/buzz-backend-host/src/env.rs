use crate::wire::{AgentPayload, LaunchBlock};
use std::collections::BTreeMap;

const START_NONCE: &str = "BUZZ_MANAGED_AGENT_START_NONCE";
const AUTHORITATIVE: &[&str] = &[
    "BUZZ_RELAY_URL",
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_AUTH_TAG",
    "BUZZ_ACP_AGENT_OWNER",
    "BUZZ_ACP_AGENT_COMMAND",
    "BUZZ_ACP_AGENT_ARGS",
    "BUZZ_ACP_RESPOND_TO",
    "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
    "BUZZ_ACP_MCP_COMMAND",
    "BUZZ_ACP_EXIT_AFTER_INACTIVITY",
    "BUZZ_ACP_REPOS_DIR",
    "BUZZ_ACP_GIT_COMMAND",
    "BUZZ_ACP_GIT_CREDENTIAL_HELPER",
    START_NONCE,
];

fn non_blank(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn validate_access(mode: &str, allowlist: Option<&[String]>) -> Result<(), String> {
    if !["owner-only", "allowlist", "anyone", "nobody"].contains(&mode) {
        return Err(format!("deploy refused: invalid respond_to mode {mode:?}"));
    }
    if mode == "allowlist" {
        let entries = allowlist.unwrap_or_default();
        if entries.is_empty()
            || entries.iter().any(|entry| {
                let entry = entry.trim();
                entry.len() != 64 || !entry.chars().all(|c| c.is_ascii_hexdigit())
            })
        {
            return Err("deploy refused: allowlist mode requires valid 64-hex pubkeys".into());
        }
    }
    Ok(())
}

pub struct RemoteInputs<'a> {
    pub generation: &'a str,
    pub agent_command: &'a str,
    pub mcp_command: &'a str,
    pub home: &'a str,
    pub path: &'a str,
    pub repos_dir: &'a str,
    pub git_command: &'a str,
    pub git_credential_helper: &'a str,
}

pub fn build(
    agent: &AgentPayload,
    remote: RemoteInputs<'_>,
) -> Result<BTreeMap<String, String>, String> {
    let default_launch = LaunchBlock::default();
    let launch = agent.launch.as_ref().unwrap_or(&default_launch);
    let mut env = launch.policy_env.clone();
    if agent.launch.is_some() {
        env.extend(launch.env.clone());
    } else {
        env.extend(agent.env_vars.clone());
    }

    for key in env.keys() {
        if !valid_key(key) {
            return Err(format!(
                "env key {key:?} is not a POSIX environment variable name"
            ));
        }
        if key.eq_ignore_ascii_case("BUZZ_ACP_NO_PRESENCE") {
            return Err("BUZZ_ACP_NO_PRESENCE must not be set on a remote agent".into());
        }
    }
    for key in AUTHORITATIVE {
        env.remove(*key);
    }
    // Desktop-local process paths are meaningless on the remote host. They
    // are cleared here; HOME/PATH are re-derived below from the remote user.
    for key in [
        "PATH",
        "HOME",
        "CLAUDE_CODE_EXECUTABLE",
        "BUZZ_ACP_SETUP_PAYLOAD",
    ] {
        env.remove(key);
    }

    let relay = non_blank(&agent.relay_url)
        .ok_or_else(|| "deploy refused: relay_url is empty".to_string())?;
    env.insert("BUZZ_RELAY_URL".into(), relay.into());
    env.insert("BUZZ_PRIVATE_KEY".into(), agent.private_key_nsec.clone());
    env.insert("NOSTR_PRIVATE_KEY".into(), agent.private_key_nsec.clone());

    let auth_tag = agent.auth_tag.as_deref().and_then(non_blank);
    let owner = launch.owner_pubkey.as_deref().and_then(non_blank);
    if auth_tag.is_none() && owner.is_none() {
        return Err("deploy refused: neither auth_tag nor launch.owner_pubkey resolved".into());
    }
    if let Some(value) = auth_tag {
        env.insert("BUZZ_AUTH_TAG".into(), value.into());
    }
    if let Some(value) = owner {
        env.insert("BUZZ_ACP_AGENT_OWNER".into(), value.into());
    }

    env.insert("BUZZ_ACP_AGENT_COMMAND".into(), remote.agent_command.into());
    if !launch.args.is_empty() {
        env.insert("BUZZ_ACP_AGENT_ARGS".into(), launch.args.join(","));
    }
    env.insert("BUZZ_ACP_MCP_COMMAND".into(), remote.mcp_command.into());
    env.insert("HOME".into(), remote.home.into());
    env.insert("PATH".into(), remote.path.into());
    env.insert("BUZZ_ACP_REPOS_DIR".into(), remote.repos_dir.into());
    env.insert("BUZZ_ACP_GIT_COMMAND".into(), remote.git_command.into());
    env.insert(
        "BUZZ_ACP_GIT_CREDENTIAL_HELPER".into(),
        remote.git_credential_helper.into(),
    );
    if let Some(mode) = agent.respond_to.as_deref().filter(|mode| !mode.is_empty()) {
        validate_access(mode, agent.respond_to_allowlist.as_deref())?;
        env.insert("BUZZ_ACP_RESPOND_TO".into(), mode.into());
    }
    if let Some(list) = agent
        .respond_to_allowlist
        .as_ref()
        .filter(|list| !list.is_empty())
    {
        env.insert("BUZZ_ACP_RESPOND_TO_ALLOWLIST".into(), list.join(","));
    }
    env.insert(START_NONCE.into(), remote.generation.into());
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(extra: serde_json::Value) -> AgentPayload {
        let mut base = serde_json::json!({
            "relay_url": "wss://relay.example",
            "private_key_nsec": "nsec1example",
            "auth_tag": "owner-tag"
        });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn remote_values_and_identity_win() {
        let payload = agent(serde_json::json!({"launch": {
            "command": "hermes-acp",
            "owner_pubkey": "beef",
            "env": {
                "HOME": "/Users/local",
                "PATH": "/local/bin",
                "BUZZ_PRIVATE_KEY": "forged",
                "BUZZ_ACP_AGENT_COMMAND": "bad",
                "BUZZ_ACP_REPOS_DIR": "/attacker/repos",
                "BUZZ_ACP_GIT_COMMAND": "/attacker/git",
                "BUZZ_ACP_GIT_CREDENTIAL_HELPER": "/attacker/helper"
            }
        }}));
        let env = build(&payload, inputs()).unwrap();
        assert_eq!(env["BUZZ_PRIVATE_KEY"], "nsec1example");
        assert_eq!(env["BUZZ_ACP_AGENT_COMMAND"], "/remote/hermes-acp");
        assert_eq!(env["BUZZ_ACP_MCP_COMMAND"], "/remote/buzz-dev-mcp");
        assert_eq!(env["PATH"], "/home/remote/.local/bin:/usr/bin");
        assert_eq!(env["BUZZ_ACP_REPOS_DIR"], "/srv/repos");
        assert_eq!(env["BUZZ_ACP_GIT_COMMAND"], "/usr/bin/git");
        assert_eq!(
            env["BUZZ_ACP_GIT_CREDENTIAL_HELPER"],
            "/usr/bin/git-credential-nostr"
        );
    }

    #[test]
    fn refuses_identityless_or_presence_suppressed_launches() {
        let no_owner: AgentPayload = serde_json::from_value(serde_json::json!({
            "relay_url": "wss://relay", "private_key_nsec": "x"
        }))
        .unwrap();
        assert!(build(&no_owner, inputs()).is_err());
        let suppressed = agent(serde_json::json!({"launch": {
            "owner_pubkey": "beef", "env": {"BUZZ_ACP_NO_PRESENCE": "1"}
        }}));
        assert!(build(&suppressed, inputs()).is_err());
    }

    fn inputs() -> RemoteInputs<'static> {
        RemoteInputs {
            generation: "generation",
            agent_command: "/remote/hermes-acp",
            mcp_command: "/remote/buzz-dev-mcp",
            home: "/home/remote",
            path: "/home/remote/.local/bin:/usr/bin",
            repos_dir: "/srv/repos",
            git_command: "/usr/bin/git",
            git_credential_helper: "/usr/bin/git-credential-nostr",
        }
    }
}
