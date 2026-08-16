//! Derives the app version baked into the binary as `OIS_VERSION`.
//!
//! Priority: an explicit `OIS_VERSION` in the environment (CI passes `1.0.1-<sha>` — see
//! deploy/backend.Dockerfile) wins and is read directly by `env!`. Otherwise we build it from the
//! repo-root `VERSION` file (the single programmer-controlled base) plus the current short commit,
//! matching how the web and docs images derive it locally. `.git` is absent in the image build, but
//! there CI always supplies `OIS_VERSION`, so the git fallback only runs for local `cargo` builds.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=OIS_VERSION");

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let version_file = Path::new(&manifest).join("..").join("VERSION");
    println!("cargo:rerun-if-changed={}", version_file.display());

    // A non-empty OIS_VERSION from the environment is authoritative — `env!` reads it directly.
    if std::env::var("OIS_VERSION").ok().filter(|s| !s.trim().is_empty()).is_some() {
        return;
    }

    let base = std::fs::read_to_string(&version_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_default());

    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let full = match sha {
        Some(s) => format!("{base}-{s}"),
        None => base,
    };
    println!("cargo:rustc-env=OIS_VERSION={full}");
}
