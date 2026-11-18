# Dependency impact analysis service

Comparison API in the shared `rgctl-service` HTTP server, importing the selected `rgctl-source` checkout's graph, pipeline,
analysis, registry, and Java crates. Maven is invoked for dependency resolution
and artifact acquisition; rgctl is never invoked as a subprocess.

The [OpenAPI contract](openapi.yaml) describes the Rust endpoints. Backstage
exposes the comparison operations under `/api/dependency-impact/comparisons`,
authenticates callers, and checks catalog entity access using delegated caller
credentials before submission or report/source reads. The browser receives no
Rust service credential.

## Run locally

Prerequisites: a selected `rgctl-source` checkout, a Rust compiler meeting that
checkout's minimum version (currently 1.99), Maven, and JDK 17 or later. No
toolchain file or isolated dependency caches are required.

From the application repository root, configure `local-dev/service-config.json`
and run the [shared service](SERVICE.md). It serves comparisons and application
onboarding on the same port with `RGCTL_SERVICE_TOKEN`.

The local-only `prepare-demos.py` builds controlled releases and source JARs in
the ordinary Maven repository. Published artifacts use Maven acquisition; private
repositories and credentials use normal Maven settings.

The `dependency-impact-server` binary remains a compatibility entry point for
standalone comparisons using `IMPACT_CONFIG`, `IMPACT_SERVICE_TOKEN` and
`IMPACT_LISTEN`. It must not write the same report store as the shared service.

## Submit a comparison

Direct Rust API example:

```sh
curl -H 'Authorization: Bearer rgctl-local-service' \
  -H 'Content-Type: application/json' \
  -d '{"entityRef":"component:default/dependency-impact-checkout","applicationRevision":"demo-baseline","dependency":{"groupId":"io.rgctl.demo","artifactId":"pricing"},"targetVersion":"1.1.0","strategy":"direct-upgrade"}' \
  http://localhost:7070/v1/comparisons
```

The response is `202` with an `id`. Poll `/v1/comparisons/{id}` until the state is
`succeeded` or `failed`; retrieve `/v1/comparisons/{id}/report` after success.
Use the same authorization header on each request.

To submit through Backstage instead, use
`http://localhost:7007/api/dependency-impact/comparisons` with Backstage
credentials (the local setup's development token is
`dependency-impact-local-development`). The request body is identical.

| Demo | Group | Artifact | Target | Strategy |
| --- | --- | --- | --- | --- |
| checkout | io.rgctl.demo | pricing | 1.1.0 | direct-upgrade |
| delivery | io.rgctl.demo | shipping-rules | 1.1.0 | direct-upgrade |
| notification | io.rgctl.demo | token-core | 1.0.1 | transitive-override |
| file-import | commons-io | commons-io | 2.15.1 | direct-upgrade |
| no-impact | io.rgctl.demo | pricing-maintenance | 1.0.1 | direct-upgrade |
| injected-policy | io.rgctl.demo | injected-risk-policy | 1.0.1 | direct-upgrade |
| caller-separation | io.rgctl.demo | routing-risk-policy | 1.0.1 | direct-upgrade |
| stable-preview | io.rgctl.demo | preview-risk-policy | 1.0.1 | direct-upgrade |

All eight use `applicationRevision: "demo-baseline"` and the corresponding
`component:default/dependency-impact-<demo>` entity reference.

## Analysis behavior

- A configured application path selects one standalone Maven module. Only
  `src/main/java` application sources are analyzed; tests are excluded. Arbitrary
  repository checkout, reactor selection, classified artifacts, and non-JAR
  analysis are not supported in this version and are not silently guessed.
- The application input is copied once and hashed. Candidate POM edits happen
  only in job-owned copies. The same application Java source is used on both sides.
- Resolve both effective dependency trees, verify that the requested version
  took effect, and report all resulting dependency deltas. For transitive
  overrides, verify that direct versions and binary checksums remain unchanged.
- Acquire binary and matching source JARs through Maven. Record coordinates and
  SHA-256 digests; missing source JARs yield diagnostics and partial coverage.
  There is no release-checkout fallback.
- Stream parallel rgctl Java extraction into columnar snapshots. Final snapshot
  paths and node IDs use snapshot-root-relative logical source locations, with
  dependency ownership prefixes stable across versions. The physical input root
  stays on the snapshot context and is used for source/annotation reads.
