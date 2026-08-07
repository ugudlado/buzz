import { invokeTauri } from "@/shared/api/tauri";

/** GitHub connection state. The token itself never reaches the webview. */
export type GithubConnectionStatus = {
  connected: boolean;
  login?: string;
};

/** Pull request fields returned by the Rust GitHub client (camelCase). */
export type GithubPullRequest = {
  number: number;
  title: string;
  body: string;
  author: string;
  state: string;
  draft: boolean;
  merged: boolean;
  headRef: string;
  baseRef: string;
  headSha: string;
  htmlUrl: string;
  createdAt: string;
  updatedAt: string;
  labels: string[];
  requestedReviewers: string[];
};

export function getGithubConnectionStatus(): Promise<GithubConnectionStatus> {
  return invokeTauri<GithubConnectionStatus>("github_connection_status");
}

/** Validate and store a personal-access token (kept in the OS keyring). */
export function connectGithub(token: string): Promise<GithubConnectionStatus> {
  return invokeTauri<GithubConnectionStatus>("github_connect", { token });
}

/** Import the token from an authenticated `gh` CLI. */
export function connectGithubFromGhCli(): Promise<GithubConnectionStatus> {
  return invokeTauri<GithubConnectionStatus>("github_connect_from_gh_cli");
}

export function disconnectGithub(): Promise<void> {
  return invokeTauri<void>("github_disconnect");
}

export function listGithubPullRequests(
  owner: string,
  repo: string,
): Promise<GithubPullRequest[]> {
  return invokeTauri<GithubPullRequest[]>("github_list_pull_requests", {
    owner,
    repo,
  });
}

/** Repository summary for the project-creation repo picker. */
export type GithubRepoSummary = {
  owner: string;
  name: string;
  cloneUrl: string;
  private: boolean;
};

/** Repos visible to the connected GitHub account, most recently pushed first. */
export function listGithubRepos(): Promise<GithubRepoSummary[]> {
  return invokeTauri<GithubRepoSummary[]>("github_list_repos");
}

/** Branch summary for the project repository/branch picker. */
export type GithubBranchSummary = {
  name: string;
  commitSha: string;
  protected: boolean;
};

/** List branches for a GitHub repository. */
export function listGithubBranches(
  owner: string,
  repo: string,
): Promise<GithubBranchSummary[]> {
  return invokeTauri<GithubBranchSummary[]>("github_list_branches", {
    owner,
    repo,
  });
}

/** File/directory entry in the repository tree for a given branch/path. */
export type GithubTreeEntry = {
  name: string;
  path: string;
  entryType: string;
  size: number | null;
};

/**
 * List the file tree at a given branch/path (repo root when `path` is
 * omitted). Errors if the path resolves to a single file — use
 * `getGithubFileContent` for that.
 */
export function getGithubTree(
  owner: string,
  repo: string,
  branch: string,
  path?: string,
): Promise<GithubTreeEntry[]> {
  return invokeTauri<GithubTreeEntry[]>("github_get_tree", {
    owner,
    repo,
    branch,
    path,
  });
}

/** Aggregate GitHub activity counts for a repository (projects-list stats). */
export type GithubRepoActivity = {
  openIssueCount: number;
  openPrCount: number;
  commitCount: number;
  /** Unix seconds, from the repo's pushed_at/updated_at. */
  updatedAt: number;
};

/**
 * Fetch aggregate activity counts (open issues excluding PRs, open PRs,
 * an approximate commit count, and last-pushed time) for a GitHub
 * repository in ~3 API calls.
 */
export function getGithubRepoActivity(
  owner: string,
  repo: string,
): Promise<GithubRepoActivity> {
  return invokeTauri<GithubRepoActivity>("github_repo_activity", {
    owner,
    repo,
  });
}

/** Decoded file content from the repository. */
export type GithubFileContent = {
  path: string;
  content: string;
  size: number;
};

/** Fetch a single file's decoded content. Errors for binary files. */
export function getGithubFileContent(
  owner: string,
  repo: string,
  branch: string,
  path: string,
): Promise<GithubFileContent> {
  return invokeTauri<GithubFileContent>("github_get_file_content", {
    owner,
    repo,
    branch,
    path,
  });
}
