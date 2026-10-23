//! Root binary build script: macOS SDK linker flag only. The engine crate
//! owns the SYMSLICE_BUILD_* compile-time env vars, and the extractors crate
//! owns its own query pack generator.

#[cfg(target_os = "macos")]
fn macos_link_sdk() {
    if std::env::var_os("SDKROOT").is_some() {
        return;
    }
    let output = match std::process::Command::new("xcrun")
        .args(["--show-sdk-path"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return,
    };
    let sdk_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sdk_root.is_empty() {
        return;
    }
    println!("cargo:rustc-link-arg=-isysroot");
    println!("cargo:rustc-link-arg={sdk_root}");
}

#[cfg(target_os = "macos")]
fn main() {
    macos_link_sdk();
}

#[cfg(not(target_os = "macos"))]
fn main() {}
