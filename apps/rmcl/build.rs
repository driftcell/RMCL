fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icons/rmcl.ico");
    embed_windows_resources();
}

#[cfg(windows)]
fn embed_windows_resources() {
    let icon =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/icons/rmcl.ico");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(
        icon.to_str()
            .expect("Windows icon path must be valid UTF-8"),
    );
    resource.set("ProductName", "RMCL");
    resource.set("FileDescription", "RMCL Minecraft Launcher");
    resource.set("FileVersion", env!("CARGO_PKG_VERSION"));
    resource.set("ProductVersion", env!("CARGO_PKG_VERSION"));
    resource
        .compile()
        .expect("failed to compile Windows resources");
}

#[cfg(not(windows))]
fn embed_windows_resources() {}
