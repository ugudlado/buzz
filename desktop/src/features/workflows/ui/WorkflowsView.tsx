import { Bot, Plus, RefreshCw, Search, Zap } from "lucide-react";
import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  allWorkflowsQueryKey,
  workflowListFocusRefetchPolicy,
} from "@/features/workflows/hooks";
import {
  useManagedAgentsQuery,
  useUpdateManagedAgentMutation,
} from "@/features/agents/hooks";
import { AgentMarketplaceDialog } from "@/features/workflows/ui/AgentMarketplaceDialog";
import { WorkflowCard } from "@/features/workflows/ui/WorkflowCard";
import { WorkflowDeleteDialog } from "@/features/workflows/ui/WorkflowDeleteDialog";
import { WorkflowDetailPanel } from "@/features/workflows/ui/WorkflowDetailPanel";
import { WorkflowDialog } from "@/features/workflows/ui/WorkflowDialog";
import {
  formatMicrounits,
  getWorkflowMarketplace,
} from "@/features/workflows/marketplace";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceBadge } from "@/features/presence/ui/PresenceBadge";
import {
  getMarketplaceAgents,
  type MarketplaceAgent,
} from "@/shared/api/marketplace";
import type {
  Channel,
  ManagedAgent,
  PresenceStatus,
  Workflow,
} from "@/shared/api/types";
import type { ManagedAgentMarketplace } from "@/shared/api/marketplace";
import {
  deleteWorkflow,
  getChannelsWorkflows,
  triggerWorkflow,
} from "@/shared/api/tauriWorkflows";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
import { Card } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { PubKey } from "@/shared/ui/PubKey";
import { Skeleton } from "@/shared/ui/skeleton";

type WorkflowsViewProps = {
  channels: Channel[];
  onCloseWorkflow: () => void;
  onSelectWorkflow: (workflowId: string) => void;
  selectedWorkflowId: string | null;
  surface: "manage" | "marketplace";
};

type WorkflowWithChannel = {
  workflow: Workflow;
  channelName: string;
};

type DialogState =
  | { mode: "closed" }
  | { mode: "create" }
  | { mode: "edit"; workflow: Workflow }
  | { mode: "duplicate"; workflow: Workflow };

type CatalogTab = "agents" | "workflows";

function WorkflowsListSkeleton() {
  return (
    <div className="space-y-2">
      {["first", "second", "third", "fourth"].map((card) => (
        <Card className="p-4" key={card}>
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0 flex-1 space-y-3">
              <div className="flex items-center gap-2">
                <Skeleton className="h-5 w-44" />
                <Skeleton className="h-5 w-16 rounded-full" />
              </div>
              <Skeleton className="h-4 w-full max-w-2xl" />
              <div className="flex flex-wrap gap-2">
                <Skeleton className="h-5 w-20 rounded-full" />
                <Skeleton className="h-5 w-24 rounded-full" />
                <Skeleton className="h-5 w-16 rounded-full" />
              </div>
            </div>
            <div className="hidden shrink-0 gap-2 sm:flex">
              <Skeleton className="h-8 w-8 rounded-lg" />
              <Skeleton className="h-8 w-8 rounded-lg" />
            </div>
          </div>
        </Card>
      ))}
    </div>
  );
}

