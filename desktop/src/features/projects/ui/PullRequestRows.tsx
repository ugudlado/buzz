import { Check, GitBranch, MessageSquare, X } from "lucide-react";

import {
  type ProjectPullRequest,
} from "@/features/projects/hooks";
import {
  formatExactTimestamp,
  relativeTime,
} from "@/features/projects/lib/projectsViewHelpers";
import {
  resolveWorkItemAuthor,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  ProjectFeedRow,
  ProjectFeedRowCluster,
  ProjectFeedRowMonoCell,
} from "./ProjectFeedRow";
import { CopyCommitHashButton } from "./ProjectCommitCopyButton";
import { pullRequestStatusClassName } from "./pullRequestPresentation";
import { ProfileAuthorName, ProfileIdentityButton } from "./ProjectProfileIdentity";

/** Dedupe/filter key for a pull request author: normalized pubkey for
 * Nostr authors, the raw GitHub login otherwise (normalizing would
 * lowercase a login and corrupt grouping). */
export function pullRequestAuthorFilterKey(
  pullRequest: ProjectPullRequest,
): string {
  return pullRequest.authorKind === "nostr"
    ? normalizePubkey(pullRequest.author)
    : pullRequest.author;
}

export function PullRequestCommitRow({
  author,
  authorKind = "nostr",
  branch,
  createdAt,
  hash,
  message,
  onOpenCommit,
  profiles,
}: {
  author: string;
  authorKind?: "nostr" | "github";
  branch: string | null;
  createdAt: number;
  hash: string | null;
  message: string;
  onOpenCommit?: (commitHash: string) => void;
  profiles?: UserProfileLookup;
}) {
  const resolvedAuthor = resolveWorkItemAuthor({
    author,
    authorKind,
    profiles,
  });
  const authorLabel = resolvedAuthor.label;
  const openCommit =
    hash && onOpenCommit ? () => onOpenCommit(hash) : undefined;

  return (
    <ProjectFeedRow
      meta={
        <>
          <ProfileIdentityButton
            avatarClassName="shrink-0"
            avatarSize="xs"
            avatarUrl={resolvedAuthor.profile?.avatarUrl ?? null}
            isAgent={resolvedAuthor.profile?.isAgent === true}
            label={authorLabel}
            pubkey={resolvedAuthor.pubkey}
            showLabel={false}
          />
          <span className="truncate">
            <ProfileAuthorName pubkey={resolvedAuthor.pubkey}>
              {authorLabel}
            </ProfileAuthorName>{" "}
            authored{" "}
            <span title={formatExactTimestamp(createdAt)}>
              {relativeTime(createdAt)}
            </span>
          </span>
          {branch ? (
            <span className="inline-flex min-w-0 items-center gap-1 rounded-full border border-border/60 px-1.5 py-0.5 font-mono text-2xs">
              <GitBranch className="h-3 w-3 shrink-0" />
              <span className="truncate">{branch}</span>
            </span>
          ) : null}
        </>
      }
      onOpen={openCommit}
      testId="project-pull-request-commit-row"
      title={message}
      trailing={
        <>
          {hash ? (
            <ProjectFeedRowCluster>
              <ProjectFeedRowMonoCell
                label={hash.slice(0, 7)}
                onClick={openCommit}
                title={`View commit ${hash.slice(0, 7)}`}
              />
              <CopyCommitHashButton hash={hash} />
            </ProjectFeedRowCluster>
          ) : null}
          <span
            className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground sm:block"
            data-testid="project-pull-request-commit-row-date"
            title={formatExactTimestamp(createdAt)}
          >
            {relativeTime(createdAt)}
          </span>
        </>
      }
    />
  );
}

export function PullRequestRow({
  onOpen,
  profiles,
  pullRequest,
}: {
  onOpen: () => void;
  profiles?: UserProfileLookup;
  pullRequest: ProjectPullRequest;
}) {
  const author = resolveWorkItemAuthor({
    author: pullRequest.author,
    authorKind: pullRequest.authorKind,
    profiles,
  });
  const authorLabel = author.label;
  const StatusIcon =
    pullRequest.status === "Closed" || pullRequest.status === "Draft"
      ? X
      : Check;
  const statusClassName = pullRequestStatusClassName(pullRequest.status);

  return (
    <ProjectFeedRow
      eventId={pullRequest.id}
      meta={
        <>
          <ProfileIdentityButton
            avatarClassName="shrink-0"
            avatarSize="xs"
            avatarUrl={author.profile?.avatarUrl ?? null}
            isAgent={author.profile?.isAgent === true}
            label={authorLabel}
            pubkey={author.pubkey}
            showLabel={false}
          />
          <span className="truncate">
            <ProfileAuthorName pubkey={author.pubkey}>
              {authorLabel}
            </ProfileAuthorName>{" "}
            created this pull request
          </span>
          {pullRequest.branchName ? (
            <span className="inline-flex min-w-0 items-center gap-1 rounded-full border border-border/60 px-1.5 py-0.5 font-mono text-2xs">
              <GitBranch className="h-3 w-3 shrink-0" />
              <span className="truncate">{pullRequest.branchName}</span>
            </span>
          ) : null}
          <span
            className={`rounded-full border border-border/60 px-1.5 py-0.5 text-2xs font-medium ${statusClassName}`}
          >
            {pullRequest.status}
          </span>
        </>
      }
      onOpen={onOpen}
      statusIcon={
        <StatusIcon className={`h-3.5 w-3.5 shrink-0 ${statusClassName}`} />
      }
      testId="project-pull-request-row"
      title={pullRequest.title}
      trailing={
        <>
          {pullRequest.comments.length > 0 ? (
            <button
              aria-label={`View ${pullRequest.comments.length} comments`}
              className="flex items-center gap-1 rounded-md text-xs text-muted-foreground hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={onOpen}
              type="button"
            >
              <MessageSquare className="h-3.5 w-3.5" />
              {pullRequest.comments.length}
            </button>
          ) : null}
          <ProjectFeedRowCluster>
            <ProjectFeedRowMonoCell
              label={`#${pullRequest.id.slice(0, 8)}`}
              onClick={onOpen}
              title="View pull request"
            />
          </ProjectFeedRowCluster>
          <span
            className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground sm:block"
            data-testid="project-pull-request-row-date"
            title={formatExactTimestamp(pullRequest.createdAt)}
          >
            {relativeTime(pullRequest.createdAt)}
          </span>
        </>
      }
    />
  );
}
