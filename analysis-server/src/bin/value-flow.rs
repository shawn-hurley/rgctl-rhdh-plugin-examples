//! Compare native reachability, dispatch-only ablation and adapter fact joins.
#[path = "../dispatch.rs"]
mod dispatch;
use anyhow::{Context, Result, ensure};
use rgctl_analysis::BlastRadiusEngine;
use rgctl_graph::{
    SnapshotNodeStore,
    schema::{EdgeType, Node, NodeType},
};
use rgctl_pipeline::{PipelineConfig, ProcessingPipeline};
use rgctl_plugin_api::LanguagePlugin;
use rgctl_registry::LanguageRegistry;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

fn callable_key(n: &Node, root: &Path) -> Result<String> {
    Ok(format!(
        "{}|{}|{}",
        Path::new(n.file_path.as_deref().context("source path")?)
            .strip_prefix(root)?
            .display(),
        n.qualified_name.as_deref().unwrap_or(&n.name),
        n.identity_discriminator()
            .or_else(|| n.signature_text())
            .unwrap_or("")
    ))
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().context("fixture root")?).canonicalize()?;
    let language = args.next().context("language")?;
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("case.json"))?)?;
    let plugin: Arc<dyn LanguagePlugin> = match language.as_str() {
        "java" => Arc::new(rgctl_lang_java::JavaPlugin::new()?),
        "javascript" => Arc::new(rgctl_lang_javascript::JavaScriptPlugin::new()?),
        "typescript" => Arc::new(rgctl_lang_typescript::TypeScriptPlugin::new()?),
        "python" => Arc::new(rgctl_lang_python::PythonPlugin::new()?),
        "go" => Arc::new(rgctl_lang_go::GoPlugin::new()?),
        _ => anyhow::bail!("Unknown language"),
    };
    let mut registry = LanguageRegistry::new();
    registry.register_language_plugin(plugin.clone());
    let mut config = PipelineConfig {
        show_progress: false,
        facts_cache: false,
        thread_count: Some(2),
        ..Default::default()
    };
    config.discovery.languages = Some(vec![language]);
    let registry = Arc::new(registry);
    let pipeline = ProcessingPipeline::with_config(registry.clone(), config);
    let mut sides = vec![];
    let mut extracted = vec![];
    for side in ["baseline", "candidate"] {
        let input = root.join(side);
        let path = input.join(".rgctl/graph.snapshot.bin");
        std::fs::create_dir_all(path.parent().context("snapshot parent")?)?;
        let (stats, _) = pipeline.process_repository_to_snapshot(&input, &path, Some(&input))?;
        let store = SnapshotNodeStore::open(&path)?;
        let mut functions = BTreeMap::new();
        let mut duplicate_keys = vec![];
        let mut nodes = vec![];
        store.for_each_node_seq(|n| nodes.push(n.clone()))?;
        for n in nodes.iter().filter(|n| n.node_type == NodeType::Function) {
            let key = callable_key(n, &input)?;
            if functions.insert(key.clone(), n.clone()).is_some() {
                duplicate_keys.push(key);
            }
        }
        ensure!(
            duplicate_keys.is_empty(),
            "Duplicate callable identities: {:?}",
            duplicate_keys
        );
        if side == "baseline" {
            let files: HashSet<_> = nodes
                .iter()
                .filter_map(|n| n.file_path.as_deref())
                .collect();
            for file in files {
                if !Path::new(file).is_file() {
                    continue;
                }
                let facts = rgctl_extraction::Extractor::new(registry.clone())
                    .with_repo_root(&input)
                    .extract_file_with_source(Path::new(file), std::fs::read(file)?)?;
                extracted.push(json!({"file":Path::new(file).strip_prefix(&input)?.to_string_lossy(),"relations":facts.relations}));
            }
        }
        sides.push((store, functions, stats.files_failed, nodes));
    }
    let old = &sides[0];
    let new = &sides[1];
    let changes: Vec<_> = old
        .1
        .iter()
        .filter(|(key, _)| key.starts_with("dependency/"))
        .filter(|(key, n)| {
            new.1
                .get(*key)
                .is_none_or(|next| n.code_hash.is_some() && n.code_hash != next.code_hash)
        })
        .map(|(_, n)| n.clone())
        .collect();
    let seeds: Vec<_> = changes.iter().map(|n| n.id).collect();
    let raw = BlastRadiusEngine::build_scoped(&old.0, &seeds)?;
    let overlay = dispatch::DispatchGraph::build_with_root(&old.0, &root.join("baseline"))?;
    let previous_overlay =
        dispatch::DispatchGraph::build_without_joins(&old.0, &root.join("baseline"))?;
    let expanded = overlay.engine(&old.0, &seeds)?;
    let previous_engine = previous_overlay.engine(&old.0, &seeds)?;
    let impacts = |engine: &BlastRadiusEngine| -> Result<(HashSet<uuid::Uuid>, Vec<Value>)> {
        let mut ids = HashSet::new();
        for seed in &seeds {
            ids.extend(engine.analyze(*seed)?.impact_zone_ids);
        }
        let rows=old.1.iter().filter(|(key,n)|key.starts_with("application/")&&ids.contains(&n.id)).map(|(_,n)|json!({"id":n.id,"name":n.name.to_string(),"qualifiedName":n.qualified_name.as_deref(),"file":n.file_path.as_deref()})).collect();
        Ok((ids, rows))
    };
    let (raw_ids, raw_impacts) = impacts(&raw)?;
    let (expanded_ids, expanded_impacts) = impacts(&expanded)?;
    let (previous_ids, _) = impacts(&previous_engine)?;
    let mut watches = vec![];
    for expected in manifest["watches"].as_array().context("watches")? {
        let name = expected["name"].as_str().context("watch name")?;
        let matches: Vec<_> = old
            .1
            .iter()
            .filter(|(key, n)| key.starts_with("application/") && &*n.name == name)
            .map(|(_, n)| n)
            .collect();
        watches.push(json!({"name":name,"expectedPossibleImpact":expected["expectedPossibleImpact"],"matchedSymbols":matches.iter().map(|n|json!({"id":n.id,"qualifiedName":n.qualified_name.as_deref()})).collect::<Vec<_>>(),"rawReached":matches.iter().any(|n|raw_ids.contains(&n.id)),"previousAdapterReached":matches.iter().any(|n|previous_ids.contains(&n.id)),"adapterReached":matches.iter().any(|n|expanded_ids.contains(&n.id))}));
    }
    let mut edges = vec![];
    old.0.for_each_edge(|from, to, kind| {
        if matches!(
            kind,
            EdgeType::Calls
                | EdgeType::Overrides
                | EdgeType::Implements
                | EdgeType::Extends
                | EdgeType::Injects
                | EdgeType::Instantiates
                | EdgeType::References
        ) {
            edges.push(json!({"from":from,"to":to,"type":format!("{kind:?}")}));
        }
        Ok(())
    })?;
    let mut resolution_reports = serde_json::Map::new();
    for side in ["baseline", "candidate"] {
        let path = root.join(side).join(".rgctl/callable_resolution.json");
        let report = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null);
        resolution_reports.insert(side.into(), report);
    }
    println!(
        "{}",
        json!({"callableResolution":resolution_reports,"changes":changes,"watches":watches,"rawImpacts":raw_impacts,"adapterImpacts":expanded_impacts,"possibleDispatch":overlay.evidence(),"originCoverage":overlay.origin_coverage,"unresolvedInterfaceCalls":overlay.unresolved,"baselineNodes":old.3,"candidateNodes":new.3,"edges":edges,"extracted":extracted,"extractionFailures":[old.2,new.2],"functionCounts":[old.1.len(),new.1.len()],"missingDependencyHashes":old.1.iter().filter(|(k,n)|k.starts_with("dependency/")&&n.code_hash.is_none()).count(),"provenance":{"rgctlRevision":env!("RGCTL_REVISION"),"rgctlSourceSha256":env!("RGCTL_SOURCE_SHA256")}})
    );
    Ok(())
}
