# Application onboarding and API-connected graphs

Application onboarding runs in the shared `rgctl-service` process using rgctl crates. The
`application-graph` dynamic backend plugin obtains authorized Backstage catalog
entities and sends onboarding work to it. A developer supplies an entity
reference; repository locations and API relationships come from the catalog.

The service stores immutable native rgctl graph snapshots, source revisions,
checksums and HTTP operation evidence. Graphs survive restarts. Cross-service
connections retain both application identities and graph versions.

## Start locally

Select an rgctl checkout using `local-dev/select-rgctl.py`. Use an installed Rust
toolchain meeting that checkout's `rust-version` when building natively; no
toolchain file is installed by this project.

With the existing Podman VM running:

```sh
sh local-dev/build-plugins.sh
sh local-dev/build-service.sh
sh local-dev/start-service.sh
podman compose -f local-dev/application-graph-compose.yaml up -d --no-build
python3 local-dev/verify-application-graph.py
```

This host serves RHDH on `http://localhost:17007`. Both plugins use the same
Rust service on port 7070; host port 17071 is a compatibility alias to that
listener. Graphs and comparison reports keep separate persistent stores. See
[shared configuration and migration](SERVICE.md). The included development
tokens are for this loopback-only setup.

The `application-graph-server` binary remains a compatibility entry point using
its original `APPLICATION_GRAPH_*` environment variables. Do not run it alongside
the shared service against the same graph store.

Open the RHDH catalog and examine `spring-quoters`, `spring-quotes-client` and
`quotes-api`. The Components use the standard `spec.providesApis` and
`spec.consumesApis` fields. Their API relations are visible through Backstage's
ordinary catalog views.

## Onboard an existing Java application

Register its Component in the catalog. Set the standard source annotation and
API fields, for example:

```yaml
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: orders
  annotations:
    backstage.io/source-location: url:https://github.com/acme/orders/tree/main/service
spec:
  type: service
  lifecycle: production
  owner: payments
  providesApis: [orders-api]
  consumesApis: [pricing-api]
```

Register the corresponding `API` entities as well. API names are resolved in
the Component's namespace; explicitly qualified references support other
namespaces. `spec.dependsOn` can describe a broader dependency, but it does not
identify an HTTP API contract and is not substituted for API relationships.

After catalog registration, an onboarding workflow calls:

```sh
curl -sS http://localhost:17007/api/application-graph/onboard \
  -H 'Authorization: Bearer application-graph-local-development' \
  -H 'Content-Type: application/json' \
  -d '{"entityRef":"component:default/spring-quoters"}'
```

The response is a queued job. Poll `GET /api/application-graph/jobs/{id}` until
`succeeded` or `failed`. `GET /api/application-graph/applications` returns stored
graph manifests and opaque application keys.

Onboarding is an explicit backend action. Re-onboarding pins the newly resolved
Git commit and creates a new immutable graph version. A failed attempt leaves the
last successful graph available. Requests cannot override the catalog's source
location or relationships.

## Storage and graph access

Each successful version stores:

```text
versions/<graphVersion>/
  job.json
  repository/             checked-out Git revision
  input/                  copied production Java files
  graph.snapshot.bin      native rgctl snapshot
  manifest.json           graph/source provenance and HTTP operations
applications/<applicationKey>.json  current successful version
```

The selected repository/subdirectory supplies production Java source across its
modules. Test sources, build directories and symlinks are excluded. Onboarding
does not compile or run application code. By default the graph indexes application
sources. `application-graph.io/maven-sources: 'true'` opts a catalog Component into
Maven dependency resolution and binary/source JAR acquisition, including
transitive dependencies, using the normal Maven repository. Select one Maven
module per opted-in Component.

Maven manifests include `projectCoordinate`, direct/transitive `artifacts` with
source JAR hashes, and `dependencyTraces`. Native call and receiver facts produce
code paths across application and dependency sources. `resourceUses` records
literal file/JDBC operations and their application paths, including injected
interface dispatch. Source path limits are recorded as diagnostics when reached.

The snapshot is opened with `SnapshotNodeStore`, with checksum verification.
Node and edge queries have bounded pages. Keep one service writer per data
volume. Interrupted jobs become explicit failures on startup, while previously
published graphs remain available.

Backstage endpoints, under `/api/application-graph`:

| Method/path | Result |
| --- | --- |
| `POST /onboard` | Queue a graph for an authorized catalog Component |
| `GET /jobs/{id}` | Job state; authorized against its Component |
| `GET /applications` | Visible indexed Components and current manifests |
| `GET /applications/{key}` | Current graph manifest |
| `GET /applications/{key}/versions/{version}` | An immutable manifest belonging to this application |
| `GET /applications/{key}/nodes` | Native graph nodes |
| `GET /applications/{key}/edges` | Native typed edges, including `Calls` |
| `GET /connections` | Current catalog API relationships and source-backed operation matches |
| `GET /relationships` | Component/resource dependencies, component hierarchy, and transitive paths with code evidence |

Node/edge queries accept `offset`, `limit` (1–1000), `nodeId`, and `graphVersion`.
Connections may be filtered by canonical `entityRef`. Every read delegates
catalog authorization, including both Components and the API in a connection.
Catalog failures return errors rather than an apparently empty result.
Relationship queries accept a Component or Resource `entityRef` and `depth`
(1–8, default 4). Catalog traversal is bounded at 200 visible entities and 200
dependency paths. `partOf`/`hasPart` membership is returned as structural evidence.

