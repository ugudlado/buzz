import { Check, Clock, SkipForward, X } from "lucide-react";

import { CopyButton } from "@/features/agents/ui/CopyButton";
import type { MarketplaceAgent } from "@/shared/api/marketplace";
import type { WorkflowApproval, WorkflowRun } from "@/shared/api/types";
import type { AssignmentReceipt } from "@/shared/api/workflowTypes";
import { PubKey } from "@/shared/ui/PubKey";
import { Badge, type BadgeProps } from "@/shared/ui/badge";
import { WorkflowApprovalCard } from "@/features/workflows/ui/WorkflowApprovalCard";
import { AssignmentTelemetry } from "@/features/workflows/ui/AssignmentTelemetry";
import {
  formatDurationMs,
  formatMicrounits,
  formatReportedTokens,
  summarizeContributorEstimates,
} from "@/features/workflows/marketplace";

type WorkflowRunTraceProps = {
  run: WorkflowRun;
  approvals?: WorkflowApproval[];
  marketplaceAgents?: readonly MarketplaceAgent[];
};

function formatStatusLabel(status: string) {
  return status.replace(/_/g, " ");
}

function StepStatusBadge({ status }: { status: string }) {
  const variants: Record<string, BadgeProps["variant"]> = {
    completed: "success",
    failed: "destructive",
    not_started: "destructive",
    error: "destructive",
    running: "info",
    pending: "secondary",
    cancelled: "secondary",
    skipped: "secondary",
    waiting_approval: "warning",
    waiting_agent: "warning",
  };

  return (
    <Badge variant={variants[status] ?? "secondary"}>
      {formatStatusLabel(status)}
    </Badge>
  );
}

function StepStatusIcon({ status }: { status: string }) {
  switch (status) {
    case "completed":
      return <Check className="h-4 w-4 text-green-500" />;
    case "failed":
    case "not_started":
    case "error":
      return <X className="h-4 w-4 text-red-500" />;
    case "skipped":
      return <SkipForward className="h-4 w-4 text-muted-foreground" />;
    case "waiting_approval":
    case "waiting_agent":
      return <Clock className="h-4 w-4 text-amber-500" />;
    default:
      return <Clock className="h-4 w-4 text-blue-500" />;
  }
}

function formatDuration(startedAt: number | null, completedAt: number | null) {
  if (startedAt === null || completedAt === null) return null;
  const seconds = completedAt - startedAt;
  if (seconds < 1) return `${Math.round(seconds * 1000)}ms`;
  return `${seconds.toFixed(1)}s`;
}

function ReceiptId({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span className="w-20 shrink-0 text-2xs text-muted-foreground">
        {label}
      </span>
      <code className="min-w-0 flex-1 truncate text-xs" title={value}>
        {value}
      </code>
      <CopyButton
        className="h-6 w-6 shrink-0"
        iconOnly
        label={`Copy ${label}`}
        size="icon"
        value={value}
        variant="ghost"
      />
    </div>
  );
}

