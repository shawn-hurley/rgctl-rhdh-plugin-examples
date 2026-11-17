use super::model::*;
use serde_json::{Value, json};

fn route_matches(route: &str, path: &str) -> bool {
    let route: Vec<_> = route.trim_matches('/').split('/').collect();
    let path: Vec<_> = path.trim_matches('/').split('/').collect();
    route.len() == path.len()
        && route
            .iter()
            .zip(path)
            .all(|(a, b)| *a == b || (a.starts_with('{') && a.ends_with('}')))
}

/// Cross-service edges are explicit HTTP transitions, never forged native Calls.
/// Catalog API identity is mandatory; coincidentally equal routes are insufficient.
pub fn connect(applications: &[Manifest]) -> Vec<Value> {
    let mut connections = vec![];
    for consumer in applications {
        for provider in applications {
            if consumer.entity_ref == provider.entity_ref {
                continue;
            }
            for api in &consumer.consumes_apis {
                if !provider.provides_apis.contains(api) {
                    continue;
                }
                let mut matches = vec![];
                for call in consumer
                    .operations
                    .iter()
                    .filter(|o| o.direction == "consumes")
                {
                    let Some(path) = call.path.as_deref() else {
                        continue;
                    };
                    for method in &call.methods {
                        let candidates: Vec<_> = provider
                            .operations
                            .iter()
                            .filter(|o| {
                                o.direction == "provides"
                                    && o.methods.contains(method)
                                    && o.path
                                        .as_deref()
                                        .is_some_and(|route| route_matches(route, path))
                            })
                            .collect();
                        let exact = candidates.iter().any(|o| o.path.as_deref() == Some(path));
                        for endpoint in candidates
                            .into_iter()
                            .filter(|o| !exact || o.path.as_deref() == Some(path))
                        {
                            matches.push(json!({"type":"HttpApi","method":method,"path":path,
                                "caller":{"entityRef":consumer.entity_ref,"graphVersion":consumer.graph_version,"symbol":call.symbol},
                                "callee":{"entityRef":provider.entity_ref,"graphVersion":provider.graph_version,"symbol":endpoint.symbol},
                                "evidence":["catalog-consumesApi-providesApi","rgctl-java-http-operation"]}));
                        }
                    }
                }
                connections.push(json!({"apiRef":api,"consumer":consumer.entity_ref,"provider":provider.entity_ref,
                    "relationship":"consumesApi/providesApi","status":if matches.is_empty(){"catalog-linked"}else{"source-linked"},"operationMatches":matches}));
            }
        }
    }
    connections
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_templates_do_not_cross_segment_boundaries() {
        assert!(route_matches("/orders/{id}", "/orders/42"));
        assert!(!route_matches("/orders/{id}", "/orders/42/items"));
        assert!(!route_matches("/orders/{id}", "/users/42"));
    }
}
