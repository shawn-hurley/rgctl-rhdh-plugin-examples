//! Server-side completeness joins over native operands, bindings and storage facts.
//! Origins identify possible aliases; only complete producer walks permit pruning.
use super::{Candidate, ReceiverRefinements, receivers::Types};
use rgctl_analysis::{
    ProgramDependenceGraph,
    call_bindings::{
        BindingDiagnostic, BindingSource, CallBindings, CallRef, InvocationTarget, ReceiverSupply,
        TransferKind, ValueRef,
    },
    environment::{EnvironmentDiagnostic, EnvironmentFacts},
    interprocedural_cfg::InterproceduralCFG,
    local_value_flow::{LocalFlowDiagnostic, LocalValueFlow},
    operands::*,
    receiver_closure::ResolvedConstruction,
    value_origins::{OriginAnalysis, OriginDiagnostic, OriginTransferCause},
};
use rgctl_graph::schema::{Node, NodeType};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use uuid::Uuid;

#[derive(Clone, Default)]
struct Producers {
    types: BTreeSet<Uuid>,
    objects: HashSet<ValueRef>,
}
impl Producers {
    fn merge(&mut self, other: Self) {
        self.types.extend(other.types);
        self.objects.extend(other.objects);
    }
}
struct FieldRead {
    base: ValueRef,
    member: String,
}
struct Check<'a> {
    icfg: &'a InterproceduralCFG,
    roots: HashMap<ValueRef, Uuid>,
    incoming: HashMap<ValueRef, Vec<ValueRef>>,
    fields: HashMap<ValueRef, FieldRead>,
    writes: HashMap<String, Vec<(ValueRef, ValueRef)>>,
    elements: HashMap<ValueRef, ValueRef>,
    element_writes: Vec<(ValueRef, ValueRef)>,
    environment_reads: HashSet<ValueRef>,
    hard_bad: HashSet<ValueRef>,
    aliases: HashSet<ValueRef>,
    storage_values: HashSet<ValueRef>,
    closed: HashSet<Uuid>,
    bad: HashSet<ValueRef>,
    bad_functions: HashSet<Uuid>,
    storage_types: &'a HashSet<Uuid>,
    field_blocked: bool,
    incomplete_storage: bool,
    escaped: bool,
    remaining: usize,
    reasons: HashMap<&'static str, usize>,
    missing: HashSet<ValueRef>,
}
impl Check<'_> {
    fn deny<T>(&mut self, reason: &'static str) -> Option<T> {
        *self.reasons.entry(reason).or_default() += 1;
        None
    }
    fn prove(&mut self, value: ValueRef, active: &mut HashSet<ValueRef>) -> Option<Producers> {
        if active.len() >= 256 {
            return self.deny("producer-depth");
        }
        if self.remaining == 0 {
            return self.deny("budget");
        }
        self.remaining -= 1;
        if !active.insert(value) {
            return self.deny("cycle");
        }
        let result = self.visit(value, active);
        active.remove(&value);
        result
    }
    fn visit(&mut self, value: ValueRef, active: &mut HashSet<ValueRef>) -> Option<Producers> {
        let cfg = self.icfg.function_cfgs.get(&value.function)?;
        let e = cfg.operands.expressions.get(value.expression.0 as usize)?;
        // A known nominal allocation is independent of skipped nested bodies in
        // a module record. No arbitrary computation becomes an allocation root.
        if let Some(&ty) = self.roots.get(&value) {
            if cfg.operands.diagnostics.iter().any(|d| {
                e.span
                    .is_none_or(|s| d.span.start < s.end && s.start < d.span.end)
            }) {
                return self.deny("partial-allocation");
            }
            return Some(Producers {
                types: BTreeSet::from([ty]),
                objects: HashSet::from([value]),
            });
        }
        if self.hard_bad.contains(&value) {
            return self.deny("unmodeled-producer");
        }
        if self.bad_functions.contains(&value.function) {
            return self.deny("partial-function");
        }
        if let Some(read) = self.fields.get(&value) {
            if self.escaped || self.field_blocked {
                return self.deny("unqualified-field-inventory");
            }
            let base = read.base;
            let member = read.member.clone();
            let receiver = self.prove(base, active)?;
            if receiver.types.is_empty()
                || receiver
                    .types
                    .iter()
                    .any(|t| !self.storage_types.contains(t))
            {
                return self.deny("unknown-field-owner");
            }
            let stores = self
                .writes
                .get(&member)
                .cloned()
                .filter(|v| !v.is_empty())?;
            let mut result = Producers::default();
            // Merge every modeled object and write; never infer a last write or
            // prune a different object just because one receiver was found.
            for (base, rhs) in stores {
                let object = self.prove(base, active)?;
                if object.types.is_empty()
                    || object.types.iter().any(|t| !self.storage_types.contains(t))
                {
                    return self.deny("unknown-write-owner");
                }
                // Nominal owner types are complete here. Different admitted
                // owner types are disjoint; instances of the same type merge.
                if object.types.is_disjoint(&receiver.types) {
                    continue;
                }
                result.merge(self.prove(rhs, active)?);
            }
            return Some(result);
        }
        if let Some(&base) = self.elements.get(&value) {
            if self.escaped || self.incomplete_storage {
                return self.deny("escaped-or-incomplete-container");
            }
            let container = self.prove(base, active)?;
            if container.objects.is_empty() {
                return self.deny("unknown-container");
            }
            let mut sources = Vec::new();
            for object in &container.objects {
                let facts = &self.icfg.function_cfgs.get(&object.function)?.operands;
                let e = facts.expressions.get(object.expression.0 as usize)?;
                if e.kind != ExpressionKind::Collection {
                    return self.deny("non-collection-element");
                }
                sources.extend(
                    e.inputs
                        .slice(&facts.inputs)?
                        .iter()
                        .map(|&expression| ValueRef {
                            function: object.function,
                            expression,
                        }),
                );
            }
            for (base, rhs) in self.element_writes.clone() {
                let other = self.prove(base, active)?;
                if other.objects.is_empty() {
                    return self.deny("unknown-element-write");
                }
                // Allocation sites remain merged across invocations, but a
                // different complete collection root cannot alias this one.
                if !other.objects.is_disjoint(&container.objects) {
                    sources.push(rhs);
                }
            }
            if sources.is_empty() {
                return self.deny("missing-element-producer");
            }
            let mut result = Producers::default();
            for source in sources {
                result.merge(self.prove(source, active)?);
            }
            return Some(result);
        }
        if self.bad.contains(&value) && !self.environment_reads.contains(&value) {
            return self.deny("unknown-local-producer");
        }
        match e.kind {
            ExpressionKind::Computation if self.aliases.contains(&value) => {
                let from = *self.incoming.get(&value)?.first()?;
                self.prove(from, active)
            }
            ExpressionKind::Collection => Some(Producers {
                types: BTreeSet::new(),
                objects: HashSet::from([value]),
            }),
            ExpressionKind::Literal => Some(Producers::default()),
            ExpressionKind::Formal(_) if !self.closed.contains(&value.function) => {
                self.deny("open-formal")
            }
            ExpressionKind::Name(_)
            | ExpressionKind::Formal(_)
            | ExpressionKind::Parenthesized
            | ExpressionKind::AssertedValue
            | ExpressionKind::Conditional { .. }
            | ExpressionKind::CallResult(_) => {
                let Some(predecessors) =
                    self.incoming.get(&value).cloned().filter(|v| !v.is_empty())
                else {
                    self.missing.insert(value);
                    return self.deny("missing-value-predecessor");
                };
                let mut result = Producers::default();
                for predecessor in predecessors {
                    result.merge(self.prove(predecessor, active)?);
                }
                Some(result)
            }
            _ => self.deny("unsupported-producer"),
        }
    }
}
fn label(cfg: &rgctl_analysis::cfg::ControlFlowGraph, mut id: ExpressionId) -> Option<&str> {
    for _ in 0..128 {
        match cfg.operands.expressions.get(id.0 as usize)?.kind {
            ExpressionKind::Name(n) | ExpressionKind::Member(n) => return cfg.operands.name(n),
            ExpressionKind::TypeApplication(head) => id = head,
            _ => return None,
        }
    }
    None
}