function AssignmentReceiptPanel({
  receipt,
  agent,
}: {
  receipt: AssignmentReceipt;
  agent: MarketplaceAgent | undefined;
}) {
  const hasRate =
    receipt.rateCurrency !== null && receipt.rateMicrounitsPerHour !== null;
  const hasEstimate = hasRate && receipt.estimatedMicrounits !== null;
  const reportedUsage = receipt.reportedUsage;
  const reportedTokens = reportedUsage
    ? formatReportedTokens(
        reportedUsage.inputTokens,
        reportedUsage.outputTokens,
      )
    : null;

  return (
    <div
      className="mt-3 space-y-3 rounded-lg border border-border/70 bg-muted/20 p-3"
      data-testid="assignment-receipt"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <p className="text-xs font-medium">
            {agent?.name ?? "Agent assignment"}
          </p>
          <PubKey className="text-2xs" pubkey={receipt.agentPubkey} />
        </div>
        <div className="flex flex-wrap gap-1.5">
          <Badge
            variant={receipt.outcome === "completed" ? "success" : "warning"}
          >
            {formatStatusLabel(receipt.outcome)}
          </Badge>
          <Badge
            variant={
              receipt.reviewState === "human_review_required"
                ? "warning"
                : "secondary"
            }
          >
            {formatStatusLabel(receipt.reviewState)}
          </Badge>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
        <div>
          <p className="text-2xs text-muted-foreground">Elapsed duration</p>
          <p>
            {receipt.durationMs === null
              ? "Unavailable"
              : formatDurationMs(receipt.durationMs)}
          </p>
        </div>
        <div>
          <p className="text-2xs text-muted-foreground">Estimated value</p>
          <p>
            {hasEstimate
              ? formatMicrounits(
                  receipt.rateCurrency as string,
                  receipt.estimatedMicrounits as number,
                )
              : hasRate
                ? "Pending"
                : "Unpriced"}
          </p>
        </div>
        <div>
          <p className="text-2xs text-muted-foreground">Rate snapshot</p>
          <p>
            {hasRate
              ? `${formatMicrounits(
                  receipt.rateCurrency as string,
                  receipt.rateMicrounitsPerHour as number,
                )}/hour`
              : "Unavailable"}
          </p>
        </div>
        <div>
          <p className="text-2xs text-muted-foreground">Observed</p>
          {receipt.promptPublishedAtMs !== null &&
          receipt.terminalAtMs !== null ? (
            <>
              <p>{new Date(receipt.promptPublishedAtMs).toLocaleString()}</p>
              <p className="text-2xs text-muted-foreground">
                to {new Date(receipt.terminalAtMs).toLocaleString()}
              </p>
            </>
          ) : (
            <p>Incomplete</p>
          )}
        </div>
        <div className="col-span-2" data-testid="reported-usage">
          <p className="text-2xs text-muted-foreground">Reported usage</p>
          {reportedUsage ? (
            <p className="flex flex-wrap items-baseline gap-x-1.5">
              <span data-testid="reported-usage-model">
                {reportedUsage.model ?? reportedUsage.harness}
              </span>
              {reportedTokens ? (
                <span
                  className="text-muted-foreground"
                  data-testid="reported-usage-tokens"
                >
                  {reportedTokens}
                </span>
              ) : null}
              {reportedUsage.currency !== null &&
              reportedUsage.costMicrounits !== null ? (
                <span data-testid="reported-usage-cost">
                  {formatMicrounits(
                    reportedUsage.currency,
                    reportedUsage.costMicrounits,
                  )}
                </span>
              ) : null}
              <span className="text-2xs text-muted-foreground">
                self-reported
              </span>
            </p>
          ) : (
            <p>None</p>
          )}
        </div>
      </div>

      <div className="space-y-1 border-t border-border/60 pt-2">
        <div className="flex min-w-0 items-center gap-2">
          <span className="w-20 shrink-0 text-2xs text-muted-foreground">
            Agent owner
          </span>
          {receipt.agentOwnerPubkey ? (
            <PubKey pubkey={receipt.agentOwnerPubkey} />
          ) : (
            <span className="text-xs text-muted-foreground">Unavailable</span>
          )}
        </div>
        <ReceiptId label="Prompt" value={receipt.promptEventId} />
        {receipt.agentRelayPubkey ? (
          <ReceiptId label="Home relay" value={receipt.agentRelayPubkey} />
        ) : null}
        {receipt.listingEventId ? (
          <ReceiptId label="Listing" value={receipt.listingEventId} />
        ) : null}
        {receipt.completionEventId ? (
          <ReceiptId label="Completion" value={receipt.completionEventId} />
        ) : (
          <p className="text-2xs text-muted-foreground">
            Completion ID unavailable
          </p>
        )}
      </div>

      <AssignmentTelemetry receipt={receipt} />
    </div>
  );
}

function NotStartedReceiptPanel() {
  return (
    <div
      className="mt-3 flex flex-wrap items-center justify-between gap-2 rounded-lg border border-border/70 bg-muted/20 p-3"
      data-testid="assignment-receipt"
    >
      <p className="text-xs font-medium">Assignment not started</p>
      <div className="flex gap-1.5">
        <Badge variant="warning">not started</Badge>
        <Badge variant="secondary">review not required</Badge>
      </div>
    </div>
  );
}

