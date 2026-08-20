import type {
  MarketplaceAgent,
  ManagedAgentMarketplace,
} from "@/shared/api/marketplace";
import type { ManagedAgent } from "@/shared/api/types";

/** Project a marketplace listing onto the shape stored on a local agent. */
export function marketplaceFromListing(
  agent: MarketplaceAgent,
): ManagedAgentMarketplace {
  return {
    listed: true,
    description: agent.description,
    capabilities: agent.capabilities,
    deployment: agent.deployment,
    pricing: agent.pricing
      ? {
          currency: agent.pricing.currency,
          microunits_per_hour: agent.pricing.microunitsPerHour,
        }
      : null,
    remote_invocation: agent.remoteInvocation,
  };
}

/**
 * Build the listing a locally-owned agent would publish, so the owner's own
 * agents render through the same card as everyone else's.
 */
export function marketplaceAgentFromLocal(
  agent: ManagedAgent,
  ownerPubkey: string,
  marketplace: ManagedAgentMarketplace,
  sourceCommunity: NonNullable<MarketplaceAgent["sourceCommunity"]>,
): MarketplaceAgent {
  return {
    pubkey: agent.pubkey.toLowerCase(),
    name: agent.name,
    ownerPubkey,
    description: marketplace.description,
    capabilities: marketplace.capabilities,
    deployment: marketplace.deployment,
    pricing: marketplace.pricing
      ? {
          currency: marketplace.pricing.currency,
          microunitsPerHour: marketplace.pricing.microunits_per_hour,
        }
      : null,
    remoteInvocation: marketplace.remote_invocation ?? null,
    directUse:
      agent.respondTo === "anyone"
        ? "community"
        : agent.respondTo === "allowlist"
          ? "restricted"
          : "owner",
    sourceCommunity,
  };
}

/**
 * Whether this community's relay may invoke `agent` remotely. Requires both a
 * known caller relay and an explicit allowance on the listing — an agent with
 * no `remoteInvocation` block is discovery-only.
 */
export function remotePolicyAllows(
  agent: MarketplaceAgent,
  callerRelayPubkey: string | null,
): boolean {
  if (!callerRelayPubkey || !agent.remoteInvocation) return false;
  return (
    agent.remoteInvocation.policy === "any_community" ||
    agent.remoteInvocation.relay_pubkeys.includes(callerRelayPubkey)
  );
}

/** Coordinate key for matching a marketplace listing to an installed agent. */
export function agentCoordinateKey(
  relayPubkey: string,
  pubkey: string,
): string {
  return `${relayPubkey.toLowerCase()}:${pubkey.toLowerCase()}`;
}
