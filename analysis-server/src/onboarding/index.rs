use super::{model::*, store};
use anyhow::{Context, Result, ensure};
use rgctl_analysis::{cfg_builder::ParsedSourceFile, operands::*};
use rgctl_graph::{
    SnapshotNodeStore,
    schema::{Node, NodeType},
};
use rgctl_pipeline::{PipelineConfig, ProcessingPipeline};
use rgctl_plugin_api::{LanguagePlugin, Relation, RelationType};
use rgctl_registry::LanguageRegistry;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

fn strings(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value.and_then(Value::as_str) else {
        return vec![String::new()];
    };
    if value.starts_with('{') && value.ends_with('}') {
        return value[1..value.len() - 1]
            .split(',')
            .filter_map(|s| serde_json::from_str::<String>(s.trim()).ok())
            .collect();
    }
    if value.is_empty() || value.starts_with('/') {
        vec![value.into()]
    } else {
        vec![]
    }
}
fn paths(annotations: &[&Relation]) -> Vec<String> {
    for r in annotations {
        if matches!(
            r.to.as_str(),
            "RequestMapping"
                | "GetMapping"
                | "PostMapping"
                | "PutMapping"
                | "PatchMapping"
                | "DeleteMapping"
                | "Path"
        ) {
            return strings(
                r.metadata
                    .get("annotation_args")
                    .and_then(|a| a.get("path").or_else(|| a.get("value"))),
            );
        }
    }
    vec![String::new()]
}
fn methods(annotations: &[&Relation]) -> Vec<String> {
    for r in annotations {
        let verb = match r.to.as_str() {
            "GetMapping" | "GET" => Some("GET"),
            "PostMapping" | "POST" => Some("POST"),
            "PutMapping" | "PUT" => Some("PUT"),
            "PatchMapping" | "PATCH" => Some("PATCH"),
            "DeleteMapping" | "DELETE" => Some("DELETE"),
            "HEAD" => Some("HEAD"),
            "OPTIONS" => Some("OPTIONS"),
            _ => None,
        };
        if let Some(verb) = verb {
            return vec![verb.into()];
        }
        if r.to == "RequestMapping" {
            let configured = r.metadata["annotation_args"]["method"]
                .as_str()
                .unwrap_or("");
            let verbs: Vec<_> = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]
                .into_iter()
                .filter(|v| configured.is_empty() || configured.contains(v))
                .map(str::to_owned)
                .collect();
            return verbs;
        }
    }
    vec![]
}
fn known_annotation(
    r: &Relation,
    extracted: &rgctl_plugin_api::ExtractAllResult,
    source: &[u8],
) -> bool {
    let hint = r
        .to_qualified_hint
        .as_deref()
        .or_else(|| r.metadata["fqn"].as_str())
        .unwrap_or("");
    let packages = [
        "org.springframework.web.bind.annotation.",
        "org.springframework.cloud.openfeign.",
        "javax.ws.rs.",
        "jakarta.ws.rs.",
        "org.eclipse.microprofile.rest.client.inject.",
    ];
    if packages.iter().any(|p| hint.starts_with(p)) {
        return true;
    }
    if extracted
        .symbols
        .iter()
        .any(|s| s.symbol_type == rgctl_plugin_api::SymbolType::Annotation && s.name == r.to)
    {
        return false;
    }
    let imports: Vec<_> = extracted
        .relations
        .iter()
        .filter(|i| i.metadata["import"] == true)
        .collect();
    if imports.iter().any(|i| {
        i.to == r.to
            && !packages
                .iter()
                .any(|p| i.metadata["fqn"].as_str().is_some_and(|f| f.starts_with(p)))
    }) {
        return false;
    }
    imports.iter().any(|i| {
        let fqn = i.metadata["fqn"].as_str().unwrap_or("");
        if !packages.iter().any(|p| p.trim_end_matches('.') == fqn) {
            return false;
        }
        let Some(end) = i
            .call_site_start_byte
            .zip(i.call_site_byte_len)
            .map(|(s, n)| s as usize + n as usize)
        else {
            return false;
        };
        source
            .get(end..)
            .is_some_and(|tail| String::from_utf8_lossy(tail).trim_start().starts_with(".*"))
    })
}
pub(super) fn symbol(node: &Node, root: &Path) -> Symbol {
    let file = Path::new(node.file_path.as_deref().unwrap_or(""));
    Symbol {
        node_id: node.id.to_string(),
        qualified_name: node.qualified_name.as_deref().unwrap_or(&node.name).into(),
        file: file
            .strip_prefix(root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/"),
        start_line: node.start_line.unwrap_or(0),
        end_line: node.end_line.unwrap_or(0),
    }
}
fn call_name(facts: &FunctionOperands, expression: ExpressionId) -> Option<&str> {
    match facts.expressions.get(expression.0 as usize)?.kind {
        ExpressionKind::Name(n) | ExpressionKind::Member(n) => {
            facts.names.get(n.0 as usize).map(String::as_str)
        }
        _ => None,
    }
}
fn literal(facts: &FunctionOperands, expression: ExpressionId, source: &[u8]) -> Option<String> {
    let value = facts.expressions.get(expression.0 as usize)?;
    if value.kind != ExpressionKind::Literal {
        return None;
    }
    let span = value.span?;
    serde_json::from_slice(source.get(span.start as usize..span.end as usize)?).ok()
}
fn resource_literal(
    facts: &FunctionOperands,
    expression: ExpressionId,
    source: &[u8],
    depth: usize,
) -> Option<String> {
    if depth > 8 {
        return None;
    }
    if let Some(value) = literal(facts, expression, source) {
        return Some(value);
    }
    if let ExpressionKind::CallResult(id) = facts.expressions.get(expression.0 as usize)?.kind {
        let call = facts.calls.get(id.0 as usize)?;
        if ["of", "get"].contains(&call_name(facts, call.callee)?)
            && call
                .receiver
                .and_then(|r| call_name(facts, r))
                .is_some_and(|name| ["Path", "Paths"].contains(&name))
            && call.arguments.len == 1
        {
            return resource_literal(
                facts,
                call.arguments.slice(&facts.arguments)?.first()?.value,
                source,
                depth + 1,
            );
        }
    }
    None
}
fn chain_method(facts: &FunctionOperands, mut expression: ExpressionId) -> Vec<String> {
    for _ in 0..64 {
        let Some(value) = facts.expressions.get(expression.0 as usize) else {
            break;
        };
        match value.kind {
            ExpressionKind::CallResult(id) => {
                let Some(call) = facts.calls.get(id.0 as usize) else {
                    break;
                };
                if let Some(name) = call_name(facts, call.callee) {
                    let upper = name.to_ascii_uppercase();
                    if ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]
                        .contains(&upper.as_str())
                    {
                        return vec![upper];
                    }
                }
                let Some(receiver) = call.receiver else { break };
                expression = receiver;
            }
            ExpressionKind::Parenthesized | ExpressionKind::AssertedValue => {
                let Some(first) = value.inputs.slice(&facts.inputs).and_then(|s| s.first()) else {
                    break;
                };
                expression = *first;
            }
            _ => break,
        }
    }
    vec![]
}
pub fn url_path(value: &str) -> Option<String> {
    let path = if value.starts_with("http://") || value.starts_with("https://") {
        url::Url::parse(value)
            .ok()?
            .path()
            .replace("%7B", "{")
            .replace("%7D", "}")
    } else if value.starts_with('/') {
        value.split('?').next()?.to_owned()
    } else {
        return None;
    };
    Some(if path == "/" {
        path
    } else {
        path.trim_end_matches('/').into()
    })
}