- Use rgctl `diff_snapshots` with `VecDiffSink`, then filter collected node events
  to functions and dependency ownership. Equal code hashes suppress metadata-only
  changes such as line shifts. Run blast radius on the augmented upstream closure
  for modified/removed baseline functions.
  Added methods have no implied baseline callers. Renames appear as removal/addition.
- An adapter overlay expands compatible implementation targets using rgctl type
  ancestry, override facts, and canonical callable identities. It includes candidates
  across profiles, primary preferences, and conditional factories. Understood
  literal field qualifiers can narrow candidates; unknown bindings stay broad.
  Application and dependency inputs are prepared explicitly by this service;
  the selected callable-identity worktree does not perform automatic SDK ingestion.
- Reports include potential impacts and ordered paths backed by `Calls` and
  `PossibleDispatch` edges. Possible edges include declared/implementation names
  and paths use `resolution: "possible-dispatch"`. Persisted rgctl snapshots retain
  their original facts.
  Full enclosing source blocks are inline for affected application functions,
  changed dependency functions, and available intermediate path functions.
  Source paths are relative to the application's Java root or the source JAR.
- Source checksums identify the analyzed inputs. Named local revisions are not
  Git commit claims. No remote source links are invented for local demos.

The selected overload-fix worktree preserves canonical callable discriminators.
The service uses those discriminators for cross-snapshot matching, including
annotated methods whose display signatures are not unique. Its extraction
observer only reports duplicate canonical identities; distinct overloads are not
suppressed. Regression tests cover both annotated overload edits and ordinary
body changes. Historical reports produced by the old build retain their original
warnings and provenance. The adapter now covers conservative interface dispatch;
see the [dispatch policy and qualification results](../demo-evaluation/qualification/DISPATCH-POLICY.md)
for the remaining gaps.

All reports currently use `coverage.status: "partial"` because complete Java call
resolution is not certified. A successful job and an empty impact list do not
prove upgrade safety. Reachability is potential static impact, not a behavioral
test result. Newly added/removed whole dependency artifacts are shown in the
resolution deltas; cross-artifact symbol migration is not inferred.

## Jobs and operations

One analysis executes at a time outside the HTTP executor. The queue defaults to
eight pending jobs; a full queue returns `503` with `Retry-After`. Job metadata,
logs, source artifacts, snapshots, and reports persist in `dataDir`. An interrupted
job becomes failed on restart and can be resubmitted. Use one service instance per
data directory. Automatic job resume, cleanup/retention, and result caching are
not implemented.

Each Maven invocation has a three-minute limit. Source archives are restricted
to enclosed Java paths and a 256 MiB extraction limit. Comparisons exceeding the
supported change/path report limits fail explicitly instead of silently truncating.

`GET /health` is unauthenticated. All comparison endpoints use the shared
`RGCTL_SERVICE_TOKEN`; configuration lives under `comparisons` in the combined
service config. See [service configuration](SERVICE.md).

The build records the rgctl commit and a source-content fingerprint, including
working-tree changes. All rgctl dependencies come from that same checkout.

Focused validation during development:

```sh
cargo test --manifest-path analysis-server/Cargo.toml
```

This covers body changes without signature/line-count changes, harmless line
shifts, add/remove identity, overload diagnostics, and transitive POM edits.

## Logical source paths and native comparison

The library pipeline currently writes physical extraction paths. The service's
snapshot-construction boundary uses rgctl's segmented spill writer to persist
logical paths and recomputed deterministic node IDs, remapping every edge endpoint.
Only the ID map is retained across nodes; this does not hydrate an entire graph.
The intermediate extraction snapshot is removed after successful construction.
Root-relative paths reject parent traversal and unrelated absolute roots. Physical
source files remain unchanged and are read through the snapshot's canonical root.
This adds a streaming rewrite during construction rather than normalizing keys
ad hoc on every diff request. Native analysis fixes are maintained in the selected rgctl worktree; see the
[implementation record](../demo-evaluation/value-flow/INITIALIZATION-CLOSURES-CONTEXTS.md).

Native diff stable keys replace the adapter's signature fallback. Function filtering
happens after collecting diff events for now: whole-graph stats and internal diff
work remain unfiltered. Fresh reports record `comparisonEngine`, `pathIdentity`,
and `functionFilter` in provenance options. Existing saved reports remain readable;
new comparisons create fresh snapshots and IDs. The path-policy change does not
alter dependency coordinates, public source-block paths or coverage guarantees.

Context-dependent comparisons evaluate each changed function separately. Source
paths are reconstructed from the same retained call edges as impact detection,
so an excluded preview caller cannot reappear in a submission's explanatory path.
