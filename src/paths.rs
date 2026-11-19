//! Where Deskflip keeps its files.
//!
//! Settings the user would want to carry between machines live under `%APPDATA%` (roaming);
//! machine-specific state and logs live under `%LOCALAPPDATA%`.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const APP_DIR: &str = "Deskflip";

pub fn config_file() -> PathBuf {
    config_file_in(&env_dir("APPDATA"))
}

pub fn state_file() -> PathBuf {
    state_file_in(&env_dir("LOCALAPPDATA"))
}

pub fn log_file() -> PathBuf {
    log_file_in(&env_dir("LOCALAPPDATA"))
}

pub fn config_file_in(roaming: &Path) -> PathBuf {
    roaming.join(APP_DIR).join("config.toml")
}

pub fn state_file_in(local: &Path) -> PathBuf {
    local.join(APP_DIR).join("state.toml")
}

pub fn log_file_in(local: &Path) -> PathBuf {
    local.join(APP_DIR).join("deskflip.log")
}

/// Writes a file so that it is always whole: the text goes to a temporary file beside it,
/// which then takes its place. Another process reading meanwhile sees the old file or the new
/// one, never half of either.
pub fn replace_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, path)
}

/// Windows always sets these variables for an interactive user. If one is missing we fall back
/// to the current directory rather than failing: a misplaced file is easier to diagnose than an
/// app that refuses to start.
fn env_dir(var: &str) -> PathBuf {
    env::var_os(var).map_or_else(|| PathBuf::from("."), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_under_roaming() {
        assert_eq!(
            config_file_in(Path::new(r"C:\R")),
            PathBuf::from(r"C:\R\Deskflip\config.toml")
        );
    }

    #[test]
    fn state_and_log_are_under_local() {
        assert_eq!(
            state_file_in(Path::new(r"C:\L")),
            PathBuf::from(r"C:\L\Deskflip\state.toml")
        );
        assert_eq!(
            log_file_in(Path::new(r"C:\L")),
            PathBuf::from(r"C:\L\Deskflip\deskflip.log")
        );
    }
}