function AgentCatalogCard({
  agent,
  presenceStatus,
  localAgent,
  onEdit,
  onUnpublish,
}: {
  agent: MarketplaceAgent;
  presenceStatus: PresenceStatus | null;
  localAgent?: ManagedAgent;
  onEdit: (agent: ManagedAgent) => void;
  onUnpublish: (agent: ManagedAgent) => void;
}) {
  return (
    <Card className="p-4" data-testid={`marketplace-agent-${agent.pubkey}`}>
      <div className="flex items-start gap-3">
        <div className="mt-0.5 rounded-md bg-primary/10 p-2 text-primary">
          <Bot className="h-4 w-4" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-sm font-semibold">{agent.name}</h3>
            {presenceStatus ? (
              <PresenceBadge
                className="px-2 py-0.5 text-2xs"
                label={getPresenceLabel(presenceStatus)}
                status={presenceStatus}
              />
            ) : (
              <Badge variant="secondary">Presence unavailable</Badge>
            )}
            <Badge variant="secondary">{agent.deployment}</Badge>
          </div>
          {agent.description ? (
            <p className="mt-2 text-xs text-muted-foreground">
              {agent.description}
            </p>
          ) : null}
          <div className="mt-3 flex flex-wrap gap-1.5">
            {agent.capabilities.map((capability) => (
              <Badge key={capability} variant="outline">
                {capability}
              </Badge>
            ))}
          </div>
          <div className="mt-3 grid gap-2 border-t pt-3 text-xs sm:grid-cols-2">
            <div>
              <p className="text-2xs text-muted-foreground">Hourly rate</p>
              <p>
                {agent.pricing
                  ? `${formatMicrounits(
                      agent.pricing.currency,
                      agent.pricing.microunitsPerHour,
                    )}/hour`
                  : "Unpriced"}
              </p>
            </div>
            <div>
              <p className="text-2xs text-muted-foreground">Availability</p>
              <p>
                Workflows · Direct use{" "}
                {agent.directUse.replace("owner", "owner only")}
              </p>
            </div>
            <div className="sm:col-span-2">
              <p className="text-2xs text-muted-foreground">Publisher</p>
              <PubKey pubkey={agent.ownerPubkey} />
            </div>
          </div>
          {localAgent ? (
            <div className="mt-3 flex gap-2">
              <Button
                onClick={() => onEdit(localAgent)}
                size="sm"
                variant="outline"
              >
                Edit listing
              </Button>
              <Button
                onClick={() => onUnpublish(localAgent)}
                size="sm"
                variant="ghost"
              >
                Unpublish
              </Button>
            </div>
          ) : null}
        </div>
      </div>
    </Card>
  );
}

