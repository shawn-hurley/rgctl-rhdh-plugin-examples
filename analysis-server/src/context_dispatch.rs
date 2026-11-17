//! Entry-sensitive projection of the existing conservative call graph.
//! All receiver proofs come from rgctl's balanced invocation-context query.
use super::receivers::Types;
use rgctl_analysis::{
    call_bindings::{CallRef, InvocationTarget, ValueRef},
    context_receivers::ContextReceiverProgram,
    interprocedural_cfg::InterproceduralCFG,
    operands::*,
    receiver_closure::ResolvedConstruction,
    value_origins::OriginLimits,
};
use rgctl_graph::schema::{Node, NodeType};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub struct ContextDispatch {
    icfg: InterproceduralCFG,
    program: ContextReceiverProgram,
    direct: HashMap<(Uuid, Uuid), Vec<InvocationTarget>>,
    direct_sites: HashSet<CallRef>,
    known: HashMap<(Uuid, Uuid), Vec<InvocationTarget>>,
    roots: HashSet<ValueRef>,
    methods: HashMap<Uuid, String>,
    allowed: HashMap<(Uuid, String), HashSet<Uuid>>,
    contracts: HashSet<Uuid>,
}
fn label(cfg: &rgctl_analysis::cfg::ControlFlowGraph, mut e: ExpressionId) -> Option<&str> {
    for _ in 0..128 {
        match cfg.operands.expressions.get(e.0 as usize)?.kind {
            ExpressionKind::Name(n) | ExpressionKind::Member(n) => return cfg.operands.name(n),
            ExpressionKind::TypeApplication(head) => e = head,
            _ => return None,
        }
    }
    None
}
impl ContextDispatch {
    pub fn build(
        icfg: InterproceduralCFG,
        nodes: &HashMap<Uuid, Node>,
        types: &Types<'_>,
        roots: &[ResolvedConstruction],
        direct: &[InvocationTarget],
        known: &[InvocationTarget],
        limited: bool,
    ) -> Option<Self> {
        if limited || direct.is_empty() {
            return None;
        }
        // Heap/state mutation needs context-sensitive effect summaries too.
        // Keep the existing graph whenever those facts would be required.
        if icfg
            .function_cfgs
            .values()
            .any(|c| !c.operands.field_writes.is_empty() || !c.operands.element_writes.is_empty())
        {
            return None;
        }
        let owners: HashSet<_> = roots.iter().map(|r| r.type_id).collect();
        let names: HashSet<_> = icfg
            .function_cfgs
            .values()
            .flat_map(|cfg| {
                cfg.operands
                    .calls
                    .iter()
                    .filter_map(|c| label(cfg, c.callee))
            })
            .collect();
        if owners.len().saturating_mul(names.len()) > 65_536 {
            return None;
        }
        let mut allowed = HashMap::new();
        let mut allowed_count = 0usize;
        for owner in owners {
            for &method in &names {
                let targets = types.closed_targets(owner, method);
                allowed_count = allowed_count.saturating_add(targets.len());
                if allowed_count > 250_000 {
                    return None;
                }
                allowed.insert((owner, method.to_owned()), targets);
            }
        }
        let functions: Vec<_> = icfg.function_cfgs.keys().copied().collect();
        let program = ContextReceiverProgram::build(
            &icfg,
            &functions,
            roots,
            direct,
            OriginLimits::default(),
        );
        if program.limited {
            return None;
        }
        let mut by_edge: HashMap<_, Vec<_>> = HashMap::new();
        let mut direct_sites = HashSet::new();
        for &target in direct {
            by_edge
                .entry((target.call.caller, target.callee))
                .or_default()
                .push(target);
            direct_sites.insert(target.call);
        }
        let mut known_by_edge: HashMap<_, Vec<_>> = HashMap::new();
        for &target in known {
            known_by_edge
                .entry((target.call.caller, target.callee))
                .or_default()
                .push(target);
        }
        let methods = nodes
            .values()
            .filter(|n| n.node_type == NodeType::Function)
            .map(|n| (n.id, n.name.to_string()))
            .collect();
        let contracts = nodes
            .values()
            .filter(|n| n.node_type == NodeType::Function)
            .filter(|n| {
                n.qualified_name
                    .as_deref()
                    .and_then(|q| q.rsplit_once('.'))
                    .is_some_and(|(owner, _)| {
                        nodes.values().any(|n| {
                            n.node_type == NodeType::Interface
                                && n.qualified_name.as_deref() == Some(owner)
                        })
                    })
            })
            .map(|n| n.id)
            .collect();
        Some(Self {
            icfg,
            program,
            direct: by_edge,
            direct_sites,
            known: known_by_edge,
            roots: roots.iter().map(|r| r.value).collect(),
            methods,
            allowed,
            contracts,
        })
    }
    /// Exclude only a complete direct edge whose every selected invocation has
    /// no remaining path to the current change seeds. Other graph edges remain.
    pub fn excludes(
        &self,
        from: Uuid,
        to: Uuid,
        seeds: &HashSet<Uuid>,
        outgoing: &HashMap<Uuid, Vec<Uuid>>,
        budget: &mut usize,
    ) -> bool {
        let Some(targets) = self.direct.get(&(from, to)) else {
            return false;
        };
        if self
            .known
            .get(&(from, to))
            .into_iter()
            .flatten()
            .any(|t| !targets.contains(t))
        {
            return false;
        }
        if !self.safe_function(from, &[], budget) {
            return false;
        }
        targets
            .iter()
            .all(|&target| !self.reaches(target.callee, vec![target], seeds, outgoing, budget))
    }
    fn proofs(
        &self,
        function: Uuid,
        path: &[InvocationTarget],
        budget: &mut usize,
    ) -> Option<HashMap<CallRef, Vec<Uuid>>> {
        let steps = (*budget).min(2_000);
        if steps == 0 {
            return None;
        }
        *budget -= steps;
        Some(
            self.program
                .receivers(&self.icfg, function, path, steps)
                .into_iter()
                .map(|r| (r.call, r.types))
                .collect(),
        )
    }
    fn safe_function(&self, function: Uuid, path: &[InvocationTarget], budget: &mut usize) -> bool {
        let Some(cfg) = self.icfg.function_cfgs.get(&function) else {
            return false;
        };
        if cfg
            .operands
            .diagnostics
            .iter()
            .any(|d| d.kind != OperandDiagnosticKind::NestedCallable)
        {
            return false;
        }
        let Some(proofs) = self.proofs(function, path, budget) else {
            return false;
        };
        cfg.operands.calls.iter().enumerate().all(|(i, c)| {
            let call = CallRef {
                caller: function,
                site: CallSiteId(i as u32),
            };
            self.direct_sites.contains(&call)
                || self.roots.contains(&ValueRef {
                    function,
                    expression: c.result,
                })
                || proofs.get(&call).is_some_and(|types| {
                    !types.is_empty()
                        && types.iter().all(|ty| {
                            label(cfg, c.callee).is_some_and(|method| {
                                self.allowed
                                    .get(&(*ty, method.to_owned()))
                                    .is_some_and(|v| !v.is_empty())
                            })
                        })
                })
        })
    }
    fn receiver_excludes(
        &self,
        from: Uuid,
        to: Uuid,
        proofs: &HashMap<CallRef, Vec<Uuid>>,
    ) -> bool {
        if self.contracts.contains(&to) {
            return false;
        }
        let Some(method) = self.methods.get(&to) else {
            return false;
        };
        let Some(cfg) = self.icfg.function_cfgs.get(&from) else {
            return false;
        };
        let matching: Vec<_> = cfg
            .operands
            .calls
            .iter()
            .enumerate()
            .filter(|(_, c)| label(cfg, c.callee) == Some(method))
            .collect();
        if matching.is_empty() || matching.iter().any(|(_, c)| c.receiver.is_none()) {
            return false;
        }
        // A bound/indirect invocation of the same target remains separate from
        // ordinary receiver calls, even if its source-level label is different.
        if self.known.get(&(from, to)).into_iter().flatten().any(|t| {
            cfg.operands
                .calls
                .get(t.call.site.0 as usize)
                .is_none_or(|c| c.receiver.is_none() || label(cfg, c.callee) != Some(method))
        }) {
            return false;
        }
        matching.iter().all(|(i, _)| {
            let call = CallRef {
                caller: from,
                site: CallSiteId(*i as u32),
            };
            proofs.get(&call).is_some_and(|types| {
                !types.is_empty()
                    && types.iter().all(|ty| {
                        self.allowed
                            .get(&(*ty, method.clone()))
                            .is_some_and(|allowed| !allowed.contains(&to))
                    })
            })
        })
    }
    fn reaches(
        &self,
        function: Uuid,
        path: Vec<InvocationTarget>,
        seeds: &HashSet<Uuid>,
        outgoing: &HashMap<Uuid, Vec<Uuid>>,
        budget: &mut usize,
    ) -> bool {
        let mut pending = vec![(function, path)];
        let mut seen = HashSet::new();
        while let Some((function, path)) = pending.pop() {
            if *budget == 0 || path.len() > 24 || seen.len() > 512 {
                return true;
            }
            *budget -= 1;
            if seeds.contains(&function) {
                return true;
            }
            if !seen.insert((function, path.clone())) {
                continue;
            }
            let Some(next) = outgoing.get(&function) else {
                continue;
            };
            if !self.safe_function(function, &path, budget) {
                return true;
            }
            let Some(proofs) = self.proofs(function, &path, budget) else {
                return true;
            };
            for &to in next {
                if pending.len() >= 2_048 {
                    return true;
                }
                if self.receiver_excludes(function, to, &proofs) {
                    continue;
                }
                if let Some(targets) = self.direct.get(&(function, to)) {
                    for &target in targets {
                        if pending.len() >= 2_048 {
                            return true;
                        }
                        let mut child = path.clone();
                        child.push(target);
                        pending.push((to, child));
                    }
                } else {
                    pending.push((to, Vec::new()));
                }
            }
        }
        false
    }
}

