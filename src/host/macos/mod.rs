pub mod event_tap;
pub mod ffi;
pub mod permissions;
pub mod tray;
pub mod virtual_display;

use crate::discovery::{DiscoveredClient, DiscoveryEvent, spawn_client_browser};
use crate::host::HostArgs;
use crate::host::macos::event_tap::{SharedTapState, spawn_event_tap_thread};
use crate::host::macos::tray::{NetCommand, UiEvent, run_menubar_app};
use crate::protocol::{
    ClientHello, ClientMessage, DEFAULT_PORT, HostMessage, ServerHello, compute_auth_proof,
    read_auth_ack,
};
use anyhow::{Context, Result, bail};
use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
enum TargetMode {
    Auto,
    PinnedClient(DiscoveredClient),
    PinnedAddr(String),
    Paused,
}

enum SessionExit {
    NetworkLost(String),
    AuthFailed(String),
    UserCommand(NetCommand),
}

fn read_local_hostname() -> String {
    let mut buf = [0u8; 128];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc == 0 {
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        if let Ok(s) = std::str::from_utf8(&buf[..len])
            && !s.trim().is_empty()
        {
            return s.trim().trim_end_matches(".local").to_string();
        }
    }
    "MacBook".to_string()
}

fn resolve_target_addrs(target: &str) -> Result<Vec<SocketAddr>> {
    let trimmed = target.trim();
    let with_port = if trimmed.contains(':') {
        trimmed.to_string()
    } else {
        format!("{}:{}", trimmed, DEFAULT_PORT)
    };
    let mut addrs: Vec<SocketAddr> = with_port
        .to_socket_addrs()
        .with_context(|| format!("Could not resolve '{}'", with_port))?
        .collect();
    addrs.sort_by_key(|a| if a.is_ipv4() { 0 } else { 1 });
    if addrs.is_empty() {
        bail!("No addresses resolved for '{}'", with_port);
    }
    Ok(addrs)
}

fn drain_discovery(
    disc_rx: &Receiver<DiscoveryEvent>,
    discovered: &mut Vec<DiscoveredClient>,
    ui_tx: &Sender<UiEvent>,
) {
    let mut changed = false;
    while let Ok(ev) = disc_rx.try_recv() {
        changed = true;
        match ev {
            DiscoveryEvent::Upserted(client) => {
                if let Some(existing) = discovered
                    .iter_mut()
                    .find(|c| c.fullname == client.fullname || (c.client_id.is_some() && c.client_id == client.client_id))
                {
                    *existing = client;
                } else {
                    discovered.push(client);
                }
            }
            DiscoveryEvent::Removed { fullname } => {
                discovered.retain(|c| c.fullname != fullname);
            }
        }
    }
    if changed {
        let _ = ui_tx.send(UiEvent::DiscoveredListChanged(discovered.clone()));
    }
}

