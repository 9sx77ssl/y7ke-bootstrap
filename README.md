# y7ke-bootstrap

Stateless Kademlia DHT bootstrap node for [Y7KE](https://github.com/9sx77ssl/y7ke).

This daemon does **one thing**: participate in the `/y7ke/kad/1.0.0`
DHT so Y7KE clients on the open internet can find each other's
multiaddrs. It carries no application protocols (no
`/y7ke/handshake`, no `/y7ke/msg`, no `/y7ke/sync`), holds no per-user
state, and cannot decrypt traffic — it never sees Y7KE wire types.

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
