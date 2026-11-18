# Go dependency update comparisons

Go comparisons use the shared Rust service, comparison endpoints and Dependency
Impact screen. The service imports rgctl's Go plugin as a crate. Go's module
commands resolve/acquire releases; they do not supply the call graph or run the
application. Native rgctl snapshot diff, function filtering, dispatch analysis,
impact tracing and source blocks are shared with Java comparisons.

Application Graph catalog onboarding remains Java/Maven in this change.

## Configure and submit

Configure one application module per Component under
`comparisons.applications` in the shared service config:

```json
{
  "component:default/go-checkout": {
    "path": "/applications/checkout",
    "revision": "baseline"
  }
}
```

Mount that module into the service and register the Component in Backstage.
Go coordinates use `modulePath`, while existing Maven requests keep their
`groupId`/`artifactId` shape:

```sh
curl http://localhost:7070/v1/comparisons \
  -H 'Authorization: Bearer rgctl-local-service' \
  -H 'Content-Type: application/json' \
  -d '{"entityRef":"component:default/go-checkout","applicationRevision":"baseline","dependency":{"modulePath":"example.org/pricing"},"targetVersion":"v1.0.1","strategy":"direct-upgrade"}'
```

Through Backstage, use `/api/dependency-impact/comparisons` and the caller's
Backstage credentials. Poll the job and read its report as for Java. Add an input
to the controlled discovery manifest using `packageManager: "gomod"` to make
it selectable in the UI. The browser receives no module repository credentials.

The container includes Go. Native development needs Go installed alongside Rust.
Acquisition uses the normal Go module cache, `GOPROXY`, `GOPRIVATE`, `GONOSUMDB`
and checksum settings. Optional `comparisons.goProxy` selects a configured proxy
without changing browser requests. The supplied Compose file mounts the normal
`~/go/pkg/mod` cache; adjust that mount if your normal cache is elsewhere.

## Comparison process

1. Copy/hash the configured application once; the supplied source is unchanged.
2. Create separate baseline/candidate working copies. Only the candidate runs
   `go mod edit -require=<module>@<target>`.
3. Resolve both effective module sets with `go list -mod=mod -m -json all`.
   Verify the selected baseline exists, differs from the target and matches the
   requested direct/transitive strategy. Verify the effective candidate version.
4. For a transitive override, retain every baseline direct dependency version.
   Download both sides with `go mod download -json`, then verify those unchanged
   direct module archive checksums match. An unreleased wrapper can therefore
   remain fixed while its underlying dependency is overridden.
5. Prepare application and dependency production Go sources with stable logical
   ownership prefixes. Exclude `_test.go`, vendor, testdata, hidden directories
   and nested modules. Preserve `go.mod` context and use the same application
   source on both sides.
6. Build both native graphs, diff functions, and trace baseline application paths
   to changed dependency functions. Return the same graph/source-block report
   format as Java, with Go module identities and acquisition checksums.

Module records retain path/version, module sums, source-tree SHA-256 and archive
SHA-256. Candidate version selection is verified independently of the advertised
discovery input. Missing/unresolvable module sources fail the comparison.

This implementation selects one module, disables ambient `go.work`, and uses the
installed Go toolchain. `replace` directives are rejected explicitly because
comparing the requested release against replacement code would misidentify the
source. Source preparation includes production files across platform/build-tag
variants. Standard-library/toolchain updates and versioned module path migrations
such as `/v2` are separate from same-module release comparisons.

Go's [module reference](https://go.dev/ref/mod) documents module identity, minimum
version selection, downloading and proxy/checksum behavior used here.

## Local examples and expected results

Each example has its own application directory and catalog Component, prefixed
`dependency-impact-go-`. All compare `v1.0.0` with the patch `v1.0.1`.

| Example | Dependency change | Application result | Runtime baseline → candidate |
| --- | --- | --- | --- |
| Public | `Quote` body | `Run` and its caller affected | 105 → 110 |
| Internal | unexported `normalize`, public `Quote` unchanged | `Run → Quote → normalize` | 105 → 110 |
| Patch | `Validate` body, signature unchanged | validation caller affected | true → false |
| Transitive | `token-core.Encode`, `template` held at v1.0.0 | `Run → Render → Encode` | 101 → 102 |
| Injected | `RiskPolicy.Assess` implementation supplied as `Scorer` | `Run → Checkout.Submit → RiskPolicy.Assess` | 101 → 102 |
| No impact | unused `UnusedPreview` | changed function found; zero application impacts | 105 → 105 |

The source applications, controlled module proxy, catalog descriptors and
qualification scripts stay local under `demo-evaluation/go-dependency-updates/`,
excluded from Git as requested. A fresh clone has discovery metadata, but needs
its own configured module inputs and catalog registrations to run comparisons.

Where the local assets exist:

```sh
python3 demo-evaluation/go-dependency-updates/prepare.py
python3 demo-evaluation/go-dependency-updates/verify.py \
  --url http://localhost:7007/api/dependency-impact \
  --token dependency-impact-local-development
```

Preparation preserves existing immutable release archives and creates six
separate application descriptors. The verifier checks expected changed/impacted
functions, excludes test source blocks, saves complete reports and independently
executes both versions using the normal Go cache. The local Compose/config
overrides mount these assets and the proxy; the shared service's tracked default
configuration still has no preconfigured comparison applications.

Unit tests cover both coordinate formats, mixed/invalid identities, production
source selection, and native Go diff/tracing with internal and unused-function
controls. Existing Java tests continue to use the same comparison pipeline.

See the [validation record](GO-VALIDATION.md) for the observed results.
