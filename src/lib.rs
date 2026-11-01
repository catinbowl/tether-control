pub mod coords;
pub mod discovery;
pub mod keymap;
pub mod protocol;

pub mod client;
pub mod host;

use std::path::PathBuf;

/// Standard cross-platform path to the TetherControl application log file.
pub fn log_file_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        #[cfg(target_os = "macos")]
        {
            return PathBuf::from(home).join("Library/Logs/TetherControl/tether-control.log");
        }
        #[cfg(not(target_os = "macos"))]
        {
            return PathBuf::from(home).join(".local/state/tether-control/tether-control.log");
        }
    }
    PathBuf::from("/tmp/tether-control.log")
}
