//! Embeds the application icon in the executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut resources = winresource::WindowsResource::new();
        resources.set_icon("assets/icon.ico");
        if let Err(error) = resources.compile() {
            // A build script has no better way to fail than to panic with the reason.
            panic!("could not embed the icon: {error}");
        }
    }
}
