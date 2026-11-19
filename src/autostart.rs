//! Starting Deskflip when the user signs in.
//!
//! The per-user `Run` registry key is the single source of truth: the setting is never copied
//! into the settings file, so the two cannot disagree.

use std::env;
use std::io;
use std::path::Path;

use windows_registry::CURRENT_USER;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Deskflip";

pub fn is_enabled() -> bool {
    CURRENT_USER
        .open(RUN_KEY)
        .and_then(|key| key.get_string(VALUE))
        .is_ok()
}

pub fn set(enabled: bool) -> io::Result<()> {
    let key = CURRENT_USER.create(RUN_KEY)?;
    if enabled {
        key.set_string(VALUE, run_value(&env::current_exe()?))?;
    } else if is_enabled() {
        key.remove_value(VALUE)?;
    }
    Ok(())
}

/// The command Windows runs at sign-in. Quoted, because the path may contain spaces.
fn run_value(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn run_value_quotes_the_path() {
        assert_eq!(
            run_value(Path::new(r"C:\Program Files\Deskflip\deskflip.exe")),
            r#""C:\Program Files\Deskflip\deskflip.exe""#
        );
    }
}
