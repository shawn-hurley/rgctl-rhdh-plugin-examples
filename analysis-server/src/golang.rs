//! Go module acquisition; graph comparison and impact tracing stay in analysis.
use crate::{
    analysis::Prepared,
    maven,
    model::{Comparison, Config, Coordinate, Strategy},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct Module {
    path: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    main: bool,
    #[serde(default)]
    indirect: bool,
    #[serde(default)]
    replace: Option<Value>,
}
impl Module {
    fn coordinate(&self) -> Value {
        json!({"modulePath":self.path,"version":self.version,"indirect":self.indirect})
    }
}
fn hash(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}
fn run(config: &Config, cwd: &Path, args: &[String], log: &Path) -> Result<Vec<u8>> {
    let output = fs::File::create(log)?;
    let errors = fs::File::create(log.with_extension("stderr.log"))?;
    let mut command = Command::new("go");
    command
        .args(args)
        .current_dir(cwd)
        .env("GOWORK", "off")
        .env("GOTOOLCHAIN", "local")
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(errors);
    if let Some(proxy) = &config.go_proxy {
        command.env("GOPROXY", proxy);
    }
    let mut child = command
        .spawn()
        .context("Cannot launch Go; install Go to resolve modules")?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "Go command failed; see {} and its stderr log",
                log.display()
            );
            return Ok(fs::read(log)?);
        }
        if started.elapsed() > Duration::from_secs(180) {
            child.kill()?;
            child.wait()?;
            bail!("Go command timed out; see {}", log.display());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn list(config: &Config, app: &Path, log: &Path) -> Result<Vec<Module>> {
    let data = run(
        config,
        app,
        &[
            "list".into(),
            "-mod=mod".into(),
            "-m".into(),
            "-json".into(),
            "all".into(),
        ],
        log,
    )?;
    let mut modules: Vec<Module> = serde_json::Deserializer::from_slice(&data)
        .into_iter::<Module>()
        .collect::<std::result::Result<_, _>>()?;
    // A versioned replacement changes the selected identity; local replacements
    // aren't immutable releases. Reject both instead of comparing the wrong code.
    ensure!(
        modules.iter().all(|m| m.replace.is_none()),
        "Go replace directives are unsupported for release comparisons"
    );
    modules.retain(|m| !m.main);
    modules.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(modules)
}
fn copy_sources(root: &Path, dest: &Path) -> Result<()> {
    fn walk(current: &Path, dest: &Path) -> Result<()> {
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || ["vendor", "testdata", "target"].contains(&name.as_ref()) {
                continue;
            }
            let kind = entry.file_type()?;
            ensure!(!kind.is_symlink(), "Go source symlinks are unsupported");
            if kind.is_dir() {
                if !entry.path().join("go.mod").exists() {
                    walk(&entry.path(), &dest.join(name.as_ref()))?;
                }
            } else if kind.is_file()
                && (name == "go.mod" || (name.ends_with(".go") && !name.ends_with("_test.go")))
            {
                fs::copy(entry.path(), dest.join(name.as_ref()))?;
            }
        }
        Ok(())
    }
    walk(root, dest)
}

