import { useMutation } from "@tanstack/react-query";
import * as React from "react";

import { installedRemoteAgentDefinition } from "@/features/workflows/marketplace";
import { ChannelCombobox } from "@/features/workflows/ui/ChannelCombobox";
import type { MarketplaceAgent } from "@/shared/api/marketplace";
import { createWorkflow, triggerWorkflow } from "@/shared/api/tauriWorkflows";
import type { Channel, Workflow } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Textarea } from "@/shared/ui/textarea";
import { stringify as yamlStringify } from "yaml";

/**
 * Add a remote agent to this community by installing the workflow that proxies
 * to it. The agent keeps running on its home relay; only the calling stub
 * lives here.
 */
export function InstallAgentDialog({
  agent,
  channels,
  onInstalled,
  onOpenChange,
}: {
  agent: MarketplaceAgent | null;
  channels: Channel[];
  onInstalled: () => void;
  onOpenChange: (open: boolean) => void;
}) {
  const [channelId, setChannelId] = React.useState("");
  React.useEffect(() => {
    if (agent) setChannelId(channels[0]?.id ?? "");
  }, [agent, channels]);

  const installMutation = useMutation({
    mutationFn: async () => {
      if (!agent) throw new Error("no agent selected");
      const definition = installedRemoteAgentDefinition(agent);
      if (!definition) throw new Error("listing is missing its home community");
      return createWorkflow(channelId, yamlStringify(definition));
    },
    onSuccess: onInstalled,
  });
  const install = installMutation.mutate;

  return (
    <Dialog
      onOpenChange={(open) => {
        if (!open) installMutation.reset();
        onOpenChange(open);
      }}
      open={agent !== null}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Add {agent?.name} to this community</DialogTitle>
          <DialogDescription>
            The agent keeps running in {agent?.sourceCommunity?.name}. Answers
            to your requests are posted in the channel you pick.
          </DialogDescription>
        </DialogHeader>
        <ChannelCombobox
          channels={channels}
          disabled={installMutation.isPending}
          onChange={setChannelId}
          value={channelId}
        />
        {installMutation.error instanceof Error ? (
          <p className="text-sm text-destructive">
            {installMutation.error.message}
          </p>
        ) : null}
        <div className="flex justify-end gap-2">
          <Button onClick={() => onOpenChange(false)} variant="outline">
            Cancel
          </Button>
          <Button
            disabled={!channelId || installMutation.isPending}
            onClick={() => install()}
          >
            {installMutation.isPending ? "Adding..." : "Add to community"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** Send a one-off prompt to an installed remote agent. */
export function AskAgentDialog({
  onAsked,
  onOpenChange,
  target,
}: {
  onAsked: () => void;
  onOpenChange: (open: boolean) => void;
  target: { workflow: Workflow; agentName: string } | null;
}) {
  const [prompt, setPrompt] = React.useState("");
  React.useEffect(() => {
    if (target) setPrompt("");
  }, [target]);

  const askMutation = useMutation({
    mutationFn: async () => {
      if (!target) throw new Error("no agent selected");
      return triggerWorkflow(target.workflow.id, { prompt: prompt.trim() });
    },
    onSuccess: onAsked,
  });
  const ask = askMutation.mutate;

  return (
    <Dialog
      onOpenChange={(open) => {
        if (!open) askMutation.reset();
        onOpenChange(open);
      }}
      open={target !== null}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Ask {target?.agentName}</DialogTitle>
          <DialogDescription>
            Runs on the agent&apos;s home community; the answer is posted in
            this community when it completes.
          </DialogDescription>
        </DialogHeader>
        <Textarea
          aria-label="Prompt"
          disabled={askMutation.isPending}
          onChange={(event) => setPrompt(event.target.value)}
          placeholder="What do you want the agent to do?"
          rows={4}
          value={prompt}
        />
        {askMutation.error instanceof Error ? (
          <p className="text-sm text-destructive">
            {askMutation.error.message}
          </p>
        ) : null}
        <div className="flex justify-end gap-2">
          <Button onClick={() => onOpenChange(false)} variant="outline">
            Cancel
          </Button>
          <Button
            disabled={!prompt.trim() || askMutation.isPending}
            onClick={() => ask()}
          >
            {askMutation.isPending ? "Sending..." : "Send"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
