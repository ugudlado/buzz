import { useMutation, useQueryClient } from "@tanstack/react-query";

import type { Repository } from "@/features/projects/hooks";
import { buildRepositoryChannelBindingTemplate } from "@/features/projects/projectRepositoryCreation";
import {
  replaceRepositoryInProjectsCache,
  republishRepositoryAnnouncement,
} from "@/features/projects/repositoryAnnouncementMutation";
import { getIdentity } from "@/shared/api/tauriIdentity";

type BindProjectRepositoryChannelInput = {
  channelId: string;
  repository: Repository;
};

async function bindProjectRepositoryChannel({
  channelId,
  repository,
}: BindProjectRepositoryChannelInput): Promise<Repository> {
  const identity = await getIdentity();
  return republishRepositoryAnnouncement({
    template: buildRepositoryChannelBindingTemplate({
      channelId,
      ownerPubkey: identity.pubkey,
      repository,
    }),
    repository,
    timeoutMessage: "Timed out repairing repository access.",
    failureMessage: "Failed to repair repository access.",
    unreadableMessage: "Repository access was repaired but could not be read.",
  });
}

export function useBindProjectRepositoryChannelMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: bindProjectRepositoryChannel,
    onSuccess: (repository) =>
      replaceRepositoryInProjectsCache(queryClient, repository),
  });
}
