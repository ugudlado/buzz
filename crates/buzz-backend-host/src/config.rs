#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub host: String,
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
    })
}

pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "title": "Host",
                "description": "SSH config alias for the host. Authentication and host verification use your existing SSH configuration."
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
}
