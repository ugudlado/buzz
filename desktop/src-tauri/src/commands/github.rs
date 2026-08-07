//! GitHub connection: token in the OS keyring, validated against the GitHub
//! API. The token never reaches the webview — TS sees only connection status
//! and mapped pull-request data; git clones read it via [`github_token`].

use base64::Engine as _;
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
    check_github_status(
        &response,
        Some(&format!(
            "GitHub repository {owner}/{repo} was not found (or the token cannot see it)."
        )),
    )?;
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

/// Repository summary for the project-creation repo picker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepoSummary {
    pub owner: String,
    pub name: String,
    pub clone_url: String,
    pub private: bool,
}

/// Cap on repos returned to the picker — plenty for a dropdown, and bounds
/// worst-case pagination against very large accounts/orgs.
const LIST_REPOS_MAX: usize = 500;
const LIST_REPOS_PER_PAGE: u32 = 100;

/// List repositories visible to the connected GitHub account (owned,
/// collaborator, and organization-member repos), most recently pushed
/// first. Paginates until exhausted or [`LIST_REPOS_MAX`] is reached.
#[tauri::command]
pub async fn github_list_repos() -> Result<Vec<GithubRepoSummary>, String> {
    let token =
        github_token().ok_or_else(|| "GitHub is not connected. Connect it first.".to_string())?;

    #[derive(Deserialize)]
    struct RawOwner {
        login: String,
    }
    #[derive(Deserialize)]
    struct RawRepo {
        name: String,
        owner: RawOwner,
        #[serde(default)]
        private: bool,
    }

    let client = api_client()?;
    let mut repos = Vec::new();
    let mut page = 1u32;
    loop {
        let response = client
            .get(format!("{GITHUB_API}/user/repos"))
            .query(&[
                ("affiliation", "owner,collaborator,organization_member"),
                ("sort", "pushed"),
                ("per_page", &LIST_REPOS_PER_PAGE.to_string()),
                ("page", &page.to_string()),
            ])
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| format!("github: {error}"))?;
        check_github_status(&response, None)?;
        let raw_repos: Vec<RawRepo> = response
            .json()
            .await
            .map_err(|error| format!("github: {error}"))?;
        let fetched = raw_repos.len();

        for repo in raw_repos {
            repos.push(GithubRepoSummary {
                clone_url: format!("https://github.com/{}/{}", repo.owner.login, repo.name),
                owner: repo.owner.login,
                name: repo.name,
                private: repo.private,
            });
            if repos.len() >= LIST_REPOS_MAX {
                return Ok(repos);
            }
        }

        if fetched < LIST_REPOS_PER_PAGE as usize {
            break;
        }
        page += 1;
    }

    Ok(repos)
}

/// Branch summary for the project repository/branch picker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubBranchSummary {
    pub name: String,
    pub commit_sha: String,
    pub protected: bool,
}

/// Cap on branches returned to the picker — proportionate for a picker, not
/// a full sync.
const LIST_BRANCHES_MAX: usize = 300;
const LIST_BRANCHES_PER_PAGE: u32 = 100;

/// List branches for a GitHub repository. Paginates until exhausted or
/// [`LIST_BRANCHES_MAX`] is reached.
#[tauri::command]
pub async fn github_list_branches(
    owner: String,
    repo: String,
) -> Result<Vec<GithubBranchSummary>, String> {
    if !valid_repo_segment(&owner) || !valid_repo_segment(&repo) {
        return Err("Invalid GitHub repository reference.".into());
    }
    let token =
        github_token().ok_or_else(|| "GitHub is not connected. Connect it first.".to_string())?;

    #[derive(Deserialize)]
    struct RawCommit {
        sha: String,
    }
    #[derive(Deserialize)]
    struct RawBranch {
        name: String,
        commit: RawCommit,
        #[serde(default)]
        protected: bool,
    }

    let client = api_client()?;
    let mut branches = Vec::new();
    let mut page = 1u32;
    loop {
        let response = client
            .get(format!("{GITHUB_API}/repos/{owner}/{repo}/branches"))
            .query(&[
                ("per_page", &LIST_BRANCHES_PER_PAGE.to_string()),
                ("page", &page.to_string()),
            ])
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| format!("github: {error}"))?;
        check_github_status(
            &response,
            Some(&format!(
                "GitHub repository {owner}/{repo} was not found (or the token cannot see it)."
            )),
        )?;
        let raw_branches: Vec<RawBranch> = response
            .json()
            .await
            .map_err(|error| format!("github: {error}"))?;
        let fetched = raw_branches.len();

        for branch in raw_branches {
            branches.push(GithubBranchSummary {
                name: branch.name,
                commit_sha: branch.commit.sha,
                protected: branch.protected,
            });
            if branches.len() >= LIST_BRANCHES_MAX {
                return Ok(branches);
            }
        }

        if fetched < LIST_BRANCHES_PER_PAGE as usize {
            break;
        }
        page += 1;
    }

    Ok(branches)
}

