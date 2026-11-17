//! Conservative joins over native RGCTL facts. No independent source parsing.
use anyhow::Result;
use rgctl_graph::schema::{Node, NodeType};
use rgctl_plugin_api::{Relation, RelationType};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;
#[path = "context_dispatch.rs"]
mod context_dispatch;
#[path = "origin_joins.rs"]
mod origin_joins;
#[path = "receivers.rs"]
mod receivers;
#[path = "storage_receivers.rs"]
mod storage_receivers;
pub use context_dispatch::ContextDispatch;

pub type ReceiverRefinements = HashMap<(Uuid, String), HashSet<Uuid>>;

pub struct JoinAnalysis {
    pub receiver_refinements: ReceiverRefinements,
    pub candidates: Vec<Candidate>,
    pub origin_coverage: Value,
    pub context: Option<ContextDispatch>,
}

pub struct Candidate {
    pub from: Uuid,
    pub to: Uuid,
    pub reason: &'static str,
    pub evidence: Value,
    pub declared_target: Option<Uuid>,
}

fn name(n: &Node) -> &str {
    n.qualified_name.as_deref().unwrap_or(&n.name)
}
fn logical(root: &Path, path: &str) -> PathBuf {
    Path::new(path)
        .strip_prefix(root)
        .unwrap_or(Path::new(path))
        .to_path_buf()
}
fn contains(outer: &Relation, inner: &Relation) -> bool {
    let a = &outer.location;
    let b = &inner.location;
    let start = (a.start_line, a.start_column);
    let end = (a.end_line, a.end_column);
    start <= (b.start_line, b.start_column)
        && (b.end_line, b.end_column) <= end
        && (start, end) != ((b.start_line, b.start_column), (b.end_line, b.end_column))
        && a.file == b.file
}
fn arguments(r: &Relation) -> Option<Vec<&str>> {
    let identity = r
        .metadata
        .get("argument_identity_discriminator")?
        .as_str()?;
    let body = identity.strip_prefix('(')?.strip_suffix(')')?;
    if body.contains(['<', '>']) {
        return None;
    }
    Some(if body.is_empty() {
        vec![]
    } else {
        body.split(',').map(str::trim).collect()
    })
}

/// Module metadata is a resolver input, not a guessed package-name suffix.
fn go_module(root: &Path) -> Option<(String, String)> {
    if root.join("go.work").exists() {
        return None;
    }
    let bytes = std::fs::read(root.join("go.mod")).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let mut modules = Vec::new();
    for line in text.lines() {
        let line = line.split("//").next()?.trim();
        let words: Vec<_> = line.split_whitespace().collect();
        if words.first() == Some(&"module") {
            if words.len() != 2 {
                return None;
            }
            modules.push(words[1].trim_matches('"').to_owned());
        }
    }
    if modules.len() != 1 || modules[0].is_empty() {
        return None;
    }
    Some((modules.remove(0), hex::encode(Sha256::digest(bytes))))
}
fn go_nominal_type(
    nodes: &HashMap<Uuid, Node>,
    root: &Path,
    file: &Path,
    raw: &str,
    module: Option<&str>,
    admitted: &HashSet<Uuid>,
) -> Option<Uuid> {
    let (qualifier, simple) = raw
        .rsplit_once('.')
        .map_or((None, raw), |(q, n)| (Some(q), n));
    let mut found = Vec::new();
    for n in nodes.values().filter(|n| {
        n.node_type == NodeType::Struct && &*n.name == simple && admitted.contains(&n.id)
    }) {
        let source = logical(root, n.file_path.as_deref()?);
        let directory = source.parent()?;
        let resolved = if let Some(qualifier) = qualifier {
            let Some(module) = module else {
                continue;
            };
            // A nested module has its own coordinate and cannot be resolved by
            // the outer module's directory mapping.
            if directory
                .ancestors()
                .take_while(|p| !p.as_os_str().is_empty())
                .any(|p| root.join(p).join("go.mod").exists())
            {
                continue;
            }
            let relative = directory.to_string_lossy().replace('\\', "/");
            let expected = if relative.is_empty() {
                module.to_owned()
            } else {
                format!("{module}/{relative}")
            };
            let imports: Vec<_> = nodes
                .values()
                .filter(|v| {
                    v.node_type == NodeType::Import
                        && &*v.name == qualifier
                        && v.file_path
                            .as_deref()
                            .is_some_and(|p| logical(root, p) == file)
                })
                .collect();
            imports.len() == 1 && name(imports[0]) == expected
        } else {
            directory == file.parent()?
        };
        if resolved {
            found.push(n.id);
        }
    }
    (found.len() == 1).then(|| found[0])
}

