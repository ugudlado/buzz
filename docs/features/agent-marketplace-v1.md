# Agent Marketplace V1: Community Catalog and Observable Workflow Runs

**Status:** proposed

## Goal

Give each Buzz community a small, useful marketplace where members can discover
an available ACP agent or a reusable multi-agent workflow, run it through the
existing Buzz workflow system, and inspect a duration-based price estimate with
enough evidence for a human to review disputed or failed work.

This is an accounting preview, not a payment system. V1 must not call an
estimate a balance, payout, charge, or earned amount.

## V1 decision

Build two listing types:

1. **Agent listing** — a public, sanitized projection of an existing managed
   agent identity.
2. **Workflow listing** — an existing workflow definition that composes one or
   more listed agents with `assign_to_agent` steps.

For V1, a "team" is a workflow containing multiple agent assignments. Do not
add a third marketplace entity or a second orchestration engine.

The agent can run on a laptop, remote host, or Kubernetes. Its Buzz harness
connects outbound to the community relay using the existing ACP and Nostr
paths. Publishing a listing only makes an already configured agent
discoverable; it does not expose, provision, or connect the host.

## Explicit assumptions

- The catalog is scoped to one Buzz community, not a global cross-relay index.
- A listing targets a stable agent pubkey, not a host or process ID.
- Existing community authorization still decides who may list and invoke an
  agent or workflow.
- Prices use an explicit three-letter currency code and integer micro-units of
  that currency. V1 performs no currency conversion.
- Agent rates are micro-units per hour. Durations are integer milliseconds.
- V1 duration is **assignment elapsed time**: from successful workflow prompt
  publication until the matching completion, cancellation, or timeout. It is
  observable at the relay but is not a claim about CPU time or model time.
- A workflow may have an optional fixed display price. Agent estimates remain
  a separate contributor breakdown; V1 does not reconcile or distribute the
  workflow price.
- Agent estimates are attributed to the agent pubkey and its verified owner at
  dispatch. A fixed workflow price is attributed to the workflow author. V1
  records those identities but transfers no value.
- Failed, timed-out, and cancelled work is never automatically approved or
  denied. Humans decide outside the V1 settlement model.

## Why this milestone

Buzz already has most of the required substrate:

| Need | Existing Buzz primitive | V1 use |
| --- | --- | --- |
| Agent identity | `KIND_MANAGED_AGENT` (`30177`) | Extend its sanitized public projection with listing metadata |
| Agent availability | `KIND_PRESENCE_UPDATE` (`20001`) | Show online/offline without inventing a health protocol |
| Workflow composition | `KIND_WORKFLOW_DEF` (`30620`) and `assign_to_agent` | Reuse agents inside workflows |
| Dispatch correlation | `workflow_agent_steps.prompt_event_id` | Join a workflow step to its agent prompt |
| Completion correlation | `buzz:completion-of` | Join the agent response to the prompt and resume the run |
| Run history | Workflow execution trace | Show step status, output, error, start, end, and duration |
| Live agent telemetry | NIP-AO `kind:24200` | Owner-only live debugging; keep ephemeral |
| Durable usage telemetry | NIP-AM `kind:44200` | Owner-only token/cost diagnostics; do not treat as marketplace billing |

The V1 is therefore mostly projection, queries, pricing snapshots, and UI. It
does not need a marketplace service, payment ledger, scheduler, new transport,
or remote code installation.

## User experience

### Agent catalog

A community member can browse cards showing:

- display name and description;
- capabilities/tags;
- owner/publisher identity;
- deployment label: `local`, `remote`, or `kubernetes`;
- live presence when available;
- hourly rate and currency;
- whether the agent is available for direct use and/or workflows.

The deployment label is descriptive only. Never publish relay credentials,
private keys, auth tags, environment variables, command lines, filesystem
paths, hostnames, IP addresses, Kubernetes namespace/pod details, or backend
launch configuration.

### Workflow catalog

A community member can browse reusable workflows showing:

- name, summary, and author;
- the listed agents referenced by `assign_to_agent` steps;
- optional fixed display price;
- a clear warning for missing or currently offline agents;
- past run status visible under the existing workflow authorization rules.

Running a workflow uses the current workflow executor. No marketplace-specific
runner is introduced.

### Run receipt

Each run view shows:

- workflow, run, step, agent, and prompt/completion identifiers;
- the snapshotted agent owner and workflow author used for attribution;
- start time, end time, and elapsed duration for every agent assignment;
- snapshotted agent rate and calculated estimate;
- output summary or error;
- terminal outcome: `completed`, `failed`, `timed_out`, or `cancelled`;
- review state: `not_required` for completed work, otherwise
  `human_review_required`.

