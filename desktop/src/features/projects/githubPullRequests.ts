// GitHub pull-request provider for project repositories.
//
// Repositories whose canonical remote is github.com (`Repository.githubRepo`)
// get their pull requests from the GitHub API (via the Rust `github_*`
// commands — the token stays in the OS keyring) instead of relay NIP-34
// events. PRs are mapped into the shared ProjectPullRequest shape with
// `github:`-prefixed ids so they can never collide with 64-hex event ids.

import {
  getGithubConnectionStatus,
  type GithubPullRequest,
  listGithubPullRequests,
} from "@/shared/api/tauriGithub";
import type { GithubRepoRef } from "./projectModels";
import type { ProjectPullRequest } from "./projectPullRequests.mjs";

const GITHUB_PR_ID_PREFIX = "github:";

/** True when a ProjectPullRequest id refers to a GitHub pull request. */
export function isGithubPullRequestId(id: string): boolean {
  return id.startsWith(GITHUB_PR_ID_PREFIX);
}

/** `https://github.com/...` page for a mapped GitHub PR (from its `web` tag). */
export function githubPullRequestUrl(
  pullRequest: ProjectPullRequest,
): string | null {
  return (
    pullRequest.tags.find((tag) => tag[0] === "web" && tag[1])?.[1] ?? null
  );
}

function isoToUnixSeconds(iso: string): number {
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? 0 : Math.floor(ms / 1_000);
}

function githubPullRequestStatus(
  pull: GithubPullRequest,
): ProjectPullRequest["status"] {
  if (pull.merged) return "Merged";
  if (pull.state === "closed") return "Closed";
  if (pull.draft) return "Draft";
  return "Open";
}

/** Map one GitHub PR into the shared ProjectPullRequest shape. */
export function githubPullToProjectPullRequest(
  pull: GithubPullRequest,
  repoAddress: string,
): ProjectPullRequest {
  return {
    id: `${GITHUB_PR_ID_PREFIX}${pull.number}`,
    title: `#${pull.number} ${pull.title}`,
    content: pull.body,
    tags: [["web", pull.htmlUrl]],
    author: pull.author,
    createdAt: isoToUnixSeconds(pull.createdAt),
    repoAddress,
    channelId: null,
    originAgentName: null,
    labels: pull.labels,
    recipients: [],
    reviewers: pull.requestedReviewers,
    approvals: [],
    changeRequests: [],
    status: githubPullRequestStatus(pull),
    statusEventId: null,
    statusCreatedAt: null,
    branchName: pull.headRef,
    targetBranch: pull.baseRef,
    initialCommit: pull.headSha,
    commit: pull.headSha,
    cloneUrls: [],
    updateCount: 0,
    updatedAt: isoToUnixSeconds(pull.updatedAt),
    updates: [],
    comments: [],
  };
}

type GithubTrackedRepository = {
  repoAddress: string;
  githubRepo: GithubRepoRef;
};

/**
 * Fetch pull requests for GitHub-hosted repositories (one API call per repo,
 * concurrently). Returns PRs keyed by repoAddress. When GitHub is not
 * connected this resolves empty — not-connected is the normal state for
 * viewers who haven't linked GitHub, not a load failure.
 */
export async function fetchGithubPullRequestsForRepos(
  repos: GithubTrackedRepository[],
  listPullRequests: typeof listGithubPullRequests = listGithubPullRequests,
  getConnectionStatus: typeof getGithubConnectionStatus = getGithubConnectionStatus,
): Promise<Map<string, ProjectPullRequest[]>> {
  const result = new Map<string, ProjectPullRequest[]>();
  if (repos.length === 0) return result;
  const status = await getConnectionStatus().catch(() => null);
  if (!status?.connected) return result;
  await Promise.all(
    repos.map(async (repo) => {
      const pulls = await listPullRequests(
        repo.githubRepo.owner,
        repo.githubRepo.name,
      );
      result.set(
        repo.repoAddress,
        pulls.map((pull) =>
          githubPullToProjectPullRequest(pull, repo.repoAddress),
        ),
      );
    }),
  );
  return result;
}
