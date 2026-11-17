#[path = "../snapshot.rs"]
mod snapshot;
// Compare the former function matching with native post-filtered snapshot diff.
use anyhow::{Context, Result, ensure};
use rgctl_graph::{
    SnapshotNodeStore,
    schema::{Node, NodeType},
    snapshot_diff::{VecDiffSink, diff_snapshots},
};
use rgctl_pipeline::{PipelineConfig, ProcessingPipeline};
use rgctl_registry::LanguageRegistry;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

fn path(n: &Node, root: &Path) -> Result<String> {
    let p = Path::new(n.file_path.as_deref().context("source path")?);
    Ok(if p.is_absolute() {
        p.strip_prefix(root)?.to_string_lossy().into_owned()
    } else {
        p.to_string_lossy().into_owned()
    })
}
fn dependency(n: &Node, root: &Path) -> bool {
    path(n, root).is_ok_and(|p| p.starts_with("dependency/") || p.starts_with("dependencies/"))
}
fn identity(n: &Node, root: &Path, fallback: bool) -> Result<String> {
    let discriminator = n
        .identity_discriminator()
        .or_else(|| if fallback { n.signature_text() } else { None })
        .unwrap_or("");
    Ok(format!(
        "{}|{}|{}",
        path(n, root)?,
        n.qualified_name.as_deref().unwrap_or(&n.name),
        discriminator
    ))
}
fn index(store: &SnapshotNodeStore, root: &Path) -> Result<BTreeMap<String, Node>> {
    let mut rows = vec![];
    store.for_each_node_of_type(NodeType::Function, |n| rows.push(n.clone()))?;
    let mut index = BTreeMap::new();
    for n in rows {
        if dependency(&n, root) {
            ensure!(
                index.insert(identity(&n, root, true)?, n).is_none(),
                "Function identity collision"
            );
        }
    }
    Ok(index)
}
fn event(kind: &str, old: Option<&Node>, new: Option<&Node>, root: &Path) -> Result<Value> {
    let n = old.or(new).context("node")?;
    Ok(
        json!({"kind":kind,"stableKey":snapshot::function_key(n),"function":identity(n,root,false)?,"baselineHash":old.and_then(|n|n.code_hash.as_deref()),"candidateHash":new.and_then(|n|n.code_hash.as_deref())}),
    )
}
fn current(
    base: &SnapshotNodeStore,
    head: &SnapshotNodeStore,
    broot: &Path,
    hroot: &Path,
) -> Result<Vec<Value>> {
    let b = index(base, broot)?;
    let h = index(head, hroot)?;
    let mut result = vec![];
    for (key, n) in &b {
        let next = h.get(key);
        if next.is_some_and(|next| next.code_hash == n.code_hash) {
            continue;
        }
        result.push(event(
            if next.is_some() {
                "modified"
            } else {
                "removed"
            },
            Some(n),
            next,
            broot,
        )?);
    }
    for (key, n) in &h {
        if !b.contains_key(key) {
            result.push(event("added", None, Some(n), hroot)?);
        }
    }
    Ok(result)
}
fn structural(
    base: &SnapshotNodeStore,
    head: &SnapshotNodeStore,
    broot: &Path,
    hroot: &Path,
) -> Result<Value> {
    let mut sink = VecDiffSink::default();
    let stats = diff_snapshots(base, head, &mut sink)?;
    let mut events = vec![];
    let mut body = vec![];
    let mut metadata = vec![];
    for e in sink.nodes.into_iter().filter(|e| {
        e.base
            .or(e.head)
            .is_some_and(|r| r.node_type == NodeType::Function)
    }) {
        let old = e.base.map(|r| base.get_node(r.id)).transpose()?.flatten();
        let new = e.head.map(|r| head.get_node(r.id)).transpose()?.flatten();
        let root = if old.is_some() { broot } else { hroot };
        let n = old.as_ref().or(new.as_ref()).context("event node")?;
        if !dependency(n, root) {
            continue;
        }
        let kind = match (old.as_ref(), new.as_ref()) {
            (Some(_), None) => "removed",
            (None, Some(_)) => "added",
            _ => "modified",
        };
        let row = event(kind, old.as_ref(), new.as_ref(), root)?;
        events.push(row.clone());
        if old.is_some()
            && new.is_some()
            && old.as_ref().and_then(|n| n.code_hash.as_deref())
                == new.as_ref().and_then(|n| n.code_hash.as_deref())
        {
            metadata.push(row);
        } else {
            body.push(row);
        }
    }
    Ok(
        json!({"allGraphStats":stats,"dependencyFunctionEvents":events,"bodyChanges":body,"metadataOnly":metadata}),
    )
}
fn normalized(store: &SnapshotNodeStore, root: &Path, dest: &Path) -> Result<SnapshotNodeStore> {
    snapshot::write_logical_snapshot(store, root, dest)?;
    Ok(SnapshotNodeStore::open(dest)?)
}

