use super::model::*;
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}
pub fn key(entity: &str) -> String {
    hash(entity.as_bytes())
}
pub fn write(path: &Path, data: &impl Serialize) -> Result<()> {
    fs::create_dir_all(path.parent().context("Missing storage parent")?)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(data)?)?;
    fs::rename(temp, path)?;
    Ok(())
}
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub fn job_dir(config: &Config, id: &str) -> Result<PathBuf> {
    ensure!(uuid::Uuid::parse_str(id).is_ok(), "Invalid job identity");
    Ok(config.data_dir.join("versions").join(id))
}
pub fn current(config: &Config, application_key: &str) -> Result<Manifest> {
    ensure!(
        application_key.len() == 64 && application_key.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid application key"
    );
    read(
        &config
            .data_dir
            .join("applications")
            .join(format!("{application_key}.json")),
    )
}
pub fn version(config: &Config, application_key: &str, version: &str) -> Result<Manifest> {
    let manifest: Manifest = read(&job_dir(config, version)?.join("manifest.json"))?;
    ensure!(
        manifest.application_key == application_key,
        "Graph version belongs to another application"
    );
    Ok(manifest)
}
pub fn list(config: &Config) -> Result<Vec<Manifest>> {
    let dir = config.data_dir.join("applications");
    fs::create_dir_all(&dir)?;
    let mut result = vec![];
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if p.extension().is_some_and(|x| x == "json") {
            result.push(read(&p)?);
        }
    }
    result.sort_by(|a: &Manifest, b| a.entity_ref.cmp(&b.entity_ref));
    Ok(result)
}

pub fn validate(config: &Config, request: &Onboard) -> Result<()> {
    ensure!(
        entity_ref(&request.entity_ref, "component"),
        "Expected a canonical Component reference"
    );
    ensure!(
        request.provides_apis.len() <= 100 && request.consumes_apis.len() <= 100,
        "Too many API references"
    );
    ensure!(
        request
            .provides_apis
            .iter()
            .chain(&request.consumes_apis)
            .all(|r| entity_ref(r, "api")),
        "Expected canonical API references"
    );
    let url = url::Url::parse(&request.source.repository_url)?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.port().is_none(),
        "Expected an HTTPS repository URL without credentials or options"
    );
    ensure!(
        config
            .allowed_repository_hosts
            .iter()
            .any(|h| Some(h.as_str()) == url.host_str()),
        "Repository host is not allowed"
    );
    let revision = &request.source.revision;
    ensure!(
        !revision.is_empty()
            && revision.len() <= 200
            && !revision.starts_with('-')
            && !revision.contains("..")
            && revision
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._/-".contains(&c)),
        "Invalid Git revision"
    );
    let path = Path::new(&request.source.sub_path);
    ensure!(
        !path.is_absolute()
            && path.components().all(|p| matches!(
                p,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )),
        "Subdirectory must stay inside the checkout"
    );
    Ok(())
}

pub fn git(args: &[&str], directory: &Path, log: &Path) -> Result<()> {
    let file = fs::File::create(log)?;
    let mut child = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.file.allow=never",
            "-c",
            "protocol.ext.allow=never",
        ])
        .args(args)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::from(file.try_clone()?))
        .stderr(Stdio::from(file))
        .spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "Git operation failed; see service logs");
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(180) {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("Git operation timed out")
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn checkout(request: &Onboard, dir: &Path) -> Result<(PathBuf, String)> {
    git(
        &[
            "clone",
            "--depth",
            "1",
            "--no-checkout",
            "--",
            &request.source.repository_url,
            "repository",
        ],
        dir,
        &dir.join("clone.log"),
    )?;
    let repository = dir.join("repository");
    let target = if request.source.revision == "HEAD" {
        "HEAD"
    } else {
        git(
            &["fetch", "--depth", "1", "origin", &request.source.revision],
            &repository,
            &dir.join("fetch.log"),
        )?;
        "FETCH_HEAD"
    };
    git(
        &["checkout", "--detach", target, "--"],
        &repository,
        &dir.join("checkout.log"),
    )?;
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repository)
        .output()?;
    ensure!(commit.status.success(), "Cannot read checked-out revision");
    let commit = String::from_utf8(commit.stdout)?.trim().to_owned();
    let root = repository.canonicalize()?;
    let selected = root.join(&request.source.sub_path).canonicalize()?;
    ensure!(
        selected.starts_with(&root) && selected.is_dir(),
        "Source directory is outside the repository"
    );
    Ok((selected, commit))
}

// Retain an immutable Java input tree. Never follow symlinks into external files,
// run a build, or execute repository hooks. Tests and build outputs are excluded.
pub fn copy_java(source: &Path, target: &Path) -> Result<Vec<PathBuf>> {
    fn visit(root: &Path, path: &Path, target: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let relative = entry.path().strip_prefix(root)?.to_owned();
            let name = entry.file_name();
            let ty = entry.file_type()?;
            if ty.is_symlink() {
                continue;
            }
            if ty.is_dir() {
                let normalized = relative.to_string_lossy().replace('\\', "/");
                if [".git", ".rgctl", "target", "build", "node_modules"]
                    .iter()
                    .any(|x| name == *x)
                    || normalized.ends_with("src/test")
                    || normalized.ends_with("src/testFixtures")
                {
                    continue;
                }
                visit(root, &entry.path(), target, files)?;
            } else if ty.is_file() && relative.extension().is_some_and(|x| x == "java") {
                let destination = target.join(&relative);
                fs::create_dir_all(destination.parent().context("Source parent")?)?;
                fs::copy(entry.path(), &destination)?;
                files.push(relative);
                ensure!(files.len() <= 100_000, "Java source limit exceeded");
            }
        }
        Ok(())
    }
    fs::create_dir_all(target)?;
    let mut files = vec![];
    visit(source, source, target, &mut files)?;
    files.sort();
    ensure!(!files.is_empty(), "No production Java sources found");
    Ok(files)
}
