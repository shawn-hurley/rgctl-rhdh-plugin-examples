use crate::{
    maven::{self, Artifact},
    model::{Comparison, Config, now},
};
use anyhow::{Context, Result, ensure};
#[cfg(test)]
use rgctl_analysis::BlastRadiusEngine;
use rgctl_graph::{
    SnapshotNodeStore,
    schema::{Node, NodeType},
};
use rgctl_pipeline::{PipelineConfig, ProcessingPipeline};
use rgctl_registry::LanguageRegistry;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn hash(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}
struct Snapshot {
    root: PathBuf,
    store: SnapshotNodeStore,
    functions: BTreeMap<String, Node>,
    owners: BTreeMap<String, Value>,
    label: String,
    ambiguous: HashSet<(String, String)>,
}
impl Snapshot {
    fn is_ambiguous(&self, n: &Node) -> bool {
        self.ambiguous.contains(&(
            n.file_path.as_deref().unwrap_or("").into(),
            n.qualified_name.as_deref().unwrap_or(&n.name).into(),
        ))
    }
    fn relative(&self, n: &Node) -> Result<String> {
        Ok(crate::snapshot::logical_path(
            &self.root,
            Path::new(
                n.file_path
                    .as_deref()
                    .context("Function has no source file")?,
            ),
        )?
        .to_string_lossy()
        .replace('\\', "/"))
    }

    fn owner(&self, n: &Node) -> Result<(&Value, String)> {
        let relative = self.relative(n)?;
        let (prefix, owner) = self
            .owners
            .iter()
            .find(|(prefix, _)| relative.starts_with(&format!("{prefix}/")))
            .context("Function source ownership is unknown")?;
        Ok((owner, relative[prefix.len() + 1..].into()))
    }
    fn symbol(&self, n: &Node) -> Result<Value> {
        let (owner, file) = self.owner(n)?;
        let ambiguous = self.is_ambiguous(n);
        Ok(
            json!({"id":format!("{}:{}",self.label,n.id),"rgctlId":n.id,"snapshot":self.label,
            "name":n.name.to_string(),"qualifiedName":n.qualified_name.as_deref(),"signature":if ambiguous {None}else{n.signature_text()},
            "resolution":if ambiguous {"ambiguous-overload"}else{"resolved-static"},
            "sourceId":owner["id"],"file":file,"startLine":if ambiguous {None}else{n.start_line},"endLine":if ambiguous {None}else{n.end_line}}),
        )
    }
    fn block(&self, n: &Node) -> Result<Value> {
        ensure!(
            !self.is_ambiguous(n),
            "Cannot attribute a source block to a merged overload identity"
        );
        let (owner, file) = self.owner(n)?;
        let path = self.root.join(self.relative(n)?);
        let start = n.start_line.context("Missing function start line")?;
        let end = n.end_line.context("Missing function end line")?;
        let content = fs::read_to_string(&path)?;
        let lines: Vec<_> = content.lines().collect();
        ensure!(
            start > 0 && end >= start && end <= lines.len(),
            "Invalid function range for {}",
            file
        );
        Ok(
            json!({"id":format!("{}:{}",self.label,n.id),"snapshot":self.label,"sourceId":owner["id"],
            "sourceIdentity":owner,"file":file,"startLine":start,"endLine":end,
            "precision":"enclosing-function","text":lines[start-1..end].join("\n")}),
        )
    }
}

