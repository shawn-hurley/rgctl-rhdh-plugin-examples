# Phase 1 — Demo applications

**Status:** Complete and accepted for function/block context display.

**Start:** Project decisions are recorded and no demo suite exists.

**End:** Four isolated catalog demos reproduce dependency-version behavior changes;
rgctl binary output identifies changed functions, upstream impact, and the enclosing
application function/block with statement context. Expected results and evidence
are recorded. The existing HTTP probes are supporting evidence, not a requirement
that the existing GQL endpoint expose the complete future server contract.

See the [binary verification](../demo-evaluation/evidence/binary-verification/README.md)
and [evaluation report](../demo-evaluation/evidence/REPORT.md). The UI will show the
full enclosing function or code block. Exact call-column ranges and repeated-call
occurrence preservation are not required. Public-crate integration and the new
Rust JSON HTTP contract belong to Phase 3, not Phase 1 acceptance.

## Proposed demo suite

| Demo | Change | Expected path |
| --- | --- | --- |
| Checkout | Private rounding helper changes beneath an unchanged public pricing method | Changed helper → public pricing method → application call site |
| Delivery | Public shipping calculation changes | Changed public method → application call site |
| Notification | Transitive token-processing dependency changes beneath an unchanged template dependency | Changed transitive method → direct dependency → application call site |
| File import | Released patch fixes file comparison with unchanged public signatures | Changed comparison implementation → application file-import comparison |

The first three examples are controlled Java/Maven fixtures. Application source
stays constant across each comparison. Controlled dependency versions are distinct
artifacts in an isolated Maven repository, never overwritten releases.

Each fixture needs a runnable entry point and focused behavioral test. A web UI
for the demo application is not required by the baseline.

## Isolation and catalog entries

Keep all evaluation work under the top-level `demo-evaluation/` directory,
including candidate checkouts, fixture source, dependency artifacts, scripts,
isolated Maven state, generated indexes, and HTTP evidence. Do not place evaluation
assets in future plugin, service, or application implementation directories.

Each of the four demos has its own directory and `catalog-info.yaml` Component
entity with a unique name. It remains independently runnable and selectable in
the catalog even if evaluation tooling is shared. Register any additional accepted
existing application as a separate entity as well.

## Transitive override scenario

Model an application depending on Dep A, which depends on vulnerable Dep B.
Dep A has not released a version carrying the fix. Keep Dep A's artifact version,
checksum, and source unchanged; override only Dep B through Maven dependency
management. Verify that the effective runtime dependency tree selects the intended
Dep B version and that the application still calls it through Dep A.

Expected output identifies Dep B's changed method, the unchanged Dep A wrapper,
and the affected application location. Include baseline/candidate dependency trees
and behavioral evidence. A controlled fixture may model a CVE remediation, but
must not claim a real CVE or fixed-version range without verified advisory evidence.
Live CVE-feed integration remains outside this phase.

## Evidence qualification for every demo

Define expected behavior, changed functions, upstream paths, affected application
files, enclosing function/block ranges, statement context, and unaffected controls
independently of rgctl output. The verified binary workflow compares exported
function hashes, runs blast radius for detected changes, and retrieves function
ranges plus CFG statements. Capture commands, responses, tool versions, sources,
and assertions. The UI shows full blocks for context rather than separate call
occurrence markers; repeated calls may share one displayed block.

Retain the successful GQL HTTP path probes and their projection limitations as
supporting evidence. The crate-backed Rust service and its own HTTP contract are
implemented and tested in Phase 3. There is no additional crate-access or source
identity investigation required before Phase 2.

## Released patch demonstration

Use `commons-io:commons-io:2.15.0` → `2.15.1`, targeting the documented fix for
`FileChannels.contentEquals`. Two eight-byte files differ in their first chunk;
with a four-byte buffer the old release incorrectly reports equality, while the
patch correctly reports inequality. An identical-file control passes in both.

The file-import example preserves public signatures and application source. The
original 2.16.0 → 2.16.1 reset scenario is retained as an unqualified candidate; its
baseline artifact could not be fetched. The selected 2.15.x pair was available in
local caches and both behaviors have been executed. Dependency source JARs are now indexed; HTTP path checks pass; binary verification also supplies changed-body evidence and
function/block context at the accepted precision.

