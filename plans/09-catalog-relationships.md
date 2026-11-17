# Component/resource dependencies and subcomponents

**Status:** Implemented and verified. See the
[validation results](../analysis-server/CATALOG-RELATIONSHIP-VALIDATION.md).
The runnable applications, sample catalog, walkthrough and demo scripts remain
local and ignored by request. Generic backend functionality and tests are
committed; local-only mounts/configuration use an ignored Compose override.

## 1. Extend the catalog relationship view

Start with persisted application graphs and authorized catalog reads. Expose
`dependsOn`/`dependencyOf` for Components and Resources, and component
`partOf`/`hasPart` from `spec.subcomponentOf`. Read live processed relations,
preserve namespaces, bound traversal/cycles, and omit inaccessible entities.
End with direct and transitive catalog paths available through `/relationships`.

## 2. Add source-backed transitive and injection examples

Start with controlled multi-module Java sources. Build Orders and Audit using a
direct workflow library and a transitive file-storage library. Exercise
constructor/field injection and method-argument injection against the same
interface. Acquire Maven source JARs during opted-in graph onboarding and join
native call/receiver facts into source-to-resource paths.
End when independent Java runtime checks and persisted graph checks agree on
each used resource and exclude the other implementation's resource.

## 3. Register, verify and document

Start with the service, libraries, parent Component, and storage Resources in the
catalog. Verify standard catalog cards/relations, native symbol references,
transitive Maven provenance, authorization and existing API regressions. Keep
hierarchy/configuration relations separate from executable code evidence.
End with runnable examples, a walkthrough, recorded verification and a
self-contained commit in the onboarding worktree.
