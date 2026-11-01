#[cfg(target_os = "macos")]
pub mod macos;

#[derive(Debug, Clone, Default)]
pub struct HostArgs {
    pub connect: Option<String>,
    pub swap_cmd_ctrl: bool,
    pub token: Option<String>,
    pub no_mdns: bool,
}

/// Runs the host controller on the current platform.
pub fn run_host(args: HostArgs) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::run_host(args)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = args;
        anyhow::bail!(
            "Host mode is currently supported on macOS. Linux and Windows host support will be added in a future update."
        )
    }
}

