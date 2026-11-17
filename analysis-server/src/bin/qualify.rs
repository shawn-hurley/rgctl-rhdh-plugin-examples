#[path = "../dispatch.rs"]
mod dispatch;
use anyhow::{Context, Result};
use rgctl_analysis::BlastRadiusEngine;
use rgctl_graph::{
    SnapshotNodeStore,
    schema::{EdgeType, NodeType},
};
use rgctl_pipeline::{PipelineConfig, ProcessingPipeline};
use rgctl_plugin_api::LanguagePlugin;
use rgctl_registry::LanguageRegistry;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).context("source root")?).canonicalize()?;
    let materialize_fields = std::env::args().any(|s| s == "--materialize-fields");
    let mut registry = LanguageRegistry::new();
    rgctl_lang_java::register(&mut registry);
    let pipeline = ProcessingPipeline::with_config(
        Arc::new(registry),
        PipelineConfig {
            show_progress: false,
            materialize_fields,
            facts_cache: false,
            thread_count: Some(4),
            ..Default::default()
        },
    );
    let mut sides = vec![];
    for side in ["baseline", "candidate"] {
        let input = root.join(side);
        let snapshot = input.join(".rgctl/graph.snapshot.bin");
        std::fs::create_dir_all(snapshot.parent().context("parent")?)?;
        let (stats, _) =
            pipeline.process_repository_to_snapshot(&input, &snapshot, Some(&input))?;
        let store = SnapshotNodeStore::open(&snapshot)?;
        let mut functions = BTreeMap::new();
        store.for_each_node_of_type(NodeType::Function, |n| {
            functions.insert(
                format!(
                    "{}|{}",
                    n.qualified_name.as_deref().unwrap_or(&n.name),
                    n.identity_discriminator()
                        .or_else(|| n.signature_text())
                        .unwrap_or("")
                ),
                n.clone(),
            );
        })?;
        sides.push((store, functions, stats.files_failed));
    }
    let plugin = rgctl_lang_java::JavaPlugin::new()?;
    let mut extracted = vec![];
    let mut candidate_extracted = vec![];
    for side in ["baseline", "candidate"] {
        for relative in [
            "application/qualification/Main.java",
            "dependency/qualification/Library.java",
        ] {
            let file = root.join(side).join(relative);
            let facts = plugin.extract_all(&file, &std::fs::read(&file)?)?;
            for r in facts.relations {
                let value = json!({"type":format!("{:?}",r.relation_type),"from":r.from,"to":r.to,"typeHint":r.to_type_hint,"qualifiedHint":r.to_qualified_hint,"metadata":r.metadata});
                if side == "baseline" {
                    extracted.push(value);
                } else {
                    candidate_extracted.push(value);
                }
            }
        }
    }
    let old = &sides[0];
    let new = &sides[1];
    let mut changes = vec![];
    let mut impacts = vec![];
    let seeds: Vec<_> = old
        .1
        .iter()
        .filter(|(_, n)| {
            n.file_path
                .as_deref()
                .is_some_and(|p| p.contains("/dependency/"))
        })
        .filter(|(key, n)| {
            new.1
                .get(*key)
                .is_none_or(|next| next.code_hash != n.code_hash)
        })
        .map(|(_, n)| n.id)
        .collect();
    let dispatch = dispatch::DispatchGraph::build_with_root(&old.0, &root.join("baseline"))?;
    let engine = dispatch.engine(&old.0, &seeds)?;
    for id in seeds {
        let n = old.0.get_node(id)?.context("seed")?;
        changes.push(n.qualified_name.as_deref().unwrap_or(&n.name).to_owned());
        for caller in engine.analyze(id)?.impact_zone_ids {
            if let Some(n) = old.0.get_node(caller)? {
                if n.file_path
                    .as_deref()
                    .is_some_and(|p| p.contains("/application/"))
                {
                    impacts.push(n.qualified_name.as_deref().unwrap_or(&n.name).to_owned());
                }
            }
        }
    }
    impacts.sort();
    impacts.dedup();
    let mut edges = vec![];
    old.0.for_each_edge(|from,to,kind|{if matches!(kind,EdgeType::Calls|EdgeType::Injects|EdgeType::Implements|EdgeType::Overrides|EdgeType::Instantiates){let a=old.0.get_node(from)?;let b=old.0.get_node(to)?;edges.push(json!({"type":format!("{kind:?}"),"from":a.map(|n|n.qualified_name.as_deref().unwrap_or(&n.name).to_owned()),"to":b.map(|n|n.qualified_name.as_deref().unwrap_or(&n.name).to_owned())}));}Ok(())})?;
    let mut nodes = Vec::new();
    old.0.for_each_node_seq(|n| nodes.push(n.clone()))?;
    let mut backend = old.0.hydrate_backend()?;
    let detailed_edges = backend.all_edges()?;
    let candidate_edges = new.0.hydrate_backend()?.all_edges()?;
    let mut candidate_nodes = Vec::new();
    new.0
        .for_each_node_seq(|n| candidate_nodes.push(n.clone()))?;
    let before_calls: std::collections::HashSet<_> = detailed_edges
        .iter()
        .filter(|e| e.edge_type == EdgeType::Calls)
        .map(|e| (e.from, e.to))
        .collect();
    let stats = rgctl_analysis::mro::apply_mro_pass(
        &mut backend,
        rgctl_analysis::mro::MroStrategy::ClassOverInterface,
    )?;
    let after_edges = backend.all_edges()?;
    let added_calls: Vec<_> = after_edges
        .iter()
        .filter(|e| e.edge_type == EdgeType::Calls && !before_calls.contains(&(e.from, e.to)))
        .collect();
    let mut mro_impacts = vec![];
    let mro_engine = BlastRadiusEngine::build(&backend)?;
    for n in old
        .1
        .values()
        .filter(|n| changes.contains(&n.qualified_name.as_deref().unwrap_or(&n.name).to_owned()))
    {
        for id in mro_engine.analyze(n.id)?.impact_zone_ids {
            if let Some(caller) = old.0.get_node(id)? {
                if caller
                    .file_path
                    .as_deref()
                    .is_some_and(|p| p.contains("/application/"))
                {
                    mro_impacts.push(
                        caller
                            .qualified_name
                            .as_deref()
                            .unwrap_or(&caller.name)
                            .to_owned(),
                    );
                }
            }
        }
    }
    mro_impacts.sort();
    mro_impacts.dedup();
    println!(
        "{}",
        json!({"possibleDispatch":dispatch.evidence(),"originCoverage":dispatch.origin_coverage,"unresolvedDispatch":dispatch.unresolved,"policy":"all-known-compatible-implementations-across-configurations","extractedRelations":extracted,"changes":changes,"applicationImpacts":impacts,"edges":edges,
        "baselineNodes":nodes,"candidateNodes":candidate_nodes,"candidateDetailedEdges":candidate_edges,"candidateExtractedRelations":candidate_extracted,"detailedEdges":detailed_edges,"materializeFields":materialize_fields,
        "mroExperiment":{"callsRetargeted":stats.calls_retargeted,"addedCalls":added_calls,"applicationImpacts":mro_impacts},
        "extractionFailures":[old.2,new.2],"rgctlRevision":env!("RGCTL_REVISION"),"rgctlSourceSha256":env!("RGCTL_SOURCE_SHA256")})
    );
    Ok(())
}
