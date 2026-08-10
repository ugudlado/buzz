import { useMutation, useQueryClient } from "@tanstack/react-query";

import type { Repository } from "@/features/projects/hooks";
import type { RepositoryIssueTracker } from "@/features/projects/projectModels";
import { buildRepositoryIssueTrackerTemplate } from "@/features/projects/projectRepositoryCreation";
import {
  replaceRepositoryInProjectsCache,
  republishRepositoryAnnouncement,
} from "@/features/projects/repositoryAnnouncementMutation";
import { getIdentity } from "@/shared/api/tauriIdentity";

type SetRepositoryIssueTrackerInput = {
  issueTracker: RepositoryIssueTracker;
  repository: Repository;
};

async function setRepositoryIssueTracker({
  issueTracker,
  repository,
}: SetRepositoryIssueTrackerInput): Promise<Repository> {
  const identity = await getIdentity();
  return republishRepositoryAnnouncement({
    template: buildRepositoryIssueTrackerTemplate({
      issueTracker,
      ownerPubkey: identity.pubkey,
      repository,
    }),
    repository,
    timeoutMessage: "Timed out updating the issue tracker.",
    failureMessage: "Failed to update the issue tracker.",
    unreadableMessage: "The issue tracker was updated but could not be read.",
  });
}

export function useSetRepositoryIssueTrackerMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: setRepositoryIssueTracker,
    onSuccess: (repository) => {
      replaceRepositoryInProjectsCache(queryClient, repository);
      void queryClient.invalidateQueries({
        queryKey: ["projects", "work-items"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["project", repository.id, "issues"],
      });
    },
  });
}