pub fn candidate_calls(
    nodes: &HashMap<Uuid, Node>,
    root: &Path,
    parents: &HashMap<String, Vec<String>>,
    native_calls: &HashSet<(Uuid, Uuid)>,
    instantiations: &HashSet<(Uuid, Uuid)>,
) -> Result<JoinAnalysis> {
    let mut registry = rgctl_registry::LanguageRegistry::new();
    rgctl_lang_java::register(&mut registry);
    rgctl_lang_javascript::register(&mut registry);
    rgctl_lang_typescript::register(&mut registry);
    rgctl_lang_python::register(&mut registry);
    rgctl_lang_go::register(&mut registry);
    let extractor = rgctl_extraction::Extractor::new(Arc::new(registry)).with_repo_root(root);
    let mut names: HashMap<&str, Vec<&Node>> = HashMap::new();
    let mut files = std::collections::BTreeSet::new();
    for node in nodes.values() {
        names.entry(name(node)).or_default().push(node);
        if let Some(file) = node.file_path.as_deref() {
            if matches!(
                Path::new(file).extension().and_then(|s| s.to_str()),
                Some("java" | "js" | "jsx" | "ts" | "tsx" | "py" | "go")
            ) {
                files.insert(logical(root, file));
            }
        }
    }
    let compatible = |actual: &str, formal: &str| -> bool {
        if actual == formal {
            return true;
        }
        let types = |s: &str| -> Vec<&Node> {
            nodes
                .values()
                .filter(|n| {
                    matches!(
                        n.node_type,
                        NodeType::Class | NodeType::Struct | NodeType::Interface | NodeType::Enum
                    ) && (name(n) == s || name(n).ends_with(&format!(".{s}")) || &*n.name == s)
                })
                .collect()
        };
        let a = types(actual);
        let b = types(formal);
        if a.len() != 1 || b.len() != 1 {
            return false;
        }
        let target = name(b[0]);
        let mut seen = HashSet::new();
        let mut queue = vec![name(a[0])];
        while let Some(key) = queue.pop() {
            if key == target {
                return true;
            }
            if seen.insert(key) {
                queue.extend(parents.get(key).into_iter().flatten().map(String::as_str));
            }
        }
        false
    };
    // Native extraction inventories class members even when their initialization
    // is not represented by an executable caller edge. Preserve those warnings.
    let mut java_no_fields: HashMap<String, bool> = HashMap::new();
    let mut java_initializers = HashSet::new();
    let mut java_initialization_parents: HashMap<String, Vec<String>> = HashMap::new();
    let mut context_nominal_types = HashSet::new();
    let mut direct_functions = HashSet::new();
    let mut private_functions = HashSet::new();
    let mut value_names = HashSet::new();
    let mut shadowed_names = HashSet::new();
    let mut trivial_types = HashSet::new();
    let mut storage_types = HashSet::new();
    let mut ts_types = HashSet::new();
    let mut ts_dynamic_writes = false;
    let mut nominal_names = HashMap::new();
    let mut storage_aliases = HashMap::new();
    let mut result = vec![];
    let mut relations = vec![];
    let mut locals = HashMap::new();
    let mut cfgs = HashMap::new();
    let mut procedural_nodes = HashMap::new();
    let mut environment_modules = Vec::new();
    let mut line_offsets = HashMap::new();
    let mut preparation_failures = Vec::new();
    let mut preparation_limited = false;
    for file in files {
        let physical = root.join(&file);
        // Failure is an analysis error, not a successful empty result.
        let facts = extractor.extract_file_with_source(&physical, std::fs::read(&physical)?)?;
        let functions: Vec<_> = nodes
            .values()
            .filter(|n| {
                n.node_type == NodeType::Function
                    && n.file_path
                        .as_deref()
                        .is_some_and(|p| logical(root, p) == file)
            })
            .collect();
        let mut offsets = vec![0usize];
        offsets.extend(
            facts
                .source
                .iter()
                .enumerate()
                .filter_map(|(i, b)| (*b == b'\n').then_some(i + 1)),
        );
        line_offsets.insert(file.clone(), offsets);
        let file_language = functions
            .first()
            .and_then(|f| f.get_property("language"))
            .or_else(|| match file.extension().and_then(|s| s.to_str()) {
                Some("js" | "jsx") => Some("javascript"),
                Some("ts" | "tsx") => Some("typescript"),
                Some("py") => Some("python"),
                Some("go") => Some("go"),
                Some("java") => Some("java"),
                _ => None,
            });
        if matches!(file_language, Some("java" | "typescript" | "go")) {
            for symbol in facts.symbols.iter().filter(|s| {
                matches!(
                    s.symbol_type,
                    rgctl_plugin_api::SymbolType::Class | rgctl_plugin_api::SymbolType::Struct
                )
            }) {
                // The native field span is used only as a negative admission
                // guard. No initializer or data-flow edge is fabricated here.
                let no_initializers = symbol.fields.iter().all(|f| {
                    if file_language == Some("go")
                        && f.field_type.as_deref().is_some_and(|t| {
                            t.trim_start_matches('*')
                                .rsplit('.')
                                .next()
                                .unwrap_or(t)
                                .split('[')
                                .next()
                                == Some(&f.name)
                        })
                    {
                        return false;
                    }

                    file_language == Some("go")
                        || f.source_span
                            .and_then(|span| {
                                facts
                                    .source
                                    .get(span.start_byte as usize..span.end_byte as usize)
                            })
                            .is_some_and(|bytes| {
                                !bytes.contains(&b'=')
                                    && !bytes.contains(&b'@')
                                    && !bytes.contains(&b'\\')
                            })
                });
                if no_initializers {
                    let found: Vec<_> = nodes
                        .values()
                        .filter(|n| {
                            matches!(n.node_type, NodeType::Class | NodeType::Struct)
                                && &*n.name == symbol.name
                                && n.file_path
                                    .as_deref()
                                    .is_some_and(|p| logical(root, p) == file)
                        })
                        .collect();
                    if found.len() == 1 {
                        storage_types.insert(found[0].id);
                    }
                }
            }
        }
        if file_language == Some("java") {
            for symbol in &facts.symbols {
                let Some(q) = symbol.qualified_name.as_deref() else {
                    continue;
                };
                if matches!(
                    symbol.symbol_type,
                    rgctl_plugin_api::SymbolType::Class | rgctl_plugin_api::SymbolType::Interface
                ) {
                    java_no_fields
                        .entry(q.to_owned())
                        .and_modify(|v| *v &= symbol.fields.is_empty())
                        .or_insert(symbol.fields.is_empty());
                }
                if [
                    "is_constructor",
                    "is_static_initializer",
                    "is_instance_initializer",
                ]
                .iter()
                .any(|key| symbol.metadata.get(*key).and_then(Value::as_bool) == Some(true))
                    && symbol.metadata.get("has_initializer_body") != Some(&Value::Bool(false))
                    && let Some((owner, _)) = q.rsplit_once('.')
                {
                    java_initializers.insert(owner.to_owned());
                }
            }
            for r in facts.relations.iter().filter(|r| {
                matches!(
                    r.relation_type,
                    RelationType::Extends | RelationType::Implements
                )
            }) {
                java_initialization_parents
                    .entry(r.from.clone())
                    .or_default()
                    .push(r.to_qualified_hint.as_deref().unwrap_or(&r.to).to_owned());
            }
        }
        if let Some(language) = file_language {
            let parsed =
                rgctl_analysis::cfg_builder::ParsedSourceFile::parse(language, &facts.source);
            if let Ok(parsed) = parsed.as_ref() {
                let simple = rgctl_analysis::context_receivers::simple_nominal_classes(
                    parsed,
                    language,
                    &facts.source,
                );
                for n in nodes.values().filter(|n| {
                    n.node_type == NodeType::Class
                        && n.file_path
                            .as_deref()
                            .is_some_and(|p| logical(root, p) == file)
                }) {
                    if simple
                        .iter()
                        .any(|s| n.start_byte == Some(s.start) && n.end_byte == Some(s.end))
                    {
                        context_nominal_types.insert(n.id);
                    }
                }
                let scope = rgctl_analysis::receiver_scope::receiver_scope_facts(
                    parsed,
                    &facts.source,
                    language,
                    250_000,
                );
                if scope.limited {
                    preparation_limited = true;
                } else {
                    value_names.extend(scope.value_names);
                    shadowed_names.extend(scope.shadowed_names);
                    for allocation in scope.nominal_allocations {
                        nominal_names.insert(
                            (file.clone(), allocation.span.start, allocation.span.end),
                            allocation.type_name,
                        );
                    }
                    if language == "typescript" {
                        ts_dynamic_writes |= scope.dynamic_writes;
                    }
                    let overlaps = |n: &Node, span: &rgctl_analysis::operands::OperandSpan| {
                        n.start_byte.zip(n.end_byte).is_some_and(|(s, e)| {
                            // Export wrappers may extend the declaration prefix.
                            // Nested declarations must never qualify their owner.
                            e == span.end && s <= span.end && span.start <= e
                        })
                    };
                    for n in &functions {
                        if scope.direct_functions.iter().any(|s| overlaps(n, s)) {
                            direct_functions.insert(n.id);
                        }
                        if scope.private_functions.iter().any(|s| overlaps(n, s)) {
                            private_functions.insert(n.id);
                        }
                    }
                    for n in nodes.values().filter(|n| {
                        matches!(n.node_type, NodeType::Class | NodeType::Struct)
                            && n.file_path
                                .as_deref()
                                .is_some_and(|p| logical(root, p) == file)
                    }) {
                        if scope.trivial_types.iter().any(|s| overlaps(n, s)) {
                            trivial_types.insert(n.id);
                            if language == "typescript" {
                                ts_types.insert(n.id);
                            }
                        }
                    }
                }
            }
            let mut functions = functions;
            functions.sort_by_key(|f| (f.start_byte, f.end_byte, f.id));
            for function in functions {
                if cfgs.len() >= 1_024 {
                    preparation_limited = true;
                    break;
                }
                let built = parsed
                    .as_ref()
                    .map_err(|e| e.to_string())
                    .and_then(|parsed| {
                        let exact = match (function.start_byte, function.end_byte) {
                            (Some(start), Some(end)) => parsed.build_cfg_for_byte_range(
                                language,
                                &facts.source,
                                &function.name,
                                start as usize,
                                end as usize,
                            ),
                            _ => parsed.build_cfg(language, &facts.source, &function.name),
                        };
                        // The native fallback rejects ambiguous names; it never
                        // chooses a same-named method arbitrarily.
                        exact
                            .or_else(|_| parsed.build_cfg(language, &facts.source, &function.name))
                            .map_err(|e| e.to_string())
                    });
                match built {
                    Ok(cfg) => {
                        locals.insert(function.id, cfg.local_types.clone());
                        cfgs.insert(function.id, cfg);
                        procedural_nodes.insert(function.id, function.clone());
                    }
                    Err(reason) => {
                        preparation_failures.push(json!({"function":function.id,"reason":reason}))
                    }
                }
            }
            if let Ok(parsed) = parsed.as_ref() {
                if cfgs.len() < 1_024 {
                    let digest =
                        Sha256::digest(format!("environment-module:{}", file.display()).as_bytes());
                    let mut bytes = [0u8; 16];
                    bytes.copy_from_slice(&digest[..16]);
                    let module = uuid::Uuid::from_bytes(bytes);
                    match parsed.build_module_cfg(language, &facts.source) {
                        Ok(cfg) => {
                            let node = Node::new(NodeType::Function, "<module-init>")
                                .with_file_path(physical.to_string_lossy().to_string())
                                .with_qualified_name(format!("{}::<module-init>", file.display()))
                                .with_property("language".into(), language.into());
                            let mut node = node;
                            node.id = module;
                            procedural_nodes.insert(module, node);
                            cfgs.insert(module, cfg);
                            let mut procedures: Vec<_> = procedural_nodes
                                .iter()
                                .filter(|(_, n)| {
                                    n.file_path
                                        .as_deref()
                                        .is_some_and(|p| logical(root, p) == file)
                                })
                                .filter_map(|(&id, _)| {
                                    cfgs.get(&id).map(|cfg| {
                                        rgctl_analysis::environment::EnvironmentProcedure {
                                            id,
                                            operands: &cfg.operands,
                                        }
                                    })
                                })
                                .collect();
                            procedures.sort_by_key(|p| p.id);
                            let environment_facts = parsed.environment_facts(
                                language,
                                &facts.source,
                                module,
                                &procedures,
                                Default::default(),
                            )?;
                            environment_modules.push(
                                rgctl_analysis::environment::EnvironmentModule {
                                    path: file.to_string_lossy().replace('\\', "/"),
                                    language: language.into(),
                                    facts: environment_facts,
                                },
                            );
                        }
                        Err(reason) => preparation_failures
                            .push(json!({"module":file,"reason":reason.to_string()})),
                    }
                } else {
                    preparation_limited = true;
                }
            }
        }
        if file_language == Some("go") {
            for (&function, cfg) in &cfgs {
                if !procedural_nodes
                    .get(&function)
                    .and_then(|n| n.file_path.as_deref())
                    .is_some_and(|p| logical(root, p) == file)
                {
                    continue;
                }
                for (i, e) in cfg
                    .operands
                    .expressions
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        e.kind == rgctl_analysis::operands::ExpressionKind::Computation
                    })
                {
                    let Some(input) = e
                        .inputs
                        .slice(&cfg.operands.inputs)
                        .filter(|v| v.len() == 1)
                        .map(|v| v[0])
                    else {
                        continue;
                    };
                    let Some((outer, inner)) =
                        e.span.zip(cfg.operands.expressions[input.0 as usize].span)
                    else {
                        continue;
                    };
                    let prefix = facts
                        .source
                        .get(outer.start as usize..inner.start as usize)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .map(str::trim);
                    let suffix = facts
                        .source
                        .get(inner.end as usize..outer.end as usize)
                        .and_then(|v| std::str::from_utf8(v).ok())
                        .map(str::trim);
                    if prefix == Some("&") && suffix == Some("") {
                        storage_aliases.insert(
                            rgctl_analysis::call_bindings::ValueRef {
                                function,
                                expression: rgctl_analysis::operands::ExpressionId(i as u32),
                            },
                            rgctl_analysis::call_bindings::ValueRef {
                                function,
                                expression: input,
                            },
                        );
                    }
                }
            }
        }
        for relation in facts
            .relations
            .iter()
            .filter(|r| r.relation_type == RelationType::Calls)
        {
            let source_identity = relation
                .metadata
                .get("from_identity_discriminator")
                .and_then(Value::as_str)
                .unwrap_or("");
            let sources: Vec<_> = names
                .get(relation.from.as_str())
                .into_iter()
                .flatten()
                .copied()
                .filter(|n| {
                    n.node_type == NodeType::Function
                        && n.identity_discriminator().unwrap_or("") == source_identity
                        && n.file_path
                            .as_deref()
                            .is_some_and(|p| logical(root, p) == file)
                        && n.start_line
                            .is_some_and(|line| line <= relation.location.start_line)
                        && n.end_line
                            .is_some_and(|line| line >= relation.location.end_line)
                })
                .collect();
            if sources.len() != 1 {
                continue;
            }
            let source = sources[0];
            let hint = relation
                .to_qualified_hint
                .as_deref()
                .unwrap_or(&relation.to);
            let args = arguments(relation);
            let targets: Vec<_> = names
                .get(hint)
                .into_iter()
                .flatten()
                .copied()
                .filter(|n| {
                    n.node_type == NodeType::Function
                        && args.as_ref().is_some_and(|a| {
                            a.len() == n.parameters.len()
                                && a.iter().zip(&n.parameters).all(|(actual, p)| {
                                    p.param_type
                                        .as_deref()
                                        .is_some_and(|formal| compatible(actual, formal))
                                })
                        })
                })
                .collect();
            if targets.len() == 1 {
                result.push(Candidate {from:source.id,to:targets[0].id,declared_target:None,reason:"external-assignability-join",evidence:json!({"location":relation.location,"argumentIdentity":relation.metadata.get("argument_identity_discriminator"),"declaredParameters":targets[0].parameters})});
                continue;
            }
            for inner in facts.relations.iter().filter(|r| {
                r.relation_type == RelationType::Calls
                    && r.from == relation.from
                    && contains(relation, r)
                    && r.metadata.get("from_identity_discriminator")
                        == relation.metadata.get("from_identity_discriminator")
            }) {
                let producer_hint = inner.to_qualified_hint.as_deref().unwrap_or(&inner.to);
                for producer in names
                    .get(producer_hint)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|n| n.node_type == NodeType::Function)
                {
                    let Some(return_type) = producer.return_type_text() else {
                        continue;
                    };
                    let method = relation.to.rsplit('.').next().unwrap_or(&relation.to);
                    for target in nodes
                        .values()
                        .filter(|n| n.node_type == NodeType::Function && &*n.name == method)
                    {
                        let Some((owner, _)) = name(target).rsplit_once('.') else {
                            continue;
                        };
                        if !compatible(owner, return_type)
                            || args
                                .as_ref()
                                .is_some_and(|a| a.len() != target.parameters.len())
                        {
                            continue;
                        }
                        result.push(Candidate{from:source.id,to:target.id,declared_target:None,reason:"external-contained-call-return-candidates",evidence:json!({"producer":producer.id,"declaredReturnType":return_type,"location":relation.location,"producerLocation":inner.location,"assumption":"contained call result may supply receiver; expression role is not proven"})});
                    }
                }
            }
        }
        relations.extend(facts.relations);
    }
    result.extend(receivers::candidates(
        nodes,
        root,
        parents,
        native_calls,
        &result,
        &relations,
        &locals,
    ));
    let environment =
        rgctl_analysis::environment::link_environment_modules(environment_modules, 250_000);
    if environment.diagnostics.iter().any(|d| {
        matches!(
            d,
            rgctl_analysis::environment::EnvironmentDiagnostic::LimitReached
        )
    }) {
        preparation_limited = true;
    }
    let mut initialization_free: HashSet<_> = nodes
        .values()
        .filter(|n| n.get_property("language") == Some("java"))
        .filter(|n| {
            let mut seen = HashSet::new();
            let mut pending = vec![name(n)];
            while let Some(owner) = pending.pop() {
                if !seen.insert(owner) {
                    continue;
                }
                if java_no_fields.get(owner) != Some(&true) || java_initializers.contains(owner) {
                    return false;
                }
                pending.extend(
                    java_initialization_parents
                        .get(owner)
                        .into_iter()
                        .flatten()
                        .map(String::as_str),
                );
            }
            true
        })
        .map(|n| n.id)
        .collect();
    if ts_dynamic_writes {
        trivial_types.retain(|id| !ts_types.contains(id));
    }
    trivial_types.retain(|id| !shadowed_names.contains(&*nodes[id].name));
    initialization_free.extend(trivial_types);
    direct_functions.retain(|id| !value_names.contains(&*nodes[id].name));
    private_functions.retain(|id| !value_names.contains(&*nodes[id].name));
    storage_types.retain(|id| {
        let class = &nodes[id];
        if nodes.values().any(|n| {
            n.node_type == NodeType::Function
                && n.file_path == class.file_path
                && name(n).rsplit_once('.').is_some_and(|(owner, _)| {
                    owner == name(class) || owner.rsplit('.').next() == Some(&*class.name)
                })
                && !cfgs.contains_key(&n.id)
        }) {
            return false;
        }
        if relations.iter().any(|r| {
            if !class
                .file_path
                .as_deref()
                .is_some_and(|p| logical(root, p) == logical(root, &r.location.file))
                || r.from.rsplit('.').next() != Some(&*class.name)
            {
                return false;
            }
            if r.relation_type == RelationType::AnnotatedWith {
                return true;
            }
            r.relation_type == RelationType::Extends
                && !nodes.values().any(|n| {
                    n.node_type == NodeType::Interface
                        && (name(n) == r.to_qualified_hint.as_deref().unwrap_or(&r.to)
                            || &*n.name == r.to)
                })
        }) {
            return false;
        }
        if shadowed_names.contains(&*class.name) {
            return false;
        }
        if nodes.values().any(|n| {
            n.node_type == NodeType::Function
                && n.file_path == class.file_path
                && name(n).rsplit_once('.').is_some_and(|(owner, _)| {
                    owner == name(class) || owner.rsplit('.').next() == Some(&*class.name)
                })
                && (n.get_property("is_static_initializer") == Some("true")
                    || n.get_property("is_instance_initializer") == Some("true"))
                && n.get_property("has_initializer_body") != Some("false")
        }) {
            return false;
        }
        if parents.get(name(class)).into_iter().flatten().any(|p| {
            nodes
                .values()
                .any(|n| n.node_type == NodeType::Class && name(n) == p)
        }) {
            return false;
        }
        // Reject constructor-return replacement and any mutation of a class
        // binding or method slot. Ordinary this.inner stores remain eligible.
        if nodes.values().any(|n| {
            n.node_type == NodeType::Function
                && n.file_path == class.file_path
                && name(n).rsplit_once('.').is_some_and(|(owner, _)| {
                    owner == name(class) || owner.rsplit('.').next() == Some(&*class.name)
                })
                && n.signature.as_deref().is_some_and(|s| {
                    s.split_whitespace()
                        .any(|word| matches!(word, "get" | "set"))
                })
        }) {
            return false;
        }
        for (&function, cfg) in &cfgs {
            let owner = procedural_nodes.get(&function).map(name).unwrap_or("");
            if (owner == format!("{}.<init>", name(class))
                || procedural_nodes.get(&function).is_some_and(|n| {
                    n.get_property("is_constructor") == Some("true")
                        && n.name == class.name
                        && n.file_path == class.file_path
                }))
                && cfg.operands.language == rgctl_analysis::operands::OperandLanguage::TypeScript
                && cfg.operands.returns.iter().any(|r| r.values.len > 0)
            {
                return false;
            }
            if cfg.operands.local_writes.iter().any(|w| {
                cfg.operands
                    .local_slots
                    .get(w.slot.0 as usize)
                    .and_then(|n| cfg.operands.name(*n))
                    == Some(&*class.name)
                    && w.value.is_some()
            }) {
                return false;
            }
            if cfg.operands.field_writes.iter().any(|w| {
                cfg.operands.name(w.member).is_some_and(|m| {
                    m == "prototype"
                        || nodes.values().any(|n| {
                            n.node_type == NodeType::Function
                                && &*n.name == m
                                && name(n)
                                    .rsplit_once('.')
                                    .is_some_and(|(owner, _)| owner == name(class))
                        })
                })
            }) {
                return false;
            }
        }
        true
    });
    let module = go_module(root);
    let mut nominal_allocations = HashMap::new();
    let mut storage_allocations = HashMap::new();
    for (&caller, cfg) in &cfgs {
        let node = &procedural_nodes[&caller];
        let file = logical(root, node.file_path.as_deref().unwrap_or(""));
        for (i, e) in cfg
            .operands
            .expressions
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                matches!(
                    e.kind,
                    rgctl_analysis::operands::ExpressionKind::Construction(_)
                )
            })
        {
            let Some(span) = e.span else {
                continue;
            };
            let Some(raw) = nominal_names.get(&(file.clone(), span.start, span.end)) else {
                continue;
            };
            if let Some(id) = go_nominal_type(
                nodes,
                root,
                &file,
                raw,
                module.as_ref().map(|m| m.0.as_str()),
                &storage_types,
            ) {
                storage_allocations.insert(
                    (caller, rgctl_analysis::operands::ExpressionId(i as u32)),
                    id,
                );
            }
            if let Some(id) = go_nominal_type(
                nodes,
                root,
                &file,
                raw,
                module.as_ref().map(|m| m.0.as_str()),
                &initialization_free,
            ) {
                nominal_allocations.insert(
                    (caller, rgctl_analysis::operands::ExpressionId(i as u32)),
                    id,
                );
            }
        }
    }
    let mut origin = origin_joins::candidates(
        nodes,
        root,
        parents,
        native_calls,
        &result,
        &relations,
        cfgs,
        &procedural_nodes,
        &environment,
        &line_offsets,
        &initialization_free,
        &direct_functions,
        &private_functions,
        instantiations,
        &nominal_allocations,
        &storage_types,
        &storage_allocations,
        &storage_aliases,
        &context_nominal_types,
        preparation_failures,
        preparation_limited,
    )?;
    origin.coverage["receiverModuleInput"] = module
        .map(|(coordinate, sha256)| json!({"path":"go.mod","module":coordinate,"sha256":sha256}))
        .unwrap_or(Value::Null);
    result.extend(origin.calls);
    result.sort_by_key(|c| (c.from, c.to, c.reason));
    result.dedup_by(|a, b| a.from == b.from && a.to == b.to && a.reason == b.reason);
    Ok(JoinAnalysis {
        receiver_refinements: origin.receiver_refinements,
        candidates: result,
        origin_coverage: origin.coverage,
        context: origin.context,
    })
}

