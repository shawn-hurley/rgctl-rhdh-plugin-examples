# Shared-service validation

Validated on 2026-11-18 against rgctl `fix/module-qualified-symbols` at
`72823ed2`, using the existing local RHDH hosts and persisted stores.

- Cargo: 122 passing test executions across the library and helper binaries,
  including four new shared-server tests. The helper binaries repeat some
  existing analysis tests; this is not a count of distinct scenarios.
- Backstage: 13 application-graph backend tests and 11 catalog-import/module
  tests passed.
- Container: built one `localhost/rgctl-service:dev` image. Both Compose host
  configurations point to `http://rgctl-service:7070` with one service token.
  Process inspection confirms one running Rust container with `rgctl-service`
  as PID 1. Both host ports 7070 and 17071 return the combined health response.
- Migration: checked both old queues were idle, stopped their Rust containers,
  reused the original report/graph volumes and restored six imported catalog
  locations after recreating the RHDH hosts. The startup guard rejected the
  legacy writer before migration and accepted existing volumes afterward.
- Preservation: the saved comparison report has identical canonical JSON
  SHA-256. All six previous graph versions retain their commit, source checksum
  and snapshot checksum; native node reads work for those historic versions.
- Catalogs: all 209 Dependency Impact entities and 19 Application Graph entities
  were restored. Every prior relationship on non-Location entities remains
  present, including component/resource dependencies and API inverse relations.
- Fresh work: submitted the no-impact Maven comparison through the Dependency
  Impact backend and `orders-app` onboarding through the Application Graph
  backend. Both succeeded using the shared service. Both backend status APIs
  report their Rust service available.
- API evidence: the Spring client/provider connection remains `source-linked`
  with its GET `/api/random` caller/callee evidence. Generated OpenAPI retains
  Quote and QuoteResource DTO schemas and the int64 `/api/{id}` path parameter.

The shared-router tests hold the actual worker semaphore while submitting work
to both APIs, verifying both jobs stay queued until the shared permit is released.
They also cover bearer authentication, real Git onboarding/native snapshots,
independent interrupted-job recovery and rejection of overlapping stores.

Local demo inputs, migration overrides and execution logs remain excluded from
Git. The tracked configuration starts with no comparison applications; configure
those inputs as described in [SERVICE.md](SERVICE.md) and the local setup guide.