pub fn discover(
    root: &Path,
    files: &[std::path::PathBuf],
    graph: &SnapshotNodeStore,
) -> Result<(Vec<Operation>, Vec<Value>, Vec<ResourceUse>)> {
    let mut nodes = BTreeMap::<String, Vec<Node>>::new();
    graph.for_each_node_of_type(NodeType::Function, |n| {
        nodes
            .entry(symbol(n, root).file)
            .or_default()
            .push(n.clone());
    })?;
    let java = rgctl_lang_java::JavaPlugin::new()?;
    let mut operations = vec![];
    let mut diagnostics = vec![];
    let mut resource_uses = vec![];
    for relative in files {
        let name = relative.to_string_lossy().replace('\\', "/");
        let Some(functions) = nodes.get(&name) else {
            continue;
        };
        let source = fs::read(root.join(relative))?;
        let extracted = java.extract_all(&root.join(relative), &source)?;
        let annotations: Vec<_> = extracted
            .relations
            .iter()
            .filter(|r| {
                r.relation_type == RelationType::AnnotatedWith
                    && known_annotation(r, &extracted, &source)
            })
            .collect();
        let source_text = String::from_utf8_lossy(&source);
        let http_client = source_text.contains("org.springframework.web.client.")
            || source_text.contains("org.springframework.web.reactive.function.client.");
        let resource_client = source_text.contains("java.nio.file.")
            || source_text.contains("java.sql.")
            || source_text.contains("javax.sql.");
        let parsed = ParsedSourceFile::parse("java", &source)?;
        for node in functions {
            let location = symbol(node, root);
            let qn = location.qualified_name.as_str();
            let owner = qn.rsplit_once('.').map(|(o, _)| o).unwrap_or("");
            let class_annotations: Vec<_> = annotations
                .iter()
                .copied()
                .filter(|r| r.from == owner)
                .collect();
            let method_annotations: Vec<_> = annotations
                .iter()
                .copied()
                .filter(|r| {
                    r.from == qn
                        && r.location.start_line >= location.start_line
                        && r.location.start_line <= location.end_line
                })
                .collect();
            let verbs = methods(&method_annotations);
            if !verbs.is_empty() {
                let client = class_annotations
                    .iter()
                    .any(|r| r.to == "FeignClient" || r.to == "RegisterRestClient");
                let mut routes = vec![];
                for base in paths(&class_annotations) {
                    for leaf in paths(&method_annotations) {
                        routes.push(
                            format!("/{}/{}", base.trim_matches('/'), leaf.trim_matches('/'))
                                .replace("//", "/"),
                        );
                    }
                }
                if routes.is_empty() {
                    diagnostics.push(json!({"code":"UNRESOLVED_ROUTE","symbol":location}));
                    operations.push(Operation {
                        direction: if client { "consumes" } else { "provides" }.into(),
                        framework: "java-http-annotation".into(),
                        methods: verbs.clone(),
                        path: None,
                        url: None,
                        symbol: location.clone(),
                    });
                }
                for path in routes {
                    operations.push(Operation {
                        direction: if client { "consumes" } else { "provides" }.into(),
                        framework: if client {
                            "declarative-http-client"
                        } else {
                            "java-http-annotation"
                        }
                        .into(),
                        methods: verbs.clone(),
                        path: url_path(&path),
                        url: None,
                        symbol: location.clone(),
                    });
                }
            }
            if !http_client && !resource_client {
                continue;
            }
            let cfg = match (node.start_byte, node.end_byte) {
                (Some(start), Some(end)) => parsed.build_cfg_for_byte_range(
                    "java",
                    &source,
                    &node.name,
                    start as usize,
                    end as usize,
                ),
                _ => parsed.build_cfg_for_node_span(
                    "java",
                    &source,
                    &node.name,
                    location.start_line,
                    location.end_line,
                ),
            };
            let Ok(cfg) = cfg else {
                diagnostics
                    .push(json!({"code":"INVOCATION_OPERANDS_UNAVAILABLE","symbol":location}));
                continue;
            };
            let facts = &cfg.operands;
            for call in &facts.calls {
                let Some(method) = call_name(facts, call.callee) else {
                    continue;
                };
                if resource_client {
                    let receiver = call
                        .receiver
                        .and_then(|r| call_name(facts, r))
                        .unwrap_or("");
                    let kind = if receiver == "Files"
                        && [
                            "writeString",
                            "readString",
                            "write",
                            "readAllBytes",
                            "createFile",
                            "delete",
                            "exists",
                        ]
                        .contains(&method)
                    {
                        Some("file-store")
                    } else if method == "getConnection" {
                        Some("database")
                    } else {
                        None
                    };
                    if let Some(kind) = kind {
                        let value = call
                            .arguments
                            .slice(&facts.arguments)
                            .and_then(|a| a.first())
                            .and_then(|a| resource_literal(facts, a.value, &source, 0));
                        let uri = if kind == "file-store" {
                            value.map(|p| format!("file:{p}"))
                        } else {
                            value
                        };
                        if uri.is_none() {
                            diagnostics.push(json!({"code":"UNRESOLVED_RESOURCE_URI","symbol":location,"operation":method}));
                        }
                        resource_uses.push(ResourceUse {
                            resource_type: kind.into(),
                            uri,
                            operation: method.into(),
                            symbol: location.clone(),
                            application_paths: vec![],
                        });
                    }
                }
                if !http_client {
                    continue;
                }
                let verb = match method {
                    "getForObject" | "getForEntity" => Some("GET"),
                    "postForObject" | "postForEntity" => Some("POST"),
                    "put" => Some("PUT"),
                    "delete" => Some("DELETE"),
                    "patchForObject" => Some("PATCH"),
                    "headForHeaders" => Some("HEAD"),
                    "optionsForAllow" => Some("OPTIONS"),
                    _ => None,
                };
                if verb.is_none() && method != "uri" {
                    continue;
                }
                let methods = verb.map(|v| vec![v.into()]).unwrap_or_else(|| {
                    call.receiver
                        .map(|r| chain_method(facts, r))
                        .unwrap_or_default()
                });
                if method == "uri" && methods.is_empty() {
                    diagnostics.push(json!({"code":"UNRESOLVED_HTTP_METHOD","symbol":location}));
                }
                let value = call
                    .arguments
                    .slice(&facts.arguments)
                    .and_then(|a| a.first())
                    .and_then(|a| literal(facts, a.value, &source));
                let path = value.as_deref().and_then(url_path);
                if path.is_none() {
                    diagnostics.push(
                        json!({"code":"UNRESOLVED_HTTP_URL","symbol":location,"method":method}),
                    );
                }
                operations.push(Operation {
                    direction: "consumes".into(),
                    framework: "spring-http-invocation".into(),
                    methods,
                    path,
                    url: value,
                    symbol: location.clone(),
                });
            }
        }
    }
    operations.sort_by(|a, b| {
        (&a.direction, &a.symbol.qualified_name, &a.path).cmp(&(
            &b.direction,
            &b.symbol.qualified_name,
            &b.path,
        ))
    });
    Ok((operations, diagnostics, resource_uses))
}