This is a fourth demo and a release-category dimension: a patch may contain the
same internal or public implementation changes covered by other examples. Do not
classify every patch upgrade as breaking or every reachable change as a failure.

Source: [Commons IO release notes](https://commons.apache.org/proper/commons-io/changes.html).

## Additional real-world candidates

| Candidate | Investigation |
| --- | --- |
| Commons Lang 3.12.0 → 3.13.0 | `NumberUtils.createNumber` changes as a direct public-method example |
| Commons Text 1.10.0 → 1.11.0 | `StringTokenizer.getTokenList` behavior and the accompanying Commons Lang upgrade |
| OWASP Dependency-Check | Pin an application revision and qualify a reachable Commons Text/Lang change |

These are research candidates, not verified impact demonstrations. Inspect
released source and run focused tests before promotion. Phase 3 requires one
qualified existing application in addition to the small demos; a wrapper around
a real library alone does not satisfy that existing-application requirement.

Sources: [Commons Lang changes](https://commons.apache.org/proper/commons-lang/changes.html),
[Commons Text changes](https://commons.apache.org/proper/commons-text/changes.html),
[Dependency-Check](https://github.com/dependency-check/DependencyCheck).

## Work and deliverables

1. Establish the isolated evaluation layout and per-demo catalog identities.
2. Create the controlled examples and release-specific Maven artifacts.
3. Build the file-import patch demonstration against both released versions.
4. Qualify additional candidates through source inspection and behavioral tests.
5. Record a scenario manifest, independent expected results, and binary verification
   evidence for every demo; retain HTTP probes as supporting evidence.
6. Provide commands that build and exercise baseline and candidate without editing
   application source or depending on a shared developer Maven cache.

Scenario records contain application revision/source digest, artifact coordinates,
baseline and target versions, resolved dependency trees, source provenance and
checksums, input, expected old/new outcome, changed methods, expected call paths,
application locations, and unaffected controls. Record method signatures and
source ranges; do not identify overloads by simple name alone.

For transitive examples, verify the effective dependency version actually changes.
A declared version or dependency relationship alone is insufficient evidence.

## Validation and exit checklist

- [x] All four scenarios have independently reproduced old/new behavior.
- [x] Application source is identical across each comparison.
- [x] Changed artifacts are pinned and their sources match the analyzed versions.
- [x] Internal and transitive demos preserve the relevant public wrapper methods.
- [x] Unaffected paths and unused changed methods provide negative controls.
- [x] Full block context covers repeated calls; similarly named methods provide resolution controls.
- [x] Expected symbols and paths are derived from source/tests, not copied from rgctl output.
- [x] Required changed-function, impact, and block-context information is backed by binary outputs.
- [x] Output precision is defined as enclosing function/block with statement context.
- [x] Every demo has a separate catalog entity and stays within `demo-evaluation/`.
- [x] The transitive comparison changes only Dep B; Dep A's artifact remains identical.
- [x] Real-world candidate qualification evidence and remaining gaps are documented.
- [x] Build/run instructions and repeat runs use isolated evaluation Maven state.

Fresh-cache bootstrap and shared CI automation are Phase 2 deliverables.

Phase 1 establishes demo ground truth and binary feasibility; Phase 3 implements
the reusable Rust analysis server and its JSON HTTP contract. A source change demonstrates
potential impact; behavioral tests establish what happens for the chosen inputs,
not for all possible inputs.

## Recorded review decisions

- Use a new evaluation directory isolated from future application directories.
- Give each demo a separate catalog entry.
- Qualify expected output using the binary; preserve the completed HTTP probes as evidence.
- Model CVE remediation by overriding Dep B while Dep A remains unchanged.

## Accepted follow-on scope

The UI displays the full enclosing function/code block where an affected call is
made. Exact call columns and repeated occurrence preservation are excluded from
MVP requirements. Existing rgctl information is sufficient for Phase 1 acceptance.
Phase 2 provides reproducible tooling and dynamic plugin scaffolding. Phase 3
builds the Rust server importing rgctl crates and exposing the purpose-built JSON
HTTP contract. Source identity handling is assumed sufficient for now; recorded
experiments remain available without an additional investigation gate.
