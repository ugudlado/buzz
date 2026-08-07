// GitHub-backed repository browsing for the "Remote" source on GitHub-linked
// projects (`Repository.githubRepo`). The Rust commands wrap GitHub's
// Contents API, which is inherently per-directory — there is no recursive
// tree fetch — so these hooks fetch lazily: one query per directory the user
// actually opens, and one query per file the user actually views. React
// Query's cache (keyed by owner/repo/branch/path) makes repeat visits free.

import { useQuery } from "@tanstack/react-query";

import type { Repository } from "@/features/projects/projectModels";
import {
  getGithubFileContent,
  getGithubTree,
  listGithubBranches,
  type GithubBranchSummary,
  type GithubTreeEntry,
} from "@/shared/api/tauriGithub";

const README_NAME_PATTERN = /^readme(?:\.(?:md|markdown|mdx|txt))?$/i;

function githubRepoQueryKeyBase(repository: Repository | null | undefined) {
  return repository?.githubRepo
    ? [repository.githubRepo.owner, repository.githubRepo.name]
    : ["none", "none"];
}

/** Branches for a GitHub-linked repository, for the branch picker dropdown. */
export function useGithubRepoBranchesQuery(
  repository: Repository | null | undefined,
) {
  const githubRepo = repository?.githubRepo ?? null;
  return useQuery<GithubBranchSummary[]>({
    enabled: Boolean(githubRepo),
    queryKey: ["github-repo-branches", ...githubRepoQueryKeyBase(repository)],
    queryFn: () => {
      if (!githubRepo) throw new Error("No GitHub repository linked.");
      return listGithubBranches(githubRepo.owner, githubRepo.name);
    },
    staleTime: 30_000,
  });
}

/**
 * Merges relay-derived branch names with a GitHub-linked repository's
 * branches (deduped) — GitHub-linked repositories aren't mirrored on the
 * Buzz relay, so their branch list comes from GitHub's API instead. Returns
 * `branchOptions` unchanged for non-GitHub repositories.
 */
export function useGithubAwareBranchOptions(
  repository: Repository | null | undefined,
  branchOptions: string[],
): string[] {
  const githubBranches = useGithubRepoBranchesQuery(
    repository?.githubRepo ? repository : null,
  ).data;
  if (!repository?.githubRepo) return branchOptions;
  return [
    ...new Set([
      ...branchOptions,
      ...(githubBranches?.map((branch) => branch.name) ?? []),
    ]),
  ];
}

/**
 * One directory's entries (non-recursive). Pass `path: ""` for the repo
 * root. Disabled until a branch is known.
 */
export function useGithubRepoTreeQuery(
  repository: Repository | null | undefined,
  branch: string | null | undefined,
  path: string,
) {
  const githubRepo = repository?.githubRepo ?? null;
  return useQuery<GithubTreeEntry[]>({
    enabled: Boolean(githubRepo && branch),
    queryKey: [
      "github-repo-tree",
      ...githubRepoQueryKeyBase(repository),
      branch ?? "none",
      path,
    ],
    queryFn: () => {
      if (!githubRepo || !branch) {
        throw new Error("No GitHub repository or branch selected.");
      }
      return getGithubTree(githubRepo.owner, githubRepo.name, branch, path);
    },
    staleTime: 30_000,
  });
}

/** A single file's decoded content, fetched on demand (e.g. on click). */
export function useGithubRepoFileQuery(
  repository: Repository | null | undefined,
  branch: string | null | undefined,
  path: string | null,
) {
  const githubRepo = repository?.githubRepo ?? null;
  return useQuery({
    enabled: Boolean(githubRepo && branch && path),
    queryKey: [
      "github-repo-file",
      ...githubRepoQueryKeyBase(repository),
      branch ?? "none",
      path ?? "none",
    ],
    queryFn: () => {
      if (!githubRepo || !branch || !path) {
        throw new Error("No GitHub repository, branch, or path selected.");
      }
      return getGithubFileContent(
        githubRepo.owner,
        githubRepo.name,
        branch,
        path,
      );
    },
    staleTime: 30_000,
  });
}

/**
 * Locates the README in a directory listing (root only — GitHub repos
 * conventionally keep it there) and fetches its content. Returns `null`
 * file/entry when no README is present in the root listing.
 */
export function useGithubReadmeQuery(
  repository: Repository | null | undefined,
  branch: string | null | undefined,
) {
  const rootTreeQuery = useGithubRepoTreeQuery(repository, branch, "");
  const readmeEntry =
    rootTreeQuery.data?.find(
      (entry) =>
        entry.entryType === "file" && README_NAME_PATTERN.test(entry.name),
    ) ?? null;
  const fileQuery = useGithubRepoFileQuery(
    repository,
    branch,
    readmeEntry?.path ?? null,
  );

  // `isLoading` stays true for a *disabled* query, so gate each leg on the
  // query actually being enabled — otherwise "no README in this repo" reads
  // as an infinite spinner instead of an empty state.
  const treeLoading = rootTreeQuery.isFetching && !rootTreeQuery.data;
  const fileLoading =
    Boolean(readmeEntry) && fileQuery.isFetching && !fileQuery.data;

  return {
    entry: readmeEntry,
    isLoading: treeLoading || fileLoading,
    error: rootTreeQuery.error ?? fileQuery.error,
    content: fileQuery.data ?? null,
  };
}
