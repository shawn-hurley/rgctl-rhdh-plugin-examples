# Phase 3 — Rust analysis server and JSON HTTP API

**Status:** Complete for the agreed four-demo scope, with explicitly partial
static coverage and the acknowledged upstream Java overload limitation.

**Start:** Reproducible infrastructure and dynamically loaded skeleton plugins work.

**End:** The Rust service acquires Maven dependency sources, analyzes all four
demos against independently verified expectations, runs in local Compose, and
supports submission/status/report access through the Backstage backend.

## Agreed phase scope

- Maven dependency resolution and source acquisition are required in this phase,
  including the transitive override with an unchanged direct dependency.
- Use matching Maven source JARs. Missing sources produce diagnostics; repository
  checkout fallback is deferred.
- The Rust service returns full enclosing source blocks inline in its report,
  with source identity, relative file path, inclusive line range, and text.
  The UI consumes this context directly.
- Add the thin Backstage backend routes for submission, status, and reports in
  this phase. User-facing comparison controls and report rendering remain Phase 4.
- Qualification of an additional existing application is a follow-up milestone;
  it is not a Phase 3 completion requirement.

## Milestones

| Milestone | Start | End |
| --- | --- | --- |
| JSON contract | Accepted demo evidence | Agreed comparison requests, job states, reports, and source-block examples |
| Crate-backed analysis | Contract agreed | Prepared sources for all four demos produce the expected results through rgctl crate APIs |
| Maven preparation | Demo analysis works | Effective dependency versions and matching sources are acquired for baseline/candidate comparisons, including transitive overrides |
| Local integration | End-to-end service works | Rust service runs in Compose and the Backstage backend submits comparisons and retrieves status/reports |

## Architecture and flow

Implement a standalone Rust analysis server importing rgctl crates directly.
Expose a purpose-built, versioned JSON HTTP contract consumed by a thin TypeScript
Backstage backend plugin. Analysis runs inside the Rust service using library
APIs; the service does not shell out to rgctl or proxy its existing GQL HTTP API.

The JSON CLI survey provides reference outputs for verifying equivalent crate
behavior, especially blast radius. Its transport and response projections are
not the public contract of the new service. Keep evaluation-only code under
`demo-evaluation/`; place the service in a separate implementation directory.

Use `rgctl-graph` snapshot/function metadata for changed-symbol analysis,
`rgctl-analysis` for blast radius, and graph/structured-query APIs for identities,
locations, and relationships. Reuse `rgctl-service` operations where appropriate;
do not restrict responses to its existing JSON projections when richer public
crate data is available. Pin all imported crates to the same rgctl revision.

1. Resolve the catalog application revision and effective Maven dependency tree.
2. Prepare isolated baseline/candidate environments, changing only the selected
   version or transitive override while holding application source constant.
   For the transitive CVE-remediation demo, keep Dep A's artifact identical and
   override only Dep B through Maven dependency management.
3. Acquire matching Maven source JARs for the resolved dependency versions and
   extract them for analysis; record artifact identities and checksums. Missing
   sources produce explicit diagnostics. Repository checkout fallback is deferred.
4. Use rgctl crate APIs to index source, compare snapshots, and obtain changed
   symbols and call relationships, using the validated binary outputs as
   implementation references.
5. Use rgctl's blast-radius analysis to find upstream impacted functions across
   dependency boundaries. Retain graph relationships for explanatory call paths;
   an impact set alone is not an ordered path.
6. Attach source locations, snippets, and supporting evidence paths.

Use the prepared baseline/candidate sources and their artifact identities.
Assume the current source identity approach is sufficient for the MVP; no further
identity investigation is a prerequisite. Exported function hashes and CFG
statements already pass the binary checks. Integrate the corresponding crate
operations while building this server, rather than adding a separate upfront spike.
Follow baseline callers for changed/removed methods. Added symbols do not imply
an existing application caller. Analyze patch releases identically to other upgrades.

Use effective dependency resolution plus code relationships for transitive impact.
Do not equate a dependency DAG with a call graph. Report ambiguity and unsupported
dynamic behavior rather than inventing confirmed paths.

## Interface contract

