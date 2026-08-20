import assert from "node:assert/strict";
import test from "node:test";

import {
  formatReportedTokens,
  getInstalledRemoteAgent,
  getWorkflowAgentDependencies,
  getWorkflowMarketplace,
  installMarketplaceWorkflowSnapshot,
  installedRemoteAgentDefinition,
  resolveAssignmentTelemetryCorrelation,
  summarizeContributorEstimates,
} from "./marketplace.ts";
import { formStateToYaml, yamlToFormState } from "./ui/workflowFormTypes.ts";

const A = "11".repeat(32);
const B = "22".repeat(32);
const agent = (pubkey) => ({ pubkey });

test("workflow listing exposes fixed display price separately", () => {
  assert.deepEqual(
    getWorkflowMarketplace({
      marketplace: {
        listed: true,
        summary: " Release review ",
        fixed_price: { currency: "USD", microunits: 5_000_000 },
      },
    }),
    {
      listed: true,
      summary: "Release review",
      fixedPrice: { currency: "USD", microunits: 5_000_000 },
    },
  );
});

test("listed workflow definitions stay in the lossless YAML editor", () => {
  assert.deepEqual(
    yamlToFormState(`
name: Listed
trigger: { on: message_posted }
marketplace: { listed: true, summary: Review }
steps: []
`),
    {
      ok: false,
      error: "Marketplace metadata is edited in the YAML editor",
    },
  );
});

test("manual workflows are editable in the form builder", () => {
  const result = yamlToFormState(`
name: Manual review
trigger: { on: manual }
steps: []
`);

  assert.equal(result.ok, true);
  assert.equal(result.state?.trigger.on, "manual");
});

test("remote agent coordinates survive form editing", () => {
  const relayPubkey = "33".repeat(32);
  const parsed = yamlToFormState(`
name: Remote review
trigger: { on: manual }
steps:
  - id: review
    action: assign_to_agent
    agent: Reviewer
    agent_pubkey: "${A}"
    agent_relay_pubkey: "${relayPubkey}"
    agent_relay_url: wss://agents.example.com
    instruction: Review this
`);

  assert.equal(parsed.ok, true);
  const yaml = formStateToYaml(parsed.state);
  assert.match(yaml, new RegExp(`agent_relay_pubkey: "${relayPubkey}"`));
  assert.match(yaml, /agent_relay_url: wss:\/\/agents\.example\.com/);
});

test("workflow install snapshots origin and remote agent coordinates", () => {
  const relayPubkey = "44".repeat(32);
  const eventId = "55".repeat(32);
  const definition = {
    name: "Review",
    trigger: { on: "manual" },
    marketplace: { listed: true, summary: "Review changes" },
    steps: [
      {
        id: "review",
        action: "assign_to_agent",
        agent: "Reviewer",
        agent_pubkey: A,
      },
      { id: "done", action: "send_message", text: "Done" },
    ],
  };

  const installed = installMarketplaceWorkflowSnapshot({
    eventId,
    workflowId: "workflow-id",
    name: "Review",
    ownerPubkey: B,
    definition,
    createdAt: 1,
    sourceCommunity: {
      id: "community-a",
      name: "Community A",
      relayUrl: "wss://community-a.example",
      relayPubkey,
    },
  });

  assert.deepEqual(installed.marketplace, {
    listed: false,
    summary: "Review changes",
    origin_event_id: eventId,
  });
  assert.equal(installed.steps[0].agent_relay_pubkey, relayPubkey);
  assert.equal(installed.steps[0].agent_relay_url, "wss://community-a.example");
  assert.equal(definition.steps[0].agent_relay_pubkey, undefined);
});

test("workflow dependencies distinguish offline and missing agents", () => {
  const dependencies = getWorkflowAgentDependencies(
    {
      steps: [
        { action: "assign_to_agent", agent: "Reviewer", agent_pubkey: A },
        { action: "assign_to_agent", agent: "Writer", agent_pubkey: B },
      ],
    },
    [agent(A)],
    {},
    true,
  );

  assert.deepEqual(
    dependencies.map(({ name, state }) => ({ name, state })),
    [
      { name: "Reviewer", state: "offline" },
      { name: "Writer", state: "missing" },
    ],
  );
});

