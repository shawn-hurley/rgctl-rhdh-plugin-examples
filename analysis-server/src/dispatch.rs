//! Conservative runtime-dispatch overlay. Persisted rgctl facts remain unchanged.
use anyhow::{Context, Result};
use rgctl_analysis::BlastRadiusEngine;
use rgctl_graph::{
    SnapshotNodeStore,
    backend::{GraphBackend, MemoryBackend},
    schema::{Edge, EdgeType, Node, NodeType},
};
use rgctl_plugin_api::{LanguagePlugin, Relation, RelationType};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs,
};
use uuid::Uuid;
#[path = "joins.rs"]
mod joins;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PossibleDispatch {
    pub declared_target: Uuid,
    pub declared_qualified_name: String,
    pub implementation_qualified_name: String,
    pub reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join_evidence: Option<serde_json::Value>,
}

pub struct DispatchGraph {
    pub incoming: HashMap<Uuid, Vec<Uuid>>,
    pub possible: HashMap<(Uuid, Uuid), PossibleDispatch>,
    pub unresolved: usize,
    pub origin_coverage: serde_json::Value,
    context: Option<joins::ContextDispatch>,
    outgoing: HashMap<Uuid, Vec<Uuid>>,
}

fn qualified(n: &Node) -> &str {
    n.qualified_name.as_deref().unwrap_or(&n.name)
}
fn literal(arguments: &str) -> Option<String> {
    let text = arguments
        .trim()
        .strip_prefix('(')?
        .strip_suffix(')')?
        .trim();
    let text = text
        .strip_prefix("value")
        .map(|v| v.trim().strip_prefix('=').unwrap_or("").trim())
        .unwrap_or(text);
    let text = text.strip_prefix('"')?.strip_suffix('"')?;
    if text.is_empty() || text.contains(['"', '\\']) {
        None
    } else {
        Some(text.to_owned())
    }
}
fn spring_annotation(r: &Relation, source: &str, simple: &str, package: &str) -> bool {
    if r.to != simple {
        return false;
    }
    let fqn = format!("{package}.{simple}");
    if r.to_qualified_hint.as_deref() == Some(&fqn) {
        return true;
    }
    // rgctl currently falls back to the source package for wildcard imports.
    // Only recognize a wildcard when nothing in this file can shadow its name.
    source.contains(&format!("import {package}.*;"))
        && !source.contains(&format!("@interface {simple}"))
        && !source.lines().any(|line| {
            line.trim().starts_with("import ")
                && line.trim().ends_with(&format!(".{simple};"))
                && !line.contains(&fqn)
        })
}

// Narrow only direct field receivers whose binding can be established. Parameters,
// local aliases, setters, custom qualifiers and factories keep the broader fallback.
fn receiver_field(tree: &tree_sitter::Tree, source: &[u8], r: &Relation) -> Option<String> {
    let point = tree_sitter::Point::new(
        r.location.start_line.checked_sub(1)?,
        r.location.start_column,
    );
    let mut call = tree.root_node().descendant_for_point_range(point, point)?;
    while call.kind() != "method_invocation" {
        call = call.parent()?;
    }
    let object = call.child_by_field_name("object")?;
    let (field, explicit) = match object.kind() {
        "identifier" => (object.utf8_text(source).ok()?.to_owned(), false),
        "field_access" if object.child_by_field_name("object")?.kind() == "this" => (
            object
                .child_by_field_name("field")?
                .utf8_text(source)
                .ok()?
                .to_owned(),
            true,
        ),
        _ => return None,
    };
    if !explicit {
        let mut method = call;
        while !matches!(
            method.kind(),
            "method_declaration" | "constructor_declaration"
        ) {
            method = method.parent()?;
        }
        let mut stack = vec![method];
        while let Some(n) = stack.pop() {
            if matches!(
                n.kind(),
                "formal_parameter"
                    | "spread_parameter"
                    | "variable_declarator"
                    | "catch_formal_parameter"
                    | "inferred_parameters"
            ) && n
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                == Some(&field)
            {
                return None;
            }
            if n.kind() == "lambda_expression" {
                if let Some(parameters) = n.child_by_field_name("parameters") {
                    let mut pending = vec![parameters];
                    while let Some(p) = pending.pop() {
                        if p.kind() == "identifier" && p.utf8_text(source).ok() == Some(&field) {
                            return None;
                        }
                        let mut c = p.walk();
                        pending.extend(p.named_children(&mut c));
                    }
                }
            }
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c));
        }
    }
    Some(field)
}

#[derive(Default)]
struct Bindings {
    names: HashMap<String, HashSet<String>>,
    // File, callable name/discriminator, declared target -> qualifier for each call.
    calls: HashMap<(String, String, String, String), Vec<Option<String>>>,
    has_factories: bool,
}
impl Bindings {
    fn load(nodes: &HashMap<Uuid, Node>, root: &std::path::Path) -> Result<Self> {
        let plugin = rgctl_lang_java::JavaPlugin::new()?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&plugin.grammar().context("Java grammar")?)?;
        let files: std::collections::BTreeSet<_> = nodes
            .values()
            .filter_map(|n| n.file_path.as_deref())
            .filter(|p| p.ends_with(".java"))
            .collect();
        let mut bindings = Self::default();
        for file in files {
            let physical = root.join(file);
            let bytes = fs::read(&physical)?;
            let source = std::str::from_utf8(&bytes)?;
            let facts = plugin.extract_all(&physical, &bytes)?;
            let mut qualifiers = HashMap::new();
            for r in &facts.relations {
                if r.relation_type != RelationType::AnnotatedWith {
                    continue;
                }
                if r.to == "Bean" {
                    bindings.has_factories = true;
                }
                let args = r
                    .metadata
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .and_then(literal);
                if spring_annotation(
                    r,
                    source,
                    "Qualifier",
                    "org.springframework.beans.factory.annotation",
                ) {
                    if let Some(value) = args.clone() {
                        qualifiers.insert(r.from.clone(), value);
                    }
                }
                if ["Component", "Service", "Repository"].iter().any(|simple| {
                    spring_annotation(r, source, simple, "org.springframework.stereotype")
                }) {
                    // Only explicit, literal bean names establish an exclusion.
                    if let Some(value) = args {
                        bindings
                            .names
                            .entry(r.from.clone())
                            .or_default()
                            .insert(value);
                    }
                }
            }
            // Component qualifiers can be aliases in addition to the bean name.
            // Unknown class annotations may be custom qualifiers, so do not use
            // that class's name to exclude a possible binding.
            for r in facts
                .relations
                .iter()
                .filter(|r| r.relation_type == RelationType::AnnotatedWith)
            {
                if !bindings.names.contains_key(&r.from) {
                    continue;
                }
                if let Some(alias) = qualifiers.get(&r.from) {
                    bindings
                        .names
                        .get_mut(&r.from)
                        .unwrap()
                        .insert(alias.clone());
                }
                if ![
                    "Component",
                    "Service",
                    "Repository",
                    "Qualifier",
                    "Primary",
                    "Profile",
                    "Lazy",
                    "Scope",
                    "Deprecated",
                ]
                .contains(&r.to.as_str())
                    || (r.to == "Qualifier" && !qualifiers.contains_key(&r.from))
                {
                    bindings.names.remove(&r.from);
                }
            }
            let tree = parser
                .parse(&bytes, None)
                .context("Parse Java receiver context")?;
            for r in facts
                .relations
                .iter()
                .filter(|r| r.relation_type == RelationType::Calls)
            {
                let Some(target) = r.to_qualified_hint.as_ref() else {
                    continue;
                };
                let Some((owner, _)) = r.from.rsplit_once('.') else {
                    continue;
                };
                let value = receiver_field(&tree, &bytes, r)
                    .and_then(|field| qualifiers.get(&format!("{owner}.{field}")).cloned());
                let discriminator = r
                    .metadata
                    .get("from_identity_discriminator")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                bindings
                    .calls
                    .entry((
                        file.to_owned(),
                        r.from.clone(),
                        discriminator,
                        target.clone(),
                    ))
                    .or_default()
                    .push(value);
            }
        }
        Ok(bindings)
    }
    fn allows(&self, caller: &Node, declared: &Node, implementation: &Node) -> bool {
        if self.has_factories {
            return true;
        }
        let Some((owner, _)) = qualified(implementation).rsplit_once('.') else {
            return true;
        };
        let Some(name) = self.names.get(owner) else {
            return true;
        };
        let key = (
            caller.file_path.as_deref().unwrap_or("").to_owned(),
            qualified(caller).to_owned(),
            caller.identity_discriminator().unwrap_or("").to_owned(),
            qualified(declared).to_owned(),
        );
        let Some(calls) = self.calls.get(&key) else {
            return true;
        };
        // An unqualified call or any matching injection keeps this implementation.
        calls.is_empty()
            || calls
                .iter()
                .any(|q| q.as_ref().is_none_or(|q| name.contains(q)))
    }
}

