// GitHub activity-count provider for project/repository cards.
//
// Repositories whose canonical remote is github.com (`Repository.githubRepo`)
// get their commit/PR/issue counts from the GitHub API (via the Rust
// `github_repo_activity` command — the token stays in the OS keyring)
// instead of relay NIP-34 events, since GitHub-hosted repos aren't mirrored
// on the relay and would otherwise always aggregate to zero. Buzz-native
// repositories are unaffected and keep their relay-derived counts.

import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import {
  getGithubConnectionStatus,
  getGithubRepoActivity,
  type GithubRepoActivity,
} from "@/shared/api/tauriGithub";
import type { GithubRepoRef, Repository } from "./projectModels";

type GithubTrackedRepository = {
  repoAddress: string;
  githubRepo: GithubRepoRef;
};

/**
 * Fetch activity counts for GitHub-hosted repositories (one API call group
 * per repo, concurrently). Returns activity keyed by repoAddress. When
 * GitHub is not connected this resolves empty — not-connected is the normal
 * state for viewers who haven't linked GitHub, not a load failure. A single
 * repo's fetch failure (rate limit, 404, network) is swallowed so it doesn't
 * blank out the rest.
 */
export async function fetchGithubRepoActivitiesForRepos(
  repos: GithubTrackedRepository[],
  fetchActivity: typeof getGithubRepoActivity = getGithubRepoActivity,
  getConnectionStatus: typeof getGithubConnectionStatus = getGithubConnectionStatus,
): Promise<Map<string, GithubRepoActivity>> {
  const result = new Map<string, GithubRepoActivity>();
  if (repos.length === 0) return result;
  const status = await getConnectionStatus().catch(() => null);
  if (!status?.connected) return result;

  await Promise.all(
    repos.map(async (repo) => {
      try {
        const activity = await fetchActivity(
          repo.githubRepo.owner,
          repo.githubRepo.name,
        );
        result.set(repo.repoAddress, activity);
      } catch {
        // Leave this repo out of the result map — callers fall back to the
        // existing relay-derived summary for it.
      }
    }),
  );
  return result;
}

/**
 * Loads GitHub activity counts for the GitHub-linked repositories among
 * `repositories`, keyed by repoAddress. Skips the network entirely when
 * there are no GitHub-linked repos, so non-GitHub projects never pay for
 * this query. Cached for 5 minutes per repo set so navigation/re-renders
 * don't re-hit the GitHub API.
 */
export function useGithubRepoActivitiesQuery(repositories: Repository[]) {
  const githubRepos = React.useMemo(
    () =>
      repositories
        .filter(
          (
            repository,
          ): repository is Repository & { githubRepo: GithubRepoRef } =>
            Boolean(repository.githubRepo),
        )
        .map((repository) => ({
          repoAddress: repository.repoAddress,
          githubRepo: repository.githubRepo,
        })),
    [repositories],
  );
  const repoAddresses = React.useMemo(
    () => githubRepos.map((repository) => repository.repoAddress).sort(),
    [githubRepos],
  );

  return useQuery({
    enabled: repoAddresses.length > 0,
    queryKey: ["projects", "github-activity", repoAddresses],
    queryFn: () => fetchGithubRepoActivitiesForRepos(githubRepos),
    staleTime: 5 * 60_000,
  });
}

/**
 * Overlays GitHub-sourced counts onto a relay-derived `ProjectActivitySummary`
 * for a GitHub-linked repository. `issueCount`/`prCount`/`commitCount`/
 * `updatedAt` come from GitHub; everything else (participants, latest
 * commit, activityByDay) stays relay-derived since GitHub doesn't feed the
 * contribution graph.
 */
export function applyGithubRepoActivity<
  Summary extends {
    issueCount: number;
    prCount: number;
    commitCount: number;
    updatedAt: number;
  },
>(summary: Summary, activity: GithubRepoActivity | undefined): Summary {
  if (!activity) return summary;
  return {
    ...summary,
    issueCount: activity.openIssueCount,
    prCount: activity.openPrCount,
    commitCount: activity.commitCount,
    updatedAt: Math.max(summary.updatedAt, activity.updatedAt),
  };
}

/**
 * Applies {@link applyGithubRepoActivity} across a whole repository-address
 * → summary map. Returns `relaySummaries` unchanged (same reference) when no
 * GitHub activity data is available yet, so callers can skip re-rendering.
 */
export function overlayGithubActivity<
  Summary extends {
    issueCount: number;
    prCount: number;
    commitCount: number;
    updatedAt: number;
  },
>(
  relaySummaries: Record<string, Summary>,
  githubActivityByRepository: Map<string, GithubRepoActivity> | undefined,
): Record<string, Summary> {
  if (!githubActivityByRepository) return relaySummaries;
  return Object.fromEntries(
    Object.entries(relaySummaries).map(([repoAddress, summary]) => [
      repoAddress,
      applyGithubRepoActivity(
        summary,
        githubActivityByRepository.get(repoAddress),
      ),
    ]),
  );
}
