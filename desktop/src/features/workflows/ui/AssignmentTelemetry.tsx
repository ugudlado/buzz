import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import { useObserverEvents } from "@/features/agents/ui/useObserverEvents";
import { CopyButton } from "@/features/agents/ui/CopyButton";
import { resolveAssignmentTelemetryCorrelation } from "@/features/workflows/marketplace";
import { useIdentityQuery } from "@/shared/api/hooks";
import { readArchivedEvents } from "@/shared/api/tauriArchive";
import type { AssignmentReceipt } from "@/shared/api/workflowTypes";

type Metric = {
  cumulative?: { inputTokens?: number; outputTokens?: number } | null;
  deltaReliable?: boolean;
  harness?: string;
  model?: string | null;
  sessionId?: string | null;
  turn?: {
    inputTokens?: number;
    outputTokens?: number;
    costUsd?: number;
  } | null;
  turnId?: string | null;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function metricFrom(value: unknown): Metric | null {
  return isRecord(value) && typeof value.harness === "string"
    ? (value as Metric)
    : null;
}

function CorrelationId({ label, value }: { label: string; value: string }) {
  return (
    <span className="inline-flex min-w-0 items-center gap-1">
      {label} <code className="max-w-32 truncate">{value}</code>
      <CopyButton
        className="h-5 w-5"
        iconOnly
        label={`Copy ${label}`}
        size="icon"
        value={value}
        variant="ghost"
      />
    </span>
  );
}

export function AssignmentTelemetry({
  receipt,
}: {
  receipt: AssignmentReceipt;
}) {
  const identity = useIdentityQuery().data?.pubkey ?? null;
  const isOwner = Boolean(
    identity &&
      receipt.agentOwnerPubkey &&
      identity.toLowerCase() === receipt.agentOwnerPubkey.toLowerCase(),
  );
  const observer = useObserverEvents(isOwner, receipt.agentPubkey);
  const correlation = React.useMemo(
    () =>
      resolveAssignmentTelemetryCorrelation(
        observer.events,
        receipt.promptEventId,
      ),
    [observer.events, receipt.promptEventId],
  );

  const metricQuery = useQuery({
    queryKey: [
      "workflow-assignment-metric",
      identity,
      correlation?.sessionId,
      correlation?.turnId,
    ],
    enabled: Boolean(
      isOwner && identity && correlation?.sessionId && correlation.turnId,
    ),
    queryFn: async () => {
      const rows = await readArchivedEvents("owner_p", identity as string, {
        kinds: [44200],
        limit: 200,
      });
      return rows
        .map((row) => metricFrom(row))
        .find(
          (metric) =>
            metric &&
            metric.sessionId === correlation?.sessionId &&
            metric.turnId === correlation?.turnId,
        );
    },
    staleTime: 30_000,
  });

  if (!isOwner) return null;
  const metric = metricQuery.data;
  return (
    <div className="space-y-1 border-t border-border/60 pt-2 text-2xs text-muted-foreground">
      {correlation ? (
        <div className="flex flex-wrap items-center gap-2">
          <span>Live telemetry:</span>
          {correlation.sessionId ? (
            <CorrelationId label="session" value={correlation.sessionId} />
          ) : (
            <span>session unknown</span>
          )}
          {correlation.turnId ? (
            <CorrelationId label="turn" value={correlation.turnId} />
          ) : (
            <span>turn unknown</span>
          )}
        </div>
      ) : (
        <p>Live telemetry: unavailable</p>
      )}
      {metric ? (
        <p>
          Usage diagnostics: {metric.model ?? metric.harness}
          {metric.turn?.inputTokens !== undefined
            ? ` · ${metric.turn.inputTokens} input tokens`
            : ""}
          {metric.turn?.outputTokens !== undefined
            ? ` · ${metric.turn.outputTokens} output tokens`
            : ""}
          {metric.turn?.costUsd !== undefined
            ? ` · $${metric.turn.costUsd.toFixed(6)} provider estimate`
            : ""}
          {metric.deltaReliable === false ? " · incomplete delta" : ""}
        </p>
      ) : (
        <p>Usage diagnostics: unavailable</p>
      )}
    </div>
  );
}
