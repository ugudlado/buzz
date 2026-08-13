import {
  AlertTriangle,
  Clock,
  Copy,
  MoreHorizontal,
  Pencil,
  Play,
  Trash2,
  Zap,
} from "lucide-react";

import type { Workflow } from "@/shared/api/types";
import type { MarketplaceAgent } from "@/shared/api/marketplace";
import type { PresenceLookup } from "@/shared/api/types";
import { PubKey } from "@/shared/ui/PubKey";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  formatMicrounits,
  getWorkflowAgentDependencies,
  getWorkflowMarketplace,
} from "../marketplace";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  getWorkflowDescription,
  getWorkflowDisplayStatus,
  getWorkflowTriggerSummary,
} from "./workflowDefinition";

type WorkflowCardProps = {
  workflow: Workflow;
  channelName?: string;
  isActive?: boolean;
  onSelect: (workflowId: string) => void;
  onTrigger: (workflowId: string) => void;
  onEdit: (workflow: Workflow) => void;
  onDuplicate: (workflow: Workflow) => void;
  onDelete: (workflow: Workflow) => void;
  marketplaceAgents?: readonly MarketplaceAgent[];
  presence?: PresenceLookup;
  presenceLoaded?: boolean;
  canManage?: boolean;
};

function StatusBadge({ status }: { status: Workflow["status"] }) {
  const variants: Record<
    Workflow["status"],
    "success" | "secondary" | "warning"
  > = {
    active: "success",
    disabled: "secondary",
    archived: "warning",
  };

  return <Badge variant={variants[status]}>{status}</Badge>;
}

export function WorkflowCard({
  workflow,
  channelName,
  isActive = false,
  onSelect,
  onTrigger,
  onEdit,
  onDuplicate,
  onDelete,
  marketplaceAgents = [],
  presence,
  presenceLoaded = false,
  canManage = false,
}: WorkflowCardProps) {
  const displayStatus = getWorkflowDisplayStatus(workflow);
  const description = getWorkflowDescription(workflow.definition);
  const triggerSummary = getWorkflowTriggerSummary(workflow.definition);
  const marketplace = getWorkflowMarketplace(workflow.definition);
  const dependencies = getWorkflowAgentDependencies(
    workflow.definition,
    marketplaceAgents,
    presence,
    presenceLoaded,
  );
  const unavailableDependencies = dependencies.filter(
    (dependency) =>
      dependency.state === "missing" || dependency.state === "offline",
  );

  return (
    <div
      className={`relative w-full rounded-lg border bg-card p-3 text-left transition-colors hover:bg-muted/50 ${
        isActive ? "border-primary/40 bg-primary/5 shadow-xs" : ""
      }`}
      data-testid={`workflow-card-${workflow.id}`}
    >
      <button
        className="absolute inset-0 rounded-lg"
        onClick={() => onSelect(workflow.id)}
        type="button"
      >
        <span className="sr-only">View {workflow.name}</span>
      </button>

      <div className="flex items-start justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <Zap className="h-4 w-4 shrink-0 text-amber-500" />
            <span className="truncate text-sm font-medium">
              {workflow.name}
            </span>
            <StatusBadge status={displayStatus} />
            {marketplace ? <Badge variant="info">Listed</Badge> : null}
          </div>
          <div className="mt-1.5 flex items-center gap-3 pl-6 text-2xs text-muted-foreground">
            {channelName ? <span>{channelName}</span> : null}
            {triggerSummary ? <span>{triggerSummary}</span> : null}
            <span className="flex items-center gap-1">
              <Clock className="h-4 w-4" />
              {new Date(workflow.updatedAt * 1000).toLocaleDateString()}
            </span>
          </div>
          {marketplace?.summary || description ? (
            <p className="mt-2 pl-6 text-xs text-muted-foreground">
              {marketplace?.summary || description}
            </p>
          ) : null}
          {marketplace ? (
            <div className="mt-2 flex flex-wrap items-center gap-2 pl-6 text-2xs text-muted-foreground">
              <span>
                {marketplace.fixedPrice
                  ? `${formatMicrounits(
                      marketplace.fixedPrice.currency,
                      marketplace.fixedPrice.microunits,
                    )} fixed display price`
                  : "Usage-based"}
              </span>
              <span className="flex items-center gap-1">
                by <PubKey pubkey={workflow.ownerPubkey} />
              </span>
            </div>
          ) : null}
          {dependencies.length > 0 ? (
            <div className="mt-2 flex flex-wrap gap-1.5 pl-6">
              {dependencies.map((dependency) => (
                <Badge
                  key={dependency.pubkey ?? dependency.name}
                  variant={
                    dependency.state === "missing" ||
                    dependency.state === "offline"
                      ? "warning"
                      : dependency.state === "online"
                        ? "success"
                        : "secondary"
                  }
                >
                  {dependency.name} · {dependency.state}
                </Badge>
              ))}
            </div>
          ) : null}
          {unavailableDependencies.length > 0 ? (
            <p className="mt-2 flex items-center gap-1.5 pl-6 text-xs text-amber-600 dark:text-amber-400">
              <AlertTriangle className="h-4 w-4 shrink-0" />
              {unavailableDependencies.length === 1
                ? "1 agent dependency is unavailable"
                : `${unavailableDependencies.length} agent dependencies are unavailable`}
            </p>
          ) : null}
        </div>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Workflow actions"
              className="relative z-10 h-7 w-7 shrink-0"
              size="icon"
              variant="ghost"
            >
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {canManage ? (
              <DropdownMenuItem onClick={() => onTrigger(workflow.id)}>
                <Play className="mr-2 h-4 w-4" />
                Trigger
              </DropdownMenuItem>
            ) : null}
            {canManage ? (
              <DropdownMenuItem onClick={() => onEdit(workflow)}>
                <Pencil className="mr-2 h-4 w-4" />
                Edit
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuItem onClick={() => onDuplicate(workflow)}>
              <Copy className="mr-2 h-4 w-4" />
              Duplicate
            </DropdownMenuItem>
            {canManage ? (
              <DropdownMenuItem
                className="text-destructive"
                onClick={() => onDelete(workflow)}
              >
                <Trash2 className="mr-2 h-4 w-4" />
                Delete
              </DropdownMenuItem>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}
