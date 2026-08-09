//! Backlog connection + data plane: credentials in the OS keyring, all HTTP
//! on the Rust side. The webview sees connection *status*, project lists,
//! mapped task data, and provisioned agent env — never the user token.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::app_state::keyring_service;
use crate::secret_store::SecretStore;

const BASE_URL_KEY: &str = "backlog.base_url";
const TOKEN_KEY: &str = "backlog.token";
const USER_NAME_KEY: &str = "backlog.user_name";

fn store() -> SecretStore {
    SecretStore::keyring(keyring_service())
}

fn client() -> Result<reqwest::Client, String> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let built = reqwest::Client::builder()
        .user_agent("buzz-desktop")
        .build()
        .map_err(|error| format!("backlog client: {error}"))?;
    Ok(CLIENT.get_or_init(|| built).clone())
}

struct Connection {
    base_url: String,
    token: String,
}

fn stored_connection() -> Result<Option<Connection>, String> {
    let store = store();
    match (store.load(BASE_URL_KEY)?, store.load(TOKEN_KEY)?) {
        (Some(base_url), Some(token)) => Ok(Some(Connection { base_url, token })),
        _ => Ok(None),
    }
}

fn require_connection() -> Result<Connection, String> {
    stored_connection()?.ok_or_else(|| "Backlog is not connected. Connect it first.".to_string())
}

/// Trims whitespace/trailing slashes and defaults a missing scheme to
/// `http://` — a bare `host:port` (e.g. pasted from a terminal prompt) is
/// not a valid absolute URL and would otherwise fail deep in `reqwest` with
/// an opaque "builder error" instead of connecting.
fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

/// Plain `http://` sends the email/password or token over cleartext, which
/// is only safe on loopback (the local Backlog dev server this app talks to
/// in development). Reject `http://` to any other host — remote connections
/// must use `https://`.
fn reject_insecure_remote_url(base_url: &str) -> Result<(), String> {
    let Some(rest) = base_url.strip_prefix("http://") else {
        return Ok(());
    };
    let host = if let Some(bracketed) = rest.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        rest.split(['/', ':']).next().unwrap_or("")
    };
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return Ok(());
    }
    Err("Refusing to connect over http:// to a non-localhost Backlog server — credentials would be sent in cleartext. Use https://.".into())
}

async fn backlog_request(
    connection: &Connection,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<reqwest::Response, String> {
    let mut request = client()?
        .request(method, format!("{}{path}", connection.base_url))
        .bearer_auth(&connection.token)
        .header("Accept", "application/json");
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("backlog: {error}"))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Backlog rejected the stored token. Reconnect Backlog.".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "backlog: unexpected status {} on {path}",
            response.status()
        ));
    }
    Ok(response)
}