pub(crate) fn prepare(
    config: &Config,
    request: &Comparison,
    dir: &Path,
    stage: &dyn Fn(&str) -> Result<()>,
) -> Result<Prepared> {
    let Coordinate::Go(coordinate) = &request.dependency else {
        bail!("Expected Go module coordinates")
    };
    stage("resolving")?;
    let app_config = config
        .applications
        .get(&request.entity_ref)
        .context("Unknown application")?;
    let original = dir.join("application");
    maven::copy_tree(&app_config.path, &original)?;
    ensure!(
        original.join("go.mod").is_file(),
        "Configured Go application must contain go.mod"
    );
    let digest = maven::tree_digest(&original)?;
    let identity = json!({"id":format!("application:{digest}"),"kind":"application","revision":request.application_revision,"sha256":digest});
    let mut resolved = vec![];
    for label in ["baseline", "candidate"] {
        let root = dir.join(label);
        fs::create_dir_all(&root)?;
        let app = root.join("app");
        maven::copy_tree(&original, &app)?;
        if label == "candidate" {
            run(
                config,
                &app,
                &[
                    "mod".into(),
                    "edit".into(),
                    format!(
                        "-require={}@{}",
                        coordinate.module_path, request.target_version
                    ),
                ],
                &root.join("edit.log"),
            )?;
        }
        let modules = list(config, &app, &root.join("resolve.log"))?;
        let selected = modules
            .iter()
            .find(|m| m.path == coordinate.module_path)
            .context("Selected Go dependency absent from effective module graph")?;
        if label == "baseline" {
            ensure!(
                selected.version != request.target_version,
                "Target version is already the effective baseline"
            );
            ensure!(
                !selected.indirect == (request.strategy == Strategy::DirectUpgrade),
                "Strategy must match a direct Go requirement or transitive override"
            );
        } else {
            ensure!(
                selected.version == request.target_version,
                "Requested Go version was not effective in candidate"
            );
        }
        resolved.push((label, root, modules));
    }
    if request.strategy == Strategy::TransitiveOverride {
        for direct in resolved[0].2.iter().filter(|m| !m.indirect) {
            ensure!(
                resolved[1]
                    .2
                    .iter()
                    .any(|m| m.path == direct.path && m.version == direct.version),
                "Go transitive override changed a direct dependency version"
            );
        }
    }
    let keys: BTreeSet<_> = resolved
        .iter()
        .flat_map(|(_, _, m)| m.iter().map(|m| m.path.clone()))
        .collect();
    let deltas: Vec<_> = keys.into_iter().filter_map(|key| {
        let old = resolved[0].2.iter().find(|m| m.path == key); let new = resolved[1].2.iter().find(|m| m.path == key);
        (old.map(|m| &m.version) != new.map(|m| &m.version)).then(|| json!({"coordinate":key,"baseline":old.map(Module::coordinate),"candidate":new.map(Module::coordinate)}))
    }).collect();
    stage("acquiring-sources")?;
    let mut inputs = vec![];
    let mut sources = vec![];
    for (label, root, modules) in &resolved {
        let input = root.join("input");
        copy_sources(&root.join("app"), &input.join("app"))?;
        let mut owners = BTreeMap::from([("app".into(), identity.clone())]);
        for module in modules {
            let data = run(
                config,
                &root.join("app"),
                &[
                    "mod".into(),
                    "download".into(),
                    "-json".into(),
                    format!("{}@{}", module.path, module.version),
                ],
                &root.join(format!("download-{}.json", hash(&module.path))),
            )?;
            let downloaded: Value = serde_json::from_slice(&data)?;
            ensure!(
                downloaded["Error"].is_null(),
                "Go source download failed: {}",
                downloaded["Error"]
            );
            ensure!(
                downloaded["Path"] == module.path && downloaded["Version"] == module.version,
                "Downloaded Go module identity mismatch"
            );
            let path = PathBuf::from(
                downloaded["Dir"]
                    .as_str()
                    .context("Downloaded module directory missing")?,
            );
            let archive = PathBuf::from(
                downloaded["Zip"]
                    .as_str()
                    .context("Downloaded module archive missing")?,
            );
            let prefix = format!("dependencies/{}", hash(&module.path));
            let destination = input.join(&prefix);
            copy_sources(&path, &destination)?;
            let source_digest = maven::tree_digest(&destination)?;
            let owner = json!({"id":format!("gomod:{}@{}:{}",module.path,module.version,source_digest),"kind":"dependency","artifact":module.coordinate(),"sha256":source_digest});
            owners.insert(prefix, owner);
            sources.push(json!({"snapshot":label,"artifact":module.coordinate(),"status":"available","sourceSha256":source_digest,"archiveSha256":maven::digest(&archive)?,"moduleSum":downloaded["Sum"],"goModSum":downloaded["GoModSum"]}));
        }
        inputs.push((input, *label, owners));
    }
    if request.strategy == Strategy::TransitiveOverride {
        for direct in resolved[0].2.iter().filter(|m| !m.indirect) {
            let checksums: Vec<_> = sources
                .iter()
                .filter(|s| s["artifact"]["modulePath"] == direct.path)
                .map(|s| s["archiveSha256"].clone())
                .collect();
            ensure!(
                checksums.len() == 2 && checksums[0] == checksums[1],
                "Unchanged direct Go dependency archive could not be verified"
            );
        }
    }
    Ok(Prepared {
        snapshots_input: inputs,
        sources,
        diagnostics: vec![],
        application_identity: identity,
        resolution: json!({"requested":request,"baseline":resolved[0].2.iter().map(Module::coordinate).collect::<Vec<_>>(),"candidate":resolved[1].2.iter().map(Module::coordinate).collect::<Vec<_>>(),"deltas":deltas}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_inputs_exclude_test_files_vendor_and_nested_modules() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("module");
        for dir in ["pkg", "vendor/lib", "nested", "testdata", ".git"] {
            fs::create_dir_all(root.join(dir))?;
        }
        for (name, content) in [
            ("go.mod", "module example.test/lib\n"),
            ("lib.go", "package lib\n"),
            ("lib_test.go", "package lib\n"),
            ("pkg/value.go", "package pkg\n"),
            ("nested/go.mod", "module example.test/nested\n"),
            ("nested/hidden.go", "package nested\n"),
            ("vendor/lib/hidden.go", "package lib\n"),
            ("testdata/hidden.go", "invalid go fixture"),
            (".git/hidden.go", "invalid go fixture"),
        ] {
            fs::write(root.join(name), content)?;
        }
        let dest = temp.path().join("input");
        copy_sources(&root, &dest)?;
        assert!(
            dest.join("go.mod").exists()
                && dest.join("lib.go").exists()
                && dest.join("pkg/value.go").exists()
        );
        for name in ["lib_test.go", "vendor", "nested", "testdata", ".git"] {
            assert!(!dest.join(name).exists(), "{name}");
        }
        Ok(())
    }
}
