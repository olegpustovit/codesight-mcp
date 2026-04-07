#[cfg(target_os = "macos")]
fn main() {
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

#[cfg(not(target_os = "macos"))]
fn main() {}
