# Go dependency comparison validation

Validated with the shared Rust service and rgctl `fix/module-qualified-symbols`
at `72823ed2`. Go module resolution/runtime checks used Go 1.27.4 on the host
and in the service container.

All six controlled examples passed through both the native HTTP service and the
live Backstage backend/container. Runtime execution independently checked the
behavior of the baseline and candidate releases using the normal module cache.

| Example | Changed dependency function | Application impacts | Runtime result |
| --- | --- | --- | --- |
| Public | Quote | Run, main | 105 → 110 |
| Internal | normalize | Run, main, through unchanged Quote | 105 → 110 |
| Patch | Validate | Run, main | true → false |
| Transitive | Encode | Run, main, through unchanged Render | 101 → 102 |
| Injected | RiskPolicy.Assess | Submit, Run, main | 101 → 102 |
| Unaffected | UnusedPreview | none | 105 → 105 |

The transitive override retains template v1.0.0, verifies its unchanged archive
checksum, and includes its wrapper in the path to token-core v1.0.1. The injected
example constructs RiskPolicy, passes it as Scorer through NewCheckout, stores
it on Checkout and invokes it from Submit. No graph/impact results are mocked.

Headless Chrome verified all fourteen discovery choices and opened all six Go
reports. Every positive displayed its graph nodes and full source block; the
injected case selected Submit as a testing area. The unaffected case displayed
“No affected application paths found” with no testing areas. No JavaScript
runtime exceptions occurred.

The Rust library's 64 tests pass, including Go coordinate compatibility,
production-source filtering and a native Go snapshot diff/tracing regression.
The existing helper-target tests passed, TypeScript type checking passed, and
all 24 onboarding backend/catalog-module tests passed. The shared service still
serves both plugins, with their existing data stores preserved.

All eight Java/Maven demos were also rerun through Backstage. For each, the set
of affected application files/qualified functions exactly matches its latest
saved successful report: checkout (3), delivery (2), notification (2), file-import
(2), no-impact (0), injected-policy (3), caller-separation (3) and stable-preview
(0). The Commons IO case completed its large-library trace; its original eight-minute
run time required a longer polling timeout than the small controlled examples.
Both plugin status APIs remained available, all original catalog entities were
retained, and the source-linked Spring API connection remained intact.

Fixture source, module proxy releases, descriptors, runtime copies, execution
logs, complete reports and local setup overrides remain outside tracked source.
See [Go setup and acquisition](GO.md) for the request contract and source policy.
