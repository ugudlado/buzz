import * as React from "react";
import { stringify as yamlStringify } from "yaml";

import {
  useCreateWorkflowMutation,
  useUpdateWorkflowMutation,
} from "@/features/workflows/hooks";
import type { Channel, Workflow } from "@/shared/api/types";
import { triggerWorkflow } from "@/shared/api/tauriWorkflows";
import { getRelayHttpUrl } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { ChannelCombobox } from "./ChannelCombobox";
import { WorkflowFormBuilder } from "./WorkflowFormBuilder";
import { WorkflowWebhookSecretDialog } from "./WorkflowWebhookSecretDialog";
import { FieldLabel } from "./workflowFormPrimitives";

type DialogMode = "create" | "edit" | "duplicate";

type WorkflowDialogProps = {
  channels: Channel[];
  mode: DialogMode;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  workflow?: Workflow | null;
  initialDefinition?: Record<string, unknown> | null;
  runAfterCreate?: boolean;
};

function getInitialYaml(
  mode: DialogMode,
  workflow: Workflow | null | undefined,
  initialDefinition: Record<string, unknown> | null | undefined,
): string {
  if (!workflow)
    return initialDefinition ? yamlStringify(initialDefinition) : "";
  const def = { ...workflow.definition };
  if (mode === "duplicate") {
    def.name = `${def.name ?? workflow.name} (copy)`;
  }
  return yamlStringify(def);
}

const TITLES: Record<DialogMode, string> = {
  create: "Create Workflow",
  edit: "Edit Workflow",
  duplicate: "Duplicate Workflow",
};

const SUBMIT_LABELS: Record<DialogMode, string> = {
  create: "Create",
  edit: "Save",
  duplicate: "Create Copy",
};

const PENDING_LABELS: Record<DialogMode, string> = {
  create: "Creating...",
  edit: "Saving...",
  duplicate: "Creating...",
};

