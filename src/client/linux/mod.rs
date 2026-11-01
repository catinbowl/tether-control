pub mod display;
pub mod uinput;

use crate::client::ClientArgs;
use crate::client::linux::display::{PhysicalDisplayInfo, detect_physical_display};
use crate::client::linux::uinput::VirtualInputHub;
use crate::discovery::advertise_client;
use crate::protocol::{
    ClientHello, ClientMessage, HostMessage, ServerHello, compute_auth_proof, stable_hash_u32,
    write_auth_ack,
};
use anyhow::{Context, Result, bail};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info, warn};

fn read_hostname(override_name: Option<&str>) -> String {
    if let Some(n) = override_name {
        let trimmed = n.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(s) = fs::read_to_string("/etc/hostname") {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let mut buf = [0u8; 128];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc == 0 {
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        if let Ok(s) = std::str::from_utf8(&buf[..len])
            && !s.trim().is_empty()
        {
            return s.trim().to_string();
        }
    }
    "linux-client".to_string()
}

fn compute_client_id(hostname: &str) -> u32 {
    if let Ok(machine_id) = fs::read_to_string("/etc/machine-id") {
        let trimmed = machine_id.trim();
        if !trimmed.is_empty() {
            return stable_hash_u32(trimmed);
        }
    }
    stable_hash_u32(hostname)
}

fn random_nonce() -> [u8; 16] {
    let mut nonce = [0u8; 16];
    if let Ok(mut f) = File::open("/dev/urandom")
        && f.read_exact(&mut nonce).is_ok()
    {
        return nonce;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id() as u128;
    nonce.copy_from_slice(&(nanos ^ (pid << 64)).to_le_bytes());
    nonce
}

fn handle_host_session(
    mut stream: TcpStream,
    peer: SocketAddr,
    hostname: &str,
    client_id: u32,
    disp_info: PhysicalDisplayInfo,
    token: Option<&str>,
    hub: &mut VirtualInputHub,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let nonce = random_nonce();
    let auth_required = token.map(|t| !t.is_empty()).unwrap_or(false);
    let server_hello = ServerHello {
        hostname: hostname.to_string(),
        display_width: disp_info.width,
        display_height: disp_info.height,
        refresh_hz: disp_info.refresh_hz,
        client_id,
        auth_required,
        nonce,
    };
    server_hello
        .write_to(&mut stream)
        .context("Failed sending ServerHello")?;

    let client_hello = ClientHello::read_from(&mut stream).context("Failed reading ClientHello")?;

    if auth_required {
        let expected = compute_auth_proof(&nonce, token);
        if client_hello.auth_proof != expected {
            let _ = write_auth_ack(&mut stream, false);
            bail!(
                "Authentication failed for host '{}' ({}): token mismatch",
                client_hello.host_name,
                peer
            );
        }
    }
    write_auth_ack(&mut stream, true)?;

    // Host sends Ping every 2s; 10s read timeout detects sudden LAN loss cleanly.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    info!(
        "Connected to macOS host '{}' ({}) — mapped 1:1 to {}x{} @ {}Hz",
        client_hello.host_name, peer, disp_info.width, disp_info.height, disp_info.refresh_hz
    );

    let mut write_stream = stream
        .try_clone()
        .context("Failed cloning TCP stream for control replies")?;
    let mut reader = BufReader::with_capacity(4096, stream);

    loop {
        let msg = match HostMessage::read_from(&mut reader) {
            Ok(m) => m,
            Err(e) => {
                hub.release_all();
                return Err(e.context("Host stream closed or timed out"));
            }
        };

        match msg {
            HostMessage::PointerEnter { x, y } => {
                debug!("Host pointer entered virtual display at ({}, {})", x, y);
                hub.move_abs(x, y)?;
            }
            HostMessage::MouseMove { x, y } => {
                hub.move_abs(x, y)?;
            }
            HostMessage::MouseButton { button, pressed } => {
                hub.mouse_button(button, pressed)?;
            }
            HostMessage::Scroll { v_hi_res, h_hi_res } => {
                hub.scroll(v_hi_res, h_hi_res)?;
            }
            HostMessage::Key {
                evdev_code,
                pressed,
            } => {
                hub.key(evdev_code, pressed)?;
            }
            HostMessage::PointerLeave => {
                debug!("Host pointer returned to MacBook");
                hub.release_all();
            }
            HostMessage::ReleaseAll => {
                hub.release_all();
            }
            HostMessage::Ping { seq } => {
                ClientMessage::Pong { seq }.write_to(&mut write_stream)?;
            }
            HostMessage::Disconnect => {
                info!("Host '{}' disconnected gracefully", client_hello.host_name);
                hub.release_all();
                return Ok(());
            }
        }
    }
}

pub fn run_client(args: ClientArgs) -> Result<()> {
    let hostname = read_hostname(args.name.as_deref());
    let client_id = compute_client_id(&hostname);

    let mut current_display =
        detect_physical_display(args.width, args.height, args.refresh)?;
    let mut hub = VirtualInputHub::new(current_display.width, current_display.height)?;

    let listener = if args.bind == "0.0.0.0" {
        // Try dual-stack [::]:port first so both IPv4 and IPv6 work, falling back to 0.0.0.0:port
        TcpListener::bind(format!("[::]:{}", args.port))
            .or_else(|_| TcpListener::bind(format!("0.0.0.0:{}", args.port)))
            .with_context(|| format!("Failed to bind TCP server on port {}", args.port))?
    } else {
        TcpListener::bind(format!("{}:{}", args.bind, args.port))
            .with_context(|| format!("Failed to bind TCP server on {}:{}", args.bind, args.port))?
    };

    let local_addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| format!("{}:{}", args.bind, args.port));

    info!(
        "TetherControl Linux client '{}' (id={:08x}) listening on {}",
        hostname, client_id, local_addr
    );
    info!(
        "Physical display: {}x{} @ {}Hz",
        current_display.width, current_display.height, current_display.refresh_hz
    );

    let _mdns = if !args.no_mdns {
        match advertise_client(
            &hostname,
            args.port,
            current_display.width,
            current_display.height,
            current_display.refresh_hz,
            client_id,
        ) {
            Ok(daemon) => Some(daemon),
            Err(e) => {
                warn!(
                    "mDNS advertising unavailable ({:#}); manual --connect {}:{} still works",
                    e, "<linux-ip>", args.port
                );
                None
            }
        }
    } else {
        None
    };

    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(s) => s,
            Err(e) => {
                error!("TCP accept error: {}", e);
                continue;
            }
        };
        let peer = match stream.peer_addr() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Re-check physical display resolution before handshake in case monitor mode changed.
        if let Ok(latest) = detect_physical_display(args.width, args.height, args.refresh)
            && (latest.width != current_display.width || latest.height != current_display.height)
        {
            info!(
                "Display resolution changed {}x{} -> {}x{}; recreating uinput pointer",
                current_display.width, current_display.height, latest.width, latest.height
            );
            match VirtualInputHub::new(latest.width, latest.height) {
                Ok(new_hub) => {
                    hub = new_hub;
                    current_display = latest;
                }
                Err(e) => warn!("Failed recreating uinput hub for new resolution: {:#}", e),
            }
        }

        if let Err(e) = handle_host_session(
            stream,
            peer,
            &hostname,
            client_id,
            current_display,
            args.token.as_deref(),
            &mut hub,
        ) {
            warn!("Session with {} ended: {:#}", peer, e);
        }
        hub.release_all();
    }

    Ok(())
}
