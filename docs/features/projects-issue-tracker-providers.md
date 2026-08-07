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
