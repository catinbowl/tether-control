use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::fs::{self, OpenOptions};
use std::io;
use std::sync::{Arc, Mutex};
use tether_control::log_file_path;
use tether_control::protocol::DEFAULT_PORT;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct DualWriter {
    file: Option<Arc<Mutex<fs::File>>>,
}

impl io::Write for DualWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stdout().write(buf);
        if let Some(ref f_lock) = self.file
            && let Ok(mut f) = f_lock.lock()
        {
            let _ = io::Write::write_all(&mut *f, buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stdout().flush();
        if let Some(ref f_lock) = self.file
            && let Ok(mut f) = f_lock.lock()
        {
            let _ = io::Write::flush(&mut *f);
        }
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for DualWriter {
    type Writer = DualWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "tether-control",
    version,
    about = "Control a Linux PC from a MacBook over LAN using a 1:1 macOS virtual external display"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the macOS Menu Bar Host controller (creates virtual external display & forwards input)
    Host(HostCliArgs),
    /// Run the Linux Client input daemon (detects physical display & injects absolute input via uinput)
    Client(ClientCliArgs),
}

#[derive(Args, Debug, Clone, Default)]
struct HostCliArgs {
    /// Directly connect to a Linux client `<ip[:port]>` instead of waiting for mDNS auto-discovery
    #[arg(short, long)]
    connect: Option<String>,

    /// Swap macOS Command (⌘) and Control (⌃) keys when controlling the Linux PC
    #[arg(long)]
    swap_cmd_ctrl: bool,

    /// Optional shared authentication token matching the Linux client's `--token`
    #[arg(long)]
    token: Option<String>,

    /// Disable mDNS LAN auto-discovery
    #[arg(long)]
    no_mdns: bool,
}

#[derive(Args, Debug, Clone)]
struct ClientCliArgs {
    /// Interface address to bind the TCP server on
    #[arg(short, long, default_value = "0.0.0.0")]
    bind: String,

    /// TCP port to listen on
    #[arg(short, long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Override physical display width in pixels (auto-detected via Wayland / DRM by default)
    #[arg(long)]
    width: Option<u32>,

    /// Override physical display height in pixels (auto-detected via Wayland / DRM by default)
    #[arg(long)]
    height: Option<u32>,

    /// Override physical display refresh rate in Hz (auto-detected via Wayland by default)
    #[arg(long)]
    refresh: Option<u16>,

    /// Override advertised client hostname
    #[arg(long)]
    name: Option<String>,

    /// Optional shared authentication token required from the macOS host
    #[arg(long)]
    token: Option<String>,

    /// Disable mDNS LAN service advertisement
    #[arg(long)]
    no_mdns: bool,
}

impl Default for ClientCliArgs {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0".to_string(),
            port: DEFAULT_PORT,
            width: None,
            height: None,
            refresh: None,
            name: None,
            token: None,
            no_mdns: false,
        }
    }
}

fn init_logging() {
    let log_path = log_file_path();
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok()
        .map(|f| Arc::new(Mutex::new(f)));

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("tether_control=info,info"));

    let writer = DualWriter { file };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_target(false)
        .compact()
        .init();

    tracing::info!("TetherControl initialized. Log file: {}", log_path.display());
}

fn main() -> Result<()> {
    init_logging();
    let cli = Cli::parse();

    let cmd = match cli.command {
        Some(c) => c,
        None => {
            #[cfg(target_os = "macos")]
            {
                Commands::Host(HostCliArgs::default())
            }
            #[cfg(target_os = "linux")]
            {
                Commands::Client(ClientCliArgs::default())
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                bail!("Unsupported OS: tether-control supports macOS (host) and Linux (client)")
            }
        }
    };

    match cmd {
        Commands::Host(args) => {
            tether_control::host::run_host(tether_control::host::HostArgs {
                connect: args.connect,
                swap_cmd_ctrl: args.swap_cmd_ctrl,
                token: args.token,
                no_mdns: args.no_mdns,
            })
        }
        Commands::Client(args) => {
            tether_control::client::run_client(tether_control::client::ClientArgs {
                bind: args.bind,
                port: args.port,
                width: args.width,
                height: args.height,
                refresh: args.refresh,
                name: args.name,
                token: args.token,
                no_mdns: args.no_mdns,
            })
        }
    }
}