/// Admit only native simple classes whose local/import binding resolves to the
/// same source declaration. Ordinary functions sharing a class name, assignment
/// shadows, constructor overrides, and heap mutation cannot establish a root.
pub fn nominal_roots(
    icfg: &InterproceduralCFG,
    nodes: &HashMap<Uuid, Node>,
    types: &Types<'_>,
    admitted: &HashSet<Uuid>,
) -> Vec<ResolvedConstruction> {
    use std::path::{Component, Path, PathBuf};
    fn normalized(path: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for p in path.components() {
            match p {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => {}
                p => out.push(p.as_os_str()),
            }
        }
        out
    }
    let mut result = Vec::new();
    for (&function, cfg) in &icfg.function_cfgs {
        if !matches!(
            cfg.operands.language,
            OperandLanguage::JavaScript | OperandLanguage::Python
        ) {
            continue;
        }
        let Some(caller) = nodes.get(&function) else {
            continue;
        };
        for call in &cfg.operands.calls {
            if call.receiver.is_some()
                || call.arguments.len != 0
                || (cfg.operands.language == OperandLanguage::JavaScript
                    && call.kind != InvocationKind::Construction)
                || !matches!(
                    cfg.operands
                        .expressions
                        .get(call.callee.0 as usize)
                        .map(|e| e.kind),
                    Some(ExpressionKind::Name(_))
                )
            {
                continue;
            }
            let Some(raw) = label(cfg, call.callee) else {
                continue;
            };
            let candidates = types.ids(raw);
            if candidates.len() != 1 {
                continue;
            }
            let id = *candidates.iter().next().unwrap();
            if !admitted.contains(&id) {
                continue;
            }
            let class = &nodes[&id];
            // Class rebinding in a real source procedure is not a nominal allocation.
            if icfg
                .function_cfgs
                .iter()
                .filter(|(id, _)| nodes.contains_key(id))
                .any(|(_, cfg)| {
                    cfg.operands.local_writes.iter().any(|w| {
                        cfg.operands
                            .local_slots
                            .get(w.slot.0 as usize)
                            .is_some_and(|&n| cfg.operands.name(n) == Some(raw))
                    })
                })
            {
                continue;
            }
            if nodes.values().any(|n| {
                matches!(n.node_type, NodeType::Variable | NodeType::Function)
                    && n.qualified_name == class.qualified_name
            }) {
                continue;
            }
            let imported = nodes
                .values()
                .filter(|n| n.node_type == NodeType::Import && n.file_path == caller.file_path)
                .filter(|n| {
                    n.get_property("local")
                        .or_else(|| n.get_property("binding"))
                        == Some(raw)
                })
                .any(|n| {
                    let Some(module) = n.get_property("module") else {
                        return false;
                    };
                    let exported = n.name.rsplit(':').next().unwrap_or(raw);
                    if exported != &*class.name {
                        return false;
                    }
                    if cfg.operands.language == OperandLanguage::Python {
                        class.qualified_name.as_deref() == Some(&format!("{module}.{exported}"))
                    } else if module.starts_with('.') {
                        caller
                            .file_path
                            .as_deref()
                            .and_then(|p| Path::new(p).parent())
                            .zip(class.file_path.as_deref())
                            .is_some_and(|(parent, file)| {
                                normalized(&parent.join(module)) == normalized(Path::new(file))
                            })
                    } else {
                        false
                    }
                });
            let local = caller.file_path == class.file_path && &*class.name == raw;
            if imported || local {
                result.push(ResolvedConstruction {
                    value: ValueRef {
                        function,
                        expression: call.result,
                    },
                    type_id: id,
                });
            }
        }
    }
    result
}
