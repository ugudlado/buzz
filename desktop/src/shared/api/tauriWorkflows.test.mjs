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
