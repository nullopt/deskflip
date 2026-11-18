//! File logger behind the `log` facade.
//!
//! The log exists so a user can attach it to a bug report. It is a single file that is moved
//! aside to `<name>.old` once it grows past [`MAX_BYTES`], so it never needs cleaning up.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{LevelFilter, Log, Metadata, Record};

const MAX_BYTES: u64 = 1024 * 1024;

/// Starts logging to `path`. Logging is best-effort: if the file cannot be opened the app
/// carries on without a log.
pub fn init(path: &Path) {
    if let Ok(logger) = FileLogger::open(path) {
        // `set_boxed_logger` fails only if a logger is already installed, which is harmless.
        if log::set_boxed_logger(Box::new(logger)).is_ok() {
            log::set_max_level(LevelFilter::Info);
        }
    }
}

struct FileLogger {
    file: Mutex<File>,
}

impl FileLogger {
    fn open(path: &Path) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        if fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
            let mut old = path.as_os_str().to_owned();
            old.push(".old");
            fs::rename(path, old)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    fn write_line(&self, line: &str) {
        // A poisoned lock only means another thread panicked mid-write; the file is still usable.
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        // Nothing useful can be done if the log itself cannot be written.
        let _ = writeln!(file, "{line}");
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            self.write_line(&format!(
                "{} {:5} {}",
                format_timestamp(now),
                record.level(),
                record.args()
            ));
        }
    }

    fn flush(&self) {
        let _ = self.file.lock().unwrap_or_else(|e| e.into_inner()).flush();
    }
}

/// Formats seconds since the Unix epoch as an ISO 8601 UTC timestamp.
fn format_timestamp(unix_seconds: u64) -> String {
    let days = unix_seconds / 86_400;
    let secs = unix_seconds % 86_400;

    // Civil-from-days (Howard Hinnant's algorithm), valid for all dates after 1970.
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("deskflip-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn logger_rotates_when_over_cap() {
        let dir = temp_dir("rotate");
        let path = dir.join("deskflip.log");
        fs::write(&path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();

        let logger = FileLogger::open(&path).unwrap();
        logger.write_line("hello");

        assert!(dir.join("deskflip.log.old").exists());
        assert!(fs::metadata(&path).unwrap().len() < 200);
    }

    #[test]
    fn small_log_is_appended_to() {
        let dir = temp_dir("append");
        let path = dir.join("deskflip.log");
        fs::write(&path, "earlier\n").unwrap();

        FileLogger::open(&path).unwrap().write_line("later");

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("earlier\n"));
        assert!(text.trim_end().ends_with("later"));
        assert!(!dir.join("deskflip.log.old").exists());
    }

    #[test]
    fn missing_directory_is_created() {
        let dir = temp_dir("mkdir");
        let path = dir.join("nested").join("deskflip.log");

        FileLogger::open(&path).unwrap().write_line("x");

        assert!(path.exists());
    }

    #[test]
    fn timestamp_is_iso_utc() {
        assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_timestamp(1_795_046_400), "2026-11-19T00:00:00Z");
        assert_eq!(format_timestamp(951_782_399), "2000-02-28T23:59:59Z");
        assert_eq!(format_timestamp(951_868_800), "2000-03-01T00:00:00Z");
    }
}
