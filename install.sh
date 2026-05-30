#!/usr/bin/env bash
# Universal installer for y7ke-bootstrap on Ubuntu / Debian.
# Six steps, nothing more:
#   1. Download the latest release binary for the host architecture.
#   2. Create the y7ke-bootstrap system user (no shell, no home).
#   3. Place the binary at /usr/local/bin/y7ke-bootstrap.
#   4. Install /etc/systemd/system/y7ke-bootstrap.service.
#   5. Open TCP+UDP 4101 (IPv4+IPv6) on any firewall that is *already* active.
#   6. systemctl daemon-reload && enable --now, then print the PeerId.
#
# Re-running this script is safe: the binary is upgraded in place and
# the identity key at /var/lib/y7ke-bootstrap/identity.key is preserved
# so the PeerId remains stable.

set -euo pipefail

REPO="9sx77ssl/y7ke-bootstrap"
BIN_NAME="y7ke-bootstrap"
INSTALL_PATH="/usr/local/bin/${BIN_NAME}"
SERVICE_PATH="/etc/systemd/system/${BIN_NAME}.service"
DATA_DIR="/var/lib/${BIN_NAME}"
LISTEN_PORT="4101"
USER_NAME="y7ke-bootstrap"

log()  { printf "\033[1;36m[y7ke-bootstrap]\033[0m %s\n" "$*"; }
warn() { printf "\033[1;33m[y7ke-bootstrap]\033[0m %s\n" "$*"; }
die()  { printf "\033[1;31m[y7ke-bootstrap]\033[0m %s\n" "$*" >&2; exit 1; }

[[ $EUID -eq 0 ]] || die "this installer needs root (run with sudo)"

# Detect architecture.
ARCH=$(uname -m)
case "$ARCH" in
    x86_64)  ASSET="y7ke-bootstrap-linux-x86_64" ;;
    aarch64) ASSET="y7ke-bootstrap-linux-aarch64" ;;
    *)       die "unsupported architecture: $ARCH" ;;
esac
log "host: $(uname -sr), arch: $ARCH"

# 1. Download the latest release binary.
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
log "downloading ${URL}"
if ! curl -fsSL -o "$TMP/$BIN_NAME" "$URL"; then
    die "download failed — has v0.1.0+ been released yet?"
fi
chmod +x "$TMP/$BIN_NAME"

# 2. Create the system user (idempotent).
if ! id -u "$USER_NAME" >/dev/null 2>&1; then
    log "creating user ${USER_NAME}"
    useradd --system --no-create-home --shell /usr/sbin/nologin "$USER_NAME"
else
    log "user ${USER_NAME} already exists"
fi

# 3. Place the binary.
log "installing ${INSTALL_PATH}"
install -m 0755 "$TMP/$BIN_NAME" "$INSTALL_PATH"

# 3b. Install the self-update helper alongside it. The systemd unit
#     calls this via ExecStartPre on every restart (best-effort, fails
#     silently). The identity key is never touched so the PeerId is
#     stable across upgrades.
log "installing ${INSTALL_PATH}-update"
UPDATE_URL="https://github.com/${REPO}/raw/main/update.sh"
if curl -fsSL -o "${INSTALL_PATH}-update" "$UPDATE_URL"; then
    chmod 0755 "${INSTALL_PATH}-update"
else
    warn "update helper download failed — restart auto-update will be a no-op"
fi

# Data dir for the identity key.
mkdir -p "$DATA_DIR"
chown "$USER_NAME":"$USER_NAME" "$DATA_DIR"
chmod 0750 "$DATA_DIR"

# 4. Install the systemd unit.
log "installing ${SERVICE_PATH}"
cat >"$SERVICE_PATH" <<'SERVICE'
[Unit]
Description=Y7KE Kademlia DHT bootstrap node
Documentation=https://github.com/9sx77ssl/y7ke-bootstrap
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=y7ke-bootstrap
Group=y7ke-bootstrap
# `-+` = best-effort (non-fatal) AND run as root (the daemon itself
# runs unprivileged as User=). Pulls the latest binary from GitHub
# before every restart; identity at /var/lib/y7ke-bootstrap/identity.key
# is never touched, so the PeerId stays stable across upgrades.
ExecStartPre=-+/usr/local/bin/y7ke-bootstrap-update
ExecStart=/usr/local/bin/y7ke-bootstrap
Restart=on-failure
RestartSec=5s
Environment=RUST_LOG=info,y7ke_bootstrap=info
StandardOutput=journal
StandardError=journal
MemoryMax=256M
LimitNOFILE=65535
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
# /usr/local/bin needs to be writable for the self-update step above.
ReadWritePaths=/var/lib/y7ke-bootstrap /usr/local/bin
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictNamespaces=true
RestrictRealtime=true
LockPersonality=true
SystemCallArchitectures=native
AmbientCapabilities=
CapabilityBoundingSet=

