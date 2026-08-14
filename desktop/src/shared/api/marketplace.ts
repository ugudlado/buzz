import { relayClient } from "@/shared/api/relayClient";
import { withReadOnlyRelayClient } from "@/shared/api/readOnlyRelayClient";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_MANAGED_AGENT } from "@/shared/constants/kinds";

const MAX_MICROUNITS = Number.MAX_SAFE_INTEGER;

export type MarketplaceAgent = {
  pubkey: string;
  name: string;
  ownerPubkey: string;
  description: string;
  capabilities: string[];
  deployment: "local" | "remote" | "kubernetes";
  pricing: {
    currency: string;
    microunitsPerHour: number;
  } | null;
  directUse: "community" | "restricted" | "owner";
  sourceCommunity?: MarketplaceCommunity;
};

export type MarketplaceCommunity = {
  id: string;
  name: string;
  relayUrl: string;
};

export type ManagedAgentMarketplace = {
  listed: boolean;
  description: string;
  capabilities: string[];
  deployment: "local" | "remote" | "kubernetes";
  pricing?: { currency: string; microunits_per_hour: number } | null;
};

export const MARKETPLACE_AGENT_FILTER = {
  kinds: [KIND_MANAGED_AGENT],
  limit: 500,
};

export const marketplaceAgentQueryKey = (
  communities: readonly MarketplaceCommunity[],
) => [
  "marketplace-agents",
  ...communities
    .map(({ id, name, relayUrl }) => `${id}:${name}:${relayUrl}`)
    .sort(),
];

export function marketplacePresenceTargets(
  agents: readonly MarketplaceAgent[],
  excludeRelayUrl: string | undefined,
) {
  const pubkeysByRelay = new Map<string, Set<string>>();
  for (const agent of agents) {
    const relayUrl = agent.sourceCommunity?.relayUrl;
    if (!relayUrl || relayUrl === excludeRelayUrl) continue;
    const pubkeys = pubkeysByRelay.get(relayUrl) ?? new Set<string>();
    pubkeys.add(agent.pubkey);
    pubkeysByRelay.set(relayUrl, pubkeys);
  }
  return [...pubkeysByRelay]
    .map(([relayUrl, pubkeys]) => ({ relayUrl, pubkeys: [...pubkeys].sort() }))
    .sort((left, right) => left.relayUrl.localeCompare(right.relayUrl));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function isSafeMicrounits(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= 0 &&
    value <= MAX_MICROUNITS
  );
}

function parseMarketplaceAgent(event: RelayEvent): MarketplaceAgent | null {
  const pubkey = event.tags.find((tag) => tag[0] === "d")?.[1]?.toLowerCase();
  if (!pubkey || !/^[0-9a-f]{64}$/.test(pubkey)) return null;

  let content: unknown;
  try {
    content = JSON.parse(event.content);
  } catch {
    return null;
  }
  if (!isRecord(content) || typeof content.name !== "string") return null;

  const marketplace = content.marketplace;
  if (!isRecord(marketplace) || marketplace.listed !== true) return null;
  const description = marketplace.description;
  const deployment = marketplace.deployment;
  const capabilities = marketplace.capabilities;
  if (
    typeof description !== "string" ||
    [...description].length > 500 ||
    (deployment !== "local" &&
      deployment !== "remote" &&
      deployment !== "kubernetes") ||
    !Array.isArray(capabilities) ||
    capabilities.length > 20 ||
    !capabilities.every(
      (capability) =>
        typeof capability === "string" &&
        capability.trim().length > 0 &&
        [...capability].length <= 40,
    )
  ) {
    return null;
  }

  const rawPricing = marketplace.pricing;
  let pricing: MarketplaceAgent["pricing"] = null;
  if (rawPricing !== undefined && rawPricing !== null) {
    if (
      !isRecord(rawPricing) ||
      typeof rawPricing.currency !== "string" ||
      !/^[A-Z]{3}$/.test(rawPricing.currency) ||
      !isSafeMicrounits(rawPricing.microunits_per_hour)
    ) {
      return null;
    }
    pricing = {
      currency: rawPricing.currency,
      microunitsPerHour: rawPricing.microunits_per_hour,
    };
  }

  const respondTo = content.respond_to;
  return {
    pubkey,
    name: content.name.trim() || pubkey,
    ownerPubkey: event.pubkey.toLowerCase(),
    description: description.trim(),
    capabilities: capabilities.map((capability) =>
      (capability as string).trim().toLowerCase(),
    ),
    deployment,
    pricing,
    directUse:
      respondTo === "anyone"
        ? "community"
        : respondTo === "allowlist"
          ? "restricted"
          : "owner",
  };
}

export function parseMarketplaceAgents(
  events: readonly RelayEvent[],
  sourceCommunity?: MarketplaceCommunity,
): MarketplaceAgent[] {
  const latestByCoordinate = new Map<string, RelayEvent>();

  for (const event of events) {
    if (event.kind !== KIND_MANAGED_AGENT) continue;
    const pubkey = event.tags.find((tag) => tag[0] === "d")?.[1]?.toLowerCase();
    if (!pubkey || !/^[0-9a-f]{64}$/.test(pubkey)) continue;
    const coordinate = `${event.pubkey.toLowerCase()}:${pubkey}`;
    const previous = latestByCoordinate.get(coordinate);
    if (
      previous &&
      (previous.created_at > event.created_at ||
        (previous.created_at === event.created_at &&
          previous.id.localeCompare(event.id) >= 0))
    ) {
      continue;
    }
    latestByCoordinate.set(coordinate, event);
  }

  return [...latestByCoordinate.values()]
    .map(parseMarketplaceAgent)
    .filter((listing): listing is MarketplaceAgent => listing !== null)
    .map((listing) =>
      sourceCommunity ? { ...listing, sourceCommunity } : listing,
    )
    .sort((left, right) => left.name.localeCompare(right.name));
}

export async function getMarketplaceAgents(
  communities: readonly MarketplaceCommunity[],
  activeRelayUrl: string,
): Promise<MarketplaceAgent[]> {
  const results = await Promise.allSettled(
    communities.map(async (community) => {
      const events =
        community.relayUrl === activeRelayUrl
          ? await relayClient.fetchEvents(MARKETPLACE_AGENT_FILTER)
          : await withReadOnlyRelayClient(community.relayUrl, (client) =>
              client.fetchEvents(MARKETPLACE_AGENT_FILTER),
            );
      return parseMarketplaceAgents(events, community);
    }),
  );
  if (
    results.length > 0 &&
    results.every((result) => result.status === "rejected")
  ) {
    throw results[0].reason;
  }
  return results
    .flatMap((result) => (result.status === "fulfilled" ? result.value : []))
    .sort((left, right) => left.name.localeCompare(right.name));
}
