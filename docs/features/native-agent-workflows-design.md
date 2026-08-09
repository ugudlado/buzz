# Native agent-driven workflows: the `AssignToAgent` action

**Status: reviewed design, ready to implement.** Revision 3 — `RunScript`
dropped from this pass (see Decision 2); script-like steps are assigned to a
cheap agent instead. Revision 2 resolved both open decisions and corrected one
broken assumption (metrics, §6) via code-verified research. Supersedes the
external-orchestrator-CLI dependency that a since-removed pack-import feature
depended on for the step-execution piece.

## Context

An earlier, now-removed feature imported external "orchestrator packs" (YAML
step defs + prompts) into Buzz personas, but importing only created
persona/DAG *display* records in Redis — nothing in Buzz actually drove
execution. The generated "coordinator" persona's prompt instead told the
agent to shell out to an external `orchestrator` CLI binary and improvise
step-through in-model.

Decision: drop the external-orchestrator dependency. Extend Buzz's own
`buzz-workflow` YAML engine with one new action type so a workflow YAML can define
"a series of steps with agents" natively, driven end-to-end by Buzz:

- `AssignToAgent` — suspend, @mention a channel-member agent, resume on their
  structured reply.

The external orchestrator's script-vs-agent step split collapses onto this one
primitive: script steps are assigned to a **cheap agent** (a low-cost-model
persona with shell tooling) rather than executed by a dedicated no-LLM runner
(Decision 2 below).

Key simplification found during review: **the outbound half of `AssignToAgent`
already ships.** `ActionDef::SendMessage` dispatches through
`RelayActionSink::send_message` (`crates/buzz-relay/src/workflow_sink.rs:159-172`),
which signs a kind:9 with the relay keypair and — via `resolve_mention_pubkeys`
(`workflow_sink.rs:44-142`) — reverse-parses `@Name` against channel members'
display names and appends the `p` tags that wake an ACP agent
(`crates/buzz-acp/src/filter.rs:390-399` matches on p-tag mention). The new work is
the *inbound* half: suspend the run, match the agent's reply, resume with output.

## Prerequisite fixes (verified in code, block everything downstream)

1. **`finalize_run` fails all suspended runs.** `crates/buzz-workflow/src/lib.rs:237-261`
   marks any run with an `approval_token` as `Failed` ("approval gates not yet
   implemented — see WF-08"). `RunStatus::WaitingApproval` is never written by
   production code, and `create_approval` (`buzz-db/src/workflow.rs:948`) has zero
   production callers (executor.rs:663 has the TODO). The entire grant/resume path
   in `crates/buzz-relay/src/handlers/command_executor.rs` (note: relay, not
   buzz-workflow) is dead code today. **`RequestApproval` is broken end-to-end.**
2. **Suspended steps never get a trace entry.** Only `Completed` and `Skipped`
   push to `execution_trace` (executor.rs:1174, :1204). After resume,
   `{{steps.<approval_id>.*}}` is unresolvable and the step is invisible in the
   trace. Failing steps are also absent (failure paths return without appending).
3. **Likely double-prefixed trace on resume.** `execute_from_step`
   (executor.rs:1037-1045) re-reads `execution_trace` from the DB internally,
   and `resume_workflow_after_approval` (command_executor.rs:1358-1368) *also*
   passes `existing_trace` into `finalize_run`, which prepends it (lib.rs:232-233).
   Verify and fix before reusing this path for agent-step resume.
4. **`update_workflow_run` has no status precondition** (`buzz-db/src/workflow.rs:890-902`
   — unconditional overwrite). The only real double-resume guard is the CAS on the
   approval row (`update_approval_by_stored_hash`, `workflow.rs:1089-1120`,
   `WHERE status = 'pending'`). Agent-step resume must have the same CAS on its
   own row (§3) — do not rely on run-status reads.
5. **No expiry sweeper exists.** `ApprovalStatus::Expired` has zero write sites;
   expiry is only a lazy read-time reject in `handle_approval_grant`
   (command_executor.rs:1064-1068). Add one relay interval task that sweeps both
   `workflow_approvals` and the new `workflow_agent_steps` to `expired`/`failed` —
   fix once, applies to both.
6. **Agent author gate.** Workflow messages are signed by the **relay keypair**
   (`workflow_sink.rs:301-304`), and ACP's `author_allowed`
   (`crates/buzz-acp/src/lib.rs:236-257`) defaults to `OwnerOnly` — a relay-signed
   prompt would be dropped before the mention check runs. Smallest fix: in
   `author_allowed`, accept a non-DM channel message that carries the
   `buzz:workflow` tag and is signed by the connected relay's pubkey (agents can
   fetch it via NIP-11). Without this, `AssignToAgent` prompts silently go
   unanswered for default-configured agents.