fn signatures(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .map(|r| {
            format!(
                "{}|{}",
                r["kind"].as_str().unwrap(),
                r["function"].as_str().unwrap()
            )
        })
        .collect()
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(args.next().context("manifest")?)?)?;
    let out = PathBuf::from(args.next().context("output directory")?);
    std::fs::create_dir_all(&out)?;
    let mut results = vec![];
    for case in manifest.as_array().context("case list")? {
        let id = case["id"].as_str().context("id")?;
        let folder = out.join(id);
        std::fs::create_dir_all(&folder)?;
        let broot = PathBuf::from(case["baselineRoot"].as_str().context("base")?).canonicalize()?;
        let hroot =
            PathBuf::from(case["candidateRoot"].as_str().context("head")?).canonicalize()?;
        if case["indexJava"] == true {
            let mut registry = LanguageRegistry::new();
            rgctl_lang_java::register(&mut registry);
            let pipeline = ProcessingPipeline::with_config(
                Arc::new(registry),
                PipelineConfig {
                    show_progress: false,
                    facts_cache: false,
                    thread_count: Some(2),
                    ..Default::default()
                },
            );
            for root in [&broot, &hroot] {
                let p = root.join(".rgctl/graph.snapshot.bin");
                std::fs::create_dir_all(p.parent().unwrap())?;
                pipeline.process_repository_to_snapshot(root, &p, Some(root))?;
            }
        }
        let base = SnapshotNodeStore::open(&broot.join(".rgctl/graph.snapshot.bin"))?;
        let head = SnapshotNodeStore::open(&hroot.join(".rgctl/graph.snapshot.bin"))?;
        let existing = current(&base, &head, &broot, &hroot)?;
        let raw = structural(&base, &head, &broot, &hroot)?;
        let nb = normalized(&base, &broot, &folder.join("baseline.bin"))?;
        let nh = normalized(&head, &hroot, &folder.join("candidate.bin"))?;
        let normalized = structural(&nb, &nh, &broot, &hroot)?;
        let a = signatures(&existing);
        let b = signatures(
            normalized["bodyChanges"]
                .as_array()
                .context("body changes")?,
        );
        let row = json!({"id":id,"current":existing,"raw":raw,"normalized":normalized,"sameBodyChanges":a==b,"currentOnly":a.difference(&b).collect::<Vec<_>>(),"nativeOnly":b.difference(&a).collect::<Vec<_>>()});
        std::fs::write(folder.join("result.json"), serde_json::to_vec_pretty(&row)?)?;
        results.push(row);
    }
    std::fs::write(
        out.join("results.json"),
        serde_json::to_vec_pretty(
            &json!({"probeProvenance":{"rgctlRevision":env!("RGCTL_REVISION"),"rgctlSourceSha256":env!("RGCTL_SOURCE_SHA256")},"cases":results}),
        )?,
    )?;
    println!("Compared {} snapshot pairs", results.len());
    Ok(())
}
