//! Stopgap receiver candidates derived from native graph facts, not value-flow proofs.
use super::{Candidate, arguments, logical, name};
use rgctl_graph::schema::{Node, NodeType};
use rgctl_plugin_api::{Relation, RelationType};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::Path,
};
use uuid::Uuid;

pub(super) struct Types<'a> {
    nodes: &'a HashMap<Uuid, Node>,
    aliases: HashMap<String, HashSet<Uuid>>,
    ancestors: HashMap<Uuid, HashSet<Uuid>>,
    methods: HashMap<String, Vec<(Uuid, HashSet<Uuid>)>>,
}
impl<'a> Types<'a> {
    pub(super) fn new(
        nodes: &'a HashMap<Uuid, Node>,
        parents: &HashMap<String, Vec<String>>,
    ) -> Self {
        let mut result = Self {
            nodes,
            aliases: HashMap::new(),
            ancestors: HashMap::new(),
            methods: HashMap::new(),
        };
        for n in nodes.values().filter(|n| {
            matches!(
                n.node_type,
                NodeType::Class | NodeType::Struct | NodeType::Interface | NodeType::Enum
            )
        }) {
            result
                .aliases
                .entry(name(n).into())
                .or_default()
                .insert(n.id);
            result
                .aliases
                .entry(n.name.to_string())
                .or_default()
                .insert(n.id);
        }
        for n in nodes.values().filter(|n| {
            matches!(
                n.node_type,
                NodeType::Class | NodeType::Struct | NodeType::Interface | NodeType::Enum
            )
        }) {
            let mut seen = HashSet::new();
            let mut queue = vec![n.id];
            while let Some(key) = queue.pop() {
                if !seen.insert(key) {
                    continue;
                }
                if let Some(node) = nodes.get(&key) {
                    for parent in parents.get(name(node)).into_iter().flatten() {
                        queue.extend(result.ids(parent));
                    }
                }
            }
            result.ancestors.insert(n.id, seen);
        }
        for n in nodes.values().filter(|n| n.node_type == NodeType::Function) {
            if let Some((owner, _)) = name(n).rsplit_once('.') {
                let owners = result.ids(owner);
                if !owners.is_empty() {
                    result
                        .methods
                        .entry(n.name.to_string())
                        .or_default()
                        .push((n.id, owners));
                }
            }
        }
        result
    }
    pub(super) fn ids(&self, raw: &str) -> HashSet<Uuid> {
        let value = raw.trim_start_matches(['*', '&']).trim();
        if value.contains(['|', '&']) {
            return HashSet::new();
        }
        // Native generic-owner labels retain their type arguments. Resolve the
        // nominal owner; arguments remain separate value/type evidence.
        let value = value.split(['<', '[']).next().unwrap_or(value);
        let simple = value.rsplit('.').next().unwrap_or(value);
        self.aliases
            .get(value)
            .into_iter()
            .chain(self.aliases.get(simple))
            .flat_map(|s| s.iter().copied())
            .collect()
    }
    /// Include inherited definitions and compatible implementations. The
    /// provider supplies conservative canonical candidates, not exclusions.
    pub(super) fn origin_targets(&self, receiver: Uuid, method: &str) -> Vec<Uuid> {
        let mut result: Vec<_> = self
            .methods
            .get(method)
            .into_iter()
            .flatten()
            .filter(|(_, owners)| {
                owners.iter().any(|owner| {
                    self.ancestors
                        .get(&receiver)
                        .is_some_and(|s| s.contains(owner))
                        || self
                            .ancestors
                            .get(owner)
                            .is_some_and(|s| s.contains(&receiver))
                })
            })
            .map(|(id, _)| *id)
            .collect();
        result.sort();
        result
    }
    /// Exact qualified Java class, bypassing fuzzy/simple-name candidate lookup.
    pub(super) fn exact_java_class(&self, qualified: &str) -> Option<Uuid> {
        let mut found = self.nodes.values().filter(|n| {
            n.node_type == NodeType::Class
                && name(n) == qualified
                && n.get_property("language") == Some("java")
                && !n
                    .get_property("modifiers")
                    .unwrap_or("")
                    .split_whitespace()
                    .any(|m| m == "abstract")
        });
        let id = found.next()?.id;
        found.next().is_none().then_some(id)
    }
    /// Keep every definition on an exact concrete receiver's ancestor chain.
    /// Interface definitions remain visible, but their sibling implementations
    /// are not possible receivers of this local allocation.
    pub(super) fn closed_targets(&self, receiver: Uuid, method: &str) -> HashSet<Uuid> {
        self.methods
            .get(method)
            .into_iter()
            .flatten()
            .filter(|(_, owners)| {
                owners.iter().any(|owner| {
                    self.ancestors
                        .get(&receiver)
                        .is_some_and(|a| a.contains(owner))
                })
            })
            .map(|(id, _)| *id)
            .collect()
    }
    /// Canonical constructor definitions owned by a possible constructed type.
    pub(super) fn constructor_targets(&self, receiver: Uuid) -> Vec<Uuid> {
        let mut result: Vec<_> = self
            .methods
            .values()
            .flatten()
            .filter(|(id, owners)| {
                name(&self.nodes[id]).ends_with(".<init>") && owners.contains(&receiver)
            })
            .map(|(id, _)| *id)
            .collect();
        result.sort();
        result.dedup();
        result
    }
    /// An unknown computed selector may choose any compatible canonical method.
    pub(super) fn origin_all_targets(&self, receiver: Uuid) -> Vec<Uuid> {
        let mut result: Vec<_> = self
            .methods
            .keys()
            .flat_map(|method| self.origin_targets(receiver, method))
            .collect();
        result.sort();
        result.dedup();
        result
    }
    fn targets(&self, available: &HashSet<Uuid>, method: &str) -> Vec<Uuid> {
        self.methods
            .get(method)
            .into_iter()
            .flatten()
            .filter(|(_, owners)| {
                owners.iter().any(|o| {
                    self.ancestors
                        .get(o)
                        .is_some_and(|a| !a.is_disjoint(available))
                })
            })
            .map(|(id, _)| *id)
            .collect()
    }
    fn arguments_fit(&self, r: &Relation, target: &Node) -> bool {
        let Some(args) = arguments(r) else {
            return true;
        };
        if args.len() != target.parameters.len() {
            return false;
        }
        let primitive = |s: &str| {
            matches!(
                s,
                "byte" | "short" | "int" | "long" | "float" | "double" | "boolean" | "char"
            )
        };
        args.iter().zip(&target.parameters).all(|(actual, formal)| {
            let Some(formal) = formal.param_type.as_deref() else {
                return true;
            };
            if *actual == formal {
                return true;
            }
            // Preserve unknown boxing/reference and numeric conversions. Only
            // exclude clearly incompatible scalar signatures here.
            if r.metadata.get("language").and_then(Value::as_str) == Some("java") {
                if (*actual == "String" && primitive(formal))
                    || (formal == "String" && primitive(actual))
                {
                    return false;
                }
                if primitive(actual) && primitive(formal) {
                    return (*actual == "boolean") == (formal == "boolean");
                }
            }
            let a = self.ids(actual);
            let f = self.ids(formal);
            a.is_empty()
                || f.is_empty()
                || a.iter()
                    .any(|i| self.ancestors.get(i).is_some_and(|s| !s.is_disjoint(&f)))
        })
    }
    fn display(&self, available: &HashSet<Uuid>) -> Vec<&str> {
        let names: BTreeSet<_> = available
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .map(name)
            .collect();
        names.into_iter().collect()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn candidates(
    nodes: &HashMap<Uuid, Node>,
    root: &Path,
    parents: &HashMap<String, Vec<String>>,
    native_calls: &HashSet<(Uuid, Uuid)>,
    previous: &[Candidate],
    relations: &[Relation],
    locals: &HashMap<Uuid, HashMap<String, String>>,
) -> Vec<Candidate> {
    let types = Types::new(nodes, parents);
    let functions: Vec<_> = nodes
        .values()
        .filter(|n| n.node_type == NodeType::Function)
        .collect();
    let mut names: HashMap<&str, Vec<&Node>> = HashMap::new();
    for function in &functions {
        names.entry(name(function)).or_default().push(function);
    }
    let mut bound = vec![];
    for r in relations.iter().filter(|r| {
        matches!(
            r.relation_type,
            RelationType::Calls | RelationType::Instantiates
        )
    }) {
        let identity = r
            .metadata
            .get("from_identity_discriminator")
            .and_then(Value::as_str)
            .unwrap_or("");
        let matches: Vec<_> = names
            .get(r.from.as_str())
            .into_iter()
            .flatten()
            .copied()
            .filter(|n| {
                n.identity_discriminator().unwrap_or("") == identity
                    && n.file_path
                        .as_deref()
                        .is_some_and(|p| logical(root, p) == logical(root, &r.location.file))
                    && n.start_line.is_some_and(|l| l <= r.location.start_line)
                    && n.end_line.is_some_and(|l| l >= r.location.end_line)
            })
            .collect();
        if matches.len() == 1 {
            bound.push((matches[0], r));
        }
    }
    let mut current: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    for &(a, b) in native_calls {
        current.entry(a).or_default().insert(b);
    }
    for c in previous {
        current.entry(c.from).or_default().insert(c.to);
    }
    let mut result = vec![];
    for &(source, r) in &bound {
        if r.relation_type != RelationType::Calls {
            continue;
        }
        let hint = r.to_qualified_hint.as_deref().unwrap_or(&r.to);
        let mut targets: HashSet<_> = names
            .get(hint)
            .into_iter()
            .flatten()
            .map(|n| n.id)
            .collect();
        if !hint.contains('.') {
            targets.extend(
                names
                    .get(format!("{}.{hint}", name(source)).as_str())
                    .into_iter()
                    .flatten()
                    .map(|n| n.id),
            );
            if let Some((scope, _)) = name(source).rsplit_once('.') {
                targets.extend(
                    names
                        .get(format!("{scope}.{hint}").as_str())
                        .into_iter()
                        .flatten()
                        .map(|n| n.id),
                );
            }
        }
        for target in targets {
            let Some(node) = nodes.get(&target) else {
                continue;
            };
            if r.metadata
                .get("to_identity_discriminator")
                .and_then(Value::as_str)
                .is_some_and(|id| node.identity_discriminator().unwrap_or("") != id)
            {
                continue;
            }
            if !types.arguments_fit(r, node) {
                continue;
            }
            current.entry(source.id).or_default().insert(target);
            result.push(Candidate{from:source.id,to:target,declared_target:None,reason:"stopgap-known-symbol-call",evidence:json!({"location":r.location,"targetHint":hint,"assumption":"known lexical or qualified callable retained when argument evidence is incomplete"})});
        }
    }
    let mut visible: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    for n in &functions {
        let set = visible.entry(n.id).or_default();
        for p in &n.parameters {
            if let Some(ty) = p.param_type.as_deref() {
                set.extend(types.ids(ty));
            }
        }
        for ty in locals.get(&n.id).into_iter().flat_map(|l| l.values()) {
            set.extend(types.ids(ty));
        }
        for variable in nodes
            .values()
            .filter(|v| v.node_type == NodeType::Variable && v.file_path == n.file_path)
        {
            if let (Some(a), Some(b), Some(c), Some(d)) = (
                n.start_byte,
                n.end_byte,
                variable.start_byte,
                variable.end_byte,
            ) {
                if a <= c && c < d && d <= b {
                    if let Some(ty) = variable.return_type_text() {
                        set.extend(types.ids(ty));
                    }
                }
            }
        }
        for target in current.get(&n.id).into_iter().flatten() {
            if let Some(ty) = nodes.get(target).and_then(Node::return_type_text) {
                set.extend(types.ids(ty));
            }
        }
    }
    for &(source, r) in &bound {
        let allocated = types.ids(r.to_qualified_hint.as_deref().unwrap_or(&r.to));
        if r.relation_type == RelationType::Instantiates
            || (r.relation_type == RelationType::Calls && !allocated.is_empty())
        {
            visible.entry(source.id).or_default().extend(allocated);
        }
    }
    let mut ordered = functions.clone();
    ordered.sort_by_key(|n| name(n).len());
    for child in ordered {
        for parent in &functions {
            if child.id != parent.id
                && child.file_path == parent.file_path
                && name(child).starts_with(&format!("{}.", name(parent)))
            {
                let inherited = visible.get(&parent.id).cloned().unwrap_or_default();
                visible.entry(child.id).or_default().extend(inherited);
            }
        }
    }
    for &(source, r) in &bound {
        if r.relation_type != RelationType::Calls {
            continue;
        }
        let method = r.to.rsplit('.').next().unwrap_or(&r.to);
        let mut hinted = r
            .to_type_hint
            .as_deref()
            .map(|s| types.ids(s))
            .unwrap_or_default();
        if let Some((owner, _)) = r
            .to_qualified_hint
            .as_deref()
            .and_then(|s| s.rsplit_once('.'))
        {
            hinted.extend(types.ids(owner));
        }
        let declared = r
            .to_qualified_hint
            .as_deref()
            .and_then(|s| names.get(s))
            .filter(|n| n.len() == 1)
            .map(|n| n[0].id);
        for (reason, available) in [
            ("stopgap-contract-target", hinted),
            (
                "stopgap-visible-type-target",
                visible.get(&source.id).cloned().unwrap_or_default(),
            ),
        ] {
            for target in types.targets(&available, method) {
                if !types.arguments_fit(r, &nodes[&target]) {
                    continue;
                }
                result.push(Candidate{from:source.id,to:target,declared_target:declared,reason,evidence:json!({"location":r.location,"candidateReceiverTypes":types.display(&available),"receiverBinding":"not-established","assumption":"a type visible in the function or lexical parent may supply the receiver; no value-flow proof","factSources":["native allocations","formal parameters","typed Variable nodes","available CFG local types","known callee return types","lexical parent types"]})});
            }
        }
    }
    result
}