## Decision 1 — agent addressing (RESOLVED, simplified)

The doc previously proposed carrying persona slugs and extending
`redis_agent_step_key` to be relay-readable. Research killed that:
`redis_agent_step_key` (`agent_pack.rs:256-258`) is written only by
`buzz workflows import-pack --redis`, read by nothing, and maps step_id → persona
*blueprint*, not a pubkey. There is no channel → slug → pubkey index anywhere, and
building one is unnecessary — the relay already resolves channel-member agents by
**display name** (`resolve_mention_pubkeys` joins `get_members` + `users` rows).

**Design:** `AssignToAgent` carries `agent: String` = the agent's channel display
name. At dispatch, resolve exactly as `resolve_mention_pubkeys` does (refactor its
member-lookup into a shared `resolve_agent(channel, name) -> Option<Pubkey>`);
unresolvable or ambiguous name → typed step failure, not a panic. The prompt
message body is `"@{agent} {instruction}"`, sent through the existing
`send_message` sink so the p-tag falls out for free. Import maps `DagNode.agent_id`
slug → persona display name at import time (personas carry names); no Redis
plumbing, no new authoring concept.

## Decision 2 — script steps (RESOLVED: no `RunScript` this pass, use a cheap agent)

`RunScript` (a headless no-LLM shell executor) is **dropped from this pass**.
Script-like steps in imported packs are instead assigned to a cheap agent — a
persona backed by the lowest-cost model, with shell tooling (`buzz-dev-mcp`),
deployed into the channel like any other agent. Its instruction is the pack
step's command plus "run this and report exit code/stdout in the completion
block."

Why this wins for now:
- Zero new execution infrastructure: no relay-side shell, no sandboxing question,
  no config gate, no second suspend flavor. `AssignToAgent` is the only
  primitive; the engine stays one code path.
- The agent runs where a repo checkout already exists (its harness workspace),
  sidestepping the entire headless-checkout problem.
- Cost is the tradeoff being consciously accepted: a cheap model relaying a
  shell command costs cents, not the engineering of a new subsystem.

Fidelity note for import: pack script steps still import — they become
`AssignToAgent` steps like every other step, with the target agent named right
in the workflow YAML's `agent:` field (defaulted by import, editable by the
author afterward). No hardcoded runner concept anywhere in the engine.

A prior design for a relay-side ephemeral-worktree executor (reusing the git
`hydrate_for_read` S3-CAS → bare-tempdir machinery + hardened subprocess env)
was verified feasible and is preserved in git history (revision 2 of this doc)
— revive it if cheap-agent script execution proves too slow, flaky, or costly.

## Design

### 1. Schema — `crates/buzz-workflow/src/schema.rs`
Add `ActionDef::AssignToAgent { agent: String, instruction: String, timeout: Option<String> }`
(agent = display name per Decision 1; instruction is templated, agent name is not).
`validate()` rejects empty `agent`/`instruction`. No `requires_elevated_authority`
change — agent assignment is no more privileged than `SendMessage`.

### 2. Fix suspend handling — `crates/buzz-workflow/src/lib.rs` + `executor.rs`
- Make approval suspension real: create the `workflow_approvals` row (the unused
  `create_approval` finally gets its caller) and set `RunStatus::WaitingApproval`
  in `finalize_run` instead of failing.
