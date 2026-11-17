use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
fn files(path: &Path, result: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        result.push(path.to_owned());
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if ["target", ".git", "node_modules", ".rgctl"]
            .iter()
            .any(|name| entry.file_name() == *name)
        {
            continue;
        }
        if entry.file_type()?.is_dir() {
            files(&entry.path(), result)?;
        } else if entry.file_type()?.is_file() {
            result.push(entry.path());
        }
    }
    Ok(())
}
fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rgctl-source");
    let revision = Command::new("git")
        .args(["-C"])
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .or_else(|| std::env::var("RGCTL_REVISION").ok())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=RGCTL_REVISION={revision}");
    // Include working-tree contents, not just Git HEAD: the sibling checkout
    // may contain pre-existing changes. The same inputs are copied to Podman.
    let mut inputs = Vec::new();
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "languages.toml",
        "src",
        "crates",
        "rgctl-macros",
    ] {
        let path = root.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        files(&path, &mut inputs).expect("Read rgctl build provenance inputs");
    }
    inputs.sort();
    let mut digest = Sha256::new();
    for path in inputs {
        digest.update(
            path.strip_prefix(&root)
                .expect("Relative rgctl source")
                .to_string_lossy()
                .as_bytes(),
        );
        digest.update([0]);
        digest.update(std::fs::read(path).expect("Read rgctl source"));
    }
    println!(
        "cargo:rustc-env=RGCTL_SOURCE_SHA256={}",
        hex::encode(digest.finalize())
    );
    println!("cargo:rerun-if-changed={}/.git/HEAD", root.display());
    println!("cargo:rerun-if-env-changed=RGCTL_REVISION");
}
