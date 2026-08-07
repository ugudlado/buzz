import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  type Project,
  projectsQueryKey,
  type Repository,
} from "@/features/projects/hooks";
import {
  eventToRepository,
  type RepositoryIssueTracker,
} from "@/features/projects/projectModels";
import { buildRepositoryIssueTrackerTemplate } from "@/features/projects/projectRepositoryCreation";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";

type SetRepositoryIssueTrackerInput = {
  issueTracker: RepositoryIssueTracker;
  repository: Repository;
};

async function setRepositoryIssueTracker({
  issueTracker,
  repository,
}: SetRepositoryIssueTrackerInput): Promise<Repository> {
  const identity = await getIdentity();
  const template = buildRepositoryIssueTrackerTemplate({
    issueTracker,
    ownerPubkey: identity.pubkey,
    repository,
  });
  const event = await signRelayEvent({
    ...template,
    createdAt: Math.max(
      Math.floor(Date.now() / 1_000),
      repository.createdAt + 1,
    ),
  });
  await relayClient.publishEvent(
    event,
    "Timed out updating the issue tracker.",
    "Failed to update the issue tracker.",
  );

  const updated = eventToRepository(event, getCachedRelayOrigin());
  if (!updated) {
    throw new Error("The issue tracker was updated but could not be read.");
  }
  return updated;
}

export function useSetRepositoryIssueTrackerMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: setRepositoryIssueTracker,
    onSuccess: (repository) => {
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
      void queryClient.invalidateQueries({
        queryKey: ["projects", "work-items"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["project", repository.id, "issues"],
      });
    },
  });
}
