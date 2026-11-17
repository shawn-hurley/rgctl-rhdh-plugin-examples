# Phase 4 — Complete the dynamic plugin

**Status:** Complete for the controlled four-demo scope; browser and dynamic
distribution validation passed November 10, 2026.

**Start:** Phase 3 analysis and result contracts pass acceptance.

**End:** Users install dynamic packages locally and complete every demo through the UI.

## Backend

- Use a mock input-discovery service backed by a typed static manifest for the
  four demos. Provide authenticated `GET /api/dependency-impact/inputs`, with an
  optional entity reference filter and catalog access checks. Return configured
  revisions, Maven coordinates, baseline/target versions, strategy, and scenario
  descriptions. Keep the frontend independent of the manifest.
- Discovery is behind an interface so future package-manager-specific providers
  can use Artifactory or upstream open-source registries. Those integrations and
  additional containers are outside the controlled demo scope. The Rust service
  still performs real resolution, Maven source acquisition, and analysis.

- Extend the dynamically loaded TypeScript backend scaffold and the
  submission/status/report integration delivered in Phase 3. Use Backstage
  services for configuration, authentication, authorization, catalog integration,
  and logging.
- Resolve catalog/repository authorization in the TypeScript backend, then submit
  authorized analysis requests to the Rust server's JSON HTTP API. Use a typed
  client for the server contract; keep analysis logic in the crate-backed server.
- The Rust server runs bounded jobs outside HTTP request handling and persists
  artifacts/results. Cache only against source revision, resolved dependency
  graph, tool versions, and analysis options.
- Authorize submission, report reads, and source access server-side. Do not expose
  arbitrary shell operations or raw filesystem paths to browser requests.

Expose submission, status, and report operations through the Phase 3 contract.
Full source excerpts are inline in the authenticated report. Job states are queued, running, succeeded, or failed. Successful
reports separately identify incomplete coverage; successful execution does not
mean the analysis proves no impact.

## Frontend

- Add a Dependency Impact tab to catalog component pages.
- Let users select a source revision, dependency, and target version; show the
  effective baseline and actual candidate dependency versions.
- Show progress, changed symbols, affected application files, full source blocks,
  and expandable call paths from application code to changed dependency methods.
- Show the entire enclosing function or code block where the affected call is
  made, with file/line context and links to the analyzed revision. Repeated calls
  in one block may share a result; exact call-column highlights are not required.
- Include all four demos, including patch changes with unchanged public signatures.

Configured revisions and dependencies come from mock discovery; arbitrary Git
checkout and registry searches remain deferred. Links navigate within persisted
reports to the exact analyzed block. Local demos have no verified external
revision URL, so the UI does not invent repository links. Source identities and
digests remain visible, and complete JSON reports can be downloaded.

## Dynamic distribution

Produce independently exported frontend and backend artifacts with installation
configuration, frontend wiring, configuration schemas, and integrity information.
Embed private plugin dependencies appropriately while matching host-provided
Backstage dependencies to the pinned compatibility target.

Verify installation and disablement through configuration without rebuilding the
host. Ship the Rust server as a separate service container with rgctl crates linked
into its build. Document its endpoint and service credentials as backend-only
configuration. No rgctl CLI executable is required by the server at runtime.

Reference: [RHDH dynamic plugin requirements](https://github.com/redhat-developer/rhdh/blob/main/docs/dynamic-plugins/export-derived-package.md).

## Acceptance

- [x] Mock discovery returns the four demo choices through an authenticated, catalog-filtered backend endpoint.
- [x] All demos can be submitted and inspected through dynamically loaded packages.
- [x] The comparison form obtains the four demo choices from mock discovery without hardcoded frontend coordinates.
- [x] Reports, source snippets, evidence paths, and links agree on source revision.
- [x] Authorization is enforced on report and inline source operations.
- [x] Unavailable analysis tooling and failed jobs produce actionable UI feedback.
- [x] Packaged artifacts load independently of workspace development imports.
- [x] Installation, disablement, and re-enablement do not require a host rebuild.

## Execution result

The global page and dedicated entity tab provide discovery-backed selection,
editable target version, actual job stages, full source blocks, call-path
navigation, dependency-change comparison/filtering, diagnostics, and report
download. Report/source URLs reopen persisted results. The backend uses a typed
Rust service client and returns actionable `503` connection failures.

See the [validation record](../plugins/dependency-impact/evidence/README.md) and
[local usage/package guide](../local-dev/README.md). All four demos match their
independent expected application functions and paths through the UI. Partial
coverage and Java overload ambiguity remain explicit; the UI makes no claim
that an empty impact result proves upgrade safety.

## Testing-focus refinement

Implemented a follow-up focused on application areas to exercise for an update.
Boundary application callers form the testing areas; upstream callers appear as
context in a clickable Backstage graph. Full source blocks and report/source
links are retained. Dependency-wide change inventory is available in downloaded
JSON rather than the main UI. Missing or ambiguous evidence cannot turn an empty
result into an unqualified no-impact claim.

A fifth independent catalog demo, `no-impact`, upgrades pricing-maintenance
1.0.0 to 1.0.1. Only unusedAuditFee changes. Both application runs produce
`10.01;repeat=10.01,10.01;health=ok`; real analysis detects one change and zero
application impacts/paths. The four existing reports also passed graph/grouping
and keyboard source selection checks. No generated test scenarios, CVE feed,
or existing-test discovery was added.