#[cfg(test)]
mod resolution_tests {
    use super::*;

    #[test]
    fn go_qualified_allocations_require_the_actual_module_import() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let concrete = Node::new(NodeType::Struct, "W")
            .with_qualified_name("dependency.W")
            .with_file_path("dependency/lib.go");
        let concrete_id = concrete.id;
        let admitted = HashSet::from([concrete_id]);
        for (alias, coordinate, expected) in [
            ("dependency", "example.test/demo/dependency", true),
            ("dep", "example.test/demo/dependency", true),
            ("dependency", "example.test/other/dependency", false),
        ] {
            let import = Node::new(NodeType::Import, alias)
                .with_qualified_name(coordinate)
                .with_file_path("application/app.go");
            let nodes = HashMap::from([(concrete_id, concrete.clone()), (import.id, import)]);
            let raw = format!("{alias}.W");
            assert_eq!(
                go_nominal_type(
                    &nodes,
                    dir.path(),
                    Path::new("application/app.go"),
                    &raw,
                    Some("example.test/demo"),
                    &admitted
                )
                .is_some(),
                expected
            );
            assert!(
                go_nominal_type(
                    &nodes,
                    dir.path(),
                    Path::new("application/app.go"),
                    &raw,
                    None,
                    &admitted
                )
                .is_none()
            );
        }
        Ok(())
    }

    #[test]
    fn module_metadata_and_nested_modules_are_not_guessed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(go_module(dir.path()).is_none());
        std::fs::write(
            dir.path().join("go.mod"),
            "module example.test/demo\n\ngo 1.18\n",
        )?;
        assert_eq!(go_module(dir.path()).unwrap().0, "example.test/demo");
        std::fs::write(dir.path().join("go.work"), "go 1.18\n")?;
        assert!(go_module(dir.path()).is_none());
        let concrete = Node::new(NodeType::Struct, "W")
            .with_qualified_name("dependency.W")
            .with_file_path("dependency/lib.go");
        let import = Node::new(NodeType::Import, "dependency")
            .with_qualified_name("example.test/demo/dependency")
            .with_file_path("application/app.go");
        let admitted = HashSet::from([concrete.id]);
        let nodes = HashMap::from([(concrete.id, concrete), (import.id, import)]);
        std::fs::create_dir(dir.path().join("dependency"))?;
        std::fs::write(
            dir.path().join("dependency/go.mod"),
            "module example.test/separate\n",
        )?;
        assert!(
            go_nominal_type(
                &nodes,
                dir.path(),
                Path::new("application/app.go"),
                "dependency.W",
                Some("example.test/demo"),
                &admitted
            )
            .is_none()
        );
        Ok(())
    }
}
