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

## v0.1.6+: transport-agnostic client descriptor

On startup the daemon prints the **shorthand descriptor** the Y7KE
client expects in its bootstrap list — a transport-AGNOSTIC address
with no `/tcp` or `/udp`:

```
Y7KE bootstrap descriptor (paste into client): /dns4/your-host.example/4101/p2p/12D3KooW…
```

The client expands this one line into BOTH `/tcp/4101` and
`/udp/4101/quic-v1` and races them — QUIC wins on UDP-open networks
(the path that enables direct hole-punch), TCP is the fallback. One
descriptor per unique host:port, derived from the configured external
addresses (so the TCP and QUIC external-addrs of the same endpoint
collapse to a single line). Operators copy this string straight into
the client's Settings → bootstraps field; no transport bookkeeping
on either side. The line is also emitted as a structured
`client bootstrap descriptor` log record for journald.

If the daemon crashes, restarts, or is destroyed, no user content is
lost; clients re-bootstrap from the new instance (or a different one)
the next time they come online.

## v0.2.0+: IPv6 (best-effort) + UDP firewall

The daemon binds IPv6 (`/ip6/::`) for TCP **and** QUIC, but **best-effort**:
a v6-disabled host (or an EADDRINUSE dual-stack collision) only logs a warning
and keeps serving IPv4 — it no longer crash-loops (fixed in v0.2.0). `install.sh`
now opens **both TCP and UDP** on the port, IPv4 **and** IPv6 (QUIC rides UDP).

To actually advertise IPv6 to clients (so two v6-capable peers connect directly
over IPv6, bypassing NAT) the operator must, in addition:

1. Publish an **`AAAA`** record for the host (none exists for `bootstrap1.y7v.lol`
   yet — it's IPv4-only today).
2. Open the **v6 firewall** for the port (the installer's `ip6tables` branch
   does this on iptables hosts; ufw/firewalld cover v6 automatically).
3. Add a v6 external-addr to `Y7KE_BOOTSTRAP_EXTERNAL_ADDR`, e.g.
   `/dns6/your-host.example/tcp/4101,/dns6/your-host.example/udp/4101/quic-v1`
   (or a single `/dns/...` which the daemon now emits as a family-agnostic
   `/dns/...` client descriptor — A+AAAA).

Until the AAAA + firewall + external-addr are in place, IPv6 is **inert** (the
sockets bind but no peer learns a v6 address). This is an ops gap, not a code
gap — the client and daemon code paths are IP-family-agnostic.

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

It **cannot read your messages.** It runs Kademlia (server), circuit-relay-v2
(server), and AutoNAT-v2 (server) on top of identify + ping — that's the whole
surface. Crucially, it has **zero `y7ke-*` dependencies** and never sees a Y7KE
wire type: relayed traffic is Noise- + ChaCha20-Poly1305-encrypted end-to-end
*before* it reaches the circuit, so the relay forwards opaque bytes it can't
decrypt. It is stateless (no message store, no accounts, no plaintext). It is
NOT a gossip pub-sub, a directory, or anything that inspects application data.

## License

MIT OR Apache-2.0 at your option.
