//! GitHub connection: token in the OS keyring, validated against the GitHub
//! API. The token never reaches the webview — TS sees only connection status
//! and mapped pull-request data; git clones read it via [`github_token`].

use serde::{Deserialize, Serialize};

use crate::app_state::keyring_service;
use crate::secret_store::SecretStore;

const GITHUB_TOKEN_KEY: &str = "github.token";
const GITHUB_LOGIN_KEY: &str = "github.login";
const GITHUB_API: &str = "https://api.github.com";
const USER_AGENT: &str = "buzz-desktop";

fn store() -> SecretStore {
    SecretStore::keyring(keyring_service())
}

/// Stored GitHub token, if the user connected GitHub. Used by git clone auth.
pub(crate) fn github_token() -> Option<String> {
    store().load(GITHUB_TOKEN_KEY).ok().flatten()
}

#[derive(Debug, Clone, Serialize)]
pub struct GithubConnectionStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

fn api_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| format!("github client: {error}"))
}

async fn fetch_login(token: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    let response = api_client()?
        .get(format!("{GITHUB_API}/user"))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("github: {error}"))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("GitHub rejected the token. Check it and try again.".into());
    }
    if !response.status().is_success() {
        return Err(format!("github: unexpected status {}", response.status()));
    }
    let user: User = response
        .json()
        .await
        .map_err(|error| format!("github: {error}"))?;
    Ok(user.login)
}

/// Current connection state (no network call — reads the keyring only).
#[tauri::command]
pub async fn github_connection_status() -> Result<GithubConnectionStatus, String> {
    let store = store();
    let connected = store.load(GITHUB_TOKEN_KEY)?.is_some();
    Ok(GithubConnectionStatus {
        connected,
        login: connected
            .then(|| store.load(GITHUB_LOGIN_KEY).ok().flatten())
            .flatten(),
    })
}

/// Validate a personal-access token against the GitHub API and store it in
/// the OS keyring.
#[tauri::command]
pub async fn github_connect(token: String) -> Result<GithubConnectionStatus, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("A GitHub token is required.".into());
    }
    let login = fetch_login(&token).await?;
    let store = store();
    store.store(GITHUB_TOKEN_KEY, &token)?;
    store.store(GITHUB_LOGIN_KEY, &login)?;
    Ok(GithubConnectionStatus {
        connected: true,
        login: Some(login),
    })
}

/// Import the token from an authenticated `gh` CLI (`gh auth token`).
#[tauri::command]
pub async fn github_connect_from_gh_cli() -> Result<GithubConnectionStatus, String> {
    let output = tokio::task::spawn_blocking(|| {
        std::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
    .map_err(|_| "The gh CLI was not found on PATH.".to_string())?;
    if !output.status.success() {
        return Err("gh is not logged in. Run `gh auth login` first.".into());
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    github_connect(token).await
}

/// Remove the stored token.
#[tauri::command]
pub async fn github_disconnect() -> Result<(), String> {
    let store = store();
    store.delete(GITHUB_TOKEN_KEY)?;
    store.delete(GITHUB_LOGIN_KEY)?;
    Ok(())
}

/// Pull request fields the projects UI consumes (mapped to the shared
/// ProjectPullRequest shape on the TS side).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubPullRequest {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub state: String,
    pub draft: bool,
    pub merged: bool,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: String,
    pub labels: Vec<String>,
    pub requested_reviewers: Vec<String>,
}

fn valid_repo_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.starts_with('-')
        && !segment.contains("..")
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// List pull requests (all states) for a GitHub repository. Requires a
/// connected token; returns an error naming the gap otherwise.
#[tauri::command]
pub async fn github_list_pull_requests(
    owner: String,
    repo: String,
) -> Result<Vec<GithubPullRequest>, String> {
    if !valid_repo_segment(&owner) || !valid_repo_segment(&repo) {
        return Err("Invalid GitHub repository reference.".into());
    }
    let token =
        github_token().ok_or_else(|| "GitHub is not connected. Connect it first.".to_string())?;

    #[derive(Deserialize)]
    struct RawUser {
        login: String,
    }
    #[derive(Deserialize)]
    struct RawLabel {
        name: String,
    }
    #[derive(Deserialize)]
    struct RawRef {
        #[serde(rename = "ref")]
        name: String,
        sha: String,
    }
    #[derive(Deserialize)]
    struct RawPull {
        number: u64,
        title: String,
        body: Option<String>,
        user: Option<RawUser>,
        state: String,
        #[serde(default)]
        draft: bool,
        merged_at: Option<String>,
        head: RawRef,
        base: RawRef,
        html_url: String,
        created_at: String,
        updated_at: String,
        #[serde(default)]
        labels: Vec<RawLabel>,
        #[serde(default)]
        requested_reviewers: Vec<RawUser>,
    }

    let response = api_client()?
        .get(format!(
            "{GITHUB_API}/repos/{owner}/{repo}/pulls?state=all&per_page=100&sort=updated&direction=desc"
        ))
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("github: {error}"))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("GitHub rejected the stored token. Reconnect GitHub.".into());
    }
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!(
            "GitHub repository {owner}/{repo} was not found (or the token cannot see it)."
        ));
    }
    if !response.status().is_success() {
        return Err(format!("github: unexpected status {}", response.status()));
    }
    let pulls: Vec<RawPull> = response
        .json()
        .await
        .map_err(|error| format!("github: {error}"))?;

    Ok(pulls
        .into_iter()
        .map(|pull| GithubPullRequest {
            number: pull.number,
            title: pull.title,
            body: pull.body.unwrap_or_default(),
            author: pull.user.map(|user| user.login).unwrap_or_default(),
            merged: pull.merged_at.is_some(),
            state: pull.state,
            draft: pull.draft,
            head_ref: pull.head.name,
            base_ref: pull.base.name,
            head_sha: pull.head.sha,
            html_url: pull.html_url,
            created_at: pull.created_at,
            updated_at: pull.updated_at,
            labels: pull.labels.into_iter().map(|label| label.name).collect(),
            requested_reviewers: pull
                .requested_reviewers
                .into_iter()
                .map(|user| user.login)
                .collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::valid_repo_segment;

    #[test]
    fn repo_segments_reject_flags_and_traversal() {
        assert!(valid_repo_segment("buzz"));
        assert!(valid_repo_segment("my.repo-name_1"));
        assert!(!valid_repo_segment(""));
        assert!(!valid_repo_segment("-flag"));
        assert!(!valid_repo_segment("a..b"));
        assert!(!valid_repo_segment("a/b"));
    }
}
