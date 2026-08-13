mod config;
mod env;
mod identity;
mod remote;
mod ssh;
mod wire;

use std::io::Read;
use std::path::Path;
use wire::{Request, Response};

const RELAY_MESH_PROVIDER: &str = "relay-mesh";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "remote-deploy" => remote_deploy_main(),
        [command, generation, acp, workspace] if command == "run" => {
            if let Err(error) =
                remote::run(Path::new(generation), Path::new(acp), Path::new(workspace))
            {
                eprintln!("buzz-backend-host runner: {error}");
                std::process::exit(1);
            }
        }
        [] => provider_main(),
        _ => {
            eprintln!(
                "usage: buzz-backend-host [remote-deploy|run <generation> <buzz-acp> <workspace>]"
            );
            std::process::exit(1);
        }
    }
}

fn provider_main() {
    let input = match read_stdin() {
        Ok(input) => input,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    print_response(respond(&input));
}

fn remote_deploy_main() {
    let response = match read_stdin().and_then(|input| {
        serde_json::from_str(&input).map_err(|_| "invalid remote deploy request".into())
    }) {
        Ok(request) => match remote::deploy(&request) {
            Ok(agent_id) => Response::deployed(agent_id),
            Err(error) => Response::error(error),
        },
        Err(error) => Response::error(error),
    };
    print_response(response);
}

fn read_stdin() -> Result<String, String> {
    let mut input = String::new();
    std::io::stdin()
        .take(2 * 1024 * 1024 + 1)
        .read_to_string(&mut input)
        .map_err(|e| format!("could not read request from stdin: {e}"))?;
    if input.len() > 2 * 1024 * 1024 {
        return Err("request exceeds 2 MB".into());
    }
    Ok(input)
}

fn print_response(response: Response) {
    println!(
        "{}",
        serde_json::to_string(&response)
            .unwrap_or_else(|_| r#"{"ok":false,"error":"could not encode response"}"#.into())
    );
}

fn respond(input: &str) -> Response {
    let raw: serde_json::Value = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(_) => return Response::error("request is not valid JSON"),
    };
    if raw
        .get("agent")
        .and_then(|agent| agent.get("provider"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|provider| provider.trim() == RELAY_MESH_PROVIDER)
    {
        return Response::error("deploy refused: relay-mesh agents run on relay compute");
    }
    let request: Request = match serde_json::from_value(raw) {
        Ok(request) => request,
        Err(_) => return Response::error("could not understand the request"),
    };
    match request {
        Request::Info => Response::info(),
        Request::Deploy(request) => {
            let config = match config::parse(&request.provider_config) {
                Ok(config) => config,
                Err(error) => return Response::error(error),
            };
            if let Err(error) = identity::Identity::from_nsec(&request.agent.private_key_nsec) {
                return Response::error(error);
            }
            match ssh::deploy(&config.host, &request) {
                Ok(agent_id) => Response::deployed(agent_id),
                Err(error) => Response::error(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(response: Response) -> String {
        serde_json::to_value(response).unwrap()["error"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn info_is_pure_and_exposes_host_workspace_and_repos_fields() {
        let value = serde_json::to_value(respond(r#"{"op":"info"}"#)).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["protocol_version"], wire::PROTOCOL_VERSION);
        assert!(value["description"]
            .as_str()
            .unwrap()
            .contains("https://github.com/block/buzz/blob/main/docs/host-agents.md"));
        assert!(value["config_schema"]["properties"]["host"].is_object());
        let properties = value["config_schema"]["properties"].as_object().unwrap();
        assert_eq!(properties.len(), 3);
        assert!(properties["workspace_dir"].is_object());
        assert!(properties["repos_dir"].is_object());
    }

    #[test]
    fn malformed_and_relay_mesh_requests_fail_in_band() {
        assert!(error(respond("not-json")).contains("valid JSON"));
        let mesh = r#"{"op":"deploy","agent":{"provider":" relay-mesh ","relay_url":"wss://r","private_key_nsec":"bad"},"provider_config":{"host":"vps"}}"#;
        assert!(error(respond(mesh)).contains("relay-mesh"));
    }
}
