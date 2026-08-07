import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { migrateLegacyBacklogConnection } from "@/features/projects/backlogIssues";
import type { Repository } from "@/features/projects/hooks";
import { PROJECT_FORM_FIELD_CLASS } from "@/features/projects/ui/projectPanelStyles";
import { useSetRepositoryIssueTrackerMutation } from "@/features/projects/useSetRepositoryIssueTracker";
import {
  connectBacklog,
  disconnectBacklog,
  getBacklogStatus,
  listBacklogProjects,
  provisionBacklogAgentEnv,
} from "@/shared/api/tauriBacklog";
import { listPersonas, updatePersona } from "@/shared/api/tauriPersonas";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

export const backlogConnectionQueryKey = ["backlog-connection"] as const;
export const backlogProjectsQueryKey = ["backlog-projects"] as const;

// Identity of the pack-generated coordinator persona (kept in sync with
// buzz-workflow's ORCHESTRATOR_STEP_ID and the desktop import's source team).
const ORCHESTRATOR_SOURCE_TEAM = "orchestrator-pack";
const ORCHESTRATOR_SLUG = "orchestrator";
/** Backlog-side agent identity Buzz provisions tokens for. */
const BACKLOG_AGENT_NAME = "buzz-orchestrator";

/**
 * Choose where a repository's issues live: Buzz-native NIP-34 events, or a
 * Backlog project. Handles the Backlog connection (login or token → OS
 * keyring via Rust), lists projects for guid-free binding, and can provision
 * a project-pinned agent token onto the Orchestrator persona's env.
 */
