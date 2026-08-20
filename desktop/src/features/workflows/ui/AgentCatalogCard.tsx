import { Bot } from "lucide-react";

import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceBadge } from "@/features/presence/ui/PresenceBadge";
import { formatMicrounits } from "@/features/workflows/marketplace";
import type { MarketplaceAgent } from "@/shared/api/marketplace";
import type {
  ManagedAgent,
  PresenceStatus,
  Workflow,
} from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { PubKey } from "@/shared/ui/PubKey";

/**
 * One marketplace listing. Renders three mutually exclusive action rows: the
 * owner's own agent (edit/earnings/unpublish), an already-installed agent
 * (ask/runs/remove), or an installable one (add).
 */
export function AgentCatalogCard({
  agent,
  presenceStatus,
  localAgent,
  canInstall,
  installedWorkflow,
  onEdit,
  onInstall,
  onAsk,
  onRemove,
  onShowRuns,
  onShowEarnings,
  onUnpublish,
}: {
  agent: MarketplaceAgent;
  presenceStatus: PresenceStatus | null;
  localAgent?: ManagedAgent;
  canInstall: boolean;
  installedWorkflow: Workflow | null;
  onEdit: (agent: ManagedAgent) => void;
  onInstall: (agent: MarketplaceAgent) => void;
  onAsk: (workflow: Workflow, agentName: string) => void;
  onRemove: (workflow: Workflow) => void;
  onShowRuns: (workflowId: string) => void;
  onShowEarnings: (agentPubkey: string, agentName: string) => void;
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
            <Badge variant={agent.remoteInvocation ? "default" : "outline"}>
              {agent.remoteInvocation ? "Remote-ready" : "Discovery only"}
            </Badge>
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
              <p className="text-2xs text-muted-foreground">Community</p>
              <p>{agent.sourceCommunity?.name ?? "Current community"}</p>
            </div>
            <div className="sm:col-span-2">
              <p className="text-2xs text-muted-foreground">Agent</p>
              <PubKey
                pubkey={agent.pubkey}
                testId={`marketplace-agent-pubkey-${agent.pubkey}`}
              />
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
                onClick={() => onShowEarnings(agent.pubkey, agent.name)}
                size="sm"
                variant="outline"
              >
                Jobs &amp; earnings
              </Button>
              <Button
                onClick={() => onUnpublish(localAgent)}
                size="sm"
                variant="ghost"
              >
                Unpublish
              </Button>
            </div>
          ) : installedWorkflow ? (
            <div className="mt-3 flex items-center gap-2">
              <Button
                onClick={() => onAsk(installedWorkflow, agent.name)}
                size="sm"
              >
                Ask
              </Button>
              <Button
                onClick={() => onShowRuns(installedWorkflow.id)}
                size="sm"
                variant="outline"
              >
                Runs
              </Button>
              <Button
                onClick={() => onRemove(installedWorkflow)}
                size="sm"
                variant="ghost"
              >
                Remove from community
              </Button>
              <Badge variant="secondary">Added</Badge>
            </div>
          ) : canInstall ? (
            <div className="mt-3">
              <Button onClick={() => onInstall(agent)} size="sm">
                Add to community
              </Button>
            </div>
          ) : null}
        </div>
      </div>
    </Card>
  );
}
