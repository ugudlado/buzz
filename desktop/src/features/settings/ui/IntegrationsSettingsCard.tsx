import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import { migrateLegacyBacklogConnection } from "@/features/projects/backlogIssues";
import { GitHubMark } from "@/features/projects/ui/GitHubMark";
import {
  githubConnectionQueryKey,
  useGithubConnectionQuery,
} from "@/features/projects/ui/GithubConnectionDialog";
import {
  connectGithub,
  connectGithubFromGhCli,
  disconnectGithub,
} from "@/shared/api/tauriGithub";
import {
  connectBacklog,
  disconnectBacklog,
  getBacklogStatus,
} from "@/shared/api/tauriBacklog";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export const backlogConnectionQueryKey = ["backlog-connection"] as const;

function useBacklogConnectionQuery() {
  return useQuery({
    queryFn: async () => {
      await migrateLegacyBacklogConnection();
      return getBacklogStatus();
    },
    queryKey: backlogConnectionQueryKey,
    staleTime: 60_000,
  });
}

/** GitHub row: connect via gh CLI (primary) or paste a token (fallback). */
function GithubIntegrationCard() {
  const queryClient = useQueryClient();
  const statusQuery = useGithubConnectionQuery();
  const [token, setToken] = React.useState("");
  const [showTokenForm, setShowTokenForm] = React.useState(false);

  const refresh = React.useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: githubConnectionQueryKey });
    void queryClient.invalidateQueries({
      queryKey: ["projects", "work-items"],
    });
  }, [queryClient]);

  const ghCliMutation = useMutation({
    mutationFn: connectGithubFromGhCli,
    onError: (error: Error) => toast.error(error.message),
    onSuccess: (status) => {
      toast.success(`Connected to GitHub as ${status.login ?? "unknown"}.`);
      refresh();
    },
  });
  const connectMutation = useMutation({
    mutationFn: (pastedToken: string) => connectGithub(pastedToken),
    onError: (error: Error) => toast.error(error.message),
    onSuccess: (status) => {
      toast.success(`Connected to GitHub as ${status.login ?? "unknown"}.`);
      setToken("");
      setShowTokenForm(false);
      refresh();
    },
  });
  const disconnectMutation = useMutation({
    mutationFn: disconnectGithub,
    onError: (error: Error) => toast.error(error.message),
    onSuccess: () => {
      toast.success("GitHub disconnected.");
      refresh();
    },
  });

  const busy =
    connectMutation.isPending ||
    ghCliMutation.isPending ||
    disconnectMutation.isPending;
  const status = statusQuery.data;

  return (
    <div
      className="rounded-2xl border border-border/60 bg-muted/20 px-4 py-4"
      data-testid="integrations-github"
    >
      <div className="flex items-start gap-3">
        <GitHubMark className="mt-0.5 h-5 w-5 shrink-0 text-foreground" />
        <div className="min-w-0 flex-1 space-y-3">
          <div>
            <p className="text-sm font-medium">GitHub</p>
            <p className="text-sm font-normal text-muted-foreground">
              Unlocks private-repository clones and pull-request viewing. The
              token is stored in your OS keychain.
            </p>
          </div>

          {status?.connected ? (
            <div className="flex items-center justify-between rounded-lg border border-border/60 bg-background px-3 py-2 text-sm">
              <span data-testid="github-connected-as">
                Connected as <strong>{status.login ?? "unknown"}</strong>
              </span>
              <Button
                data-testid="github-disconnect"
                disabled={busy}
                onClick={() => disconnectMutation.mutate()}
                size="sm"
                type="button"
                variant="outline"
              >
                Disconnect
              </Button>
            </div>
          ) : (
            <div className="space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  data-testid="github-connect-cli"
                  disabled={busy}
                  onClick={() => ghCliMutation.mutate()}
                  size="sm"
                  type="button"
                >
                  {ghCliMutation.isPending
                    ? "Importing…"
                    : "Connect with gh CLI"}
                </Button>
                <Button
                  data-testid="github-connect-token-toggle"
                  disabled={busy}
                  onClick={() => setShowTokenForm((prev) => !prev)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Paste token
                </Button>
              </div>
              {showTokenForm ? (
                <form
                  className="space-y-2"
                  onSubmit={(event) => {
                    event.preventDefault();
                    if (!token.trim()) {
                      toast.error("Paste a GitHub token first.");
                      return;
                    }
                    connectMutation.mutate(token);
                  }}
                >
                  <label
                    className="block space-y-1.5 text-sm font-medium"
                    htmlFor="github-token"
                  >
                    <span>Personal access token</span>
                    <Input
                      data-testid="github-token"
                      disabled={busy}
                      id="github-token"
                      onChange={(event) => setToken(event.target.value)}
                      placeholder="ghp_… or github_pat_…"
                      type="password"
                      value={token}
                    />
                  </label>
                  <p className="text-xs text-muted-foreground">
                    Needs <code>repo</code> scope (or fine-grained Contents +
                    Pull requests read access).
                  </p>
                  <Button
                    data-testid="github-connect-token-submit"
                    disabled={busy}
                    size="sm"
                    type="submit"
                  >
                    {connectMutation.isPending ? "Connecting…" : "Connect"}
                  </Button>
                </form>
              ) : null}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** Backlog row: base URL + token (or email/password) connect form. */
function BacklogIntegrationCard() {
  const queryClient = useQueryClient();
  const statusQuery = useBacklogConnectionQuery();
  const [baseUrl, setBaseUrl] = React.useState("http://localhost:4321");
  const [token, setToken] = React.useState("");
  const [email, setEmail] = React.useState("");
  const [password, setPassword] = React.useState("");

  const refresh = React.useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: backlogConnectionQueryKey });
  }, [queryClient]);

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
      refresh();
    },
  });
  const disconnectMutation = useMutation({
    mutationFn: disconnectBacklog,
    onError: (error: Error) => toast.error(error.message),
    onSuccess: () => {
      toast.success("Backlog disconnected.");
      refresh();
    },
  });

  const busy = connectMutation.isPending || disconnectMutation.isPending;
  const status = statusQuery.data;

  return (
    <div
      className="rounded-2xl border border-border/60 bg-muted/20 px-4 py-4"
      data-testid="integrations-backlog"
    >
      <div className="min-w-0 flex-1 space-y-3">
        <div>
          <p className="text-sm font-medium">Backlog</p>
          <p className="text-sm font-normal text-muted-foreground">
            Track project issues in Backlog instead of Buzz. Credentials are
            stored in your OS keychain.
          </p>
        </div>

        {status?.connected ? (
          <div className="flex items-center justify-between rounded-lg border border-border/60 bg-background px-3 py-2 text-sm">
            <span data-testid="backlog-connected-as">
              Connected as <strong>{status.userName || "unknown"}</strong>
              {status.baseUrl ? (
                <span className="text-muted-foreground">
                  {" "}
                  · {status.baseUrl}
                </span>
              ) : null}
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
        ) : (
          <form
            className="space-y-2"
            onSubmit={(event) => {
              event.preventDefault();
              connectMutation.mutate();
            }}
          >
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
              size="sm"
              type="submit"
            >
              {connectMutation.isPending ? "Connecting…" : "Connect"}
            </Button>
          </form>
        )}
      </div>
    </div>
  );
}

/**
 * Settings → Integrations: one connect/disconnect row per provider (GitHub,
 * Backlog). Consolidates the connection-only logic that used to live inside
 * the per-repo `GithubConnectionDialog` / `RepositoryIssueTrackerDialog`
 * dialogs — those keep only the repo-binding concern and link back here.
 */
export function IntegrationsSettingsCard() {
  return (
    <section data-testid="settings-integrations">
      <SettingsSectionHeader
        description="Connect the providers your projects use. Connections are per-device and shared across all your projects."
        title="Integrations"
      />
      <div className="space-y-4">
        <GithubIntegrationCard />
        <BacklogIntegrationCard />
      </div>
    </section>
  );
}
