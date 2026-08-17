import * as React from "react";

import type { ManagedAgentMarketplace } from "@/shared/api/marketplace";
import type { ManagedAgent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

export function AgentMarketplaceDialog({
  agent,
  onOpenChange,
  onSave,
  open,
  pending,
}: {
  agent: ManagedAgent | null;
  onOpenChange: (open: boolean) => void;
  onSave: (marketplace: ManagedAgentMarketplace) => void;
  open: boolean;
  pending: boolean;
}) {
  const current = agent?.marketplace;
  const [description, setDescription] = React.useState("");
  const [capabilities, setCapabilities] = React.useState("");
  const [deployment, setDeployment] =
    React.useState<ManagedAgentMarketplace["deployment"]>("local");
  const [currency, setCurrency] = React.useState("");
  const [rate, setRate] = React.useState("");
  const [remoteMode, setRemoteMode] = React.useState<
    "off" | "any" | "allowlist"
  >("off");
  const [remoteRelays, setRemoteRelays] = React.useState("");

  React.useEffect(() => {
    if (!open) return;
    setDescription(current?.description ?? "");
    setCapabilities(current?.capabilities.join(", ") ?? "");
    setDeployment(current?.deployment ?? "local");
    setCurrency(current?.pricing?.currency ?? "");
    setRate(current?.pricing?.microunits_per_hour.toString() ?? "");
    setRemoteMode(
      current?.remote_invocation?.policy === "any_community"
        ? "any"
        : current?.remote_invocation?.policy === "allowlist"
          ? "allowlist"
          : "off",
    );
    setRemoteRelays(
      current?.remote_invocation?.policy === "allowlist"
        ? current.remote_invocation.relay_pubkeys.join(", ")
        : "",
    );
  }, [current, open]);

  const parsedRate = rate === "" ? null : Number(rate);
  const priceValid =
    (currency === "" && parsedRate === null) ||
    (/^[A-Z]{3}$/.test(currency) &&
      Number.isSafeInteger(parsedRate) &&
      (parsedRate as number) >= 0);
  const parsedRemoteRelays = remoteRelays
    .split(",")
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean);
  const remoteValid =
    remoteMode !== "allowlist" ||
    (parsedRemoteRelays.length > 0 &&
      parsedRemoteRelays.length <= 100 &&
      parsedRemoteRelays.every((value) => /^[0-9a-f]{64}$/.test(value)));

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {current?.listed ? "Edit" : "Publish"} agent listing
          </DialogTitle>
          <DialogDescription>
            Only this sanitized metadata is published. Runtime and host
            configuration stay local.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <label
            className="block space-y-1 text-xs"
            htmlFor="agent-listing-description"
          >
            <span>Description</span>
            <Textarea
              id="agent-listing-description"
              maxLength={500}
              onChange={(event) => setDescription(event.target.value)}
              value={description}
            />
          </label>
          <label
            className="block space-y-1 text-xs"
            htmlFor="agent-listing-capabilities"
          >
            <span>Capabilities (comma-separated)</span>
            <Input
              id="agent-listing-capabilities"
              onChange={(event) => setCapabilities(event.target.value)}
              value={capabilities}
            />
          </label>
          <label
            className="block space-y-1 text-xs"
            htmlFor="agent-listing-deployment"
          >
            <span>Deployment</span>
            <select
              id="agent-listing-deployment"
              className="h-9 w-full rounded-md border bg-background px-3"
              onChange={(event) =>
                setDeployment(
                  event.target.value as ManagedAgentMarketplace["deployment"],
                )
              }
              value={deployment}
            >
              <option value="local">Local</option>
              <option value="remote">Remote</option>
              <option value="kubernetes">Kubernetes</option>
            </select>
          </label>
          <div className="grid grid-cols-2 gap-3">
            <label
              className="block space-y-1 text-xs"
              htmlFor="agent-listing-currency"
            >
              <span>Currency</span>
              <Input
                id="agent-listing-currency"
                maxLength={3}
                onChange={(event) =>
                  setCurrency(event.target.value.toUpperCase())
                }
                placeholder="USD"
                value={currency}
              />
            </label>
            <label
              className="block space-y-1 text-xs"
              htmlFor="agent-listing-rate"
            >
              <span>Micro-units per hour</span>
              <Input
                id="agent-listing-rate"
                min="0"
                onChange={(event) => setRate(event.target.value)}
                type="number"
                value={rate}
              />
            </label>
          </div>
          <label
            className="block space-y-1 text-xs"
            htmlFor="agent-listing-remote-policy"
          >
            <span>Other communities</span>
            <select
              className="h-9 w-full rounded-md border bg-background px-3"
              id="agent-listing-remote-policy"
              onChange={(event) =>
                setRemoteMode(event.target.value as "off" | "any" | "allowlist")
              }
              value={remoteMode}
            >
              <option value="off">Discovery only</option>
              <option value="any">Allow invocation from any community</option>
              <option value="allowlist">Allow specific relay identities</option>
            </select>
          </label>
          {remoteMode === "allowlist" ? (
            <label
              className="block space-y-1 text-xs"
              htmlFor="agent-listing-remote-relays"
            >
              <span>Allowed relay pubkeys (comma-separated)</span>
              <Textarea
                id="agent-listing-remote-relays"
                onChange={(event) => setRemoteRelays(event.target.value)}
                placeholder="64-character NIP-11 relay pubkeys"
                value={remoteRelays}
              />
            </label>
          ) : null}
        </div>
        <DialogFooter>
          <Button onClick={() => onOpenChange(false)} variant="ghost">
            Cancel
          </Button>
          <Button
            disabled={!agent || !priceValid || !remoteValid || pending}
            onClick={() =>
              onSave({
                listed: true,
                description,
                capabilities: capabilities
                  .split(",")
                  .map((value) => value.trim())
                  .filter(Boolean),
                deployment,
                pricing:
                  parsedRate === null
                    ? null
                    : {
                        currency,
                        microunits_per_hour: parsedRate,
                      },
                remote_invocation:
                  remoteMode === "any"
                    ? { policy: "any_community" }
                    : remoteMode === "allowlist"
                      ? {
                          policy: "allowlist",
                          relay_pubkeys: parsedRemoteRelays,
                        }
                      : null,
              })
            }
          >
            Publish
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
