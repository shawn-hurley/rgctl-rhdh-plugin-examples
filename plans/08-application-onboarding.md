# Application graph onboarding and API connections

**Status:** Implemented and verified. See the
[validation results](../analysis-server/APPLICATION-GRAPH-VALIDATION.md) and
[setup/API guide](../analysis-server/APPLICATION-GRAPH.md).

## 1. Persist an existing Java application

Start with a catalog Component and its standard `backstage.io/source-location`.
The dynamic application-graph backend delegates catalog authorization, resolves
the repository/subdirectory, and submits an onboarding job to a separate Rust
application. Rust checks out a revision, indexes production Java sources with
rgctl crates, and saves an immutable native graph snapshot plus source provenance.
End when the graph can be reopened and queried after service restart.

## 2. Discover HTTP boundaries and connect applications

Use rgctl Java annotation facts for Spring MVC/JAX-RS/declarative clients and
native invocation operands for literal RestTemplate/RestClient/WebClient calls.
Preserve unresolved URL/method cases explicitly. Join provider and consumer
Components through Backstage's processed `providesApi`/`consumesApi` relations
from `spec.providesApis`/`spec.consumesApis`; `dependsOn` alone does not identify an
API contract. API identity gates operation matching; matching paths alone must
not connect unrelated services. Keep each application's graph/version identity.
End with catalog connections and source-backed HTTP operation matches available
through authenticated JSON endpoints.

## 3. Verify existing services and dynamic packaging

Use Spring's Quoters provider and Consuming REST guide as real source inputs.
Register both Components and one API using standard catalog fields. Exercise
onboarding, graph queries, connection discovery, restart persistence, catalog
authorization, incorrect/missing relationships and unsupported URL expressions.
Export the new backend as an RHDH dynamic plugin and document setup/API usage.
End with reproducible results; retain dependencies and build caches in their
ordinary locations. Existing comparison service behavior remains covered by its
existing tests. No native rgctl modifications are planned.
