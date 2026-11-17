# Dependency impact backend

This dynamically loaded plugin authenticates Backstage requests, checks catalog
entity access, and forwards comparisons to the Rust analysis service.

## Demo input discovery

`GET /api/dependency-impact/inputs` returns the five configured demo choices
visible to the authenticated caller. Filter for an entity with
`?entityRef=component%3Adefault%2Fdependency-impact-checkout`.

```sh
curl -H 'Authorization: Bearer dependency-impact-local-development' \
  http://localhost:7007/api/dependency-impact/inputs
```

The response contains `schemaVersion: "1"`, `provider: "demo-manifest"`, and an
`inputs` array. Each choice contains an ID, label, scenario description, package
manager, catalog entity reference, configured application revision, Maven
coordinates, baseline version, target version, and comparison strategy.
See the [verified response example](examples/inputs-response.json).

For example, the notification choice selects `io.rgctl.demo:token-core`, baseline
`1.0.0`, target `1.0.1`, and `transitive-override`. Its unchanged direct dependency
is template-api 1.0.0. The patch choice selects `commons-io:commons-io`, 2.15.0 →
2.15.1.

The typed static manifest is [demoInputs.ts](src/demoInputs.ts).
[InputDiscoveryService](src/inputDiscovery.ts) separates the provider from the
HTTP route. Future package-manager-specific providers can use Artifactory or
upstream registries behind this interface. The demo provider performs no registry
lookups or analysis and needs no additional container.

The advertised baseline is a demo default. The Rust service independently
resolves the effective baseline and acquires the actual Maven source JARs.
To submit a selected choice to `POST /api/dependency-impact/comparisons`, send
only `entityRef`, `applicationRevision`, `dependency`, `targetVersion`, and
`strategy`; display metadata and `baselineVersion` are not submission fields.

Discovery is protected by normal Backstage authentication. Global discovery
omits entities the caller cannot read; catalog failures return an error rather
than an empty list. A filtered request returns `400` for an invalid entity
reference, `403` for denied access, or `404` for a missing entity. An accessible
entity with no demo choice returns an empty `inputs` array.

The frontend uses discovery to populate its comparison form and reads full
source context from authenticated reports. `AnalysisClient` is the typed
backend-only Rust client; connection failures return `503` with actionable
guidance. Inline source blocks share the report's catalog authorization check.
See the [local setup](../../local-dev/README.md) for build and reload commands.

Verified November 10, 2026 after dynamic export and reload: all four choices match
the Phase 3 request examples and Phase 1 baseline versions; entity filters each
return the corresponding choice. Unauthenticated discovery returns `401`, invalid
or repeated entity filters return `400`, and a missing entity returns `404`.
A notification choice from discovery successfully submitted to the real analysis
service and returned the expected message → render → clean → escape path.
