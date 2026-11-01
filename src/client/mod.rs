#[cfg(target_os = "linux")]
pub mod linux;

#[derive(Debug, Clone)]
pub struct ClientArgs {
    pub bind: String,
    pub port: u16,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub refresh: Option<u16>,
    pub name: Option<String>,
    pub token: Option<String>,
    pub no_mdns: bool,
}

/// Runs the client input daemon on the current platform.
pub fn run_client(args: ClientArgs) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::run_client(args)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        anyhow::bail!(
            "Client mode is currently supported on Linux. macOS and Windows client support will be added in a future update."
        )
    }
}

