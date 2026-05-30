//! Y7KE Kademlia bootstrap node.
//!
//! A standalone libp2p daemon that participates in the `/y7ke/kad/1.0.0`
//! DHT as a server. It carries no Y7KE app protocols (`/y7ke/handshake`,
//! `/y7ke/msg`, `/y7ke/sync`) and never sees plaintext or ciphertext —
//! its sole job is helping clients find each other's multiaddrs.
//!
//! Run with `RUST_LOG=info,y7ke_bootstrap=debug` for verbose tracing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use futures::StreamExt;
use libp2p::{
    autonat, identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    multiaddr::Protocol,
    ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId, StreamProtocol, SwarmBuilder,
};
use rand::rngs::OsRng;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

const KAD_PROTOCOL: StreamProtocol = StreamProtocol::new("/y7ke/kad/1.0.0");
const IDENTIFY_PROTOCOL: &str = "/y7ke/0.1.0";
const IDENTIFY_AGENT: &str = concat!("y7ke-bootstrap/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Parser)]
#[command(name = "y7ke-bootstrap", version, about)]
struct Args {
    /// TCP port to listen on (both IPv4 and IPv6).
    #[arg(long, default_value_t = 4101)]
    listen_port: u16,

    /// Path to the persistent identity key. Created with mode 0600 on
    /// first run; reused on subsequent runs so the PeerId stays stable.
    #[arg(long, default_value = "/var/lib/y7ke-bootstrap/identity.key")]
    key_path: PathBuf,

    /// Public-facing multiaddrs to advertise as external addresses.
    /// Required for the circuit-relay-v2 server: the reservation
    /// acknowledgement returns these to clients, and clients dial them
    /// back as the relay endpoint. Repeat for multiple (dns4/ip4/ip6).
    /// Also accepted via `Y7KE_BOOTSTRAP_EXTERNAL_ADDR` (comma-separated).
    #[arg(long, value_name = "MULTIADDR")]
    external_addr: Vec<Multiaddr>,
}

#[derive(NetworkBehaviour)]
struct BootBehaviour {
    kad: kad::Behaviour<MemoryStore>,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    relay: relay::Behaviour,
    /// AutoNAT v2 server — answers `DialRequest` from clients by dialing
    /// back over a fresh outbound socket. Pure responder, zero state
    /// beyond per-connection RNG. Lets clients learn their public
    /// reachability without us touching their traffic. See
    /// `docs/V2_GLOBAL_NETWORKING_PLAN.md` §2 in the client repo.
    autonat: autonat::v2::server::Behaviour<OsRng>,
}

impl BootBehaviour {
    fn new(kp: &Keypair) -> Self {
        let peer_id = kp.public().to_peer_id();
        let mut kad_cfg = kad::Config::new(KAD_PROTOCOL);
        kad_cfg.set_periodic_bootstrap_interval(Some(Duration::from_secs(300)));
        let mut kad = kad::Behaviour::with_config(peer_id, MemoryStore::new(peer_id), kad_cfg);
        kad.set_mode(Some(kad::Mode::Server));

        // Identify push of listen-addr updates so clients dialing us
        // learn about new transports (e.g. when QUIC comes online) and
        // about other observed peers without waiting for the periodic
        // tick. Cheap on the bootstrap side; load-bearing on the client
        // side for DCUtR.
        let identify = identify::Behaviour::new(
            identify::Config::new(IDENTIFY_PROTOCOL.to_string(), kp.public())
                .with_agent_version(IDENTIFY_AGENT.to_string())
                .with_interval(Duration::from_secs(300))
                .with_push_listen_addr_updates(true),
        );
        let ping = ping::Behaviour::new(
            ping::Config::new()
                .with_interval(Duration::from_secs(30))
                .with_timeout(Duration::from_secs(10)),
        );

        // Generous limits: this node is a public relay for NAT-bound peers.
        let relay = relay::Behaviour::new(
            peer_id,
            relay::Config {
                max_reservations: 1024,
                max_circuits: 1024,
                max_circuits_per_peer: 16,
                reservation_duration: Duration::from_secs(3600),
                max_circuit_duration: Duration::from_secs(3600),
                max_circuit_bytes: 0,
                ..Default::default()
            },
        );

        let autonat = autonat::v2::server::Behaviour::new(OsRng);

        Self {
            kad,
            identify,
            ping,
            relay,
            autonat,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,y7ke_bootstrap=info")),
        )
        .init();

    let args = Args::parse();
    let keypair = load_or_generate_keypair(&args.key_path)?;
    let peer_id = keypair.public().to_peer_id();
    info!(%peer_id, port = args.listen_port, "y7ke-bootstrap starting");
    println!("PeerId: {peer_id}");

    let mut swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default().nodelay(true),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .context("tcp/noise/yamux setup")?
        .with_quic()
        .with_dns()
        .context("dns transport")?
        .with_behaviour(BootBehaviour::new)
        .context("behaviour setup")?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(600)))
        .build();

    // v4 TCP stays mandatory — a node with no IPv4 TCP is useless.
    swarm
        .listen_on(format!("/ip4/0.0.0.0/tcp/{}", args.listen_port).parse()?)
        .context("listen ipv4 tcp")?;
    // v6 TCP is best-effort, matching the QUIC binds below. A v6-disabled
    // kernel/container — or a dual-stack host where /ip6/:: collides
    // (EADDRINUSE) with the v4-mapped socket — must NOT abort the daemon and
    // crash-loop it (Restart=on-failure), taking v4 TCP + QUIC + relay +
    // AutoNAT + Kad down with it.
    if let Err(e) = swarm.listen_on(format!("/ip6/::/tcp/{}", args.listen_port).parse()?) {
        warn!(error = %e, "listen ipv6 tcp failed (best effort; continuing on IPv4)");
    }
    // QUIC listens on the same port number but UDP, with the libp2p
    // /quic-v1 suffix. AutoNAT v2 explicitly tests addresses with a
    // fresh outbound socket, so the QUIC listener doesn't interfere with
    // the relay TCP path — both serve their roles in parallel.
    if let Err(e) =
        swarm.listen_on(format!("/ip4/0.0.0.0/udp/{}/quic-v1", args.listen_port).parse()?)
    {
        warn!(error = %e, "listen ipv4 quic failed (best effort)");
    }
    if let Err(e) = swarm.listen_on(format!("/ip6/::/udp/{}/quic-v1", args.listen_port).parse()?) {
        warn!(error = %e, "listen ipv6 quic failed (best effort)");
    }

    let mut external_addrs = args.external_addr.clone();
    if let Ok(env_val) = std::env::var("Y7KE_BOOTSTRAP_EXTERNAL_ADDR") {
        for piece in env_val.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match piece.parse::<Multiaddr>() {
                Ok(addr) => external_addrs.push(addr),
                Err(e) => {
                    warn!(value = %piece, error = %e, "ignoring malformed Y7KE_BOOTSTRAP_EXTERNAL_ADDR entry")
                }
            }
        }
    }
    for addr in &external_addrs {
        info!(%addr, "advertising external address");
        swarm.add_external_address(addr.clone());
    }
    if external_addrs.is_empty() {
        warn!("no external addresses configured — circuit-relay reservations will be rejected by clients with NoAddressesInReservation");
    }

    // Print the transport-agnostic shorthand descriptor(s) the Y7KE client
    // expects in its bootstrap list. One line per unique host:port — the
    // client expands each into both /tcp and /udp/quic-v1 and races them,
    // so the operator pastes ONE address and gets both transports.
    let descriptors = shorthand_descriptors(&external_addrs, &peer_id);
    if descriptors.is_empty() {
        warn!("no shorthand descriptor derivable (external addrs lack host+port)");
    } else {
        for d in &descriptors {
            println!("Y7KE bootstrap descriptor (paste into client): {d}");
            info!(descriptor = %d, "client bootstrap descriptor");
        }
    }

    let mut term = signal(SignalKind::terminate()).context("install SIGTERM handler")?;
    let mut intr = signal(SignalKind::interrupt()).context("install SIGINT handler")?;

    loop {
        tokio::select! {
            biased;
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        info!(%address, "listening");
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        let addr = endpoint.get_remote_address();
                        info!(%peer_id, %addr, "peer connected");
                    }
                    SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                        debug!(%peer_id, ?cause, "peer disconnected");
                    }
                    SwarmEvent::Behaviour(BootBehaviourEvent::Identify(ev)) => {
                        if let identify::Event::Received { peer_id, info, .. } = ev {
                            for addr in info.listen_addrs {
                                swarm.behaviour_mut().kad.add_address(&peer_id, addr);
                            }
                        }
                    }
                    SwarmEvent::Behaviour(BootBehaviourEvent::Kad(ev)) => {
                        debug!(?ev, "kad event");
                    }
                    SwarmEvent::Behaviour(BootBehaviourEvent::Ping(_)) => {}
                    SwarmEvent::Behaviour(BootBehaviourEvent::Autonat(ev)) => {
                        debug!(?ev, "autonat server event");
                    }
                    SwarmEvent::Behaviour(BootBehaviourEvent::Relay(ev)) => match ev {
                        relay::Event::ReservationReqAccepted { src_peer_id, .. } => {
                            info!(%src_peer_id, "relay: reservation accepted");
                        }
                        relay::Event::ReservationReqDenied { src_peer_id, .. } => {
                            warn!(%src_peer_id, "relay: reservation denied");
                        }
                        relay::Event::CircuitReqAccepted { src_peer_id, dst_peer_id } => {
                            info!(%src_peer_id, %dst_peer_id, "relay: circuit accepted");
                        }
                        relay::Event::CircuitClosed { .. } => {
                            debug!(?ev, "relay: circuit closed");
                        }
                        _ => debug!(?ev, "relay event"),
                    },
                    other => debug!(?other, "swarm event"),
                }
            }
            _ = term.recv() => {
                info!("SIGTERM received — shutting down");
                break;
            }
            _ = intr.recv() => {
                info!("SIGINT received — shutting down");
                break;
            }
        }
    }
    Ok(())
}

