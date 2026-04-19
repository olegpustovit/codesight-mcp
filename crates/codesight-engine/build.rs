//! Engine build script: emit CODESIGHT_BUILD_SHA and CODESIGHT_BUILD_TIME env
//! vars for `config::defaults` so every response advertises the running
//! binary's build stamp. On macOS, also inject the Xcode SDK path so test
//! binaries link against the correct `libc++` under Nix shells.

fn emit_build_stamp() {
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=CODESIGHT_BUILD_SHA={sha}");

    let t = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });
    println!("cargo:rustc-env=CODESIGHT_BUILD_TIME={t}");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
}

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

#[cfg(not(target_os = "macos"))]
fn macos_link_sdk() {}

fn main() {
    emit_build_stamp();
    macos_link_sdk();
}
