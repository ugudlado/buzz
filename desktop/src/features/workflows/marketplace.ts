import type {
  MarketplaceAgent,
  MarketplaceWorkflow,
} from "@/shared/api/marketplace";
import type { PresenceLookup } from "@/shared/api/types";
import type { AssignmentReceipt } from "@/shared/api/workflowTypes";

export type WorkflowMarketplaceMetadata = {
  listed: true;
  summary: string;
  fixedPrice: { currency: string; microunits: number } | null;
};

export type WorkflowAgentDependency = {
  name: string;
  pubkey: string | null;
  relayPubkey: string | null;
  state: "online" | "away" | "offline" | "unknown" | "missing";
};

type TelemetryEvent = {
  kind: string;
  payload: unknown;
  sessionId: string | null;
  turnId: string | null;
};

export function resolveAssignmentTelemetryCorrelation(
  events: readonly TelemetryEvent[],
  promptEventId: string,
): { sessionId: string | null; turnId: string | null } | null {
  for (const event of events) {
    if (event.kind !== "turn_started") continue;
    const payload = asRecord(event.payload);
    const ids = payload?.triggeringEventIds;
    if (Array.isArray(ids) && ids.includes(promptEventId)) {
      const sessionId =
        event.sessionId ??
        (event.turnId
          ? events.find(
              (candidate) =>
                candidate.turnId === event.turnId &&
                candidate.sessionId !== null,
            )?.sessionId
          : null) ??
        null;
      return { sessionId, turnId: event.turnId };
    }
  }
  return null;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

export function getWorkflowMarketplace(
  definition: Record<string, unknown>,
): WorkflowMarketplaceMetadata | null {
  const marketplace = asRecord(definition.marketplace);
  if (marketplace?.listed !== true) return null;
  const summary =
    typeof marketplace.summary === "string" ? marketplace.summary.trim() : "";
  const rawFixedPrice = asRecord(marketplace.fixed_price);
  const fixedPrice =
    rawFixedPrice &&
    typeof rawFixedPrice.currency === "string" &&
    /^[A-Z]{3}$/.test(rawFixedPrice.currency) &&
    typeof rawFixedPrice.microunits === "number" &&
    Number.isSafeInteger(rawFixedPrice.microunits) &&
    rawFixedPrice.microunits >= 0
      ? {
          currency: rawFixedPrice.currency,
          microunits: rawFixedPrice.microunits,
        }
      : null;
  return { listed: true, summary, fixedPrice };
}

export function installMarketplaceWorkflowSnapshot(
  workflow: MarketplaceWorkflow,
): Record<string, unknown> | null {
  const relayPubkey = workflow.sourceCommunity.relayPubkey;
  if (!relayPubkey) return null;

  const definition = structuredClone(workflow.definition);
  const steps = Array.isArray(definition.steps) ? definition.steps : [];
  for (const candidate of steps) {
    const step = asRecord(candidate);
    if (step?.action !== "assign_to_agent" || step.agent_relay_pubkey) continue;
    if (typeof step.agent_pubkey !== "string") return null;
    step.agent_relay_pubkey = relayPubkey;
    step.agent_relay_url = workflow.sourceCommunity.relayUrl;
  }
  definition.marketplace = {
    ...(asRecord(definition.marketplace) ?? {}),
    listed: false,
    origin_event_id: workflow.eventId,
  };
  return definition;
}

export type InstalledRemoteAgent = {
  name: string;
  pubkey: string;
  relayPubkey: string;
  relayUrl: string;
};

/**
 * Hidden single-assignment workflow representing a remote marketplace agent
 * installed into this community ("Add to community"). Triggering it with a
 * `prompt` field asks the agent; the second step posts the answer into the
 * workflow's channel.
 */
export function installedRemoteAgentDefinition(
  agent: MarketplaceAgent,
): Record<string, unknown> | null {
  const source = agent.sourceCommunity;
  if (!source?.relayPubkey) return null;
  return {
    name: agent.name,
    description: `Remote agent from ${source.name}.`,
    installed_agent: true,
    trigger: { on: "manual" },
    steps: [
      {
        id: "ask",
        action: "assign_to_agent",
        agent: agent.name,
        agent_pubkey: agent.pubkey,
        agent_relay_pubkey: source.relayPubkey,
        agent_relay_url: source.relayUrl,
        instruction: "{{trigger.prompt}}",
      },
      {
        id: "post",
        action: "send_message",
        text: "{{steps.ask.output.result}}",
      },
    ],
  };
}

/** Coordinate of an installed remote agent, or null for ordinary workflows. */
export function getInstalledRemoteAgent(
  definition: Record<string, unknown>,
): InstalledRemoteAgent | null {
  if (definition.installed_agent !== true) return null;
  const steps = Array.isArray(definition.steps) ? definition.steps : [];
  const step = steps.map(asRecord).find((s) => s?.action === "assign_to_agent");
  if (
    !step ||
    typeof step.agent_pubkey !== "string" ||
    typeof step.agent_relay_pubkey !== "string" ||
    typeof step.agent_relay_url !== "string"
  ) {
    return null;
  }
  return {
    name: typeof step.agent === "string" ? step.agent : "Agent",
    pubkey: step.agent_pubkey.toLowerCase(),
    relayPubkey: step.agent_relay_pubkey.toLowerCase(),
    relayUrl: step.agent_relay_url,
  };
}

export function getWorkflowAgentDependencies(
  definition: Record<string, unknown>,
  agents: readonly MarketplaceAgent[],
  presence: PresenceLookup | undefined,
  presenceLoaded: boolean,
): WorkflowAgentDependency[] {
  const dependencies = new Map<string, WorkflowAgentDependency>();
  const steps = Array.isArray(definition.steps) ? definition.steps : [];

  for (const candidate of steps) {
    const step = asRecord(candidate);
    if (step?.action !== "assign_to_agent") continue;
    const name = typeof step.agent === "string" ? step.agent.trim() : "Agent";
    const pubkey =
      typeof step.agent_pubkey === "string"
        ? step.agent_pubkey.trim().toLowerCase()
        : null;
    const relayPubkey =
      typeof step.agent_relay_pubkey === "string"
        ? step.agent_relay_pubkey.trim().toLowerCase()
        : null;
    const key = pubkey ? `${relayPubkey ?? "local"}:${pubkey}` : `name:${name}`;
    if (dependencies.has(key)) continue;

    let state: WorkflowAgentDependency["state"] = "missing";
    if (
      pubkey &&
      agents.some(
        (agent) =>
          agent.pubkey === pubkey &&
          (!relayPubkey || agent.sourceCommunity?.relayPubkey === relayPubkey),
      )
    ) {
      state = relayPubkey
        ? "unknown"
        : presenceLoaded
          ? (presence?.[pubkey] ?? "offline")
          : "unknown";
    }
    dependencies.set(key, {
      name: name || "Agent",
      pubkey,
      relayPubkey,
      state,
    });
  }

  return [...dependencies.values()];
}

export type ContributorEstimateSummary =
  | { kind: "none"; unpriced: number; pending: number }
  | {
      kind: "priced";
      currency: string;
      microunits: number;
      unpriced: number;
      pending: number;
    }
  | { kind: "mixed"; currencies: string[]; unpriced: number; pending: number }
  | { kind: "overflow"; currency: string; unpriced: number; pending: number };

export function summarizeContributorEstimates(
  receipts: readonly AssignmentReceipt[],
): ContributorEstimateSummary {
  const priced = receipts.filter(
    (receipt) =>
      receipt.rateCurrency !== null && receipt.estimatedMicrounits !== null,
  );
  const pending = receipts.filter(
    (receipt) => receipt.outcome === "pending",
  ).length;
  const unpriced = receipts.filter(
    (receipt) => receipt.outcome !== "pending" && receipt.rateCurrency === null,
  ).length;
  if (priced.length === 0) return { kind: "none", unpriced, pending };

  const currencies = [
    ...new Set(priced.map((receipt) => receipt.rateCurrency as string)),
  ].sort();
  if (currencies.length > 1) {
    return { kind: "mixed", currencies, unpriced, pending };
  }

  let microunits = 0;
  for (const receipt of priced) {
    const next = microunits + (receipt.estimatedMicrounits as number);
    if (!Number.isSafeInteger(next)) {
      return { kind: "overflow", currency: currencies[0], unpriced, pending };
    }
    microunits = next;
  }
  return {
    kind: "priced",
    currency: currencies[0],
    microunits,
    unpriced,
    pending,
  };
}

export function formatMicrounits(currency: string, microunits: number) {
  return `${currency} ${(microunits / 1_000_000).toLocaleString(undefined, {
    maximumFractionDigits: 6,
  })}`;
}

/**
 * "15,500 in / 2,000 out", dropping either side the agent left out. Returns
 * null when it reported no token counts at all.
 */
export function formatReportedTokens(
  inputTokens: number | null,
  outputTokens: number | null,
) {
  const parts: string[] = [];
  if (inputTokens !== null) {
    parts.push(`${inputTokens.toLocaleString()} in`);
  }
  if (outputTokens !== null) {
    parts.push(`${outputTokens.toLocaleString()} out`);
  }
  return parts.length > 0 ? parts.join(" / ") : null;
}

export function formatDurationMs(durationMs: number) {
  if (durationMs < 1_000) return `${durationMs}ms`;
  return `${(durationMs / 1_000).toFixed(durationMs % 1_000 === 0 ? 0 : 1)}s`;
}
