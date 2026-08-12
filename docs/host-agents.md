# Host agent operations

Use the Host backend when an existing Linux server already has the ACP runtime
and configuration you want Buzz to use. Kubernetes is unnecessary for this
path.

## Prerequisites

1. Give the server a normal SSH config alias. Tailscale may provide the route,
   but Buzz only sees the alias:

   ```sshconfig
   Host buzz-vps
     HostName agent-vps.example-tailnet.ts.net
     User agent
   ```

2. Confirm non-interactive SSH and host-key trust before opening Buzz:

   ```bash
   ssh -o BatchMode=yes buzz-vps true
   ```

3. Install `buzz-backend-host`, `buzz-acp`, `buzz`, and the selected ACP
   runtime in the remote user's `~/.local/bin`. The local and remote
   `buzz-backend-host` binaries should come from the same Buzz release.

4. Ensure the user service manager survives logout:

   ```bash
   sudo loginctl enable-linger agent
   ssh buzz-vps 'systemctl --user is-system-running'
   ```

Buzz does not enable lingering, modify SSH configuration, copy credentials, or
disable host-key verification for you.

## Add and verify an agent

In the agent dialog, choose the Host backend and enter `buzz-vps` in **Host**.
Select the runtime normally. The runtime command is resolved on the server,
under the SSH user's `HOME` and remote `PATH`; shell startup files are not a
portable credential source, so keep runtime authentication in its normal
remote config or in the agent/persona environment.

For an existing Hermes install, choose `hermes-acp`. Its config under the
remote user's home is retained. Buzz sets
`HERMES_ACP_SKIP_CONFIGURED_MCP=1` by default to avoid duplicate MCP startup;
set it to `0` in the agent or persona environment to opt into Hermes's existing
configured MCP servers.

After Start, verify the agent becomes online in Buzz and responds to a mention.
That signed relay presence is the connectivity check: an SSH or TCP probe
cannot prove the agent authenticated to the intended Buzz community. Stop
sends owner-authenticated `!shutdown`; wait for offline presence before
deleting the record.

## Cleanup

Provider protocol v1 has no undeploy operation. Deleting a deployed agent from
Desktop requires an orphan warning confirmation and may leave its stopped
systemd user unit and private generation files on the server. To remove an
agent permanently, first stop it from Buzz, then inspect the provider-owned
units and state on the host before deleting the matching pubkey-derived unit
and directory. Do not delete by name prefix alone: verify the full pubkey and
Host-provider management marker written by the remote helper.

If deployment succeeds but presence never appears, check remote DNS/egress,
the exact `BUZZ_RELAY_URL`, the user journal for the unit, and whether the
runtime command exists in the remote `PATH`. Never paste the private launch
file or its environment into an issue or log bundle.