/// `backlog_request` + JSON decode, shared by every body-returning call.
async fn backlog_json<T: serde::de::DeserializeOwned>(
    connection: &Connection,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<T, String> {
    backlog_request(connection, method, path, body)
        .await?
        .json()
        .await
        .map_err(|error| format!("backlog: {error}"))
}

async fn fetch_me(connection: &Connection) -> Result<String, String> {
    #[derive(Deserialize)]
    struct MeUser {
        name: String,
    }
    #[derive(Deserialize)]
    struct Me {
        user: Option<MeUser>,
    }
    let me: Me = backlog_json(connection, reqwest::Method::GET, "/api/me", None).await?;
    Ok(me.user.map(|user| user.name).unwrap_or_default())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklogConnectionStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
}

/// Current connection state (keyring read only — no network).
#[tauri::command]
pub async fn backlog_status() -> Result<BacklogConnectionStatus, String> {
    let Some(connection) = stored_connection()? else {
        return Ok(BacklogConnectionStatus {
            connected: false,
            base_url: None,
            user_name: None,
        });
    };
    Ok(BacklogConnectionStatus {
        connected: true,
        base_url: Some(connection.base_url),
        user_name: store().load(USER_NAME_KEY)?,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklogConnectRequest {
    pub base_url: String,
    /// Existing `bklg_` token — or omit and pass email + password.
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

/// Connect: use a pasted token, or mint one via `POST /api/auth/login`.
/// Validated with `GET /api/me`, then stored in the OS keyring.
#[tauri::command]
pub async fn backlog_connect(
    input: BacklogConnectRequest,
) -> Result<BacklogConnectionStatus, String> {
    let base_url = normalize_base_url(&input.base_url);
    if base_url.is_empty() {
        return Err("Backlog server URL is required.".into());
    }
    reject_insecure_remote_url(&base_url)?;

    let token = match input.token.map(|t| t.trim().to_string()) {
        Some(token) if !token.is_empty() => token,
        _ => {
            let (Some(email), Some(password)) = (input.email, input.password) else {
                return Err("Provide a token, or email and password.".into());
            };
            #[derive(Deserialize)]
            struct Login {
                token: String,
            }
            let response = client()?
                .post(format!("{base_url}/api/auth/login"))
                .json(&json!({ "email": email, "password": password }))
                .send()
                .await
                .map_err(|error| format!("backlog: {error}"))?;
            if !response.status().is_success() {
                return Err("Backlog login failed. Check email and password.".into());
            }
            let login: Login = response
                .json()
                .await
                .map_err(|error| format!("backlog: {error}"))?;
            login.token
        }
    };

    let connection = Connection {
        base_url: base_url.clone(),
        token: token.clone(),
    };
    let user_name = fetch_me(&connection).await?;

    let store = store();
    store.store(BASE_URL_KEY, &base_url)?;
    store.store(TOKEN_KEY, &token)?;
    store.store(USER_NAME_KEY, &user_name)?;
    Ok(BacklogConnectionStatus {
        connected: true,
        base_url: Some(base_url),
        user_name: Some(user_name),
    })
}

/// Remove the stored connection.
#[tauri::command]
pub async fn backlog_disconnect() -> Result<(), String> {
    let store = store();
    store.delete(BASE_URL_KEY)?;
    store.delete(TOKEN_KEY)?;
    store.delete(USER_NAME_KEY)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklogProject {
    pub guid: String,
    pub path: String,
}

/// Projects visible to the connected user (`GET /api/projects`).
#[tauri::command]
pub async fn backlog_list_projects() -> Result<Vec<BacklogProject>, String> {
    #[derive(Deserialize)]
    struct Projects {
        projects: Vec<BacklogProject>,
    }
    let connection = require_connection()?;
    let projects: Projects =
        backlog_json(&connection, reqwest::Method::GET, "/api/projects", None).await?;
    Ok(projects.projects)
}

/// Raw task rows for a project (the TS provider maps them to ProjectIssue).
#[tauri::command]
pub async fn backlog_list_tasks(project_guid: String) -> Result<serde_json::Value, String> {
    let connection = require_connection()?;
    backlog_json(
        &connection,
        reqwest::Method::GET,
        &format!("/api/projects/{}/tasks", urlencode(&project_guid)),
        None,
    )
    .await
}

/// Create a task; returns the raw created task (with `id`).
#[tauri::command]
pub async fn backlog_create_task(
    project_guid: String,
    title: String,
    description: Option<String>,
) -> Result<serde_json::Value, String> {
    let connection = require_connection()?;
    let mut body = json!({ "title": title });
    if let Some(description) = description.filter(|d| !d.trim().is_empty()) {
        body["description"] = json!(description.trim());
    }
    backlog_json(
        &connection,
        reqwest::Method::POST,
        &format!("/api/projects/{}/tasks", urlencode(&project_guid)),
        Some(body),
    )
    .await
}

/// Append a comment to a task.
#[tauri::command]
pub async fn backlog_create_task_comment(
    project_guid: String,
    task_id: String,
    body: String,
) -> Result<(), String> {
    let connection = require_connection()?;
    backlog_request(
        &connection,
        reqwest::Method::POST,
        &format!(
            "/api/projects/{}/tasks/{}/comments",
            urlencode(&project_guid),
            urlencode(&task_id)
        ),
        Some(json!({ "body": body })),
    )
    .await?;
    Ok(())
}

/// Env block for a workflow agent: create/reuse a Backlog agent, grant it the
/// project, mint a project-pinned token. The returned map is meant for
/// persona `env_vars` (persisted local config — not a user credential).
/// Projects are addressed by guid everywhere (the server's resolveProjectRef
/// accepts guid strings on grants and tokens).
#[tauri::command]
pub async fn backlog_provision_agent_env(
    agent_name: String,
    project_guid: String,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    #[derive(Deserialize)]
    struct Agent {
        id: String,
        name: String,
    }
    #[derive(Deserialize)]
    struct Token {
        token: String,
    }
    let connection = require_connection()?;
    let agent_name = agent_name.trim();
    if agent_name.is_empty() {
        return Err("Agent name is required.".into());
    }

    let agents: Vec<Agent> =
        backlog_json(&connection, reqwest::Method::GET, "/api/agents", None).await?;
    let agent_id = match agents.into_iter().find(|agent| agent.name == agent_name) {
        Some(agent) => agent.id,
        None => {
            let created: Agent = backlog_json(
                &connection,
                reqwest::Method::POST,
                "/api/agents",
                Some(json!({ "name": agent_name })),
            )
            .await?;
            created.id
        }
    };

    backlog_request(
        &connection,
        reqwest::Method::POST,
        &format!("/api/agents/{}/grants", urlencode(&agent_id)),
        Some(json!({ "projectId": project_guid })),
    )
    .await?;

    // Rotate rather than accumulate: the agent is Buzz-managed, so its old
    // tokens are always ours — revoke them before minting the fresh one.
    backlog_request(
        &connection,
        reqwest::Method::DELETE,
        &format!("/api/agents/{}/tokens", urlencode(&agent_id)),
        None,
    )
    .await?;

    let token: Token = backlog_json(
        &connection,
        reqwest::Method::POST,
        &format!("/api/agents/{}/tokens", urlencode(&agent_id)),
        Some(json!({ "label": "buzz", "projectId": project_guid })),
    )
    .await?;

    Ok(std::collections::BTreeMap::from([
        ("BACKLOG_URL".into(), connection.base_url.clone()),
        ("BACKLOG_TOKEN".into(), token.token),
        ("BACKLOG_PROJECT_ID".into(), project_guid),
    ]))
}

fn urlencode(segment: &str) -> String {
    segment
        .bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{normalize_base_url, reject_insecure_remote_url, urlencode};

    #[test]
    fn base_url_normalization_strips_trailing_slashes() {
        assert_eq!(
            normalize_base_url(" http://localhost:4321// "),
            "http://localhost:4321"
        );
    }

    #[test]
    fn base_url_normalization_defaults_missing_scheme_to_http() {
        assert_eq!(
            normalize_base_url("127.0.0.1:4321"),
            "http://127.0.0.1:4321"
        );
        assert_eq!(
            normalize_base_url("localhost:4321"),
            "http://localhost:4321"
        );
        assert_eq!(
            normalize_base_url("https://backlog.example.com/"),
            "https://backlog.example.com"
        );
    }

    #[test]
    fn urlencode_escapes_reserved_bytes() {
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(urlencode("my guid/x"), "my%20guid%2Fx");
    }

    #[test]
    fn insecure_remote_url_rejects_non_loopback_http() {
        assert!(reject_insecure_remote_url("http://localhost:4321").is_ok());
        assert!(reject_insecure_remote_url("http://127.0.0.1:4321").is_ok());
        assert!(reject_insecure_remote_url("http://[::1]:4321").is_ok());
        assert!(reject_insecure_remote_url("https://backlog.example.com").is_ok());
        assert!(reject_insecure_remote_url("http://backlog.example.com").is_err());
    }
}
