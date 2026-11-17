use super::{index::symbol, model::*};
use anyhow::Result;
use rgctl_graph::{SnapshotNodeStore, schema::NodeType};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
};
use uuid::Uuid;

pub fn populate(
    graph: &SnapshotNodeStore,
    root: &Path,
    with_dependencies: bool,
    artifacts: &[LibrarySource],
    uses: &mut [ResourceUse],
    diagnostics: &mut Vec<serde_json::Value>,
) -> Result<Vec<DependencyTrace>> {
    if artifacts.is_empty() && uses.is_empty() {
        return Ok(vec![]);
    }
    let dispatch = crate::dispatch::DispatchGraph::build_with_root(graph, root)?;
    let limited = Cell::new(false);
    let mut symbols = HashMap::new();
    graph.for_each_node_of_type(NodeType::Function, |n| {
        symbols.insert(n.id, symbol(n, root));
    })?;
    let application: HashSet<_> = symbols
        .iter()
        .filter(|(_, s)| !with_dependencies || s.file.starts_with("application/"))
        .map(|(id, _)| *id)
        .collect();
    let paths = |seed: Uuid| -> Result<Vec<CodePath>> {
        let (_, incoming) = dispatch.projection(graph, &[seed])?;
        let mut queue = VecDeque::from([vec![seed]]);
        let mut result = vec![];
        let mut explored = 0;
        while let Some(path) = queue.pop_front() {
            explored += 1;
            if explored > 10000 || result.len() >= 32 {
                limited.set(true);
                break;
            }
            let current = *path.last().unwrap();
            if application.contains(&current) {
                let forward: Vec<_> = path.iter().rev().copied().collect();
                let path_symbols: Vec<_> = forward
                    .iter()
                    .filter_map(|id| symbols.get(id).cloned())
                    .collect();
                if path_symbols.len() == forward.len() {
                    let edges=forward.windows(2).map(|pair| {
                        let possible=dispatch.possible.get(&(pair[0],pair[1]));
                        serde_json::json!({"from":pair[0],"to":pair[1],"type":if possible.is_some(){"PossibleDispatch"}else{"Calls"},"binding":possible})
                    }).collect();
                    result.push(CodePath {
                        symbols: path_symbols,
                        edges,
                    });
                }
            }
            if path.len() >= 24 {
                if incoming.get(&current).is_some_and(|v| !v.is_empty()) {
                    limited.set(true);
                }
                continue;
            }
            for previous in incoming.get(&current).into_iter().flatten() {
                if !path.contains(previous) {
                    let mut next = path.clone();
                    next.push(*previous);
                    queue.push_back(next);
                }
            }
        }
        result.sort_by_key(|p| std::cmp::Reverse(p.symbols.len()));
        Ok(result)
    };
    for usage in uses {
        usage.application_paths = paths(Uuid::parse_str(&usage.symbol.node_id)?)?;
    }
    let mut dependencies = vec![];
    for artifact in artifacts {
        let prefix = format!("{}/", artifact.prefix);
        let targets: Vec<_> = symbols
            .iter()
            .filter(|(id, s)| {
                s.file.starts_with(&prefix)
                    && dispatch.incoming.get(id).is_some_and(|callers| {
                        callers
                            .iter()
                            .any(|c| symbols.get(c).is_some_and(|s| !s.file.starts_with(&prefix)))
                    })
            })
            .map(|(id, _)| *id)
            .collect();
        let mut code_paths = vec![];
        for target in targets {
            code_paths.extend(paths(target)?);
            if code_paths.len() >= 64 {
                limited.set(true);
                break;
            }
        }
        code_paths.sort_by_key(|p| std::cmp::Reverse(p.symbols.len()));
        code_paths.truncate(64);
        if !code_paths.is_empty() {
            dependencies.push(DependencyTrace {
                coordinate: artifact.coordinate.clone(),
                paths: code_paths,
            });
        }
    }
    if limited.get() {
        diagnostics.push(serde_json::json!({"code":"SOURCE_TRACE_LIMIT","message":"Source path enumeration reached its bounded traversal or result limit"}));
    }
    Ok(dependencies)
}
