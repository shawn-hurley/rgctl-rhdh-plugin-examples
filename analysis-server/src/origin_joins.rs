//! Assemble canonical graph candidates for the native origin solver. No AST walking.
use super::receivers::Types;
use super::{Candidate, ReceiverRefinements, logical, name};
use anyhow::Result;
use rgctl_analysis::{
    call_bindings::{CallRef, InvocationTarget, ReceiverSupply, ValueRef},
    cfg::ControlFlowGraph,
    interprocedural_cfg::InterproceduralCFG,
    operands::*,
    value_origins::{
        CallableCandidate, ConstructorCandidate, MemberCandidate, OriginAnalysis, OriginDiagnostic,
        OriginInputs, OriginLimits, OriginRequirement, OriginSeed, callable_member_values,
    },
};
use rgctl_graph::{
    backend::{GraphBackend, MemoryBackend},
    schema::{Edge, EdgeType, Node, NodeType},
};
use rgctl_plugin_api::{Relation, RelationType};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub struct OriginJoin {
    pub receiver_refinements: ReceiverRefinements,
    pub calls: Vec<Candidate>,
    pub coverage: Value,
    pub context: Option<super::ContextDispatch>,
}
fn label(cfg: &ControlFlowGraph, mut id: ExpressionId) -> Option<&str> {
    for _ in 0..256 {
        match cfg.operands.expressions.get(id.0 as usize)?.kind {
            ExpressionKind::Name(n) | ExpressionKind::Member(n) => return cfg.operands.name(n),
            ExpressionKind::TypeApplication(head) => id = head,
            _ => return None,
        }
    }
    None
}
fn coordinate(r: &Relation, lines: &[usize]) -> Option<(u32, u32)> {
    if let (Some(start), Some(len)) = (r.call_site_start_byte, r.call_site_byte_len) {
        return Some((start, start.checked_add(u32::from(len))?));
    }
    let start = lines
        .get(r.location.start_line.checked_sub(1)?)?
        .checked_add(r.location.start_column)?;
    let end = lines
        .get(r.location.end_line.checked_sub(1)?)?
        .checked_add(r.location.end_column)?;
    Some((u32::try_from(start).ok()?, u32::try_from(end).ok()?))
}
fn matches_site(cfg: &ControlFlowGraph, site: &CallSiteFact, range: (u32, u32)) -> bool {
    let Some(callee) = cfg
        .operands
        .expressions
        .get(site.callee.0 as usize)
        .and_then(|e| e.span)
    else {
        return false;
    };
    range == (site.span.start, site.span.end)
        || range == (callee.start, callee.end)
        || (range.0 >= callee.start && range.1 == callee.end && range.0 < range.1)
}
fn convention(cfg: Option<&ControlFlowGraph>, site: &CallSiteFact) -> ReceiverSupply {
    if site.kind == InvocationKind::Construction {
        return ReceiverSupply::Constructed;
    }
    match cfg.map(|c| c.operands.receiver_convention) {
        Some(ReceiverConvention::None) | None => ReceiverSupply::None,
        _ => ReceiverSupply::Member,
    }
}
fn construction_types(
    types: &Types<'_>,
    cfg: &ControlFlowGraph,
    head: ExpressionId,
) -> BTreeSet<Uuid> {
    let mut result = BTreeSet::new();
    let mut seen = HashSet::new();
    let mut stack = vec![head];
    while let Some(id) = stack.pop() {
        if seen.len() >= 256 || !seen.insert(id) {
            continue;
        }
        let Some(expression) = cfg.operands.expressions.get(id.0 as usize) else {
            continue;
        };
        if let Some(raw) = label(cfg, id) {
            result.extend(types.ids(raw));
        }
        // Keep all resolvable construction-head references when the syntax is
        // compound; these are possible types, not a proof of name resolution.
        if !matches!(expression.kind, ExpressionKind::CallResult(_)) {
            stack.extend(
                expression
                    .inputs
                    .slice(&cfg.operands.inputs)
                    .unwrap_or(&[])
                    .iter()
                    .copied(),
            );
        }
    }
    result
}
#[allow(clippy::too_many_arguments)]
pub fn candidates(
    nodes: &HashMap<Uuid, Node>,
    root: &Path,
    parents: &HashMap<String, Vec<String>>,
    native_calls: &HashSet<(Uuid, Uuid)>,
    previous: &[Candidate],
    relations: &[Relation],
    cfgs: HashMap<Uuid, ControlFlowGraph>,
    procedural_nodes: &HashMap<Uuid, Node>,
    environment: &rgctl_analysis::environment::EnvironmentFacts,
    line_offsets: &HashMap<PathBuf, Vec<usize>>,
    initialization_free: &HashSet<Uuid>,
    direct_functions: &HashSet<Uuid>,
    private_functions: &HashSet<Uuid>,
    instantiations: &HashSet<(Uuid, Uuid)>,
    nominal_allocations: &HashMap<(Uuid, ExpressionId), Uuid>,
    storage_types: &HashSet<Uuid>,
    storage_allocations: &HashMap<(Uuid, ExpressionId), Uuid>,
    storage_aliases: &HashMap<ValueRef, ValueRef>,
    context_nominal_types: &HashSet<Uuid>,
    preparation_failures: Vec<Value>,
    preparation_limited: bool,
) -> Result<OriginJoin> {
    let types = Types::new(nodes, parents);
    let mut type_ids: Vec<_> = nodes
        .values()
        .filter(|n| {
            matches!(
                n.node_type,
                NodeType::Class | NodeType::Struct | NodeType::Interface | NodeType::Enum
            )
        })
        .map(|n| n.id)
        .collect();
    type_ids.sort();
    let mut known: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    for &(from, to) in native_calls {
        known.entry(from).or_default().insert(to);
    }
    for candidate in previous.iter().filter(|c| {
        matches!(
            c.reason,
            "stopgap-known-symbol-call" | "external-assignability-join"
        )
    }) {
        known
            .entry(candidate.from)
            .or_default()
            .insert(candidate.to);
    }
    let mut source_relations: HashMap<(&str, &str), Vec<(usize, &Relation)>> = HashMap::new();
    for (index, relation) in relations.iter().enumerate().filter(|(_, r)| {
        matches!(
            r.relation_type,
            RelationType::Calls | RelationType::Instantiates
        )
    }) {
        let identity = relation
            .metadata
            .get("from_identity_discriminator")
            .and_then(Value::as_str)
            .unwrap_or("");
        source_relations
            .entry((&relation.from, identity))
            .or_default()
            .push((index, relation));
    }
    let mut ordered: Vec<_> = cfgs.keys().copied().collect();
    ordered.sort();
    let mut receiver_refinements = ReceiverRefinements::new();
    let mut refinement_evidence = Vec::new();
    let mut program_roots = Vec::new();
    let mut direct_targets = Vec::new();
    let mut direct_bound_relations = HashSet::new();
    let mut seeds = HashSet::new();
    let mut initial = HashSet::new();
    let mut members = Vec::new();
    let mut callables = Vec::new();
    let mut constructors = Vec::new();
    let mut matched_relations = HashSet::new();
    let mut limited = preparation_limited;
    let limits = OriginLimits::default();
    for &caller in &ordered {
        let cfg = &cfgs[&caller];
        let node = &procedural_nodes[&caller];
        let file = logical(root, node.file_path.as_deref().unwrap_or(""));
        let lines = line_offsets.get(&file).map(Vec::as_slice).unwrap_or(&[]);
        let mut bound: HashMap<(&str, u32), Vec<(usize, &Relation, (u32, u32))>> = HashMap::new();
        for &(index, r) in source_relations
            .get(&(name(node), node.identity_discriminator().unwrap_or("")))
            .into_iter()
            .flatten()
        {
            if logical(root, &r.location.file) != file {
                continue;
            }
            if let Some(span) = coordinate(r, lines) {
                let terminal = r.to.rsplit('.').next().unwrap_or(&r.to);
                bound
                    .entry((terminal, span.1))
                    .or_default()
                    .push((index, r, span));
                if let Some(hint) = r
                    .to_qualified_hint
                    .as_deref()
                    .and_then(|q| q.rsplit('.').next())
                    .filter(|h| *h != terminal)
                {
                    bound
                        .entry((hint, span.1))
                        .or_default()
                        .push((index, r, span));
                }
            }
        }
        let mut exact_constructions = Vec::new();
        for (index, call) in cfg.operands.calls.iter().enumerate() {
            let call_ref = CallRef {
                caller,
                site: CallSiteId(index as u32),
            };
            let Some(method) = label(cfg, call.callee) else {
                continue;
            };
            let callee_end = cfg.operands.expressions[call.callee.0 as usize]
                .span
                .map(|s| s.end);
            let mut at = Vec::new();
            for end in [Some(call.span.end), callee_end].into_iter().flatten() {
                at.extend(
                    bound
                        .get(&(method, end))
                        .into_iter()
                        .flatten()
                        .filter(|(_, _, range)| matches_site(cfg, call, *range)),
                );
            }
            at.sort_by_key(|(index, _, _)| *index);
            at.dedup_by_key(|(index, _, _)| *index);
            let hinted_types: BTreeSet<_> = at
                .iter()
                .flat_map(|(_, relation, _)| {
                    [
                        relation.to_qualified_hint.as_deref(),
                        relation.to_type_hint.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    .flat_map(|hint| types.ids(hint))
                })
                .collect();
            // A possible origin seed is never an exclusion proof. For local
            // Java narrowing require a unique qualified Instantiates resolution
            // at this exact native source occurrence.
            if cfg.operands.language == OperandLanguage::Java
                && call.kind == InvocationKind::Construction
            {
                let allocations: Vec<_> = at
                    .iter()
                    .filter(|(_, r, _)| r.relation_type == RelationType::Instantiates)
                    .collect();
                let exact: Option<Vec<_>> = allocations
                    .iter()
                    .map(|(_, r, _)| {
                        r.to_qualified_hint
                            .as_deref()
                            .and_then(|q| types.exact_java_class(q))
                    })
                    .collect();
                if let Some(ids) = exact {
                    let ids: HashSet<_> = ids.into_iter().collect();
                    if ids.len() == 1 && ids.iter().all(|id| initialization_free.contains(id)) {
                        exact_constructions.push(
                            rgctl_analysis::receiver_closure::ResolvedConstruction {
                                value: ValueRef {
                                    function: caller,
                                    expression: call.result,
                                },
                                type_id: *ids.iter().next().expect("one construction type"),
                            },
                        );
                    }
                }
            }
            if cfg.operands.language == OperandLanguage::TypeScript
                && call.kind == InvocationKind::Construction
                && matches!(
                    cfg.operands.expressions[call.callee.0 as usize].kind,
                    ExpressionKind::Name(_)
                )
            {
                let ids = types.ids(method);
                if ids.len() == 1
                    && let Some(&id) = ids.iter().next()
                    && initialization_free.contains(&id)
                    && instantiations.contains(&(caller, id))
                {
                    exact_constructions.push(
                        rgctl_analysis::receiver_closure::ResolvedConstruction {
                            value: ValueRef {
                                function: caller,
                                expression: call.result,
                            },
                            type_id: id,
                        },
                    );
                }
            }
            for &(index, relation, _) in &at {
                if relation.relation_type != RelationType::Calls {
                    continue;
                }
                let Some(q) = relation.to_qualified_hint.as_deref() else {
                    continue;
                };
                let found: Vec<_> = nodes
                    .values()
                    .filter(|n| {
                        n.node_type == NodeType::Function
                            && direct_functions.contains(&n.id)
                            && name(n) == q
                            && &*n.name == method
                    })
                    .collect();
                if found.len() == 1 && cfgs.contains_key(&found[0].id) {
                    direct_targets.push(InvocationTarget {
                        call: call_ref,
                        callee: found[0].id,
                        receiver: ReceiverSupply::None,
                    });
                    direct_bound_relations.insert(index);
                }
            }
            for (relation_index, relation, _) in at {
                matched_relations.insert(relation_index);
                if relation.relation_type != RelationType::Calls {
                    continue;
                }
                for &callee in known.get(&caller).into_iter().flatten() {
                    let Some(target) = nodes.get(&callee) else {
                        continue;
                    };
                    if &*target.name != method
                        && name(target) != relation.to
                        && Some(name(target)) != relation.to_qualified_hint.as_deref()
                    {
                        continue;
                    }
                    if relation
                        .metadata
                        .get("to_identity_discriminator")
                        .and_then(Value::as_str)
                        .is_some_and(|id| target.identity_discriminator().unwrap_or("") != id)
                    {
                        continue;
                    }
                    if initial.len() >= limits.max_targets {
                        limited = true;
                        break;
                    }
                    initial.insert(InvocationTarget {
                        call: call_ref,
                        callee,
                        receiver: convention(cfgs.get(&callee), call),
                    });
                }
            }
            let possible_types = if call.kind == InvocationKind::Construction
                || (cfg.operands.language == OperandLanguage::Python
                    && matches!(
                        cfg.operands.expressions[call.callee.0 as usize].kind,
                        ExpressionKind::Name(_)
                    )) {
                let mut possible = construction_types(&types, cfg, call.callee);
                possible.extend(hinted_types);
                possible
            } else {
                BTreeSet::new()
            };
            for type_id in possible_types {
                if seeds.len() >= limits.max_facts {
                    limited = true;
                    break;
                }
                seeds.insert(OriginSeed {
                    value: ValueRef {
                        function: caller,
                        expression: call.result,
                    },
                    type_id: Some(type_id),
                });
                for callee in types.constructor_targets(type_id) {
                    if initial.len() >= limits.max_targets {
                        limited = true;
                        break;
                    }
                    initial.insert(InvocationTarget {
                        call: call_ref,
                        callee,
                        receiver: ReceiverSupply::Constructed,
                    });
                }
            }
            if call.receiver.is_some() {
                for &receiver_type in &type_ids {
                    for callee in types.origin_targets(receiver_type, method) {
                        if members.len() >= limits.max_member_candidates {
                            limited = true;
                            break;
                        }
                        members.push(MemberCandidate {
                            target: InvocationTarget {
                                call: call_ref,
                                callee,
                                receiver: convention(cfgs.get(&callee), call),
                            },
                            receiver_type,
                        });
                    }
                    if members.len() >= limits.max_member_candidates {
                        break;
                    }
                }
            }
        }
        if !exact_constructions.is_empty() && nodes.contains_key(&caller) {
            let pdg = rgctl_analysis::ProgramDependenceGraph::build(cfg, &[])?;
            let flow = rgctl_analysis::local_value_flow::LocalValueFlow::build(
                cfg,
                &pdg,
                Default::default(),
            );
            let closed = rgctl_analysis::receiver_closure::closed_local_receivers(
                caller,
                cfg,
                &flow,
                &exact_constructions,
                250_000,
            );
            let closed_by_site: HashMap<_, _> = closed.iter().map(|c| (c.call.site, c)).collect();
            let mut methods: BTreeMap<&str, Option<HashSet<Uuid>>> = BTreeMap::new();
            // The overlay stores caller/target pairs, not individual call sites.
            // A differently named callback/reflective/unknown invocation could
            // also reach a target named `execute`. Do not narrow that pair just
            // because every *direct* execute invocation was qualified.
            let all_invocations_closed = cfg.operands.calls.iter().enumerate().all(|(i, c)| {
                (c.kind == InvocationKind::Construction
                    && exact_constructions
                        .iter()
                        .any(|r| r.value.expression == c.result))
                    || closed_by_site.contains_key(&CallSiteId(i as u32))
            });
            for (i, call) in cfg.operands.calls.iter().enumerate() {
                if call.kind == InvocationKind::Construction {
                    continue;
                }
                let Some(method) = label(cfg, call.callee) else {
                    continue;
                };
                let allowed = methods
                    .entry(method)
                    .or_insert_with(|| Some(HashSet::new()));
                match (allowed.as_mut(), closed_by_site.get(&CallSiteId(i as u32))) {
                    (Some(targets), Some(proof)) => {
                        for &ty in &proof.types {
                            targets.extend(types.closed_targets(ty, method));
                        }
                    }
                    _ => *allowed = None, // any unresolved repeated call keeps broad dispatch
                }
            }
            for (method, allowed) in methods {
                if let Some(allowed) = allowed.filter(|a| all_invocations_closed && !a.is_empty()) {
                    refinement_evidence.push(json!({"caller":caller,"method":method,"allowedTargets":allowed.iter().copied().collect::<BTreeSet<_>>(),"proofs":closed.iter().filter(|c|label(cfg,cfg.operands.calls[c.call.site.0 as usize].callee)==Some(method)).collect::<Vec<_>>(),"scope":"all invocations locally qualified; exact named Java allocations without declared initialization effects"}));
                    receiver_refinements.insert((caller, method.to_owned()), allowed);
                }
            }
        }
        program_roots.extend(exact_constructions);
        for (index, expression) in cfg.operands.expressions.iter().enumerate() {
            if let Some(&type_id) = nominal_allocations.get(&(caller, ExpressionId(index as u32))) {
                program_roots.push(rgctl_analysis::receiver_closure::ResolvedConstruction {
                    value: ValueRef {
                        function: caller,
                        expression: ExpressionId(index as u32),
                    },
                    type_id,
                });
            }
            if let ExpressionKind::Construction(head) = expression.kind {
                for type_id in construction_types(&types, cfg, head) {
                    if seeds.len() >= limits.max_facts {
                        limited = true;
                        break;
                    }
                    seeds.insert(OriginSeed {
                        value: ValueRef {
                            function: caller,
                            expression: ExpressionId(index as u32),
                        },
                        type_id: Some(type_id),
                    });
                }
            }
        }
    }
    let mut functions_by_file: HashMap<(PathBuf, String), Vec<Uuid>> = HashMap::new();
    for (&id, node) in nodes
        .iter()
        .filter(|(_, n)| n.node_type == rgctl_graph::schema::NodeType::Function)
    {
        let owner = name(node)
            .rsplit_once('.')
            .map(|(owner, _)| owner)
            .unwrap_or("");
        if types.ids(owner).is_empty() && !name(node).ends_with(".<init>") {
            functions_by_file
                .entry((
                    logical(root, node.file_path.as_deref().unwrap_or("")),
                    node.name.to_string(),
                ))
                .or_default()
                .push(id);
        }
    }
    for ids in functions_by_file.values_mut() {
        ids.sort();
    }
    for &function in &ordered {
        let cfg = &cfgs[&function];
        let file = logical(
            root,
            procedural_nodes[&function]
                .file_path
                .as_deref()
                .unwrap_or(""),
        );
        for (index, expression) in cfg.operands.expressions.iter().enumerate() {
            if callables.len() >= limits.max_member_candidates {
                limited = true;
                break;
            }
            let value = ValueRef {
                function,
                expression: ExpressionId(index as u32),
            };
            match expression.kind {
                ExpressionKind::Name(n) => {
                    if let Some(label) = cfg.operands.name(n) {
                        for type_id in types.ids(label) {
                            let bodies = types.constructor_targets(type_id);
                            if bodies.is_empty() {
                                if constructors.len() < limits.max_member_candidates {
                                    constructors.push(ConstructorCandidate {
                                        value,
                                        type_id,
                                        callee: None,
                                    });
                                } else {
                                    limited = true;
                                }
                            } else {
                                for callee in bodies {
                                    if constructors.len() >= limits.max_member_candidates {
                                        limited = true;
                                        break;
                                    }
                                    constructors.push(ConstructorCandidate {
                                        value,
                                        type_id,
                                        callee: Some(callee),
                                    });
                                }
                            }
                        }

                        for &callee in functions_by_file
                            .get(&(file.clone(), label.to_owned()))
                            .into_iter()
                            .flatten()
                        {
                            if callables.len() >= limits.max_member_candidates {
                                limited = true;
                                break;
                            }
                            callables.push(CallableCandidate {
                                value,
                                callee,
                                receiver: ReceiverSupply::None,
                                requirement: None,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        for fact in callable_member_values(&cfg.operands) {
            if callables.len() >= limits.max_member_candidates {
                limited = true;
                break;
            }
            let value = ValueRef {
                function,
                expression: fact.value,
            };
            let receiver = ValueRef {
                function,
                expression: fact.receiver,
            };
            let method = fact.member.and_then(|n| cfg.operands.name(n));
            for &type_id in &type_ids {
                if callables.len() >= limits.max_member_candidates {
                    limited = true;
                    break;
                }
                let targets = method.map_or_else(
                    || types.origin_all_targets(type_id),
                    |m| types.origin_targets(type_id, m),
                );
                for callee in targets {
                    if callables.len() >= limits.max_member_candidates {
                        limited = true;
                        break;
                    }
                    callables.push(CallableCandidate {
                        value,
                        callee,
                        receiver: ReceiverSupply::BoundValue(receiver),
                        requirement: Some(OriginRequirement {
                            value: receiver,
                            type_id,
                        }),
                    });
                }
            }
            if let Some(owner) = label(cfg, fact.receiver) {
                for type_id in types.ids(owner) {
                    if callables.len() >= limits.max_member_candidates {
                        limited = true;
                        break;
                    }
                    let targets = method.map_or_else(
                        || types.origin_all_targets(type_id),
                        |m| types.origin_targets(type_id, m),
                    );
                    for callee in targets {
                        if callables.len() >= limits.max_member_candidates {
                            limited = true;
                            break;
                        }
                        callables.push(CallableCandidate {
                            value,
                            callee,
                            receiver: ReceiverSupply::FirstArgument,
                            requirement: None,
                        });
                    }
                }
            }
        }
    }
    for literal in &environment.callables {
        if callables.len() >= limits.max_member_candidates {
            limited = true;
            break;
        }
        callables.push(CallableCandidate {
            value: literal.value,
            callee: literal.callee,
            receiver: ReceiverSupply::None,
            requirement: None,
        });
    }
    let mut backend = MemoryBackend::new();
    // Hydrate only the explicitly bounded prepared functions, sharing Arc fields.
    for id in &ordered {
        backend.insert_node(procedural_nodes[id].clone())?;
    }
    for &(from, to) in native_calls {
        if cfgs.contains_key(&from) && cfgs.contains_key(&to) {
            backend.insert_edge(Edge::new(from, to, EdgeType::Calls))?;
        }
    }
    let prepared_count = cfgs.len();
    let icfg = InterproceduralCFG::from_cfg_archive(&backend, cfgs)?;
    let seeds: Vec<_> = seeds.into_iter().collect();
    let initial: Vec<_> = initial.into_iter().collect();
    direct_targets.sort_by_key(|t| (t.call.caller, t.call.site.0, t.callee));
    direct_targets.dedup();
    let closed_formals: HashSet<_> = private_functions
        .iter()
        .copied()
        .filter(|id| {
            let target = &nodes[id];
            let mut count = 0;
            for (index, r) in relations.iter().enumerate().filter(|(_, r)| {
                r.relation_type == RelationType::Calls
                    && r.to.rsplit('.').next() == Some(&*target.name)
            }) {
                if r.to_qualified_hint.as_deref() != Some(name(target))
                    || !direct_bound_relations.contains(&index)
                {
                    return false;
                }
                count += 1;
            }
            count > 0
                && native_calls
                    .iter()
                    .filter(|(_, to)| to == id)
                    .all(|(from, _)| {
                        direct_targets
                            .iter()
                            .any(|t| t.call.caller == *from && t.callee == *id)
                    })
        })
        .collect();
    let program = rgctl_analysis::receiver_program::closed_program_receivers(
        &icfg,
        rgctl_analysis::receiver_program::ReceiverProgramInputs {
            functions: &ordered,
            roots: &program_roots,
            targets: &direct_targets,
            closed_formals: &closed_formals,
        },
        limits,
    );
    if !limited && !program.limited {
        let closed_by_site: HashMap<_, _> = program.receivers.iter().map(|r| (r.call, r)).collect();
        let direct_by_site: HashMap<_, _> = direct_targets.iter().map(|t| (t.call, t)).collect();
        for &caller in &ordered {
            if !nodes.contains_key(&caller) {
                continue;
            }
            let cfg = &icfg.function_cfgs[&caller];
            let qualified = cfg.operands.calls.iter().enumerate().all(|(i, c)| {
                let call = CallRef {
                    caller,
                    site: CallSiteId(i as u32),
                };
                closed_by_site.contains_key(&call)
                    || direct_by_site.contains_key(&call)
                    || (c.kind == InvocationKind::Construction
                        && program_roots.iter().any(|r| {
                            r.value
                                == ValueRef {
                                    function: caller,
                                    expression: c.result,
                                }
                        }))
            });
            if !qualified {
                continue;
            }
            let mut allowed_by_method: BTreeMap<&str, Option<HashSet<Uuid>>> = BTreeMap::new();
            for (i, c) in cfg.operands.calls.iter().enumerate() {
                if c.kind == InvocationKind::Construction {
                    continue;
                }
                let Some(method) = label(cfg, c.callee) else {
                    continue;
                };
                let allowed = allowed_by_method
                    .entry(method)
                    .or_insert_with(|| Some(HashSet::new()));
                match (
                    allowed.as_mut(),
                    closed_by_site.get(&CallRef {
                        caller,
                        site: CallSiteId(i as u32),
                    }),
                ) {
                    (Some(targets), Some(proof)) => {
                        for &ty in &proof.types {
                            targets.extend(types.closed_targets(ty, method));
                        }
                    }
                    _ => *allowed = None,
                }
            }
            for (method, allowed) in allowed_by_method {
                if let Some(allowed) = allowed.filter(|v| !v.is_empty()) {
                    refinement_evidence.push(json!({"caller":caller,"method":method,"allowedTargets":allowed.iter().copied().collect::<BTreeSet<_>>(),"proofs":program.receivers.iter().filter(|r|r.call.caller==caller && label(cfg,cfg.operands.calls[r.call.site.0 as usize].callee)==Some(method)).collect::<Vec<_>>(),"scope":"complete native local and direct argument/return producers; merged admitted callers"}));
                    receiver_refinements.insert((caller, method.to_owned()), allowed);
                }
            }
        }
    }
    let output = OriginAnalysis::build_with_environment(
        &icfg,
        OriginInputs {
            seeds: &seeds,
            targets: &initial,
            members: &members,
            callables: &callables,
            constructors: &constructors,
        },
        environment,
        limits,
    );
    let mut storage_roots: Vec<_> = storage_allocations
        .iter()
        .map(|(&(function, expression), &type_id)| {
            rgctl_analysis::receiver_closure::ResolvedConstruction {
                value: ValueRef {
                    function,
                    expression,
                },
                type_id,
            }
        })
        .collect();
    for (&function, cfg) in &icfg.function_cfgs {
        for c in cfg
            .operands
            .calls
            .iter()
            .filter(|c| c.kind == InvocationKind::Construction)
        {
            let Some(raw) = label(cfg, c.callee) else {
                continue;
            };
            if !matches!(
                cfg.operands.expressions[c.callee.0 as usize].kind,
                ExpressionKind::Name(_)
            ) {
                continue;
            }
            let ids: Vec<_> = types
                .ids(raw)
                .into_iter()
                .filter(|id| matches!(nodes[id].node_type, NodeType::Class | NodeType::Struct))
                .collect();
            if ids.len() != 1 || !storage_types.contains(&ids[0]) {
                continue;
            }
            let type_id = ids[0];
            let admissible = match cfg.operands.language {
                OperandLanguage::Java => {
                    name(&nodes[&type_id]) == raw
                        || (&*nodes[&type_id].name == raw
                            && nodes[&type_id].file_path == procedural_nodes[&function].file_path)
                        || instantiations.contains(&(function, type_id))
                }
                OperandLanguage::TypeScript => {
                    instantiations.contains(&(function, type_id))
                        || nodes.values().any(|n| {
                            if n.node_type != NodeType::Import
                                || n.file_path != procedural_nodes[&function].file_path
                                || n.get_property("local") != Some(raw)
                            {
                                return false;
                            }
                            let Some(module) =
                                n.get_property("module").filter(|m| m.starts_with('.'))
                            else {
                                return false;
                            };
                            if n.name.rsplit(':').next() != Some(&*nodes[&type_id].name) {
                                return false;
                            }
                            let Some(file) = n.file_path.as_deref().map(Path::new) else {
                                return false;
                            };
                            let Some(directory) = file.parent() else {
                                return false;
                            };
                            let path = directory.join(module);
                            let mut normalized = PathBuf::new();
                            for part in path.components() {
                                match part {
                                    std::path::Component::ParentDir => {
                                        normalized.pop();
                                    }
                                    std::path::Component::CurDir => {}
                                    other => normalized.push(other.as_os_str()),
                                }
                            }
                            nodes[&type_id].file_path.as_deref().is_some_and(|p| {
                                [
                                    normalized.with_extension("ts"),
                                    normalized.with_extension("tsx"),
                                    normalized.join("index.ts"),
                                ]
                                .iter()
                                .any(|v| v == Path::new(p))
                            })
                        })
                }
                _ => false,
            };
            if admissible {
                storage_roots.push(rgctl_analysis::receiver_closure::ResolvedConstruction {
                    value: ValueRef {
                        function,
                        expression: c.result,
                    },
                    type_id,
                });
            }
        }
    }
    let mut storage_targets: Vec<_> = initial
        .iter()
        .copied()
        .chain(direct_targets.iter().copied())
        .chain(output.receiver_targets.iter().map(|r| r.target))
        .chain(output.callable_targets.iter().map(|r| r.target))
        .collect();
    storage_targets.sort_by_key(|t| (t.call.caller, t.call.site.0, t.callee));
    storage_targets.dedup();
    let mut storage_calls = Vec::new();
    let storage_coverage = super::storage_receivers::refine(
        nodes,
        &types,
        &icfg,
        &output,
        environment,
        &storage_roots,
        storage_types,
        storage_aliases,
        &storage_targets,
        &mut receiver_refinements,
        &mut storage_calls,
        limited,
    );
    let mut groups: BTreeMap<(Uuid, Uuid), Vec<Value>> = BTreeMap::new();
    let mut declared_by_call: HashMap<CallRef, HashSet<Uuid>> = HashMap::new();
    for target in &initial {
        declared_by_call
            .entry(target.call)
            .or_default()
            .insert(target.callee);
    }
    let mut declarations: HashMap<(Uuid, Uuid), HashSet<Uuid>> = HashMap::new();
    for evidence in &output.receiver_targets {
        let origin = &output.seeds[evidence.origin.0 as usize];
        declarations
            .entry((evidence.target.call.caller, evidence.target.callee))
            .or_default()
            .extend(
                declared_by_call
                    .get(&evidence.target.call)
                    .into_iter()
                    .flatten()
                    .copied(),
            );
        groups.entry((evidence.target.call.caller,evidence.target.callee)).or_default().push(json!({"call":evidence.target.call,"origin":origin,"receiverSupply":evidence.target.receiver}));
    }
    let mut diagnostics: BTreeMap<&str, usize> = BTreeMap::new();
    for diagnostic in &output.diagnostics {
        let category = match diagnostic {
            OriginDiagnostic::MissingFunction(_) => "missing-function",
            OriginDiagnostic::Environment(_) => "environment-coverage",
            OriginDiagnostic::InvalidValue(_) | OriginDiagnostic::InvalidCall(_) => {
                "invalid-reference"
            }
            OriginDiagnostic::Local { .. } => "local-coverage",
            OriginDiagnostic::Binding { .. } | OriginDiagnostic::BindingError(_) => {
                "binding-coverage"
            }
            OriginDiagnostic::UnsupportedTransfer { .. } => "unsupported-transfer",
            OriginDiagnostic::ImplicitValue { .. } => "implicit-value",
            OriginDiagnostic::UnresolvedCall(_) => "unresolved-call",
            OriginDiagnostic::UnresolvedReceiver { .. } => "unresolved-receiver",
            OriginDiagnostic::LimitReached => "query-limit",
        };
        *diagnostics.entry(category).or_default() += 1;
    }
    let mut calls = storage_calls;
    for evidence in &output.callable_targets {
        calls.push(Candidate {
            from: evidence.target.call.caller,
            to: evidence.target.callee,
            declared_target: Some(evidence.target.callee),
            reason: "native-callable-origin-target",
            evidence: json!({"invocation":evidence.target,"origin":output.seeds[evidence.origin.0 as usize],"callableValues":output.callable_values.iter().filter(|v|v.origin==evidence.origin).collect::<Vec<_>>(),"contextPolicy":output.context_policy,"assumption":"canonical possible callable values; callers and receiver alternatives merge"}),
        });
    }
    for target in &direct_targets {
        calls.push(Candidate {
            from: target.call.caller,
            to: target.callee,
            declared_target: Some(target.callee),
            reason: "native-complete-direct-call",
            evidence: json!({"invocation":target,"scope":"qualified direct free/static function"}),
        });
    }
    for target in &initial {
        if target.receiver == ReceiverSupply::Constructed {
            calls.push(Candidate {
                from: target.call.caller,
                to: target.callee,
                declared_target: Some(target.callee),
                reason: "native-constructor-target",
                evidence: json!({"invocation":target,"assumption":"native construction possible-type candidate and canonical constructor owner","contextPolicy":output.context_policy}),
            });
        }
    }
    for ((from, to), evidence) in groups {
        let mut declared = declarations.remove(&(from, to)).unwrap_or_default();
        let declared_target = if declared.len() == 1 {
            declared.drain().next()
        } else {
            None
        };
        calls.push(Candidate{from,to,declared_target,reason:"native-value-origin-target",evidence:json!({"invocations":evidence,"contextPolicy":output.context_policy,"nativeTransferCount":output.transfers.len(),"assumptions":["construction head types are conservative native-symbol candidates","caller contexts are merged","unsupported handoffs remain partial"],"diagnosticCounts":diagnostics})});
    }
    calls.retain(|call| nodes.contains_key(&call.from) && nodes.contains_key(&call.to));
    let mut context_roots = program_roots.clone();
    context_roots.extend(storage_roots.iter().copied());
    context_roots.extend(super::context_dispatch::nominal_roots(
        &icfg,
        nodes,
        &types,
        context_nominal_types,
    ));
    let context = super::ContextDispatch::build(
        icfg,
        nodes,
        &types,
        &context_roots,
        &direct_targets,
        &storage_targets,
        limited,
    );
    let context_available = context.is_some();
    Ok(OriginJoin {
        receiver_refinements,
        context,
        calls,
        coverage: json!({"contextReceiverQuery":{"available":context_available,"policy":"balanced explicit invocation contexts; complete direct bindings only","roots":context_roots.len(),"admittedSimpleTypes":context_nominal_types},"storedReceiverRefinement":storage_coverage,"storageAllocationRoots":storage_roots.iter().map(|r|json!({"value":r.value,"typeId":r.type_id})).collect::<Vec<_>>(),"storageTypes":storage_types,"receiverProgram":program,"closedFormalFunctions":closed_formals.len(),"receiverRefinements":refinement_evidence,"status":"partial","engine":"rgctl-native-value-origins","contextPolicy":output.context_policy,"preparedFunctions":prepared_count,"environmentTransferCount":environment.transfers.len(),"implicitMemberReadCount":environment.member_reads.len(),"literalCallableCount":environment.callables.len(),"preparationFailures":preparation_failures,"preparationLimited":limited,"seedCount":output.seeds.len(),"valueCount":output.values.len(),"receiverTargetCount":output.receiver_targets.len(),"callableValueCount":output.callable_values.len(),"callableTargetCount":output.callable_targets.len(),"constructorValueCount":output.constructor_values.len(),"constructionCount":output.construction_evidence.len(),"transferCount":output.transfers.len(),"associatedRelations":matched_relations.len(),"diagnosticCounts":diagnostics,"diagnosticTotal":output.diagnostics.len(),"diagnosticExamples":output.diagnostics.iter().take(100).collect::<Vec<_>>(),"diagnosticsOmitted":output.diagnostics.len().saturating_sub(100)}),
    })
}
