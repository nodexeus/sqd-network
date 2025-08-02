use std::time::Duration;
use std::sync::Arc;

use clap::Parser;

mod config;
mod http_server;
mod metrics;
mod state;
use config::{MetricsConfig, DEFAULT_METRICS_HOST, DEFAULT_METRICS_PORT};
use env_logger::Env;
use futures::StreamExt;
use libp2p::{
    autonat,
    gossipsub::{self, MessageAuthenticity},
    identify,
    kad::{self, store::MemoryStore, Mode},
    ping, relay,
    swarm::SwarmEvent,
    PeerId, SwarmBuilder,
};
use libp2p_connection_limits::ConnectionLimits;
use libp2p_swarm_derive::NetworkBehaviour;
use prometheus_client::registry::Registry;
use tokio::signal::unix::{signal, SignalKind};
use tokio::task::JoinHandle;

use sqd_network_transport::{
    get_agent_info,
    protocol::{dht_protocol, ID_PROTOCOL},
    util::{addr_is_reachable, get_keypair},
    AgentInfo, BootNode, Keypair, QuicConfig, TransportArgs, WhitelistBehavior, Wrapped,
};

#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[derive(Parser)]
#[command(
    version, 
    author,
    about = "SQD Network bootnode with monitoring capabilities",
    long_about = "A libp2p bootnode for the SQD Network that helps new peers discover and connect to the network. Includes optional HTTP monitoring endpoints for health checks and Prometheus metrics collection.",
    after_help = "EXAMPLES:
    # Start bootnode with default monitoring on port 9090
    sqd-bootnode --key /path/to/key

    # Start bootnode with custom monitoring port
    sqd-bootnode --key /path/to/key --metrics-port 8080

    # Start bootnode with monitoring on specific interface
    sqd-bootnode --key /path/to/key --metrics-host 127.0.0.1 --metrics-port 3000

    # Start bootnode with monitoring disabled
    sqd-bootnode --key /path/to/key --disable-metrics

    # Using environment variables
    METRICS_HOST=0.0.0.0 METRICS_PORT=9090 sqd-bootnode --key /path/to/key

    # Check bootnode health (when running)
    curl http://localhost:9090/health
    curl http://localhost:9090/ready
    curl http://localhost:9090/metrics"
)]
struct Cli {
    #[command(flatten)]
    transport: TransportArgs,

    #[command(flatten)]
    monitoring: config::MonitoringArgs,

    #[arg(long, env, value_delimiter = ',', help = "Allowed nodes")]
    allowed_nodes: Vec<PeerId>,
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    identify: identify::Behaviour,
    kademlia: kad::Behaviour<MemoryStore>,
    relay: relay::Behaviour,
    gossipsub: gossipsub::Behaviour,
    ping: ping::Behaviour,
    autonat: autonat::Behaviour,
    conn_limits: libp2p_connection_limits::Behaviour,
    whitelist: Wrapped<WhitelistBehavior>,
}

