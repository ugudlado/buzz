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

const REPO_A = "30617:aaa:one";
const REPO_B = "30617:bbb:two";

const TASK = {
  id: "task-1",
  displayId: "ORC-12",
  title: "Fix the thing",
  status: "In Progress",
  assignee: { name: "spidey" },
  createdDate: "2026-08-01T00:00:00.000Z",
  updatedDate: "2026-08-02T00:00:00.000Z",
  labels: ["bug"],
  description: "It is broken.",
};

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
  assert.equal(issue.authorKind, "backlog");
  assert.ok(issue.createdAt > 0);
  assert.ok(issue.updatedAt > issue.createdAt);
  assert.deepEqual(issue.labels, ["bug"]);
});

test("unassigned task falls back to a plain-text label", () => {
  const issue = backlogTaskToProjectIssue({ ...TASK, assignee: null }, REPO_A);
  assert.equal(issue.author, "Unassigned");
  assert.equal(issue.authorKind, "backlog");
});

test("fetch groups repos by backlog project (one request per project)", async () => {
  const calls = [];
  const byRepo = await fetchBacklogIssuesForRepos(
    [
      { repoAddress: REPO_A, backlogProject: "guid-1" },
      { repoAddress: REPO_B, backlogProject: "guid-1" },
    ],
    async (projectGuid) => {
      calls.push(projectGuid);
      return [TASK];
    },
  );
  assert.deepEqual(calls, ["guid-1"]);
  assert.equal(byRepo.get(REPO_A).length, 1);
  assert.equal(byRepo.get(REPO_B).length, 1);
});

test("create issue passes trimmed description and returns synthetic id", async () => {
  const calls = [];
  const id = await createBacklogIssue(
    "guid-1",
    { title: "New", body: "  Body  " },
    async (projectGuid, title, description) => {
      calls.push({ description, projectGuid, title });
      return { id: "task-9" };
    },
  );
  assert.equal(id, "backlog:task-9");
  assert.deepEqual(calls, [
    { description: "Body", projectGuid: "guid-1", title: "New" },
  ]);
});

test("comment resolves the task id from the synthetic issue id", async () => {
  const calls = [];
  await createBacklogIssueComment(
    "guid-1",
    "backlog:task-9",
    "hello",
    async (projectGuid, taskId, body) => {
      calls.push({ body, projectGuid, taskId });
    },
  );
  assert.deepEqual(calls, [
    { body: "hello", projectGuid: "guid-1", taskId: "task-9" },
  ]);
});
