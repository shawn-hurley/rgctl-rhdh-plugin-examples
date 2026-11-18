# Running the RHDH plugins locally

Both workflows share one Rust service and run in separate RHDH hosts. Start
the shared service once; either or both hosts can then connect to it.

| Workflow | Compose file | RHDH | Rust | Dynamic plugin config |
| --- | --- | --- | --- | --- |
| Dependency Impact | `local-dev/compose.yaml` | `http://localhost:7007` | `http://localhost:7070` | `local-dev/dynamic-plugins.yaml` |
| Application Graph | `local-dev/application-graph-compose.yaml` | `http://localhost:17007` | `http://localhost:7070` (17071 alias) | `local-dev/application-graph-plugins.yaml` |

Run commands from the repository root. Use Chrome and choose **Enter** under
**Guest** when opening either host.

## Prerequisites and rgctl

Install Git, Python 3, Node.js 22+, Yarn 4 and Podman Compose. On macOS, start the
Podman machine from your own terminal first; these instructions manage containers
only. Native service builds also need Rust satisfying rgctl's `rust-version`
(currently 1.99 or newer). Native Maven acquisition/comparison and local demo
preparation need Maven and JDK 17+; native Go comparisons need Go installed.
Container images include their runtime tools.
Use normal dependency caches; there is no repository toolchain pin.

Select the combined rgctl branch published to the fork:

```sh
git clone --branch fix/module-qualified-symbols --single-branch \
  git@github.com:shawn-hurley/rgctl.git ../rgctl-qualified-symbols
python3 local-dev/select-rgctl.py ../rgctl-qualified-symbols
git -C rgctl-source branch --show-current
git -C rgctl-source rev-parse HEAD
```

Skip cloning if you already have that branch checked out and pass its path to
the selector. The last verified revision is `72823ed2`. The ignored
`rgctl-source` symlink selects the same source for Cargo dependencies, provenance
and Podman's named build context. The selector does not alter the selected
checkout. Do not substitute upstream `main` for this branch.

Build all dynamic exports once:

```sh
sh local-dev/build-plugins.sh
```

This runs Yarn installation, TypeScript declaration generation and RHDH dynamic
export for the frontend, both backend plugins and the catalog module. Image
builds below use the selected rgctl checkout through their helpers. Use
`up --no-build`; implicit Compose builds do not supply that named source context.
Initial downloads/compilation can take several minutes.

## Shared Rust service

```sh
sh local-dev/build-service.sh
sh local-dev/start-service.sh
```

`service-compose.yaml` runs one `rgctl-service` container and creates the shared
`rgctl-services` network. Both host configurations use `http://rgctl-service:7070`
and the same backend-only token. Both host ports 7070 and 17071 reach this same
listener. `build-analysis.sh` and `build-application-graph.sh` remain compatibility
aliases for the single image build.

Configure `service-config.json`: `comparisons` contains application inputs and
its report store; `applicationGraphs` contains repository permissions and its graph
store. `maxConcurrentJobs` defaults to 1 across both queues. The two persistent
stores reuse the existing named volumes. The startup helper creates missing
volumes and refuses to run alongside a legacy writer. See
[configuration and migration](../analysis-server/SERVICE.md).

To supply local sources or settings, use an ignored Compose override:

```sh
sh local-dev/start-service.sh -f local-dev/.generated/service.override.yaml
```

## Application Graph

The tracked setup uses public Spring repositories and needs no ignored fixtures:

```sh
podman compose -f local-dev/application-graph-compose.yaml up -d --no-build
python3 local-dev/verify-application-graph.py
```

