//! Store logical paths/IDs in snapshots; physical source roots belong to the job.
use anyhow::{Context, Result, ensure};
use rgctl_graph::{
    SnapshotNodeStore,
    schema::{Edge, Node, NodeType},
    segmented_spill::{SegmentedSpill, write_columnar_from_spill},
    stable_key::stable_key_from_node,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
};

pub fn logical_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .context("Source outside snapshot root")?
    } else {
        path
    };
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "Invalid logical source path: {}",
        relative.display()
    );
    Ok(relative.to_owned())
}

pub fn write_logical_snapshot(
    source: &SnapshotNodeStore,
    root: &Path,
    destination: &Path,
) -> Result<()> {
    let mut spill = SegmentedSpill::create(destination.with_extension("path-spill"))?;
    let mut ids = HashMap::new();
    let mut unique = HashSet::new();
    let mut error = None;
    source.for_each_node_seq(|node| {
        if error.is_some() {
            return;
        }
        let result = (|| -> Result<()> {
            let mut n: Node = node.clone();
            let original = n.id;
            if let Some(file) = n.file_path.as_deref() {
                let path = Path::new(file);
                // Relative synthetic sources are retained; absolute source files
                // must belong to this prepared input. Never strip arbitrary prefixes.
                let relative = logical_path(root, path)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if n.node_type == NodeType::File {
                    n.name = relative.clone().into();
                }
                n = n.with_file_path(relative);
            }
            ensure!(unique.insert(n.id), "Logical snapshot identity collision");
            ids.insert(original, n.id);
            spill.append_node(&n)?;
            Ok(())
        })();
        if let Err(e) = result {
            error = Some(e);
        }
    })?;
    if let Some(e) = error {
        return Err(e);
    }
    source.for_each_edge(|from, to, kind| {
        let from = *ids
            .get(&from)
            .ok_or_else(|| rgctl_error::Error::Other("Missing edge source".into()))?;
        let to = *ids
            .get(&to)
            .ok_or_else(|| rgctl_error::Error::Other("Missing edge target".into()))?;
        spill.append_edge(&Edge::new(from, to, kind))
    })?;
    write_columnar_from_spill(spill.finish()?, destination)?;
    Ok(())
}

/// Native stable identity, without a display-signature/body fallback.
pub fn function_key(node: &Node) -> String {
    format!("{:016x}", stable_key_from_node(node).as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logical_paths_reject_escape_and_unrelated_roots() -> Result<()> {
        let root = Path::new("/jobs/baseline/input");
        assert_eq!(
            logical_path(root, &root.join("dependencies/a/Api.java"))?,
            Path::new("dependencies/a/Api.java")
        );
        assert!(logical_path(root, Path::new("../outside.java")).is_err());
        assert!(logical_path(root, Path::new("/jobs/candidate/input/Api.java")).is_err());
        assert!(logical_path(root, Path::new("/jobs/baseline/input-other/Api.java")).is_err());
        Ok(())
    }
}
