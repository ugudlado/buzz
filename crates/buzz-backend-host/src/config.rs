#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub host: String,
    pub workspace_dir: Option<String>,
    pub repos_dir: Option<String>,
}

pub fn parse(value: &serde_json::Value) -> Result<Config, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "provider_config must be a JSON object".to_string())?;
    let host = object
        .get("host")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .ok_or_else(|| "provider_config.host is required".to_string())?;
    let valid = host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && host.starts_with(|c: char| c.is_ascii_alphanumeric());
    if !valid {
        return Err("provider_config.host must be an SSH config alias containing only letters, numbers, '.', '_' or '-'".to_string());
    }
    Ok(Config {
        host: host.to_string(),
        workspace_dir: optional_remote_path(object, "workspace_dir")?,
        repos_dir: optional_remote_path(object, "repos_dir")?,
    })
}

fn optional_remote_path(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<String>, String> {
    match object.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(serde_json::Value::String(value)) => {
            let value = value.trim();
            let valid = (value == "~"
                || value.starts_with("~/")
                || std::path::Path::new(value).is_absolute())
                && !value.chars().any(char::is_control);
            valid.then(|| Some(value.to_string())).ok_or_else(|| {
                format!("provider_config.{field} must be an absolute path or start with ~/")
            })
        }
        Some(_) => Err(format!("provider_config.{field} must be a string")),
    }
}

pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "title": "Host",
                "description": "SSH config alias for the host. Authentication and host verification use your existing SSH configuration."
            },
            "workspace_dir": {
                "type": "string",
                "title": "Workspace folder",
                "description": "Existing persistent folder on the host. Defaults to the remote user's home."
            },
            "repos_dir": {
                "type": "string",
                "title": "Repositories folder",
                "description": "Existing repository folder on the host. Defaults to a persistent REPOS folder inside the workspace."
            }
        },
        "required": ["host"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ssh_aliases_and_rejects_option_injection() {
        assert_eq!(
            parse(&serde_json::json!({"host": "agent-vps.tailnet"}))
                .unwrap()
                .host,
            "agent-vps.tailnet"
        );
        for host in [
            "",
            "-oProxyCommand=bad",
            "user@host",
            "host;bad",
            "host name",
        ] {
            assert!(parse(&serde_json::json!({"host": host})).is_err(), "{host}");
        }
    }

    #[test]
    fn accepts_only_absolute_or_home_relative_remote_paths() {
        let config = parse(&serde_json::json!({
            "host": "vps", "workspace_dir": "~/buzz workspace", "repos_dir": "/srv/repos"
        }))
        .unwrap();
        assert_eq!(config.workspace_dir.as_deref(), Some("~/buzz workspace"));
        assert_eq!(config.repos_dir.as_deref(), Some("/srv/repos"));
        for value in ["relative/path", "~other/path", "~/bad\npath"] {
            assert!(parse(&serde_json::json!({"host": "vps", "workspace_dir": value})).is_err());
        }
    }
}
