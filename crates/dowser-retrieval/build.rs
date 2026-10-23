//! Build script: point the linker at the macOS SDK when building inside the
//! Nix shell. Outside Nix this is a no-op: `xcrun` is absent and we bail out.
//! Needed because this crate transitively links C++ libraries (`tokenizers` /
//! `onig_sys` via `model2vec-rs`), and without `-isysroot` the Nix linker
//! can't find `libc++`.

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
