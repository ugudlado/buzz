import assert from "node:assert/strict";
import test from "node:test";

import {
  backlogStatusToIssueStatus,
  backlogTaskIdFromIssueId,
  backlogTaskToProjectIssue,
  createBacklogIssue,
  createBacklogIssueComment,
  fetchBacklogIssuesForRepos,
} from "./backlogIssues.ts";

const CONNECTION = { baseUrl: "http://localhost:4321/", token: "bklg_test" };
const REPO_A = "30617:aaa:one";
const REPO_B = "30617:bbb:two";

const TASK = {
  id: "task-1",
  displayId: "ORC-12",
  title: "Fix the thing",
  status: "In Progress",
  assignee: { id: "u1", name: "spidey" },
  createdDate: "2026-08-01T00:00:00.000Z",
  updatedDate: "2026-08-02T00:00:00.000Z",
  labels: ["bug"],
  description: "It is broken.",
};

function fakeFetch(handler) {
  const calls = [];
  const impl = async (url, init) => {
    calls.push({ url, init });
    const body = handler(url, init);
    return {
      ok: true,
      status: 200,
      json: async () => body,
    };
  };
  return { calls, impl };
}

test("status mapping covers the default backlog ramp", () => {
  assert.equal(backlogStatusToIssueStatus("To Do"), "Backlog");
  assert.equal(backlogStatusToIssueStatus("Ready"), "Backlog");
  assert.equal(backlogStatusToIssueStatus("In Progress"), "In Progress");
  assert.equal(backlogStatusToIssueStatus("Review"), "In Review");
  assert.equal(backlogStatusToIssueStatus("Verify"), "In Review");
  assert.equal(backlogStatusToIssueStatus("done"), "Done");
  assert.equal(backlogStatusToIssueStatus("Weird Custom"), "Triage");
});

test("task maps to ProjectIssue with a synthetic id", () => {
  const issue = backlogTaskToProjectIssue(TASK, REPO_A);
  assert.equal(issue.id, "backlog:task-1");
  assert.equal(backlogTaskIdFromIssueId(issue.id), "task-1");
  assert.equal(issue.displayId, "ORC-12");
  assert.equal(issue.title, "Fix the thing");
  assert.equal(issue.content, "It is broken.");
  assert.equal(issue.status, "In Progress");
  assert.equal(issue.repoAddress, REPO_A);
  assert.equal(issue.author, "spidey");
  assert.ok(issue.createdAt > 0);
  assert.ok(issue.updatedAt > issue.createdAt);
  assert.deepEqual(issue.labels, ["bug"]);
});

test("fetch groups repos by backlog project (one request per project)", async () => {
  const { calls, impl } = fakeFetch(() => [TASK]);
  const byRepo = await fetchBacklogIssuesForRepos(
    [
      { repoAddress: REPO_A, backlogProject: "guid-1" },
      { repoAddress: REPO_B, backlogProject: "guid-1" },
    ],
    { connection: CONNECTION, fetchImpl: impl },
  );
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "http://localhost:4321/api/projects/guid-1/tasks");
  assert.equal(calls[0].init.headers.Authorization, "Bearer bklg_test");
  assert.equal(byRepo.get(REPO_A).length, 1);
  assert.equal(byRepo.get(REPO_B).length, 1);
});

test("create issue posts to the nested route and returns synthetic id", async () => {
  const { calls, impl } = fakeFetch(() => ({ id: "task-9" }));
  const id = await createBacklogIssue(
    "guid-1",
    { title: "New", body: "  Body  " },
    { connection: CONNECTION, fetchImpl: impl },
  );
  assert.equal(id, "backlog:task-9");
  assert.equal(calls[0].url, "http://localhost:4321/api/projects/guid-1/tasks");
  assert.deepEqual(JSON.parse(calls[0].init.body), {
    title: "New",
    description: "Body",
  });
});

test("comment posts body to the task comments route", async () => {
  const { calls, impl } = fakeFetch(() => ({}));
  await createBacklogIssueComment("guid-1", "backlog:task-9", "hello", {
    connection: CONNECTION,
    fetchImpl: impl,
  });
  assert.equal(
    calls[0].url,
    "http://localhost:4321/api/projects/guid-1/tasks/task-9/comments",
  );
  assert.deepEqual(JSON.parse(calls[0].init.body), { body: "hello" });
});

test("missing connection rejects with a clear error", async () => {
  await assert.rejects(
    fetchBacklogIssuesForRepos(
      [{ repoAddress: REPO_A, backlogProject: "guid-1" }],
      {},
    ),
    /Backlog is not connected/,
  );
});
