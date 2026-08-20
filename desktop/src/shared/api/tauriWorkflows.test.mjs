import assert from "node:assert/strict";
import test from "node:test";

import { fromRawTraceEntry } from "./tauriWorkflows.ts";

test("workflow trace conversion preserves a durable assignment receipt", () => {
  const trace = fromRawTraceEntry({
    step_id: "review",
    status: "failed",
    assignment_receipt: {
      agent_pubkey: "11".repeat(32),
      agent_owner_pubkey: "22".repeat(32),
      origin_relay_pubkey: "44".repeat(32),
      agent_relay_pubkey: "55".repeat(32),
      agent_relay_url: "wss://agents.example",
      listing_event_id: "66".repeat(32),
      prompt_event_id: "33".repeat(32),
      completion_event_id: null,
      prompt_published_at_ms: 1_000,
      terminal_at_ms: 91_000,
      duration_ms: 90_000,
      rate_currency: "USD",
      rate_microunits_per_hour: 12_000_000,
      estimated_microunits: 300_000,
      outcome: "failed",
      review_state: "human_review_required",
    },
  });

  assert.equal(trace.assignmentReceipt.durationMs, 90_000);
  assert.equal(trace.assignmentReceipt.estimatedMicrounits, 300_000);
  assert.equal(trace.assignmentReceipt.reviewState, "human_review_required");
  assert.equal(trace.assignmentReceipt.completionEventId, null);
  assert.equal(trace.assignmentReceipt.agentRelayPubkey, "55".repeat(32));
  assert.equal(trace.assignmentReceipt.listingEventId, "66".repeat(32));
});

test("old workflow traces remain compatible without a receipt", () => {
  assert.equal(
    fromRawTraceEntry({ step_id: "notify", status: "completed" })
      .assignmentReceipt,
    null,
  );
});

test("in-flight receipt preserves its pending outcome and rate snapshot", () => {
  const trace = fromRawTraceEntry({
    step_id: "review",
    status: "waiting_agent",
    assignment_receipt: {
      agent_pubkey: "11".repeat(32),
      agent_owner_pubkey: "22".repeat(32),
      prompt_event_id: "33".repeat(32),
      completion_event_id: null,
      prompt_published_at_ms: 1_000,
      terminal_at_ms: null,
      duration_ms: null,
      rate_currency: "USD",
      rate_microunits_per_hour: 12_000_000,
      estimated_microunits: null,
      outcome: "pending",
      review_state: "not_required",
    },
  });

  assert.equal(trace.assignmentReceipt.outcome, "pending");
  assert.equal(trace.assignmentReceipt.rateMicrounitsPerHour, 12_000_000);
});

test("incomplete migrated receipt evidence remains null", () => {
  const trace = fromRawTraceEntry({
    step_id: "legacy",
    status: "timed_out",
    assignment_receipt: {
      agent_pubkey: "11".repeat(32),
      agent_owner_pubkey: null,
      prompt_event_id: "33".repeat(32),
      completion_event_id: null,
      prompt_published_at_ms: null,
      terminal_at_ms: null,
      duration_ms: null,
      rate_currency: null,
      rate_microunits_per_hour: null,
      estimated_microunits: null,
      outcome: "timed_out",
      review_state: "human_review_required",
    },
  });

  assert.equal(trace.assignmentReceipt.agentOwnerPubkey, null);
  assert.equal(trace.assignmentReceipt.durationMs, null);
});

function receiptWithUsage(reportedUsage) {
  return fromRawTraceEntry({
    step_id: "review",
    status: "completed",
    assignment_receipt: {
      agent_pubkey: "11".repeat(32),
      agent_owner_pubkey: "22".repeat(32),
      prompt_event_id: "33".repeat(32),
      completion_event_id: "44".repeat(32),
      prompt_published_at_ms: 1_000,
      terminal_at_ms: 91_000,
      duration_ms: 90_000,
      rate_currency: "USD",
      rate_microunits_per_hour: 12_000_000,
      estimated_microunits: 300_000,
      outcome: "completed",
      review_state: "not_required",
      reported_usage: reportedUsage,
    },
  }).assignmentReceipt;
}

test("reported usage maps a fully populated self-report", () => {
  assert.deepEqual(
    receiptWithUsage({
      harness: "goose",
      model: "claude-sonnet-5",
      input_tokens: 15_500,
      output_tokens: 2_000,
      cost_microunits: 42_000,
      currency: "USD",
    }).reportedUsage,
    {
      harness: "goose",
      model: "claude-sonnet-5",
      inputTokens: 15_500,
      outputTokens: 2_000,
      costMicrounits: 42_000,
      currency: "USD",
    },
  );
});

test("reported usage is null when the agent reported nothing", () => {
  assert.equal(receiptWithUsage(null).reportedUsage, null);
  assert.equal(receiptWithUsage(undefined).reportedUsage, null);
  assert.equal(receiptWithUsage("goose").reportedUsage, null);
  assert.equal(
    receiptWithUsage({ model: "claude-sonnet-5" }).reportedUsage,
    null,
  );
});

test("reported usage drops a cost without a well-formed currency pair", () => {
  const usd = receiptWithUsage({
    harness: "goose",
    cost_microunits: 42_000,
    currency: "usd",
  }).reportedUsage;
  assert.equal(usd.costMicrounits, null);
  assert.equal(usd.currency, null);
  assert.equal(usd.harness, "goose");

  const noCurrency = receiptWithUsage({
    harness: "goose",
    cost_microunits: 42_000,
    currency: null,
  }).reportedUsage;
  assert.equal(noCurrency.costMicrounits, null);
  assert.equal(noCurrency.currency, null);

  const noCost = receiptWithUsage({
    harness: "goose",
    cost_microunits: null,
    currency: "USD",
  }).reportedUsage;
  assert.equal(noCost.currency, null);
});

test("reported usage drops negative and non-integer token counts", () => {
  const usage = receiptWithUsage({
    harness: "goose",
    model: null,
    input_tokens: -1,
    output_tokens: 2.5,
    cost_microunits: -42_000,
    currency: "USD",
  }).reportedUsage;

  assert.equal(usage.inputTokens, null);
  assert.equal(usage.outputTokens, null);
  assert.equal(usage.costMicrounits, null);
  assert.equal(usage.currency, null);
  assert.equal(usage.model, null);
});
