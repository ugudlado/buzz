import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import { WorkflowsScreen } from "@/features/workflows/ui/WorkflowsScreen";

export function MarketplaceRouteScreen() {
  const [selectedWorkflowId, setSelectedWorkflowId] = React.useState<
    string | null
  >(null);
  const channels = useChannelsQuery().data ?? [];

  return (
    <WorkflowsScreen
      channels={channels.filter((channel) => channel.isMember)}
      onCloseWorkflow={() => setSelectedWorkflowId(null)}
      onSelectWorkflow={setSelectedWorkflowId}
      selectedWorkflowId={selectedWorkflowId}
      surface="marketplace"
    />
  );
}
