import * as React from "react";
import { toast } from "sonner";

import {
  getBacklogConnection,
  setBacklogConnection,
} from "@/features/projects/backlogIssues";
import type { Repository } from "@/features/projects/hooks";
import { PROJECT_FORM_FIELD_CLASS } from "@/features/projects/ui/projectPanelStyles";
import { useSetRepositoryIssueTrackerMutation } from "@/features/projects/useSetRepositoryIssueTracker";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

/**
 * Choose where a repository's issues live: Buzz-native NIP-34 events, or a
 * Backlog project. Saving Backlog also stores the app-wide Backlog connection
 * (server URL + token) used by the issue provider.
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
  const mutation = useSetRepositoryIssueTrackerMutation();
  const [kind, setKind] = React.useState<"buzz" | "backlog">("buzz");
  const [backlogProject, setBacklogProject] = React.useState("");
  const [baseUrl, setBaseUrl] = React.useState("");
  const [token, setToken] = React.useState("");

  const trackerKind = repository.issueTracker.kind;
  const trackerProject =
    repository.issueTracker.kind === "backlog"
      ? repository.issueTracker.project
      : "";
  React.useEffect(() => {
    if (!open) return;
    setKind(trackerKind);
    setBacklogProject(trackerProject);
    const stored = getBacklogConnection();
    setBaseUrl(stored?.baseUrl ?? "");
    setToken(stored?.token ?? "");
  }, [open, trackerKind, trackerProject]);

  async function handleSave(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      if (kind === "backlog") {
        if (!baseUrl.trim() || !token.trim()) {
          throw new Error("Backlog server URL and token are required.");
        }
        setBacklogConnection({ baseUrl: baseUrl.trim(), token: token.trim() });
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

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        className="max-w-md"
        footer={
          <div className="flex w-full justify-end gap-2">
            <Button
              disabled={mutation.isPending}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              data-testid="issue-tracker-save"
              disabled={mutation.isPending}
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
              disabled={mutation.isPending}
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
            <>
              <label
                className="block space-y-1.5 text-sm font-medium"
                htmlFor="issue-tracker-backlog-project"
              >
                <span>Backlog project id</span>
                <Input
                  data-testid="issue-tracker-backlog-project"
                  id="issue-tracker-backlog-project"
                  disabled={mutation.isPending}
                  onChange={(event) => setBacklogProject(event.target.value)}
                  placeholder="Project guid"
                  value={backlogProject}
                />
              </label>
              <label
                className="block space-y-1.5 text-sm font-medium"
                htmlFor="issue-tracker-backlog-url"
              >
                <span>Backlog server URL</span>
                <Input
                  data-testid="issue-tracker-backlog-url"
                  id="issue-tracker-backlog-url"
                  disabled={mutation.isPending}
                  onChange={(event) => setBaseUrl(event.target.value)}
                  placeholder="http://localhost:4321"
                  value={baseUrl}
                />
              </label>
              <label
                className="block space-y-1.5 text-sm font-medium"
                htmlFor="issue-tracker-backlog-token"
              >
                <span>Backlog token</span>
                <Input
                  data-testid="issue-tracker-backlog-token"
                  id="issue-tracker-backlog-token"
                  disabled={mutation.isPending}
                  onChange={(event) => setToken(event.target.value)}
                  placeholder="bklg_…"
                  type="password"
                  value={token}
                />
              </label>
            </>
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
