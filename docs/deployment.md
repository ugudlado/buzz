# Deployment — Local & Remote

One page for "how do I build/run this here" and "how does it ship there".
Details live in the linked docs; this is the map.

## Local

### Relay + desktop (development)

```bash
. ./bin/activate-hermit
just setup     # once: .env, tools, Docker services, migrations
just dev       # relay (ws://localhost:3000) + desktop app together
```

Split terminals: `just relay` + `just desktop-dev`. See
[README § Quick start](../README.md#quick-start).

**Testing relay-coupled features (workflows, assign-to-agent, agent steps):
always run against the local dev relay** (`ws://localhost:3000`, the default
for `just dev` and the Buzz Dev debug app). `just staging` / `just production`
and installed release builds point at remote relays that may not run your
branch's relay-side code yet — the feature will silently no-op there. After
pulling or merging relay changes, restart the relay (`just relay` rebuilds)
**and** restart managed agents: `buzz-acp` caches the relay's signing pubkey
at startup, so a relay restart with a changed key silently desyncs running
agents.

Postgres/Redis: to use your own instances (e.g. Homebrew) instead of the
docker compose containers, set `BUZZ_EXTERNAL_POSTGRES=1` and/or
`BUZZ_EXTERNAL_REDIS=1` in `.env` — `just` recipes then skip starting and
health-waiting that service. Point `DATABASE_URL`/`REDIS_URL` at your
instance (the default `buzz:buzz_dev@localhost:5432/buzz` role/db must
exist). Unset, docker compose spins them up as before.

### Desktop app — local build

Full unsigned bundle (release profile, same shape CI produces):

```bash
just desktop-release-build                 # aarch64-apple-darwin by default
just desktop-release-build x86_64-apple-darwin
```

Artifact: `desktop/src-tauri/target/<target>/release/bundle/` (macOS `.app`
+ `.dmg` under `macos/`/`dmg/`). Sidecar binaries are stubbed by the recipe —
fine for UI testing; agent-launch features need real sidecars.

Debug dev app ("Buzz Dev", separate bundle id, safe next to a production
install):

```bash
cd desktop
pnpm tauri build --debug --config src-tauri/tauri.conf.dev.json
open "src-tauri/target/debug/bundle/macos/Buzz Dev.app"
```

Gotcha: `tauri build` can re-bundle a **stale** binary. Verify before trusting
a rebuild:

```bash
md5 desktop/src-tauri/target/debug/buzz-desktop
md5 "desktop/src-tauri/target/debug/bundle/macos/Buzz Dev.app/Contents/MacOS/buzz-desktop"
```

If they differ, copy the fresh binary into the bundle and re-sign
(`codesign --force --deep --sign "Buzz Dev Signing" --entitlements
desktop/src-tauri/Entitlements.plist "…/Buzz Dev.app"`). See
`docs/features/dev-build-keychain-signing.md` for the stable dev signing
identity.

### Relay — local production-like

The root `docker-compose.yml` is dev infrastructure only. For a
production-shaped stack on your own machine, use the compose bundle below.

## Remote

### Relay

| Path | When | How |
|------|------|-----|
| Railway | Team relay, no servers | One-click: [Deploy on Railway](https://railway.com/deploy/buzz-relay-block) ([blog](https://engineering.block.xyz/blog/run-your-own-buzz-relay)) |
| Single node / VPS | Self-hosted, one box | [`deploy/compose/`](../deploy/compose/README.md) — Postgres, Redis, MinIO, optional Caddy/TLS. `cp .env.example .env`, fill `CHANGE_ME`s, `./run.sh start` |
| Container image | Your own orchestration | `ghcr.io/block/buzz`, released via `just release-relay` (see [RELEASING.md](../RELEASING.md)) |
| Block staging | Internal | `sprout-oss` builds the image → ECR → `block-coder-tf-stacks` (Terraform + ArgoCD) deploys it. See [AGENTS.md § Ecosystem](../CLAUDE.md) |

### Desktop

Public releases are cut from `main` via release PRs — `just release-desktop
<version>` — producing signed/notarized macOS, unsigned Windows, and Linux
artifacts on the GitHub release. Block-internal signed builds come from
`squareup/buzz-releases`. Full flow: [RELEASING.md](../RELEASING.md).

### Mobile

Immutable candidate tags, no release branch:
`scripts/mobile-release.sh candidate X.Y.Z`, then manual handoff to the
private `buzz-releases` pipeline. Full flow: [RELEASING.md](../RELEASING.md).