impl DispatchGraph {
    #[allow(dead_code)]
    pub fn build(store: &SnapshotNodeStore) -> Result<Self> {
        Self::build_with_root(store, std::path::Path::new("."))
    }
    pub fn build_with_root(store: &SnapshotNodeStore, root: &std::path::Path) -> Result<Self> {
        Self::build_internal(store, root, true)
    }
    #[allow(dead_code)]
    pub fn build_without_joins(store: &SnapshotNodeStore, root: &std::path::Path) -> Result<Self> {
        Self::build_internal(store, root, false)
    }
    fn build_internal(
        store: &SnapshotNodeStore,
        root: &std::path::Path,
        enable_joins: bool,
    ) -> Result<Self> {
        let mut nodes = HashMap::new();
        store.for_each_node_seq(|n| {
            nodes.insert(n.id, n.clone());
        })?;
        let bindings = Bindings::load(&nodes, root)?;
        let mut calls = vec![];
        let mut instantiations = HashSet::new();
        let mut overrides: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        let mut parents: HashMap<String, Vec<String>> = HashMap::new();
        store.for_each_edge(|from, to, kind| {
            match kind {
                EdgeType::Calls => calls.push((from, to)),
                EdgeType::Instantiates => {
                    instantiations.insert((from, to));
                }
                EdgeType::Overrides => overrides.entry(to).or_default().push(from),
                EdgeType::Implements | EdgeType::Extends | EdgeType::InheritsClass => {
                    if let (Some(child), Some(parent)) = (nodes.get(&from), nodes.get(&to)) {
                        parents
                            .entry(qualified(child).into())
                            .or_default()
                            .push(qualified(parent).into());
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        // Some overloaded implementations lack explicit Overrides edges upstream.
        let native_calls: HashSet<_> = calls.iter().copied().collect();
        let mut origin_coverage = serde_json::json!({"status":"not-run"});
        let mut receiver_refinements = joins::ReceiverRefinements::new();
        let mut context = None;
        let joined = if enable_joins {
            let analysis =
                joins::candidate_calls(&nodes, root, &parents, &native_calls, &instantiations)?;
            origin_coverage = analysis.origin_coverage;
            receiver_refinements = analysis.receiver_refinements;
            context = analysis.context;
            analysis
                .candidates
                .into_iter()
                .filter(|c| {
                    let declared = c.declared_target.unwrap_or(c.to);
                    match (nodes.get(&c.from), nodes.get(&declared), nodes.get(&c.to)) {
                        (Some(caller), Some(declared), Some(target)) => {
                            receiver_refinements
                                .get(&(c.from, target.name.to_string()))
                                .is_none_or(|allowed| allowed.contains(&c.to))
                                && bindings.allows(caller, declared, target)
                        }
                        _ => false,
                    }
                })
                .collect()
        } else {
            vec![]
        };
        calls.extend(joined.iter().map(|c| (c.from, c.to)));
        // Derive compatible methods from type ancestry AND canonical parameter
        // identity; a matching method name alone never establishes a dispatch.
        let mut ancestry: HashMap<String, HashSet<String>> = HashMap::new();
        for owner in parents.keys() {
            let mut seen = HashSet::new();
            let mut queue = vec![owner.clone()];
            while let Some(name) = queue.pop() {
                if seen.insert(name.clone()) {
                    queue.extend(parents.get(&name).into_iter().flatten().cloned());
                }
            }
            ancestry.insert(owner.clone(), seen);
        }
        let interfaces: HashSet<_> = nodes
            .values()
            .filter(|n| n.node_type == NodeType::Interface)
            .map(|n| qualified(n).to_owned())
            .collect();
        let mut methods: HashMap<(&str, &str), Vec<&Node>> = HashMap::new();
        for n in nodes.values().filter(|n| n.node_type == NodeType::Function) {
            if let Some(identity) = n.identity_discriminator() {
                methods.entry((&n.name, identity)).or_default().push(n);
            }
        }
        for declared in calls.iter().filter_map(|(_, to)| nodes.get(to)) {
            let Some((owner, _)) = qualified(declared).rsplit_once('.') else {
                continue;
            };
            let Some(identity) = declared.identity_discriminator() else {
                continue;
            };
            for candidate in methods
                .get(&(&*declared.name, identity))
                .into_iter()
                .flatten()
            {
                let Some((candidate_owner, _)) = qualified(candidate).rsplit_once('.') else {
                    continue;
                };
                if candidate_owner == owner || interfaces.contains(candidate_owner) {
                    continue;
                }
                let compatible = ancestry
                    .get(candidate_owner)
                    .is_some_and(|a| a.contains(owner))
                    || (interfaces.contains(owner)
                        && ancestry
                            .values()
                            .any(|a| a.contains(owner) && a.contains(candidate_owner)));
                if compatible {
                    overrides.entry(declared.id).or_default().push(candidate.id);
                }
            }
        }
        for methods in overrides.values_mut() {
            methods.sort();
            methods.dedup();
        }
        calls.sort();
        calls.dedup();
        let original: HashSet<_> = calls.iter().copied().collect();
        let mut possible = HashMap::new();
        for candidate in joined {
            if native_calls.contains(&(candidate.from, candidate.to)) {
                continue;
            }
            if let Some(target) = nodes.get(&candidate.to) {
                possible
                    .entry((candidate.from, candidate.to))
                    .or_insert(PossibleDispatch {
                        declared_target: candidate.declared_target.unwrap_or(candidate.to),
                        declared_qualified_name: nodes
                            .get(&candidate.declared_target.unwrap_or(candidate.to))
                            .map(qualified)
                            .unwrap_or_else(|| qualified(target))
                            .into(),
                        implementation_qualified_name: qualified(target).into(),
                        reason: candidate.reason,
                        join_evidence: Some(candidate.evidence),
                    });
            }
        }
        let mut unresolved = 0;
        for (caller, declared) in &calls {
            let Some(declared_node) = nodes.get(declared) else {
                continue;
            };
            let Some(caller_node) = nodes.get(caller) else {
                continue;
            };
            let mut queue = VecDeque::from([*declared]);
            let mut seen = HashSet::from([*declared]);
            while let Some(parent) = queue.pop_front() {
                for implementation in overrides.get(&parent).into_iter().flatten() {
                    if !seen.insert(*implementation) {
                        continue;
                    }
                    queue.push_back(*implementation);
                    let Some(n) = nodes.get(implementation) else {
                        continue;
                    };
                    // Preserve exact callable identity while expanding transitive overrides.
                    if n.node_type != NodeType::Function
                        || (declared_node.get_property("language") == Some("java")
                            && n.identity_discriminator() != declared_node.identity_discriminator())
                        || receiver_refinements
                            .get(&(*caller, n.name.to_string()))
                            .is_some_and(|allowed| !allowed.contains(implementation))
                        || !bindings.allows(caller_node, declared_node, n)
                    {
                        continue;
                    }
                    if !original.contains(&(*caller, *implementation)) {
                        possible
                            .entry((*caller, *implementation))
                            .or_insert(PossibleDispatch {
                                declared_target: *declared,
                                declared_qualified_name: qualified(declared_node).into(),
                                implementation_qualified_name: qualified(n).into(),
                                reason: "compatible-override-across-configurations",
                                join_evidence: None,
                            });
                    }
                }
            }
            if seen.len() == 1 {
                let owner = qualified(declared_node).rsplit_once('.').map(|(o, _)| o);
                if owner.is_some_and(|o| interfaces.contains(o)) {
                    unresolved += 1;
                }
            }
        }
        calls.extend(possible.keys().copied());
        let mut incoming: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for (from, to) in calls {
            incoming.entry(to).or_default().push(from);
        }
        for list in incoming.values_mut() {
            list.sort();
            list.dedup();
        }
        let mut outgoing: HashMap<_, Vec<_>> = HashMap::new();
        for (&to, from) in &incoming {
            for &from in from {
                outgoing.entry(from).or_default().push(to);
            }
        }
        for values in outgoing.values_mut() {
            values.sort();
            values.dedup();
        }
        Ok(Self {
            incoming,
            context,
            outgoing,
            possible,
            unresolved,
            origin_coverage,
        })
    }
    /// Keep rgctl's SCC/blast engine, hydrating only the augmented upstream closure.
    pub fn engine(&self, store: &SnapshotNodeStore, seeds: &[Uuid]) -> Result<BlastRadiusEngine> {
        Ok(self.projection(store, seeds)?.0)
    }
    /// Use exactly these retained edges for source-path reconstruction too.
    pub fn projection(
        &self,
        store: &SnapshotNodeStore,
        seeds: &[Uuid],
    ) -> Result<(BlastRadiusEngine, HashMap<Uuid, Vec<Uuid>>)> {
        let mut retained: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        let mut graph = MemoryBackend::new();
        let seed_set: HashSet<_> = seeds.iter().copied().collect();
        let mut context_budget = 500_000usize;
        let mut queue: VecDeque<_> = seeds.iter().copied().collect();
        let mut seen = HashSet::new();
        let mut edges = vec![];
        while let Some(to) = queue.pop_front() {
            if !seen.insert(to) {
                continue;
            }
            if let Some(n) = store.get_node(to)? {
                graph.insert_node(n)?;
            }
            for from in self.incoming.get(&to).into_iter().flatten() {
                if self.context.as_ref().is_some_and(|c| {
                    c.excludes(*from, to, &seed_set, &self.outgoing, &mut context_budget)
                }) {
                    continue;
                }
                retained.entry(to).or_default().push(*from);
                edges.push(Edge::new(*from, to, EdgeType::Calls));
                queue.push_back(*from);
            }
        }
        graph.insert_edges_batch(edges)?;
        Ok((BlastRadiusEngine::build(&graph)?, retained))
    }
    pub fn uses_context(&self) -> bool {
        self.context.is_some()
    }
    pub fn evidence(&self) -> Vec<serde_json::Value> {
        let mut edges: BTreeMap<_, _> = BTreeMap::new();
        for ((from, to), reason) in &self.possible {
            edges.insert(
                (*from, *to),
                serde_json::json!({"from":from,"to":to,"type":"PossibleDispatch","binding":reason}),
            );
        }
        edges.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgctl_pipeline::{PipelineConfig, ProcessingPipeline};
    use rgctl_registry::LanguageRegistry;
    use std::sync::Arc;

    fn fixture(source: &str) -> Result<(tempfile::TempDir, SnapshotNodeStore)> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("Example.java"), source)?;
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
        let snapshot = dir.path().join(".rgctl/graph.snapshot.bin");
        fs::create_dir_all(snapshot.parent().unwrap())?;
        pipeline.process_repository_to_snapshot(dir.path(), &snapshot, Some(dir.path()))?;
        Ok((dir, SnapshotNodeStore::open(&snapshot)?))
    }
    fn id(store: &SnapshotNodeStore, name: &str) -> Uuid {
        let mut id = None;
        store
            .for_each_node_of_type(NodeType::Function, |n| {
                if qualified(n) == name {
                    id = Some(n.id);
                }
            })
            .unwrap();
        id.unwrap_or_else(|| panic!("Missing {name}"))
    }
    fn language_fixture(
        language: &str,
        file: &str,
        source: &str,
    ) -> Result<(tempfile::TempDir, SnapshotNodeStore)> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join(file), source)?;
        let mut registry = LanguageRegistry::new();
        match language {
            "javascript" => rgctl_lang_javascript::register(&mut registry),
            "typescript" => rgctl_lang_typescript::register(&mut registry),
            "go" => rgctl_lang_go::register(&mut registry),
            "python" => rgctl_lang_python::register(&mut registry),
            _ => unreachable!(),
        }
        let pipeline = ProcessingPipeline::with_config(
            Arc::new(registry),
            PipelineConfig {
                show_progress: false,
                facts_cache: false,
                thread_count: Some(2),
                ..Default::default()
            },
        );
        let snapshot = dir.path().join(".rgctl/graph.snapshot.bin");
        fs::create_dir_all(snapshot.parent().unwrap())?;
        pipeline.process_repository_to_snapshot(dir.path(), &snapshot, Some(dir.path()))?;
        Ok((dir, SnapshotNodeStore::open(&snapshot)?))
    }
    #[test]
    fn stopgap_receiver_warning_is_explicit_and_type_bounded() -> Result<()> {
        let (dir, store) = language_fixture(
            "javascript",
            "app.js",
            r#"
class X { execute() { return 100; } }
class W { execute() { return 200; } }
class Unrelated { execute() { return 300; } }
function exercise() { const unused = new X(); const w = new W(); return w.execute(); }
"#,
        )?;
        let graph = DispatchGraph::build_with_root(&store, dir.path())?;
        let caller = id(&store, "app.exercise");
        let x = id(&store, "X.execute");
        let unrelated = id(&store, "Unrelated.execute");
        assert!(
            graph
                .engine(&store, &[x])?
                .analyze(x)?
                .impact_zone_ids
                .contains(&caller)
        );
        assert!(!graph.possible.contains_key(&(caller, unrelated)));
        let evidence = graph.possible[&(caller, x)].join_evidence.as_ref().unwrap();
        assert_eq!(evidence["receiverBinding"], "not-established");
        Ok(())
    }
    #[test]
    fn stopgap_consumes_native_go_variable_type() -> Result<()> {
        let (dir, store) = language_fixture(
            "go",
            "app.go",
            r#"package sample
type Y interface { Execute() int }
type X struct{}
func (x X) Execute() int { return 100 }
func exercise() int { var value Y = X{}; return value.Execute() }
"#,
        )?;
        let graph = DispatchGraph::build_with_root(&store, dir.path())?;
        let changed = id(&store, "X.Execute");
        let caller = id(&store, "sample.exercise");
        assert!(
            graph
                .engine(&store, &[changed])?
                .analyze(changed)?
                .impact_zone_ids
                .contains(&caller)
        );
        Ok(())
    }
    const TYPES: &str = r#"
package sample;
import org.springframework.stereotype.Component;
import org.springframework.beans.factory.annotation.*;
interface X { int execute(); }
@Component("z")
class Z implements X {
 public int execute() { return internal(); }
 int internal() { return 100; }
}
@Component("w")
class W implements X {
 public int execute() { return 200; }
}
"#;
    fn suffix_id(store: &SnapshotNodeStore, suffix: &str) -> Uuid {
        let mut found = Vec::new();
        store
            .for_each_node_of_type(NodeType::Function, |n| {
                if qualified(n).ends_with(suffix) {
                    found.push(n.id);
                }
            })
            .unwrap();
        assert_eq!(found.len(), 1, "ambiguous/missing {suffix}");
        found[0]
    }
    #[test]
    fn complete_argument_and_factory_paths_exclude_siblings_in_three_languages() -> Result<()> {
        let cases = [
            (
                "java",
                "Example.java",
                "package sample; interface Y{int execute();} class X implements Y{public int execute(){return 100;}} class W implements Y{public int execute(){return 200;}} class App{static int consume(Y v){return v.execute();} static <T extends Y>T forward(T v){return v;} static Y make(){return new W();} static Y conditional(){return Boolean.getBoolean(\"flag\")?new W():new W();} public static int exercise(){consume(new W());forward(new W()).execute();make().execute();return conditional().execute();}}",
                "X.execute",
            ),
            (
                "typescript",
                "app.ts",
                "interface Y{execute():number;} class X implements Y{execute(){return 100;}} class W implements Y{execute(){return 200;}} function consume(v:Y){return v.execute();} function forward<T extends Y>(v:T):T{return v;} function make():Y{return new W();} function conditional():Y{return globalThis.flag?new W():new W();} export function exercise(){consume(new W());forward(new W()).execute();make().execute();return conditional().execute();}",
                "X.execute",
            ),
            (
                "go",
                "app.go",
                "package sample\ntype Y interface{Execute()int}\ntype X struct{}\nfunc(x X)Execute()int{return 100}\ntype W struct{}\nfunc(w W)Execute()int{return 200}\nfunc consume(v Y)int{return v.Execute()}\nfunc forward[T Y](v T)T{return v}\nfunc makeValue()Y{return W{}}\nfunc conditional(flag bool)Y{if flag{return W{}};return W{}}\nfunc exercise()int{consume(W{});forward(W{}).Execute();makeValue().Execute();return conditional(true).Execute()}",
                "X.Execute",
            ),
        ];
        for (language, file, source, changed) in cases {
            let (dir, store) = if language == "java" {
                fixture(source)?
            } else {
                language_fixture(language, file, source)?
            };
            let graph = DispatchGraph::build_with_root(&store, dir.path())?;
            let caller = suffix_id(&store, ".exercise");
            let x = suffix_id(&store, changed);
            let w = suffix_id(&store, &changed.replace("X.", "W."));
            assert!(
                !graph
                    .engine(&store, &[x])?
                    .analyze(x)?
                    .impact_zone_ids
                    .contains(&caller),
                "{language}: X remained reachable"
            );
            assert!(
                graph
                    .engine(&store, &[w])?
                    .analyze(w)?
                    .impact_zone_ids
                    .contains(&caller),
                "{language}: W was lost"
            );
        }
        Ok(())
    }
    #[test]
    fn shared_argument_contexts_keep_only_matching_implementations() -> Result<()> {
        let (dir, store) = fixture(
            "package sample; interface Y{int execute();} class X implements Y{public int execute(){return 100;}} class W implements Y{public int execute(){return 200;}} class App{static int consume(Y value){return value.execute();} public static int exerciseW(){return consume(new W());} public static int exerciseX(){return consume(new X());}}",
        )?;
        let graph = DispatchGraph::build_with_root(&store, dir.path())?;
        for implementation in ["X.execute", "W.execute"] {
            let changed = suffix_id(&store, implementation);
            let (engine, retained) = graph.projection(&store, &[changed])?;
            let impact = engine.analyze(changed)?;
            let consume = suffix_id(&store, ".consume");
            for (caller, matching) in [
                (".exerciseW", implementation == "W.execute"),
                (".exerciseX", implementation == "X.execute"),
            ] {
                assert_eq!(
                    impact.impact_zone_ids.contains(&suffix_id(&store, caller)),
                    matching,
                    "{implementation}: {caller}"
                );
                assert_eq!(
                    retained
                        .get(&consume)
                        .is_some_and(|v| v.contains(&suffix_id(&store, caller))),
                    matching,
                    "path edges must agree with caller impact: {implementation}: {caller}"
                );
            }
            assert!(
                impact
                    .impact_zone_ids
                    .contains(&suffix_id(&store, ".consume"))
            );
        }
        let consume = suffix_id(&store, ".consume");
        let impact = graph.engine(&store, &[consume])?.analyze(consume)?;
        for caller in [".exerciseW", ".exerciseX"] {
            assert!(impact.impact_zone_ids.contains(&suffix_id(&store, caller)));
        }
        Ok(())
    }
    #[test]
    fn factory_side_effects_survive_return_receiver_refinement() -> Result<()> {
        let (dir, store) = fixture(
            "package sample; interface Y{int execute();} class X implements Y{public int execute(){return 100;}} class W implements Y{public int execute(){return 200;}} class App{static Y make(){new X().execute();return new W();} public static int exercise(){return make().execute();}}",
        )?;
        let graph = DispatchGraph::build_with_root(&store, dir.path())?;
        let changed = suffix_id(&store, "X.execute");
        assert!(
            graph
                .engine(&store, &[changed])?
                .analyze(changed)?
                .impact_zone_ids
                .contains(&suffix_id(&store, ".exercise"))
        );
        Ok(())
    }
    fn assert_stored_impact(language: &str, source: &str, expected: bool) -> Result<()> {
        let (dir, store) = match language {
            "java" => fixture(source)?,
            "typescript" => language_fixture(language, "app.ts", source)?,
            "go" => language_fixture(language, "app.go", source)?,
            _ => unreachable!(),
        };
        let graph = DispatchGraph::build_with_root(&store, dir.path())?;
        let changed = suffix_id(
            &store,
            if language == "go" {
                "X.Execute"
            } else {
                "X.execute"
            },
        );
        let reached = graph
            .engine(&store, &[changed])?
            .analyze(changed)?
            .impact_zone_ids
            .contains(&suffix_id(&store, ".exercise"));
        assert_eq!(
            reached, expected,
            "{language}: {source}\n{}",
            graph.origin_coverage["storedReceiverRefinement"]
        );
        Ok(())
    }
    const STORED_JAVA: &str = "package sample; interface Y{int execute();} class X implements Y{public int execute(){return 100;}} class W implements Y{public int execute(){return 200;}}";
    const STORED_TS: &str = "interface Y{execute():number;} class X implements Y{execute(){return 100;}} class W implements Y{execute(){return 200;}}";
    const STORED_GO: &str = "package sample\ntype Y interface{Execute()int}\ntype X struct{}\nfunc(x X)Execute()int{return 100}\ntype W struct{}\nfunc(w W)Execute()int{return 200}\n";
    #[test]
    fn stored_constructor_setter_and_wrapper_controls_exclude_x() -> Result<()> {
        for suffix in [
            "class Consumer{final Y value;Consumer(Y v){this.value=v;}int run(){return value.execute();}} class App{public static int exercise(){return new Consumer(new W()).run();}}",
            "class Consumer{Y value;void set(Y v){this.value=v;}int run(){return value.execute();}} class App{public static int exercise(){Consumer c=new Consumer();c.set(new W());return c.run();}}",
            "class Wrapper implements Y{final Y inner;Wrapper(Y v){this.inner=v;}public int execute(){return inner.execute();}} class App{public static int exercise(){Y value=new Wrapper(new W());return value.execute();}}",
        ] {
            assert_stored_impact("java", &format!("{STORED_JAVA}{suffix}"), false)?;
        }
        for suffix in [
            "class Consumer{inner:Y;constructor(v:Y){this.inner=v;}run(){return this.inner.execute();}} export function exercise(){return new Consumer(new W()).run();}",
            "class Consumer{inner!:Y;set(v:Y){this.inner=v;}run(){return this.inner.execute();}} export function exercise(){const c=new Consumer();c.set(new W());return c.run();}",
            "class Wrapper implements Y{inner:Y;constructor(v:Y){this.inner=v;}execute(){return this.inner.execute();}} export function exercise(){const value:Y=new Wrapper(new W());return value.execute();}",
        ] {
            assert_stored_impact("typescript", &format!("{STORED_TS}{suffix}"), false)?;
        }
        for suffix in [
            "type Consumer struct{value Y}\nfunc NewConsumer(v Y)*Consumer{return &Consumer{value:v}}\nfunc(c *Consumer)Run()int{return c.value.Execute()}\nfunc exercise()int{return NewConsumer(W{}).Run()}",
            "type Consumer struct{value Y}\nfunc(c *Consumer)Set(v Y){c.value=v}\nfunc(c *Consumer)Run()int{return c.value.Execute()}\nfunc exercise()int{c:=&Consumer{};c.Set(W{});return c.Run()}",
            "type Wrapper struct{inner Y}\nfunc(w Wrapper)Execute()int{return w.inner.Execute()}\nfunc exercise()int{var value Y=Wrapper{inner:W{}};return value.Execute()}",
        ] {
            assert_stored_impact("go", &format!("{STORED_GO}{suffix}"), false)?;
        }
        Ok(())
    }
    #[test]
    fn alias_conditional_and_other_instance_writes_keep_x() -> Result<()> {
        let java = "class Consumer{Y value;void set(Y v){this.value=v;}int run(){return value.execute();}}";
        for body in [
            "Consumer c=new Consumer();c.set(new W());Consumer alias=c;alias.set(new X());return c.run();",
            "Consumer c=new Consumer();c.set(flag?new W():new X());return c.run();",
            "Consumer c=new Consumer();c.set(new W());Consumer other=new Consumer();other.set(new X());return c.run();",
            "Consumer c=new Consumer();c.set(unknown);return c.run();",
        ] {
            assert_stored_impact(
                "java",
                &format!(
                    "{STORED_JAVA}{java}class App{{public static int exercise(boolean flag,Y unknown){{{body}}}}}"
                ),
                true,
            )?;
        }
        assert_stored_impact(
            "typescript",
            &format!(
                "{STORED_TS}class Consumer{{inner!:Y;set(v:Y){{this.inner=v;}}run(){{return this.inner.execute();}}}} export function exercise(flag:boolean){{const c=new Consumer();c.set(new W());const alias=c;if(flag)alias.set(new X());return c.run();}}"
            ),
            true,
        )?;
        assert_stored_impact(
            "go",
            &format!(
                "{STORED_GO}type Consumer struct{{value Y}}\nfunc(c *Consumer)Set(v Y){{c.value=v}}\nfunc(c *Consumer)Run()int{{return c.value.Execute()}}\nfunc exercise(flag bool)int{{c:=&Consumer{{}};c.Set(W{{}});alias:=c;if flag{{alias.Set(X{{}})}};return c.Run()}}"
            ),
            true,
        )?;
        Ok(())
    }
    #[test]
    fn unresolved_mutators_and_initializer_effects_keep_x() -> Result<()> {
        let prefix = STORED_GO.replacen(
            "package sample\n",
            "package sample\nimport \"example.test/external\"\n",
            1,
        );
        assert_stored_impact(
            "go",
            &format!(
                "{prefix}type Consumer struct{{value Y}}\nfunc(c *Consumer)Set(v Y){{c.value=v}}\nfunc(c *Consumer)Run()int{{return c.value.Execute()}}\nfunc exercise()int{{c:=&Consumer{{}};c.Set(W{{}});setter:=c.Set;external.Dispatch(setter);return c.Run()}}"
            ),
            true,
        )?;
        assert_stored_impact(
            "java",
            &format!(
                "{STORED_JAVA}class App{{public static int exercise(){{return new W(){{public int execute(){{return new X().execute();}}}}.execute();}}}}"
            ),
            true,
        )?;
        assert_stored_impact(
            "java",
            &format!(
                "{STORED_JAVA}class Consumer{{Y value;void set(Y v){{this.value=v;}}int run(){{return value.execute();}}}}class App{{static native void mutate(Consumer c);public static int exercise(){{Consumer c=new Consumer();c.set(new W());mutate(c);return c.run();}}}}"
            ),
            true,
        )?;
        assert_stored_impact(
            "typescript",
            &format!(
                "{STORED_TS}class Consumer{{inner!:Y;set(v:Y){{this.inner=v;}}run(){{return this.inner.execute();}}}} declare function mutate(c:Consumer):void; export function exercise(){{const c=new Consumer();c.set(new W());mutate(c);return c.run();}}"
            ),
            true,
        )?;
        assert_stored_impact(
            "java",
            &format!(
                "{STORED_JAVA}class Wrapper implements Y{{Y inner;Wrapper(Y v){{new X().execute();this.inner=v;}}public int execute(){{return inner.execute();}}}}class App{{public static int exercise(){{Y value=new Wrapper(new W());return value.execute();}}}}"
            ),
            true,
        )?;
        Ok(())
    }
    #[test]
    fn unrelated_field_owners_and_collection_roots_are_not_collided() -> Result<()> {
        assert_stored_impact(
            "java",
            &format!(
                "{STORED_JAVA}class Consumer{{final Y value;Consumer(Y v){{this.value=v;}}int run(){{return value.execute();}}}} class Other{{Y value;Other(Y v){{this.value=v;}}}}class App{{public static int exercise(){{Other unrelated=new Other(new X());return new Consumer(new W()).run();}}}}"
            ),
            false,
        )?;
        assert_stored_impact(
            "typescript",
            &format!(
                "{STORED_TS}export function exercise(){{const values:Y[]=[new W()];const other:Y[]=[new W()];other[0]=new X();return values[0].execute();}}"
            ),
            false,
        )?;
        Ok(())
    }
    #[test]
    fn collection_and_capture_writes_merge_all_producers() -> Result<()> {
        for (body, expected) in [
            (
                "const values:Y[]=[new W()];return values[0].execute();",
                false,
            ),
            (
                "const values:Y[]=[new W()];values[0]=new X();return values[0].execute();",
                true,
            ),
            (
                "const values:Y[]=[new W()];mutate(values);return values[0].execute();",
                true,
            ),
            (
                "let selected:Y=new W();const fn=()=>selected.execute();return fn();",
                false,
            ),
            (
                "let selected:Y=new W();const fn=()=>selected.execute();if(flag)selected=new X();return fn();",
                true,
            ),
        ] {
            assert_stored_impact(
                "typescript",
                &format!(
                    "{STORED_TS}declare function mutate(values:Y[]):void;export function exercise(flag:boolean){{{body}}}"
                ),
                expected,
            )?;
        }
        Ok(())
    }
    #[test]
    fn stored_refinement_never_hides_a_differently_named_bound_call() -> Result<()> {
        assert_stored_impact(
            "go",
            &format!(
                "{STORED_GO}func exercise()int{{x:=X{{}};fn:=x.Execute;w:=W{{}};w.Execute();return fn()}}"
            ),
            true,
        )?;
        assert_stored_impact(
            "java",
            &format!(
                "{STORED_JAVA}class App{{public static int exercise(){{java.util.function.IntSupplier fn=new X()::execute;Y value=new W();value.execute();return fn.getAsInt();}}}}"
            ),
            true,
        )?;
        Ok(())
    }
    #[test]
    fn typescript_constructor_rebinding_and_shadowing_keep_warnings() -> Result<()> {
        for (declarations, body) in [
            (
                "class W implements Y{constructor(){return new X();}execute(){return 200;}}",
                "return new W().execute();",
            ),
            (
                "class W implements Y{constructor(){class Inner{} return new X();}execute(){return 200;}}",
                "return new W().execute();",
            ),
            (
                "class W implements Y{execute(){return 200;}} W=X;",
                "return new W().execute();",
            ),
            (
                "class W implements Y{execute(){return 200;}}",
                "const W=X;return new W().execute();",
            ),
            (
                "class W implements Y{execute(){return 200;}}",
                "const {W}={W:X};return new W().execute();",
            ),
            (
                "class W implements Y{get execute(){return ()=>new X().execute();}}",
                "return new W().execute();",
            ),
        ] {
            let (dir, store) = language_fixture(
                "typescript",
                "app.ts",
                &format!(
                    "interface Y{{execute():number;}} class X implements Y{{execute(){{return 100;}}}} {declarations} export function exercise(){{{body}}}"
                ),
            )?;
            let graph = DispatchGraph::build_with_root(&store, dir.path())?;
            let x = suffix_id(&store, "X.execute");
            assert!(
                graph
                    .engine(&store, &[x])?
                    .analyze(x)?
                    .impact_zone_ids
                    .contains(&suffix_id(&store, ".exercise")),
                "lost {declarations} / {body}"
            );
        }
        Ok(())
    }
    #[test]
    fn closed_local_receiver_excludes_siblings_and_keeps_native_facts() -> Result<()> {
        let (dir, store) = fixture(&format!(
            "{TYPES}\nclass App {{ int exercise() {{ X unused=new Z(); X value=new W(); X alias=value; return alias.execute(); }} }}"
        ))?;
        let graph = DispatchGraph::build_with_root(&store, dir.path())?;
        let caller = id(&store, "sample.App.exercise");
        let z = id(&store, "sample.Z.execute");
        let w = id(&store, "sample.W.execute");
        assert!(
            !graph
                .engine(&store, &[z])?
                .analyze(z)?
                .impact_zone_ids
                .contains(&caller)
        );
        assert!(
            graph
                .engine(&store, &[w])?
                .analyze(w)?
                .impact_zone_ids
                .contains(&caller)
        );
        assert!(
            !graph.origin_coverage["receiverRefinements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let declared = id(&store, "sample.X.execute");
        let mut native_interface_call = false;
        store.for_each_edge(|from, to, kind| {
            native_interface_call |= (from, to, kind) == (caller, declared, EdgeType::Calls);
            Ok(())
        })?;
        assert!(native_interface_call);
        Ok(())
    }
    #[test]
    fn receiver_refinement_preserves_alternatives_and_unknown_calls() -> Result<()> {
        for body in [
            "X value=flag?new Z():new W(); return value.execute();",
            "X value=new W(); if(flag) value=new Z(); return value.execute();",
            "X value=flag?unknown:new W(); return value.execute();",
            "X value=new W(); value.execute(); return unknown.execute();",
            "X value=new W(); value.execute(); return make().execute();",
            "java.util.function.IntSupplier action = new Z()::execute; X value=new W(); value.execute(); return action.getAsInt();",
            "X value=new W(); while(flag) { value=new Z(); break; } return value.execute();",
            "X value=new W(); try { if(flag) throw new RuntimeException(); } catch(RuntimeException e) { value=new Z(); } return value.execute();",
        ] {
            let (dir, store) = fixture(&format!(
                "{TYPES}\nclass App {{ X make() {{ return new Z(); }} int exercise(boolean flag, X unknown) {{ {body} }} }}"
            ))?;
            let graph = DispatchGraph::build_with_root(&store, dir.path())?;
            let caller = id(&store, "sample.App.exercise");
            for method in ["sample.Z.execute", "sample.W.execute"] {
                let target = id(&store, method);
                assert!(
                    graph
                        .engine(&store, &[target])?
                        .analyze(target)?
                        .impact_zone_ids
                        .contains(&caller),
                    "lost {method}: {body}"
                );
            }
        }
        Ok(())
    }
    #[test]
    fn closed_receiver_keeps_inherited_and_default_method_bodies() -> Result<()> {
        for (declarations, target) in [
            (
                "interface X { int execute(); } class Base { public int execute(){return 100;} } class W extends Base implements X {}",
                "sample.Base.execute",
            ),
            (
                "interface X { default int execute(){return 100;} } class W implements X {}",
                "sample.X.execute",
            ),
        ] {
            let (dir, store) = fixture(&format!(
                "package sample; {declarations} class Z implements X {{public int execute(){{return 200;}}}} class App {{int exercise() {{X value=new W();return value.execute();}}}}"
            ))?;
            let graph = DispatchGraph::build_with_root(&store, dir.path())?;
            let caller = id(&store, "sample.App.exercise");
            let target = id(&store, target);
            assert!(
                graph
                    .engine(&store, &[target])?
                    .analyze(target)?
                    .impact_zone_ids
                    .contains(&caller)
            );
            let sibling = id(&store, "sample.Z.execute");
            assert!(
                !graph
                    .engine(&store, &[sibling])?
                    .analyze(sibling)?
                    .impact_zone_ids
                    .contains(&caller)
            );
        }
        Ok(())
    }
    #[test]
    fn initialization_side_effects_keep_the_conservative_warning() -> Result<()> {
        for declaration in [
            "class W implements X { W(){new Z().execute();} public int execute(){return 200;} }",
            "class W implements X { static {new Z().execute();} public int execute(){return 200;} }",
            "class W implements X { {new Z().execute();} public int execute(){return 200;} }",
            "class W implements X { int initialized=new Z().execute(); public int execute(){return 200;} }",
            "class Base { Base(){new Z().execute();} } class W extends Base implements X { public int execute(){return 200;} }",
        ] {
            let (dir, store) = fixture(&format!(
                "package sample; interface X{{int execute();}} class Z implements X{{public int execute(){{return 100;}}}} {declaration} class App{{int exercise(){{X value=new W();return value.execute();}}}}"
            ))?;
            let graph = DispatchGraph::build_with_root(&store, dir.path())?;
            let caller = id(&store, "sample.App.exercise");
            let changed = id(&store, "sample.Z.execute");
            assert!(
                graph
                    .engine(&store, &[changed])?
                    .analyze(changed)?
                    .impact_zone_ids
                    .contains(&caller),
                "lost construction side effect: {declaration}"
            );
        }
        Ok(())
    }
    #[test]
    fn joined_assignability_restores_upstream_application_calls() -> Result<()> {
        let (dir, store) = fixture(&format!(
            r#"{TYPES}
class App {{
 static int consume(X value) {{ return value.execute(); }}
 int exercise() {{ return consume(new Z()); }}
}}
"#
        ))?;
        let graph = DispatchGraph::build_with_root(&store, dir.path())?;
        let changed = id(&store, "sample.Z.execute");
        let caller = id(&store, "sample.App.exercise");
        assert!(
            graph
                .engine(&store, &[changed])?
                .analyze(changed)?
                .impact_zone_ids
                .contains(&caller)
        );
        let mut native = HashSet::new();
        store.for_each_edge(|a, b, k| {
            if k == EdgeType::Calls {
                native.insert((a, b));
            }
            Ok(())
        })?;
        let consume = id(&store, "sample.App.consume");
        if !native.contains(&(caller, consume)) {
            assert_eq!(
                graph.possible[&(caller, consume)].reason,
                "external-assignability-join"
            );
        }
        Ok(())
    }
    #[test]
    fn joined_conditional_factory_retains_both_return_implementations() -> Result<()> {
        let (dir, store) = fixture(&format!(
            r#"{TYPES}
class App {{
 static X make() {{ return Boolean.getBoolean("useW") ? new W() : new Z(); }}
 int exercise() {{ return make().execute(); }}
}}
"#
        ))?;
        let graph = DispatchGraph::build_with_root(&store, dir.path())?;
        let caller = id(&store, "sample.App.exercise");
        for target in ["sample.Z.execute", "sample.W.execute"] {
            let target = id(&store, target);
            assert!(
                graph
                    .engine(&store, &[target])?
                    .analyze(target)?
                    .impact_zone_ids
                    .contains(&caller)
            );
            let evidence = &graph.possible[&(caller, target)];
            assert_eq!(evidence.reason, "external-contained-call-return-candidates");
            assert!(
                evidence
                    .join_evidence
                    .as_ref()
                    .is_some_and(|e| e.get("producer").is_some())
            );
        }
        Ok(())
    }
    #[test]
    fn implementations_reach_internal_changes_without_mutating_snapshot() -> Result<()> {
        let (_dir, store) = fixture(&format!(
            "{TYPES}\nclass Y {{ X service; int run() {{ return service.execute(); }} }}"
        ))?;
        let graph = DispatchGraph::build(&store)?;
        let caller = id(&store, "sample.Y.run");
        let implementation = id(&store, "sample.Z.execute");
        let target = id(&store, "sample.Z.internal");
        assert!(graph.possible.contains_key(&(caller, implementation)));
        assert!(
            graph
                .engine(&store, &[target])?
                .analyze(target)?
                .impact_zone_ids
                .contains(&caller)
        );
        let raw = BlastRadiusEngine::build_scoped(&store, &[target])?;
        assert!(!raw.analyze(target)?.impact_zone_ids.contains(&caller));
        Ok(())
    }
    #[test]
    fn qualifier_filter_is_per_receiver_and_not_per_interface() -> Result<()> {
        let (_dir, store) = fixture(&format!(
            r#"{TYPES}
class Y {{
 @Autowired @Qualifier("w") X selected;
 @Autowired X unknown;
 int fixed() {{ return selected.execute(); }}
 int broad() {{ return unknown.execute(); }}
 int both() {{ return selected.execute() + unknown.execute(); }}
}}
"#
        ))?;
        let graph = DispatchGraph::build(&store)?;
        let z = id(&store, "sample.Z.execute");
        let w = id(&store, "sample.W.execute");
        assert!(
            !graph
                .possible
                .contains_key(&(id(&store, "sample.Y.fixed"), z))
        );
        assert!(
            graph
                .possible
                .contains_key(&(id(&store, "sample.Y.fixed"), w))
        );
        assert!(
            graph
                .possible
                .contains_key(&(id(&store, "sample.Y.broad"), z))
        );
        assert!(
            graph
                .possible
                .contains_key(&(id(&store, "sample.Y.both"), z))
        );
        Ok(())
    }
    #[test]
    fn unknown_qualifier_and_factory_keep_conservative_candidates() -> Result<()> {
        for annotation in ["@Qualifier(NAME)", "@Qualifier(\"w\")"] {
            let factory = if annotation.contains("NAME") {
                ""
            } else {
                "class Config { @Bean X factory() { return condition ? new Z() : new W(); } }"
            };
            let (_dir, store) = fixture(&format!(
                "{TYPES}\nclass Y {{ static final String NAME=\"z\"; @Autowired {annotation} X service; int run() {{ return service.execute(); }} }}\n{factory}"
            ))?;
            let graph = DispatchGraph::build(&store)?;
            assert!(
                graph
                    .possible
                    .contains_key(&(id(&store, "sample.Y.run"), id(&store, "sample.Z.execute")))
            );
        }
        Ok(())
    }
    #[test]
    fn overloaded_overrides_do_not_cross_parameter_identities() -> Result<()> {
        let (_dir, store) = fixture(
            r#"package sample;
interface X { int execute(int value); int execute(String value); }
class Z implements X {
 public int execute(int value) { return 100; }
 public int execute(String value) { return 200; }
}
class Y {
 X service;
 int run() { return service.execute(1); }
}
"#,
        )?;
        let graph = DispatchGraph::build(&store)?;
        let caller = id(&store, "sample.Y.run");
        let mut targets = vec![];
        for ((from, to), _) in &graph.possible {
            if *from == caller {
                targets.push(
                    store
                        .get_node(*to)?
                        .unwrap()
                        .identity_discriminator()
                        .unwrap_or("")
                        .to_owned(),
                );
            }
        }
        let mut raw = vec![];
        store.for_each_edge(|from, to, kind| {
            if matches!(kind, EdgeType::Calls | EdgeType::Overrides) {
                raw.push((
                    store.get_node(from)?.map(|n| {
                        (
                            qualified(&n).to_owned(),
                            n.identity_discriminator().unwrap_or("").to_owned(),
                        )
                    }),
                    store.get_node(to)?.map(|n| {
                        (
                            qualified(&n).to_owned(),
                            n.identity_discriminator().unwrap_or("").to_owned(),
                        )
                    }),
                    kind,
                ));
            }
            Ok(())
        })?;
        assert_eq!(targets, vec!["(int)"], "Raw graph: {raw:?}");
        Ok(())
    }
    #[test]
    fn inherited_implementation_reaches_interface_call() -> Result<()> {
        let (_dir, store) = fixture(
            r#"package sample;
interface X { int execute(); }
class Base { public int execute() { return 100; } }
class Z extends Base implements X {}
class Y { X service; int run() { return service.execute(); } }
"#,
        )?;
        let graph = DispatchGraph::build(&store)?;
        assert!(graph.possible.contains_key(&(
            id(&store, "sample.Y.run"),
            id(&store, "sample.Base.execute")
        )));
        Ok(())
    }
    #[test]
    fn component_qualifier_alias_cannot_be_excluded_by_bean_name() -> Result<()> {
        let source = TYPES.replace("@Component(\"z\")", "@Component(\"z\") @Qualifier(\"w\")");
        let (_dir, store) = fixture(&format!(
            "{source}\nclass Y {{ @Autowired @Qualifier(\"w\") X service; int run() {{ return service.execute(); }} }}"
        ))?;
        let graph = DispatchGraph::build(&store)?;
        assert!(
            graph
                .possible
                .contains_key(&(id(&store, "sample.Y.run"), id(&store, "sample.Z.execute")))
        );
        Ok(())
    }
    #[test]
    fn contextual_dispatch_preserves_alternatives_unknowns_and_effects() -> Result<()> {
        for (declarations, body) in [
            ("", "return consume(flag?new W():new X());"),
            ("", "return consume(unknown);"),
            (
                "static Y recur(Y value){return recur(value);}",
                "return consume(recur(new W()));",
            ),
            (
                "static Y make(){new X().execute();return new W();}",
                "return consume(make());",
            ),
            (
                "static void opaque(Y value){}",
                "opaque(new X());return consume(new W());",
            ),
        ] {
            let source = format!(
                "{STORED_JAVA}class App{{static int consume(Y value){{return value.execute();}}static int other(){{return consume(new X());}}{declarations}static int exercise(boolean flag,Y unknown){{{body}}}}}"
            );
            let (dir, store) = fixture(&source)?;
            let graph = DispatchGraph::build_with_root(&store, dir.path())?;
            let changed = if body.starts_with("opaque") {
                suffix_id(&store, ".opaque")
            } else {
                suffix_id(&store, "X.execute")
            };
            assert!(
                graph
                    .engine(&store, &[changed])?
                    .analyze(changed)?
                    .impact_zone_ids
                    .contains(&suffix_id(&store, ".exercise")),
                "lost {body}"
            );
        }
        Ok(())
    }
    #[test]
    fn contextual_factory_returns_keep_the_selected_callers_values() -> Result<()> {
        let (dir, store) = fixture(&format!(
            "{STORED_JAVA}class App{{static Y forward(Y value){{return value;}}static int consume(Y value){{return value.execute();}}static int exercise(){{return consume(forward(new W()));}}static int other(){{return consume(forward(new X()));}}}}"
        ))?;
        let graph = DispatchGraph::build_with_root(&store, dir.path())?;
        let changed = suffix_id(&store, "X.execute");
        let impact = graph.engine(&store, &[changed])?.analyze(changed)?;
        assert!(
            !impact
                .impact_zone_ids
                .contains(&suffix_id(&store, ".exercise"))
        );
        assert!(
            impact
                .impact_zone_ids
                .contains(&suffix_id(&store, ".other"))
        );
        Ok(())
    }
    #[test]
    fn caller_contexts_are_separated_in_all_five_languages() -> Result<()> {
        for (language,file,source) in [
            ("java","App.java",format!("{STORED_JAVA}class App{{static int consume(Y value){{return value.execute();}}static int exerciseW(){{return consume(new W());}}static int exerciseX(){{return consume(new X());}}}}")),
            ("typescript","app.ts",format!("{STORED_TS}function consume(value:Y){{return value.execute();}}function exerciseW(){{return consume(new W());}}function exerciseX(){{return consume(new X());}}")),
            ("javascript","app.js","class X{execute(){return 100;}}class W{execute(){return 200;}}function consume(value){return value.execute();}function exerciseW(){return consume(new W());}function exerciseX(){return consume(new X());}".into()),
            ("python","app.py","class X:\n def execute(self):return 100\nclass W:\n def execute(self):return 200\ndef consume(value):\n return value.execute()\ndef exerciseW():\n return consume(W())\ndef exerciseX():\n return consume(X())\n".into()),
            ("go","app.go",format!("{STORED_GO}\nfunc consume(value Y)int{{return value.Execute()}}\nfunc exerciseW()int{{return consume(W{{}})}}\nfunc exerciseX()int{{return consume(X{{}})}}")),
        ] {
            let (dir,store)=if language=="java" {fixture(&source)?}else{language_fixture(language,file,&source)?};
            let graph=DispatchGraph::build_with_root(&store,dir.path())?;
            let changed=suffix_id(&store,if language=="go"{"X.Execute"}else{"X.execute"});
            let impact=graph.engine(&store,&[changed])?.analyze(changed)?;
            let w=suffix_id(&store,".exerciseW");
            assert!(!impact.impact_zone_ids.contains(&w),"{language}: {}",graph.origin_coverage["contextReceiverQuery"]);
            assert!(impact.impact_zone_ids.contains(&suffix_id(&store,".exerciseX")),"{language}: lost X caller");
            let mut budget=0;
            assert!(!graph.context.as_ref().unwrap().excludes(w,suffix_id(&store,".consume"),&HashSet::from([changed]),&graph.outgoing,&mut budget));
        }
        Ok(())
    }
    #[test]
    fn dynamic_constructor_replacements_keep_context_warnings() -> Result<()> {
        for (language, file, source) in [
            (
                "javascript",
                "app.js",
                "class X{execute(){return 100;}}class W{constructor(){return new X();}execute(){return 200;}}function consume(value){return value.execute();}function exercise(){return consume(new W());}function other(){return consume(new X());}",
            ),
            (
                "python",
                "app.py",
                "class X:\n def execute(self):return 100\nclass W:\n def __new__(cls):return X()\n def execute(self):return 200\ndef consume(value):return value.execute()\ndef exercise():return consume(W())\ndef other():return consume(X())\n",
            ),
        ] {
            let (dir, store) = language_fixture(language, file, source)?;
            let graph = DispatchGraph::build_with_root(&store, dir.path())?;
            let changed = suffix_id(&store, "X.execute");
            assert!(
                graph
                    .engine(&store, &[changed])?
                    .analyze(changed)?
                    .impact_zone_ids
                    .contains(&suffix_id(&store, ".exercise")),
                "{language}"
            );
        }
        Ok(())
    }
}
