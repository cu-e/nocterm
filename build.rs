//! Embeds the application icon and version information in Windows executables.

fn main() {
    println!("cargo:rerun-if-changed=assets/icons/nocterm.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/icons/nocterm.ico")
        .set("ProductName", "Nocterm")
        .set("FileDescription", "Nocterm SSH client");
    if let Err(error) = resource.compile() {
        // A missing resource compiler must not block development builds.
        println!("cargo:warning=Windows resources were not embedded: {error}");
    }
}