/// Read the persistent Ed25519 keypair from `path`, generating one on
/// first run. The file is written with mode 0600 so other system users
/// can't read it.
/// Collapse external multiaddrs into the transport-agnostic shorthand
/// descriptors the Y7KE client expects: `/{net}/{host}/{port}/p2p/{peer}`
/// (no /tcp or /udp). Each unique (net, host, port) yields one line — the
/// /tcp and /udp/quic-v1 forms of the same endpoint dedup to a single
/// descriptor, since the client re-expands it to both transports.
fn shorthand_descriptors(external: &[Multiaddr], peer: &PeerId) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for addr in external {
        let mut net_host: Option<(&'static str, String)> = None;
        let mut port: Option<u16> = None;
        for proto in addr.iter() {
            match proto {
                Protocol::Dns4(h) => net_host = Some(("dns4", h.to_string())),
                Protocol::Dns6(h) => net_host = Some(("dns6", h.to_string())),
                // Preserve the family-agnostic /dns (A+AAAA) — collapsing it to
                // /dns4 would strand IPv6 even with a published AAAA. The
                // client's expand_bootstrap accepts /dns.
                Protocol::Dns(h) => net_host = Some(("dns", h.to_string())),
                Protocol::Ip4(ip) => net_host = Some(("ip4", ip.to_string())),
                Protocol::Ip6(ip) => net_host = Some(("ip6", ip.to_string())),
                Protocol::Tcp(p) | Protocol::Udp(p) => port = Some(p),
                _ => {}
            }
        }
        if let (Some((net, host)), Some(p)) = (net_host, port) {
            let d = format!("/{net}/{host}/{p}/p2p/{peer}");
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

fn load_or_generate_keypair(path: &Path) -> Result<Keypair> {
    if path.exists() {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        if bytes.len() != 32 {
            anyhow::bail!(
                "identity file {} has unexpected length {} (want 32)",
                path.display(),
                bytes.len()
            );
        }
        let mut tmp = [0u8; 32];
        tmp.copy_from_slice(&bytes);
        let kp = Keypair::ed25519_from_bytes(&mut tmp[..])
            .map_err(|e| anyhow::anyhow!("invalid ed25519 secret: {e}"))?;
        info!(path = %path.display(), "identity loaded");
        Ok(kp)
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create_dir_all {}", parent.display()))?;
        }
        let kp = Keypair::generate_ed25519();
        let secret = kp
            .clone()
            .try_into_ed25519()
            .map_err(|_| anyhow::anyhow!("generated non-ed25519 key"))?
            .to_bytes();
        // The full 64-byte ed25519 keypair from libp2p is (secret || public),
        // but `ed25519_from_bytes` consumes 32 bytes — persist only the secret.
        let secret_only: [u8; 32] = secret[..32].try_into().expect("ed25519 secret is 32 bytes");
        write_secret(path, &secret_only)?;
        warn!(path = %path.display(), "new identity generated");
        Ok(kp)
    }
}

#[cfg(unix)]
fn write_secret(path: &Path, secret: &[u8; 32]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    std::io::Write::write_all(&mut f, secret).context("write identity")?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secret(path: &Path, secret: &[u8; 32]) -> Result<()> {
    std::fs::write(path, secret).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
