# Projects: GitHub repos + pluggable issue trackers (Backlog)

**Status:** Slice 1 shipped — issue-tracker seam + Backlog provider + tracker UI
**Date:** 2026-08-07
**Related:** [VISION_PROJECTS.md](../../VISION_PROJECTS.md), NIP-MP, `~/code/backlog` (server), `~/code/orca` (reference integration)

## Problem

Projects supported only Buzz-hosted repos and Buzz-native (NIP-34 kind:1621)
issues. Users want (a) GitHub repos as first-class project members and (b)
issue tracking in Backlog (the Bun HTTP task server, integrated the same way
Orca consumes it: plain REST, bearer token, no polymorphic provider interface).

## What exists / shipped

### Repos (mostly pre-existing)

- The Add-repository dialog already accepts an external clone URL; a
  `https://github.com/<owner>/<repo>` URL is published verbatim in the 30617
  `clone` tag and renders with the GitHub host badge
  (`lib/projectRepoHost.ts` classifies `buzz | external(host)`).
- Rust allows GitHub URLs only for **clone + open-terminal**
  (`project_git_exec.rs::validate_github_clone_url`), anonymous public only.
  Snapshot/sync/push/diff reject non-relay hosts — deliberate security
  boundary (keeps the nsec away from github.com). Widening it needs a real
  credential store; not done in this slice.

### Issue tracker seam (new)

- `Repository.issueTracker: {kind:"buzz"} | {kind:"backlog", project}` —
  parsed from a `buzz-issue-tracker` tag on the kind:30617 announcement:
  `["buzz-issue-tracker", "backlog", "<backlog project guid>"]`. Absent tag =
  Buzz issues (default).
- `backlogIssues.ts` — Backlog provider: connection in localStorage
  (`buzz-backlog-connection.v1`, `{baseUrl, token}`), project-nested REST
  (`/api/projects/{guid}/tasks[...]`, the post-BKG-547 shape), maps tasks →
  `ProjectIssue` with synthetic ids `backlog:<taskId>` (never collide with
  64-hex event ids in `${repoAddress}:${issue.id}` dedupe keys).
- Dispatch points (all of them):
  - `projectWorkItems.ts::fetchProjectsWorkItems` — backlog-tracked repos get
    issues from the provider (one GET per distinct Backlog project); PRs stay
    relay-native; backlog failure → `failedSections: ["backlog-issues"]`
    without dropping relay issues.
  - `hooks.ts::fetchProjectIssues` (per-repo panel) — routes by tracker.
  - `issueMutations.ts::publishProjectIssue` — creates a Backlog task.
  - `hooks.ts::createProjectIssueComment` — posts a task comment.
- UI: repo owner sees an **Issues** button in `ProjectRepositoryManagement`
  → `RepositoryIssueTrackerDialog` (tracker select + Backlog guid/URL/token);
  saving republishes the 30617 with the tag
  (`buildRepositoryIssueTrackerTemplate` preserves all other tags verbatim).

## GitHub connection (Slice 2 — shipped)

- Rust `commands/github.rs`: `github_connect` (PAT validated against
  `GET /user`), `github_connect_from_gh_cli` (`gh auth token` import),
  `github_connection_status`, `github_disconnect`,
  `github_list_pull_requests`. Token + login live in the OS keyring
  (`SecretStore`, keys `github.token`/`github.login`) — the token never
  reaches the webview.
- Authenticated clone: `build_git_clone_auth_config` injects the token for
  github.com remotes via a `http.https://github.com/.extraheader` basic-auth
  header carried in `GIT_CONFIG_*` env vars (actions/checkout pattern, never
  argv). Private-repo clone works once connected; other git ops on GitHub
  remotes remain gated.
- PR provider: `Repository.githubRepo` (derived from a
  `https://github.com/<owner>/<repo>` clone URL) routes pull requests through
  `githubPullRequests.ts` → Rust API call, mapped to the shared
  ProjectPullRequest shape with `github:`-prefixed ids. GitHub-hosted repos
  skip the relay kind:1618 fetch. Not-connected resolves empty (not a load
  failure); fetch errors surface as a `github-pull-requests` failed section.
- UI: a GitHub button on repository management (visible for GitHub-hosted
  repos) opens `GithubConnectionDialog` (paste PAT or import from gh CLI,
  shows the connected login, disconnect). `PullRequestReviewCard` renders
  "Open on GitHub" instead of relay review/merge actions for `github:` PRs.
- Not done: GitHub PR comments/reviews in the timeline, files-changed diff
  for GitHub PRs (needs a local clone or the compare API), fetch/push/sync
  on GitHub remotes, creating GitHub PRs/issues from Buzz, OAuth device flow
  (PAT + gh CLI only).

## Security notes (flagged, deliberate for Slice 1)

- The Backlog bearer token lives in webview localStorage
  (`buzz-backlog-connection.v1`). The GitHub token already uses the OS
  keyring — move the Backlog token to the same mechanism next.
- The `buzz-issue-tracker` tag publishes shared state whose resolution
  depends on per-device connection config; other collaborators see a
  "backlog-issues" failed section until they connect. Consider carrying the
  (non-secret) server base URL as a 4th tag position and documenting the tag
  in NIP-MP's `buzz-` table.

## Not done (next slices)

- GitHub git ops beyond clone (fetch/diff/sync/push) + private-repo
  credential store — security-sensitive Rust change.
- GitHub as an *issue* provider (orca uses `gh` CLI; Buzz has no gh/octokit).
- Backlog comments/labels shown in the issue detail timeline (comments load
  as empty; `commentCount` is available on the task if wanted).
- Activity summaries (`fetchRepositoryActivitySummaries`) still count relay
  events only — backlog repos show zero activity counts.
- Status changes from Buzz UI → Backlog (`PUT tasks/{id} {status}`).
- Connect-flow validation probe (`GET /api/me`) before saving the token.

## Tests

- `backlogIssues.test.mjs` — mapping, status ramp, request grouping, writes.
- `projectWorkItems.test.mjs` — backlog routing + failure isolation cases.
