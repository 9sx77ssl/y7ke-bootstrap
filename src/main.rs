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
    identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    StreamProtocol, SwarmBuilder,
};
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
}

#[derive(NetworkBehaviour)]
struct BootBehaviour {
    kad: kad::Behaviour<MemoryStore>,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    relay: relay::Behaviour,
}

impl BootBehaviour {
    fn new(kp: &Keypair) -> Self {
        let peer_id = kp.public().to_peer_id();
        let mut kad_cfg = kad::Config::new(KAD_PROTOCOL);
        kad_cfg.set_periodic_bootstrap_interval(Some(Duration::from_secs(300)));
        let mut kad = kad::Behaviour::with_config(peer_id, MemoryStore::new(peer_id), kad_cfg);
        kad.set_mode(Some(kad::Mode::Server));

        let identify = identify::Behaviour::new(
            identify::Config::new(IDENTIFY_PROTOCOL.to_string(), kp.public())
                .with_agent_version(IDENTIFY_AGENT.to_string())
                .with_interval(Duration::from_secs(300)),
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

        Self {
            kad,
            identify,
            ping,
            relay,
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
        .with_dns()
        .context("dns transport")?
        .with_behaviour(BootBehaviour::new)
        .context("behaviour setup")?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(600)))
        .build();

    swarm
        .listen_on(format!("/ip4/0.0.0.0/tcp/{}", args.listen_port).parse()?)
        .context("listen ipv4")?;
    swarm
        .listen_on(format!("/ip6/::/tcp/{}", args.listen_port).parse()?)
        .context("listen ipv6")?;

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