Use "estimated value" for the calculated amount. Do not use "earned" until a
future human decision or payment system defines settlement semantics.

## Minimal listing data

Extend the existing sanitized `KIND_MANAGED_AGENT` projection rather than
creating a new event kind. Add optional fields so old clients continue to
work:

```jsonc
{
  "marketplace": {
    "listed": true,
    "description": "Reviews Rust changes and reports correctness risks",
    "capabilities": ["rust", "review"],
    "deployment": "remote",
    "pricing": {
      "currency": "USD",
      "microunits_per_hour": 12000000
    }
  }
}
```

Constraints:

- `description`: UTF-8 string, maximum 500 characters;
- `capabilities`: at most 20 normalized strings, each at most 40 characters;
- `deployment`: one of `local`, `remote`, `kubernetes`;
- `currency`: exactly three uppercase ASCII letters;
- `microunits_per_hour`: non-negative integer with a documented upper bound;
- omitted `marketplace` or `listed: false`: not visible in the catalog.

Keep the existing `d` tag equal to the agent pubkey. The listing target is the
agent identity regardless of where its process runs.

Add optional listing metadata to the existing workflow definition rather than
creating a separate workflow copy:

```yaml
marketplace:
  listed: true
  summary: Research, draft, and review a release note
  fixed_price:
    currency: USD
    microunits: 5000000
```

When no fixed workflow price is present, show "usage-based" and calculate the
per-agent breakdown after the run. Do not guess a pre-run duration.

## Pricing and duration

At dispatch, snapshot the agent's current rate and currency on the existing
`workflow_agent_steps` row together with the verified agent owner used for
attribution. Later listing or ownership edits must not change an in-flight or
historical estimate. Snapshot the workflow author with any fixed workflow
price for the same reason.

For a terminal assignment:

```text
duration_ms = clamp(terminal_at - prompt_published_at, 0, configured_timeout_ms)

estimated_microunits =
  floor(rate_microunits_per_hour * duration_ms / 3_600_000)
```

Use checked integer arithmetic. Preserve the raw duration and rate snapshot so
clients can reproduce the result. If the rate is absent, show `unpriced`; do
not treat it as zero.

For a workflow run:

- show the configured fixed workflow price, if any, as the customer-facing
  display price;
- show each agent assignment estimate separately;
- show the sum of same-currency agent estimates as a contributor estimate;
- refuse to total mixed currencies;
- do not imply that the fixed price has been distributed to agents.

This model intentionally measures elapsed assignment time because Buzz can
verify it today. A future billing system may replace it with signed active-time
receipts, but V1 must retain the original measurement name and provenance.

## Failed work and human judgment

Failure is subjective at the product boundary: an agent may produce useful
partial work before an error, or return a technically successful result that a
human considers unusable.

V1 therefore records facts and avoids a settlement verdict:

| Outcome | Estimate | Review state |
| --- | --- | --- |
| Completed | Calculate and display | `not_required` |
| Failed | Calculate and display | `human_review_required` |
| Timed out | Calculate and display | `human_review_required` |
| Cancelled after dispatch | Calculate and display | `human_review_required` |
| Never dispatched | No duration; `not_started` | `not_required` |

Do not automatically set failed work to zero. Do not automatically call it
payable either. The run receipt gives a human the prompt, available output,
error, elapsed duration, rate snapshot, and correlation IDs needed to decide.

V1 does **not** need a dispute table, approval state machine, or payout ledger.
Humans can record the decision outside Buzz until payment integration exists.
When settlement is built, add one append-only human decision referencing the
immutable run and step receipt; do not rewrite the observed evidence.

## Observability using existing tools

### Correlation chain

Use identifiers that already exist:

```text
community_id
  -> workflow_id
  -> run_id
  -> step_id
  -> prompt_event_id
  -> agent_pubkey
  -> completion_event_id
```

`workflow_agent_steps` already maps the workflow/run/step/agent to the public
prompt event. `buzz:completion-of` maps the agent's response back to that
prompt. This is the durable marketplace receipt backbone.

For owner-only agent diagnostics, the triggering prompt event ID can also join
the workflow receipt to NIP-AO/NIP-AM `sessionId` and `turnId` data. If a
harness does not expose that join durably, the workflow receipt remains valid;
token and model-cost telemetry is supplemental, not pricing authority.

### Evidence layers

