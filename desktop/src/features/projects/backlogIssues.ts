// Backlog issue-tracker provider for project repositories.
//
// A repository opts in via a `buzz-issue-tracker` tag on its kind:30617
// announcement: ["buzz-issue-tracker", "backlog", "<backlog project guid>"].
// All HTTP happens in Rust (`commands/backlog.rs`) with the user token in the
// OS keyring; this module maps raw task rows into the existing `ProjectIssue`
// shape so every issue surface renders them unchanged. Ids are prefixed
// `backlog:` so they can never collide with 64-hex Nostr event ids in
// `${repoAddress}:${issue.id}` dedupe keys.

import {
  connectBacklog,
  createBacklogTask,
  createBacklogTaskComment,
  getBacklogStatus,
  listBacklogTasks,
} from "@/shared/api/tauriBacklog";
import type { ProjectIssue, ProjectIssueStatus } from "./projectIssues.mjs";

/** Pre-keyring storage key (Slice 1) — migrated to Rust on first use. */
const LEGACY_CONNECTION_KEY = "buzz-backlog-connection.v1";

/**
 * One-time migration: Slice 1 kept `{baseUrl, token}` in localStorage. Move
 * it into the OS keyring via `backlog_connect` and delete the plaintext copy.
 * Safe to call repeatedly; no-ops once localStorage is clean.
 */
export async function migrateLegacyBacklogConnection(): Promise<void> {
  if (typeof window === "undefined") return;
  type LegacyConnection = { baseUrl?: string; token?: string };
  let legacy: LegacyConnection | null = null;
  try {
    const raw = window.localStorage.getItem(LEGACY_CONNECTION_KEY);
    legacy = raw ? (JSON.parse(raw) as LegacyConnection) : null;
  } catch {
    legacy = null;
  }
  if (!legacy?.baseUrl || !legacy.token) {
    window.localStorage.removeItem(LEGACY_CONNECTION_KEY);
    return;
  }
  try {
    const status = await getBacklogStatus();
    if (!status.connected) {
      await connectBacklog({ baseUrl: legacy.baseUrl, token: legacy.token });
    }
    window.localStorage.removeItem(LEGACY_CONNECTION_KEY);
  } catch {
    // Keyring/connect unavailable (e.g. server down) — keep the legacy copy
    // so a later attempt can still migrate it.
  }
}

/** Synthetic issue-id namespace for Backlog tasks. */
const BACKLOG_ISSUE_ID_PREFIX = "backlog:";

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
  assignee: { name: string } | null;
  createdDate: string;
  updatedDate?: string;
  labels: string[];
  description?: string;
};

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
    displayId: task.displayId,
    title: task.title,
    content: task.description ?? "",
    tags: [],
    author: task.assignee?.name ?? "Unassigned",
    authorKind: "backlog",
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
 * Fetch issues for Backlog-tracked repositories, one request per distinct
 * Backlog project (repos sharing a project share the request). Returns issues
 * keyed by repoAddress.
 */
export async function fetchBacklogIssuesForRepos(
  repos: BacklogTrackedRepository[],
  listTasks: typeof listBacklogTasks = listBacklogTasks,
): Promise<Map<string, ProjectIssue[]>> {
  const result = new Map<string, ProjectIssue[]>();
  if (repos.length === 0) return result;
  await migrateLegacyBacklogConnection();

  const reposByProject = new Map<string, BacklogTrackedRepository[]>();
  for (const repo of repos) {
    const group = reposByProject.get(repo.backlogProject) ?? [];
    group.push(repo);
    reposByProject.set(repo.backlogProject, group);
  }

  await Promise.all(
    [...reposByProject.entries()].map(async ([project, projectRepos]) => {
      const tasks = (await listTasks(project)) as BacklogTask[];
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
  createTask: typeof createBacklogTask = createBacklogTask,
): Promise<string> {
  const task = await createTask(
    backlogProject,
    input.title,
    input.body.trim() || undefined,
  );
  return `${BACKLOG_ISSUE_ID_PREFIX}${task.id}`;
}

/** Append a comment to a Backlog task behind a synthetic issue id. */
export async function createBacklogIssueComment(
  backlogProject: string,
  issueId: string,
  body: string,
  createComment: typeof createBacklogTaskComment = createBacklogTaskComment,
): Promise<void> {
  await createComment(backlogProject, backlogTaskIdFromIssueId(issueId), body);
}