See the [JSON contract](03-json-contract.md), implemented
[OpenAPI schema](../analysis-server/openapi.yaml), and
[service guide](../analysis-server/README.md) for endpoints, request fields,
result structure, and error semantics.

The plugin-facing contract includes comparison submission, job status, reports,
and authorized source excerpts. Input includes entity reference, application source
revision, dependency coordinates, and target version; derive the baseline from
resolved dependencies.

Reports include resolved versions, source provenance, changed symbols, affected
application locations, call paths, evidence precision, and completeness diagnostics.
Distinguish potential code impact from observed behavioral failure. Never label a
declaration location as an exact call site or incomplete analysis as no impact.

Provide comparison jobs/status/results with symbol, impact, relationship,
and source-block data inline in the report. Return structured entities with stable identities,
qualified names/signatures, artifact/source identities, and explicit location
precision. Return the enclosing function/code block, its file and line range,
and source text so the UI can show the entire context. Exact call columns and
individual repeated occurrences are not required. Return capability/completeness diagnostics
for unavailable evidence; do not manufacture locations from function names.

Implement submission, status, and report routes in the existing TypeScript
backend scaffold during this phase, with backend-only Rust service configuration.
The full frontend workflow remains Phase 4.

The TypeScript backend owns Backstage authentication, catalog integration, and
authorization. The Rust server owns resolution/orchestration, bounded analysis
jobs, artifacts, and results. The service is accessed by the backend rather than
directly by the browser, and requires an authenticated service connection when
deployed beyond the local evaluation environment.

## Crate integration and remaining gaps

Rust crate integration is the selected architecture, not a conditional fallback.
Keep language analysis owned by rgctl and preserve its parallel extraction and
streaming/mmap access patterns. Keep CPU-heavy work off the async HTTP executor.

Use the validated function/block-level data to return complete source context.
Repeated calls within one block may share a single result. Do not add occurrence
storage work or exact-column requirements. Existing GQL projection limitations
are the reason for exposing our own JSON contract; they are not acceptance gates
for this server's internal data access. No upstream changes or independent Java
parser are assumed.

## Acceptance

- [x] Internal, public, transitive, and patch demos match independent expected paths.
- [x] Rust JSON HTTP results agree with independent demo expectations and the
      validated CLI/library behavior; provenance distinguishes old GQL probes.
- [x] The Rust server uses imported rgctl crates directly, with no rgctl subprocess adapter.
- [x] Maven source acquisition and effective baseline/candidate resolution work for the demos.
- [x] Transitive overrides keep Dep A unchanged while changing the selected underlying package.
- [x] The Rust service runs in local Compose and Backstage backend submission/status/report routes work.
- [x] Same-signature and same-line-count body changes are detected.
- [x] Full enclosing blocks are returned; repeated calls within a block need no separate occurrence records.
- [x] Removed methods use baseline callers; harmless line shifts preserve function matching.
- [x] Overload identity ambiguity is detected and reported; merged overload-specific changes/blocks are omitted.
- [x] Unused changes and unrelated functions do not become confirmed impact findings.
- [x] Missing sources and partial analysis are explicit; invalid snapshots fail jobs instead of appearing as no impact.
- [x] Source locations resolve to the exact analyzed inputs, recorded by digest.

## Execution result and limits

See the [validation record](../analysis-server/evidence/README.md) and concrete
[request/report examples](../analysis-server/examples/checkout-report.json).
Both dynamic plugins and the Rust service run in local Podman Compose. The
comparison API works through Backstage; frontend comparison controls remain Phase 4.

The implemented application scope is a configured standalone Maven module using
`src/main/java`, including all four demos. Remote application checkout and reactor
module selection are deferred. Maven source JAR acquisition is implemented.

All reports explicitly retain partial coverage: general call resolution is not
certified. The imported rgctl graph can merge Java overloads; the user authorized
continuing with that known limitation. The service observes ambiguity during
extraction, suppresses unsupported overload-specific claims, and does not modify
rgctl or introduce an independent Java parser. General overload identity repair
is follow-up upstream work, not a claim of this phase's completion.

## Follow-up milestone

Qualify and analyze one existing application beyond the four demo wrappers after
the initial service is working. Select the application and acceptance scope
separately; this does not block Phase 3 completion.
