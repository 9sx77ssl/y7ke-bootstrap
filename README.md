# y7ke-bootstrap

Stateless Kademlia DHT bootstrap node for [Y7KE](https://github.com/9sx77ssl/y7ke).

This daemon does **one thing**: participate in the `/y7ke/kad/1.0.0`
DHT so Y7KE clients on the open internet can find each other's
multiaddrs. It carries no application protocols (no
`/y7ke/handshake`, no `/y7ke/msg`, no `/y7ke/sync`), holds no per-user
state, and cannot decrypt traffic — it never sees Y7KE wire types.

## v0.1.3+: circuit relay

From v0.1.3 the bootstrap also acts as a libp2p
[circuit-relay-v2](https://github.com/libp2p/specs/blob/master/relay/circuit-v2.md)
server. Y7KE clients stuck behind NAT or CGNAT can reserve a slot on
the bootstrap and forward their encrypted traffic through it to peers
they otherwise couldn't dial directly. The relay only shuttles
ciphertext frames — Noise + ChaCha20-Poly1305 wrap every byte before
it leaves the client, so the same stateless guarantee holds: the
bootstrap still cannot decrypt anything and holds no per-user state.
Same TCP port (`4101`), same identity key.

**v0.1.4 added required external-address config.** The relay
reservation hand-back to clients lists the bootstrap's publicly
reachable multiaddrs; with no addresses configured, libp2p clients
reject the reservation with `NoAddressesInReservation`. Declare them
with `--external-addr` (repeatable) or the
`Y7KE_BOOTSTRAP_EXTERNAL_ADDR` env var (comma-separated):

```ini
# /etc/systemd/system/y7ke-bootstrap.service.d/external.conf
[Service]
Environment=Y7KE_BOOTSTRAP_EXTERNAL_ADDR=/dns4/your-host.example/tcp/4101
```

## v0.1.5+: QUIC transport + AutoNAT v2 server

From v0.1.5 the daemon listens on QUIC in addition to TCP — same
port number but UDP, with the libp2p `/quic-v1` suffix. Clients that
support QUIC (Y7KE ≥ v0.1.53) dial the bootstrap over QUIC first
and fall back to TCP; the relay and AutoNAT paths work on either
transport. Declare the QUIC external address alongside the TCP one:

```ini
# /etc/systemd/system/y7ke-bootstrap.service.d/external.conf
[Service]
Environment=Y7KE_BOOTSTRAP_EXTERNAL_ADDR=/dns4/your-host.example/tcp/4101,/dns4/your-host.example/udp/4101/quic-v1
```

The same release also runs an **AutoNAT v2 server** behaviour. Y7KE
clients connect to the bootstrap and probe their own external
reachability by asking the bootstrap to dial them back over a fresh
outbound socket. Pure responder, no per-client state. The result
flips a `NatReachability::{Public,Private,Unknown}` verdict in the
client's UI and gates the upgrade-from-relay loop. No operator
action required beyond the QUIC external-addr above.

If the daemon crashes, restarts, or is destroyed, no user content is
lost; clients re-bootstrap from the new instance (or a different one)
the next time they come online.

## One-line install (Ubuntu / Debian)

```bash
bash <(curl -sSL https://github.com/9sx77ssl/y7ke-bootstrap/raw/main/install.sh)
```

The installer:

1. Downloads the latest release binary for your architecture
   (`uname -m` → `x86_64` or `aarch64`).
2. Creates a system user `y7ke-bootstrap` (no shell, no home).
3. Places the binary at `/usr/local/bin/y7ke-bootstrap`.
4. Installs a systemd unit at
   `/etc/systemd/system/y7ke-bootstrap.service`.
5. Opens TCP 4101 **only if** a firewall (`ufw` / `firewalld` /
   restrictive `iptables`) is already active — the script never
   installs or enables a firewall itself.
6. Runs `systemctl daemon-reload && enable --now`, then prints the
   bootstrap's PeerId.

Re-running upgrades the binary in place. The identity key at
`/var/lib/y7ke-bootstrap/identity.key` is preserved, so the PeerId
stays the same.

## Live bootstrap nodes

| name | multiaddr |
|---|---|
| `bootstrap1` (DE) | `/dns4/bootstrap1.y7v.lol/tcp/4101/p2p/12D3KooWEVq9A1w4xk1paGxywwPNy4vz8D92wxE4XKBh8DpA8fSo` |

If the DNS record isn't pointing yet, fall back to the raw IP:
`/ip4/89.35.130.67/tcp/4101/p2p/12D3KooWEVq9A1w4xk1paGxywwPNy4vz8D92wxE4XKBh8DpA8fSo`

Y7KE clients (≥ v0.1.21) auto-discover this in
`y7ke_net::DEFAULT_BOOTSTRAPS`; no client-side config needed.

## Updates

Two ways to update an already-installed node, both safe — the identity
key at `/var/lib/y7ke-bootstrap/identity.key` is never touched, so the
PeerId stays the same.

**Auto-update on restart** (default since v0.1.1). The systemd unit
calls `ExecStartPre=-+/usr/local/bin/y7ke-bootstrap-update` before every
start. The `-` makes failures non-fatal (GitHub outage doesn't block
the service from running); the `+` lets it run as root long enough to
replace `/usr/local/bin/y7ke-bootstrap`. To pick up a new release:

```bash
sudo systemctl restart y7ke-bootstrap
```

**Manual re-run** of the installer also works — it's idempotent:

```bash
bash <(curl -sSL https://github.com/9sx77ssl/y7ke-bootstrap/raw/main/install.sh)
```

To skip the auto-update behaviour (e.g. on an air-gapped host),
comment out the `ExecStartPre=` line in
`/etc/systemd/system/y7ke-bootstrap.service` and run `daemon-reload`.

## Multi-node setups

There is no clustering or shared state. To run more than one bootstrap
node, deploy each on its own host with its own subdomain
(`bootstrap1.y7v.lol`, `bootstrap2.y7v.lol`, …) and add each multiaddr
to the client's bootstrap list. Kad's routing table replicates entries
between bootstraps automatically — no orchestration required.

## Operation

```
systemctl status y7ke-bootstrap     # current state
journalctl -fu y7ke-bootstrap       # follow logs
systemctl restart y7ke-bootstrap    # restart (PeerId preserved)
```

The service is configured with `Restart=on-failure` and `MemoryMax=256M`.
Idle memory is well under that cap; the limit exists to keep a
runaway bug from eating the host.

## What this is *not*

Not a relay, not a NAT-traversal helper, not an AutoNAT server, not a
gossip pub-sub. Those are separate concerns and may ship as additional
features in later Y7KE V2 tracks. This binary's surface stays
intentionally minimal so the security review is trivial: it speaks
identify + ping + kad and nothing else.

## License

MIT OR Apache-2.0 at your option.
