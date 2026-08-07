import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  connectGithub,
  connectGithubFromGhCli,
  disconnectGithub,
  getGithubConnectionStatus,
} from "@/shared/api/tauriGithub";
import { Button } from "@/shared/ui/button";
import { Dialog, DialogContent } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

export const githubConnectionQueryKey = ["github-connection"] as const;

export function useGithubConnectionQuery() {
  return useQuery({
    queryFn: getGithubConnectionStatus,
    queryKey: githubConnectionQueryKey,
    staleTime: 60_000,
  });
}

/**
 * Connect GitHub: paste a personal-access token or import from an
 * authenticated `gh` CLI. The token is validated against the GitHub API and
 * stored in the OS keyring on the Rust side — it never enters the webview.
 * Connected GitHub unlocks private-repo clones and pull-request viewing.
 */
export function GithubConnectionDialog({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const queryClient = useQueryClient();
  const { goSettings } = useAppNavigation();
  const statusQuery = useGithubConnectionQuery();
  const [token, setToken] = React.useState("");

  const refresh = React.useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: githubConnectionQueryKey });
    void queryClient.invalidateQueries({
      queryKey: ["projects", "work-items"],
    });
  }, [queryClient]);

  const connectMutation = useMutation({
    mutationFn: (pastedToken: string) => connectGithub(pastedToken),
    onError: (error: Error) => toast.error(error.message),
    onSuccess: (status) => {
      toast.success(`Connected to GitHub as ${status.login ?? "unknown"}.`);
      setToken("");
      refresh();
    },
  });
  const ghCliMutation = useMutation({
    mutationFn: connectGithubFromGhCli,
    onError: (error: Error) => toast.error(error.message),
    onSuccess: (status) => {
      toast.success(`Connected to GitHub as ${status.login ?? "unknown"}.`);
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
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <div className="space-y-4">
          <div>
            <h2 className="text-lg font-semibold">GitHub connection</h2>
            <p className="text-sm text-muted-foreground">
              Unlocks private-repository clones and pull-request viewing. The
              token is stored in your OS keychain.
            </p>
          </div>
          <button
            className="text-xs font-medium text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            data-testid="github-manage-in-settings"
            onClick={() => {
              onOpenChange(false);
              void goSettings("integrations");
            }}
            type="button"
          >
            Manage connection in Settings →
          </button>
          {status?.connected ? (
            <div className="flex items-center justify-between rounded-lg border border-border/60 px-3 py-2 text-sm">
              <span data-testid="github-connected-as">
                Connected as <strong>{status.login ?? "unknown"}</strong>
              </span>
              <Button
                data-testid="github-disconnect"
                disabled={busy}
                onClick={() => disconnectMutation.mutate()}
                size="sm"
                variant="outline"
              >
                Disconnect
              </Button>
            </div>
          ) : (
            <>
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
                  Needs <code>repo</code> scope (or fine-grained Contents + Pull
                  requests read access).
                </p>
                <Button
                  data-testid="github-connect"
                  disabled={busy}
                  type="submit"
                >
                  {connectMutation.isPending ? "Connecting…" : "Connect"}
                </Button>
              </form>
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <span>or</span>
                <Button
                  data-testid="github-connect-gh-cli"
                  disabled={busy}
                  onClick={() => ghCliMutation.mutate()}
                  size="sm"
                  variant="outline"
                >
                  {ghCliMutation.isPending ? "Importing…" : "Use gh CLI login"}
                </Button>
              </div>
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
