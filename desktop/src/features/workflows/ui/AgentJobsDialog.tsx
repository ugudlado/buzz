import { useAgentJobsQuery } from "@/features/workflows/hooks";
import {
  formatDurationMs,
  formatMicrounits,
} from "@/features/workflows/marketplace";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Badge } from "@/shared/ui/badge";
import { PubKey } from "@/shared/ui/PubKey";
import { Skeleton } from "@/shared/ui/skeleton";

type AgentJobsDialogProps = {
  agentPubkey: string | null;
  agentName: string;
  onOpenChange: (open: boolean) => void;
};

function outcomeVariant(
  outcome: "completed" | "failed" | "pending",
): "success" | "warning" | "secondary" {
  if (outcome === "completed") return "success";
  if (outcome === "failed") return "warning";
  return "secondary";
}

/**
 * Provider-side earnings/usage ledger for one of your listed agents: which
 * communities invoked it, for how long, and the estimated cost from your own
 * rate. Owner-gated by the relay.
 */
export function AgentJobsDialog({
  agentPubkey,
  agentName,
  onOpenChange,
}: AgentJobsDialogProps) {
  const query = useAgentJobsQuery(agentPubkey);
  const ledger = query.data;

  return (
    <Dialog onOpenChange={onOpenChange} open={agentPubkey !== null}>
      <DialogContent className="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{agentName} — jobs & earnings</DialogTitle>
          <DialogDescription>
            Cross-community jobs other communities ran on {agentName}, with the
            estimated cost from your listing rate.
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
          {query.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-16 w-full" />
              <Skeleton className="h-16 w-full" />
            </div>
          ) : query.isError ? (
            <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              Couldn&apos;t load this agent&apos;s jobs. Only the agent&apos;s
              owner can view its ledger.
            </p>
          ) : !ledger || ledger.jobs.length === 0 ? (
            <p className="py-10 text-center text-sm text-muted-foreground">
              No cross-community jobs yet. When another community runs{" "}
              {agentName}, it shows up here.
            </p>
          ) : (
            <>
              <section className="space-y-2">
                <p className="text-2xs uppercase tracking-wide text-muted-foreground">
                  By community · {ledger.totals.jobCount} job
                  {ledger.totals.jobCount === 1 ? "" : "s"} total
                </p>
                <div className="space-y-2">
                  {ledger.totals.byCaller.map((row) => (
                    <div
                      className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-muted/20 px-3 py-2"
                      key={`${row.callerRelayPubkey}:${row.currency ?? "none"}`}
                    >
                      <div className="min-w-0">
                        <p className="text-2xs text-muted-foreground">
                          Community
                        </p>
                        <PubKey
                          className="text-xs"
                          pubkey={row.callerRelayPubkey}
                        />
                      </div>
                      <div className="flex items-center gap-4 text-right text-xs tabular-nums">
                        <div>
                          <p className="text-2xs text-muted-foreground">Jobs</p>
                          <p>{row.jobCount}</p>
                        </div>
                        <div>
                          <p className="text-2xs text-muted-foreground">Time</p>
                          <p>{formatDurationMs(row.totalDurationMs)}</p>
                        </div>
                        <div>
                          <p className="text-2xs text-muted-foreground">
                            Est. cost
                          </p>
                          <p className="font-medium">
                            {row.currency
                              ? formatMicrounits(
                                  row.currency,
                                  row.estimatedMicrounits,
                                )
                              : "Unpriced"}
                          </p>
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              </section>

              <section className="space-y-2 border-t border-border/60 pt-3">
                <p className="text-2xs uppercase tracking-wide text-muted-foreground">
                  Individual jobs
                </p>
                <div className="space-y-2">
                  {ledger.jobs.map((job) => (
                    <div
                      className="rounded-lg border border-border/70 bg-muted/10 p-3"
                      key={job.requestEventId}
                    >
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <PubKey
                          className="text-2xs"
                          pubkey={job.callerRelayPubkey}
                        />
                        <Badge variant={outcomeVariant(job.outcome)}>
                          {job.outcome}
                        </Badge>
                      </div>
                      <div className="mt-2 grid grid-cols-3 gap-2 text-xs tabular-nums">
                        <div>
                          <p className="text-2xs text-muted-foreground">
                            Duration
                          </p>
                          <p>
                            {job.durationMs === null
                              ? "—"
                              : formatDurationMs(job.durationMs)}
                          </p>
                        </div>
                        <div>
                          <p className="text-2xs text-muted-foreground">Rate</p>
                          <p>
                            {job.rateCurrency && job.rateMicrounitsPerHour
                              ? `${formatMicrounits(
                                  job.rateCurrency,
                                  job.rateMicrounitsPerHour,
                                )}/hr`
                              : "Unpriced"}
                          </p>
                        </div>
                        <div>
                          <p className="text-2xs text-muted-foreground">
                            Est. cost
                          </p>
                          <p className="font-medium">
                            {job.rateCurrency &&
                            job.estimatedMicrounits !== null
                              ? formatMicrounits(
                                  job.rateCurrency,
                                  job.estimatedMicrounits,
                                )
                              : job.outcome === "pending"
                                ? "Pending"
                                : "Unpriced"}
                          </p>
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              </section>
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
