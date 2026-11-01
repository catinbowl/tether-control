use anyhow::{Context, Result, bail};
use std::fs;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalDisplayInfo {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u16,
}

#[derive(Default)]
struct OutputEntry {
    current_mode: Option<(u32, u32, u16)>,
    preferred_mode: Option<(u32, u32, u16)>,
}

#[derive(Default)]
struct WaylandOutputProbe {
    outputs: Vec<(u32, OutputEntry)>,
}

impl Dispatch<WlRegistry, ()> for WaylandOutputProbe {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "wl_output"
        {
            state.outputs.push((name, OutputEntry::default()));
            let _output = registry.bind::<WlOutput, _, _>(name, version.min(3), qh, name);
        }
    }
}

impl Dispatch<WlOutput, u32> for WaylandOutputProbe {
    fn event(
        state: &mut Self,
        _proxy: &WlOutput,
        event: wl_output::Event,
        output_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode {
            flags: WEnum::Value(mode_flags),
            width,
            height,
            refresh,
        } = event
        {
            if width <= 0 || height <= 0 {
                return;
            }
            let w = width as u32;
            let h = height as u32;
            let hz = if refresh > 0 {
                ((refresh as f64) / 1000.0).round().clamp(24.0, 500.0) as u16
            } else {
                60
            };

            if let Some((_, entry)) = state.outputs.iter_mut().find(|(id, _)| id == output_name) {
                if mode_flags.contains(wl_output::Mode::Current) {
                    entry.current_mode = Some((w, h, hz));
                }
                if mode_flags.contains(wl_output::Mode::Preferred) {
                    entry.preferred_mode = Some((w, h, hz));
                }
            }
        }
    }
}

fn probe_wayland_connection(conn: Connection) -> Result<PhysicalDisplayInfo> {
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    let _registry = display.get_registry(&qh, ());

    let mut state = WaylandOutputProbe::default();
    // First roundtrip discovers wl_output globals; second roundtrip receives wl_output::Mode events.
    event_queue.roundtrip(&mut state)?;
    event_queue.roundtrip(&mut state)?;

    for (_, entry) in &state.outputs {
        if let Some((width, height, refresh_hz)) = entry.current_mode.or(entry.preferred_mode) {
            return Ok(PhysicalDisplayInfo {
                width,
                height,
                refresh_hz,
            });
        }
    }

    bail!("Wayland compositor returned no active wl_output modes")
}

/// Discovers candidate Wayland sockets even when invoked via `sudo` (which strips `XDG_RUNTIME_DIR`).
fn find_wayland_sockets() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(sudo_uid) = std::env::var("SUDO_UID") {
        let dir = PathBuf::from(format!("/run/user/{}", sudo_uid));
        collect_wayland_sockets_in_dir(&dir, &mut candidates);
    }

    if let Ok(entries) = fs::read_dir("/run/user") {
        for entry in entries.flatten() {
            collect_wayland_sockets_in_dir(&entry.path(), &mut candidates);
        }
    }

    candidates
}

fn collect_wayland_sockets_in_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("wayland-") && !name_str.ends_with(".lock") {
                let path = entry.path();
                if !out.contains(&path) {
                    out.push(path);
                }
            }
        }
    }
}

fn detect_via_wayland() -> Result<PhysicalDisplayInfo> {
    if let Ok(conn) = Connection::connect_to_env()
        && let Ok(info) = probe_wayland_connection(conn)
    {
        info!(
            "Detected Linux physical display via Wayland env: {}x{} @ {}Hz",
            info.width, info.height, info.refresh_hz
        );
        return Ok(info);
    }

    for sock_path in find_wayland_sockets() {
        debug!("Trying Wayland socket {:?}", sock_path);
        if let Ok(stream) = UnixStream::connect(&sock_path)
            && let Ok(conn) = Connection::from_socket(stream)
            && let Ok(info) = probe_wayland_connection(conn)
        {
            info!(
                "Detected Linux physical display via Wayland socket {:?}: {}x{} @ {}Hz",
                sock_path, info.width, info.height, info.refresh_hz
            );
            return Ok(info);
        }
    }

    bail!("Could not query display resolution from any Wayland socket")
}

/// Fallback: reads active connector resolution directly from Linux kernel DRM sysfs (`/sys/class/drm/card*-*/modes`).
fn detect_via_drm_sysfs() -> Result<PhysicalDisplayInfo> {
    let drm_dir = Path::new("/sys/class/drm");
    let entries = fs::read_dir(drm_dir).context("Failed to read /sys/class/drm")?;

    let mut connectors: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| {
                    let s = n.to_string_lossy();
                    s.starts_with("card") && s.contains('-')
                })
                .unwrap_or(false)
        })
        .collect();
    connectors.sort();

    for conn in connectors {
        let status = fs::read_to_string(conn.join("status")).unwrap_or_default();
        if status.trim() != "connected" {
            continue;
        }
        let modes = fs::read_to_string(conn.join("modes")).unwrap_or_default();
        if let Some(first_mode) = modes.lines().next()
            && let Some((w, h)) = parse_mode_dimensions(first_mode)
        {
            info!(
                "Detected Linux physical display via DRM sysfs ({:?}): {}x{} @ 60Hz",
                conn, w, h
            );
            return Ok(PhysicalDisplayInfo {
                width: w,
                height: h,
                refresh_hz: 60,
            });
        }
    }

    bail!("No connected display found in /sys/class/drm")
}

fn parse_mode_dimensions(mode_str: &str) -> Option<(u32, u32)> {
    let trimmed = mode_str.trim().trim_end_matches(['i', 'p']);
    let (w_str, h_str) = trimmed.split_once('x')?;
    let w: u32 = w_str.parse().ok()?;
    let h_digits: String = h_str.chars().take_while(|c| c.is_ascii_digit()).collect();
    let h: u32 = h_digits.parse().ok()?;
    if w > 0 && h > 0 { Some((w, h)) } else { None }
}

/// Detects the Linux client's physical display resolution, honoring explicit CLI overrides if provided.
pub fn detect_physical_display(
    override_width: Option<u32>,
    override_height: Option<u32>,
    override_refresh: Option<u16>,
) -> Result<PhysicalDisplayInfo> {
    if let (Some(w), Some(h)) = (override_width, override_height) {
        if w == 0 || h == 0 {
            bail!("Display width and height must be non-zero");
        }
        let refresh_hz = override_refresh.unwrap_or(60).max(30);
        info!(
            "Using CLI-configured client display resolution: {}x{} @ {}Hz",
            w, h, refresh_hz
        );
        return Ok(PhysicalDisplayInfo {
            width: w,
            height: h,
            refresh_hz,
        });
    }

    let mut detected = detect_via_wayland()
        .or_else(|e| {
            debug!("Wayland display probe fell back ({:#}); trying DRM sysfs", e);
            detect_via_drm_sysfs()
        })
        .context(
            "Unable to auto-detect physical display resolution via Wayland or /sys/class/drm. \
             Pass --width <W> --height <H> explicitly.",
        )?;

    if let Some(w) = override_width {
        detected.width = w;
    }
    if let Some(h) = override_height {
        detected.height = h;
    }
    if let Some(hz) = override_refresh {
        detected.refresh_hz = hz.max(30);
    }

    Ok(detected)
}