Open [the catalog](http://localhost:17007/catalog). The base catalog registers
`spring-quoters`, `spring-quotes-client` and `quotes-api` with declared API fields.
The verification command explicitly onboards their Java sources and checks
native nodes/edges and the source-linked HTTP connection. This base example is
separate from the local-only import recording's descriptors.

To index your application during ordinary catalog import, publish a Component
descriptor containing:

```yaml
metadata:
  annotations:
    backstage.io/source-location: url:https://github.com/example/orders
    application-graph.io/enabled: 'true'
    application-graph.io/maven-sources: 'true'
```

The Maven annotation is optional; enable it to acquire dependency source JARs
and trace library chains. Select one Maven module per enabled Component. Import
its descriptor through [Catalog Import](http://localhost:17007/catalog-import).
Indexing runs asynchronously; subsequent catalog processing adds standard
`dependsOn`, `providesApis` and `consumesApis` fields. Backstage produces inverse
relations, visible in its Dependencies/API tabs. Generated API definitions include
source-derived request/response schemas. Repository descriptors are not rewritten.

Component dependencies match indexed Maven coordinates. Resource connections
need registered `application-graph.io/resource-uri` bindings. Provider matching
uses `application-graph.io/api-base-url`; relative client requests need
`application-graph.io/http-client-base-url`. `subcomponentOf` is declared metadata.
See the [service guide](../analysis-server/APPLICATION-GRAPH.md) and
[catalog module guide](../plugins/catalog-backend-module-application-graph/README.md)
for examples, supported schema inference, authorization and configuration.

The service allows `github.com` by default; edit
`service-config.json` under `applicationGraphs` to permit other repository hosts.
Local `file:` source repositories are disabled unless explicitly allowed under
`allowedSourceRoots`. The Rust store is persisted in the shared service
`application-graph-local_application-graphs` volume and includes immutable graph
versions/checksums.

## Dependency Impact inputs

The UI discovery manifest lists eight Java/Maven and six Go module demos; the shared service
starts with no configured comparison applications. The demo applications,
qualification fixtures, preparation scripts and catalog walkthrough files are
intentionally ignored. They are not supplied by a fresh Git clone.

In an existing workspace containing `local-dev/prepare-demos.py` and
`demo-evaluation/catalog-info.yaml`:

```sh
python3 local-dev/prepare-demos.py
podman compose -f local-dev/compose.yaml up -d --no-build
```

Preparation installs demo binaries/source JARs into your normal Maven repository.
Open [Dependency Impact](http://localhost:7007/dependency-impact). Discovery
populates the application/dependency/version selector. Run a comparison, choose
an affected testing area and inspect its call graph and full source block.
**Download report** saves the complete comparison. Components annotated with
`rgctl.io/evaluation-scenario` also expose the catalog tab.

Go requests use `dependency: {modulePath: "example.org/library"}`; Maven
requests retain `groupId`/`artifactId`. Go module acquisition uses normal
`GOPROXY`/`GOPRIVATE`/checksum settings; `comparisons.goProxy` can select a
configured module proxy. See the [Go guide](../analysis-server/GO.md).

For your own inputs, configure these pieces together before starting the stack:

1. Mount a standalone Maven or Go module into the shared Rust container and add its
   canonical Component reference to `service-config.json` under
   `comparisons.applications`,
   with a container-visible `path` and matching `revision` label.
2. Register that Component through the catalog locations configured in
   `app-config.yaml`. Replace the default demo catalog mount/location when the
   local assets are absent.

3. Supply UI discovery choices through
   [the discovery provider](../plugins/dependency-impact-backend/src/inputDiscovery.ts)
   and its [typed input manifest](../plugins/dependency-impact-backend/src/demoInputs.ts).
   Current discovery is a controlled manifest, rather than an Artifactory lookup.
4. Make artifacts available through normal Maven settings/repositories or the
   configured Go module proxy. The service resolves effective versions independently.

Use an ignored Compose/config override for local applications and mounts; keep
application sources and demo fixtures outside tracked plugin directories.
The [comparison service guide](../analysis-server/README.md) documents request
fields, transitive override behavior and native service configuration.

## Reload, verify and stop

After plugin changes, rebuild exports and reinstall them in the affected host:

```sh
sh local-dev/build-plugins.sh
podman compose -f local-dev/compose.yaml run --rm install-plugins
podman compose -f local-dev/compose.yaml restart rhdh
# Or, for Application Graph:
podman compose -f local-dev/application-graph-compose.yaml run --rm install-plugins
podman compose -f local-dev/application-graph-compose.yaml restart rhdh
```

Both catalogs use an in-memory database. Preserve manually imported location
registrations before restarting RHDH and register them again afterward. Named
volumes retain Rust graphs/reports across container restarts and `down`.
If you started a stack with an ignored override, use the same `-f` files for all
subsequent commands; using only the base file can replace local catalog mounts.

For Rust-only changes, rebuild/recreate the shared service once:

```sh
sh local-dev/build-service.sh
podman compose -f local-dev/service-compose.yaml up -d --no-build --force-recreate rgctl-service
```

Include the same configuration override files used at startup. This reload affects
both workflows; wait for their active jobs to finish first. It does not restart RHDH.

Checks for code changes:

```sh
cargo test --manifest-path analysis-server/Cargo.toml
sh local-dev/test-catalog-import.sh
COREPACK_ENABLE_AUTO_PIN=0 yarn tsc --noEmit
```

The catalog-import runner includes the application-graph backend tests. The
tracked public Spring setup additionally supports:

```sh
python3 local-dev/verify-application-graph.py
python3 local-dev/verify-application-graph.py --verify-existing
```

Stop only the requested stack; do not add `-v` unless deleting its data is intended:

```sh
podman compose -f local-dev/compose.yaml down
podman compose -f local-dev/application-graph-compose.yaml down
# Stop the shared service only when both workflows can be offline:
podman compose -f local-dev/service-compose.yaml down
```

## Package for another RHDH host

```sh
sh local-dev/package-plugins.sh
```

The output in `local-dev/.generated/packages/` contains four tarballs and a
`manifest.json` with SHA-512 integrity values. Dependency Impact requires its
frontend and backend; Application Graph requires its backend and catalog module.
Preserve the frontend routes/tabs/mount points from `dynamic-plugins.yaml` and
include both entries from `application-graph-plugins.yaml` as appropriate.

To install both workflows in a single compatible RHDH host, install all four
packages, retain that frontend configuration, allow Component/API/Resource catalog
kinds, and configure backend-only `dependencyImpact.service` and
`applicationGraph.service` URLs/tokens. The provided local stacks keep their
hosts separate. The export CLI and default RHDH image use the compatible 1.10.x
line (`RHDH_IMAGE` overrides the host image).

## Troubleshooting

Status endpoints use the local development credentials in each host's config:

```sh
curl -H 'Authorization: Bearer dependency-impact-local-development' \
  http://localhost:7007/api/dependency-impact/status
curl -H 'Authorization: Bearer application-graph-local-development' \
  http://localhost:17007/api/application-graph/status
```

Expected availability flags are `analysisAvailable: true` and
`graphServiceAvailable: true`. Inspect the affected project's `install-plugins`,
`rhdh` logs and the shared project
`rgctl-service` logs with `logs --tail=60`.

- Missing exports: run the plugin build before Compose.
- Unavailable Podman socket: check the VM from your own terminal. These helpers
  must not start/stop it automatically.
- Port conflicts: use the table above and avoid changing the other running stack.
- Blank page or `PluginRoot` load error: use Chrome with a fresh profile/Incognito
  and the configured `localhost` URL. Safari is not the verified browser.
- A Compose provider requiring unavailable Docker credentials can be replaced
  per command with `PODMAN_COMPOSE_PROVIDER=podman-compose` when installed.