/// File/directory entry in the repository tree for a given branch/path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubTreeEntry {
    pub name: String,
    pub path: String,
    pub entry_type: String,
    pub size: Option<u64>,
}

/// List the file tree at a given branch/path (repo root when `path` is
/// `None`/empty). Errors if the path resolves to a single file — use
/// [`github_get_file_content`] for that.
#[tauri::command]
pub async fn github_get_tree(
    owner: String,
    repo: String,
    branch: String,
    path: Option<String>,
) -> Result<Vec<GithubTreeEntry>, String> {
    if !valid_repo_segment(&owner) || !valid_repo_segment(&repo) {
        return Err("Invalid GitHub repository reference.".into());
    }
    let token =
        github_token().ok_or_else(|| "GitHub is not connected. Connect it first.".to_string())?;

    #[derive(Deserialize)]
    struct RawEntry {
        name: String,
        path: String,
        #[serde(rename = "type")]
        entry_type: String,
        #[serde(default)]
        size: Option<u64>,
    }

    let path = path.unwrap_or_default();
    let url = if path.is_empty() {
        format!("{GITHUB_API}/repos/{owner}/{repo}/contents")
    } else {
        format!("{GITHUB_API}/repos/{owner}/{repo}/contents/{path}")
    };

    let response = api_client()?
        .get(url)
        .query(&[("ref", &branch)])
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("github: {error}"))?;
    check_github_status(
        &response,
        Some(&format!(
            "GitHub path {path} was not found on {owner}/{repo}@{branch}."
        )),
    )?;
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("github: {error}"))?;

    let raw_entries: Vec<RawEntry> = match value {
        serde_json::Value::Array(_) => {
            serde_json::from_value(value).map_err(|error| format!("github: {error}"))?
        }
        serde_json::Value::Object(_) => {
            return Err(
                "That path is a file, not a directory. Use github_get_file_content instead.".into(),
            );
        }
        _ => return Err("github: unexpected response shape for contents API".into()),
    };

    Ok(raw_entries
        .into_iter()
        .map(|entry| GithubTreeEntry {
            name: entry.name,
            path: entry.path,
            entry_type: entry.entry_type,
            size: entry.size,
        })
        .collect())
}

/// Decoded file content from the repository.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubFileContent {
    pub path: String,
    pub content: String,
    pub size: u64,
}

/// Fetch a single file's content, decoded from GitHub's base64 encoding.
/// Errors for binary files (not valid UTF-8 after decoding).
#[tauri::command]
pub async fn github_get_file_content(
    owner: String,
    repo: String,
    branch: String,
    path: String,
) -> Result<GithubFileContent, String> {
    if !valid_repo_segment(&owner) || !valid_repo_segment(&repo) {
        return Err("Invalid GitHub repository reference.".into());
    }
    if path.is_empty() {
        return Err("A file path is required.".into());
    }
    let token =
        github_token().ok_or_else(|| "GitHub is not connected. Connect it first.".to_string())?;

    #[derive(Deserialize)]
    struct RawFile {
        #[serde(default)]
        #[serde(rename = "type")]
        entry_type: String,
        size: u64,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        encoding: Option<String>,
    }

    let response = api_client()?
        .get(format!("{GITHUB_API}/repos/{owner}/{repo}/contents/{path}"))
        .query(&[("ref", &branch)])
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("github: {error}"))?;
    check_github_status(
        &response,
        Some(&format!(
            "GitHub path {path} was not found on {owner}/{repo}@{branch}."
        )),
    )?;
    let raw: RawFile = response
        .json()
        .await
        .map_err(|error| format!("github: {error}"))?;

    if raw.entry_type == "dir" {
        return Err("That path is a directory, not a file. Use github_get_tree instead.".into());
    }
    let encoding = raw.encoding.unwrap_or_default();
    if encoding != "base64" {
        return Err(format!("github: unsupported content encoding {encoding}"));
    }
    let encoded = raw.content.unwrap_or_default();
    let decoded_bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.replace(['\n', '\r'], ""))
        .map_err(|error| format!("github: failed to decode file content: {error}"))?;
    let content = String::from_utf8(decoded_bytes)
        .map_err(|_| "Binary files are not supported.".to_string())?;

    Ok(GithubFileContent {
        path,
        content,
        size: raw.size,
    })
}