function RunReceiptSummary({ run }: { run: WorkflowRun }) {
  const receipts = run.executionTrace
    .map((step) => step.assignmentReceipt)
    .filter((receipt): receipt is AssignmentReceipt => receipt !== null);
  const contributors = summarizeContributorEstimates(receipts);
  let contributorLabel = "Unavailable";
  if (contributors.kind === "priced") {
    contributorLabel = formatMicrounits(
      contributors.currency,
      contributors.microunits,
    );
    if (contributors.unpriced > 0) {
      contributorLabel += ` + ${contributors.unpriced} unpriced`;
    }
  } else if (contributors.kind === "mixed") {
    contributorLabel = `Mixed currencies (${contributors.currencies.join(", ")}) — not totaled`;
  } else if (contributors.kind === "overflow") {
    contributorLabel = `${contributors.currency} total unavailable`;
  } else if (contributors.pending > 0) {
    contributorLabel = "Pending";
  }
  if (contributors.kind !== "none" && contributors.pending > 0) {
    contributorLabel += ` + ${contributors.pending} pending`;
  }

  const fixedPrice =
    run.fixedPriceCurrency && run.fixedPriceMicrounits !== null
      ? formatMicrounits(run.fixedPriceCurrency, run.fixedPriceMicrounits)
      : "Usage-based";

  return (
    <div
      className="space-y-2 rounded-xl border border-border/70 bg-muted/20 p-3"
      data-testid="workflow-run-receipt"
    >
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs font-semibold">Run receipt</p>
        <Badge variant="secondary">Accounting preview</Badge>
      </div>
      <ReceiptId label="Workflow" value={run.workflowId} />
      <ReceiptId label="Run" value={run.id} />
      <div className="flex min-w-0 items-center gap-2">
        <span className="w-20 shrink-0 text-2xs text-muted-foreground">
          Author
        </span>
        {run.workflowAuthorPubkey ? (
          <PubKey pubkey={run.workflowAuthorPubkey} />
        ) : (
          <span className="text-xs text-muted-foreground">Unavailable</span>
        )}
      </div>
      <div className="grid grid-cols-2 gap-3 border-t border-border/60 pt-2 text-xs">
        <div>
          <p className="text-2xs text-muted-foreground">Fixed display price</p>
          <p>{fixedPrice}</p>
        </div>
        <div>
          <p className="text-2xs text-muted-foreground">Contributor estimate</p>
          <p>{contributorLabel}</p>
        </div>
      </div>
    </div>
  );
}

export function WorkflowRunTrace({
  run,
  approvals = [],
  marketplaceAgents = [],
}: WorkflowRunTraceProps) {
  if (run.executionTrace.length === 0) {
    return (
      <div className="space-y-3" data-testid="workflow-run-trace">
        <RunReceiptSummary run={run} />
        <p className="rounded-xl border border-dashed border-border/70 bg-background/60 px-4 py-6 text-center text-sm text-muted-foreground">
          No steps recorded yet.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3" data-testid="workflow-run-trace">
      <RunReceiptSummary run={run} />
      {run.executionTrace.map((step) => {
        const duration = formatDuration(step.startedAt, step.completedAt);
        const pendingApproval = approvals.find(
          (a) => a.stepId === step.stepId && a.status === "pending",
        );

        return (
          <div
            className="rounded-xl border border-border/60 bg-background/80 p-3 shadow-xs"
            key={step.stepId}
          >
            <div className="flex flex-wrap items-center gap-2 text-sm">
              <StepStatusIcon status={step.status} />
              <span className="min-w-0 flex-1 truncate font-mono text-xs font-medium">
                {step.stepId}
              </span>
              <StepStatusBadge status={step.status} />
              {duration ? (
                <span className="text-xs text-muted-foreground">
                  {duration}
                </span>
              ) : null}
            </div>
            {Object.keys(step.output).length > 0 ? (
              <div className="mt-3">
                <p className="mb-1 text-2xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
                  Output
                </p>
                <pre className="max-h-32 overflow-auto rounded-lg bg-muted/40 px-3 py-2 font-mono text-xs text-muted-foreground">
                  {JSON.stringify(step.output, null, 2)}
                </pre>
              </div>
            ) : null}
            {step.error ? (
              <div className="mt-3">
                <p className="mb-1 text-2xs font-medium uppercase tracking-[0.16em] text-red-400">
                  Error
                </p>
                <pre className="max-h-32 overflow-auto rounded-lg bg-red-500/10 px-3 py-2 font-mono text-xs text-red-400">
                  {step.error}
                </pre>
              </div>
            ) : null}
            {step.assignmentReceipt ? (
              <AssignmentReceiptPanel
                agent={marketplaceAgents.find(
                  (agent) =>
                    agent.pubkey === step.assignmentReceipt?.agentPubkey &&
                    (!step.assignmentReceipt.agentRelayPubkey ||
                      agent.sourceCommunity?.relayPubkey ===
                        step.assignmentReceipt.agentRelayPubkey),
                )}
                receipt={step.assignmentReceipt}
              />
            ) : step.status === "not_started" ? (
              <NotStartedReceiptPanel />
            ) : null}
            {pendingApproval ? (
              <div className="mt-3">
                <p className="mb-2 text-2xs font-medium uppercase tracking-[0.16em] text-amber-600">
                  Pending approval
                </p>
                <WorkflowApprovalCard approval={pendingApproval} />
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