export function RepositoryIssueTrackerDialog({
  onOpenChange,
  open,
  repository,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
  repository: Repository;
}) {
  const queryClient = useQueryClient();
  const { goSettings } = useAppNavigation();
  const mutation = useSetRepositoryIssueTrackerMutation();
  const [kind, setKind] = React.useState<"buzz" | "backlog">("buzz");
  const [backlogProject, setBacklogProject] = React.useState("");
  const [baseUrl, setBaseUrl] = React.useState("http://localhost:4321");
  const [token, setToken] = React.useState("");

  const trackerKind = repository.issueTracker.kind;
  const trackerProject =
    repository.issueTracker.kind === "backlog"
      ? repository.issueTracker.project
      : "";
  const [email, setEmail] = React.useState("");
  const [password, setPassword] = React.useState("");

  const statusQuery = useQuery({
    enabled: open,
    queryFn: async () => {
      await migrateLegacyBacklogConnection();
      return getBacklogStatus();
    },
    queryKey: backlogConnectionQueryKey,
  });
  const connected = statusQuery.data?.connected === true;
  const projectsQuery = useQuery({
    enabled: open && kind === "backlog" && connected,
    queryFn: listBacklogProjects,
    queryKey: backlogProjectsQueryKey,
  });

  React.useEffect(() => {
    if (!open) return;
    setKind(trackerKind);
    setBacklogProject(trackerProject);
  }, [open, trackerKind, trackerProject]);

  function refreshConnection() {
    void queryClient.invalidateQueries({ queryKey: backlogConnectionQueryKey });
    void queryClient.invalidateQueries({ queryKey: backlogProjectsQueryKey });
  }

  const connectMutation = useMutation({
    mutationFn: () =>
      connectBacklog({
        baseUrl,
        ...(token.trim()
          ? { token: token.trim() }
          : { email: email.trim(), password }),
      }),
    onError: (error: Error) => toast.error(error.message),
    onSuccess: (status) => {
      toast.success(`Connected to Backlog as ${status.userName || "unknown"}.`);
      setToken("");
      setPassword("");
      refreshConnection();
    },
  });
  const disconnectMutation = useMutation({
    mutationFn: disconnectBacklog,
    onError: (error: Error) => toast.error(error.message),
    onSuccess: () => {
      toast.success("Backlog disconnected.");
      refreshConnection();
    },
  });
  const provisionMutation = useMutation({
    mutationFn: async () => {
      if (!backlogProject) throw new Error("Choose a Backlog project first.");
      const env = await provisionBacklogAgentEnv(
        BACKLOG_AGENT_NAME,
        backlogProject,
      );
      const personas = await listPersonas();
      const orchestrator = personas.find(
        (persona) =>
          persona.sourceTeam === ORCHESTRATOR_SOURCE_TEAM &&
          persona.sourceTeamPersonaSlug === ORCHESTRATOR_SLUG,
      );
      if (!orchestrator) {
        throw new Error(
          "No Orchestrator persona found. Import the orchestrator pack first.",
        );
      }
      await updatePersona({
        displayName: orchestrator.displayName,
        envVars: { ...orchestrator.envVars, ...env },
        id: orchestrator.id,
        model: orchestrator.model ?? undefined,
        provider: orchestrator.provider ?? undefined,
        runtime: orchestrator.runtime ?? undefined,
        systemPrompt: orchestrator.systemPrompt,
      });
      return orchestrator.displayName;
    },
    onError: (error: Error) => toast.error(error.message),
    onSuccess: (name) => {
      toast.success(`Backlog agent token provisioned onto ${name}.`);
    },
  });

  async function handleSave(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      if (kind === "backlog" && !connected) {
        throw new Error("Connect Backlog first.");
      }
      await mutation.mutateAsync({
        issueTracker:
          kind === "backlog"
            ? { kind: "backlog", project: backlogProject }
            : { kind: "buzz" },
        repository,
      });
      toast.success(
        kind === "backlog"
          ? "Issues now tracked in Backlog."
          : "Issues now tracked in Buzz.",
      );
      onOpenChange(false);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to update the issue tracker.",
      );
    }
  }

  const busy =
    mutation.isPending ||
    connectMutation.isPending ||
    disconnectMutation.isPending ||
    provisionMutation.isPending;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        className="max-w-md"
        footer={
          <div className="flex w-full justify-end gap-2">
            <Button
              disabled={busy}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              data-testid="issue-tracker-save"
              disabled={busy || (kind === "backlog" && !backlogProject)}
              form="issue-tracker-form"
              type="submit"
            >
              {mutation.isPending ? "Saving…" : "Save"}
            </Button>
          </div>
        }
        headerSubtitle={`Where issues for ${repository.name} are tracked.`}
        title="Issue tracker"
      >
        <form
          className="space-y-4"
          id="issue-tracker-form"
          onSubmit={(event) => void handleSave(event)}
        >
          <label className="block space-y-1.5 text-sm font-medium">
            <span>Tracker</span>
            <select
              className={PROJECT_FORM_FIELD_CLASS}
              data-testid="issue-tracker-kind"
              disabled={busy}
              onChange={(event) =>
                setKind(event.target.value === "backlog" ? "backlog" : "buzz")
              }
              value={kind}
            >
              <option value="buzz">Buzz (this community)</option>
              <option value="backlog">Backlog</option>
            </select>
          </label>
          {kind === "backlog" ? (
            connected ? (
              <>
                <div className="flex items-center justify-between rounded-lg border border-border/60 px-3 py-2 text-sm">
                  <span data-testid="backlog-connected-as">
                    Connected as{" "}
                    <strong>{statusQuery.data?.userName || "unknown"}</strong>
                  </span>
                  <Button
                    data-testid="backlog-disconnect"
                    disabled={busy}
                    onClick={() => disconnectMutation.mutate()}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    Disconnect
                  </Button>
                </div>
                <button
                  className="text-xs font-medium text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
                  data-testid="backlog-manage-in-settings"
                  onClick={() => {
                    onOpenChange(false);
                    void goSettings("integrations");
                  }}
                  type="button"
                >
                  Manage connection in Settings →
                </button>
                <label className="block space-y-1.5 text-sm font-medium">
                  <span>Backlog project</span>
                  <select
                    className={PROJECT_FORM_FIELD_CLASS}
                    data-testid="issue-tracker-backlog-project"
                    disabled={busy || projectsQuery.isLoading}
                    onChange={(event) => setBacklogProject(event.target.value)}
                    value={backlogProject}
                  >
                    <option value="">
                      {projectsQuery.isLoading
                        ? "Loading projects…"
                        : "Choose a project"}
                    </option>
                    {(projectsQuery.data ?? []).map((project) => (
                      <option key={project.guid} value={project.guid}>
                        {project.path}
                      </option>
                    ))}
                  </select>
                </label>
                <Button
                  data-testid="backlog-provision-agent"
                  disabled={busy || !backlogProject}
                  onClick={() => provisionMutation.mutate()}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  {provisionMutation.isPending
                    ? "Provisioning…"
                    : "Provision Orchestrator env"}
                </Button>
              </>
            ) : (
              <div className="space-y-2">
                <label
                  className="block space-y-1.5 text-sm font-medium"
                  htmlFor="backlog-url"
                >
                  <span>Backlog server URL</span>
                  <Input
                    data-testid="backlog-url"
                    disabled={busy}
                    id="backlog-url"
                    onChange={(event) => setBaseUrl(event.target.value)}
                    placeholder="http://localhost:4321"
                    value={baseUrl}
                  />
                </label>
                <label
                  className="block space-y-1.5 text-sm font-medium"
                  htmlFor="backlog-token"
                >
                  <span>Token (or sign in below)</span>
                  <Input
                    data-testid="backlog-token"
                    disabled={busy}
                    id="backlog-token"
                    onChange={(event) => setToken(event.target.value)}
                    placeholder="bklg_…"
                    type="password"
                    value={token}
                  />
                </label>
                {!token.trim() ? (
                  <div className="grid grid-cols-2 gap-2">
                    <Input
                      aria-label="Backlog email"
                      data-testid="backlog-email"
                      disabled={busy}
                      onChange={(event) => setEmail(event.target.value)}
                      placeholder="Email"
                      value={email}
                    />
                    <Input
                      aria-label="Backlog password"
                      data-testid="backlog-password"
                      disabled={busy}
                      onChange={(event) => setPassword(event.target.value)}
                      placeholder="Password"
                      type="password"
                      value={password}
                    />
                  </div>
                ) : null}
                <Button
                  data-testid="backlog-connect"
                  disabled={busy}
                  onClick={() => connectMutation.mutate()}
                  size="sm"
                  type="button"
                >
                  {connectMutation.isPending ? "Connecting…" : "Connect"}
                </Button>
                <button
                  className="text-xs font-medium text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
                  data-testid="backlog-manage-in-settings"
                  onClick={() => {
                    onOpenChange(false);
                    void goSettings("integrations");
                  }}
                  type="button"
                >
                  Manage connection in Settings →
                </button>
              </div>
            )
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