- Add `StepResult::AwaitingAgentReply { prompt_event_id, agent_pubkey }` and
  propagate through `ExecutionResult` alongside the approval token. Push a trace
  entry for suspended steps (`status: "waiting"`) so they're visible (fixes
  prerequisite #2 for both flavors).
- Fix the double-prefix trace bug (prerequisite #3) while in here.

### 3. New table + relay hook for agent-step suspension
Migration `workflow_agent_steps`, mirroring `workflow_approvals`
(`migrations/0001_initial_schema.sql:411-436`): `community_id, prompt_event_id,
run_id, step_id, step_index, agent_pubkey, status, output, expires_at`. CRUD in
`buzz-db/src/workflow.rs` next to approval CRUD, including a CAS update
(`WHERE status = 'pending'`) exactly like `update_approval_by_stored_hash` —
this is the double-resume guard (prerequisite #4).

Trigger point: `crates/buzz-relay/src/handlers/event.rs` (~line 520-558), the
existing post-fanout block that already spawns `workflow_engine.on_event(...)` —
add a sibling check: incoming kind:9 whose NIP-10 reply `e`-tag matches a pending
`workflow_agent_steps.prompt_event_id` **and whose author pubkey equals the row's
`agent_pubkey`** (otherwise any thread participant forges a completion). For the
reply-tag parse, lift `parse_thread_tags` (`crates/buzz-acp/src/queue.rs:854-892`)
into `buzz-core` — it's a pure `nostr::Event → struct` function with no ACP
dependencies; the relay currently has no read-side NIP-10 parser. The
`buzz:workflow` recursion guard (event.rs:526) only tags relay-outbound messages,
not agent replies — no conflict.

Resume ordering (prerequisite #4/#5 hardening): CAS the row `pending → done`
*first*, then spawn resume. A crash between CAS and resume leaves a `done` row
with a stuck `waiting` run — the sweeper (below) also re-detects that state and
retries the resume, closing the crash window that exists today for approvals.

Expiry sweeper: one relay interval task (piggyback an existing periodic task or a
new `tokio::time::interval`) that marks overdue `workflow_approvals` and
`workflow_agent_steps` rows expired and finalizes their runs as failed.

### 4. Completion parsing — new `crates/buzz-workflow/src/completion.rs`
Parse a fenced ` ```completion ` YAML block from the reply content into
`{status, outputs, reason, usage?}`. Malformed/missing block → resume with
`status: failed`, `reason` = raw content (never leave the run stuck). `usage` is
an optional `{input_tokens, output_tokens, cost}` the agent self-reports (see §6
for why the relay can't get this any other way). No new signed event kind for
completions in this phase — plain chat reply matches how agent output is treated
today.

On successful parse: **write the step's parsed output into `step_outputs` and the
trace entry before resuming** so `{{steps.<id>.output.X}}` works downstream. This
is new logic — the approval-resume path never records output for its own step
(verified: command_executor.rs:1334-1346 rebuilds outputs from trace only).

### 5. Import path (historical — feature removed)
An earlier revision of this design planned to wire the orchestrator
pack-import path (`agent_pack.rs`, `cmd_import_orchestrator`, desktop
`orchestrator_import.rs`) to emit `AssignToAgent` steps automatically from an
imported pack's DAG. That import feature has since been removed entirely
(unused after `AssignToAgent` shipped as the native replacement), so this
integration was never built and no longer applies. Workflows using
`AssignToAgent` are authored directly as YAML.

## 6. Metrics (duration, tokens, cost)

**Correction to the previous draft:** kind:44200 `AgentTurnMetricPayload` events
are NIP-44 encrypted agent-key → **owner** pubkey
(`crates/buzz-acp/src/pool.rs:3753-3757`,
`buzz-core/src/agent_turn_metric.rs:168-175`). The relay holds neither key and
**cannot read token/cost from them**; 44200 is also p-gated and result-gated for
queries. The previous plan ("relay looks up the turn's 44200 event and folds
numbers into the trace") is not implementable. Revised:

- **Duration (server-side, all action types):** extend every `execution_trace`
  entry (executor.rs push sites) with `started_at`/`completed_at` (RFC3339,
  captured around `dispatch_action`; for `AssignToAgent`, `completed_at` is set
  at resume). This alone lights up the existing desktop duration UI —
  `TraceEntry.startedAt/completedAt` and `formatDuration` in
  `desktop/src/features/workflows/ui/` already exist and render empty today.
  Also add a `status: "failed"` trace entry on failure paths (currently absent).
- **Tokens/cost, primary source:** the optional `usage` field in the agent's
  completion block (§4) — self-reported, stored in the trace entry's output under
  `usage`. Missing → `usage: null`; never block resume on it.
- **Tokens/cost, display enrichment (desktop-only):** the desktop *is* the owner
  and can decrypt 44200 events client-side. The run-detail view may correlate
  44200 events in the step's channel/time window to fill gaps where the agent
  didn't self-report. Display-only, not persisted — no second source of truth.
- Total run metrics = sum of per-step trace `usage` at query time (desktop or
  `buzz workflows runs`), no stored running-total column.
- **Non-goal:** exact multi-turn attribution when one reply spans several ACP
  turns — accept the self-reported/windowed number; exact attribution needs
  ACP-level step tagging that doesn't exist.

## 7. End-to-end verification scenario (small real feature, backlog-driven)

Manual acceptance test after automated tests pass:

1. Create a test channel with personas deployed: `buzz channels create --name
   workflow-e2e-test --template <team-with-coordinator+specialist>`
   (`cmd_create_channel_from_template` only adds *already-deployed* instances —
   deploy via desktop/managed_agents first). Deployed agents must have the
   author-gate fix (prerequisite #6) or an allowlist including the relay pubkey.
2. Add a Backlog ticket describing a small concrete feature (e.g. "add a
   `--dry-run` flag to `buzz workflows trigger`").
3. Trigger an imported `patch`/`feature` workflow in that channel with the ticket
   id as trigger input — `AssignToAgent` steps for design/implement/verify, and
   the git/ticket-transition script steps assigned to a cheap-model agent via
   the YAML `agent:` field.
4. Confirm: mention reaches the right agent each step, reply resumes the run,
   `{{steps.<id>.output}}` carries forward, run reaches `Completed`.
5. Confirm metrics: per-step duration non-null in the desktop trace view,
   token/cost populated where the agent self-reported (or via desktop 44200
   decryption), total surfaced in run detail.

## Test plan

- `buzz-workflow` unit: schema round-trip for `AssignToAgent`; `dispatch_action`
  returns `AwaitingAgentReply`; unresolvable/ambiguous agent name is a typed
  error (no new `unwrap()/expect()`); `completion.rs` handles well-formed,
  malformed, and usage-bearing replies; trace entries carry timestamps for every
  action type and a `failed` entry on failure.
- `buzz-db` integration (`just test`): `workflow_agent_steps` CRUD + CAS
  (concurrent update, only one wins), mirroring approval tests.
- Relay/e2e (`buzz-test-client`): trigger workflow with one `AssignToAgent` step
  → assert mention event with correct `p`-tag → synthetic reply with completion
  block + NIP-10 reply tag → assert resume, output propagation, `Completed`,
  trace duration + usage. Negative: reply from wrong pubkey does **not** resume;
  reply without completion block resumes as failed with reason; expired pending
  step swept to failed (covers approvals too).
- Manual: §7 scenario.

## Verification

- `cargo test -p buzz-workflow`; `just test` (Postgres+Redis) for DB + e2e.
- §7 manual run with metrics visible in the desktop run detail view.

## Implementation order

1. Prerequisites #1-#5 (suspend/resume repair + sweeper) — makes
   `RequestApproval` actually work, independently shippable.
2. ACP author-gate fix (prerequisite #6) — small, independent.
3. `AssignToAgent`: schema + `workflow_agent_steps` + relay hook + completion
   parser + trace timestamps.
4. Import path (`WorkflowDef` generation, all steps as `AssignToAgent`) +
   metrics display.

No open decisions remain. One watch item during implementation: confirm the
double-prefix trace suspicion (prerequisite #3) empirically.
