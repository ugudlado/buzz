import assert from "node:assert/strict";
import test from "node:test";

import { fetchProjectsWorkItems } from "./projectWorkItems.ts";

// ── Work-item deduplication ─────────────────────────────────────────────────
//
// NIP-MP §Multiple membership: a repository may belong to several projects.
// When it does, global issue/PR lists must produce exactly one row per work
// item — not one row per project membership. These tests call the exported
// production function with a stubbed fetchEvents to verify the dedup contract
// end-to-end, not just the filter algorithm in isolation.

const REPO_OWNER = "a".repeat(64);
const REPO_DTAG = "relay";
const REPO_ADDRESS = `30617:${REPO_OWNER}:${REPO_DTAG}`;

const ISSUE_ID = "i".repeat(64);
const PR_ID = "p".repeat(64);
const PR_ID_2 = "q".repeat(64);

// Two projects that both contain the same repository.
const projectA = {
  repositories: [{ repoAddress: REPO_ADDRESS }],
};
const projectB = {
  repositories: [{ repoAddress: REPO_ADDRESS }],
};

// Minimal valid NIP-34 issue event for the shared repo.
function makeIssue(id, updatedAt = 100) {
  return {
    id,
    kind: 1621,
    pubkey: REPO_OWNER,
    created_at: updatedAt,
    content: "An issue",
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Fix the thing"],
    ],
  };
}

// Minimal valid NIP-34 pull request event for the shared repo.
function makePR(id, updatedAt = 100) {
  return {
    id,
    kind: 1618, // KIND_GIT_PULL_REQUEST
    pubkey: REPO_OWNER,
    created_at: updatedAt,
    content: "A PR",
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Add a feature"],
    ],
  };
}

// fetchEvents stub: returns the given root events (issues + PRs) and empty
// arrays for all other query kinds (updates, comments, statuses).
function makeFetchEvents(rootEvents) {
  return async (filter) => {
    const { kinds } = filter;
    // Root issues (kind 1621) + PRs (kind 1618)
    if (kinds?.includes(1621) || kinds?.includes(1618)) {
      return rootEvents.filter((e) => kinds.includes(e.kind));
    }
    // Everything else (PR updates, comments, statuses) — empty
    return [];
  };
}

test("fetchProjectsWorkItems deduplicates issues from a shared repository", async () => {
  const issue = makeIssue(ISSUE_ID);
  const fetchEvents = makeFetchEvents([issue]);

  const result = await fetchProjectsWorkItems(
    [projectA, projectB],
    fetchEvents,
  );

  assert.equal(
    result.issues.items.length,
    1,
    "duplicate issue from shared repo must collapse to one row",
  );
  assert.equal(result.issues.items[0].issue.id, ISSUE_ID);
});

test("fetchProjectsWorkItems deduplicates pull requests from a shared repository", async () => {
  const pr1 = makePR(PR_ID, 100);
  const pr2 = makePR(PR_ID_2, 90);
  const fetchEvents = makeFetchEvents([pr1, pr2]);

  const result = await fetchProjectsWorkItems(
    [projectA, projectB],
    fetchEvents,
  );

  assert.equal(
    result.pullRequests.items.length,
    2,
    "distinct PRs must survive dedup; only exact-id duplicates collapse",
  );
  const ids = result.pullRequests.items.map((item) => item.pullRequest.id);
  assert.ok(ids.includes(PR_ID), "first PR must be present");
  assert.ok(ids.includes(PR_ID_2), "second PR must be present");
});

test("fetchProjectsWorkItems returns a single row for a PR present in both project contexts", async () => {
  // Same PR id returned twice (once per project's relay query).
  const pr = makePR(PR_ID, 100);
  // The stub returns the same event for every root query, simulating
  // the relay returning the same PR for both projects' repo addresses.
  let callCount = 0;
  const fetchEvents = async (filter) => {
    if (filter.kinds?.includes(1618)) {
      callCount += 1;
      return [pr];
    }
    return [];
  };

  const result = await fetchProjectsWorkItems(
    [projectA, projectB],
    fetchEvents,
  );

  // The relay was queried once per unique repo address — but even if it
  // returned the same id twice across the two project contexts, dedupe fires.
  assert.equal(
    result.pullRequests.items.length,
    1,
    "same PR id appearing in both project contexts must produce one row",
  );
  // Sanity: the stub was actually called (proves we ran the production path).
  assert.ok(callCount >= 1, "fetchEvents must have been called");
});

test("fetchProjectsWorkItems routes backlog-tracked repos to the backlog provider", async () => {
  const BACKLOG_REPO = `30617:${"b".repeat(64)}:tracked`;
  const backlogProjectGuid = "guid-1";
  const projects = [
    {
      repositories: [
        { repoAddress: REPO_ADDRESS },
        {
          repoAddress: BACKLOG_REPO,
          issueTracker: { kind: "backlog", project: backlogProjectGuid },
        },
      ],
    },
  ];
  const relayIssue = makeIssue(ISSUE_ID);
  // Relay also has a stale 1621 for the backlog-tracked repo — must be ignored.
  const staleIssue = {
    ...makeIssue("s".repeat(64)),
    tags: [
      ["a", BACKLOG_REPO],
      ["subject", "Stale relay issue"],
    ],
  };
  const fetchEvents = makeFetchEvents([relayIssue, staleIssue]);
  const backlogIssue = {
    id: "backlog:task-1",
    title: "ORC-1 From backlog",
    content: "",
    tags: [],
    author: "spidey",
    createdAt: 10,
    repoAddress: BACKLOG_REPO,
    channelId: null,
    originAgentName: null,
    labels: [],
    recipients: [],
    status: "In Progress",
    statusEventId: null,
    updatedAt: 20,
    comments: [],
  };
  const fetchBacklogIssues = async (repos) => {
    assert.deepEqual(repos, [
      { repoAddress: BACKLOG_REPO, backlogProject: backlogProjectGuid },
    ]);
    return new Map([[BACKLOG_REPO, [backlogIssue]]]);
  };

  const result = await fetchProjectsWorkItems(
    projects,
    fetchEvents,
    fetchBacklogIssues,
  );

  const ids = result.issues.items.map((item) => item.issue.id);
  assert.deepEqual(ids.sort(), ["backlog:task-1", ISSUE_ID].sort());
  assert.deepEqual(result.issues.failedSections, []);
});

test("fetchProjectsWorkItems reports backlog failure without dropping relay issues", async () => {
  const BACKLOG_REPO = `30617:${"c".repeat(64)}:tracked`;
  const projects = [
    {
      repositories: [
        { repoAddress: REPO_ADDRESS },
        {
          repoAddress: BACKLOG_REPO,
          issueTracker: { kind: "backlog", project: "guid-2" },
        },
      ],
    },
  ];
  const fetchEvents = makeFetchEvents([makeIssue(ISSUE_ID)]);
  const fetchBacklogIssues = async () => {
    throw new Error("backlog down");
  };

  const result = await fetchProjectsWorkItems(
    projects,
    fetchEvents,
    fetchBacklogIssues,
  );

  assert.equal(result.issues.items.length, 1);
  assert.equal(result.issues.items[0].issue.id, ISSUE_ID);
  assert.deepEqual(result.issues.failedSections, ["backlog-issues"]);
});
