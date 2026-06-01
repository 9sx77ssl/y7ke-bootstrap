# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with
code in this repository. Keep it short and load-bearing; the source code
(`src/main.rs`) is the single source of truth.

> **This repo is its own git repo, checked out NESTED inside the y7ke client
> workspace at `Y7KE/y7ke-bootstrap/`.** The client's `.gitignore` has
> `/y7ke-bootstrap/`, so Y7KE never tracks it, and `Cargo.toml` has an empty
> `[workspace]` table so cargo doesn't claim this crate as a Y7KE workspace
> member. Commits here go to `9sx77ssl/y7ke-bootstrap`, never to the client repo.

## Mandatory task protocol (read first, every task)

Non-negotiable, every task:

**Before starting:**
1. Read this `CLAUDE.md` (it IS the architecture doc — single-crate daemon).
2. Read `src/main.rs` — verify how it actually behaves; never invent.

**Before finishing:**
1. **Review `git diff`** — every hunk intentional, no secrets, the stateless invariant held.
2. **Update docs** — this `CLAUDE.md` + `README.md` if behaviour/flags/deploy changed.
3. **Run the gate** — `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo build --release`.
4. **Commit locally** — always commit a finished task; **never `git push` unless explicitly asked**. Releases are tag-driven (see below).

**Absolute rules:**
- **Never invent architecture.** If it isn't in the code or this file, don't assume it.
- **This node MUST stay stateless and dependency-isolated** (see pins). It can never decrypt Y7KE traffic — that's the whole security premise.
- A task is NOT complete until docs match reality.

## Product

`y7ke-bootstrap` (v0.2.1) is a **standalone, stateless libp2p daemon** for the
Y7KE messenger: a Kademlia DHT **server** + circuit-relay-v2 **server** +
AutoNAT-v2 **server** that helps NAT-bound Y7KE clients discover each other and
hole-punch. It carries **none** of Y7KE's app protocols (`/y7ke/handshake`,
`/y7ke/msg`, `/y7ke/sync`) and never sees plaintext *or* ciphertext — its only
job is routing + reachability. The Y7KE client lives in `9sx77ssl/y7ke`.

## Layout

```
src/main.rs              # the entire daemon (BootBehaviour + swarm loop + CLI + identity)
y7ke-bootstrap.service   # systemd unit (unprivileged user, self-update ExecStartPre, sandboxed)
install.sh               # one-shot VPS installer (release binary, user, systemd, ufw)
update.sh                # best-effort self-update (pulls latest release binary; preserves identity)
.github/workflows/ci.yml      # fmt + clippy -D warnings + release build (push/PR to main)
.github/workflows/release.yml # on v* tag: cross-build x86_64 + aarch64 → GH release
Cargo.lock               # GITIGNORED — binary-only daemon; not committed
```

## Conventions

- **Short, factual comments.** No paragraph blocks restating the code.
- **No speculative scaffolding.** This daemon is intentionally minimal.
- `unsafe_code = "forbid"` (Cargo.toml lints) — keep it.
- No test suite (the behaviour is libp2p wiring). The gate is fmt + clippy + release build. Add a `#[cfg(test)]` module only if you introduce testable logic.

## Useful commands

```bash
cargo build --release                                  # → target/release/y7ke-bootstrap
cargo fmt --all && cargo clippy --all-targets -- -D warnings

# run locally (prints PeerId + the client descriptor to paste into Y7KE Settings)
RUST_LOG=info,y7ke_bootstrap=debug \
  ./target/release/y7ke-bootstrap \
  --listen-port 4101 \
  --key-path /tmp/boot.key \
  --external-addr /dns/bootstrap1.y7v.lol/tcp/4101 \
  --external-addr /dns/bootstrap1.y7v.lol/udp/4101/quic-v1

# cut a release (tag-driven — there is NO auto version bump here)
#   1. bump `version` in Cargo.toml by hand
#   2. git tag vX.Y.Z && git push origin vX.Y.Z   ← release.yml builds + publishes
```

## Architectural pins (do not casually break)

- **Protocol IDs MUST match the Y7KE client.** `KAD_PROTOCOL = /y7ke/kad/1.0.0`
  and identify `IDENTIFY_PROTOCOL = /y7ke/0.1.0` (main.rs:30-31) must stay
  byte-identical to the client's `crates/y7ke-net/src/protocol.rs`. Change one
  here and clients silently can't reach this node. CROSS-REPO contract.
