import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { runtimeAvailabilityWarning } from "./runtimeAvailabilityWarning";

export function RunWarning({
  locus,
  runtime,
}: {
  locus: "local" | "provider";
  runtime?: AcpRuntimeCatalogEntry;
}) {
  const warning = runtime ? runtimeAvailabilityWarning(runtime, locus) : null;
  if (!warning) return null;
  return (
    <p className="text-xs text-warning">
      {warning}
      {locus === "local" && " Visit Settings > Agents to set it up."}
    </p>
  );
}
