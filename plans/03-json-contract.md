# Phase 3 — JSON contract

**Status:** Initial version implemented. Source JAR acquisition and inline source
blocks are agreed requirements. See the [OpenAPI schema](../analysis-server/openapi.yaml)
and [verified response example](../analysis-server/examples/checkout-report.json).

## Service boundary

The browser uses authenticated Backstage routes under
`/api/dependency-impact/comparisons`. The Backstage backend resolves the catalog
entity and its configured application source, checks access, and forwards an
authorized request to the Rust service. Browser requests cannot supply arbitrary
filesystem locations or Maven repository credentials.

The Rust service imports rgctl crates directly. It resolves Maven dependencies,
acquires source JARs, indexes and compares code, runs blast-radius analysis, and
stores jobs and reports. Source repositories, local demo locations, and Maven
repositories are configured server-side. The four local demos need no remote
application repository checkout to run.

## Endpoints

| Rust endpoint | Result |
| --- | --- |
| `GET /health` | Service availability |
| `POST /v1/comparisons` | `202` with job ID and initial status |
| `GET /v1/comparisons/{id}` | Job status, progress stage, and failure details |
| `GET /v1/comparisons/{id}/report` | Completed report including source blocks |

The Backstage backend provides the three comparison operations under its own
plugin route and checks access on every operation. The Rust health endpoint is
operational and is not a comparison operation.

Start with one executing analysis job at a time and a bounded queue. Store job
metadata and reports on the service's local volume. Jobs interrupted by restart
become failed with an interruption diagnostic; automatic resume is deferred.

States: `queued`, `running`, `succeeded`, `failed`.
Running stages: `resolving`, `acquiring-sources`, `indexing`, `comparing`,
`tracing-impact`, `assembling-report`. Report actual stages rather than invented
percentage progress.

## Submission

Proposed browser-facing request; coordinates and versions here are illustrative:

```json
{
  "entityRef": "component:default/dependency-impact-notification",
  "applicationRevision": "demo-baseline",
  "dependency": {
    "groupId": "org.example",
    "artifactId": "dep-b",
    "type": "jar"
  },
  "targetVersion": "1.0.1",
  "strategy": "transitive-override"
}
```

`strategy` is `direct-upgrade` or `transitive-override`. Baseline versions come
from effective Maven resolution. Coordinate matching includes type and optional
classifier; group/artifact alone must not silently select multiple artifacts.
For multi-module applications, a module selector is needed when the configured
application scope does not uniquely determine the dependency.

The backend resolves the application reference to a server-configured source
input. The Rust job records the exact application source identity it analyzed;
the same application source is used for baseline and candidate. Named demo
revisions map to prepared local inputs. Remote repository support can later use
resolved commit identities without changing comparison semantics.

A direct upgrade changes the selected direct dependency version. A transitive
override adds or changes dependency management for the selected underlying
artifact. It leaves Dep A unchanged. Both strategies rerun Maven resolution and
report the actual resolved dependency differences, including secondary changes.
If the requested version is not effective, the comparison fails with a resolution
diagnostic rather than analyzing the wrong artifact.

## Report

The report is versioned with `schemaVersion: "1"` and contains:

| Field | Content |
| --- | --- |
| `comparisonId` | Owning job ID |
| `application` | Entity reference and analyzed source identity |
| `resolution` | Requested change, baseline/candidate effective versions, actual dependency deltas |
| `sources` | Application and Maven artifact identities, source JAR origin, checksums, acquisition status |
| `changes` | Added, removed, or modified dependency symbols with qualified names/signatures and baseline/candidate references |
| `impacts` | Affected application functions, source-block references, changed-symbol references, and explanatory path references |
| `paths` | Ordered application-to-change symbol chains with supporting call edges |
| `sourceBlocks` | Revision-specific enclosing functions with file, inclusive line range, and source text |
| `coverage` | Complete or partial analysis within the declared static-analysis scope and known limits |
| `diagnostics` | Structured problems tied to a stage, artifact, or symbol |
| `provenance` | Server and rgctl revisions, analysis options, and timestamps |

Symbol IDs are opaque. Match symbols across snapshots using source/artifact
ownership, qualified names, and signatures; temporary extraction directories
must not determine identity. Retain the original rgctl references as evidence.

Each source block identifies `baseline` or `candidate`, source ownership, relative
file path, inclusive `startLine`/`endLine`, and text. Locations are function/block
precision. Repeated calls within a function can share one impact and source
block. No exact call columns or individual occurrence records are required.

Follow baseline callers for modified/removed dependency functions. An added
function has no implied baseline application caller. Include changed but unused
functions in `changes` without inventing an application impact. A call path must
be supported by graph edges; a blast-radius set alone does not establish order.

The Rust service returns full source blocks inline in the report, including
source identity, relative file path, inclusive line range, and source text. This
is an agreed requirement: the frontend does not reconstruct the blocks or acquire
dependency sources itself. Separate symbol/source queries
or paginated report sections can be introduced if measured report size requires
them. Any truncated result must say so explicitly.

## Sources and outcomes

Acquire the matching Maven source JAR for each analyzed resolved dependency
version, including dependencies needed to traverse transitive call paths. Reuse
normal Maven repository configuration and caching. Record binary and source
artifact identities and checksums. Do not substitute a nearby release's sources.

Missing dependency sources may yield a succeeded job with partial coverage and
any findings that are still supported. Unavailable application source or failed
effective dependency resolution prevents the comparison and fails the job.
An empty partial report must never be presented as proof of no impact.

Static reachability means potential impact, not demonstrated behavioral failure.
"Complete" describes processing within the declared analysis scope; it does not
claim to model all reflection or runtime dispatch.

Errors use an envelope containing `code`, `message`, and optional `details`.
Proposed HTTP behavior: malformed input `400`, unauthorized/forbidden `401/403`,
unknown job `404`, report not ready `409`, and queue full `503` with `Retry-After`.
Failures after submission are recorded on the job. Diagnostics identify missing
sources, ambiguous relationships, unsupported graph schemas, and partial graphs.

## Implemented scope and follow-up

- Applications are server-configured standalone Maven modules. Named local demo
  revisions are recorded together with exact input digests. Remote application
  checkout and Maven reactor selection are deferred.
- Unclassified JARs are supported. Other artifact types/classifiers are rejected
  or diagnosed rather than silently matched to a different artifact.
- Reports currently always declare partial static coverage. Merged Java overload
  identities receive ambiguity diagnostics and no overload-specific change/block
  claims. Paths involving such identities are marked ambiguous.
- Source blocks are inline. An impact's `sourceBlockId` is null when reliable
  block attribution is unavailable, with a diagnostic explaining why.
- Request examples for all four demos and a complete checkout report are under
  `analysis-server/examples/`. Further UI-driven API changes belong to Phase 4.
