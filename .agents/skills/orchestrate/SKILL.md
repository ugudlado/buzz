---
name: orchestrate
description: Drive a repository workflow through the orchestrator CLI and Buzz step agents. Use only when acting as Team Lead and a user explicitly asks to start, resume, or complete a development workflow or ticket.
---

# Orchestrate a Buzz workflow

Use the orchestrator as the only workflow state machine. Its dispatch output
contains the current role, agent name, and pinned Buzz pubkey from
`roster.yaml`. Do not maintain a second plan, routing table, retry counter, or
completion state in chat.

## Preconditions

1. Run only when the active persona is Team Lead.
2. Treat the current ACP working directory as the shared repository checkout.
   Resolve `REPO_ROOT` with `git rev-parse --show-toplevel`; never post it.
3. Resolve the roster from `${ORCHESTRATOR_ROSTER:-$REPO_ROOT/roster.yaml}`.
   Stop if it is missing, malformed, lacks the dispatched step, or has no
   64-character pubkey for its agent.
4. Require the orchestrator and ticket environment already provisioned by the
   harness. Never print secrets or invent fallback credentials.
5. Treat the dispatched `agent_pubkey` as authoritative. The display name is
   presentation only. Stop if the pubkey is not a current channel member.
6. Shell variables do not persist between tool calls. Re-declare `REPO_ROOT`,
   `STATE`, and `ORCHESTRATOR_ROSTER` in every shell invocation that uses them.
7. Set `ORCHESTRATOR_REMOTE_SESSION=1` for every orchestrator command. Buzz
   agents already share the channel checkout; do not create or name a separate
   worktree in chat.

## Start or resume

Treat `pick up <ticket-id>` as complete workflow intent. Fetch ticket context
through the configured ticket tool, then choose `bugfix` when the ticket is
labeled as a bug or clearly describes a defect; otherwise choose `feature`
when it clearly describes new work. Ask one short question only when the
ticket does not make the workflow clear. A workflow explicitly named by the
user wins. Seed the run without starting local model subprocesses:

```bash
STATE=$(ORCHESTRATOR_REMOTE_SESSION=1 orchestrator run "$TICKET_ID" \
  --ticket-id "$TICKET_ID" --schema "$WORKFLOW" \
  --repo "$REPO_ROOT" --seed-only | tail -1)
```

Keep that state path for the thread. If the thread already contains a run ID
or state path for the same ticket, resume it instead of seeding another run.
For a run ID without a path, resolve and verify the default state file:

```bash
STATE="${WORKFLOW_STATE_DIR:-$HOME/.orchestrator/state}/$RUN_ID.yaml"
test -f "$STATE"
```

## Dispatch

Call:

```bash
ORCHESTRATOR_REMOTE_SESSION=1 \
ORCHESTRATOR_ROSTER="${ORCHESTRATOR_ROSTER:-$REPO_ROOT/roster.yaml}" \
  orchestrator next "$STATE"
```

Interpret only the CLI result:

- Exit 0 with no agent action: an inline step ran; call `next` again.
- Exit 0 with a JSON object containing `model`: require `role`, `agent`, and
  `agent_pubkey` in the same object. Post one short human handoff in the current
  thread with readable `@<agent>` text and pass `agent_pubkey` explicitly as the
  mention identity. Name the ticket and work naturally, for example:
  `@Explorer Please handle the diagnose step for BUZZ-15 and report back here.`
  Never paste `instruction`, a charter, workflow metadata, or paths. The
  agent's persona and installed `workflow-step` skill own how the role works.
  End the turn and wait for that agent's human reply. Never re-resolve the name.
- Exit 1: post the workflow report and stop.
- Exit 2: post the blocker and wait for human input.
- Exit 3 or higher: surface the CLI error and stop after three identical
  failures.

Some ticket-transition script steps currently require caller-supplied values
on the same `next` invocation:

```bash
ORCHESTRATOR_REMOTE_SESSION=1 TICKET_SYNC_STATUS="In Progress" TICKET_SYNC_LOG_PREFIX=ticket-start orchestrator next "$STATE"
ORCHESTRATOR_REMOTE_SESSION=1 TICKET_SYNC_STATUS="Review" TICKET_SYNC_LOG_PREFIX=ticket-review orchestrator next "$STATE"
ORCHESTRATOR_REMOTE_SESSION=1 TICKET_SYNC_STATUS="Verify" TICKET_SYNC_LOG_PREFIX=ticket-qa orchestrator next "$STATE"
```

Use the matching command only when that script is the state's next step.

## Record a worker reply

Treat the worker's human reply as a notification only. Never parse completion
data from chat. Read this relative path in the shared channel checkout without
posting it. Normalize the ticket ID to lowercase for the directory name:

```text
spec/changes/<ticket-id-lower>/completions/<step-id>.yaml
```

If it is missing, ask the worker in one plain sentence to write it and wait.
Do not reconstruct the payload from prose. Validate that its `step_id` matches
the dispatched step and that `status` is `completed` or `failed`, then record
its status, artifacts, and outputs using the dispatched phase, attempt, and
model value:

```bash
REPO_ROOT="<repository-root>"
STATE="<state-path>"
printf '%s\n' '<done-payload-json>' | ORCHESTRATOR_REMOTE_SESSION=1 orchestrator done "$STATE"
```

The payload must carry the dispatched `step_id`, `phase`, `attempt`, status,
model alias in `agent`, usage, and the file's artifacts and outputs. After
`done` succeeds, call `next` again. If the CLI dispatches a prior step, explain
the rework naturally and follow the new dispatch; do not implement loop-back
logic yourself.

## Chat behavior

Post only run start, observable transitions, rework, blockers, agent dispatches,
and the final report. Write these as short teammate updates. Never paste
completion YAML/JSON, workflow-state fields, state paths, phase/attempt/model
metadata, token usage, cost, or raw CLI output into chat. Do not restate a
worker's reply before advancing.
