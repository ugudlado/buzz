#!/usr/bin/env bash
set -euo pipefail

GIT_VERSION="2.50.1"
GIT_SHA256="7e3e6c36decbd8f1eedd14d42db6674be03671c2204864befa2a41756c5c8fc4"
RUNTIME="hermes-acp"
WORKSPACE="$HOME"
REPOS_DIR=""
INSTALL_SYSTEM_DEPS=false
ENABLE_LINGER=false
GIT_BUILD_DIR=""

usage() {
  cat <<'EOF'
Usage: scripts/setup-host-agent.sh [options]

Run this explicitly on the Linux host from a matching Buzz source checkout.

Options:
  --runtime COMMAND       ACP runtime to verify (default: hermes-acp)
  --workspace DIR         Agent working directory (default: $HOME)
  --repos-dir DIR         Repository root (default: <workspace>/REPOS)
  --install-system-deps   Use apt/sudo for build dependencies
  --enable-linger         Enable lingering for the current user
  -h, --help              Show this help
EOF
}

while (($#)); do
  case "$1" in
    --runtime) RUNTIME="${2:?--runtime requires a command}"; shift 2 ;;
    --workspace) WORKSPACE="${2:?--workspace requires a directory}"; shift 2 ;;
    --repos-dir) REPOS_DIR="${2:?--repos-dir requires a directory}"; shift 2 ;;
    --install-system-deps) INSTALL_SYSTEM_DEPS=true; shift ;;
    --enable-linger) ENABLE_LINGER=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ "$(uname -s)" == "Linux" ]] || { echo "This script supports Linux hosts only." >&2; exit 1; }
[[ -f Cargo.toml && -f crates/buzz-backend-host/Cargo.toml ]] || {
  echo "Run this from the root of a matching Buzz source checkout." >&2
  exit 1
}
[[ -n "$REPOS_DIR" ]] || REPOS_DIR="$WORKSPACE/REPOS"
[[ -n "$RUNTIME" && "$RUNTIME" != -* && "$RUNTIME" != *$'\n'* && "$RUNTIME" != *$'\r'* ]] || {
  echo "Runtime must name a command and must not start with '-'." >&2
  exit 1
}
for path in "$WORKSPACE" "$REPOS_DIR"; do
  [[ "$path" == /* ]] || { echo "Path must be absolute: $path" >&2; exit 1; }
  [[ "$path" != *$'\n'* && "$path" != *$'\r'* ]] || {
    echo "Paths must not contain control characters." >&2
    exit 1
  }
  [[ ! -e "$path" || -d "$path" ]] || { echo "Not a directory: $path" >&2; exit 1; }
done

export PATH="$HOME/.local/bin:$PATH"
runtime_path="$(type -P -- "$RUNTIME" 2>/dev/null || true)"
[[ -n "$runtime_path" && -x "$runtime_path" ]] || {
  echo "Install and configure the selected ACP runtime first: $RUNTIME" >&2
  exit 1
}
command -v cargo >/dev/null || { echo "Cargo is required." >&2; exit 1; }
command -v systemctl >/dev/null || { echo "systemd user services are required." >&2; exit 1; }

version_at_least() {
  [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]
}

run_privileged() {
  if ((EUID == 0)); then "$@"; else sudo "$@"; fi
}

if $INSTALL_SYSTEM_DEPS || $ENABLE_LINGER; then
  if ((EUID != 0)); then
    command -v sudo >/dev/null || { echo "sudo is required for the selected flags." >&2; exit 1; }
  fi
fi
if $INSTALL_SYSTEM_DEPS; then
  command -v apt-get >/dev/null || {
    echo "--install-system-deps supports apt-based hosts only." >&2
    exit 1
  }
  run_privileged apt-get update
  run_privileged apt-get install -y \
    build-essential ca-certificates curl gettext libcurl4-openssl-dev \
    libexpat1-dev libssl-dev tar xz-utils zlib1g-dev
fi

current_git="$(git version 2>/dev/null | awk '{print $3}' || true)"
need_git=false
if [[ -z "$current_git" ]] || ! version_at_least "$current_git" "2.46.0"; then
  need_git=true
  for command in make gcc curl tar sha256sum; do
    command -v "$command" >/dev/null || {
      echo "Missing $command; use --install-system-deps on Ubuntu/Debian." >&2
      exit 1
    }
  done
fi

# Build before installing. Cargo and the optional Git source download use the
# network unless their inputs are already cached.
cargo build --release --target-dir target \
  -p buzz-backend-host -p buzz-acp -p buzz-dev-mcp \
  -p buzz-cli -p git-credential-nostr

if $need_git; then
  GIT_BUILD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/buzz-git-build.XXXXXX")"
  trap '[[ -z "$GIT_BUILD_DIR" ]] || rm -rf -- "$GIT_BUILD_DIR"' EXIT
  archive="$GIT_BUILD_DIR/git.tar.xz"
  curl -fL --retry 3 \
    "https://www.kernel.org/pub/software/scm/git/git-${GIT_VERSION}.tar.xz" \
    -o "$archive"
  printf '%s  %s\n' "$GIT_SHA256" "$archive" | sha256sum --check --status
  tar -xf "$archive" -C "$GIT_BUILD_DIR"
  make -C "$GIT_BUILD_DIR/git-${GIT_VERSION}" -j2 prefix="$HOME/.local" all
fi

# Mutation boundary. Existing operator directories retain their permissions.
for directory in "$HOME/.local/bin" "$WORKSPACE" "$REPOS_DIR"; do
  [[ -d "$directory" ]] || install -d -m 700 "$directory"
done
if $need_git; then
  make -C "$GIT_BUILD_DIR/git-${GIT_VERSION}" prefix="$HOME/.local" install
fi
for binary in buzz-backend-host buzz-acp buzz-dev-mcp buzz git-credential-nostr; do
  install -m 755 "target/release/$binary" "$HOME/.local/bin/$binary"
done
if $ENABLE_LINGER; then
  run_privileged loginctl enable-linger "$(id -un)"
fi

for command in buzz-backend-host buzz-acp buzz-dev-mcp buzz git-credential-nostr; do
  command -v "$command" >/dev/null || { echo "Installation missing $command" >&2; exit 1; }
done
version_at_least "$(git version | awk '{print $3}')" "2.46.0"
systemctl --user show-environment >/dev/null

cat <<EOF
Host setup complete.
  Workspace folder:    $WORKSPACE
  Repositories folder: $REPOS_DIR
  Runtime:             $RUNTIME

Use these paths in Buzz Desktop's Host provider fields. Add agent performs
deployment only; it never runs this machine-setup script.
EOF
