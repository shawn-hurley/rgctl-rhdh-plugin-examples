# Catalog relationship and Maven tracing validation

Two Java applications were compiled and executed independently, then indexed
through the dynamically loaded RHDH backend. Each declares a direct workflow
dependency whose file-storage dependency is transitive. The graph service
acquires their actual Maven source JARs and records source hashes and scope.

| Application | Native graph | Verified source path | Runtime result |
| --- | --- | --- | --- |
| Orders | 43 nodes, 102 edges | `OrdersApp.main → OrderService.place → OrderSink.save → FileStore.writeOrder` | `orders.txt = order-42`; no audit file |
| Audit | 41 nodes, 95 edges | `AuditApp.main → AuditService.record → AuditSink.save → FileStore.writeAudit` | `audit.txt = audit-7`; no orders file |

The Orders case uses constructor/field injection. Audit passes the interface as
a method argument. Both traces contain an actual `PossibleDispatch` binding from
`example.workflow.Sink.save` to the selected implementation. The other
implementation's resource has no application source path in that graph.

## Catalog and persistence checks

- Component-to-Component, Component-to-Resource and Resource-to-Resource
  `dependsOn` links and inverse `dependencyOf` links are present.
- Native source evidence is consistent in both directions of a dependency.
- A parent Component has four subcomponents through each child's
  `spec.subcomponentOf`; Backstage generates `partOf` and `hasPart`.
- Structural membership never fabricates code-call evidence.
- Root-specific transitive paths keep caller context: a shared library's ability
  to use both resources does not qualify both for either caller.
- Every reported code symbol resolves in its owning persisted graph.
- Reopening after service restart preserves versions, source/snapshot hashes,
  dependency provenance and both injected resource paths.
- The existing Spring API provider/consumer graphs and connection still pass.
- Chrome renders both the standard subcomponent graph and component/resource
  dependency graph in the catalog's Dependencies tab.

## Checks run

- 140 Rust tests pass across binaries, including source checkout validation,
  native interface-to-resource tracing and existing comparison/dispatch tests.
- 13 backend tests pass, including namespaces, structural relations, caller-owned
  reverse evidence, cycles, authorization, outage behavior and prior API tests.
- Two independent Maven/runtime oracles pass, including unused-resource controls.
- Live RHDH onboarding, transitive source-JAR, URI-binding, graph-node and restart
  checks pass. No source enumeration limit is reached by these cases.
- Dynamic plugin export/package, Rust formatting and shell syntax checks pass.

Native rgctl remains at `72823ed2fb4c5aa3f5fa1614b08d7cecd545ef0b`; no native
changes are part of this feature. Generated logs, graph manifests, runtime
observations and screenshots remain in the ignored development-output directory.

## Import-driven enrichment

All six Components were subsequently imported individually through Chrome's
**Import an existing Git repository** wizard, starting from descriptors with no
`dependsOn`, `providesApis` or `consumesApis` fields. The dynamic catalog module
submitted graph jobs automatically. Catalog processing populated standard fields
and Backstage rendered them in its built-in Dependencies/API tabs:

- File storage and workflow acquire their reachable Resource dependencies.
- Orders and Audit acquire their direct workflow dependency and only their own
  injected resource dependency.
- The existing Spring provider acquires `providesApis` for a generated OpenAPI
  entity; its client acquires `consumesApis` and a provider Component dependency.
- The generated API relation also produces a source-linked HTTP connection.

Nine new enrichment/processor tests and all 13 existing backend tests pass.
Type checking and dynamic module export pass. Native Rust graph extraction was
unchanged for this addition. HTTP identity uses provider base-URL bindings, with
an explicit client base URL for the existing client's relative request facts.
Before/after catalog records and browser screenshots remain local and ignored.
