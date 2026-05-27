#!/usr/bin/env bash
# y7ke-bootstrap-update — best-effort fetch of the latest release.
#
# Wired into systemd as `ExecStartPre=-/usr/local/bin/y7ke-bootstrap-update`
# (the `-` makes failures non-fatal, so a GitHub outage doesn't prevent
# the service from starting with the existing binary).
#
# Identity (`/var/lib/y7ke-bootstrap/identity.key`) is never touched, so
# upgrades preserve the bootstrap's PeerId.
#
# Safe to run manually: `sudo y7ke-bootstrap-update && sudo systemctl restart y7ke-bootstrap`.

set -euo pipefail

REPO="9sx77ssl/y7ke-bootstrap"
INSTALL_PATH="/usr/local/bin/y7ke-bootstrap"

ARCH=$(uname -m)
case "$ARCH" in
    x86_64)  ASSET="y7ke-bootstrap-linux-x86_64" ;;
    aarch64) ASSET="y7ke-bootstrap-linux-aarch64" ;;
    *) echo "[y7ke-bootstrap-update] unsupported arch: $ARCH" >&2; exit 0 ;;
esac

URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

# 10-second connect timeout; ignore the update on flaky network.
if ! curl -fsSL --connect-timeout 10 --max-time 60 -o "$TMP" "$URL"; then
    echo "[y7ke-bootstrap-update] download failed — keeping existing binary"
    exit 0
fi
chmod +x "$TMP"

# Sanity check: the downloaded binary must report a version. If it
# fails to execute at all (wrong arch, corrupt download), skip the
# swap.
if ! "$TMP" --version >/dev/null 2>&1; then
    echo "[y7ke-bootstrap-update] downloaded binary failed --version check — keeping existing binary"
    exit 0
fi

# If identical to current, nothing to do.
if [[ -f "$INSTALL_PATH" ]] && cmp -s "$TMP" "$INSTALL_PATH"; then
    echo "[y7ke-bootstrap-update] already up to date"
    exit 0
fi

# Atomic replace via rename (same filesystem assumed; /tmp and /usr
# may be different mounts).
TARGET_TMP="${INSTALL_PATH}.new"
cp "$TMP" "$TARGET_TMP"
mv "$TARGET_TMP" "$INSTALL_PATH"
echo "[y7ke-bootstrap-update] upgraded $INSTALL_PATH"
