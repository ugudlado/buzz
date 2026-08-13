import assert from "node:assert/strict";
import test from "node:test";

import {
  getWorkflowAgentDependencies,
  getWorkflowMarketplace,
  resolveAssignmentTelemetryCorrelation,
  summarizeContributorEstimates,
} from "./marketplace.ts";
import { yamlToFormState } from "./ui/workflowFormTypes.ts";

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
