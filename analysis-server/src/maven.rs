use crate::model::{Comparison, Config, Strategy};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use xmltree::{Element, XMLNode};

const PLUGIN: &str = "org.apache.maven.plugins:maven-dependency-plugin:3.10.0";

pub fn digest(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}

pub fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if ["target", ".git", ".rgctl"].iter().any(|s| name == *s) {
            continue;
        }
        let meta = entry.file_type()?;
        ensure!(
            !meta.is_symlink(),
            "Symlink inputs are unsupported: {}",
            entry.path().display()
        );
        if meta.is_dir() {
            copy_tree(&entry.path(), &dst.join(name))?;
        } else if meta.is_file() {
            fs::copy(entry.path(), dst.join(name))?;
        }
    }
    Ok(())
}

pub fn tree_digest(root: &Path) -> Result<String> {
    fn walk(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let e = entry?;
            if e.file_type()?.is_dir() {
                walk(root, &e.path(), files)?;
            } else {
                files.push(e.path().strip_prefix(root)?.to_owned());
            }
        }
        Ok(())
    }
    let mut files = vec![];
    walk(root, root, &mut files)?;
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        hash.update(file.to_string_lossy().as_bytes());
        hash.update([0]);
        hash.update(digest(&root.join(file))?);
    }
    Ok(hex::encode(hash.finalize()))
}

pub fn run_maven(config: &Config, cwd: &Path, args: &[String], log: &Path) -> Result<()> {
    let mut command = Command::new("mvn");
    command.args(["-B", "-ntp", "-DskipTests", "-Dstyle.color=never"]);
    if let Some(repo) = &config.maven_repository {
        command.arg(format!("-Dmaven.repo.local={}", repo.display()));
    }
    let out = fs::File::create(log)?;
    let mut child = command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out)
        .spawn()
        .context("Cannot launch Maven")?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "Maven failed; see {}", log.display());
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(180) {
            child.kill()?;
            child.wait()?;
            bail!("Maven timed out; see {}", log.display());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn text(e: &Element, name: &str) -> String {
    e.get_child(name)
        .and_then(Element::get_text)
        .map(|s| s.into_owned())
        .unwrap_or_default()
}
fn child<'a>(e: &'a mut Element, name: &str) -> &'a mut Element {
    let index = e
        .children
        .iter()
        .position(|n| matches!(n, XMLNode::Element(c) if c.name == name));
    let index = index.unwrap_or_else(|| {
        let mut element = Element::new(name);
        element.namespace = e.namespace.clone();
        e.children.push(XMLNode::Element(element));
        e.children.len() - 1
    });
    match &mut e.children[index] {
        XMLNode::Element(c) => c,
        _ => unreachable!(),
    }
}
fn set_text(e: &mut Element, name: &str, value: &str) {
    child(e, name).children = vec![XMLNode::Text(value.into())];
}

pub fn change_pom(path: &Path, request: &Comparison) -> Result<()> {
    let mut pom = Element::parse(fs::File::open(path)?)?;
    // xmltree stores attribute local names, so do not emit xsi:schemaLocation
    // as an unqualified Maven model attribute after serializing the POM.
    pom.attributes.remove("schemaLocation");
    // The initial configured application scope is one Maven module. Reject a
    // reactor rather than silently analyze only part of an application.
    ensure!(
        pom.get_child("modules").is_none(),
        "Configure a single Maven module; reactor selection is not supported yet"
    );
    let container = if request.strategy == Strategy::TransitiveOverride {
        child(child(&mut pom, "dependencyManagement"), "dependencies")
    } else {
        child(&mut pom, "dependencies")
    };
    let mut matches = 0;
    for node in &mut container.children {
        if let XMLNode::Element(dep) = node {
            if dep.name == "dependency"
                && text(dep, "groupId") == request.dependency.group_id
                && text(dep, "artifactId") == request.dependency.artifact_id
                && text(dep, "classifier").is_empty()
                && ["", "jar"].contains(&text(dep, "type").as_str())
            {
                set_text(dep, "version", &request.target_version);
                matches += 1;
            }
        }
    }
    ensure!(matches <= 1, "Dependency declaration is ambiguous");
    if matches == 0 {
        ensure!(
            request.strategy == Strategy::TransitiveOverride,
            "Selected direct dependency must be declared in the configured module POM"
        );
        let mut dep = Element::new("dependency");
        dep.namespace = container.namespace.clone();
        set_text(&mut dep, "groupId", &request.dependency.group_id);
        set_text(&mut dep, "artifactId", &request.dependency.artifact_id);
        set_text(&mut dep, "version", &request.target_version);
        container.children.push(XMLNode::Element(dep));
    }
    pom.write(fs::File::create(path)?)?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    #[serde(rename = "type", default = "jar")]
    pub kind: String,
    #[serde(default, deserialize_with = "empty_classifier")]
    pub classifier: Option<String>,
    #[serde(default)]
    pub children: Vec<Artifact>,
    pub scope: Option<String>,
}
fn jar() -> String {
    "jar".into()
}
fn empty_classifier<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.filter(|s| !s.is_empty()))
}
impl Artifact {
    pub fn key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.group_id,
            self.artifact_id,
            self.kind,
            self.classifier.as_deref().unwrap_or("")
        )
    }
    pub fn id(&self) -> String {
        format!("{}:{}", self.key(), self.version)
    }
    pub fn selected(&self, r: &Comparison) -> bool {
        self.group_id == r.dependency.group_id
            && self.artifact_id == r.dependency.artifact_id
            && self.kind == r.dependency.kind
            && self.classifier == r.dependency.classifier
    }
}

