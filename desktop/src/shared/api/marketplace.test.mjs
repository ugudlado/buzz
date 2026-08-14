import assert from "node:assert/strict";
import test from "node:test";

import {
  MARKETPLACE_AGENT_FILTER,
  isAgentForRelay,
  marketplaceAgentQueryKey,
  parseMarketplaceAgents,
} from "./marketplace.ts";

const AGENT = "11".repeat(32);
const OWNER = "22".repeat(32);

function event(content, createdAt = 1, id = "a") {
  return {
    id,
    pubkey: OWNER,
    created_at: createdAt,
    kind: 30177,
    tags: [["d", AGENT]],
    content: JSON.stringify(content),
    sig: "",
  };
}

test("catalog query is explicitly scoped to managed-agent events", () => {
  assert.deepEqual(MARKETPLACE_AGENT_FILTER, { kinds: [30177], limit: 500 });
});

test("local publish candidates are scoped to their community relay", () => {
  assert.equal(
    isAgentForRelay("WSS://COMMUNITY-A.EXAMPLE/", "wss://community-a.example"),
    true,
  );
  assert.equal(
    isAgentForRelay("wss://community-a.example", "wss://community-b.example"),
    false,
  );
});

test("catalog cache key includes the joined communities", () => {
  assert.notDeepEqual(
    marketplaceAgentQueryKey([
      { id: "a", name: "A", relayUrl: "wss://community-a.example" },
    ]),
    marketplaceAgentQueryKey([
      { id: "b", name: "B", relayUrl: "wss://community-b.example" },
    ]),
  );
});

test("parser retains the source community for federated listings", () => {
  const sourceCommunity = {
    id: "community-a",
    name: "Community A",
    relayUrl: "wss://community-a.example",
  };
  const [listing] = parseMarketplaceAgents(
    [
      event({
        name: "Bumble",
        marketplace: {
          listed: true,
          description: "Community agent",
          capabilities: [],
          deployment: "local",
        },
      }),
    ],
    sourceCommunity,
  );
  assert.deepEqual(listing.sourceCommunity, sourceCommunity);
});

test("parser returns only sanitized listed metadata", () => {
  const [listing] = parseMarketplaceAgents([
    event({
      name: "Reviewer",
      system_prompt: "secret prompt",
      env_vars: { TOKEN: "secret" },
      respond_to: "anyone",
      marketplace: {
        listed: true,
        description: " Reviews Rust ",
        capabilities: ["Rust", "review"],
        deployment: "remote",
        pricing: { currency: "USD", microunits_per_hour: 12_000_000 },
      },
    }),
  ]);

  assert.deepEqual(listing, {
    pubkey: AGENT,
    name: "Reviewer",
    ownerPubkey: OWNER,
    description: "Reviews Rust",
    capabilities: ["rust", "review"],
    deployment: "remote",
    pricing: { currency: "USD", microunitsPerHour: 12_000_000 },
    directUse: "community",
  });
  assert.equal("systemPrompt" in listing, false);
  assert.equal("envVars" in listing, false);
});

test("parser ignores unlisted and invalid listings and keeps the newest head", () => {
  const listings = parseMarketplaceAgents([
    event({ name: "Hidden", marketplace: { listed: false } }),
    event(
      {
        name: "Old",
        marketplace: {
          listed: true,
          description: "old",
          capabilities: [],
          deployment: "local",
        },
      },
      2,
      "b",
    ),
    event(
      {
        name: "New",
        marketplace: {
          listed: true,
          description: "new",
          capabilities: [],
          deployment: "kubernetes",
        },
      },
      3,
      "c",
    ),
    event({
      name: "Bad rate",
      marketplace: {
        listed: true,
        description: "bad",
        capabilities: [],
        deployment: "local",
        pricing: { currency: "usd", microunits_per_hour: 1 },
      },
    }),
  ]);

  assert.equal(listings.length, 1);
  assert.equal(listings[0].name, "New");
});

test("a newer unlisted head removes an older agent from discovery", () => {
  const listed = event(
    {
      name: "Reviewer",
      marketplace: {
        listed: true,
        description: "listed",
        capabilities: [],
        deployment: "local",
      },
    },
    1,
    "a",
  );
  const unlisted = event(
    { name: "Reviewer", marketplace: { listed: false } },
    2,
    "b",
  );
  assert.deepEqual(parseMarketplaceAgents([listed, unlisted]), []);
});

test("another author's event cannot suppress an owner's listing", () => {
  const listed = event({
    name: "Reviewer",
    marketplace: {
      listed: true,
      description: "listed",
      capabilities: [],
      deployment: "local",
    },
  });
  const spoof = {
    ...event({ name: "Spoof", marketplace: { listed: false } }, 2, "b"),
    pubkey: "33".repeat(32),
  };

  assert.equal(parseMarketplaceAgents([listed, spoof])[0].name, "Reviewer");
});
