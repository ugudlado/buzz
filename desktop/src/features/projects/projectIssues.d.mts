import type { RelayEvent } from "@/shared/api/types";

export type ProjectIssueStatus =
  | "Triage"
  | "Backlog"
  | "In Progress"
  | "In Review"
  | "Done"
  | "Closed";

export type ProjectIssueComment = {
  id: string;
  content: string;
  tags: string[][];
  author: string;
  createdAt: number;
};

/**
 * Discriminates what `author` (and comment `author`) actually holds:
 * `"nostr"` is a real 64-hex pubkey safe to feed into profile/avatar
 * lookups; `"github"`/`"backlog"` are an external login or display name
 * that must render as plain text only.
 */
export type ProjectIssueAuthorKind = "nostr" | "github" | "backlog";

export type ProjectIssue = {
  id: string;
  /** Human-facing short id (e.g. Backlog "ORC-12"); falls back to id prefix. */
  displayId?: string;
  title: string;
  content: string;
  tags: string[][];
  author: string;
  authorKind: ProjectIssueAuthorKind;
  createdAt: number;
  repoAddress: string | null;
  channelId: string | null;
  originAgentName: string | null;
  labels: string[];
  recipients: string[];
  assignees: string[];
  assigneeOperationHeads: Record<string, string>;
  status: ProjectIssueStatus;
  statusEventId: string | null;
  updatedAt: number;
  comments: ProjectIssueComment[];
};

export const ISSUE_ASSIGNMENT_LABEL: "assignment";
export const ISSUE_UNASSIGNMENT_LABEL: "unassignment";

export const PROJECT_ISSUE_STATUS: {
  TRIAGE: "Triage";
  BACKLOG: "Backlog";
  IN_PROGRESS: "In Progress";
  IN_REVIEW: "In Review";
  DONE: "Done";
  CLOSED: "Closed";
};

export function getTag(event: RelayEvent, name: string): string | undefined;
export function getAllTags(event: RelayEvent, name: string): string[];
export function getImetaTags(event: RelayEvent): string[][];
export function eventToProjectIssue(
  issue: RelayEvent,
  statusEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
): ProjectIssue;
export function projectIssueEventsToIssues(
  issueEvents: RelayEvent[],
  statusEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
): ProjectIssue[];
export function nextProjectIssueCommentCreatedAt(
  issue: ProjectIssue,
  now: number,
  author: string,
): number;
export function buildGitIssueTags(input: {
  repoAddress: string;
  repoOwner: string;
  title: string;
  labels?: string[];
}): string[][];
export function buildGitStatusTags(input: {
  issueId: string;
  repoAddress?: string | null;
  repoOwner?: string | null;
}): string[][];