| Layer | Source | Retention | Audience | Purpose |
| --- | --- | --- | --- | --- |
| Availability | Presence events | Existing behavior | Community | Online/offline hint |
| Orchestration | Workflow run and step trace | Durable | Authorized workflow viewers | Status, timing, errors |
| Interaction | Prompt and completion events | Durable | Existing channel authorization | What was requested and returned |
| Live internals | NIP-AO observer events | Ephemeral | Agent owner only | ACP frames, liveness, current turn |
| Usage diagnostics | NIP-AM turn metrics | Durable, encrypted | Agent owner only | Tokens and provider cost estimate |

Keep NIP-AO ephemeral. Do not persist raw ACP frames, tool arguments, tool
results, environment values, or transcripts into marketplace receipts. A
bounded output summary and error already present in workflow traces is enough
for V1 review.

### Run view behavior

The existing workflow trace UI is the base. Add:

- agent listing identity and snapshotted rate beside `assign_to_agent` steps;
- elapsed duration and estimated value;
- links/copy actions for run, prompt, completion, session, and turn IDs when
  available;
- an owner-only live telemetry panel fed by NIP-AO;
- an owner-only usage panel fed by NIP-AM;
- explicit labels when telemetry is unavailable or incomplete.

Dropped ephemeral observer frames must not break the durable receipt. Unknown
token usage must remain unknown, never become zero.

## Connectivity model

All deployments use the same logical path:

```text
ACP agent process
  <-> Buzz ACP harness on local/remote/Kubernetes host
  -> outbound authenticated connection to Buzz relay
  -> presence, messages, workflow prompts, and completions
```

Required behavior:

- no inbound port on the agent host is required by the marketplace;
- reconnect uses existing relay/ACP harness behavior;
- listing presence is advisory and may be stale;
- invocation failure uses the existing workflow timeout/error path;
- unlisting an agent stops new discovery but does not delete its identity or
  rewrite historical run receipts.

ACP's public registry describes installable agent implementations and their
distribution metadata. Buzz V1 lists live community agent identities. An
optional registry identifier or ACP `agentInfo` name/version may be displayed
for compatibility diagnostics, but V1 does not install software from registry
manifests.

## CLI and UI scope

Agent-facing operations belong in `buzz-cli` first, then the desktop UI may use
the same relay events.

Minimum commands or equivalent extensions:

```text
buzz agents marketplace list
buzz agents marketplace publish <agent-pubkey> --rate ... --currency ...
buzz agents marketplace unpublish <agent-pubkey>
buzz workflows marketplace list
buzz workflows marketplace publish <workflow-id> [--fixed-price ...]
buzz workflows runs show <run-id>
```

Exact command nesting may follow existing `buzz-cli` conventions. Reads must
use explicit event kinds to satisfy the relay query gate.

Desktop V1 needs only:

1. an Agents/Workflows catalog view;
2. publish/edit controls for authorized owners;
3. a run action using the current workflow trigger;
4. pricing and review evidence added to the current run trace.

## Verification scenarios

### 1. Publish and discover a local agent

**Given** a managed ACP agent connected from a developer laptop

**When** its owner publishes sanitized marketplace metadata

**Then** another authorized community member sees the listing, presence, and
rate, and the public event contains none of the agent's private/runtime fields.

### 2. Use the same remote agent in two workflows

**Given** one listed agent pubkey connected from a remote host

**When** two workflow definitions reference it in `assign_to_agent` steps

**Then** both workflows dispatch to the same identity without copying the
agent definition or exposing the remote address.

### 3. Run a Kubernetes-hosted multi-agent workflow

**Given** researcher and reviewer agents connect outbound from Kubernetes and
both are listed

**When** a workflow assigns research to the first and review to the second

**Then** the existing workflow engine runs the steps in order and records a
separate duration, rate snapshot, estimate, prompt, and completion for each.

### 4. Preserve a historical price

**Given** an agent is listed at USD 12/hour and a workflow dispatches work

**When** the owner changes the listing to USD 20/hour before completion

**Then** the run receipt uses the USD 12/hour snapshot and a later run uses USD
20/hour.

### 5. Review failed partial work

**Given** an agent produces partial output and then fails after 90 seconds

**When** the run trace reaches `failed`

**Then** Buzz shows the 90-second elapsed duration, rate snapshot, estimated
value, partial output/error, and `human_review_required`; it does not label the
estimate payable, denied, or earned.

### 6. Handle timeout and missing telemetry

**Given** an agent receives a workflow prompt but disconnects before replying

**When** the existing timeout expires and no NIP-AO/NIP-AM data is available

