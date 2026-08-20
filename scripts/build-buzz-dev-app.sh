#!/usr/bin/env bash
# Build, stable-sign, verify, and (re)launch "Buzz Dev.app" for local testing
# (including computer-use automation).
#
# Why this exists: dev bundles used to come out ad-hoc signed — a new random
# code identity every rebuild — so macOS Keychain treated each rebuild as a
# different app and re-prompted for keychain access. tauri.dev.conf.json now
# sets bundle.macOS.signingIdentity = "Buzz Dev Signing" (a locally-trusted
# self-signed cert), giving every rebuild the same code identity. After ONE
# "Always Allow" on the keychain prompt, rebuilds never prompt again.
#
# Usage:
#   ./scripts/build-buzz-dev-app.sh              # build + verify + relaunch
#   ./scripts/build-buzz-dev-app.sh --build-only # build + verify, no launch
set -euo pipefail

IDENTITY="Buzz Dev Signing"
BUNDLE_ID="xyz.block.buzz.app.dev"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$REPO_ROOT/desktop/src-tauri/target/debug/bundle/macos/Buzz Dev.app"

cd "$REPO_ROOT"
. ./bin/activate-hermit

IDENTITIES="$(security find-identity -v -p codesigning)"
if ! grep -q "$IDENTITY" <<<"$IDENTITIES"; then
    cat >&2 <<EOF
ERROR: codesigning identity "$IDENTITY" not found in the login keychain.

One-time setup (creates a self-signed code-signing cert and trusts it):
  cd "\$(mktemp -d)"
  openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem -days 3650 -nodes \\
    -subj "/CN=$IDENTITY" \\
    -addext "keyUsage=critical,digitalSignature" \\
    -addext "extendedKeyUsage=critical,codeSigning" \\
    -addext "basicConstraints=critical,CA:FALSE"
  openssl pkcs12 -export -out cert.p12 -inkey key.pem -in cert.pem -passout pass:buzzdev
  security import cert.p12 -k ~/Library/Keychains/login.keychain-db -P buzzdev -T /usr/bin/codesign
  security add-trusted-cert -p codeSign -k ~/Library/Keychains/login.keychain-db cert.pem
Then verify with: security find-identity -v -p codesigning
EOF
    exit 1
fi

# Real sidecars when available (needed for agent features); stubs otherwise.
if [[ -x "$REPO_ROOT/target/release/buzz-acp" ]]; then
    ./scripts/bundle-sidecars.sh
else
    echo "WARNING: no release sidecars in target/release — bundling zero-byte stubs." >&2
    echo "  Agent launch will not work. Build them with:" >&2
    echo "  cargo build --release -p buzz-acp -p buzz-agent -p buzz-backend-host -p buzz-backend-kubernetes -p buzz-dev-mcp -p git-credential-nostr -p buzz-cli && ./scripts/bundle-sidecars.sh" >&2
    just _ensure-sidecar-stubs
fi

# --no-default-features drops `system-keyring`: dev/test builds store secrets
# in 0o600 files (the designed fallback) instead of the macOS keychain, so no
# keychain dialog can ever appear. A self-signed cert can't fix the dialog for
# good: keychain partition lists only match Apple `teamid:` signers, so macOS
# demands the login-keychain password on every access regardless of "Always
# Allow". First keyring-free launch needs a one-time identity re-import.
(cd desktop && pnpm tauri build --debug --config src-tauri/tauri.dev.conf.json -- --no-default-features)

# Stale-bundle guard: tauri build has been seen re-bundling WITHOUT copying the
# freshly compiled binary. Detect and repair before trusting the build.
SRC="$REPO_ROOT/desktop/src-tauri/target/debug/buzz-desktop"
DST="$APP/Contents/MacOS/buzz-desktop"
if ! cmp -s "$SRC" "$DST"; then
    echo "Stale bundle binary detected — copying fresh binary" >&2
    cp "$SRC" "$DST"
fi

# Tauri applies the dev productName to the bundle path/identifier but the
# static Info.plist template keeps CFBundleName "Buzz" — patch it so the
# menu bar reads "Buzz Dev" (distinguishes from the production install).
/usr/libexec/PlistBuddy -c "Set :CFBundleName 'Buzz Dev'" \
    -c "Set :CFBundleDisplayName 'Buzz Dev'" "$APP/Contents/Info.plist"

# Always re-sign: the plist edit (and any binary repair) invalidates the
# signature Tauri applied. Same stable identity → keychain grants survive.
codesign --force --deep --sign "$IDENTITY" \
    --entitlements "$REPO_ROOT/desktop/src-tauri/Entitlements.plist" "$APP"

# -dvv, not -dv: Authority lines only print at double verbosity. Capture
# before grepping — `codesign | grep -q` under pipefail can fail on SIGPIPE
# even when the Authority line is present.
SIG_INFO="$(codesign -dvv "$APP" 2>&1)"
if ! grep -q "Authority=$IDENTITY" <<<"$SIG_INFO"; then
    echo "ERROR: bundle is not signed with \"$IDENTITY\":" >&2
    echo "$SIG_INFO" >&2
    exit 1
fi
echo "OK: bundle signed with stable identity \"$IDENTITY\""

# Install to /Applications: builds under .worktrees/* are invisible to
# Spotlight/LaunchServices (dot-directory), so computer-use tooling cannot
# resolve "Buzz Dev" by name unless the app lives at a normal path.
INSTALLED="/Applications/Buzz Dev.app"

if [[ "${1:-}" == "--build-only" ]]; then
    echo "Built: $APP (not installed to $INSTALLED)"
    exit 0
fi

# Quit any running instance (graceful, then force) before replacing the install.
osascript -e "tell application id \"$BUNDLE_ID\" to quit" 2>/dev/null || true
for _ in $(seq 1 20); do
    pgrep -f "Buzz Dev.app/Contents/MacOS" >/dev/null || break
    sleep 0.5
done
pkill -f "Buzz Dev.app/Contents/MacOS" 2>/dev/null || true

rm -rf "$INSTALLED"
ditto "$APP" "$INSTALLED"
open "$INSTALLED"
echo "Installed + launched: $INSTALLED"