pub fn index(
    request: &Onboard,
    version: &str,
    dir: &Path,
    source: &Path,
    commit: String,
    maven_repository: Option<std::path::PathBuf>,
) -> Result<Manifest> {
    let input = dir.join("input");
    let mut files = if request.source.include_maven_dependencies {
        store::copy_java(source, &input.join("application"))?
            .into_iter()
            .map(|p| std::path::PathBuf::from("application").join(p))
            .collect()
    } else {
        store::copy_java(source, &input)?
    };
    let (project_coordinate, artifacts) = if request.source.include_maven_dependencies {
        let (coordinate, artifacts, additional) =
            super::dependencies::acquire(source, dir, &input, maven_repository)?;
        files.extend(additional);
        (Some(coordinate), artifacts)
    } else {
        (None, vec![])
    };
    files.sort();
    let mut digest = vec![];
    for relative in &files {
        digest.extend(relative.to_string_lossy().as_bytes());
        digest.push(0);
        digest.extend(store::hash_file(&input.join(relative))?.as_bytes());
        digest.push(0);
    }
    let mut registry = LanguageRegistry::new();
    rgctl_lang_java::register(&mut registry);
    let pipeline = ProcessingPipeline::with_config(
        Arc::new(registry),
        PipelineConfig {
            show_progress: false,
            facts_cache: false,
            materialize_fields: true,
            thread_count: Some(4),
            ..Default::default()
        },
    );
    let snapshot = dir.join("graph.snapshot.bin");
    let (stats, _) = pipeline.process_repository_to_snapshot(&input, &snapshot, Some(&input))?;
    ensure!(
        stats.files_failed == 0,
        "Java extraction failed for {} files",
        stats.files_failed
    );
    let graph = SnapshotNodeStore::open(&snapshot)?;
    let (operations, mut diagnostics, mut resource_uses) = discover(&input, &files, &graph)?;
    for artifact in &artifacts {
        if artifact.status != "available" {
            diagnostics.push(
                json!({"code":"MISSING_DEPENDENCY_SOURCES","coordinate":artifact.coordinate}),
            );
        }
    }
    let dependency_traces = super::trace::populate(
        &graph,
        &input,
        request.source.include_maven_dependencies,
        &artifacts,
        &mut resource_uses,
        &mut diagnostics,
    )?;
    Ok(Manifest {
        schema_version: 1,
        application_key: store::key(&request.entity_ref),
        entity_ref: request.entity_ref.clone(),
        graph_version: version.into(),
        source: request.source.clone(),
        commit,
        source_sha256: store::hash(&digest),
        snapshot_sha256: store::hash_file(&snapshot)?,
        rgctl_revision: env!("RGCTL_REVISION").into(),
        rgctl_source_sha256: env!("RGCTL_SOURCE_SHA256").into(),
        files_indexed: files.len(),
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),
        provides_apis: request.provides_apis.clone(),
        consumes_apis: request.consumes_apis.clone(),
        operations,
        diagnostics,
        project_coordinate,
        artifacts,
        dependency_traces,
        resource_uses,
    })
}

pub fn snapshot(config: &Config, manifest: &Manifest) -> Result<SnapshotNodeStore> {
    let path = store::job_dir(config, &manifest.graph_version)?.join("graph.snapshot.bin");
    ensure!(
        store::hash_file(&path)? == manifest.snapshot_sha256,
        "Stored graph checksum mismatch"
    );
    SnapshotNodeStore::open(&path).context("Cannot reopen stored rgctl graph")
}