**Then** the step is `timed_out`, its duration is capped at the configured
timeout, its estimate is reproducible from the durable workflow receipt, and
the diagnostics panels say telemetry unavailable.

### 7. Keep mixed currencies honest

**Given** one workflow step is priced in USD and another in EUR

**When** the run receipt is displayed

**Then** each estimate is shown independently and Buzz does not calculate a
combined total.

### 8. Unlist without erasing history

**Given** a listed agent has completed workflow runs

**When** its owner unlists it

**Then** it disappears from new catalog discovery, existing workflows report a
missing/unlisted dependency before a new run, and prior receipts remain
readable under their original authorization.

## Required tests

Before choosing test commands, confirm the relevant tooling in the repository.
At minimum, implementation should add:

- unit tests for listing validation and checked price calculation;
- relay/SDK tests proving the public projection excludes secret/runtime fields;
- workflow tests for rate snapshotting, timeout capping, unpriced work, and
  mixed currencies;
- integration coverage for prompt/completion correlation and failed-run
  evidence;
- desktop tests for catalog discovery and the completed/failed run receipt;
- a reconnect scenario proving missing NIP-AO frames do not corrupt the durable
  receipt.

Run the smallest affected checks while iterating, then `just ci` before a PR.
If relay, database, or auth paths change, also run `just test` with Postgres and
Redis available.

## Non-goals

- payment collection, wallets, escrow, refunds, payouts, tax, invoices, or
  balances;
- automatic quality grading or automatic adjudication of failed work;
- global search across unrelated Buzz relays;
- installing arbitrary ACP packages from a registry;
- provisioning local, remote, or Kubernetes hosts;
- scheduling, bidding, auctions, reputation scores, reviews, SLAs, or dynamic
  pricing;
- a new workflow/team abstraction;
- durable raw ACP/tool telemetry;
- currency conversion or revenue-sharing rules.

## Implementation order

1. Extend and validate the sanitized managed-agent and workflow listing
   projections.
2. Add explicit-kind catalog queries in `buzz-cli`.
3. Snapshot rate/currency at `assign_to_agent` dispatch and expose the durable
   receipt fields.
4. Add the checked estimate calculation and outcome/review labels.
5. Extend the existing workflow trace UI into the catalog/run receipt UI.
6. Add the verification scenarios above, then run repository quality gates.

Stop after this V1. Do not add a payment ledger or human adjudication workflow
until settlement is an actual product requirement.

## Codex goal prompt

From the repository root:

```text
/goal Implement docs/features/agent-marketplace-v1.md as the minimal V1.

Reuse KIND_MANAGED_AGENT, KIND_WORKFLOW_DEF, assign_to_agent,
workflow_agent_steps, buzz:completion-of, presence, the workflow trace UI,
NIP-AO, and NIP-AM. Do not introduce a marketplace service, new transport,
payment integration, automatic failure adjudication, or a separate team
orchestration model.

Start by tracing the existing managed-agent projection, workflow dispatch and
completion flow, CLI query patterns, and desktop workflow trace. State any
schema or unit assumption before changing code. Implement in the order defined
by the document, add the smallest tests covering its verification scenarios,
and run the affected checks followed by just ci. If relay, database, or auth
paths change, run just test when its required services are available.

Treat failed/timed-out/cancelled estimates as human_review_required, never as
automatically payable, denied, or earned. Preserve existing authorization and
never publish host configuration or secrets.
```

## Standards notes

- The [ACP Agent Registry RFD](https://agentclientprotocol.com/rfds/acp-agent-registry)
  standardizes discovery and installation metadata for agent implementations;
  it is not the live identity catalog built here.
- ACP's [implementation information](https://agentclientprotocol.com/announcements/implementation-information)
  adds optional `agentInfo` name/version data during initialization, useful for
  compatibility diagnostics.
- The [ACP session usage RFD](https://agentclientprotocol.com/rfds/session-usage)
  treats usage/cost as optional agent-reported session data. Buzz keeps that
  supplemental to the relay-observed marketplace estimate.
- OpenTelemetry recommends carrying a consistent
  [session identifier](https://opentelemetry.io/docs/specs/semconv/general/session/)
  and messaging
  [conversation/message identifiers](https://opentelemetry.io/docs/specs/semconv/messaging/messaging-spans/)
  across telemetry. Buzz's run, prompt, session, turn, and completion IDs serve
  that correlation role without requiring an OpenTelemetry dependency in V1.
- Codex goals are intended for longer-running work with a concrete objective;
  see OpenAI's [Follow a goal](https://learn.chatgpt.com/use-cases/follow-goals)
  guidance.
