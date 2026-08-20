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
import { useCommunities } from "@/features/communities/useCommunities";
import { AgentCatalogCard } from "@/features/workflows/ui/AgentCatalogCard";
import {
  AskAgentDialog,
  InstallAgentDialog,
} from "@/features/workflows/ui/MarketplaceAgentDialogs";
import {
  agentCoordinateKey,
  marketplaceAgentFromLocal,
  marketplaceFromListing,
  remotePolicyAllows,
} from "@/features/workflows/ui/marketplaceAgentCatalog";
import { AgentMarketplaceDialog } from "@/features/workflows/ui/AgentMarketplaceDialog";
import { AgentJobsDialog } from "@/features/workflows/ui/AgentJobsDialog";
import { WorkflowCard } from "@/features/workflows/ui/WorkflowCard";
import { WorkflowDeleteDialog } from "@/features/workflows/ui/WorkflowDeleteDialog";
import { WorkflowDetailPanel } from "@/features/workflows/ui/WorkflowDetailPanel";
import { WorkflowDialog } from "@/features/workflows/ui/WorkflowDialog";
import { MarketplaceWorkflowCard } from "@/features/workflows/ui/MarketplaceWorkflowCard";
import {
  getInstalledRemoteAgent,
  getWorkflowMarketplace,
  installMarketplaceWorkflowSnapshot,
} from "@/features/workflows/marketplace";
import {
  PRESENCE_REFETCH_INTERVAL_MS,
  presenceFocusRefetchPolicy,
  usePresenceQuery,
} from "@/features/presence/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import {
  getMarketplaceAgents,
  getMarketplaceWorkflows,
  marketplaceAgentQueryKey,
  marketplacePresenceTargets,
  marketplaceWorkflowQueryKey,
  type MarketplaceAgent,
  type MarketplaceWorkflow,
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
import { getPresence } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
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
  | {
      mode: "create";
      initialDefinition?: Record<string, unknown>;
    }
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
  const [workflowSearch, setWorkflowSearch] = React.useState("");
  const [listingOverrides, setListingOverrides] = React.useState(
    () => new Map<string, ManagedAgentMarketplace | null>(),
  );
  const isMarketplace = surface === "marketplace";
  const [listingAgent, setListingAgent] = React.useState<ManagedAgent | null>(
    null,
  );
  const [installTarget, setInstallTarget] =
    React.useState<MarketplaceAgent | null>(null);
  const [askTarget, setAskTarget] = React.useState<{
    workflow: Workflow;
    agentName: string;
  } | null>(null);
  const [earningsTarget, setEarningsTarget] = React.useState<{
    pubkey: string;
    name: string;
  } | null>(null);
  const showEarnings = React.useCallback(
    (pubkey: string, name: string) => setEarningsTarget({ pubkey, name }),
    [],
  );
  const queryClient = useQueryClient();
  const { activeCommunity, communities } = useCommunities();
  const identityPubkey = useIdentityQuery().data?.pubkey.toLowerCase() ?? null;
  const activeRelaySelf = useRelaySelfQuery(isMarketplace).data ?? null;
  const managedAgentsQuery = useManagedAgentsQuery();
  const updateManagedAgent = useUpdateManagedAgentMutation();
  const updateManagedAgentMutate = updateManagedAgent.mutate;

  const marketplaceAgentsQuery = useQuery({
    queryKey: marketplaceAgentQueryKey(communities),
    queryFn: () =>
      getMarketplaceAgents(communities, activeCommunity?.relayUrl ?? ""),
    enabled: Boolean(activeCommunity),
    staleTime: 30_000,
    refetchOnWindowFocus: true,
  });
  const marketplaceWorkflowsQuery = useQuery({
    queryKey: marketplaceWorkflowQueryKey(communities),
    queryFn: () =>
      getMarketplaceWorkflows(communities, activeCommunity?.relayUrl ?? ""),
    enabled: isMarketplace && Boolean(activeCommunity),
    staleTime: 30_000,
    refetchOnWindowFocus: true,
  });
  const relayMarketplaceAgents = marketplaceAgentsQuery.data ?? [];
  const managedAgents = managedAgentsQuery.data ?? [];
  const managedAgentByPubkey = React.useMemo(
    () =>
      new Map(
        managedAgents.map((agent) => [agent.pubkey.toLowerCase(), agent]),
      ),
    [managedAgents],
  );
  const marketplaceAgents = React.useMemo(() => {
    const merged = relayMarketplaceAgents.filter(
      (agent) =>
        agent.sourceCommunity?.relayUrl !== activeCommunity?.relayUrl ||
        agent.ownerPubkey !== identityPubkey ||
        !listingOverrides.has(agent.pubkey),
    );
    if (!identityPubkey || !activeCommunity) return merged;
    for (const [pubkey, marketplace] of listingOverrides) {
      const localAgent = managedAgentByPubkey.get(pubkey);
      if (localAgent && marketplace?.listed) {
        merged.push(
          marketplaceAgentFromLocal(localAgent, identityPubkey, marketplace, {
            id: activeCommunity.id,
            name: activeCommunity.name,
            relayUrl: activeCommunity.relayUrl,
            relayPubkey: activeRelaySelf ?? undefined,
          }),
        );
      }
    }
    return merged.sort((left, right) => left.name.localeCompare(right.name));
  }, [
    activeCommunity,
    activeRelaySelf,
    identityPubkey,
    listingOverrides,
    managedAgentByPubkey,
    relayMarketplaceAgents,
  ]);
  const agentSearchTerm = agentSearch.trim().toLowerCase();
  const filteredMarketplaceAgents = marketplaceAgents.filter(
    (agent) =>
      !agentSearchTerm ||
      [
        agent.name,
        agent.pubkey,
        agent.ownerPubkey,
        agent.description,
        agent.sourceCommunity?.name ?? "",
        ...agent.capabilities,
      ].some((value) => value.toLowerCase().includes(agentSearchTerm)),
  );
  const filteredUnpublishedAgents = managedAgents.filter(
    (agent) =>
      !marketplaceAgents.some(
        (listing) =>
          listing.ownerPubkey === identityPubkey &&
          listing.sourceCommunity?.relayUrl === activeCommunity?.relayUrl &&
          listing.pubkey === agent.pubkey.toLowerCase(),
      ) &&
      (!agentSearchTerm ||
        [agent.name, agent.pubkey].some((value) =>
          value.toLowerCase().includes(agentSearchTerm),
        )),
  );
  const activeMarketplaceAgents = React.useMemo(
    () =>
      marketplaceAgents.filter(
        (agent) =>
          agent.sourceCommunity?.relayUrl === activeCommunity?.relayUrl,
      ),
    [activeCommunity?.relayUrl, marketplaceAgents],
  );
  const marketplaceAgentPubkeys = React.useMemo(
    () => activeMarketplaceAgents.map((agent) => agent.pubkey),
    [activeMarketplaceAgents],
  );
  const presenceQuery = usePresenceQuery(marketplaceAgentPubkeys);
  const remotePresenceTargets = React.useMemo(
    () =>
      marketplacePresenceTargets(marketplaceAgents, activeCommunity?.relayUrl),
    [activeCommunity?.relayUrl, marketplaceAgents],
  );
  const remotePresenceQuery = useQuery({
    queryKey: [
      "marketplace-presence",
      ...remotePresenceTargets.map(
        ({ relayUrl, pubkeys }) => `${relayUrl}:${pubkeys.join(",")}`,
      ),
    ],
    queryFn: async () =>
      Object.fromEntries(
        await Promise.all(
          remotePresenceTargets.map(async ({ relayUrl, pubkeys }) => [
            relayUrl,
            await getPresence(pubkeys, relayUrl).catch(() => null),
          ]),
        ),
      ),
    enabled: remotePresenceTargets.length > 0,
    refetchInterval: PRESENCE_REFETCH_INTERVAL_MS,
    retry: 0,
    ...presenceFocusRefetchPolicy,
  });
  const marketplacePresenceStatus = (
    agent: MarketplaceAgent,
  ): PresenceStatus | null => {
    const sourceRelayUrl = agent.sourceCommunity?.relayUrl;
    const presence =
      sourceRelayUrl === activeCommunity?.relayUrl
        ? presenceQuery.isSuccess
          ? presenceQuery.data
          : null
        : sourceRelayUrl
          ? remotePresenceQuery.data?.[sourceRelayUrl]
          : null;
    return presence ? (presence[agent.pubkey] ?? "offline") : null;
  };

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
    // The marketplace surface needs the workflow list too — installed
    // remote agents are represented by hidden workflows.
    enabled: memberChannels.length > 0,
    ...workflowListFocusRefetchPolicy,
  });

  const allWorkflows = allWorkflowsQuery.data ?? [];
  // Hidden installed-agent workflows surface as agent cards, not workflows.
  const installedAgentWorkflows = React.useMemo(() => {
    const byCoordinate = new Map<string, Workflow>();
    for (const { workflow } of allWorkflows) {
      const installed = getInstalledRemoteAgent(workflow.definition);
      if (installed) {
        byCoordinate.set(
          agentCoordinateKey(installed.relayPubkey, installed.pubkey),
          workflow,
        );
      }
    }
    return byCoordinate;
  }, [allWorkflows]);
  const workflowSearchTerm = workflowSearch.trim().toLowerCase();
  const filteredVisibleWorkflows = allWorkflows.filter(
    ({ workflow, channelName }) =>
      getInstalledRemoteAgent(workflow.definition) === null &&
      (!workflowSearchTerm ||
        [
          workflow.name,
          workflow.ownerPubkey,
          channelName,
          getWorkflowMarketplace(workflow.definition)?.summary ?? "",
        ].some((value) => value.toLowerCase().includes(workflowSearchTerm))),
  );
  const marketplaceWorkflows = marketplaceWorkflowsQuery.data ?? [];
  const filteredMarketplaceWorkflows = marketplaceWorkflows.filter(
    (workflow) =>
      !workflowSearchTerm ||
      [
        workflow.name,
        workflow.ownerPubkey,
        workflow.sourceCommunity.name,
        getWorkflowMarketplace(workflow.definition)?.summary ?? "",
      ].some((value) => value.toLowerCase().includes(workflowSearchTerm)),
  );
  const workflowListIsFetching = isMarketplace
    ? marketplaceWorkflowsQuery.isFetching
    : allWorkflowsQuery.isFetching;
  const workflowListIsLoading = isMarketplace
    ? marketplaceWorkflowsQuery.isLoading
    : allWorkflowsQuery.isLoading;
  const workflowListIsError = isMarketplace
    ? marketplaceWorkflowsQuery.isError
    : allWorkflowsQuery.isError;

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

  const askAgent = React.useCallback(
    (workflow: Workflow, agentName: string) => {
      setAskTarget({ workflow, agentName });
    },
    [],
  );

  const useWorkflowHere = React.useCallback((workflow: MarketplaceWorkflow) => {
    const initialDefinition = installMarketplaceWorkflowSnapshot(workflow);
    if (initialDefinition) {
      setDialogState({ mode: "create", initialDefinition });
    }
  }, []);

  const saveAgentListing = React.useCallback(
    (marketplace: ManagedAgentMarketplace) => {
      if (!listingAgent) return;
      updateManagedAgentMutate(
        { pubkey: listingAgent.pubkey, marketplace },
        {
          onSuccess: () => {
            setListingOverrides((current) =>
              new Map(current).set(
                listingAgent.pubkey.toLowerCase(),
                marketplace,
              ),
            );
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
        {
          onSuccess: () => {
            setListingOverrides((overrides) =>
              new Map(overrides).set(agent.pubkey.toLowerCase(), null),
            );
            void marketplaceAgentsQuery.refetch();
          },
        },
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
                  : workflowListIsFetching
              }
              onClick={() =>
                void (isMarketplace && catalogTab === "agents"
                  ? marketplaceAgentsQuery.refetch()
                  : isMarketplace
                    ? marketplaceWorkflowsQuery.refetch()
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
                      : workflowListIsFetching
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

        {isMarketplace && catalogTab === "workflows" ? (
          <div className="relative mb-4">
            <Search className="absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              aria-label="Search published workflows"
              className="pl-9"
              onChange={(event) => setWorkflowSearch(event.target.value)}
              placeholder="Search published workflows"
              type="search"
              value={workflowSearch}
            />
          </div>
        ) : null}

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
              <p className="text-xs text-muted-foreground">
                Published listings from every community configured in this app.
              </p>
              {filteredMarketplaceAgents.length === 0 &&
              filteredUnpublishedAgents.length === 0 ? (
                <div className="flex flex-col items-center justify-center gap-3 py-10 text-muted-foreground">
                  <Bot className="h-10 w-10 opacity-30" />
                  <p className="text-sm">
                    {agentSearchTerm
                      ? "No agents match your search"
                      : "No agents available"}
                  </p>
                </div>
              ) : (
                <>
                  {filteredMarketplaceAgents.map((agent) => (
                    <AgentCatalogCard
                      agent={agent}
                      canInstall={
                        agent.sourceCommunity?.relayUrl !==
                          activeCommunity?.relayUrl &&
                        Boolean(agent.sourceCommunity?.relayPubkey) &&
                        remotePolicyAllows(agent, activeRelaySelf)
                      }
                      installedWorkflow={
                        agent.sourceCommunity?.relayPubkey
                          ? (installedAgentWorkflows.get(
                              agentCoordinateKey(
                                agent.sourceCommunity.relayPubkey,
                                agent.pubkey,
                              ),
                            ) ?? null)
                          : null
                      }
                      key={`${agent.sourceCommunity?.relayUrl}:${agent.ownerPubkey}:${agent.pubkey}`}
                      localAgent={
                        agent.ownerPubkey === identityPubkey &&
                        agent.sourceCommunity?.relayUrl ===
                          activeCommunity?.relayUrl
                          ? (() => {
                              const local = managedAgentByPubkey.get(
                                agent.pubkey,
                              );
                              return local
                                ? {
                                    ...local,
                                    marketplace: marketplaceFromListing(agent),
                                  }
                                : undefined;
                            })()
                          : undefined
                      }
                      onEdit={setListingAgent}
                      onInstall={setInstallTarget}
                      onAsk={askAgent}
                      onRemove={handleDelete}
                      onShowRuns={onSelectWorkflow}
                      onShowEarnings={showEarnings}
                      onUnpublish={unpublishAgent}
                      presenceStatus={marketplacePresenceStatus(agent)}
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
                            onClick={() =>
                              setListingAgent({ ...agent, marketplace: null })
                            }
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
        ) : workflowListIsLoading ? (
          <WorkflowsListSkeleton />
        ) : workflowListIsError ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 text-muted-foreground">
            <p className="text-sm text-red-400">Failed to load workflows</p>
            <Button
              onClick={() =>
                void (isMarketplace
                  ? marketplaceWorkflowsQuery.refetch()
                  : allWorkflowsQuery.refetch())
              }
              size="sm"
              variant="outline"
            >
              Retry
            </Button>
          </div>
        ) : (isMarketplace
            ? filteredMarketplaceWorkflows.length
            : filteredVisibleWorkflows.length) === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 text-muted-foreground">
            <Zap className="h-10 w-10 opacity-30" />
            <p className="text-sm">
              {isMarketplace && workflowSearchTerm
                ? "No published workflows match your search"
                : isMarketplace
                  ? "No listed workflows"
                  : "No workflows yet"}
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
            {isMarketplace
              ? filteredMarketplaceWorkflows.map((workflow) => (
                  <MarketplaceWorkflowCard
                    canInstall={
                      workflow.sourceCommunity.relayUrl !==
                        activeCommunity?.relayUrl &&
                      installMarketplaceWorkflowSnapshot(workflow) !== null
                    }
                    key={`${workflow.sourceCommunity.relayUrl}:${workflow.ownerPubkey}:${workflow.workflowId}`}
                    onInstall={useWorkflowHere}
                    workflow={workflow}
                  />
                ))
              : filteredVisibleWorkflows.map(({ workflow, channelName }) => (
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
        initialDefinition={
          dialogState.mode === "create"
            ? (dialogState.initialDefinition ?? null)
            : null
        }
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
      <InstallAgentDialog
        agent={installTarget}
        channels={memberChannels}
        onInstalled={() => {
          setInstallTarget(null);
          void queryClient.invalidateQueries({
            predicate: (query) =>
              query.queryKey[0] === "workflows" ||
              query.queryKey[0] === "workflows-all",
          });
        }}
        onOpenChange={(open) => {
          if (!open) setInstallTarget(null);
        }}
      />
      <AskAgentDialog
        onAsked={() => {
          setAskTarget(null);
          void queryClient.invalidateQueries({
            predicate: (query) => query.queryKey[0] === "workflow-runs",
          });
        }}
        onOpenChange={(open) => {
          if (!open) setAskTarget(null);
        }}
        target={askTarget}
      />
      <AgentJobsDialog
        agentName={earningsTarget?.name ?? ""}
        agentPubkey={earningsTarget?.pubkey ?? null}
        onOpenChange={(open) => {
          if (!open) setEarningsTarget(null);
        }}
      />
    </div>
  );
}
