// Lazy, per-directory file browser for GitHub-linked repositories viewed via
// "Remote". Mirrors RepositoryFilesPanel's look (breadcrumbs, table, file
// preview) but fetches one directory level at a time from GitHub's Contents
// API instead of assuming a full recursive file list — GitHub's API has no
// recursive-tree endpoint wired up on the Rust side, and eagerly walking
// every directory would be wasteful and slow for large repos.
import { ChevronRight, FileDiff, Folder, Loader2 } from "lucide-react";
import * as React from "react";

import type { Repository } from "@/features/projects/projectModels";
import {
  useGithubRepoFileQuery,
  useGithubRepoTreeQuery,
} from "@/features/projects/useGithubRepoBrowser";
import type { GithubTreeEntry } from "@/shared/api/tauriGithub";
import { cn } from "@/shared/lib/cn";
import { SyntaxHighlightedCode } from "@/shared/ui/markdown";
import { PROJECT_DETAIL_PANEL_CLASS } from "./projectPanelStyles";
import {
  BreadcrumbButton,
  formatFileSize,
  languageForPath,
} from "./ProjectRepositoryPanel";
import {
  RepoSourceDropdown,
  RepoSyncActionButton,
  RepositoryBranchDropdown,
  type RepoSourceHeaderControls,
} from "./ProjectRepositorySource";

function sortEntries(entries: GithubTreeEntry[]) {
  return [...entries].sort((left, right) => {
    if (left.entryType !== right.entryType) {
      return left.entryType === "dir" ? -1 : 1;
    }
    return left.name.localeCompare(right.name);
  });
}

function HeaderRow({
  sourceControls,
  pathSegments,
  onOpenPath,
}: {
  sourceControls?: RepoSourceHeaderControls;
  pathSegments: string[];
  onOpenPath: (path: string) => void;
}) {
  return (
    <div className="flex min-h-14 min-w-0 items-center gap-1 border-border/50 border-b px-3 py-3">
      {sourceControls ? (
        <>
          <RepoSourceDropdown controls={sourceControls} />
          <RepositoryBranchDropdown
            branch={sourceControls.branch}
            branchOptions={sourceControls.branchOptions}
            createBranchDisabled={sourceControls.createBranchDisabled}
            createBranchTitle={sourceControls.createBranchTitle}
            deleteBranchDisabled={sourceControls.deleteBranchDisabled}
            deleteBranchTitle={sourceControls.deleteBranchTitle}
            onBranchChange={sourceControls.onBranchChange}
            onCreateBranch={sourceControls.onCreateBranch}
            onDeleteBranch={sourceControls.onDeleteBranch}
            onTagChange={sourceControls.onTagChange}
            selectedTag={sourceControls.selectedTag}
            tagOptions={sourceControls.tagOptions}
          />
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        </>
      ) : null}
      <BreadcrumbButton onClick={() => onOpenPath("")}>Files</BreadcrumbButton>
      {pathSegments.map((segment, index) => {
        const nextPath = pathSegments.slice(0, index + 1).join("/");
        return (
          <React.Fragment key={nextPath}>
            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
            <BreadcrumbButton onClick={() => onOpenPath(nextPath)}>
              {segment}
            </BreadcrumbButton>
          </React.Fragment>
        );
      })}
      {sourceControls ? (
        <div className="ml-auto flex shrink-0 items-center">
          <RepoSyncActionButton controls={sourceControls} />
        </div>
      ) : null}
    </div>
  );
}

function GithubFileContentPanel({
  repository,
  branch,
  path,
  onOpenPath,
}: {
  repository: Repository;
  branch: string;
  path: string;
  onOpenPath: (path: string) => void;
}) {
  const fileQuery = useGithubRepoFileQuery(repository, branch, path);
  const language = languageForPath(path);
  const pathSegments = path.split("/").filter(Boolean);
  const fileName = pathSegments[pathSegments.length - 1] ?? path;
  const directorySegments = pathSegments.slice(0, -1);

  return (
    <div className={PROJECT_DETAIL_PANEL_CLASS} data-project-detail-panel>
      <div className="flex min-h-14 items-center gap-1 border-border/50 border-b bg-muted/20 px-3 py-3">
        <BreadcrumbButton onClick={() => onOpenPath("")}>
          Files
        </BreadcrumbButton>
        {directorySegments.map((segment, index) => {
          const nextPath = directorySegments.slice(0, index + 1).join("/");
          return (
            <React.Fragment key={nextPath}>
              <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
              <BreadcrumbButton onClick={() => onOpenPath(nextPath)}>
                {segment}
              </BreadcrumbButton>
            </React.Fragment>
          );
        })}
        <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        <FileDiff className="h-4 w-4 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate px-1.5 py-1 font-mono text-xs text-foreground">
          {fileName}
        </span>
        {fileQuery.data ? (
          <span className="hidden shrink-0 text-2xs text-muted-foreground sm:block">
            {formatFileSize(fileQuery.data.size)}
          </span>
        ) : null}
      </div>
      {fileQuery.isLoading ? (
        <div className="flex items-center gap-2 p-6 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          Loading file…
        </div>
      ) : fileQuery.error || !fileQuery.data ? (
        <div className="p-6 text-sm text-muted-foreground">
          Preview unavailable for this file. Large and binary files only show
          metadata.
        </div>
      ) : (
        <pre className="max-h-[36rem] overflow-auto bg-background/60 p-4">
          {language ? (
            <SyntaxHighlightedCode
              className="text-xs leading-relaxed"
              code={fileQuery.data.content}
              language={language}
            />
          ) : (
            <code className="block min-w-full whitespace-pre font-mono text-xs leading-relaxed text-foreground">
              {fileQuery.data.content}
            </code>
          )}
        </pre>
      )}
    </div>
  );
}