#[allow(clippy::too_many_arguments)]
fn run_active_session(
    addrs: &[SocketAddr],
    host_name: &str,
    token: Option<&str>,
    tap_state: &Arc<SharedTapState>,
    ui_tx: &Sender<UiEvent>,
    net_rx: &Receiver<NetCommand>,
    disc_rx: &Receiver<DiscoveryEvent>,
    discovered: &mut Vec<DiscoveredClient>,
) -> SessionExit {
    let mut connected_stream = None;
    let mut last_err = String::from("No address reachable");

    for addr in addrs {
        let _ = ui_tx.send(UiEvent::Connecting {
            target: addr.to_string(),
        });
        info!("Attempting TCP connection to {}…", addr);
        match TcpStream::connect_timeout(addr, Duration::from_secs(3)) {
            Ok(s) => {
                info!("TCP connection established with {}", addr);
                connected_stream = Some((s, *addr));
                break;
            }
            Err(e) => {
                warn!("TCP connect to {} failed: {}", addr, e);
                last_err = format!("{}: {}", addr, e);
            }
        }
    }

    let Some((mut stream, peer_addr)) = connected_stream else {
        return SessionExit::NetworkLost(last_err);
    };

    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let server_hello = match ServerHello::read_from(&mut stream) {
        Ok(sh) => sh,
        Err(e) => return SessionExit::NetworkLost(format!("Handshake read error: {:#}", e)),
    };

    let auth_proof = compute_auth_proof(&server_hello.nonce, token);
    let client_hello = ClientHello {
        host_name: host_name.to_string(),
        auth_proof,
    };
    if let Err(e) = client_hello.write_to(&mut stream) {
        return SessionExit::NetworkLost(format!("Handshake write error: {:#}", e));
    }

    match read_auth_ack(&mut stream) {
        Ok(true) => {}
        Ok(false) => {
            return SessionExit::AuthFailed(format!(
                "Authentication rejected by '{}' (pass matching --token)",
                server_hello.hostname
            ));
        }
        Err(e) => return SessionExit::NetworkLost(format!("Auth ack error: {:#}", e)),
    }

    let _ = stream.set_read_timeout(None);

    info!(
        "Connected to Linux client '{}' ({}) — display {}x{} @ {}Hz (id={:08x})",
        server_hello.hostname,
        peer_addr,
        server_hello.display_width,
        server_hello.display_height,
        server_hello.refresh_hz,
        server_hello.client_id
    );

    // Clear stale input events before activating virtual display
    tap_state.outbound.clear();

    let addr_str = peer_addr.to_string();
    let _ = ui_tx.send(UiEvent::Connected {
        hostname: server_hello.hostname.clone(),
        addr: addr_str.clone(),
        width: server_hello.display_width,
        height: server_hello.display_height,
        refresh_hz: server_hello.refresh_hz,
        client_id: server_hello.client_id,
    });

    let reader_alive = Arc::new(AtomicBool::new(true));
    let start_instant = Instant::now();
    let last_pong_ms = Arc::new(AtomicU64::new(0));

    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => return SessionExit::NetworkLost(format!("Stream clone error: {}", e)),
    };

    {
        let reader_alive = Arc::clone(&reader_alive);
        let last_pong_ms = Arc::clone(&last_pong_ms);
        let ui_tx = ui_tx.clone();
        let hostname = server_hello.hostname.clone();
        let addr_clone = addr_str.clone();
        let client_id = server_hello.client_id;

        thread::Builder::new()
            .name("tether-host-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(read_stream);
                while let Ok(msg) = ClientMessage::read_from(&mut reader) {
                    match msg {
                        ClientMessage::Pong { seq } => {
                            let elapsed = start_instant.elapsed().as_millis() as u64;
                            last_pong_ms.store(elapsed, Ordering::Release);
                            debug!("Received Pong seq={}", seq);
                        }
                        ClientMessage::DisplayChanged {
                            width,
                            height,
                            refresh_hz,
                        } => {
                            info!(
                                "Client '{}' changed resolution to {}x{} @ {}Hz",
                                hostname, width, height, refresh_hz
                            );
                            let _ = ui_tx.send(UiEvent::Connected {
                                hostname: hostname.clone(),
                                addr: addr_clone.clone(),
                                width,
                                height,
                                refresh_hz,
                                client_id,
                            });
                        }
                    }
                }
                reader_alive.store(false, Ordering::Release);
            })
            .ok();
    }

    let mut batch = Vec::with_capacity(64);
    let mut wire_buf = Vec::with_capacity(576);
    let mut last_ping = Instant::now();
    let mut ping_seq: u32 = 0;

    loop {
        drain_discovery(disc_rx, discovered, ui_tx);

        if let Ok(cmd) = net_rx.try_recv() {
            let mut end_pkt = [0u8; 9];
            let n = HostMessage::Disconnect.encode(&mut end_pkt);
            let _ = stream.write_all(&end_pkt[..n]);
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return SessionExit::UserCommand(cmd);
        }

        if !reader_alive.load(Ordering::Acquire) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return SessionExit::NetworkLost("Client closed connection".into());
        }

        tap_state
            .outbound
            .drain_wait(&mut batch, Duration::from_millis(200));

        if !batch.is_empty() {
            wire_buf.clear();
            let mut tmp = [0u8; 9];
            for msg in &batch {
                let len = msg.encode(&mut tmp);
                wire_buf.extend_from_slice(&tmp[..len]);
            }
            if let Err(e) = stream.write_all(&wire_buf) {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return SessionExit::NetworkLost(format!("Socket write error: {}", e));
            }
        }

        if last_ping.elapsed() >= Duration::from_secs(2) {
            ping_seq = ping_seq.wrapping_add(1);
            let mut tmp = [0u8; 9];
            let len = HostMessage::Ping { seq: ping_seq }.encode(&mut tmp);
            if let Err(e) = stream.write_all(&tmp[..len]) {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return SessionExit::NetworkLost(format!("Ping write error: {}", e));
            }
            last_ping = Instant::now();

            let now_ms = start_instant.elapsed().as_millis() as u64;
            let pong_ms = last_pong_ms.load(Ordering::Acquire);
            if now_ms.saturating_sub(pong_ms) > 8_500 {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return SessionExit::NetworkLost("Heartbeat timeout from Linux client".into());
            }
        }
    }
}

