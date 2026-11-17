# Phase 5 — Validate and deliver the demo release

**Status:** Java/Maven demo acceptance implemented. All eight runtime oracles and
HTTP comparisons pass; browser acceptance and packaging evidence are recorded in
the [demo release evidence](../demo-evaluation/evidence/demo-release/README.md).
Fresh-environment release certification remains separate from this local pass.

**Start:** The complete workflow operates through dynamically loaded packages.

**End:** A fresh local environment passes acceptance and another developer can
reproduce every demonstration using the runbook.

## Acceptance matrix

| Scenario | Required outcome |
| --- | --- |
| Internal helper | Impact reaches application code through an unchanged public method |
| Public method | Changed public method identifies application callers |
| Transitive dependency | Impact crosses the unchanged direct dependency into application code |
| Patch release | Commons IO upgrade identifies relevant code despite unchanged public signatures; focused tests demonstrate the fix |
| No application impact | An actively used dependency has an unused function change; real changes are detected, application behavior is unchanged, and no application paths are identified |
| Constructor injection | Impact follows an interface value from construction through the injected service to the changed implementation |
| Shared caller separation | Submission is impacted through a shared gateway; preview using an unchanged implementation is excluded from findings and explanatory paths |
| Stored receiver and closure | A changed alternative implementation produces no application paths when the static field and lambda select the unchanged implementation |
| Negative controls | Unused changes and unrelated methods do not become confirmed findings |
| Block context | Full enclosing source blocks display correctly; repeated calls may share a block; overloads, removals, and same-line-count edits are handled |
| Incomplete inputs | Missing sources, resolution failures, unavailable rgctl, and incomplete graphs produce diagnostics |
| Revision correctness | Snippets and links match the analyzed revision |
| Packaging | Installation, restart, disablement, and browser tests pass using exported artifacts |

Run fixture behavior checks, rgctl integration checks, backend/frontend tests,
and browser acceptance against the packaged plugin. Verify from fresh generated
state as well as supported cache reuse. Record tool versions, analysis duration,
resource usage, and unresolved coverage limits for each demonstration.

## Deliverables

- Updated plan documents and completion evidence.
- Fixture manifests, pinned source/artifact provenance, and independent expected results.
- Local validation results and packaged frontend/backend dynamic artifacts.
- Local environment configuration and analysis runtime setup.
- A demo runbook covering bootstrap, inputs, expected output, interpretation,
  source navigation, failure diagnosis, and cleanup.

The runbook must explain that reachability shows potential impact and a focused
behavioral test demonstrates behavior only for its tested inputs. It must distinguish
an empty complete result from an incomplete analysis.

## Deferred validation

OpenShift deployment, deployed RHDH certification, broader host compatibility,
other language ecosystems, CVE integrations, and upgrade-PR automation are outside
this local demo release. Dynamic compatibility is exercised locally throughout
development and is not deferred to this final phase.
