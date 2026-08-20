import { Zap } from "lucide-react";

import {
  formatMicrounits,
  getWorkflowMarketplace,
} from "@/features/workflows/marketplace";
import type { MarketplaceWorkflow } from "@/shared/api/marketplace";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { PubKey } from "@/shared/ui/PubKey";

export function MarketplaceWorkflowCard({
  canInstall,
  onInstall,
  workflow,
}: {
  canInstall: boolean;
  onInstall: (workflow: MarketplaceWorkflow) => void;
  workflow: MarketplaceWorkflow;
}) {
  const marketplace = getWorkflowMarketplace(workflow.definition);
  return (
    <Card
      className="p-4"
      data-testid={`marketplace-workflow-${workflow.eventId}`}
    >
      <div className="flex items-start gap-3">
        <div className="mt-0.5 rounded-md bg-amber-500/10 p-2 text-amber-500">
          <Zap className="h-4 w-4" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-sm font-semibold">{workflow.name}</h3>
            <Badge variant="info">Listed</Badge>
          </div>
          {marketplace?.summary ? (
            <p className="mt-2 text-xs text-muted-foreground">
              {marketplace.summary}
            </p>
          ) : null}
          <div className="mt-3 grid gap-2 border-t pt-3 text-xs sm:grid-cols-2">
            <div>
              <p className="text-2xs text-muted-foreground">Price</p>
              <p>
                {marketplace?.fixedPrice
                  ? `${formatMicrounits(
                      marketplace.fixedPrice.currency,
                      marketplace.fixedPrice.microunits,
                    )} fixed display price`
                  : "Usage-based"}
              </p>
            </div>
            <div>
              <p className="text-2xs text-muted-foreground">Community</p>
              <p>{workflow.sourceCommunity.name}</p>
            </div>
            <div className="sm:col-span-2">
              <p className="text-2xs text-muted-foreground">Publisher</p>
              <PubKey pubkey={workflow.ownerPubkey} />
            </div>
          </div>
          {canInstall ? (
            <Button
              className="mt-3"
              onClick={() => onInstall(workflow)}
              size="sm"
            >
              Use in this community
            </Button>
          ) : null}
        </div>
      </div>
    </Card>
  );
}
