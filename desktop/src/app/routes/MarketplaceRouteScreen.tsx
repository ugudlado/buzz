import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { WorkflowsScreen } from "@/features/workflows/ui/WorkflowsScreen";

export function MarketplaceRouteScreen() {
  const [selectedWorkflowId, setSelectedWorkflowId] = React.useState<
    string | null
  >(null);
  const { goWorkflow } = useAppNavigation();
  const channels = useChannelsQuery().data ?? [];

  return (
    <WorkflowsScreen
      channels={channels.filter((channel) => channel.isMember)}
      onCloseWorkflow={() => setSelectedWorkflowId(null)}
      // The marketplace surface only fetches *listed* workflows, so it cannot
      // render a detail panel for an installed-agent's hidden workflow. Route
      // to the workflows screen (which loads all member-channel workflows)
      // instead of selecting locally.
      onSelectWorkflow={(workflowId) => {
        void goWorkflow(workflowId);
      }}
      selectedWorkflowId={selectedWorkflowId}
      surface="marketplace"
    />
  );
}