- **Stateless + dependency-isolated.** Deps are libp2p + tokio + clap + tracing
  + anyhow + futures + rand — and **NOTHING `y7ke-*`**. Never add a `y7ke-*`
  dependency: the node must be unable to construct/parse Y7KE wire types, so it
  physically cannot decrypt traffic even if compromised. Keep `MemoryStore` (no
  on-disk DHT state).
- **`--external-addr` is REQUIRED for the relay.** Without it the relay server
  acks reservations with an empty addr list and clients fail with
  `NoAddressesInReservation` (main.rs warns at startup). These are the daemon's
  OWN explicit transport multiaddrs (TCP and QUIC) — NOT the client shorthand.
  Set via `--external-addr` (repeatable) or `Y7KE_BOOTSTRAP_EXTERNAL_ADDR`
  (comma-separated). On the VPS: systemd drop-in
  `/etc/systemd/system/y7ke-bootstrap.service.d/external-addr.conf`.
- **Client descriptor: preserve `/dns`.** `shorthand_descriptors` (main.rs:280)
  prints `/{net}/{host}/{port}/p2p/{peer}` (transport-agnostic — no `/tcp`,
  `/udp`). It maps `Dns4→dns4`, `Dns6→dns6`, and **`Dns→dns` (A+AAAA)** — do NOT
  collapse `/dns` to `/dns4` or you strand IPv6 even with a published AAAA. The
  client's `expand_bootstrap` accepts `/dns` and re-expands to both transports.
- **Kad in Server mode + identify push.** `kad.set_mode(Server)` (main.rs:75) so
  the node answers DHT queries; `with_push_listen_addr_updates(true)`
  (main.rs:86) so clients learn new transports fast — load-bearing for client
  DCUtR. On identify `Received`, peer addrs feed Kad (main.rs:227-233).
- **Dual transport, fixed port; v4 TCP mandatory, rest best-effort.** TCP + QUIC
  on `--listen-port` (4101). **`/ip4/0.0.0.0/tcp` is mandatory** (a node with no
  v4 TCP is useless); `/ip6/::/tcp` and both QUIC binds (`/udp/4101/quic-v1` v4+v6)
  are **best-effort** — they MUST NOT abort the daemon (it runs
  `Restart=on-failure` and would crash-loop, taking relay/AutoNAT/Kad down). The
  VPS firewall must open **both** `4101/tcp` and `4101/udp` (`ufw allow 4101`).
- **Persistent identity = stable PeerId.** Ed25519 secret (32 bytes, mode 0600)
  at `--key-path` (default `/var/lib/y7ke-bootstrap/identity.key`), generated on
  first run, reused after. Upgrades MUST preserve it (a new PeerId orphans every
  client's bootstrap entry).
- **Relay limits** (main.rs:95-106): reservations/circuits 1024, per-peer 16,
  durations 3600s, `max_circuit_bytes = 0` (unlimited). Tune for the VPS; don't remove.

## Deployment (VPS)

- Host: `bootstrap1.y7v.lol` / `89.35.130.67`, port `4101` (TCP + UDP/QUIC).
- `install.sh` → downloads the release binary, creates the unprivileged
  `y7ke-bootstrap` user, installs the systemd unit, opens 4101, `enable --now`.
  Idempotent; identity preserved.
- systemd: `ExecStartPre=-+/usr/local/bin/y7ke-bootstrap-update` (best-effort
  self-update; `-` non-fatal, `+` as root) then the daemon as the unprivileged
  user, sandboxed (`ProtectSystem=strict`, no capabilities, `MemoryMax=256M`).
- Releases **tag-driven**: bump `Cargo.toml` version, push `vX.Y.Z` → `release.yml`
  cross-builds x86_64 + aarch64 → GitHub release; deployed nodes self-update on restart.

## Relationship to the Y7KE client repo (`9sx77ssl/y7ke`)

Intentionally separate so the bootstrap can never link Y7KE crypto/wire code.
The client's `DEFAULT_BOOTSTRAPS` / `DEFAULT_RELAY_BOOTSTRAP` point at this
node's descriptor. When changing anything protocol-facing here (`KAD_PROTOCOL`,
identify version, descriptor format, transports), check the client's
`crates/y7ke-net/src/protocol.rs` + `y7ke_core::expand_bootstrap` stay
compatible. Never vendor any `y7ke-*` code into this repo.