[Install]
WantedBy=multi-user.target
SERVICE

# 5. Firewall — only touch one that's already active. We don't install
#    or enable a firewall; the host's owner decides their own policy.
if command -v ufw >/dev/null && ufw status 2>/dev/null | grep -q "Status: active"; then
    # TCP for the relay/Kad path AND UDP for QUIC (the hole-punch transport);
    # ufw rules cover IPv4 + IPv6 by default.
    log "ufw is active — allowing tcp+udp/${LISTEN_PORT}"
    ufw allow "${LISTEN_PORT}/tcp" >/dev/null
    ufw allow "${LISTEN_PORT}/udp" >/dev/null
elif systemctl is-active --quiet firewalld 2>/dev/null; then
    log "firewalld is active — allowing tcp+udp/${LISTEN_PORT}"
    firewall-cmd --permanent --add-port="${LISTEN_PORT}/tcp" >/dev/null
    firewall-cmd --permanent --add-port="${LISTEN_PORT}/udp" >/dev/null
    firewall-cmd --reload >/dev/null
elif command -v iptables >/dev/null && iptables -L INPUT -n 2>/dev/null | grep -qE "^DROP|^REJECT"; then
    log "iptables has restrictive rules — appending ACCEPT for tcp+udp/${LISTEN_PORT} (v4+v6)"
    iptables -I INPUT -p tcp --dport "${LISTEN_PORT}" -j ACCEPT
    iptables -I INPUT -p udp --dport "${LISTEN_PORT}" -j ACCEPT
    # Mirror onto IPv6 if ip6tables is present (best-effort; never fail install).
    if command -v ip6tables >/dev/null; then
        ip6tables -I INPUT -p tcp --dport "${LISTEN_PORT}" -j ACCEPT 2>/dev/null || true
        ip6tables -I INPUT -p udp --dport "${LISTEN_PORT}" -j ACCEPT 2>/dev/null || true
    fi
    if command -v netfilter-persistent >/dev/null; then
        netfilter-persistent save >/dev/null 2>&1 || true
    fi
else
    warn "no active firewall detected — leaving the host untouched"
fi

# 6. Enable + restart. `restart` not `start` so re-runs of this script
#    actually swap in the freshly-downloaded binary instead of leaving
#    the old process running in memory.
log "reloading systemd"
systemctl daemon-reload
INSTALL_TS=$(date +%s)
log "enabling and (re)starting ${BIN_NAME}.service"
systemctl enable "${BIN_NAME}.service" >/dev/null
systemctl restart "${BIN_NAME}.service"

# Poll the journal for up to 15 s — on slow VPSes the systemd startup
# can take a few seconds before the daemon prints its identity. Look
# only at lines emitted since this install started so we don't pick up
# a stale entry from a previous run.
PEER_ID=""
for _ in $(seq 1 15); do
    sleep 1
    PEER_ID=$(journalctl -u "${BIN_NAME}.service" \
                --no-pager \
                --since "@${INSTALL_TS}" \
                --output=cat 2>/dev/null \
              | grep -oE 'PeerId: 12D3KooW[A-Za-z0-9]+' \
              | tail -1 \
              | awk '{print $2}' || true)
    [[ -n "$PEER_ID" ]] && break
done

if [[ -n "${PEER_ID:-}" ]]; then
    log "PeerId: ${PEER_ID}"
    log "Multiaddr: /dns4/$(hostname -f 2>/dev/null || hostname)/tcp/${LISTEN_PORT}/p2p/${PEER_ID}"
else
    warn "could not extract PeerId from journal yet — check 'journalctl -fu ${BIN_NAME}'"
fi

log "done. status: systemctl status ${BIN_NAME}"
log "logs:  journalctl -fu ${BIN_NAME}"
