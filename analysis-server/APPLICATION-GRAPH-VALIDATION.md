# Application graph validation

The separate Rust service and dynamically loaded RHDH backend were exercised
against actual upstream Java repositories, registered through standard catalog
Components and an API entity. The Java services were indexed from source; these
checks do not deploy their application runtimes.

| Input | Revision | Native graph |
| --- | --- | --- |
| [Spring Quoters](https://github.com/spring-guides/quoters) | `173cb2ad861e9e83404ae73bc2ddf0e299a5af79` | 91 nodes, 242 edges; 3 provider operations |
| [Spring Consuming REST](https://github.com/spring-guides/gs-consuming-rest), `complete` | `5cbc69ed1b5ed2d96c0975e4e0dd718f8a2c0cb3` | 38 nodes, 94 edges; 1 consumer operation |

Native rgctl revision: `72823ed2fb4c5aa3f5fa1614b08d7cecd545ef0b`.
Source digest: `da2ab211cf53d1f549c2a674dc6ee529b982fa68af80e88972dae78175950d45`.
The implementation adds no native rgctl changes.

## Verified behavior

- Both Components onboard successfully through the real RHDH backend.
- Native snapshots reopen and return actual graph nodes and typed `Calls` edges.
- `ConsumingRestApplication.run.$lambda$0` is connected through `GET /api/random`
  to `QuoteController.getRandomOne`, gated by the shared `api:default/quotes-api`.
- Every cross-service node reference resolves inside its own application graph.
- Re-onboarding creates new immutable versions. Previous manifests and graph
  nodes remain readable. Accessing a version through another application returns
  404.
- Removing/restoring the consumer's standard `spec.consumesApis` field changes
  the connection without changing either stored graph version. The test restores
  the descriptor and catalog relationship before completing.
- Rust-service recreation and restart preserve graph versions, source/graph
  checksums, native node reads and connections. RHDH restart is handled by waiting
  for asynchronous catalog hydration.
- Chrome shows both the provider and consumer on the ordinary Backstage API
  catalog page. No new frontend plugin is needed for those catalog relationships.
- All three dynamic plugins export and package successfully.

## Automated checks

- 107 Rust tests pass across the analysis crate's binaries, including 11 new
  onboarding tests and the existing comparison/dispatch tests.
- 7 backend tests pass, including catalog authorization, source override
  rejection, namespace handling, live relationship edits and outage behavior.
- Rust formatting, staged whitespace checks and shell syntax checks pass.

The [setup guide](APPLICATION-GRAPH.md) provides commands to repeat the live
onboarding, catalog-edit and restart checks. Generated JSON results, source
provenance, browser screenshots and package integrity values stay in the ignored
development-output directory.
