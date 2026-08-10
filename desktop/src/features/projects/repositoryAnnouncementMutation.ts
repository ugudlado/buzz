import type { QueryClient } from "@tanstack/react-query";

import {
  type Project,
  projectsQueryKey,
  type Repository,
} from "@/features/projects/hooks";
import { eventToRepository } from "@/features/projects/projectModels";
import type { ProjectEventTemplate } from "@/features/projects/projectCreation";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";

/**
 * Sign and publish a rebuilt repository announcement (kind:30617 replacement),
 * returning the repository parsed back from the published event. The
 * `createdAt` bump guarantees the replacement supersedes the current head.
 */
export async function republishRepositoryAnnouncement({
  failureMessage,
  repository,
  template,
  timeoutMessage,
  unreadableMessage,
}: {
  failureMessage: string;
  repository: Repository;
  template: ProjectEventTemplate;
  timeoutMessage: string;
  unreadableMessage: string;
}): Promise<Repository> {
  const event = await signRelayEvent({
    ...template,
    createdAt: Math.max(
      Math.floor(Date.now() / 1_000),
      repository.createdAt + 1,
    ),
  });
  await relayClient.publishEvent(event, timeoutMessage, failureMessage);

  const updated = eventToRepository(event, getCachedRelayOrigin());
  if (!updated) {
    throw new Error(unreadableMessage);
  }
  return updated;
}

/** Replace one repository across every cached project, then refetch. */
export function replaceRepositoryInProjectsCache(
  queryClient: QueryClient,
  repository: Repository,
): void {
  queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) =>
    current.map((project) => ({
      ...project,
      repositories: project.repositories.map((candidate) =>
        candidate.repoAddress === repository.repoAddress
          ? repository
          : candidate,
      ),
    })),
  );
  void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
}