export function WorkflowsView({
  channels,
  onCloseWorkflow,
  onSelectWorkflow,
  selectedWorkflowId,
  surface,
}: WorkflowsViewProps) {
  const [dialogState, setDialogState] = React.useState<DialogState>({
    mode: "closed",
  });
  const [deleteTarget, setDeleteTarget] = React.useState<Workflow | null>(null);
  const [catalogTab, setCatalogTab] = React.useState<CatalogTab>("workflows");
  const [agentSearch, setAgentSearch] = React.useState("");
  const isMarketplace = surface === "marketplace";
  const [listingAgent, setListingAgent] = React.useState<ManagedAgent | null>(
    null,
  );
  const queryClient = useQueryClient();
  const identityPubkey = useIdentityQuery().data?.pubkey.toLowerCase() ?? null;
  const managedAgentsQuery = useManagedAgentsQuery();
  const updateManagedAgent = useUpdateManagedAgentMutation();
  const updateManagedAgentMutate = updateManagedAgent.mutate;

  const marketplaceAgentsQuery = useQuery({
    queryKey: ["marketplace-agents"],
    queryFn: getMarketplaceAgents,
    staleTime: 30_000,
    refetchOnWindowFocus: true,
  });
  const marketplaceAgents = marketplaceAgentsQuery.data ?? [];
  const managedAgents = managedAgentsQuery.data ?? [];
  const managedAgentByPubkey = React.useMemo(
    () =>
      new Map(
        managedAgents.map((agent) => [agent.pubkey.toLowerCase(), agent]),
      ),
    [managedAgents],
  );
  const searchTerm = agentSearch.trim().toLowerCase();
  const filteredMarketplaceAgents = marketplaceAgents.filter(
    (agent) =>
      !searchTerm ||
      [agent.name, agent.pubkey, agent.description, ...agent.capabilities].some(
        (value) => value.toLowerCase().includes(searchTerm),
      ),
  );
  const filteredUnpublishedAgents = managedAgents.filter(
    (agent) =>
      !marketplaceAgents.some(
        (listing) => listing.pubkey === agent.pubkey.toLowerCase(),
      ) &&
      (!searchTerm ||
        [agent.name, agent.pubkey].some((value) =>
          value.toLowerCase().includes(searchTerm),
        )),
  );
  const marketplaceAgentPubkeys = React.useMemo(
    () => marketplaceAgents.map((agent) => agent.pubkey),
    [marketplaceAgents],
  );
  const presenceQuery = usePresenceQuery(marketplaceAgentPubkeys);

  const memberChannels = channels.filter((c) => c.isMember);
  const channelIds = memberChannels.map((c) => c.id).sort();
  const channelIdKey = channelIds.join(",");

  const allWorkflowsQuery = useQuery({
    queryKey: allWorkflowsQueryKey(channelIdKey),
    queryFn: async () => {
      // Single batched relay query for all member channels, then group by the
      // channel_id each workflow carries — replaces the per-channel fanout.
      const channelNameById = new Map(
        memberChannels.map((channel) => [channel.id, channel.name]),
      );
      const workflows = await getChannelsWorkflows(channelIds);
      const results: WorkflowWithChannel[] = [];
      for (const workflow of workflows) {
        results.push({
          workflow,
          channelName: workflow.channelId
            ? (channelNameById.get(workflow.channelId) ?? "")
            : "",
        });
      }
      return results;
    },
    enabled: memberChannels.length > 0,
    ...workflowListFocusRefetchPolicy,
  });

  const allWorkflows = allWorkflowsQuery.data ?? [];
  const visibleWorkflows = isMarketplace
    ? allWorkflows.filter(({ workflow }) =>
        getWorkflowMarketplace(workflow.definition),
      )
    : allWorkflows;

  const triggerMutation = useMutation({
    mutationFn: (workflowId: string) => triggerWorkflow(workflowId),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        predicate: (query) => query.queryKey[0] === "workflow-runs",
      });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (workflowId: string) => deleteWorkflow(workflowId),
    onSuccess: (_data, workflowId) => {
      if (selectedWorkflowId === workflowId) {
        onCloseWorkflow();
      }
      void queryClient.invalidateQueries({
        predicate: (query) =>
          query.queryKey[0] === "workflows" ||
          query.queryKey[0] === "workflows-all",
      });
    },
  });

  const triggerOne = triggerMutation.mutate;
  const handleTrigger = React.useCallback(
    (workflowId: string) => triggerOne(workflowId),
    [triggerOne],
  );

  const handleDelete = React.useCallback(
    (workflow: Workflow) => setDeleteTarget(workflow),
    [],
  );

  const deleteOne = deleteMutation.mutate;
  const handleConfirmDelete = React.useCallback(
    (workflow: Workflow) => {
      deleteOne(workflow.id);
      setDeleteTarget(null);
    },
    [deleteOne],
  );

  const handleEdit = React.useCallback(
    (workflow: Workflow) => setDialogState({ mode: "edit", workflow }),
    [],
  );

  const handleDuplicate = React.useCallback(
    (workflow: Workflow) => setDialogState({ mode: "duplicate", workflow }),
    [],
  );

  const handleDialogOpenChange = React.useCallback((open: boolean) => {
    if (!open) {
      setDialogState({ mode: "closed" });
    }
  }, []);

  const saveAgentListing = React.useCallback(
    (marketplace: ManagedAgentMarketplace) => {
      if (!listingAgent) return;
      updateManagedAgentMutate(
        { pubkey: listingAgent.pubkey, marketplace },
        {
          onSuccess: () => {
            setListingAgent(null);
            void marketplaceAgentsQuery.refetch();
          },
        },
      );
    },
    [listingAgent, marketplaceAgentsQuery.refetch, updateManagedAgentMutate],
  );

  const unpublishAgent = React.useCallback(
    (agent: ManagedAgent) => {
      const current = agent.marketplace;
      if (!current) return;
      updateManagedAgentMutate(
        { pubkey: agent.pubkey, marketplace: { ...current, listed: false } },
        { onSuccess: () => void marketplaceAgentsQuery.refetch() },
      );
    },
    [marketplaceAgentsQuery.refetch, updateManagedAgentMutate],
  );

  return (
    <div
      className="relative flex min-h-0 flex-1 overflow-hidden"
      data-testid="workflows-view"
    >
      <div
        className="flex min-h-0 flex-1 flex-col overflow-y-auto px-4 pb-4 pt-4"
        data-scroll-restoration-id="workflows-list"
      >
        <div className="mb-4 flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <h2 className="text-lg font-semibold">
              {isMarketplace ? "Marketplace" : "Workflows"}
            </h2>
            <Button
              aria-label={`Refresh ${catalogTab}`}
              disabled={
                isMarketplace && catalogTab === "agents"
                  ? marketplaceAgentsQuery.isFetching
                  : allWorkflowsQuery.isFetching
              }
              onClick={() =>
                void (isMarketplace && catalogTab === "agents"
                  ? marketplaceAgentsQuery.refetch()
                  : allWorkflowsQuery.refetch())
              }
              size="icon"
              variant="ghost"
            >
              <RefreshCw
                className={`h-4 w-4 ${
                  (
                    isMarketplace && catalogTab === "agents"
                      ? marketplaceAgentsQuery.isFetching
                      : allWorkflowsQuery.isFetching
                  )
                    ? "animate-spin"
                    : ""
                }`}
              />
            </Button>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {isMarketplace ? (
              <div className="flex rounded-lg border bg-muted/30 p-0.5">
                <Button
                  onClick={() => setCatalogTab("agents")}
                  size="sm"
                  variant={catalogTab === "agents" ? "secondary" : "ghost"}
                >
                  Agents
                </Button>
                <Button
                  onClick={() => setCatalogTab("workflows")}
                  size="sm"
                  variant={catalogTab === "workflows" ? "secondary" : "ghost"}
                >
                  Workflows
                </Button>
              </div>
            ) : (
              <Button
                onClick={() => setDialogState({ mode: "create" })}
                size="sm"
              >
                <Plus className="mr-1 h-4 w-4" />
                Create Workflow
              </Button>
            )}
          </div>
        </div>

        {isMarketplace && catalogTab === "agents" ? (
          marketplaceAgentsQuery.isLoading ? (
            <WorkflowsListSkeleton />
          ) : marketplaceAgentsQuery.isError ? (
            <div className="flex flex-1 flex-col items-center justify-center gap-2 text-muted-foreground">
              <p className="text-sm text-red-400">Failed to load agents</p>
              <Button
                onClick={() => void marketplaceAgentsQuery.refetch()}
                size="sm"
                variant="outline"
              >
                Retry
              </Button>
            </div>
          ) : (
            <div className="space-y-4">
              <div className="relative">
                <Search className="absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
                <Input
                  aria-label="Search agents"
                  className="pl-9"
                  onChange={(event) => setAgentSearch(event.target.value)}
                  placeholder="Search agents"
                  type="search"
                  value={agentSearch}
                />
              </div>
              {filteredMarketplaceAgents.length === 0 &&
              filteredUnpublishedAgents.length === 0 ? (
                <div className="flex flex-col items-center justify-center gap-3 py-10 text-muted-foreground">
                  <Bot className="h-10 w-10 opacity-30" />
                  <p className="text-sm">
                    {searchTerm
                      ? "No agents match your search"
                      : "No agents available"}
                  </p>
                </div>
              ) : (
                <>
                  {filteredMarketplaceAgents.map((agent) => (
                    <AgentCatalogCard
                      agent={agent}
                      key={agent.pubkey}
                      localAgent={managedAgentByPubkey.get(agent.pubkey)}
                      onEdit={setListingAgent}
                      onUnpublish={unpublishAgent}
                      presenceStatus={
                        presenceQuery.isSuccess
                          ? (presenceQuery.data?.[agent.pubkey] ?? "offline")
                          : null
                      }
                    />
                  ))}
                  {filteredUnpublishedAgents.length > 0 ? (
                    <div className="space-y-2 border-t pt-4">
                      <p className="text-xs font-medium text-muted-foreground">
                        Your unpublished agents
                      </p>
                      {filteredUnpublishedAgents.map((agent) => (
                        <Card
                          className="flex items-center justify-between gap-3 p-4"
                          data-testid={`unpublished-agent-${agent.pubkey}`}
                          key={agent.pubkey}
                        >
                          <div className="min-w-0">
                            <p className="truncate text-sm font-medium">
                              {agent.name}
                            </p>
                            <PubKey pubkey={agent.pubkey} />
                          </div>
                          <Button
                            onClick={() => setListingAgent(agent)}
                            size="sm"
                          >
                            Publish
                          </Button>
                        </Card>
                      ))}
                    </div>
                  ) : null}
                </>
              )}
            </div>
          )
        ) : allWorkflowsQuery.isLoading ? (
          <WorkflowsListSkeleton />
        ) : allWorkflowsQuery.isError ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 text-muted-foreground">
            <p className="text-sm text-red-400">Failed to load workflows</p>
            <Button
              onClick={() => void allWorkflowsQuery.refetch()}
              size="sm"
              variant="outline"
            >
              Retry
            </Button>
          </div>
        ) : visibleWorkflows.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 text-muted-foreground">
            <Zap className="h-10 w-10 opacity-30" />
            <p className="text-sm">
              {isMarketplace ? "No listed workflows" : "No workflows yet"}
            </p>
            {!isMarketplace ? (
              <Button
                onClick={() => setDialogState({ mode: "create" })}
                size="sm"
                variant="outline"
              >
                <Plus className="mr-1 h-4 w-4" />
                Create your first workflow
              </Button>
            ) : null}
          </div>
        ) : (
          <div className="space-y-2">
            {visibleWorkflows.map(({ workflow, channelName }) => (
              <WorkflowCard
                canManage={
                  identityPubkey === workflow.ownerPubkey.toLowerCase()
                }
                channelName={channelName}
                isActive={selectedWorkflowId === workflow.id}
                key={workflow.id}
                onDelete={handleDelete}
                onDuplicate={handleDuplicate}
                onEdit={handleEdit}
                onSelect={onSelectWorkflow}
                onTrigger={handleTrigger}
                marketplaceAgents={marketplaceAgents}
                presence={presenceQuery.data}
                presenceLoaded={presenceQuery.isSuccess}
                workflow={workflow}
              />
            ))}
          </div>
        )}
      </div>

      {selectedWorkflowId ? (
        <div className="w-[400px] shrink-0">
          <WorkflowDetailPanel
            canManage={
              allWorkflows
                .find(({ workflow }) => workflow.id === selectedWorkflowId)
                ?.workflow.ownerPubkey.toLowerCase() === identityPubkey
            }
            key={selectedWorkflowId}
            marketplaceAgents={marketplaceAgents}
            onClose={onCloseWorkflow}
            onEdit={handleEdit}
            workflowId={selectedWorkflowId}
          />
        </div>
      ) : null}

      <WorkflowDialog
        channels={memberChannels}
        mode={dialogState.mode === "closed" ? "create" : dialogState.mode}
        onOpenChange={handleDialogOpenChange}
        open={dialogState.mode !== "closed"}
        workflow={
          dialogState.mode === "edit" || dialogState.mode === "duplicate"
            ? dialogState.workflow
            : null
        }
      />

      <WorkflowDeleteDialog
        onConfirm={handleConfirmDelete}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
        open={deleteTarget !== null}
        workflow={deleteTarget}
      />
      <AgentMarketplaceDialog
        agent={listingAgent}
        onOpenChange={(open) => {
          if (!open) setListingAgent(null);
        }}
        onSave={saveAgentListing}
        open={listingAgent !== null}
        pending={updateManagedAgent.isPending}
      />
    </div>
  );
}
