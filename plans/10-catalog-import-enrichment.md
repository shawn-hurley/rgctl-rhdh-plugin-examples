# Import-driven catalog enrichment

## Phase 1: Import and automatic indexing

Start with the graph service and dynamic onboarding backend. Add a dynamic
catalog module using Backstage's catalog processing extension point. Ordinary
imports opt in through a Component annotation, submit one asynchronous job and
publish job status during later processing. Preserve the imported descriptor
through outages and keep manual declarations intact.

End when individual browser imports trigger native graph persistence without
manual onboarding API calls.

## Phase 2: Standard dependency and API fields

Start with persisted manifests, authorized catalog service reads and imported
peer Components/Resources. Join direct Maven coordinates and reachable resource
URIs into `dependsOn`. Emit provider API entities from discovered operations,
then match consumer destinations, methods and routes to populate `consumesApis`
and `providesApis`. Require explicit endpoint bindings when source facts do not
establish host identity. Keep `subcomponentOf` declared.

End when Backstage's native Dependencies/API tabs show generated relations and
the source descriptors still contain no dependency/API declarations.

## Phase 3: Local walkthrough and qualification

Start with individually importable local-only descriptors for four Maven/DI
Components and the existing Spring HTTP provider/consumer. Run all six through
the actual browser wizard, verify before/after catalog fields and both
injection-specific resource controls, and capture the resulting built-in views.

End with passing processor/enrichment and backend tests, a reusable ignored
walkthrough, and a clean import-ready local catalog. Demo assets stay outside Git.