export function WorkflowDialog({
  channels,
  mode,
  onOpenChange,
  open,
  workflow,
  initialDefinition,
  runAfterCreate = false,
}: WorkflowDialogProps) {
  const channelId =
    mode === "edit" && workflow?.channelId
      ? workflow.channelId
      : (channels[0]?.id ?? "");

  const [selectedChannelId, setSelectedChannelId] = React.useState(channelId);
  const [yamlDefinition, setYamlDefinition] = React.useState(() =>
    getInitialYaml(mode, workflow, initialDefinition),
  );
  const [savedWebhookInfo, setSavedWebhookInfo] = React.useState<{
    relayHttpUrl: string;
    webhookSecret: string;
    workflowId: string;
  } | null>(null);
  const [submitError, setSubmitError] = React.useState<string | null>(null);
  const [isStarting, setIsStarting] = React.useState(false);

  const createMutation = useCreateWorkflowMutation(selectedChannelId);
  const updateMutation = useUpdateWorkflowMutation(workflow?.id ?? "");
  const mutation = mode === "edit" ? updateMutation : createMutation;

  const selectedChannel =
    channels.find((c) => c.id === selectedChannelId) ?? null;

  const defaultChannelId = channels[0]?.id ?? "";
  const workflowChannelId = workflow?.channelId ?? null;
  const resetCreate = createMutation.reset;
  const resetUpdate = updateMutation.reset;

  // Re-initialize when dialog opens or workflow/mode changes
  React.useEffect(() => {
    if (open) {
      const newChannelId =
        mode === "edit" && workflowChannelId
          ? workflowChannelId
          : defaultChannelId;
      setSelectedChannelId(newChannelId);
      setYamlDefinition(getInitialYaml(mode, workflow, initialDefinition));
      setSavedWebhookInfo(null);
      setSubmitError(null);
      setIsStarting(false);
      resetCreate();
      resetUpdate();
    }
  }, [
    open,
    mode,
    workflow,
    initialDefinition,
    workflowChannelId,
    defaultChannelId,
    resetCreate,
    resetUpdate,
  ]);

  const handleOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      if (!nextOpen) {
        resetCreate();
        resetUpdate();
      }
      onOpenChange(nextOpen);
    },
    [onOpenChange, resetCreate, resetUpdate],
  );

  async function handleSubmit() {
    if (!selectedChannelId || !yamlDefinition.trim()) return;

    try {
      const saved = await mutation.mutateAsync(yamlDefinition);
      if (runAfterCreate && mode === "create") {
        setIsStarting(true);
        try {
          await triggerWorkflow(saved.workflow.id);
        } catch (error) {
          setIsStarting(false);
          setSubmitError(
            `The workflow was created, but could not be started: ${
              error instanceof Error ? error.message : "unknown error"
            }`,
          );
          return;
        }
      }
      handleOpenChange(false);
      if (saved.webhookSecret) {
        const relayHttpUrl = await getRelayHttpUrl();
        setSavedWebhookInfo({
          relayHttpUrl,
          webhookSecret: saved.webhookSecret,
          workflowId: saved.workflow.id,
        });
      }
    } catch {
      // React Query stores the error; keep the dialog open.
    }
  }

  const showChannelSelector = mode !== "edit" && channels.length > 1;
  const showChannelInfo = mode !== "edit" && channels.length === 1;

  return (
    <>
      <Dialog onOpenChange={handleOpenChange} open={open}>
        <DialogContent className="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-lg">
          <DialogHeader className="flex-shrink-0">
            <DialogTitle>
              {runAfterCreate ? "Use Remote Agent" : TITLES[mode]}
            </DialogTitle>
            <DialogDescription>
              {runAfterCreate
                ? "Choose a channel, review the instruction, then run one assignment."
                : mode === "edit"
                  ? "Modify the workflow definition."
                  : channels.length === 1
                    ? "Create a workflow scoped to this channel."
                    : "Define a workflow and assign it to a channel."}
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
            {showChannelSelector ? (
              <div className="space-y-1.5">
                <FieldLabel htmlFor="wf-channel-select">Channel</FieldLabel>
                <ChannelCombobox
                  channels={channels}
                  disabled={mutation.isPending || isStarting}
                  id="wf-channel-select"
                  onChange={(value) => {
                    mutation.reset();
                    setSelectedChannelId(value);
                  }}
                  value={selectedChannelId}
                />
                <p className="text-xs text-muted-foreground">
                  {selectedChannel
                    ? `New workflows will belong to ${selectedChannel.name}.`
                    : "Join or create a channel before adding a workflow."}
                </p>
              </div>
            ) : (showChannelInfo || mode === "edit") && selectedChannel ? (
              <p className="text-sm text-muted-foreground">
                {mode === "edit"
                  ? "Editing workflow in"
                  : "This workflow will be created in"}{" "}
                <span className="font-medium text-foreground">
                  {selectedChannel.name}
                </span>
                .
              </p>
            ) : null}

            <WorkflowFormBuilder
              channelId={selectedChannelId || null}
              disabled={mutation.isPending || isStarting}
              onChange={(yaml) => {
                mutation.reset();
                setYamlDefinition(yaml);
              }}
              yaml={yamlDefinition}
            />

            {mutation.error instanceof Error ? (
              <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {mutation.error.message}
              </p>
            ) : null}
            {submitError ? (
              <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {submitError}
              </p>
            ) : null}
          </div>

          <div className="flex flex-shrink-0 justify-end gap-2 border-t border-border pt-4">
            <Button
              onClick={() => handleOpenChange(false)}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              disabled={
                !selectedChannelId ||
                !yamlDefinition.trim() ||
                mutation.isPending ||
                isStarting
              }
              onClick={handleSubmit}
              type="button"
            >
              {mutation.isPending || isStarting
                ? runAfterCreate
                  ? "Starting..."
                  : PENDING_LABELS[mode]
                : runAfterCreate
                  ? "Create and run"
                  : SUBMIT_LABELS[mode]}
            </Button>
          </div>
        </DialogContent>
      </Dialog>

      {savedWebhookInfo ? (
        <WorkflowWebhookSecretDialog
          onOpenChange={(nextOpen) => {
            if (!nextOpen) {
              setSavedWebhookInfo(null);
            }
          }}
          open
          relayHttpUrl={savedWebhookInfo.relayHttpUrl}
          webhookSecret={savedWebhookInfo.webhookSecret}
          workflowId={savedWebhookInfo.workflowId}
        />
      ) : null}
    </>
  );
}
