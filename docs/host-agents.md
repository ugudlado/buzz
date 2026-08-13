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

3. Install `buzz-backend-host`, `buzz-acp`, `buzz`, `git-credential-nostr`, and
   the selected ACP runtime in the remote user's `~/.local/bin`. Also ensure
   Git 2.46 or newer is on the remote `PATH`. The local and remote
   `buzz-backend-host` binaries should come from the same Buzz release.

4. Ensure the user service manager survives logout:

   ```bash
   sudo loginctl enable-linger agent
   ssh buzz-vps 'systemctl --user is-system-running'
   ```

Buzz does not enable lingering, modify SSH configuration, copy SSH or runtime
credentials, or disable host-key verification for you. The managed agent's
Buzz identity is transferred over SSH and stored in its private launch file as
described below.

## Set up the remote machine

From a matching Buzz source checkout on the server, run the explicit setup
script as the same Unix user named by the SSH alias:

```bash
./scripts/setup-host-agent.sh \
  --runtime hermes-acp \
  --install-system-deps \
  --enable-linger
```

The two flags authorize the script to use `apt`/`sudo` for build dependencies
and to enable the current user's lingering systemd manager. Omit them when the
host is already prepared. The script builds and installs all Buzz-owned
components under `~/.local/bin`, installs a checksum-pinned user-local Git
2.50.1 when Git is older than 2.46, and verifies the selected ACP runtime.
Runtime installation and credentials remain runtime-specific and are not
guessed by Buzz. The script uses the network for Cargo dependencies and, when
required, the checksum-pinned Git source from `kernel.org`; use the manual
prebuilt-release path below for an offline host.

To use non-default persistent directories:

```bash
./scripts/setup-host-agent.sh \
  --runtime hermes-acp \
  --workspace "$HOME/buzz-workspace" \
  --repos-dir "$HOME/repositories"
```

For a prebuilt release instead of a source checkout, install its binaries
manually:

```bash
mkdir -p "$HOME/.local/bin" "$HOME/REPOS"

for binary in \
  buzz-backend-host \
  buzz-acp \
  buzz-dev-mcp \
  git-credential-nostr \
  buzz
do
  install -m 755 "/path/to/buzz-release/$binary" "$HOME/.local/bin/$binary"
done

# Install the chosen ACP runtime separately. For an existing Hermes setup,
# this should resolve to the wrapper that already uses the remote HOME/config.
command -v hermes-acp
```

Then verify the non-interactive environment the Host provider uses:

```bash
ssh buzz-vps '
  set -eu
  export PATH="$HOME/.local/bin:$PATH"
  git version
  command -v buzz-backend-host
  command -v buzz-acp
  command -v buzz-dev-mcp
  command -v git-credential-nostr
  command -v buzz
  command -v hermes-acp
  systemctl --user show-environment >/dev/null
'
```

The Host provider performs a stricter Git 2.46-or-newer check during deploy.
It also verifies all required commands before writing agent state. A failed
preflight is safe to retry after fixing the remote installation.

Machine setup is intentionally separate from **Add agent**. Desktop may offer
this script/checklist, but creating an agent must not silently run it, install
packages, change `sshd`, enable lingering, or replace an existing harness.
That keeps agent creation repeatable and makes the remote trust change visible
to the operator.

## Add and verify an agent

In the agent dialog, choose the Host backend and enter `buzz-vps` in **Host**.
The optional **Workspace folder** must already exist and defaults to the SSH
user's home. A configured **Repositories folder** must also already exist; if
that field is left blank, Buzz creates and uses `REPOS` inside the workspace.
Both fields accept absolute paths or paths beginning with `~/`.

Select the runtime normally. The runtime command is resolved on the server,
under the SSH user's `HOME` and remote `PATH`; shell startup files are not a
portable credential source, so keep runtime authentication in its normal
remote config or in the agent/persona environment.

Deploying or starting an agent does not clone, fetch, or otherwise inspect a
channel repository. Repository preparation happens just in time before each
new ACP session for a linked channel: initially before that channel's first
session, and again if the harness creates a replacement session.

- A missing checkout is cloned into the repositories folder.
- An existing checkout with an approved origin is fetched from `origin`, so a
  replacement session refreshes its remote-tracking refs before it starts.
- Buzz never pulls, resets, or checks out a branch. The current branch, index,
  dirty tracked files, and untracked files are preserved.
- An existing manually prepared checkout with an unsupported origin is used
  as-is and is not fetched automatically.

Automatic repository network access is restricted to the active Buzz relay's
Git endpoint and public `https://github.com/<owner>/<repo>` URLs. Prepare any
other origin manually in the repositories folder. Git runs non-interactively;
relay repositories use `git-credential-nostr`, while public GitHub clones do
not receive Buzz credentials.

Changing Host, workspace, repositories folder, runtime, or environment settings
does not reconfigure an already active deployment: active deploy is a strict
no-op. Stop the agent, wait for offline presence, then Start it to apply the
new configuration.

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