/// Aggregate GitHub activity counts for a repository (projects-list stats).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepoActivity {
    pub open_issue_count: u64,
    pub open_pr_count: u64,
    pub commit_count: u64,
    pub updated_at: i64,
}

/// Reads the page number from a `Link` response header's `rel="last"` entry,
/// per the standard GitHub pagination trick — avoids paginating through the
/// full result set just to count it.
fn last_page_from_link_header(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let link = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    for part in link.split(',') {
        let mut segments = part.split(';');
        let url_segment = segments.next()?.trim();
        let is_last = segments.any(|segment| segment.trim() == "rel=\"last\"");
        if !is_last {
            continue;
        }
        let url = url_segment.trim_start_matches('<').trim_end_matches('>');
        let parsed = url::Url::parse(url).ok()?;
        let page = parsed
            .query_pairs()
            .find(|(key, _)| key == "page")
            .and_then(|(_, value)| value.parse::<u64>().ok())?;
        return Some(page);
    }
    None
}

fn iso_to_unix_seconds(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

/// Maps a GitHub API response's status to the shared error strings used
/// across every command in this file. `not_found_message` is the error to
/// return on a 404; pass `None` where a 404 isn't expected/handled specially
/// (falls through to the generic "unexpected status" error instead).
fn check_github_status(
    response: &reqwest::Response,
    not_found_message: Option<&str>,
) -> Result<(), String> {
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("GitHub rejected the stored token. Reconnect GitHub.".into());
    }
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        if let Some(message) = not_found_message {
            return Err(message.to_string());
        }
    }
    if !response.status().is_success() {
        return Err(format!("github: unexpected status {}", response.status()));
    }
    Ok(())
}

/// Fetches a `per_page=1` page for `path` and returns an approximate total
/// count via the `Link` header's last-page number (falls back to the number
/// of items in the single page when no `Link` header is present).
async fn approximate_count(
    client: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
    resource: &str,
    extra_query: &[(&str, &str)],
) -> Result<u64, String> {
    let mut query: Vec<(&str, &str)> = vec![("per_page", "1")];
    query.extend_from_slice(extra_query);

    let response = client
        .get(format!("{GITHUB_API}/repos/{owner}/{repo}/{resource}"))
        .query(&query)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("github: {error}"))?;
    check_github_status(&response, None)?;
    if let Some(last_page) = last_page_from_link_header(response.headers()) {
        return Ok(last_page);
    }
    let items: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|error| format!("github: {error}"))?;
    Ok(items.len() as u64)
}

/// Aggregate activity counts for a GitHub repository: open issues (excluding
/// PRs), open PRs, an approximate commit count, and the last-pushed
/// timestamp. Uses ~3 API calls total (repo metadata + two `per_page=1`
/// Link-header probes) rather than paginating through full histories, since
/// the projects list can render many cards at once.
#[tauri::command]
pub async fn github_repo_activity(
    owner: String,
    repo: String,
) -> Result<GithubRepoActivity, String> {
    if !valid_repo_segment(&owner) || !valid_repo_segment(&repo) {
        return Err("Invalid GitHub repository reference.".into());
    }
    let token =
        github_token().ok_or_else(|| "GitHub is not connected. Connect it first.".to_string())?;

    #[derive(Deserialize)]
    struct RawRepo {
        open_issues_count: u64,
        pushed_at: Option<String>,
        updated_at: Option<String>,
    }

    let client = api_client()?;

    let fetch_repo = async {
        let response = client
            .get(format!("{GITHUB_API}/repos/{owner}/{repo}"))
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| format!("github: {error}"))?;
        check_github_status(
            &response,
            Some(&format!(
                "GitHub repository {owner}/{repo} was not found (or the token cannot see it)."
            )),
        )?;
        response
            .json::<RawRepo>()
            .await
            .map_err(|error| format!("github: {error}"))
    };

    let (raw_repo, open_pr_count, commit_count) = tokio::try_join!(
        fetch_repo,
        approximate_count(
            &client,
            &token,
            &owner,
            &repo,
            "pulls",
            &[("state", "open")]
        ),
        approximate_count(&client, &token, &owner, &repo, "commits", &[]),
    )?;

    let open_issue_count = raw_repo.open_issues_count.saturating_sub(open_pr_count);
    let updated_at = raw_repo
        .pushed_at
        .as_deref()
        .or(raw_repo.updated_at.as_deref())
        .map(iso_to_unix_seconds)
        .unwrap_or(0);

    Ok(GithubRepoActivity {
        open_issue_count,
        open_pr_count,
        commit_count,
        updated_at,
    })
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