pub fn resolve(config: &Config, app: &Path, output: &Path) -> Result<(Artifact, Vec<Artifact>)> {
    let tree = output.join("dependency-tree.json");
    run_maven(
        config,
        app,
        &[
            format!("{PLUGIN}:tree"),
            "-DoutputType=json".into(),
            format!("-DoutputFile={}", tree.display()),
        ],
        &output.join("resolve.log"),
    )?;
    let root: Artifact = serde_json::from_slice(&fs::read(tree)?)?;
    fn flatten(a: &Artifact, list: &mut Vec<Artifact>) {
        for dep in &a.children {
            if dep.scope.as_deref() == Some("test") {
                continue;
            }
            let mut dep_flat = dep.clone();
            dep_flat.children.clear();
            if !list.iter().any(|a| a.id() == dep_flat.id()) {
                list.push(dep_flat);
            }
            flatten(dep, list);
        }
    }
    let mut dependencies = vec![];
    flatten(&root, &mut dependencies);
    dependencies.sort_by_key(Artifact::id);
    Ok((root, dependencies))
}

pub fn acquire(
    config: &Config,
    app: &Path,
    artifact: &Artifact,
    dest: &Path,
    source_root: &Path,
) -> Result<Value> {
    fs::create_dir_all(dest)?;
    let mut info = json!({"id":artifact.id(),"artifact":artifact,"status":"missing","acquisition":"maven-source-jar"});
    if artifact.kind != "jar" || artifact.classifier.is_some() {
        info["diagnostic"] = json!("Only unclassified JAR sources are supported");
        return Ok(info);
    }
    let base = format!(
        "{}:{}:{}:jar",
        artifact.group_id, artifact.artifact_id, artifact.version
    );
    let binary_args = vec![
        format!("{PLUGIN}:copy"),
        format!("-Dartifact={base}"),
        format!("-DoutputDirectory={}", dest.display()),
    ];
    run_maven(config, app, &binary_args, &dest.join("binary.log"))?;
    let binary = dest.join(format!("{}-{}.jar", artifact.artifact_id, artifact.version));
    info["binarySha256"] = json!(digest(&binary)?);
    let source_args = vec![
        format!("{PLUGIN}:copy"),
        format!("-Dartifact={base}:sources"),
        format!("-DoutputDirectory={}", dest.display()),
    ];
    if let Err(error) = run_maven(config, app, &source_args, &dest.join("sources.log")) {
        info["diagnostic"] = json!(error.to_string());
        return Ok(info);
    }
    let source = dest.join(format!(
        "{}-{}-sources.jar",
        artifact.artifact_id, artifact.version
    ));
    info["sourceSha256"] = json!(digest(&source)?);
    extract_sources(&source, source_root)?;
    info["status"] = json!("available");
    info["origin"] = json!({"coordinates":format!("{base}:sources"),"fileName":source.file_name().and_then(|s|s.to_str())});
    Ok(info)
}

pub fn extract_sources(archive: &Path, dest: &Path) -> Result<()> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(fs::File::open(archive)?)?;
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let path = file.enclosed_name().context("Unsafe source archive path")?;
        if file.is_dir() || path.extension().and_then(|e| e.to_str()) != Some("java") {
            continue;
        }
        total = total
            .checked_add(file.size())
            .context("Source archive size overflow")?;
        ensure!(
            total <= 256 * 1024 * 1024,
            "Source archive exceeds extraction limit"
        );
        ensure!(
            file.unix_mode().unwrap_or(0) & 0o170000 != 0o120000,
            "Source archive symlinks are unsupported"
        );
        let target = dest.join(path);
        fs::create_dir_all(target.parent().context("Missing parent")?)?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(256 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 == file.size(),
            "Source archive size mismatch"
        );
        fs::write(target, bytes)?;
    }
    ensure!(dest.exists(), "Source archive contains no Java sources");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn override_keeps_direct_dependency_and_application_properties() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("pom.xml");
        fs::write(
            &path,
            "<project><properties><v>1</v></properties><dependencies><dependency><groupId>g</groupId><artifactId>a</artifactId><version>1</version></dependency></dependencies></project>",
        )?;
        let request: Comparison = serde_json::from_value(
            json!({"entityRef":"component:default/demo","applicationRevision":"demo-baseline","dependency":{"groupId":"g","artifactId":"b"},"targetVersion":"2","strategy":"transitive-override"}),
        )?;
        change_pom(&path, &request)?;
        let pom = Element::parse(fs::File::open(path)?)?;
        assert_eq!(
            text(
                pom.get_child("dependencies")
                    .and_then(|d| d.get_child("dependency"))
                    .context("direct")?,
                "version"
            ),
            "1"
        );
        assert_eq!(
            text(
                pom.get_child("dependencyManagement")
                    .and_then(|d| d.get_child("dependencies"))
                    .and_then(|d| d.get_child("dependency"))
                    .context("override")?,
                "version"
            ),
            "2"
        );
        Ok(())
    }
}