## HTTP discovery and connections

Rgctl's Java plugin supplies annotation identities, arguments and locations.
Native CFG invocation operands supply the callable, receiver chain, literal
arguments and source span. The server joins those facts to native graph node IDs.
No application names, repository names or endpoint paths are hardcoded in the
discovery implementation.

Currently implemented rules:

- Spring MVC mapping annotations, including class prefixes, wildcard imports
  and route arrays.
- Jakarta/Javax JAX-RS paths and HTTP method annotations.
- Feign/MicroProfile declarative clients with mapping annotations.
- Literal RestTemplate verb-specific calls and fluent RestClient/WebClient
  `get/post/...().uri(...)` calls, including calls inside Java lambdas.

Dynamic URLs and unresolved HTTP methods retain explicit discovery records and
diagnostics. HTTP frameworks and URI construction forms outside these rules need
additional rules; finding a catalog relationship alone does not fabricate a
resolved code-level connection.

The Backstage adapter reads live processed `providesApi`/`consumesApi` relations
for each connection query. Catalog relationship edits therefore take effect
without rebuilding source graphs. The Rust service joins only applications
sharing the same canonical API reference, then compares HTTP methods and paths.
Exact routes take precedence over a matching path-variable route. The returned
`HttpApi` transition carries both graph versions and the caller/callee symbols;
it is separate from rgctl's native, intra-application `Calls` edges. A relationship
without a matching operation is returned as `catalog-linked`.
Manifests retain the catalog references captured at indexing time. Connection
queries from Backstage use current catalog references instead.

For private repositories, configure the graph service's existing Git credential
mechanism and explicitly allow the repository hostname in
`allowedRepositoryHosts`. The API accepts HTTPS URLs without inline credentials,
and validates revisions/subdirectories before checkout. GitHub and GitLab tree
URLs are understood by the backend; a repository URL indexes its default branch.
Local `file:` source locations are supported only under explicitly configured
`allowedSourceRoots`, and the enclosing Git repository must also be allowed.
Only committed Git inputs are cloned. Local sources are disabled by default.

Resources use an explicit `application-graph.io/resource-uri` annotation to match
native source operands; catalog `dependsOn` alone does not specify a connection
URI. Maven Component bindings use their indexed `projectCoordinate`, not a
Component's display name. Per-edge source evidence belongs to its named graph;
root-specific `dependencyPaths` preserve the calling application's injection
context across shared libraries.

## Real applications and verification

[Spring Quoters](https://github.com/spring-guides/quoters) provides
`GET /api/random` in `QuoteController.getRandomOne`.
[Spring Consuming REST](https://github.com/spring-guides/gs-consuming-rest), in
its `complete` module, calls that endpoint through RestClient. The
[catalog descriptor](../local-dev/application-catalog.yaml) connects the two
existing applications through a `quotes-api` entity. Source commits are recorded
in each manifest; the inputs are fetched from their actual repositories.

```sh
cargo test --manifest-path analysis-server/Cargo.toml --lib onboarding
sh local-dev/test-application-graph.sh
python3 local-dev/verify-application-graph.py
python3 local-dev/verify-application-graph.py --verify-catalog-edits
podman compose -f local-dev/application-graph-compose.yaml restart application-graph
python3 local-dev/verify-application-graph.py --verify-existing
```

The verification checks real native node/edge reads, the provider and consumer
operation, correctly scoped cross-service node references, and graph identities
after restart. Generated verification results remain under
`local-dev/.generated/`. No recording or fixture applications are required.
The catalog-edit check temporarily removes/restores the local consumer's
`consumesApis` field and verifies that graph versions remain unchanged.

The [catalog relationship validation](CATALOG-RELATIONSHIP-VALIDATION.md) records
the additional Maven, resource and hierarchy checks. The example applications,
catalog descriptors and walkthrough scripts used for those checks are local
development assets intentionally excluded from Git.

## Automatic enrichment during catalog import

Install the companion [dynamic catalog module](../plugins/catalog-backend-module-application-graph/README.md)
and opt Components in with `application-graph.io/enabled: 'true'`. The standard
import wizard then starts asynchronous onboarding. Subsequent processing merges
Maven/resource dependencies and HTTP API provider/consumer evidence into standard
catalog fields without modifying repository descriptors. See the module guide
for endpoint bindings, generated API definitions, job status and retry behavior.


## API request and response schemas

Provider operations now include an optional OpenAPI `contract`, and manifests
include reusable `apiSchemas`. Both are derived from rgctl's extracted Java
symbols/annotations and syntax trees for the same immutable source input.
Contracts contain typed parameters, request bodies and responses; reusable
schemas retain nested/recursive DTO references. The dynamic catalog module
copies these into the generated API's OpenAPI definition.

Reindex an existing Component to populate these fields. Previous manifests and
snapshots still open; absent schema data preserves the existing operation-only
behavior. See the [catalog module guide](../plugins/catalog-backend-module-application-graph/README.md#source-derived-api-schemas)
for supported types, annotations, defaults and unresolved serialization behavior.
