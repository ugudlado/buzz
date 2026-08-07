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
