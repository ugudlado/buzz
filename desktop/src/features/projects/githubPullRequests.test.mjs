import assert from "node:assert/strict";
import test from "node:test";

import {
  fetchGithubPullRequestsForRepos,
  githubPullRequestUrl,
  githubPullToProjectPullRequest,
  isGithubPullRequestId,
} from "./githubPullRequests.ts";

const REPO_ADDRESS = `30617:${"a".repeat(64)}:app`;

const PULL = {
  number: 7,
  title: "Fix login",
  body: "Closes #6",
  author: "octocat",
  state: "open",
  draft: false,
  merged: false,
  headRef: "fix-login",
  baseRef: "main",
  headSha: "c".repeat(40),
  htmlUrl: "https://github.com/o/r/pull/7",
  createdAt: "2026-08-01T00:00:00Z",
  updatedAt: "2026-08-02T00:00:00Z",
  labels: ["bug"],
  requestedReviewers: ["hubot"],
};

test("maps a GitHub PR to the shared shape", () => {
  const pr = githubPullToProjectPullRequest(PULL, REPO_ADDRESS);
  assert.equal(pr.id, "github:7");
  assert.ok(isGithubPullRequestId(pr.id));
  assert.equal(pr.title, "#7 Fix login");
  assert.equal(pr.status, "Open");
  assert.equal(pr.branchName, "fix-login");
  assert.equal(pr.targetBranch, "main");
  assert.equal(pr.author, "octocat");
  assert.deepEqual(pr.reviewers, ["hubot"]);
  assert.equal(githubPullRequestUrl(pr), "https://github.com/o/r/pull/7");
  assert.ok(pr.updatedAt > pr.createdAt);
});

test("status mapping: merged beats closed; draft only when open", () => {
  assert.equal(
    githubPullToProjectPullRequest(
      { ...PULL, merged: true, state: "closed" },
      REPO_ADDRESS,
    ).status,
    "Merged",
  );
  assert.equal(
    githubPullToProjectPullRequest({ ...PULL, state: "closed" }, REPO_ADDRESS)
      .status,
    "Closed",
  );
  assert.equal(
    githubPullToProjectPullRequest({ ...PULL, draft: true }, REPO_ADDRESS)
      .status,
    "Draft",
  );
});

test("fetch keys results by repoAddress with one call per repo", async () => {
  const calls = [];
  const byRepo = await fetchGithubPullRequestsForRepos(
    [
      { githubRepo: { name: "r1", owner: "o" }, repoAddress: "30617:x:r1" },
      { githubRepo: { name: "r2", owner: "o" }, repoAddress: "30617:x:r2" },
    ],
    async (owner, repo) => {
      calls.push(`${owner}/${repo}`);
      return [PULL];
    },
    async () => ({ connected: true, login: "octocat" }),
  );
  assert.deepEqual(calls.sort(), ["o/r1", "o/r2"]);
  assert.equal(byRepo.get("30617:x:r1").length, 1);
  assert.equal(byRepo.get("30617:x:r2").length, 1);
});

test("resolves empty when GitHub is not connected", async () => {
  const byRepo = await fetchGithubPullRequestsForRepos(
    [{ githubRepo: { name: "r", owner: "o" }, repoAddress: "30617:x:r" }],
    async () => {
      throw new Error("must not be called when disconnected");
    },
    async () => ({ connected: false }),
  );
  assert.equal(byRepo.size, 0);
});