fn spawn_connection_manager(
    args: HostArgs,
    tap_state: Arc<SharedTapState>,
    ui_tx: Sender<UiEvent>,
    net_rx: Receiver<NetCommand>,
    disc_rx: Receiver<DiscoveryEvent>,
) {
    thread::Builder::new()
        .name("tether-conn-mgr".into())
        .spawn(move || {
            let host_name = read_local_hostname();
            let mut discovered: Vec<DiscoveredClient> = Vec::new();
            let mut mode = match &args.connect {
                Some(target) => TargetMode::PinnedAddr(target.clone()),
                None => TargetMode::Auto,
            };

            loop {
                drain_discovery(&disc_rx, &mut discovered, &ui_tx);

                while let Ok(cmd) = net_rx.try_recv() {
                    match cmd {
                        NetCommand::ConnectManual(target) => {
                            mode = TargetMode::PinnedAddr(target);
                        }
                        NetCommand::ConnectDiscovered(client) => {
                            mode = TargetMode::PinnedClient(client);
                        }
                        NetCommand::EnableAutoConnect => {
                            mode = TargetMode::Auto;
                            let _ = ui_tx.send(UiEvent::Searching);
                        }
                        NetCommand::Disconnect => {
                            mode = TargetMode::Paused;
                        }
                        NetCommand::Shutdown => return,
                    }
                }

                let candidate_addrs: Option<Vec<SocketAddr>> = match &mode {
                    TargetMode::Paused => None,
                    TargetMode::PinnedAddr(target) => match resolve_target_addrs(target) {
                        Ok(addrs) => Some(addrs),
                        Err(e) => {
                            warn!("Target resolution failed: {:#}", e);
                            None
                        }
                    },
                    TargetMode::PinnedClient(client) => {
                        let addrs = if let Some(latest) = discovered.iter().find(|d| {
                            d.fullname == client.fullname
                                || (d.client_id.is_some() && d.client_id == client.client_id)
                        }) {
                            latest.addrs.clone()
                        } else {
                            client.addrs.clone()
                        };
                        Some(addrs)
                    }
                    TargetMode::Auto => discovered.first().map(|c| c.addrs.clone()),
                };

                let Some(addrs) = candidate_addrs else {
                    thread::sleep(Duration::from_millis(250));
                    continue;
                };

                let exit = run_active_session(
                    &addrs,
                    &host_name,
                    args.token.as_deref(),
                    &tap_state,
                    &ui_tx,
                    &net_rx,
                    &disc_rx,
                    &mut discovered,
                );

                match exit {
                    SessionExit::NetworkLost(reason) => {
                        warn!("Connection attempt failed or lost: {}", reason);
                        let _ = ui_tx.send(UiEvent::Disconnected {
                            reason: Some(reason),
                        });
                        thread::sleep(Duration::from_millis(2500));
                    }
                    SessionExit::AuthFailed(reason) => {
                        warn!("{}", reason);
                        mode = TargetMode::Paused;
                        let _ = ui_tx.send(UiEvent::Disconnected {
                            reason: Some(reason),
                        });
                    }
                    SessionExit::UserCommand(cmd) => match cmd {
                        NetCommand::ConnectManual(target) => {
                            let _ = ui_tx.send(UiEvent::Disconnected { reason: None });
                            mode = TargetMode::PinnedAddr(target);
                        }
                        NetCommand::ConnectDiscovered(client) => {
                            let _ = ui_tx.send(UiEvent::Disconnected { reason: None });
                            mode = TargetMode::PinnedClient(client);
                        }
                        NetCommand::EnableAutoConnect => {
                            mode = TargetMode::Auto;
                            let _ = ui_tx.send(UiEvent::Searching);
                        }
                        NetCommand::Disconnect => {
                            mode = TargetMode::Paused;
                            let _ = ui_tx.send(UiEvent::Disconnected {
                                reason: Some("Disconnected by user".into()),
                            });
                        }
                        NetCommand::Shutdown => return,
                    },
                }
            }
        })
        .expect("Failed spawning connection manager thread");
}

pub fn run_host(args: HostArgs) -> Result<()> {
    let tap_state = SharedTapState::new(args.swap_cmd_ctrl);
    spawn_event_tap_thread(Arc::clone(&tap_state));

    let (ui_tx, ui_rx) = mpsc::channel::<UiEvent>();
    let (net_tx, net_rx) = mpsc::channel::<NetCommand>();
    let (disc_tx, disc_rx) = mpsc::channel::<DiscoveryEvent>();

    let _mdns = if !args.no_mdns {
        match spawn_client_browser(disc_tx) {
            Ok(d) => Some(d),
            Err(e) => {
                warn!("mDNS browser failed to start ({:#}); manual connect still available", e);
                None
            }
        }
    } else {
        None
    };

    let initial_target = args.connect.clone();
    spawn_connection_manager(args, Arc::clone(&tap_state), ui_tx, net_rx, disc_rx);

    run_menubar_app(tap_state, ui_rx, net_tx, initial_target)
}

