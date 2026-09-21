use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|sha| sha.trim().to_owned());
    println!(
        "cargo:rustc-env=KAIRO_BUILD_GIT_SHA={}",
        sha.as_deref().unwrap_or("unknown")
    );
    println!("cargo:rerun-if-changed=../../.git");
}