test("workflow dependencies keep identical agent keys separate by home relay", () => {
  const relayA = "aa".repeat(32);
  const relayB = "bb".repeat(32);
  const dependencies = getWorkflowAgentDependencies(
    {
      steps: [
        {
          action: "assign_to_agent",
          agent: "A Reviewer",
          agent_pubkey: A,
          agent_relay_pubkey: relayA,
        },
        {
          action: "assign_to_agent",
          agent: "B Reviewer",
          agent_pubkey: A,
          agent_relay_pubkey: relayB,
        },
      ],
    },
    [
      { pubkey: A, sourceCommunity: { relayPubkey: relayA } },
      { pubkey: A, sourceCommunity: { relayPubkey: relayB } },
    ],
    undefined,
    false,
  );

  assert.deepEqual(
    dependencies.map(({ name, relayPubkey }) => ({ name, relayPubkey })),
    [
      { name: "A Reviewer", relayPubkey: relayA },
      { name: "B Reviewer", relayPubkey: relayB },
    ],
  );
});

test("assignment telemetry joins prompt, turn, and resolved session", () => {
  assert.deepEqual(
    resolveAssignmentTelemetryCorrelation(
      [
        {
          kind: "session_resolved",
          payload: { sessionId: "wrong-session" },
          sessionId: "wrong-session",
          turnId: "turn-2",
        },
        {
          kind: "turn_started",
          payload: { triggeringEventIds: ["prompt-1"] },
          sessionId: null,
          turnId: "turn-1",
        },
        {
          kind: "session_resolved",
          payload: { sessionId: "session-1" },
          sessionId: "session-1",
          turnId: "turn-1",
        },
      ],
      "prompt-1",
    ),
    { sessionId: "session-1", turnId: "turn-1" },
  );
});

function receipt(currency, estimate) {
  return { rateCurrency: currency, estimatedMicrounits: estimate };
}

test("contributor estimates total one currency and refuse mixed currencies", () => {
  assert.deepEqual(
    summarizeContributorEstimates([
      receipt("USD", 200_000),
      receipt("USD", 300_000),
      receipt(null, null),
    ]),
    {
      kind: "priced",
      currency: "USD",
      microunits: 500_000,
      unpriced: 1,
      pending: 0,
    },
  );
  assert.deepEqual(
    summarizeContributorEstimates([
      receipt("USD", 200_000),
      receipt("EUR", 300_000),
    ]),
    { kind: "mixed", currencies: ["EUR", "USD"], unpriced: 0, pending: 0 },
  );
  assert.deepEqual(
    summarizeContributorEstimates([
      { ...receipt("USD", null), outcome: "pending" },
    ]),
    { kind: "none", unpriced: 0, pending: 1 },
  );
});

test("reported token counts render as a grouped in/out pair", () => {
  assert.equal(formatReportedTokens(15_500, 2_000), "15,500 in / 2,000 out");
  assert.equal(formatReportedTokens(15_500, null), "15,500 in");
  assert.equal(formatReportedTokens(null, 2_000), "2,000 out");
  assert.equal(formatReportedTokens(null, null), null);
  assert.equal(formatReportedTokens(0, 0), "0 in / 0 out");
});

test("installed remote agent definition round-trips through the detector", () => {
  const definition = installedRemoteAgentDefinition({
    pubkey: A,
    name: "Bumble",
    sourceCommunity: {
      id: "c1",
      name: "Relay A",
      relayUrl: "wss://relay-a.example",
      relayPubkey: B,
    },
  });
  assert.equal(definition.installed_agent, true);
  assert.deepEqual(getInstalledRemoteAgent(definition), {
    name: "Bumble",
    pubkey: A,
    relayPubkey: B,
    relayUrl: "wss://relay-a.example",
  });
});

test("installed remote agent definition requires a home relay pubkey", () => {
  assert.equal(
    installedRemoteAgentDefinition({
      pubkey: A,
      name: "Bumble",
      sourceCommunity: {
        id: "c1",
        name: "Relay A",
        relayUrl: "wss://relay-a.example",
      },
    }),
    null,
  );
});

test("ordinary workflows are not detected as installed agents", () => {
  assert.equal(
    getInstalledRemoteAgent({
      name: "Plain",
      trigger: { on: "manual" },
      steps: [
        {
          id: "ask",
          action: "assign_to_agent",
          agent: "Local",
          agent_pubkey: A,
          instruction: "hi",
        },
      ],
    }),
    null,
  );
  assert.equal(
    getInstalledRemoteAgent({ installed_agent: true, steps: [] }),
    null,
  );
});
