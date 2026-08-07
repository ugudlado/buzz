# Design: Backlog connect — automated token minting + project binding

**Status:** Design only — implement next session
**Date:** 2026-08-07
**Builds on:** [projects-issue-tracker-providers.md](projects-issue-tracker-providers.md) (Slice 1 shipped: manual URL/token/guid in `RepositoryIssueTrackerDialog`)

## Problem

Binding a repo to Backlog today means hand-copying three values (server URL,
`bklg_` token, project guid) into the tracker dialog, and separately
hand-setting `BACKLOG_*` env vars on the Orchestrator persona. Backlog's API
supports doing all of this programmatically.

## Backlog API surface (verified against docs/api/openapi.json)

| Call | Request | Response |
|------|---------|----------|
| `POST /api/auth/login` | `{email, password}` | `{token}` (user token) |
| `GET /api/me` | bearer | `{admin, user: {id, name, memberships: [{projectId, role}]}}` |
| `GET /api/projects` | bearer | `{projects: [{id, guid, path, prefix, …}]}` |
| `POST /api/agents` | `{name}` | `{id, name, grants, createdAt}` |
| `POST /api/agents/{id}/grants` | `{projectId}` | agent with grants |
| `POST /api/agents/{id}/tokens` | `{label?, projectId}` | `{token, scopeProjectId}` — **project-pinned agent token** |

Note the id duality: tokens/grants take numeric `projectId` (or string id);
task routes address by `guid`. `GET /api/projects` returns both — keep both.

## UX (three flows, one dialog + one hook)

### 1. Connect once (email/password or token paste)
`BacklogConnectionDialog` (new, mirrors `GithubConnectionDialog`):
- Server URL + either email/password (→ `POST /api/auth/login` → token) or a
  pasted token; validate with `GET /api/me`; show "Connected as <name>".
- Store `{baseUrl, token, userId, userName}`.

### 2. Project picker instead of guid entry
In `RepositoryIssueTrackerDialog`, when connected: replace the free-text
"Backlog project id" input with a `<select>` fed by `GET /api/projects`
(label `path`, value `guid`; keep a manual fallback input on fetch error).
Optional nicety: default selection by matching project/repo name to `path`.

### 3. One-click agent env provisioning
On the Orchestrator persona (or the tracker dialog, "Provision agent
token…"): pick a Backlog project →
1. `POST /api/agents {name: "buzz-orchestrator"}` (reuse by name if listed in
   `GET /api/agents`),
2. `POST /api/agents/{id}/grants {projectId}` (idempotent),
3. `POST /api/agents/{id}/tokens {label: "buzz", projectId}` → project-pinned
   `bklg_` token,
4. write persona `env_vars`: `BACKLOG_URL`, `BACKLOG_TOKEN` (the *agent*
   token, never the user token), `BACKLOG_PROJECT_ID` (guid) via the existing
   `update_persona` command.

Per-project agent tokens (not the user token) is Orca's pattern and the
right blast-radius: revoking one agent token doesn't kill the user session.

## Where the code goes

- **Rust, not TS, for credentialed calls.** Slice 1 keeps the user token in
  localStorage (flagged debt). This feature is the moment to move Backlog
  auth to the same shape as GitHub: `commands/backlog.rs` with
  `backlog_connect(base_url, email/password|token)`, `backlog_status()`,
  `backlog_disconnect()`, `backlog_list_projects()`,
  `backlog_provision_agent_token(project)` — token in the OS keyring
  (`SecretStore`, keys `backlog.base_url`/`backlog.token`/`backlog.user`).
- Issue reads/writes (`backlogIssues.ts`) can stay in TS initially by
  fetching the token via a `backlog_token()`-style command **or** move
  behind Rust like `github_list_pull_requests`. Prefer moving them — closes
  the localStorage debt completely and keeps one pattern.
- `getBacklogConnection()`/`setBacklogConnection()` become thin wrappers
  over the Rust commands; migrate any existing localStorage value into the
  keyring on first read, then delete it.

## Slices

1. Rust connect commands + keyring + login flow + `/api/me` validation;
   migrate localStorage → keyring. Dialog gets Connect/Disconnect.
2. Project picker in the tracker dialog (`backlog_list_projects`).
3. Agent provisioning (`backlog_provision_agent_token` + persona env write).
4. Optional: `buzz` CLI parity (`buzz backlog connect|provision`) for
   headless agent setup.

## Open questions (decide at implementation)

- Signup flow (`POST /api/auth/signup`) in-dialog, or assume the user exists?
- One Backlog server per app (current assumption) vs. per-community — keyring
  keys would need a community suffix for the latter.
- Should provisioning also set `REDIS_URL`/`ORCHESTRATOR_SKIP_USAGE_CHECK=1`
  on the persona while it's there? (Probably yes — one button, whole env.)

## Tests

- Rust: token mint/grant flows against a mocked HTTP server; keyring
  round-trip; localStorage migration.
- TS: project picker states (loading/error/manual fallback).
- Live (runbook): connect via email/password, bind repo via picker,
  provision Orchestrator env, then UC4/UC5 from
  [ecosystem-e2e-testing.md](ecosystem-e2e-testing.md) without any
  hand-copied values.
