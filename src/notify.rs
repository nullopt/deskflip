//! Windows notifications, used to tell the user when a switch did not work.

use std::sync::Once;

use tauri_winrt_notification::Toast;
use windows_registry::CURRENT_USER;

/// The identity Windows files Deskflip's notifications under.
const APP_ID: &str = "Deskflip";

/// Shows a notification. Never fails loudly: a notification that cannot be shown is logged.
pub fn show(title: &str, body: &str) {
    register_app_id();
    if let Err(error) = Toast::new(APP_ID).title(title).text1(body).show() {
        log::warn!("could not show notification \"{title}: {body}\": {error}");
    }
}

/// Windows only shows notifications from an app it has a name for. An installer would register
/// one; a portable exe has to do it itself, once per run.
fn register_app_id() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        let registered = CURRENT_USER
            .create(format!(r"Software\Classes\AppUserModelId\{APP_ID}"))
            .and_then(|key| key.set_string("DisplayName", "Deskflip"));
        if let Err(error) = registered {
            log::warn!("could not register for notifications: {error}");
        }
    });
}
