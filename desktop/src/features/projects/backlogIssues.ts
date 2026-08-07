// Backlog issue-tracker provider for project repositories.
//
// A repository opts in via a `buzz-issue-tracker` tag on its kind:30617
// announcement: ["buzz-issue-tracker", "backlog", "<backlog project guid>"].
// Reads/writes go to a Backlog server (~/code/backlog) over its project-nested
// REST API (`/api/projects/{guid}/tasks…`, bearer `bklg_` token). Tasks are
// mapped into the existing `ProjectIssue` shape so every issue surface renders
// them unchanged; ids are prefixed `backlog:` so they can never collide with
// 64-hex Nostr event ids in `${repoAddress}:${issue.id}` dedupe keys.

import type { ProjectIssue, ProjectIssueStatus } from "./projectIssues.mjs";

/** Connection to a Backlog server (user-scoped, one per app). */
export type BacklogConnection = {
  baseUrl: string;
  token: string;
};

const CONNECTION_KEY = "buzz-backlog-connection.v1";

/** Read the stored Backlog connection (null when not configured). */
export function getBacklogConnection(): BacklogConnection | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(CONNECTION_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<BacklogConnection>;
    if (!parsed.baseUrl || !parsed.token) return null;
    return { baseUrl: parsed.baseUrl, token: parsed.token };
  } catch {
    return null;
  }
}

/** Store (or clear with null) the Backlog connection. */
export function setBacklogConnection(connection: BacklogConnection | null) {
  if (typeof window === "undefined") return;
  if (!connection) {
    window.localStorage.removeItem(CONNECTION_KEY);
    return;
  }
  window.localStorage.setItem(CONNECTION_KEY, JSON.stringify(connection));
}

/** Synthetic issue-id namespace for Backlog tasks. */
export const BACKLOG_ISSUE_ID_PREFIX = "backlog:";

/** True when a ProjectIssue id refers to a Backlog task. */
export function isBacklogIssueId(issueId: string): boolean {
  return issueId.startsWith(BACKLOG_ISSUE_ID_PREFIX);
}

/** Backlog task id from a synthetic issue id. */
export function backlogTaskIdFromIssueId(issueId: string): string {
  return issueId.slice(BACKLOG_ISSUE_ID_PREFIX.length);
}

/** Subset of Backlog's task list response this provider consumes. */
export type BacklogTask = {
  id: string;
  displayId: string;
  title: string;
  status: string;
  assignee: { id: string; name: string } | null;
  createdDate: string;
  updatedDate?: string;
  labels: string[];
  description?: string;
  priority?: string;
};

type BacklogDeps = {
  connection?: BacklogConnection | null;
  fetchImpl?: typeof fetch;
};

function resolveDeps(deps?: BacklogDeps): {
  connection: BacklogConnection;
  fetchImpl: typeof fetch;
} {
  const connection = deps?.connection ?? getBacklogConnection();
  if (!connection) {
    throw new Error(
      "Backlog is not connected. Set the Backlog server URL and token first.",
    );
  }
  return { connection, fetchImpl: deps?.fetchImpl ?? fetch };
}

async function backlogRequest(
  path: string,
  init: RequestInit,
  deps?: BacklogDeps,
): Promise<Response> {
  const { connection, fetchImpl } = resolveDeps(deps);
  const base = connection.baseUrl.replace(/\/+$/, "");
  const response = await fetchImpl(`${base}${path}`, {
    ...init,
    headers: {
      Accept: "application/json",
      Authorization: `Bearer ${connection.token}`,
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...init.headers,
    },
  });
  if (!response.ok) {
    throw new Error(`Backlog request failed (${response.status}): ${path}`);
  }
  return response;
}

/** Map a Backlog status string onto the project-issue status ramp. */
export function backlogStatusToIssueStatus(status: string): ProjectIssueStatus {
  const normalized = status.trim().toLowerCase();
  if (normalized === "done") return "Done";
  if (normalized === "review" || normalized === "verify") return "In Review";
  if (normalized === "in progress") return "In Progress";
  if (normalized === "to do" || normalized === "ready") return "Backlog";
  return "Triage";
}

function isoToUnixSeconds(iso: string | undefined, fallback: number): number {
  if (!iso) return fallback;
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? fallback : Math.floor(ms / 1_000);
}

/** Map one Backlog task into the shared ProjectIssue shape. */
export function backlogTaskToProjectIssue(
  task: BacklogTask,
  repoAddress: string,
): ProjectIssue {
  const createdAt = isoToUnixSeconds(task.createdDate, 0);
  return {
    id: `${BACKLOG_ISSUE_ID_PREFIX}${task.id}`,
    title: `${task.displayId} ${task.title}`.trim(),
    content: task.description ?? "",
    tags: [],
    author: task.assignee?.name ?? "",
    createdAt,
    repoAddress,
    channelId: null,
    originAgentName: null,
    labels: task.labels ?? [],
    recipients: [],
    status: backlogStatusToIssueStatus(task.status),
    statusEventId: null,
    updatedAt: isoToUnixSeconds(task.updatedDate, createdAt),
    comments: [],
  };
}

type BacklogTrackedRepository = {
  repoAddress: string;
  backlogProject: string;
};

/**
 * Fetch issues for Backlog-tracked repositories, one GET per distinct Backlog
 * project (repos sharing a project share the request). Returns issues keyed by
 * repoAddress.
 */
export async function fetchBacklogIssuesForRepos(
  repos: BacklogTrackedRepository[],
  deps?: BacklogDeps,
): Promise<Map<string, ProjectIssue[]>> {
  const reposByProject = new Map<string, BacklogTrackedRepository[]>();
  for (const repo of repos) {
    const group = reposByProject.get(repo.backlogProject) ?? [];
    group.push(repo);
    reposByProject.set(repo.backlogProject, group);
  }

  const result = new Map<string, ProjectIssue[]>();
  await Promise.all(
    [...reposByProject.entries()].map(async ([project, projectRepos]) => {
      const response = await backlogRequest(
        `/api/projects/${encodeURIComponent(project)}/tasks`,
        { method: "GET" },
        deps,
      );
      const tasks = (await response.json()) as BacklogTask[];
      for (const repo of projectRepos) {
        result.set(
          repo.repoAddress,
          tasks.map((task) =>
            backlogTaskToProjectIssue(task, repo.repoAddress),
          ),
        );
      }
    }),
  );
  return result;
}

/** Create a Backlog task; returns the synthetic issue id. */
export async function createBacklogIssue(
  backlogProject: string,
  input: { title: string; body: string },
  deps?: BacklogDeps,
): Promise<string> {
  const response = await backlogRequest(
    `/api/projects/${encodeURIComponent(backlogProject)}/tasks`,
    {
      method: "POST",
      body: JSON.stringify({
        title: input.title,
        ...(input.body.trim() ? { description: input.body.trim() } : {}),
      }),
    },
    deps,
  );
  const task = (await response.json()) as { id: string };
  return `${BACKLOG_ISSUE_ID_PREFIX}${task.id}`;
}

/** Append a comment to a Backlog task behind a synthetic issue id. */
export async function createBacklogIssueComment(
  backlogProject: string,
  issueId: string,
  body: string,
  deps?: BacklogDeps,
): Promise<void> {
  await backlogRequest(
    `/api/projects/${encodeURIComponent(backlogProject)}/tasks/${encodeURIComponent(backlogTaskIdFromIssueId(issueId))}/comments`,
    { method: "POST", body: JSON.stringify({ body }) },
    deps,
  );
}
