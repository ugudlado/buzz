import type { Project } from "@/features/projects/projectModels";
import type { ProjectActivitySummary } from "@/features/projects/hooks";

/**
 * Aggregates per-repository activity summaries into per-project summaries.
 * Pure function of `summariesByRepository` so callers can pass in
 * GitHub-overlaid summaries (see `useProjectActivitySummariesQuery`) without
 * duplicating this reduction logic.
 */
export function aggregateProjectActivitySummaries(
  projects: Project[],
  summariesByRepository: Record<string, ProjectActivitySummary>,
): Record<string, ProjectActivitySummary> {
  return Object.fromEntries(
    projects.map((project) => {
      const summaries = project.repositories.map(
        (repository) => summariesByRepository[repository.repoAddress],
      );
      const latestCommit =
        summaries
          .map((summary) => summary?.latestCommit)
          .filter(
            (
              commit,
            ): commit is NonNullable<ProjectActivitySummary["latestCommit"]> =>
              Boolean(commit),
          )
          .sort((left, right) => right.createdAt - left.createdAt)[0] ?? null;
      const activityByDay: Record<string, number> = {};
      for (const summary of summaries) {
        for (const [day, count] of Object.entries(
          summary?.activityByDay ?? {},
        )) {
          activityByDay[day] = (activityByDay[day] ?? 0) + count;
        }
      }
      return [
        project.id,
        {
          repoAddress: project.projectAddress,
          issueCount: summaries.reduce(
            (count, summary) => count + (summary?.issueCount ?? 0),
            0,
          ),
          prCount: summaries.reduce(
            (count, summary) => count + (summary?.prCount ?? 0),
            0,
          ),
          commitCount: summaries.reduce(
            (count, summary) => count + (summary?.commitCount ?? 0),
            0,
          ),
          activityCount: summaries.reduce(
            (count, summary) => count + (summary?.activityCount ?? 0),
            0,
          ),
          updatedAt: Math.max(
            0,
            ...summaries.map((summary) => summary?.updatedAt ?? 0),
          ),
          participantPubkeys: [
            ...new Set(
              summaries.flatMap((summary) => summary?.participantPubkeys ?? []),
            ),
          ],
          latestCommit,
          activityByDay,
        } satisfies ProjectActivitySummary,
      ];
    }),
  );
}