fn index(
    root: PathBuf,
    label: &str,
    owners: BTreeMap<String, Value>,
    diagnostics: &mut Vec<Value>,
) -> Result<Snapshot> {
    let mut registry = LanguageRegistry::new();
    let java = Arc::new(crate::java::ObservedJava::new()?);
    registry.register_language_plugin(java.clone());
    let mut config = PipelineConfig {
        show_progress: false,
        thread_count: Some(4),
        facts_cache: false,
        ..Default::default()
    };
    config.discovery.languages = Some(vec!["java".into()]);
    // Scan only prepared application/dependency Java inputs, streaming the graph
    // to a columnar snapshot; no CLI process or whole-graph JSON export.
    let pipeline = ProcessingPipeline::with_config(Arc::new(registry), config);
    let root = root.canonicalize()?;
    let path = root.join(".rgctl/graph.snapshot.bin");
    let extracted_path = root.join(".rgctl/extracted.snapshot.bin");
    fs::create_dir_all(path.parent().context("Snapshot parent")?)?;
    let (stats, _) =
        pipeline.process_repository_to_snapshot(&root, &extracted_path, Some(&root))?;
    if stats.files_failed > 0 {
        diagnostics.push(
            json!({"code":"EXTRACTION_FAILURE","snapshot":label,"filesFailed":stats.files_failed}),
        );
    }
    {
        let extracted = SnapshotNodeStore::open(&extracted_path)?;
        crate::snapshot::write_logical_snapshot(&extracted, &root, &path)?;
    }
    fs::remove_file(extracted_path)?;
    let store = SnapshotNodeStore::open(&path)?;
    let mut functions = BTreeMap::new();
    let mut collisions = vec![];
    store.for_each_node_of_type(NodeType::Function, |node| {
        let key = crate::snapshot::function_key(node);
        if functions.insert(key.clone(), node.clone()).is_some() {
            collisions.push(key);
        }
    })?;
    ensure!(
        collisions.is_empty(),
        "Ambiguous function identities: {}",
        collisions
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
    ensure!(!functions.is_empty(), "No Java functions were extracted");
    let ambiguous = java
        .ambiguities
        .lock()
        .map_err(|_| anyhow::anyhow!("Java diagnostics lock failed"))?
        .clone()
        .into_iter()
        .map(|(file, name)| {
            Ok((
                crate::snapshot::logical_path(&root, Path::new(&file))?
                    .to_string_lossy()
                    .replace('\\', "/"),
                name,
            ))
        })
        .collect::<Result<HashSet<_>>>()?;
    for (file, name) in &ambiguous {
        diagnostics.push(json!({"code":"AMBIGUOUS_OVERLOAD","snapshot":label,"file":file,"qualifiedName":name,"message":"The imported graph builder merges functions with the same file and qualified name. Overload-specific changes are omitted and path resolution may be ambiguous."}));
    }
    Ok(Snapshot {
        root,
        store,
        functions,
        owners,
        label: label.into(),
        ambiguous,
    })
}

struct FunctionDelta {
    kind: &'static str,
    baseline: Option<Node>,
    candidate: Option<Node>,
}
fn function_deltas(old: &Snapshot, new: &Snapshot) -> Result<Vec<FunctionDelta>> {
    let mut sink = rgctl_graph::snapshot_diff::VecDiffSink::default();
    rgctl_graph::snapshot_diff::diff_snapshots(&old.store, &new.store, &mut sink)?;
    let mut events: Vec<_> = sink
        .nodes
        .into_iter()
        .filter(|e| {
            e.base
                .or(e.head)
                .is_some_and(|r| r.node_type == NodeType::Function)
        })
        .collect();
    events.sort_by_key(|e| e.key);
    let mut deltas = vec![];
    for event in events {
        let baseline = event
            .base
            .map(|r| old.store.get_node(r.id))
            .transpose()?
            .flatten();
        let candidate = event
            .head
            .map(|r| new.store.get_node(r.id))
            .transpose()?
            .flatten();
        // Structural metadata changes (including harmless line shifts) are not
        // code-hash changes. Keep the existing report's change interpretation.
        if let (Some(a), Some(b)) = (&baseline, &candidate) {
            if a.code_hash.is_some() && a.code_hash == b.code_hash {
                continue;
            }
        }
        let kind = match (&baseline, &candidate) {
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            _ => "modified",
        };
        deltas.push(FunctionDelta {
            kind,
            baseline,
            candidate,
        });
    }
    Ok(deltas)
}

pub fn compare(
    config: &Config,
    request: &Comparison,
    dir: &Path,
    stage: &dyn Fn(&str) -> Result<()>,
) -> Result<Value> {
    let started = now();
    let app_config = config
        .applications
        .get(&request.entity_ref)
        .context("Unknown application")?;
    stage("resolving")?;
    let original = dir.join("application");
    maven::copy_tree(&app_config.path, &original)?;
    let application_digest = maven::tree_digest(&original)?;
    let mut resolved = vec![];
    for label in ["baseline", "candidate"] {
        let root = dir.join(label);
        fs::create_dir_all(&root)?;
        let app = root.join("app");
        maven::copy_tree(&original, &app)?;
        if label == "candidate" {
            maven::change_pom(&app.join("pom.xml"), request)?;
        }
        let (tree, dependencies) = maven::resolve(config, &app, &root)?;
        let selected: Vec<_> = dependencies
            .iter()
            .filter(|d| d.selected(request))
            .collect();
        ensure!(
            selected.len() == 1,
            "Selected dependency is absent or ambiguous in {label}"
        );
        if label == "candidate" {
            ensure!(
                selected[0].version == request.target_version,
                "Requested version was not effective in candidate"
            );
        }
        if label == "baseline" {
            ensure!(
                selected[0].version != request.target_version,
                "Target version is already the effective baseline"
            );
            let is_direct = tree.children.iter().any(|a| a.selected(request));
            ensure!(
                is_direct == (request.strategy == crate::model::Strategy::DirectUpgrade),
                "Strategy must match a direct dependency or a transitive override"
            );
        }
        resolved.push((label, root, tree, dependencies));
    }
    let baseline_deps = &resolved[0].3;
    let candidate_deps = &resolved[1].3;
    let keys: std::collections::BTreeSet<_> = baseline_deps
        .iter()
        .chain(candidate_deps.iter())
        .map(Artifact::key)
        .collect();
    let deltas: Vec<_> = keys
        .into_iter()
        .filter_map(|key| {
            let old = baseline_deps.iter().find(|d| d.key() == key);
            let new = candidate_deps.iter().find(|d| d.key() == key);
            if old.map(|d| &d.version) == new.map(|d| &d.version) {
                None
            } else {
                Some(json!({"coordinate":key,"baseline":old,"candidate":new}))
            }
        })
        .collect();
    if request.strategy == crate::model::Strategy::TransitiveOverride {
        for direct in &resolved[0].2.children {
            ensure!(
                resolved[1].2.children.iter().any(|d| d.id() == direct.id()),
                "Transitive override changed a direct dependency version"
            );
        }
    }
    stage("acquiring-sources")?;
    let mut diagnostics = vec![];
    let mut snapshots_input = vec![];
    let mut sources = vec![];
    let application_identity = json!({"id":format!("application:{application_digest}"),"kind":"application","revision":request.application_revision,"sha256":application_digest});
    for (label, root, _, dependencies) in &resolved {
        let mut owners =
            BTreeMap::from([("app/src/main/java".into(), application_identity.clone())]);
        // Keep Maven working files outside the extraction tree. Index main
        // application sources only; tests do not become production impacts.
        let input = root.join("input");
        maven::copy_tree(
            &root.join("app/src/main/java"),
            &input.join("app/src/main/java"),
        )?;
        for artifact in dependencies {
            let prefix = format!("dependencies/{}", hash(&artifact.key()));
            let mut source = maven::acquire(
                config,
                &root.join("app"),
                artifact,
                &root.join("artifacts").join(hash(&artifact.id())),
                &input.join(&prefix),
            )?;
            source["snapshot"] = json!(label);
            if source["status"] != "available" {
                diagnostics.push(json!({"code":"MISSING_SOURCES","stage":"acquiring-sources","artifact":artifact.id(),"snapshot":label,"message":source["diagnostic"]}));
            } else {
                owners.insert(prefix, json!({"id":format!("maven:{}:{}",artifact.id(),source["sourceSha256"].as_str().unwrap_or("")),"kind":"dependency","artifact":artifact,"sha256":source["sourceSha256"]}));
            }
            sources.push(source);
        }
        snapshots_input.push((input, *label, owners));
    }
    // Prove unchanged direct binary artifacts in the override scenario.
    if request.strategy == crate::model::Strategy::TransitiveOverride {
        for direct in &resolved[0].2.children {
            let digests: Vec<_> = sources
                .iter()
                .filter(|s| {
                    s["artifact"]["groupId"] == direct.group_id
                        && s["artifact"]["artifactId"] == direct.artifact_id
                })
                .map(|s| s["binarySha256"].clone())
                .collect();
            ensure!(
                digests.len() == 2 && digests[0] == digests[1] && digests[0].is_string(),
                "Unchanged direct dependency binary could not be verified"
            );
        }
    }
    stage("indexing")?;
    let mut snapshots = vec![];
    for (root, label, owners) in snapshots_input {
        snapshots.push(index(root, label, owners, &mut diagnostics)?);
    }
    let old = &snapshots[0];
    let new = &snapshots[1];
    stage("comparing")?;
    let mut changes = vec![];
    let mut seeds = vec![];
    let mut blocks = BTreeMap::new();
    let available_in_both = |s: &Snapshot, n: &Node| -> bool {
        let Ok(relative) = s.relative(n) else {
            return false;
        };
        let Some((prefix, _)) = s
            .owners
            .iter()
            .find(|(prefix, _)| relative.starts_with(&format!("{prefix}/")))
        else {
            return false;
        };
        old.owners.contains_key(prefix) && new.owners.contains_key(prefix)
    };
    for delta in function_deltas(old, new)? {
        let node = delta
            .baseline
            .as_ref()
            .or(delta.candidate.as_ref())
            .context("Function delta")?;
        let snapshot = if delta.baseline.is_some() { old } else { new };
        if !snapshot.relative(node)?.starts_with("dependencies/")
            || !available_in_both(snapshot, node)
        {
            continue;
        }
        if delta.baseline.as_ref().is_some_and(|n| old.is_ambiguous(n))
            || delta
                .candidate
                .as_ref()
                .is_some_and(|n| new.is_ambiguous(n))
        {
            continue;
        }
        let id = format!("change-{}", crate::snapshot::function_key(node));
        for (s, n) in delta
            .baseline
            .as_ref()
            .map(|n| (old, n))
            .into_iter()
            .chain(delta.candidate.as_ref().map(|n| (new, n)))
        {
            match s.block(n) {Ok(block)=>{blocks.insert(block["id"].as_str().context("Block ID")?.to_owned(),block);},Err(e)=>diagnostics.push(json!({"code":"SOURCE_BLOCK_UNAVAILABLE","symbol":s.symbol(n)?,"message":e.to_string()}))}
        }
        if delta
            .baseline
            .as_ref()
            .is_some_and(|n| n.code_hash.is_none())
            || delta
                .candidate
                .as_ref()
                .is_some_and(|n| n.code_hash.is_none())
        {
            diagnostics
                .push(json!({"code":"MISSING_FUNCTION_HASH","symbol":snapshot.symbol(node)?}));
            continue;
        }
        changes.push(json!({"id":id,"kind":delta.kind,"baseline":delta.baseline.as_ref().map(|n|old.symbol(n)).transpose()?,"candidate":delta.candidate.as_ref().map(|n|new.symbol(n)).transpose()?}));
        if let Some(node) = delta.baseline {
            seeds.push((id, node.id));
        }
    }
    ensure!(
        changes.len() <= 10000,
        "Comparison exceeds supported report size (10000 changed functions)"
    );
    stage("tracing-impact")?;
    let dispatch = crate::dispatch::DispatchGraph::build_with_root(&old.store, &old.root)?;
    // Fixed graphs can share a projection. Context-dependent edges must be
    // evaluated for each changed function, including its explanatory paths.
    let common_projection = if dispatch.uses_context() {
        None
    } else {
        Some(dispatch.projection(
            &old.store,
            &seeds.iter().map(|(_, id)| *id).collect::<Vec<_>>(),
        )?)
    };
    diagnostics.push(json!({"code":"NATIVE_VALUE_ORIGIN_COVERAGE","message":"Native origin propagation uses conservative construction candidates and merged caller contexts; unresolved handoffs and limits remain partial.","coverage":dispatch.origin_coverage}));
    diagnostics.push(json!({"code":"CONSERVATIVE_DISPATCH_POLICY","message":"Possible implementations are included across profiles, primary preferences and conditional factories. Explicit understood qualifiers may narrow candidates.","possibleEdges":dispatch.possible.len(),"unresolvedInterfaceCalls":dispatch.unresolved}));
    let mut impacts = BTreeMap::<String, Value>::new();
    let mut paths = vec![];
    for (change_id, target) in &seeds {
        let projection;
        let (engine, incoming) = if let Some(common) = &common_projection {
            (&common.0, &common.1)
        } else {
            projection = dispatch.projection(&old.store, &[*target])?;
            (&projection.0, &projection.1)
        };
        let blast = engine.analyze(*target)?;
        let reachable: HashSet<_> = blast.impact_zone_ids.into_iter().collect();
        let mut next = HashMap::new();
        let mut queue = VecDeque::from([*target]);
        let mut seen = HashSet::from([*target]);
        while let Some(callee) = queue.pop_front() {
            for caller in incoming.get(&callee).into_iter().flatten() {
                if seen.insert(*caller) {
                    next.insert(*caller, callee);
                    queue.push_back(*caller);
                }
            }
        }
        for node in old.functions.values() {
            if !old.relative(node)?.starts_with("app/") || !reachable.contains(&node.id) {
                continue;
            }
            let mut chain = vec![node.id];
            while *chain.last().context("Empty path")? != *target {
                let Some(id) = next.get(chain.last().context("Empty path")?) else {
                    break;
                };
                chain.push(*id);
            }
            if chain.last() != Some(target) {
                diagnostics
                    .push(json!({"code":"IMPACT_PATH_UNAVAILABLE","symbol":old.symbol(node)?}));
                continue;
            }
            let mut symbols = vec![];
            for id in &chain {
                let path_node = old.store.get_node(*id)?.context("Call path node missing")?;
                symbols.push(old.symbol(&path_node)?);
                let block_id = format!("baseline:{id}");
                if !blocks.contains_key(&block_id) {
                    match old.block(&path_node) {
                        Ok(block) => { blocks.insert(block_id, block); },
                        Err(error) => diagnostics.push(json!({"code":"SOURCE_BLOCK_UNAVAILABLE","symbol":old.symbol(&path_node)?,"message":error.to_string()})),
                    }
                }
            }
            let path_id = format!("path-{}-{}", node.id, change_id);
            let edges: Vec<_> = symbols
                .windows(2)
                .enumerate()
                .map(|(i, w)| {
                    let from = chain[i];
                    let to = chain[i+1];
                    let binding = dispatch.possible.get(&(from,to));
                    json!({"from":w[0]["id"],"to":w[1]["id"],"type":if binding.is_some(){"PossibleDispatch"}else{"Calls"},"binding":binding})
                })
                .collect();
            let resolution = if symbols
                .iter()
                .any(|s| s["resolution"] == "ambiguous-overload")
            {
                "ambiguous-overload"
            } else if chain
                .windows(2)
                .any(|w| dispatch.possible.contains_key(&(w[0], w[1])))
            {
                "possible-dispatch"
            } else {
                "resolved-static"
            };
            paths.push(json!({"id":path_id,"changeId":change_id,"symbols":symbols,"edges":edges,"resolution":resolution,"interpretation":"potential-static-impact"}));
            let id = format!("baseline:{}", node.id);
            let entry = impacts.entry(id.clone()).or_insert(json!({"id":id,"symbol":old.symbol(node)?,"sourceBlockId":id,"changeIds":[],"pathIds":[]}));
            entry["changeIds"]
                .as_array_mut()
                .context("Change IDs")?
                .push(json!(change_id));
            entry["pathIds"]
                .as_array_mut()
                .context("Path IDs")?
                .push(json!(path_id));
            match old.block(node) {
                Ok(block) => {
                    blocks.insert(id, block);
                }
                Err(e) => {
                    entry["sourceBlockId"] = Value::Null;
                    diagnostics
                        .push(json!({"code":"SOURCE_BLOCK_UNAVAILABLE","message":e.to_string()}));
                }
            }
        }
    }
    ensure!(
        paths.len() <= 50000,
        "Comparison exceeds supported path report size"
    );
    stage("assembling-report")?;
    // rgctl exposes resolved Calls edges but this integration cannot certify
    // that every Java call resolved. Never turn that unknown into "no impact".
    diagnostics.push(json!({"code":"STATIC_CALL_COVERAGE_UNVERIFIED","message":"Findings use rgctl Calls edges and conservative possible implementation paths. Complete call resolution, reflection and runtime dispatch coverage are not certified."}));
    Ok(
        json!({"schemaVersion":"1","comparisonId":dir.file_name().and_then(|s|s.to_str()),
        "application":{"entityRef":request.entity_ref,"revision":request.application_revision,"sourceIdentity":application_identity},
        "resolution":{"requested":request,"baseline":resolved[0].3,"candidate":resolved[1].3,"deltas":deltas},
        "dispatch":{"policy":"all-known-compatible-implementations-across-configurations","possibleEdges":dispatch.evidence(),"unresolvedInterfaceCalls":dispatch.unresolved,"originCoverage":dispatch.origin_coverage},"sources":sources,"changes":changes,"impacts":impacts.into_values().collect::<Vec<_>>(),"paths":paths,
        "sourceBlocks":blocks.into_values().collect::<Vec<_>>(),"coverage":{"status":"partial","scope":"Java main-source functions, Calls edges and possible implementations across configurations","truncated":false},
        "diagnostics":diagnostics,"provenance":{"serverVersion":env!("CARGO_PKG_VERSION"),"rgctlRevision":env!("RGCTL_REVISION"),"rgctlSourceSha256":env!("RGCTL_SOURCE_SHA256"),"startedAt":started,"finishedAt":now(),"options":{"language":"java","precision":"enclosing-function","applicationSources":"src/main/java","pathIdentity":"snapshot-relative","comparisonEngine":"rgctl-snapshot-diff","functionFilter":"post-diff","dispatchPolicy":"all-known-compatible-implementations-across-configurations"}}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn baseline_callers_and_context_survive_a_removed_method() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let baseline = dir.path().join("baseline");
        fs::create_dir_all(baseline.join("app"))?;
        fs::write(
            baseline.join("app/Api.java"),
            "class Api {\n static int removed() {\n return 1;\n }\n}\n",
        )?;
        fs::write(
            baseline.join("app/App.java"),
            "class App {\n static int caller() {\n return Api.removed();\n }\n}\n",
        )?;
        let owners = BTreeMap::from([("app".into(), json!({"id":"application:baseline"}))]);
        let snapshot = index(baseline, "baseline", owners, &mut vec![])?;
        let removed = snapshot
            .functions
            .values()
            .find(|n| n.name.to_string() == "removed")
            .context("Removed baseline function")?;
        let caller = snapshot
            .functions
            .values()
            .find(|n| n.name.to_string() == "caller")
            .context("Baseline caller")?;
        let engine = BlastRadiusEngine::build_scoped(&snapshot.store, &[removed.id])?;
        assert!(
            engine
                .analyze(removed.id)?
                .impact_zone_ids
                .contains(&caller.id)
        );
        let block = snapshot.block(caller)?;
        assert!(
            block["text"]
                .as_str()
                .context("Source text")?
                .contains("Api.removed()")
        );
        assert_eq!(block["startLine"], 2);
        assert_eq!(block["endLine"], 4);
        Ok(())
    }
    #[test]
    fn hashes_detect_body_edits_and_line_shifts_with_overload_diagnostics() -> Result<()> {
        let root = tempfile::tempdir()?;
        let owners = BTreeMap::from([("dependencies/library".into(), json!({"id":"test-source"}))]);
        let write = |name: &str, source: &str| -> Result<PathBuf> {
            let dir = root.path().join(name);
            fs::create_dir_all(dir.join("dependencies/library"))?;
            let source = source
                .replace("class Library { ", "class Library {\n")
                .replace(" { return ", " {\n return ")
                .replace("; }", ";\n }")
                .replace("} int", "}\n int");
            fs::write(dir.join("dependencies/library/Library.java"), source)?;
            Ok(dir)
        };
        let baseline = write(
            "baseline",
            "class Library { @Deprecated\n int quote(int x) { return x + 1; } @Deprecated\n int quote(String x) { return 0; } int body(int x) { return x + 1; } int removed() { return 1; } }",
        )?;
        let shifted = write(
            "shifted",
            "\n\nclass Library { @Deprecated\n int quote(int x) { return x + 1; } @Deprecated\n int quote(String x) { return 0; } int body(int x) { return x + 1; } int removed() { return 1; } }",
        )?;
        let candidate = write(
            "candidate",
            "class Library { @Deprecated\n int quote(int x) { return x + 1; } @Deprecated\n int quote(String x) { return 1; } int body(int x) { return x + 2; } int added() { return 1; } }",
        )?;
        let mut diagnostics = vec![];
        let old = index(baseline, "baseline", owners.clone(), &mut diagnostics)?;
        let duplicate = root.path().join("different-checkout");
        fs::create_dir_all(duplicate.join("dependencies/library"))?;
        fs::copy(
            old.root.join("dependencies/library/Library.java"),
            duplicate.join("dependencies/library/Library.java"),
        )?;
        let identical = index(duplicate, "identical", owners.clone(), &mut diagnostics)?;
        let mut identical_diff = rgctl_graph::snapshot_diff::VecDiffSink::default();
        let stats = rgctl_graph::snapshot_diff::diff_snapshots(
            &old.store,
            &identical.store,
            &mut identical_diff,
        )?;
        assert_eq!(
            stats,
            rgctl_graph::snapshot_diff::DiffStats::default(),
            "Identical source under different physical roots must have zero graph deltas"
        );
        let shifted = index(shifted, "shifted", owners.clone(), &mut diagnostics)?;
        let new = index(candidate, "candidate", owners, &mut diagnostics)?;
        assert_eq!(old.functions.len(), 4, "Overloads must remain distinct");
        assert!(
            diagnostics
                .iter()
                .all(|d| d["code"] != "AMBIGUOUS_OVERLOAD")
        );
        assert!(
            old.functions
                .values()
                .all(|n| Path::new(n.file_path.as_deref().unwrap()).is_relative())
        );
        assert!(
            function_deltas(&old, &shifted)?.is_empty(),
            "Line shifts must not become code changes"
        );
        let native = function_deltas(&old, &new)?;
        assert_eq!(native.iter().filter(|d| d.kind == "modified").count(), 2);
        assert_eq!(native.iter().filter(|d| d.kind == "removed").count(), 1);
        assert_eq!(native.iter().filter(|d| d.kind == "added").count(), 1);
        for (key, node) in &old.functions {
            assert_eq!(
                node.id, shifted.functions[key].id,
                "Physical roots and line shifts must not affect node IDs"
            );
            assert_eq!(
                node.code_hash, shifted.functions[key].code_hash,
                "Line shifts must not change body hashes"
            );
        }
        let modified: Vec<_> = old
            .functions
            .iter()
            .filter(|(k, n)| {
                new.functions
                    .get(*k)
                    .is_some_and(|next| next.code_hash != n.code_hash)
            })
            .collect();
        assert_eq!(
            modified.len(),
            2,
            "Both the body edit and the distinct overload edit must be detected"
        );
        assert!(modified.iter().any(|(_, n)| n.name.to_string() == "body"));
        assert!(modified.iter().any(|(_, n)| n.name.to_string() == "quote"));
        assert_eq!(
            old.functions
                .keys()
                .filter(|k| !new.functions.contains_key(*k))
                .count(),
            1
        );
        assert_eq!(
            new.functions
                .keys()
                .filter(|k| !old.functions.contains_key(*k))
                .count(),
            1
        );
        Ok(())
    }
    #[test]
    fn relative_snapshots_read_sources_and_preserve_qualifier_filtering() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::create_dir_all(dir.path().join("app"))?;
        fs::write(
            dir.path().join("app/App.java"),
            r#"
import org.springframework.stereotype.Component;
import org.springframework.beans.factory.annotation.*;
interface X { int execute(); }
@Component("z") class Z implements X { public int execute(){return 100;} }
@Component("w") class W implements X { public int execute(){return 200;} }
class App {
 @Autowired @Qualifier("w") X service;
 int run(){ return service.execute(); }
}
"#,
        )?;
        let snapshot = index(
            dir.path().to_owned(),
            "baseline",
            BTreeMap::from([("app".into(), json!({"id":"application"}))]),
            &mut vec![],
        )?;
        let get = |name: &str| {
            snapshot
                .functions
                .values()
                .find(|n| n.qualified_name.as_deref() == Some(name))
                .unwrap()
        };
        let graph =
            crate::dispatch::DispatchGraph::build_with_root(&snapshot.store, &snapshot.root)?;
        assert!(
            !graph
                .possible
                .contains_key(&(get("App.run").id, get("Z.execute").id))
        );
        assert!(
            graph
                .possible
                .contains_key(&(get("App.run").id, get("W.execute").id))
        );
        assert!(
            snapshot.block(get("App.run"))?["text"]
                .as_str()
                .unwrap()
                .contains("service.execute()")
        );
        Ok(())
    }
}
