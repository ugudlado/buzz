import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

export const Route = createFileRoute("/marketplace")({
  component: MarketplaceRouteComponent,
});

const MarketplaceRouteScreen = React.lazy(async () => {
  const module = await import("./MarketplaceRouteScreen");
  return { default: module.MarketplaceRouteScreen };
});

function MarketplaceRouteComponent() {
  usePreviewFeatureWarning("workflows");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="workflows" />}>
      <MarketplaceRouteScreen />
    </React.Suspense>
  );
}
