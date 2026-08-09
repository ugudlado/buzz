# `AssignToAgent` — implementation notes (branch `feat/assign-to-agent-step`)

Companion to [native-agent-workflows-design.md](native-agent-workflows-design.md)
(the reviewed design). This doc maps the actual diff to the feature, area by
area, and flags what on the branch is *not* part of it.

The original `feat/workflow-updates` branch was split for review: this branch
carries the `AssignToAgent` step end to end; the approval-gate implementation
(`suspend_run_for_approval`, approval expiry sweeper, `api/workflows.rs` HTTP
reads, desktop runs-panel Tauri calls) is stacked on top in
`feat/workflow-infra`; the multi-value `#h` filter fix went to its own
branch; and the harness QoL changes (ACP permission-mode rework,
`repo_paths.rs` repo-cwd resolution, `BUZZ_PRIVATE_KEY` subprocess binding,
`buzz-cli` workflow-import deps) live on `feat/acp-harness-qol`.

## What the feature does

A workflow step of type `assign_to_agent` posts a relay-signed kind:9
`@mention` prompt into the workflow's channel, suspends the run, and resumes it
when the assigned agent replies. The reply's fenced ` ```completion ` YAML block
becomes the step's output (`{{steps.<id>.output.X}}`); a malformed or missing
block resumes the run as `failed` instead of leaving it stuck.

```yaml
- id: implement
  action: assign_to_agent
  agent: "Coder"                 # channel display name (templated)
  agent_pubkey: "abc123…"        # optional 64-hex pin; authoritative when set
  instruction: "Fix the build for {{trigger.ticket}}"
  timeout: "2h"                  # default 24h
```

## End-to-end flow