fn close_formals(
    check: &mut Check<'_>,
    nodes: &HashMap<Uuid, Node>,
    all: &[InvocationTarget],
    bad: &HashSet<InvocationTarget>,
    targets: &HashMap<CallRef, HashSet<Uuid>>,
) {
    check.closed.clear();
    for (&function, node) in nodes
        .iter()
        .filter(|(_, n)| n.node_type == NodeType::Function)
    {
        let incoming: Vec<_> = all.iter().filter(|t| t.callee == function).collect();
        if incoming.is_empty() || incoming.iter().any(|t| bad.contains(t)) {
            continue;
        }
        if check.icfg.function_cfgs.iter().all(|(&caller, cfg)| {
            cfg.operands.calls.iter().enumerate().all(|(i, c)| {
                label(cfg, c.callee) != Some(&*node.name)
                    || targets
                        .get(&CallRef {
                            caller,
                            site: CallSiteId(i as u32),
                        })
                        .is_some_and(|v| !v.is_empty())
            })
        }) {
            check.closed.insert(function);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn refine(
    nodes: &HashMap<Uuid, Node>,
    types: &Types<'_>,
    icfg: &InterproceduralCFG,
    output: &OriginAnalysis,
    environment: &EnvironmentFacts,
    roots: &[ResolvedConstruction],
    storage_types: &HashSet<Uuid>,
    aliases: &HashMap<ValueRef, ValueRef>,
    known_targets: &[InvocationTarget],
    refinements: &mut ReceiverRefinements,
    candidates: &mut Vec<Candidate>,
    limited: bool,
) -> Value {
    if roots.len() > 500_000 || known_targets.len() > 20_000 {
        return json!({"status":"unqualified","reason":"server-input-limit"});
    }
    let mut check = Check {
        icfg,
        roots: roots.iter().map(|r| (r.value, r.type_id)).collect(),
        incoming: HashMap::new(),
        fields: HashMap::new(),
        writes: HashMap::new(),
        elements: HashMap::new(),
        element_writes: vec![],
        environment_reads: HashSet::new(),
        hard_bad: HashSet::new(),
        aliases: aliases.keys().copied().collect(),
        storage_values: HashSet::new(),
        closed: HashSet::new(),
        bad: HashSet::new(),
        bad_functions: HashSet::new(),
        storage_types,
        field_blocked: false,
        incomplete_storage: false,
        escaped: false,
        remaining: 1_000_000,
        reasons: HashMap::new(),
        missing: HashSet::new(),
    };
    for (&to, &from) in aliases {
        check.incoming.entry(to).or_default().push(from);
    }
    let mut targets: HashMap<CallRef, HashSet<Uuid>> = HashMap::new();
    for target in known_targets {
        if icfg.function_cfgs.contains_key(&target.callee) {
            targets
                .entry(target.call)
                .or_default()
                .insert(target.callee);
        }
    }
    let mut binding_bad = HashSet::new();
    for diagnostic in &output.diagnostics {
        match diagnostic {
            OriginDiagnostic::LimitReached => {
                return json!({"status":"unqualified","reason":"native-query-limit"});
            }
            OriginDiagnostic::Binding { target, diagnostic } => {
                // Java/Go void fallthrough is unknown as a RESULT, but does not
                // invalidate correctly bound positional arguments or receivers.
                let void = nodes.get(&target.callee).is_some_and(|n| {
                    n.return_type_text() == Some("void")
                        || n.get_property("language") == Some("go")
                            && n.return_type_text().is_none()
                });
                let positional = icfg
                    .function_cfgs
                    .get(&target.call.caller)
                    .and_then(|c| {
                        c.operands
                            .calls
                            .get(target.call.site.0 as usize)
                            .map(|s| (c, s))
                    })
                    .is_some_and(|(c, s)| {
                        s.arguments
                            .slice(&c.operands.arguments)
                            .is_some_and(|a| a.iter().all(|v| v.kind == ArgumentKind::Positional))
                    });
                if !(*diagnostic == BindingDiagnostic::UnsupportedConvention && void && positional)
                {
                    binding_bad.insert(*target);
                }
            }
            OriginDiagnostic::BindingError(target) => {
                binding_bad.insert(*target);
            }
            OriginDiagnostic::ImplicitValue { target, .. }
            | OriginDiagnostic::UnsupportedTransfer { target, .. } => {
                if let Some(c) = icfg
                    .function_cfgs
                    .get(&target.call.caller)
                    .and_then(|f| f.operands.calls.get(target.call.site.0 as usize))
                {
                    check.hard_bad.insert(ValueRef {
                        function: target.call.caller,
                        expression: c.result,
                    });
                }
            }
            OriginDiagnostic::Local {
                function,
                diagnostic,
            } => match diagnostic {
                LocalFlowDiagnostic::UnmodeledWrite(v)
                | LocalFlowDiagnostic::AmbiguousOccurrence(v)
                | LocalFlowDiagnostic::IntraStatementWrites(v) => {
                    check.hard_bad.insert(ValueRef {
                        function: *function,
                        expression: *v,
                    });
                }
                LocalFlowDiagnostic::UnboundRead(v) => {
                    check.bad.insert(ValueRef {
                        function: *function,
                        expression: *v,
                    });
                }
                LocalFlowDiagnostic::PartialOperands => {}
                _ => {
                    check.bad_functions.insert(*function);
                }
            },
            _ => {}
        }
    }
    if limited {
        return json!({"status":"unqualified","reason":"preparation-limit"});
    }
    let mut record_count = 0usize;
    let mut local_transfers = 0usize;
    for (&function, cfg) in &icfg.function_cfgs {
        record_count = record_count.saturating_add(
            cfg.operands.expressions.len()
                + cfg.operands.field_writes.len()
                + cfg.operands.element_writes.len(),
        );
        if record_count > 500_000 {
            return json!({"status":"unqualified","reason":"server-record-limit"});
        }
        if cfg.operands.status != OperandStatus::Captured
            || cfg
                .operands
                .diagnostics
                .iter()
                .any(|d| d.kind != OperandDiagnosticKind::NestedCallable)
        {
            check.bad_functions.insert(function);
            if nodes.contains_key(&function) {
                check.field_blocked = true;
                check.incomplete_storage = true;
            }
        }
        if let Ok(pdg) = ProgramDependenceGraph::build(cfg, &[]) {
            let local = LocalValueFlow::build(cfg, &pdg, Default::default());
            local_transfers = local_transfers.saturating_add(local.transfers.len());
            if local_transfers > 250_000 {
                return json!({"status":"unqualified","reason":"server-transfer-limit"});
            }
            for edge in local.transfers {
                check
                    .incoming
                    .entry(ValueRef {
                        function,
                        expression: edge.to,
                    })
                    .or_default()
                    .push(ValueRef {
                        function,
                        expression: edge.from,
                    });
            }
            for d in local.diagnostics {
                match d {
                    LocalFlowDiagnostic::UnboundRead(expression) => {
                        check.bad.insert(ValueRef {
                            function,
                            expression,
                        });
                    }
                    LocalFlowDiagnostic::UnmodeledWrite(expression)
                    | LocalFlowDiagnostic::AmbiguousOccurrence(expression)
                    | LocalFlowDiagnostic::IntraStatementWrites(expression) => {
                        check.hard_bad.insert(ValueRef {
                            function,
                            expression,
                        });
                    }
                    LocalFlowDiagnostic::PartialOperands => {}
                    _ => {
                        check.bad_functions.insert(function);
                    }
                }
            }
        } else {
            check.bad_functions.insert(function);
        }
        let selectors: HashSet<_> = cfg.operands.calls.iter().map(|c| c.callee).collect();
        for (i, e) in cfg.operands.expressions.iter().enumerate() {
            let value = ValueRef {
                function,
                expression: ExpressionId(i as u32),
            };
            if selectors.contains(&value.expression) {
                continue;
            }
            if let ExpressionKind::Member(n) = e.kind
                && let Some(&base) = e.inputs.slice(&cfg.operands.inputs).and_then(|v| v.first())
                && let Some(member) = cfg.operands.name(n)
            {
                check.fields.insert(
                    value,
                    FieldRead {
                        base: ValueRef {
                            function,
                            expression: base,
                        },
                        member: member.to_owned(),
                    },
                );
            }
            if matches!(
                e.kind,
                ExpressionKind::Element | ExpressionKind::Projection { position: 0 }
            ) && let Some(&base) = e.inputs.slice(&cfg.operands.inputs).and_then(|v| v.first())
            {
                check.elements.insert(
                    value,
                    ValueRef {
                        function,
                        expression: base,
                    },
                );
            }
        }
        for w in &cfg.operands.field_writes {
            if let Some(member) = cfg.operands.name(w.member) {
                // Go lowers a field-store receiver as a Name but currently
                // omits its local-use anchor. Join it to the explicit receiver
                // formal only in a declaration-only scope with no receiver
                // reassignment or shadow slot. Otherwise keep it unknown.
                if cfg.operands.language == OperandLanguage::Go
                    && let ExpressionKind::Name(n) =
                        cfg.operands.expressions[w.receiver.0 as usize].kind
                    && let Some(receiver) = cfg
                        .operands
                        .formals
                        .iter()
                        .find(|f| f.kind == FormalKind::Receiver && f.name == n)
                    && cfg
                        .operands
                        .local_slots
                        .iter()
                        .all(|n| cfg.operands.formals.iter().any(|f| f.name == *n))
                    && cfg
                        .operands
                        .local_writes
                        .iter()
                        .filter(|w| cfg.operands.local_slots.get(w.slot.0 as usize) == Some(&n))
                        .all(|w| w.value == Some(receiver.value))
                {
                    check
                        .incoming
                        .entry(ValueRef {
                            function,
                            expression: w.receiver,
                        })
                        .or_default()
                        .push(ValueRef {
                            function,
                            expression: receiver.value,
                        });
                }
                check.writes.entry(member.to_owned()).or_default().push((
                    ValueRef {
                        function,
                        expression: w.receiver,
                    },
                    ValueRef {
                        function,
                        expression: w.value,
                    },
                ));
            }
        }
        for w in &cfg.operands.element_writes {
            check.element_writes.push((
                ValueRef {
                    function,
                    expression: w.container,
                },
                ValueRef {
                    function,
                    expression: w.value,
                },
            ));
            check.field_blocked = true;
        }
    }
    for read in &environment.member_reads {
        check.fields.insert(
            read.value,
            FieldRead {
                base: read.receiver,
                member: read.member.clone(),
            },
        );
    }
    for transfer in &output.transfers {
        match transfer.cause {
            OriginTransferCause::Local(_) => {
                check
                    .incoming
                    .entry(transfer.to)
                    .or_default()
                    .push(transfer.from);
            }
            OriginTransferCause::Invocation { target, kind }
                if !binding_bad.contains(&target)
                    && matches!(
                        kind,
                        TransferKind::ActualToFormal
                            | TransferKind::DefaultToFormal { .. }
                            | TransferKind::ReceiverToFormal
                            | TransferKind::ReturnToResult { position: 0 }
                    ) =>
            {
                check
                    .incoming
                    .entry(transfer.to)
                    .or_default()
                    .push(transfer.from);
            }
            OriginTransferCause::Environment { .. } => {
                check
                    .incoming
                    .entry(transfer.to)
                    .or_default()
                    .push(transfer.from);
                check.environment_reads.insert(transfer.to);
                check.storage_values.insert(transfer.to);
            }
            _ => {}
        }
    }
    // The origin graph has only expression-to-expression edges. Preserve the
    // binder's implicit/missing alternatives too, rather than treating a known
    // W edge from another invocation as the entire formal's producer set.
    let mut implicit_bindings = CallBindings::new(icfg);
    for &target in known_targets {
        if let Ok(binding) = implicit_bindings.bind(target) {
            for transfer in binding
                .transfers
                .slice(implicit_bindings.transfers())
                .unwrap_or(&[])
            {
                if matches!(transfer.from, BindingSource::Implicit(_)) {
                    check.hard_bad.insert(transfer.to);
                }
            }
        }
    }
    for d in &environment.diagnostics {
        match d {
            EnvironmentDiagnostic::IncompleteStorage(binding) => {
                for transfer in &environment.transfers {
                    if transfer.binding == *binding {
                        check.hard_bad.insert(transfer.to);
                    }
                }
            }
            EnvironmentDiagnostic::MissingCallableDefinition(_) => {
                check.field_blocked = true;
                check.incomplete_storage = true;
            }
            EnvironmentDiagnostic::LimitReached => {
                return json!({"status":"unqualified","reason":"environment-limit"});
            }
            _ => {}
        }
    }
    close_formals(&mut check, nodes, known_targets, &binding_bad, &targets);
    for predecessors in check.incoming.values_mut() {
        predecessors.sort_by_key(|v| (v.function, v.expression.0));
        predecessors.dedup();
    }
    let mut all_targets = known_targets.to_vec();
    let mut bindings = CallBindings::new(icfg);
    let mut discovered = HashSet::new();
    // Exact receiver proofs can expose a setter/run target that the possible-
    // origin solver did not connect (for example a Go pointer alias). Bind it
    // using rgctl's API, then repeat until no new canonical call is discovered.
    for _ in 0..8 {
        let mut added = false;
        for (&caller, cfg) in &icfg.function_cfgs {
            for (site, c) in cfg.operands.calls.iter().enumerate() {
                let Some(receiver) = c.receiver else {
                    continue;
                };
                let Some(method) = label(cfg, c.callee) else {
                    continue;
                };
                let call = CallRef {
                    caller,
                    site: CallSiteId(site as u32),
                };
                let Some(proof) = check
                    .prove(
                        ValueRef {
                            function: caller,
                            expression: receiver,
                        },
                        &mut HashSet::new(),
                    )
                    .filter(|p| !p.types.is_empty())
                else {
                    continue;
                };
                let allowed: HashSet<_> = proof
                    .types
                    .iter()
                    .flat_map(|ty| types.closed_targets(*ty, method))
                    .filter(|id| icfg.function_cfgs.contains_key(id))
                    .collect();
                let identities: HashSet<_> = known_targets
                    .iter()
                    .filter(|t| t.call == call)
                    .filter_map(|t| nodes.get(&t.callee))
                    .filter_map(|n| n.identity_discriminator())
                    .collect();
                let distinct: HashSet<_> = allowed
                    .iter()
                    .filter_map(|id| nodes[id].identity_discriminator())
                    .collect();
                if identities.is_empty() && distinct.len() > 1 {
                    continue;
                }
                for callee in allowed {
                    if all_targets.len() >= 20_000 {
                        return json!({"status":"unqualified","reason":"server-target-limit"});
                    }
                    if !identities.is_empty()
                        && nodes[&callee]
                            .identity_discriminator()
                            .is_some_and(|v| !identities.contains(v))
                    {
                        continue;
                    }
                    let target = InvocationTarget {
                        call,
                        callee,
                        receiver: ReceiverSupply::Member,
                    };
                    if !discovered.insert(target) {
                        continue;
                    }
                    let Ok(binding) = bindings.bind(target) else {
                        continue;
                    };
                    if bindings.transfers().len() > 250_000 {
                        return json!({"status":"unqualified","reason":"server-binding-limit"});
                    }
                    let diagnostics = binding
                        .diagnostics
                        .slice(bindings.diagnostics())
                        .unwrap_or(&[]);
                    let void = nodes[&callee].return_type_text() == Some("void")
                        || nodes[&callee].get_property("language") == Some("go")
                            && nodes[&callee].return_type_text().is_none();
                    if diagnostics
                        .iter()
                        .any(|d| !(*d == BindingDiagnostic::UnsupportedConvention && void))
                    {
                        continue;
                    }
                    for transfer in binding.transfers.slice(bindings.transfers()).unwrap_or(&[]) {
                        if let BindingSource::Expression(from) = transfer.from
                            && matches!(
                                transfer.kind,
                                TransferKind::ActualToFormal
                                    | TransferKind::ReceiverToFormal
                                    | TransferKind::DefaultToFormal { .. }
                                    | TransferKind::ReturnToResult { position: 0 }
                            )
                        {
                            check.incoming.entry(transfer.to).or_default().push(from);
                        } else {
                            check.hard_bad.insert(transfer.to);
                        }
                    }
                    targets.entry(call).or_default().insert(callee);
                    all_targets.push(target);
                    added = true;
                    candidates.push(Candidate{from:caller,to:callee,reason:"server-storage-bound-call",declared_target:Some(callee),evidence:json!({"invocation":target,"scope":"complete receiver plus native call binding"})});
                }
            }
        }
        close_formals(&mut check, nodes, &all_targets, &binding_bad, &targets);
        for predecessors in check.incoming.values_mut() {
            predecessors.sort_by_key(|v| (v.function, v.expression.0));
            predecessors.dedup();
        }
        if !added {
            break;
        }
    }
    let mut callables_by_origin: HashMap<_, Vec<_>> = HashMap::new();
    for callable in &output.callable_values {
        callables_by_origin
            .entry(callable.origin)
            .or_default()
            .push(callable);
    }
    let mut captures_by_function: HashMap<_, Vec<_>> = HashMap::new();
    for capture in environment
        .transfers
        .iter()
        .filter(|t| t.to.function != t.from.function)
    {
        captures_by_function
            .entry(capture.to.function)
            .or_default()
            .push(capture.from);
    }
    let constructor_names: HashSet<&str> =
        storage_types.iter().map(|id| &*nodes[id].name).collect();
    // Unknown calls receiving a tracked object or a constructor binding can
    // mutate storage. Keep the warning instead of assuming missing effects empty.
    for (&caller, cfg) in &icfg.function_cfgs {
        for (site, c) in cfg.operands.calls.iter().enumerate() {
            let call = CallRef {
                caller,
                site: CallSiteId(site as u32),
            };
            if targets.get(&call).is_some_and(|s| !s.is_empty())
                || (c.kind == InvocationKind::Construction
                    && check.roots.contains_key(&ValueRef {
                        function: caller,
                        expression: c.result,
                    }))
            {
                continue;
            }
            let mut values: Vec<_> = c
                .arguments
                .slice(&cfg.operands.arguments)
                .unwrap_or(&[])
                .iter()
                .map(|a| a.value)
                .collect();
            values.extend(c.receiver);
            for expression in values {
                let value = ValueRef {
                    function: caller,
                    expression,
                };
                // Passing a bound setter is also an object escape, even if the
                // old possible-origin solver had no origin for a pointer alias.
                let mut producers = vec![value];
                let mut seen_producers = HashSet::new();
                while let Some(v) = producers.pop() {
                    if seen_producers.len() >= 1024 {
                        check.escaped = true;
                        break;
                    }
                    if !seen_producers.insert(v) {
                        continue;
                    }
                    if let Some(facts) = icfg.function_cfgs.get(&v.function).map(|c| &c.operands)
                        && let Some(e) = facts.expressions.get(v.expression.0 as usize)
                        && let ExpressionKind::Member(n) = e.kind
                        && let Some(method) = facts.name(n)
                        && let Some(&receiver) =
                            e.inputs.slice(&facts.inputs).and_then(|v| v.first())
                        && let Some(object) = check.prove(
                            ValueRef {
                                function: v.function,
                                expression: receiver,
                            },
                            &mut HashSet::new(),
                        )
                        && object
                            .types
                            .iter()
                            .any(|ty| !types.closed_targets(*ty, method).is_empty())
                    {
                        check.escaped = true;
                    }
                    if let Some(incoming) = check.incoming.get(&v) {
                        producers.extend(incoming);
                    }
                }
                for origin in output.origins(value) {
                    for &callable in callables_by_origin.get(origin).into_iter().flatten() {
                        if let ReceiverSupply::BoundValue(receiver) = callable.receiver {
                            if check
                                .prove(receiver, &mut HashSet::new())
                                .is_some_and(|p| !p.types.is_empty() || !p.objects.is_empty())
                            {
                                check.escaped = true;
                            }
                        }
                        if icfg.function_cfgs.get(&callable.callee).is_some_and(|c| {
                            !c.operands.field_writes.is_empty()
                                || !c.operands.element_writes.is_empty()
                        }) {
                            check.escaped = true;
                        }
                        for &capture in captures_by_function
                            .get(&callable.callee)
                            .into_iter()
                            .flatten()
                        {
                            if check
                                .prove(capture, &mut HashSet::new())
                                .is_some_and(|p| !p.types.is_empty() || !p.objects.is_empty())
                            {
                                check.escaped = true;
                            }
                        }
                    }
                }
                let mut pending = vec![expression];
                let mut visited = HashSet::new();
                while let Some(id) = pending.pop() {
                    if visited.len() >= 1024 {
                        check.escaped = true;
                        break;
                    }
                    if !visited.insert(id) {
                        continue;
                    }
                    if let Some(e) = cfg.operands.expressions.get(id.0 as usize) {
                        if let ExpressionKind::Name(n) = e.kind
                            && cfg
                                .operands
                                .name(n)
                                .is_some_and(|n| constructor_names.contains(n))
                        {
                            check.escaped = true;
                        }
                        pending.extend(e.inputs.slice(&cfg.operands.inputs).unwrap_or(&[]));
                    }
                }
                let proven = check
                    .prove(value, &mut HashSet::new())
                    .is_some_and(|p| !p.types.is_empty() || !p.objects.is_empty());
                if proven
                    || output.origins(value).iter().any(|id| {
                        output.seeds.get(id.0 as usize).is_some_and(|s| {
                            s.type_id.is_some()
                                || s.type_id.is_none() && check.roots.contains_key(&s.value)
                        })
                    })
                {
                    check.escaped = true;
                }
            }
        }
    }
    if check.escaped {
        return json!({"status":"unqualified","reason":"object-or-constructor-escapes-to-unresolved-call","escapedObject":true,"fieldInventoryBlocked":check.field_blocked});
    }
    let mut evidence = Vec::new();
    let mut additions = Vec::new();
    let mut functions: Vec<_> = icfg.function_cfgs.keys().copied().collect();
    functions.sort();
    for caller in functions {
        if !nodes.contains_key(&caller) {
            continue;
        }
        let cfg = &icfg.function_cfgs[&caller];
        let mut methods: HashMap<String, Option<HashSet<Uuid>>> = HashMap::new();
        let mut qualified = true;
        for (site, c) in cfg.operands.calls.iter().enumerate() {
            let call = CallRef {
                caller,
                site: CallSiteId(site as u32),
            };
            if c.kind == InvocationKind::Construction {
                if !check.roots.contains_key(&ValueRef {
                    function: caller,
                    expression: c.result,
                }) {
                    qualified = false;
                }
                continue;
            }
            let Some(method) = label(cfg, c.callee) else {
                qualified = false;
                continue;
            };
            let allowed = methods
                .entry(method.to_owned())
                .or_insert_with(|| Some(HashSet::new()));
            if let Some(receiver) = c.receiver {
                let value = ValueRef {
                    function: caller,
                    expression: receiver,
                };
                if let Some(proof) = check
                    .prove(value, &mut HashSet::new())
                    .filter(|p| !p.types.is_empty())
                {
                    if let Some(a) = allowed {
                        for ty in &proof.types {
                            a.extend(types.closed_targets(*ty, method));
                        }
                    }
                    evidence.push(json!({"call":call,"method":method,"types":proof.types,"scope":"server native storage producers; all modeled writes merge"}));
                    continue;
                }
            }
            *allowed = None;
            // The projection is caller/target based. A differently named
            // callable invocation can also reach Execute, so its target name
            // must prevent a nearby W.Execute from narrowing that shared pair.
            for target in targets.get(&call).into_iter().flatten() {
                if let Some(node) = nodes.get(target) {
                    methods.insert(node.name.to_string(), None);
                }
            }
            if !targets.get(&call).is_some_and(|s| !s.is_empty()) {
                qualified = false;
            }
        }
        if qualified {
            for (method, allowed) in methods {
                if let Some(allowed) = allowed.filter(|s| !s.is_empty()) {
                    additions.push(((caller, method), allowed));
                }
            }
        }
    }
    if check.remaining == 0 {
        return json!({"status":"unqualified","reason":"server-query-limit"});
    }
    for (key, allowed) in &additions {
        // Keep the actual permitted call path even where the old fallback knew
        // only the interface. Raw persisted graph facts are never removed.
        for &to in allowed {
            candidates.push(Candidate {
                from: key.0,
                to,
                reason: "server-storage-producer-target",
                declared_target: Some(to),
                evidence: json!({"scope":"qualified native producer inventory","method":key.1}),
            });
        }
        refinements.insert(key.clone(), allowed.clone());
    }
    json!({"status":"partial","qualifiedMethods":additions.len(),"qualifiedReceivers":evidence,"rejectionCounts":check.reasons,"escapedObject":check.escaped,"fieldInventoryBlocked":check.field_blocked,"missingProducers":check.missing.iter().map(|v|json!({"value":v,"function":nodes.get(&v.function).map(|n|n.qualified_name.as_deref().unwrap_or(&n.name)),"kind":icfg.function_cfgs.get(&v.function).and_then(|c|c.operands.expressions.get(v.expression.0 as usize)).map(|e|e.kind)})).collect::<Vec<_>>(),"closedFunctions":check.closed.iter().filter_map(|id|nodes.get(id)).map(|n|n.qualified_name.as_deref().unwrap_or(&n.name)).collect::<BTreeSet<_>>(),"discoveredBindings":all_targets.len().saturating_sub(known_targets.len()),"fieldNames":check.writes.keys().collect::<BTreeSet<_>>()})
}