/**
 * GitHub-backed file browser, one directory fetched at a time. Keyed by
 * branch at the call site (below) so switching branches remounts the
 * navigation state instead of leaving it pointed at a path/file that may not
 * exist on the new branch.
 */
function GithubRepositoryFilesBrowser({
  repository,
  branch,
  sourceControls,
}: {
  repository: Repository;
  branch: string | null;
  sourceControls?: RepoSourceHeaderControls;
}) {
  const [currentPath, setCurrentPath] = React.useState("");
  const [selectedFilePath, setSelectedFilePath] = React.useState<string | null>(
    null,
  );

  const treeQuery = useGithubRepoTreeQuery(repository, branch, currentPath);
  const entries = React.useMemo(
    () => sortEntries(treeQuery.data ?? []),
    [treeQuery.data],
  );
  const pathSegments = currentPath ? currentPath.split("/") : [];

  const openPath = (path: string) => {
    setSelectedFilePath(null);
    setCurrentPath(path);
  };

  if (selectedFilePath && branch) {
    return (
      <GithubFileContentPanel
        branch={branch}
        onOpenPath={openPath}
        path={selectedFilePath}
        repository={repository}
      />
    );
  }

  const stateMessage = !branch
    ? "Choose a branch to browse its files."
    : treeQuery.isLoading
      ? "Loading repository files…"
      : treeQuery.error
        ? "Could not load this directory from GitHub."
        : entries.length === 0
          ? "This directory is empty."
          : null;

  return (
    <div className={PROJECT_DETAIL_PANEL_CLASS} data-project-detail-panel>
      <HeaderRow
        onOpenPath={openPath}
        pathSegments={pathSegments}
        sourceControls={sourceControls}
      />
      {stateMessage ? (
        <div className={cn("p-4 text-sm text-muted-foreground")}>
          {treeQuery.isLoading ? (
            <span className="flex items-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin" />
              {stateMessage}
            </span>
          ) : (
            stateMessage
          )}
        </div>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full caption-bottom text-sm">
            <tbody>
              {entries.map((entry, index) => {
                const isDirectory = entry.entryType === "dir";
                const rowIsLast = index === entries.length - 1;
                const openEntry = () =>
                  isDirectory
                    ? openPath(entry.path)
                    : setSelectedFilePath(entry.path);

                return (
                  <tr
                    aria-label={`Open ${isDirectory ? "directory" : "file"} ${entry.name}`}
                    className={cn(
                      "cursor-pointer transition-colors hover:bg-muted/35 focus-visible:bg-muted/35 focus-visible:outline-hidden",
                      !rowIsLast && "border-border/50 border-b",
                    )}
                    key={`${entry.entryType}:${entry.path}`}
                    onClick={openEntry}
                    onKeyDown={(event) => {
                      if (event.key !== "Enter" && event.key !== " ") return;
                      event.preventDefault();
                      openEntry();
                    }}
                    tabIndex={0}
                  >
                    <td className="min-w-52 p-3 align-middle">
                      <div className="flex min-w-0 items-center gap-2">
                        {isDirectory ? (
                          <Folder className="h-4 w-4 shrink-0 fill-sky-500/25 text-sky-500" />
                        ) : (
                          <FileDiff className="h-4 w-4 shrink-0 text-muted-foreground" />
                        )}
                        <span className="truncate font-medium text-foreground">
                          {entry.name}
                        </span>
                      </div>
                    </td>
                    <td className="w-28 whitespace-nowrap p-3 text-right align-middle text-muted-foreground">
                      {isDirectory ? "" : formatFileSize(entry.size)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/** GitHub-backed file browser, one directory fetched at a time. */
export function GithubRepositoryFilesPanel({
  repository,
  branch,
  sourceControls,
}: {
  repository: Repository;
  branch: string | null;
  /** Branch picker + remote/local toggle rendered in the panel header. */
  sourceControls?: RepoSourceHeaderControls;
}) {
  return (
    <GithubRepositoryFilesBrowser
      branch={branch}
      key={branch ?? "no-branch"}
      repository={repository}
      sourceControls={sourceControls}
    />
  );
}
