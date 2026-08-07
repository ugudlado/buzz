import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import type { Repository } from "@/features/projects/hooks";
import type { RepositoryIssueTracker } from "@/features/projects/projectModels";
import { useGithubConnectionQuery } from "@/features/projects/ui/GithubConnectionDialog";
import type {
  CreateProjectInput,
  CreateProjectResult,
} from "@/features/projects/useCreateProject";
import { useSetRepositoryIssueTrackerMutation } from "@/features/projects/useSetRepositoryIssueTracker";
import {
  type BacklogProjectRef,
  getBacklogStatus,
  listBacklogProjects,
} from "@/shared/api/tauriBacklog";
import { listGithubRepos } from "@/shared/api/tauriGithub";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { SearchSelect } from "@/shared/ui/search-select";
import { Textarea } from "@/shared/ui/textarea";

type RepoSource = "github" | "cloneUrl" | "none";
type TrackerProvider = "buzz" | "backlog" | "none";

const CREATE_FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-muted/40 transition-colors duration-150 ease-out hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
const CREATE_FIELD_CONTROL_CLASS =
  "border-0 bg-transparent text-muted-foreground/55 shadow-none outline-none ring-0 transition-colors duration-150 ease-out placeholder:text-muted-foreground/55 focus:bg-transparent focus:text-foreground focus:outline-hidden focus-visible:ring-0";
const CREATE_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";

type CreateProjectDialogProps = {
  isCreating: boolean;
  onCreate: (input: CreateProjectInput) => Promise<CreateProjectResult>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
};

