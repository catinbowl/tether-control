use crate::protocol::MDNS_SERVICE_TYPE;
use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::mpsc::Sender;
use std::thread;
use tracing::{debug, info, warn};

fn get_local_ipv4s() -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut ifap) } == 0 && !ifap.is_null() {
        let mut curr = ifap;
        while !curr.is_null() {
            let ifa = unsafe { &*curr };
            if !ifa.ifa_addr.is_null()
                && unsafe { (*ifa.ifa_addr).sa_family } == libc::AF_INET as libc::sa_family_t
            {
                let sock_in = unsafe { *(ifa.ifa_addr as *const libc::sockaddr_in) };
                let ip = Ipv4Addr::from(u32::from_be(sock_in.sin_addr.s_addr));
                let oct = ip.octets();
                if !ip.is_loopback() && !ip.is_unspecified() && !(oct[0] == 169 && oct[1] == 254) {
                    out.push(ip);
                }
            }
            curr = ifa.ifa_next;
        }
        unsafe { libc::freeifaddrs(ifap) };
    }
    out
}

fn rank_address(ip: &IpAddr, local_ipv4s: &[Ipv4Addr]) -> u32 {
    match ip {
        IpAddr::V4(v4) => {
            let oct = v4.octets();
            // 1. Matches local subnet (/24) - highest priority
            for local in local_ipv4s {
                let l_oct = local.octets();
                if oct[0] == l_oct[0] && oct[1] == l_oct[1] && oct[2] == l_oct[2] {
                    return 0;
                }
            }
            // 2. Standard LAN subnets
            if oct[0] == 192 && oct[1] == 168 {
                1
            } else if oct[0] == 10 {
                2
            } else if oct[0] == 172 && (16..=31).contains(&oct[1]) {
                // Often a Docker or VM bridge (172.17.0.1, etc.)
                5
            } else {
                3
            }
        }
        IpAddr::V6(_) => 4,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredClient {
    pub fullname: String,
    pub hostname: String,
    pub addrs: Vec<SocketAddr>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub refresh_hz: Option<u16>,
    pub client_id: Option<u32>,
}

impl DiscoveredClient {
    /// Returns the preferred LAN address to connect to (sorted by local subnet proximity).
    pub fn preferred_addr(&self) -> Option<SocketAddr> {
        self.addrs.first().copied()
    }

    pub fn display_label(&self) -> String {
        let addr_str = self
            .preferred_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|| "unknown".into());
        match (self.width, self.height) {
            (Some(w), Some(h)) => format!("{} ({}×{} @ {})", self.hostname, w, h, addr_str),
            _ => format!("{} ({})", self.hostname, addr_str),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Upserted(DiscoveredClient),
    Removed { fullname: String },
}

fn sanitize_mdns_label(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches(".local").trim_end_matches('.');
    let cleaned: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-');
    if cleaned.is_empty() {
        "tether-node".to_string()
    } else {
        cleaned.chars().take(48).collect()
    }
}

/// Advertises the Linux client over mDNS (`_tether-ctrl._tcp.local.`) on all LAN interfaces.
/// Keep the returned `ServiceDaemon` alive for the lifetime of the server.
pub fn advertise_client(
    hostname: &str,
    port: u16,
    width: u32,
    height: u32,
    refresh_hz: u16,
    client_id: u32,
) -> Result<ServiceDaemon> {
    let mdns = ServiceDaemon::new().context("Failed to start mDNS daemon")?;
    let label = sanitize_mdns_label(hostname);
    let instance_name = format!("{}-{:08x}", label, client_id);
    let host_fqdn = format!("{}.local.", label);

    let mut properties = HashMap::new();
    properties.insert("hostname".to_string(), hostname.to_string());
    properties.insert("width".to_string(), width.to_string());
    properties.insert("height".to_string(), height.to_string());
    properties.insert("refresh".to_string(), refresh_hz.to_string());
    properties.insert("id".to_string(), client_id.to_string());

    let service_info = ServiceInfo::new(
        MDNS_SERVICE_TYPE,
        &instance_name,
        &host_fqdn,
        "",
        port,
        properties,
    )
    .context("Failed to build mDNS ServiceInfo")?
    .enable_addr_auto();

    mdns.register(service_info)
        .context("Failed to register mDNS service")?;

    info!(
        "mDNS advertising {} on port {} ({}x{}@{}Hz)",
        instance_name, port, width, height, refresh_hz
    );
    Ok(mdns)
}

/// Starts a background mDNS browser searching for `_tether-ctrl._tcp.local.` Linux clients on LAN.
pub fn spawn_client_browser(tx: Sender<DiscoveryEvent>) -> Result<ServiceDaemon> {
    let mdns = ServiceDaemon::new().context("Failed to start mDNS browser daemon")?;
    let receiver = mdns
        .browse(MDNS_SERVICE_TYPE)
        .context("Failed to browse mDNS service type")?;

    thread::Builder::new()
        .name("tether-mdns-browser".into())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let port = info.get_port();
                        let local_ips = get_local_ipv4s();
                        let mut addrs: Vec<SocketAddr> = info
                            .get_addresses()
                            .iter()
                            .filter_map(|&ip| {
                                if ip.is_loopback() || ip.is_unspecified() {
                                    return None;
                                }
                                if let IpAddr::V4(v4) = ip {
                                    let oct = v4.octets();
                                    if oct[0] == 169 && oct[1] == 254 {
                                        return None;
                                    }
                                }
                                if let IpAddr::V6(v6) = ip
                                    && v6.is_unicast_link_local()
                                {
                                    return None;
                                }
                                Some(SocketAddr::new(ip, port))
                            })
                            .collect();

                        // Sort by local subnet proximity so true LAN IPs rank first
                        addrs.sort_by_key(|a| rank_address(&a.ip(), &local_ips));
                        addrs.dedup();

                        if addrs.is_empty() {
                            debug!("Ignoring mDNS service {} with no usable LAN IP", info.get_fullname());
                            continue;
                        }

                        let props = info.get_properties();
                        let hostname = props
                            .get_property_val_str("hostname")
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| {
                                info.get_hostname()
                                    .trim_end_matches(".local.")
                                    .trim_end_matches('.')
                                    .to_string()
                            });
                        let width = props
                            .get_property_val_str("width")
                            .and_then(|v| v.parse::<u32>().ok());
                        let height = props
                            .get_property_val_str("height")
                            .and_then(|v| v.parse::<u32>().ok());
                        let refresh_hz = props
                            .get_property_val_str("refresh")
                            .and_then(|v| v.parse::<u16>().ok());
                        let client_id = props
                            .get_property_val_str("id")
                            .and_then(|v| v.parse::<u32>().ok());

                        let client = DiscoveredClient {
                            fullname: info.get_fullname().to_string(),
                            hostname,
                            addrs,
                            width,
                            height,
                            refresh_hz,
                            client_id,
                        };
                        if tx.send(DiscoveryEvent::Upserted(client)).is_err() {
                            break;
                        }
                    }
                    ServiceEvent::ServiceRemoved(_ty, fullname) => {
                        if tx.send(DiscoveryEvent::Removed { fullname }).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            warn!("mDNS browser loop exited");
        })
        .context("Failed spawning mDNS browser thread")?;

    Ok(mdns)
}
