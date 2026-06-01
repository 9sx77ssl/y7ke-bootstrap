---
description: Run a y7ke-bootstrap task end to end (read → plan → implement → verify → docs → commit), orchestrating the local agents
argument-hint: <what to build / fix / change>
---

Execute the following task for the **y7ke-bootstrap** daemon. You are the orchestrator: keep control, delegate the heavy parts, read each result before the next step. This is a small single-crate stateless daemon — for a one-line change, just do it inline; delegate when it's substantial.

## Task
$ARGUMENTS

## Steps
1. **Read** `CLAUDE.md` (the architecture doc) and `src/main.rs`. Verify every assumption in source — never invent.
2. **Plan** — note the blast radius and any cross-repo impact (protocol IDs / descriptor format vs the `9sx77ssl/y7ke` client). If a real decision is needed (e.g. a protocol-ID change that would break clients), **stop and ask**.
3. **Implement** — delegate to the owning agent:
   - **`bootstrap-daemon`** — `src/main.rs`: libp2p behaviours (kad/relay/autonat/identify/ping), transports, external-addr/descriptor, swarm loop, identity.
   - **`bootstrap-ops`** — deployment: `y7ke-bootstrap.service`, `install.sh`, `update.sh`, `.github/workflows/*`, VPS/firewall, release flow.
4. **Verify** — dispatch **`bootstrap-verifier`**: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release`. If RED, loop back; don't proceed until GREEN.
5. **Review diff** — run `git diff`; every hunk intentional, no secrets, the **stateless / zero-y7ke-deps** invariant held.
6. **Docs** — update `CLAUDE.md` (pins/commands) and `README.md` if behaviour, flags, or deploy changed.
7. **Commit locally** — clear conventional subject. **NEVER `git push`** unless I explicitly ask. Releases are tag-driven (bump `Cargo.toml` version + push a `vX.Y.Z` tag — only when I ask).

## Rules
- **Never invent architecture; verify in `src/main.rs`.**
- **Keep the daemon stateless and free of any `y7ke-*` dependency** — it must never be able to decrypt traffic.
- Protocol IDs (`/y7ke/kad/1.0.0`, identify `/y7ke/0.1.0`) and the descriptor format are a cross-repo contract with the y7ke client — don't break them.

## Definition of done
Gate GREEN, diff reviewed, docs updated, committed locally. Report what changed + the commit, and flag any cross-repo (client) follow-up.