/** Modal for publishing a project with its initial NIP-34 repository. */
export function CreateProjectDialog({
  isCreating,
  onCreate,
  onOpenChange,
  open,
}: CreateProjectDialogProps) {
  const [name, setName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [repoSource, setRepoSource] = React.useState<RepoSource>("none");
  const [githubRepoCloneUrl, setGithubRepoCloneUrl] = React.useState("");
  const [cloneUrl, setCloneUrl] = React.useState("");
  const [trackerProvider, setTrackerProvider] =
    React.useState<TrackerProvider>("buzz");
  const [backlogProjectGuid, setBacklogProjectGuid] = React.useState("");
  const [accessChannelId, setAccessChannelId] = React.useState("");
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const nameInputRef = React.useRef<HTMLInputElement>(null);
  const channelsQuery = useChannelsQuery({ enabled: open });
  const accessChannels = React.useMemo(
    () =>
      (channelsQuery.data ?? []).filter(
        (channel) =>
          channel.isMember &&
          !channel.archivedAt &&
          channel.channelType !== "dm",
      ),
    [channelsQuery.data],
  );

  const githubConnectionQuery = useGithubConnectionQuery();
  const githubConnected = githubConnectionQuery.data?.connected ?? false;
  const githubReposQuery = useQuery({
    enabled: open && repoSource === "github" && githubConnected,
    queryFn: listGithubRepos,
    queryKey: ["github-repos"],
  });

  const backlogStatusQuery = useQuery({
    enabled: open,
    queryFn: getBacklogStatus,
    queryKey: ["backlog-status"],
  });
  const backlogConnected = backlogStatusQuery.data?.connected ?? false;
  const backlogProjectsQuery = useQuery<BacklogProjectRef[]>({
    enabled: open && trackerProvider === "backlog" && backlogConnected,
    queryFn: listBacklogProjects,
    queryKey: ["backlog-projects"],
  });

  const setTrackerMutation = useSetRepositoryIssueTrackerMutation();

  React.useEffect(() => {
    if (!open) return;

    setName("");
    setDescription("");
    setRepoSource("none");
    setGithubRepoCloneUrl("");
    setCloneUrl("");
    setTrackerProvider("buzz");
    setBacklogProjectGuid("");
    setAccessChannelId(accessChannels[0]?.id ?? "");
    setErrorMessage(null);

    // Small delay to let the dialog animation start before focusing.
    const timerId = globalThis.setTimeout(() => {
      nameInputRef.current?.focus();
    }, 50);
    return () => globalThis.clearTimeout(timerId);
  }, [accessChannels, open]);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();

    const trimmedName = name.trim();
    if (!trimmedName || !accessChannelId) return;

    setErrorMessage(null);

    const resolvedCloneUrl =
      repoSource === "github"
        ? githubRepoCloneUrl
        : repoSource === "cloneUrl"
          ? cloneUrl.trim()
          : "";

    try {
      const { project } = await onCreate({
        accessChannelId,
        name: trimmedName,
        description: description.trim() || undefined,
        cloneUrl: resolvedCloneUrl || undefined,
      });

      const repository: Repository | undefined = project.repositories[0];
      if (trackerProvider === "backlog" && backlogProjectGuid && repository) {
        const issueTracker: RepositoryIssueTracker = {
          kind: "backlog",
          project: backlogProjectGuid,
        };
        await setTrackerMutation.mutateAsync({ issueTracker, repository });
      }

      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error ? error.message : "Failed to create project.",
      );
    }
  }

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isCreating) return;
        onOpenChange(nextOpen);
      }}
      open={open}
    >
      <ChooserDialogContent
        className="max-w-lg"
        contentClassName="pt-3"
        data-testid="create-project-dialog"
        description="Projects group one or more repositories published to this workspace's relay."
        footer={
          <div className="flex w-full items-center justify-end gap-3">
            <Button
              data-testid="create-project-submit"
              disabled={
                isCreating || name.trim().length === 0 || !accessChannelId
              }
              form="create-project-form"
              type="submit"
            >
              {isCreating ? "Creating..." : "Create project"}
            </Button>
          </div>
        }
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title="Create a new project"
      >
        <form
          className="space-y-5"
          id="create-project-form"
          onSubmit={(event) => {
            void handleSubmit(event);
          }}
        >
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-project-name"
            >
              Name
            </label>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                CREATE_FIELD_SHELL_CLASS,
              )}
            >
              <Input
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className={cn(
                  "h-8 px-0 py-0 leading-6",
                  CREATE_FIELD_CONTROL_CLASS,
                )}
                data-testid="create-project-name"
                disabled={isCreating}
                id="create-project-name"
                onChange={(event) => {
                  setName(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="bee-garden-game"
                ref={nameInputRef}
                spellCheck={false}
                value={name}
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-project-access-channel"
            >
              Repository access channel
            </label>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                CREATE_FIELD_SHELL_CLASS,
              )}
            >
              <select
                className={cn(
                  "h-8 w-full px-0 py-0",
                  CREATE_FIELD_CONTROL_CLASS,
                )}
                data-testid="create-project-access-channel"
                disabled={isCreating}
                id="create-project-access-channel"
                onChange={(event) => {
                  setAccessChannelId(event.target.value);
                  setErrorMessage(null);
                }}
                required
                value={accessChannelId}
              >
                <option value="">Select a channel</option>
                {accessChannels.map((channel) => (
                  <option key={channel.id} value={channel.id}>
                    {channel.name}
                  </option>
                ))}
              </select>
            </div>
            <p className="text-xs text-muted-foreground">
              Members of this channel can access project repositories.
            </p>
          </div>

          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-project-description"
            >
              Description
              <span className={CREATE_LABEL_OPTIONAL_CLASS}>Optional</span>
            </label>
            <div className={CREATE_FIELD_SHELL_CLASS}>
              <Textarea
                className={cn(
                  "min-h-20 resize-none px-3 py-3 leading-5",
                  CREATE_FIELD_CONTROL_CLASS,
                )}
                data-testid="create-project-description"
                disabled={isCreating}
                id="create-project-description"
                onChange={(event) => {
                  setDescription(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="What this project is about"
                rows={2}
                value={description}
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-project-repo-provider"
            >
              Repository
              <span className={CREATE_LABEL_OPTIONAL_CLASS}>Optional</span>
            </label>
            <p className="text-xs text-muted-foreground">Provider</p>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                CREATE_FIELD_SHELL_CLASS,
              )}
            >
              <select
                className={cn(
                  "h-8 w-full px-0 py-0",
                  CREATE_FIELD_CONTROL_CLASS,
                )}
                data-testid="create-project-repo-provider"
                disabled={isCreating}
                id="create-project-repo-provider"
                onChange={(event) => {
                  setRepoSource(event.target.value as RepoSource);
                  setErrorMessage(null);
                }}
                value={repoSource}
              >
                <option value="none">None</option>
                <option value="github">GitHub</option>
                <option value="cloneUrl">Clone URL</option>
              </select>
            </div>

            {repoSource === "github" ? (
              githubConnected ? (
                <div className="mt-1.5 space-y-1.5">
                  <p className="text-xs text-muted-foreground">Repository</p>
                  <div
                    className={cn(
                      "flex min-h-11 items-center px-3",
                      CREATE_FIELD_SHELL_CLASS,
                    )}
                  >
                    <SearchSelect
                      disabled={isCreating}
                      emptyLabel="No repositories match"
                      getLabel={(repo) => `${repo.owner}/${repo.name}`}
                      getValue={(repo) => repo.cloneUrl}
                      items={githubReposQuery.data ?? []}
                      loading={githubReposQuery.isLoading}
                      loadingLabel="Loading repositories..."
                      onChange={(nextValue) => {
                        setGithubRepoCloneUrl(nextValue);
                        setErrorMessage(null);
                      }}
                      placeholder="Select a repository"
                      searchPlaceholder="Search repositories..."
                      testId="create-project-github-repo"
                      value={githubRepoCloneUrl}
                    />
                  </div>
                </div>
              ) : (
                <p className="mt-1.5 text-xs text-muted-foreground">
                  Connect GitHub in Settings → Integrations to pick a repo.
                </p>
              )
            ) : null}

            {repoSource === "cloneUrl" ? (
              <div
                className={cn(
                  "mt-1.5 flex min-h-11 items-center px-3",
                  CREATE_FIELD_SHELL_CLASS,
                )}
              >
                <Input
                  autoCapitalize="none"
                  autoComplete="off"
                  autoCorrect="off"
                  className={cn(
                    "h-8 px-0 py-0 leading-6",
                    CREATE_FIELD_CONTROL_CLASS,
                  )}
                  data-testid="create-project-clone-url"
                  disabled={isCreating}
                  id="create-project-clone-url"
                  onChange={(event) => {
                    setCloneUrl(event.target.value);
                    setErrorMessage(null);
                  }}
                  placeholder="https://relay.example.com/git/bee-garden-game.git"
                  spellCheck={false}
                  value={cloneUrl}
                />
              </div>
            ) : null}
          </div>

          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-project-tracker-provider"
            >
              Issue tracker
              <span className={CREATE_LABEL_OPTIONAL_CLASS}>Optional</span>
            </label>
            <p className="text-xs text-muted-foreground">Provider</p>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                CREATE_FIELD_SHELL_CLASS,
              )}
            >
              <select
                className={cn(
                  "h-8 w-full px-0 py-0",
                  CREATE_FIELD_CONTROL_CLASS,
                )}
                data-testid="create-project-tracker-provider"
                disabled={isCreating}
                id="create-project-tracker-provider"
                onChange={(event) => {
                  setTrackerProvider(event.target.value as TrackerProvider);
                  setErrorMessage(null);
                }}
                value={trackerProvider}
              >
                <option value="buzz">Buzz issues</option>
                <option value="backlog">Backlog</option>
                <option value="none">None</option>
              </select>
            </div>

            {trackerProvider === "backlog" ? (
              backlogConnected ? (
                <div className="mt-1.5 space-y-1.5">
                  <p className="text-xs text-muted-foreground">Project</p>
                  <div
                    className={cn(
                      "flex min-h-11 items-center px-3",
                      CREATE_FIELD_SHELL_CLASS,
                    )}
                  >
                    <SearchSelect
                      disabled={isCreating}
                      emptyLabel="No projects match"
                      getLabel={(project) => project.path}
                      getValue={(project) => project.guid}
                      items={backlogProjectsQuery.data ?? []}
                      loading={backlogProjectsQuery.isLoading}
                      loadingLabel="Loading projects..."
                      onChange={(nextValue) => {
                        setBacklogProjectGuid(nextValue);
                        setErrorMessage(null);
                      }}
                      placeholder="Select a Backlog project"
                      searchPlaceholder="Search projects..."
                      testId="create-project-backlog-project"
                      value={backlogProjectGuid}
                    />
                  </div>
                </div>
              ) : (
                <p className="mt-1.5 text-xs text-muted-foreground">
                  Connect Backlog in Settings → Integrations to track issues
                  there.
                </p>
              )
            ) : null}
          </div>

          {errorMessage ? (
            <p className="text-sm text-destructive">{errorMessage}</p>
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
