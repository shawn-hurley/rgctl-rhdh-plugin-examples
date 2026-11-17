# Dependency Impact Dynamic Plugin

The new [application onboarding work](08-application-onboarding.md) adds a separate
Rust graph service and dynamic backend for catalog-connected Java applications.

## Goal

Compare two versions of a dependency and show affected application source in a
Backstage/RHDH dynamic plugin, with highlighted source, revision-specific links,
and explanatory call paths. Cover internal changes behind public APIs, public
method changes, transitive changes, and a released patch upgrade.

## Roadmap

Phase boundaries are entry and exit milestones, not calendar commitments.
Dates depend on staffing and measured effort. This is the agreed planning
baseline, updated with the user's Phase 1 review decisions.

| Phase | Start | End | Status |
| --- | --- | --- | --- |
| [1. Demo applications](01-demo-applications.md) | Decisions recorded; no fixtures | Four isolated catalog demos with verified behavior and binary change/impact/block-context evidence | Complete — accepted |
| [2. Local test environment](02-test-infrastructure.md) | Demo fixtures established | Podman Compose host, four catalog entries, dynamic plugin skeletons, brief setup guide | Complete — locally verified |
| [3. Rust server and HTTP API](03-impact-analysis.md) | Infrastructure and plugin skeletons work | Four demos pass through a crate-backed Rust service with Maven source acquisition and thin Backstage backend routes | Complete — partial coverage and overload limitation recorded |
| [4. Dynamic plugin](04-dynamic-plugin.md) | Analysis contract passes acceptance | Users run all demos through dynamically installed packages | Complete — four demos and packaging verified |
| [5. Release validation](05-release-validation.md) | Complete dynamically loaded workflow | Fresh environment passes acceptance and runbook reproduces demos | Planned |

Sequence: Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5.
The follow-up [native CFG/PDG and interprocedural repair plan](06-native-dependence-repair.md)
defines the upstream bug-fix workstream, including shared taint dependencies,
separate from the original product delivery phases. It is a local draft for review.
Minimal dynamic plugin skeletons belong to Phase 2; the full UI belongs to Phase 4.
Phase 1 uses `demo-evaluation/` for fixtures, isolated Maven state, rgctl HTTP
probes, and evidence. Phase 2 provides the local Podman Compose environment and
minimal dynamic plugin scaffolding; automated checks and CI are not planned.

## Recorded decisions

- Java/Maven first; controlled examples plus released dependencies and an existing application.
- Local Backstage development and local dynamic loading; no cluster required.
- Local environment first, using Podman Compose. Phase 2 has no toolchain-pinning,
  isolated-cache, automated-check, or CI deliverables. Keep setup documentation brief.
- Dynamic export and load testing from the beginning, rather than retrofitting packaging.
- React frontend → thin TypeScript Backstage backend plugin → Rust analysis
  server → imported rgctl crates. The Rust server exposes a purpose-built JSON
  HTTP API for comparisons, impact, symbol locations, and evidence.
- Existing JSON CLI results are validation references. The Rust server invokes
  rgctl crate APIs directly rather than wrapping CLI processes. Existing rgctl
  GQL HTTP limitations do not define the new server's response contract.
- Separate demo catalog entries, with all evaluation assets isolated under
  `demo-evaluation/` rather than future application implementation directories.
- The transitive scenario overrides Dep B to model CVE remediation while Dep A
  has no release incorporating the fix and remains unchanged.
- Rust is the chosen analysis-server implementation. Start with existing rgctl
  crate APIs during Phase 3 implementation. No separate upfront crate-access
  verification is required. No rgctl
  modifications are scheduled or implied by this architecture decision.
- On-demand comparisons for catalog applications at pinned source revisions.
- Full enclosing source function/block context and revision links, with impact paths.
  Exact call columns and repeated occurrence preservation are not MVP requirements.
- Rust server implementation and its JSON HTTP contract belong entirely to Phase 3.
  Maven source acquisition and thin Backstage submission/status/report routes are
  included. An additional existing application is a follow-up milestone and does
  not block Phase 3 completion. Full comparison UI work remains Phase 4.
  Source identity handling is assumed sufficient for now, without a separate investigation gate.
- Patch releases receive the same analysis as other upgrades; version numbering
  does not establish safety or impact.

## Initial environment findings

The application workspace was empty when planning began. The sibling `../rgctl`
checkout identifies itself as v0.4.26; the PATH binary reported v0.4.12. The Phase 1
evidence records the tested binary. Phase 2 does not add toolchain pinning.

The sibling provider's [graph investigation](../../rgctl-provider/docs/graph-extension-investigation.md)
records differences between extraction locations and persisted graph locations.
These are risks to verify through existing interfaces, not a requirement to change
rgctl. Do not present function declaration locations as exact call-site ranges.

## Boundaries

CVE feeds, automated upgrade PRs, additional ecosystems, OpenShift deployment,
and deployed RHDH certification are follow-on work. General dependency-change
analysis is the initial capability despite the workspace name.

## Demo expansion after native qualification

The Java/Maven product catalog now includes eight scenarios: the five original
flows plus constructor injection, caller separation, and stable preview using
static storage and a lambda. Each has a separate catalog Component and independent
runtime expectations. See [the runbook](../local-dev/DEMO-RUNBOOK.md).

The remaining qualification omissions are [known bugs](../demo-evaluation/KNOWN-BUGS.md),
not blockers to delivering these controlled demos. Keep their original expectations
and the recall gate; do not count them as successful negative cases. Other
language ecosystems remain outside this product-demo pass.

## Maintenance

The continuing analysis investigation qualifies individual RGCTL layers as well
as end-to-end impact. See the [CFG, PDG, dispatch and vulnerability qualification
plan](../demo-evaluation/value-flow/ANALYSIS-QUALIFICATION.md) for scenario
requirements, evidence rules and investigation start/end milestones. Its output
is an external implementation demonstrated against paired controls, followed by
an evidence-backed capability and requirements matrix and decisions about where
facts should live. Existing Calls audits must not be presented as qualification
of those other layers.

Update each phase's status and evidence when its exit criteria pass. Preserve
independent expected results separately from generated analysis. Record decisions
from the Phase 1 review in its document before creating demo implementations.

## Phase 1 execution result

Phase 1 is accepted. All four demo behaviors pass, HTTP graph paths were verified,
and binary checks identify changed functions, upstream impact, function ranges,
and statement context. This is sufficient for the agreed full-block UI.
See the [binary verification](../demo-evaluation/evidence/binary-verification/README.md).

Public-crate integration and the Rust JSON API are Phase 3 work. Exact per-call
columns and occurrence multiplicity are outside the MVP scope. Recorded source
identity experiments are retained as evidence without an additional prerequisite.