async fn start_metrics_server(
    config: MetricsConfig,
    registry: Arc<Registry>,
    bootnode_state: Arc<state::BootnodeState>,
) -> Option<JoinHandle<()>> {
    if !config.should_start_server() {
        if !config.enabled {
            log::info!("Metrics server disabled via --disable-metrics flag or DISABLE_METRICS environment variable");
        } else {
            log::warn!("Metrics server configuration is invalid (host: '{}', port: {}). Using defaults would be: host='{}', port={}", 
                config.host, config.port, DEFAULT_METRICS_HOST, DEFAULT_METRICS_PORT);
            log::warn!("Continuing without metrics server. Check your --metrics-host and --metrics-port arguments");
        }
        return None;
    }

    log::debug!("Starting metrics HTTP server on {}:{}", config.host, config.port);
    log::debug!("Metrics endpoints will be available at:");
    log::debug!("  - Health check: http://{}:{}/health", config.host, config.port);
    log::debug!("  - Readiness check: http://{}:{}/ready", config.host, config.port);
    log::debug!("  - Prometheus metrics: http://{}:{}/metrics", config.host, config.port);

    // Validate configuration before attempting to start
    if let Err(e) = config.socket_addr() {
        log::error!("Invalid metrics server socket address '{}:{}': {}", config.host, config.port, e);
        log::error!("Please check your --metrics-host and --metrics-port configuration");
        log::warn!("Continuing without metrics server - core bootnode functionality unaffected");
        return None;
    }

    let server = http_server::MetricsServer::new(registry, bootnode_state);
    let host = config.host.clone();
    let port = config.port;

    // Test server binding before spawning the task
    match tokio::net::TcpListener::bind(format!("{}:{}", host, port)).await {
        Ok(test_listener) => {
            // Close the test listener immediately
            drop(test_listener);
            log::debug!("Metrics server successfully bound to {}:{}", host, port);
        }
        Err(e) => {
            log::error!("Failed to bind metrics server to {}:{}: {}", host, port, e);
            match e.kind() {
                std::io::ErrorKind::AddrInUse => {
                    log::error!("Port {} is already in use. Try a different port with --metrics-port", port);
                }
                std::io::ErrorKind::PermissionDenied => {
                    log::error!("Permission denied binding to port {}. Try a port above 1023 or run with appropriate privileges", port);
                }
                std::io::ErrorKind::AddrNotAvailable => {
                    log::error!("Address {}:{} is not available. Check your --metrics-host setting", host, port);
                }
                _ => {
                    log::error!("Network error: {}", e);
                }
            }
            log::warn!("Continuing without metrics server - core bootnode functionality unaffected");
            return None;
        }
    }

    Some(tokio::spawn(async move {
        log::debug!("Metrics server task started successfully");
        match server.run(host.clone(), port).await {
            Ok(()) => {
                log::info!("Metrics server on {}:{} shut down gracefully", host, port);
            }
            Err(e) => {
                log::error!("Metrics server on {}:{} encountered an error: {}", host, port, e);
                log::error!("This may be due to network issues or server overload");
                log::warn!("Core bootnode functionality continues unaffected");
            }
        }
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Init logging and parse arguments
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    let listen_addrs = cli.transport.listen_addrs();
    let keypair = get_keypair(Some(cli.transport.key)).await?;
    let local_peer_id = PeerId::from(keypair.public());
    log::info!("Local peer ID: {local_peer_id}");

    let contract_client = sqd_contract_client::get_client(&cli.transport.rpc).await?;

    // Initialize monitoring components
    let metrics_config = MetricsConfig::from_monitoring_args(&cli.monitoring);
    let mut registry = Registry::default();
    let bootnode_state = Arc::new(state::BootnodeState::new());
    
    // Log monitoring configuration
    if metrics_config.enabled {
        log::debug!("Monitoring enabled - HTTP server will bind to {}:{}", metrics_config.host, metrics_config.port);
        log::debug!("Metrics endpoints: /health, /ready, /metrics");
        metrics::register_metrics(&mut registry);
        log::debug!("Prometheus metrics registry initialized successfully");
    } else {
        log::info!("Monitoring disabled - no HTTP server or metrics collection will be started");
    }
    
    let registry = Arc::new(registry);

    // Prepare behaviour & transport
    let autonat_config = autonat::Config {
        timeout: Duration::from_secs(60),
        throttle_clients_global_max: 64,
        throttle_clients_peer_max: 16,
        ..Default::default()
    };
    let mut kad_config = kad::Config::new(dht_protocol(cli.transport.rpc.network));
    kad_config.set_replication_factor(20.try_into().unwrap());
    let agent_info: AgentInfo = get_agent_info!();
    let behaviour = |keypair: &Keypair| Behaviour {
        identify: identify::Behaviour::new(
            identify::Config::new(ID_PROTOCOL.to_string(), keypair.public())
                .with_interval(Duration::from_secs(60))
                .with_push_listen_addr_updates(true)
                .with_agent_version(agent_info.to_string()),
        ),
        kademlia: kad::Behaviour::with_config(
            local_peer_id,
            MemoryStore::new(local_peer_id),
            kad_config,
        ),
        relay: relay::Behaviour::new(local_peer_id, Default::default()),
        gossipsub: gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(keypair.clone()),
            Default::default(),
        )
        .unwrap(),
        ping: ping::Behaviour::new(Default::default()),
        autonat: autonat::Behaviour::new(local_peer_id, autonat_config),
        conn_limits: libp2p_connection_limits::Behaviour::new(
            ConnectionLimits::default().with_max_established_per_peer(Some(3)),
        ),
        whitelist: WhitelistBehavior::new(contract_client, Default::default()).into(),
    };

    // Start the swarm
    let quic_config = QuicConfig::from_env();
    let mut swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_quic_config(|config| config.mtu_upper_bound(quic_config.mtu_discovery_max))
        .with_dns()?
        .with_behaviour(behaviour)
        .expect("infallible")
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(120)))
        .build();
    for listen_addr in listen_addrs {
        log::info!("Listening on {}", listen_addr);
        swarm.listen_on(listen_addr)?;
    }
    for public_addr in cli.transport.p2p_public_addrs {
        log::info!("Adding public address {public_addr}");
        swarm.add_external_address(public_addr);
    }

    swarm.behaviour_mut().kademlia.set_mode(Some(Mode::Server));

    // Start metrics server if enabled
    let _metrics_server_handle: Option<JoinHandle<()>> = if metrics_config.should_start_server() {
        match start_metrics_server(metrics_config.clone(), registry.clone(), bootnode_state.clone()).await {
            Some(handle) => {
                log::debug!("✓ Metrics server successfully started and ready to accept connections");
                log::debug!("  You can now monitor the bootnode using:");
                log::debug!("  curl http://{}:{}/health", metrics_config.host, metrics_config.port);
                log::debug!("  curl http://{}:{}/ready", metrics_config.host, metrics_config.port);
                log::debug!("  curl http://{}:{}/metrics", metrics_config.host, metrics_config.port);
                Some(handle)
            }
            None => {
                log::warn!("✗ Metrics server startup failed - see error messages above");
                log::warn!("  Bootnode will continue operating normally without monitoring");
                log::warn!("  To enable monitoring, fix the configuration and restart");
                None
            }
        }
    } else {
        log::info!("Metrics server disabled - no monitoring endpoints available");
        None
    };

    // Connect to other boot nodes
    for BootNode { peer_id, address } in cli
        .transport
        .boot_nodes
        .into_iter()
        .filter(|node| node.peer_id != local_peer_id)
    {
        log::info!("Connecting to boot node {peer_id} at {address}");
        swarm.behaviour_mut().whitelist.allow_peer(peer_id);
        swarm.behaviour_mut().kademlia.add_address(&peer_id, address);
        swarm.dial(peer_id)?;
    }

    for peer_id in cli.allowed_nodes {
        log::info!("Adding allowed peer {peer_id}");
        swarm.behaviour_mut().whitelist.allow_peer(peer_id);
    }

    // Start periodic uptime update task if metrics are enabled
    let _uptime_task_handle: Option<JoinHandle<()>> = if metrics_config.enabled {
        let state_clone = bootnode_state.clone();
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            log::debug!("Starting uptime metrics update task (10 second interval)");
            loop {
                interval.tick().await;
                if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    metrics::update_uptime(state_clone.start_time);
                })) {
                    log::error!("Metrics collection error: uptime update task failed");
                    log::debug!("Uptime metrics may be stale until next successful update");
                }
            }
        }))
    } else {
        None
    };

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    loop {
        let event = tokio::select! {
            event = swarm.select_next_some() => event,
            _ = sigint.recv() => break,
            _ = sigterm.recv() => break,
        };
        log::trace!("Swarm event: {event:?}");
        
        // Process events and update metrics/state with error handling
        match &event {
            SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info: identify::Info { listen_addrs, .. },
                ..
            })) => {
                for addr in listen_addrs.into_iter().filter(|addr| addr_is_reachable(addr)) {
                    swarm.behaviour_mut().kademlia.add_address(peer_id, addr.clone());
                }
                if metrics_config.enabled {
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event("identify_received");
                    })) {
                        log::error!("Metrics collection error: failed to record identify_received event");
                        log::debug!("This may indicate memory pressure or metrics registry issues");
                    }
                }
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                log::debug!("Connection established with peer: {}", peer_id);
                if metrics_config.enabled {
                    bootnode_state.peer_connected();
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event("connection_established");
                    })) {
                        log::error!("Metrics collection error: failed to record connection_established event for peer {}", peer_id);
                        log::debug!("Peer connection tracking may be inaccurate until metrics system recovers");
                    }
                }
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                log::debug!("Connection closed with peer: {} (cause: {:?})", peer_id, cause);
                if metrics_config.enabled {
                    bootnode_state.peer_disconnected();
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event("connection_closed");
                    })) {
                        log::error!("Metrics collection error: failed to record connection_closed event for peer {} (cause: {:?})", peer_id, cause);
                        log::debug!("Peer disconnection tracking may be inaccurate until metrics system recovers");
                    }
                }
            }
            SwarmEvent::IncomingConnection { .. } => {
                if metrics_config.enabled {
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::increment_connection_attempts();
                        metrics::record_network_event("incoming_connection");
                    })) {
                        log::error!("Metrics collection error: failed to record incoming connection attempt");
                        log::debug!("Connection attempt counters may be inaccurate");
                    }
                }
            }
            SwarmEvent::IncomingConnectionError { error, .. } => {
                log::debug!("Incoming connection error: {}", error);
                if metrics_config.enabled {
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::increment_failed_connections();
                        metrics::record_network_event("incoming_connection_error");
                    })) {
                        log::error!("Metrics collection error: failed to record incoming connection error: {}", error);
                        log::debug!("Failed connection counters may be inaccurate");
                    }
                }
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                log::debug!("Outgoing connection error to {:?}: {}", peer_id, error);
                if metrics_config.enabled {
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::increment_failed_connections();
                        metrics::record_network_event("outgoing_connection_error");
                    })) {
                        log::error!("Metrics collection error: failed to record outgoing connection error to {:?}: {}", peer_id, error);
                        log::debug!("Failed connection counters may be inaccurate");
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(kad::Event::RoutingUpdated { 
                peer, 
                is_new_peer,
                .. 
            })) => {
                if *is_new_peer {
                    log::debug!("New peer added to routing table: {}", peer);
                }
                if metrics_config.enabled {
                    // Mark DHT as ready when we have routing table entries
                    if !bootnode_state.is_dht_ready() {
                        bootnode_state.set_dht_ready(true);
                        log::debug!("DHT readiness achieved - /ready endpoint will now return ready=true");
                    }
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event("routing_updated");
                    })) {
                        log::error!("Metrics collection error: failed to record routing table update for peer {}", peer);
                        log::debug!("DHT routing metrics may be inaccurate");
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(kad::Event::InboundRequest { .. })) => {
                if metrics_config.enabled {
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event("kademlia_inbound_request");
                    })) {
                        log::error!("Metrics collection error: failed to record Kademlia inbound request");
                        log::debug!("DHT request metrics may be inaccurate");
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Ping(ping::Event { result, .. })) => {
                if metrics_config.enabled {
                    let event_name = match result {
                        Ok(_) => "ping_success",
                        Err(_) => "ping_failure",
                    };
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event(event_name);
                    })) {
                        log::error!("Metrics collection error: failed to record ping {} event", event_name);
                        log::debug!("Ping success/failure metrics may be inaccurate");
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message { .. })) => {
                if metrics_config.enabled {
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event("gossipsub_message");
                    })) {
                        log::error!("Metrics collection error: failed to record GossipSub message event");
                        log::debug!("GossipSub message metrics may be inaccurate");
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { .. })) => {
                if metrics_config.enabled {
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event("gossipsub_subscribed");
                    })) {
                        log::error!("Metrics collection error: failed to record GossipSub subscription event");
                        log::debug!("GossipSub subscription metrics may be inaccurate");
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Autonat(autonat::Event::StatusChanged { old, new })) => {
                log::debug!("NAT status changed from {:?} to {:?}", old, new);
                if metrics_config.enabled {
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event("autonat_status_changed");
                    })) {
                        log::error!("Metrics collection error: failed to record AutoNAT status change from {:?} to {:?}", old, new);
                        log::debug!("AutoNAT status change metrics may be inaccurate");
                    }
                }
            }
            _ => {
                // Record other events generically
                if metrics_config.enabled {
                    if let Err(_) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        metrics::record_network_event("other_event");
                    })) {
                        log::debug!("Metrics collection error: failed to record generic network event");
                    }
                }
            }
        }
    }

    log::info!("Bootnode stopped");
    Ok(())
}
