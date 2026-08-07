import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import {
  fetchRepositoryActivitySummaries,
  type Project,
  type ProjectActivitySummary,
} from "@/features/projects/hooks";
import {
  overlayGithubActivity,
  useGithubRepoActivitiesQuery,
} from "@/features/projects/githubRepoActivities";

/**
 * Fetches repository-specific activity for the repositories in these
 * projects. GitHub-linked repositories get their issue/PR/commit counts and
 * `updatedAt` overlaid from the GitHub API (see `githubRepoActivities.ts`);
 * Buzz-native repositories keep their relay-derived counts unchanged. A
 * GitHub fetch failure for one repo silently falls back to the relay values
 * for that repo rather than breaking the query.
 */
export function useRepositoryActivitySummariesQuery(projects: Project[]) {
  const repositories = React.useMemo(
    () => [
      ...new Map(
        projects
          .flatMap((project) => project.repositories)
          .map((repository) => [repository.repoAddress, repository]),
      ).values(),
    ],
    [projects],
  );
  const repoAddresses = React.useMemo(
    () => repositories.map((repository) => repository.repoAddress).sort(),
    [repositories],
  );

  const relaySummariesQuery = useQuery({
    enabled: repoAddresses.length > 0,
    queryKey: ["projects", "activity-summaries", "repositories", repoAddresses],
    queryFn: () => fetchRepositoryActivitySummaries(repositories),
    staleTime: 30_000,
  });
  const githubActivityQuery = useGithubRepoActivitiesQuery(repositories);

  const data = React.useMemo(():
    | Record<string, ProjectActivitySummary>
    | undefined => {
    if (!relaySummariesQuery.data) return relaySummariesQuery.data;
    return overlayGithubActivity(
      relaySummariesQuery.data,
      githubActivityQuery.data,
    );
  }, [relaySummariesQuery.data, githubActivityQuery.data]);

  return { ...relaySummariesQuery, data };
}
