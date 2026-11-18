use super::{model::*, store};
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub fn acquire(
    source: &Path,
    dir: &Path,
    input: &Path,
    repository: Option<PathBuf>,
) -> Result<(String, Vec<LibrarySource>, Vec<PathBuf>)> {
    let pom = xmltree::Element::parse(fs::File::open(source.join("pom.xml"))?)?;
    ensure!(
        pom.get_child("modules").is_none(),
        "Select one Maven module for dependency-source indexing"
    );
    let config = crate::model::Config {
        go_proxy: None,
        applications: BTreeMap::new(),
        data_dir: dir.into(),
        maven_repository: repository,
        queue_capacity: 8,
    };
    let work = dir.join("maven");
    fs::create_dir_all(&work)?;
    let (root, dependencies) = crate::maven::resolve(&config, source, &work)?;
    let coordinate =
        |a: &crate::maven::Artifact| format!("{}:{}:{}", a.group_id, a.artifact_id, a.version);
    let project = coordinate(&root);
    let mut artifacts = vec![];
    let mut files = vec![];
    fn java_files(root: &Path, path: &Path, prefix: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                java_files(root, &entry.path(), prefix, files)?;
            } else if entry.path().extension().is_some_and(|x| x == "java") {
                files.push(prefix.join(entry.path().strip_prefix(root)?));
            }
        }
        Ok(())
    }
    for artifact in dependencies {
        let prefix = format!("dependencies/{}", store::key(&artifact.key()));
        let destination = input.join(&prefix);
        let info = crate::maven::acquire(
            &config,
            source,
            &artifact,
            &work.join(store::key(&artifact.id())),
            &destination,
        )?;
        let status = info["status"].as_str().unwrap_or("missing").to_owned();
        if status == "available" {
            java_files(&destination, &destination, Path::new(&prefix), &mut files)?;
        }
        artifacts.push(LibrarySource {
            coordinate: coordinate(&artifact),
            prefix,
            direct: root.children.iter().any(|a| a.id() == artifact.id()),
            status,
            source_sha256: info["sourceSha256"].as_str().map(str::to_owned),
        });
    }
    Ok((project, artifacts, files))
}