1. **Dispatch** — `dispatch_action` (`crates/buzz-workflow/src/executor.rs`)
   resolves the agent *before* sending: `verify_agent_membership` when
   `agent_pubkey` is pinned, else `resolve_agent` by case-insensitive exact
   display name (zero or ambiguous matches = typed step failure, because
   `send_message`'s mention resolution silently drops unknown `p` tags). It
   posts `"@{agent} {instruction}"` through the existing sink and returns
   `StepResult::Suspended`.
2. **Suspend** — `finalize_run` (`crates/buzz-workflow/src/lib.rs`) writes a
   `workflow_agent_steps` row keyed by the **public prompt event id** (no
   secret to hash, unlike approvals) and sets the run to `WaitingAgent`
   (migration 0030; distinct from `WaitingApproval` so listings can tell the
   suspend kinds apart — the waking trigger still decides which table to
   check).
3. **Agent side** — the ACP harness accepts the relay-signed prompt
   (author-gate bypass, below), runs the turn, and **posts the completion reply
   itself** signed with the agent's key, flat-replying to the thread root and
   tagging `buzz:completion-of <prompt_event_id>`.
4. **Resume** — a post-store hook on every kind:9
   (`crates/buzz-relay/src/handlers/event.rs:557`) calls
   `try_resume_agent_step` (`handlers/command_executor.rs:1393`): match by
   `buzz:completion-of` tag first, NIP-10 parent as fallback; verify the author
   equals the row's `agent_pubkey` (the tag confers no authority); parse the
   completion block; CAS the row `pending → done`; CAS the run
   `waiting_agent → running`; rewrite the step's `waiting` trace entry to
   `completed`/`failed` (preserving `started_at` so agent think-time counts);
   `execute_from_step` continues the run.
5. **Sweeper** — a relay background task (`buzz-relay/src/main.rs:1025+`)
   expires overdue `workflow_agent_steps` rows (failing their runs) and
   retries "stuck done" rows (CAS'd `done` but the run never resumed — relay
   crashed in between). Interval: `BUZZ_AGENT_STEP_SWEEP_INTERVAL_SECS`
   (default 60s). The mirror-image approval sweeper is on
   `feat/workflow-infra`.

## Changes by area

### Workflow engine — `crates/buzz-workflow`

| File | Change |
|---|---|
| `schema.rs` | New `ActionDef::AssignToAgent { agent, agent_pubkey, instruction, timeout }`; validation (non-empty fields, pubkey must be 64 hex). `agent`/`instruction` are templated; `agent_pubkey` deliberately is not. |
| `executor.rs` | Suspension generalized: `SuspendReason::{Approval, AgentAssignment}`, `StepResult::Suspended { resume_token, reason, timeout }`, `ExecutionResult.suspend: Option<PendingSuspend>` (replaces `approval_token`). Dispatch arm with pre-send agent resolution. NIP-10 threading via `ThreadAnchor` — every prompt replies **flat to the run root** (`root, prompt1, reply1, prompt2, …`), never a deepening chain. Trace entries now stamped with `started_at`/`completed_at` (unix seconds) on **all** paths, including new `waiting` and `failed` statuses (failures previously left no trace entry at all). |
| `lib.rs` | `finalize_run` handles agent-step suspensions via new `suspend_run_for_agent_step` / `fail_run_after_suspend_error` (a run left waiting with no gating row would be stranded, so row-write failure fails the run). Approval suspensions still fail explicitly on this branch; the working `suspend_run_for_approval` lands with `feat/workflow-infra`. |
| `completion.rs` (new) | Parses the ` ```completion ` YAML block into `AgentCompletion { status, outputs, reason, usage }`. `parse()` **never fails outward** — missing/malformed → `status: failed`, `reason` = raw content (capped at 2,000 chars). `usage` is optional self-reported `{input_tokens, output_tokens, cost}`. |
| `action_sink.rs` | `ActionSink` gains `reply_to: Option<&ThreadAnchor>` on `send_message`, plus `resolve_agent` and `verify_agent_membership` (both `Option<Vec<u8>>`; ambiguity → `None`). |

### DB — `migrations/0029`–`0030` + `crates/buzz-db`

New table `workflow_agent_steps`: PK `(community_id, prompt_event_id)`, status
enum `pending|done|expired|failed`, `agent_pubkey BYTEA`, `output JSONB`,
`expires_at`, cascade FKs to workflows/runs, indexes by workflow/run/status.
Migration 0030 adds `run_status` value `waiting_agent` for agent suspensions.

Concurrency machinery in `buzz-db/src/workflow.rs` (+952 lines, additive):

- `update_agent_step_by_prompt_event_id` — CAS `WHERE status = 'pending'`; the
  **double-resume guard** (duplicate reply vs. expiry sweep — one winner).
- `try_mark_run_resuming` — CAS the run `waiting_agent|waiting_approval →
  running`; the concurrency boundary between a live resume and the
  crash-recovery sweep (accepts both waiting statuses so the approval resume
  path on `feat/workflow-infra` shares it).
- `sweep_expired_agent_steps` — batched (`LIMIT` subselect) so backlogs drain
  incrementally.
- `list_stuck_done_agent_steps` — crash-window recovery, with a min-age grace
  period and an `r.current_step = s.step_index` guard so an already-advanced
  run is never rewound by re-detecting an old `done` row.

### Relay — `crates/buzz-relay`

- `handlers/command_executor.rs` (+419): `try_resume_agent_step`,
  `resume_from_done_agent_step`, `retry_stuck_agent_step_resume`,
  `agent_completion_to_output_json` (injects `outputs.__reply_thread` so the
  next step threads into the same conversation without a DB round-trip).
  Harness-posted replies (tagged `buzz:completion-of`) without a completion
  block are treated as `success` with the text as `reason`; untagged replies
  keep the strict missing-block-is-failure contract.
- `handlers/event.rs` (+16): the kind:9 post-store hook (sibling of the
  workflow-trigger block; non-matches are free).
- `workflow_sink.rs`: `send_message` emits real NIP-10 `e` tags + thread
  metadata when `reply_to` is set; new `resolve_agent` /
  `named_channel_members` so free-text mentions and step resolution see the
  same membership snapshot.
- `main.rs`: the agent-step sweeper. Safe on every pod — the
  `UPDATE … RETURNING` is the at-most-once boundary per row.

### ACP harness — `crates/buzz-acp`

- **Author gate** (design prerequisite #6): `author_allowed` accepts a non-DM
  message that is relay-signed **and** tagged `buzz:workflow`
  (`is_relay_workflow_message`, fail-closed when the relay pubkey is unknown).
  Relay pubkey fetched once at startup via NIP-11 (`relay.rs:
  fetch_relay_pubkey`). Without this, default `respond_to: owner-only` agents
  silently drop workflow prompts. Same bypass wired into setup mode.
- **Completion posting** (`pool.rs: spawn_workflow_completion_if_applicable`,
  `post_workflow_completion`): the harness signs and posts the completion
  reply itself with the agent's key — cursor-agent's shell tool doesn't sign
  with the agent key, so relying on the agent to run `buzz` CLI would fail the
  relay's author check. Reply body = the agent's captured closing turn text
  (`acp.rs: turn_text` / `take_turn_text`, 16 KiB cap, cleared per tool call).
- **Prompt formatting** (`queue.rs`): workflow-tagged prompts get a
  `[Workflow step]` section telling the agent its final message is the
  completion, not to shell out to post one, and to end with a
  ` ```completion ` block (`status` + `outputs`) when later steps need
  structured data from it.
- `ThreadTags` / `parse_thread_tags` **moved to `buzz-core/src/thread.rs`**
  (new file) with the `buzz:workflow` / `buzz:completion-of` tag constants, so
  the relay's resume hook parses threads without depending on `buzz-acp`.

### Desktop

- Form builder: `assign_to_agent` in `ACTION_TYPES`/`ACTION_LABELS`; step
  fields `agent` / `agentPubkey` / `instruction` with YAML round-trip
  (`workflowFormTypes.ts`); new config UI in `WorkflowStepCard.tsx` with the
  new **`AgentCombobox`** (searchable channel-member picker that sets display
  name *and* pubkey atomically, so the identity pin is the default);
  `channelId` plumbed through `WorkflowDialog → WorkflowFormBuilder →
  WorkflowStepCard`.
- Run-status display: `waiting_agent` added to the status unions and badge
  maps (`workflowTypes.ts`, `WorkflowRunTrace.tsx`, `WorkflowDetailPanel.tsx`,
  `hooks.ts`) so agent suspensions render as "waiting agent", not "waiting
  approval".
- The runs-panel Tauri calls (`get_workflow_runs` / `get_run_approvals`
  against the relay HTTP API) are on `feat/workflow-infra`.

## Testing

- Unit: schema round-trip; dispatch suspends / pubkey pin bypasses name
  resolution / unresolved name is a typed failure ("not silent success");
  `completion.rs` well-formed / malformed / truncation; trace timestamps on
  every path. DB tests (+537 lines) cover CAS races and expiry.
- E2E (new `crates/buzz-test-client/tests/e2e_agent_assigned_workflow.rs`,
  `#[ignore]`, needs relay + Postgres + Redis;
  `cargo test -p buzz-test-client --test e2e_agent_assigned_workflow -- --ignored`):
  1. Full loop: trigger → mention with correct `p` tag → suspend → completion
     reply → `completed`, output propagated, duration derivable.
  2. Reply from wrong pubkey does **not** resume (forged-completion guard).
  3. Reply without completion block resumes as `failed` with raw content as
     reason (never stuck).
  4. Expired pending step is swept (backdated `expires_at`, same SQL as the
     sweeper; periodic-task wiring itself not re-exercised).

## Known limitations

- Structured `outputs` from harness-posted completions are **best-effort** —
  the `[Workflow step]` preamble instructs the agent to end with a
  ` ```completion ` block (which the relay parses for `outputs`), but an agent
  that ignores the instruction still resumes the chain with text-only output;
  `{{steps.<id>.output.X}}` then resolves empty.
- Token/cost metrics are self-reported via the optional `usage` field only
  (the relay cannot decrypt kind:44200 metrics — see design doc §6).
