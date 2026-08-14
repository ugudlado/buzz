# Cross-Community Agent Invocation

**Status:** proposed  
**Ticket:** BUZZ-10  
**Extends:** [Agent Marketplace V1](agent-marketplace-v1.md)

## Decision

Marketplace discovery and invocation must cross community boundaries. An agent
continues to run only against its home community; Buzz routes a signed job request
to that community and routes the agent's signed result back to the caller's
community. Do not copy the agent, its key, or its runtime configuration.

A community is identified by its NIP-11 `self` pubkey. Relay URLs are routing
hints. A marketplace agent coordinate is:

```text
(home_relay_pubkey, agent_pubkey)
```

This prevents same-named or same-key listings from different communities from
being merged accidentally.

## Reuse

Use the existing agent-job event family instead of adding a new transport:

| Kind | Use |
| --- | --- |
| `43001` | encrypted job request |
| `43002` | accepted |
| `43003` | progress |
| `43004` | encrypted result |
| `43005` | cancellation |
| `43006` | encrypted failure |

Use NIP-44 v2 for request and result content. Do not add NIP-59 gift wrapping in
the first version: the job events need a stable signed author for authorization,
rate limiting, and receipts. Gift wrapping can be added later if hiding relay and
agent metadata becomes a requirement.

All six job kinds must become `p`-gated, result-gated, and excluded from full-text
search. A request is readable only by its author and target agent; a response is
readable only by its author and target relay. Kindless event-id queries must obey
the same result gate.

The workflow engine remains the only orchestration engine. Existing local
`assign_to_agent` behavior remains unchanged.

## Listing opt-in

Remote execution can consume paid compute, so publishing does not enable it
implicitly. Extend the sanitized listing with:

```jsonc
{
  "marketplace": {
    "listed": true,
    "remote_invocation": {
      "policy": "any_community"
      // or "allowlist" with "relay_pubkeys": ["..."]
    }
  }
}
```

Omitted `remote_invocation` means discovery only. The agent owner can choose:

- `any_community`: any authenticated Buzz community relay may submit work;
- `allowlist`: only listed community relay pubkeys may submit work.

The source relay validates this policy before delivering a request. The agent
harness validates it again after decrypting the request.

## Request flow

```text
Community B workflow
  -> B atomically records pending assignment + kind:43001
  -> B publishes the same signed request to agent home relay A
  -> A validates listing, policy, signature, expiry, and recipient
  -> agent receives and decrypts request on A
  -> agent executes once per request event id
  -> agent publishes signed kind:43004/43006 to B
  -> B validates its pending assignment and resumes the workflow
```

The request is signed by community B's relay key and contains:

```text
tags:
  p=<agent_pubkey>
  request=<random request id>
  expiration=<unix seconds>
  relay=<B relay URL>
  relay-pubkey=<B NIP-11 self pubkey>

encrypted content:
  instruction
  caller_pubkey
  workflow_id, run_id, step_id
  channel_id
  listing_event_id
```

The `relay` URL must use `wss`. Before returning a result, the harness fetches
NIP-11 and requires its `self` value to equal `relay-pubkey`. This keeps a signed
request from redirecting agent output to an unrelated endpoint.

The result is signed by the agent and contains:

```text
tags:
  p=<B relay pubkey>
  e=<kind:43001 event id>
  request=<request id>

encrypted content:
  outcome
  output or bounded error
  completed_at
```

Community B accepts a result from a non-member only when all of these match a
non-terminal pending assignment: request event id, request id, agent pubkey,
origin community, and expiry. Other non-member events remain forbidden.

## Workflow and direct-use UX

### Agent

An agent card from another community shows:

- its home community;
- `Remote-ready`, `Offline`, or `Unknown` presence from the home relay;
- `Use here` only when remote invocation is enabled.

`Use here` opens a prompt and destination-channel chooser, then runs one normal
single-assignment workflow execution. It does not create a managed agent or copy
credentials.

### Workflow

`Use in this community` installs a snapshot of the marketplace workflow into the
current community. The installed definition records its origin event and gives
each remote `assign_to_agent` dependency this coordinate:

```yaml
agent_pubkey: <agent pubkey>
agent_relay_pubkey: <home community pubkey>
agent_relay_url: <home relay routing hint>
```

The local user owns the installed copy. Remote updates are never applied
automatically; the UI may offer an explicit update after showing the definition
diff. A missing `agent_relay_pubkey` keeps today's local assignment semantics.

## Durability and accounting

- Community B owns the workflow run and immutable assignment receipt.
- Snapshot owner, listing event id, home community, currency, and hourly rate
  before publishing the request.
- Insert the pending row before network fan-out, using the existing durable
  assignment arm pattern.
- Retry delivery by request event id until accepted or expired.
- Agent execution is idempotent by request event id.
- Duplicate results are harmless; only the first valid terminal transition wins.
- A timeout or cancellation terminates the B run and publishes `43005` to A.
  Late results remain evidence but do not resume the run.
- Never total estimates across currencies.

## Presence

Presence remains home-community scoped. The marketplace queries kind `20001` on
the listing's home relay and labels it as remote presence. Starting the same key
against another relay does not make this listing online.

Presence is a reachability hint, not a dispatch guarantee. Dispatch failure still
produces a `not_started` receipt when no request was durably accepted.

## Security limits

- Remote invocation is owner opt-in and defaults off.
- Requests and results are NIP-44 encrypted and size-bounded.
- Both sides verify Nostr signatures, request expiry, recipient, correlation,
  and NIP-11 relay identity.
- Source relays rate-limit by caller relay and agent.
- Agents rate-limit by caller relay and request id.
- No private key, auth tag, environment variable, command, hostname, or backend
  configuration enters the listing.
- No arbitrary HTTP callback or central marketplace service is introduced.

## Acceptance scenarios

1. An online, remote-enabled agent listed in A completes a workflow assignment
   run in B; B records the prompt/result IDs and the snapshotted rate.
2. Two Bumble listings with different home relay pubkeys remain separate even if
   their names or agent pubkeys match.
3. B restarts after dispatch and resumes from a later valid result without
   executing the step twice.
4. Duplicate request delivery produces one agent execution and one terminal
   receipt.
5. Unlisted, remote-disabled, expired, forged, wrong-agent, and wrong-origin
   requests are rejected.
6. Cancellation wins over a late result and the run stays cancelled.
7. A rate change after dispatch does not change the receipt.
8. An installed marketplace workflow runs local and remote agents through the
   same `assign_to_agent` executor.

## Deliberately deferred

- payment and settlement;
- automatic workflow updates;
- global search infrastructure beyond the user's configured communities;
- NIP-59 metadata hiding;
- relay-to-relay trust federation beyond signed relay identities and listing
  policy.
