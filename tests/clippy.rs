//! Lint gate: `cargo clippy` (with the extra lints in Cargo.toml) must be clean.

use std::process::Command;

#[test]
fn clippy_is_clean() {
    let root = env!("CARGO_MANIFEST_DIR");
    let out = Command::new(env!("CARGO"))
        .args(["clippy", "--all-targets", "--quiet", "--", "-D", "warnings"])
        .current_dir(root)
        // A separate target dir, so this doesn't wait on the `cargo test` build lock.
        .env("CARGO_TARGET_DIR", format!("{root}/target/clippy-gate"))
        .output()
        .expect("run cargo clippy");
    assert!(out.status.success(), "clippy found problems:\n{}", String::from_utf8_lossy(&out.stderr));
}
