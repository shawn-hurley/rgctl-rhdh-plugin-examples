# Running the RHDH plugins locally

Both workflows are on `main`. The supplied Compose files run separate RHDH hosts
and Rust services, with distinct project names/volumes so they can coexist.

| Workflow | Compose file | RHDH | Rust | Dynamic plugin config |
| --- | --- | --- | --- | --- |
| Dependency Impact | `local-dev/compose.yaml` | `http://localhost:7007` | `http://localhost:7070` | `local-dev/dynamic-plugins.yaml` |
| Application Graph | `local-dev/application-graph-compose.yaml` | `http://localhost:17007` | `http://localhost:17071` | `local-dev/application-graph-plugins.yaml` |

Run commands from the repository root. Use Chrome and choose **Enter** under
**Guest** when opening either host.

## Prerequisites and rgctl

Install Git, Python 3, Node.js 22+, Yarn 4 and Podman Compose. On macOS, start the
Podman machine from your own terminal first; these instructions manage containers
only. Native service builds also need Rust satisfying rgctl's `rust-version`
(currently 1.99 or newer). Native Maven acquisition/comparison and local demo
preparation need Maven and JDK 17+. Container images include their runtime tools.
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

## Application Graph

The tracked setup uses public Spring repositories and needs no ignored fixtures:

```sh
sh local-dev/build-application-graph.sh
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
`application-graph-config.json` to permit other repository hosts. Local `file:`
source repositories are disabled unless explicitly allowed under
`allowedSourceRoots`. The Rust store is persisted in the `application-graphs`
volume and includes immutable graph versions/checksums.

## Dependency Impact inputs

The default stack points at eight local Java/Maven demos. Their applications,
qualification fixtures, preparation scripts and catalog walkthrough files are
intentionally ignored. They are not supplied by a fresh Git clone.

In an existing workspace containing `local-dev/prepare-demos.py` and
`demo-evaluation/catalog-info.yaml`:

```sh
python3 local-dev/prepare-demos.py
sh local-dev/build-analysis.sh
podman compose -f local-dev/compose.yaml up -d --no-build
```

Preparation installs demo binaries/source JARs into your normal Maven repository.
Open [Dependency Impact](http://localhost:7007/dependency-impact). Discovery
populates the application/dependency/version selector. Run a comparison, choose
an affected testing area and inspect its call graph and full source block.
**Download report** saves the complete comparison. Components annotated with
`rgctl.io/evaluation-scenario` also expose the catalog tab.

For your own inputs, configure these pieces together before starting the stack:

1. Mount a standalone Maven module into the analysis container and add its
   canonical Component reference to `analysis-config.json` under `applications`,
   with a container-visible `path` and matching `revision` label.
2. Register that Component through the catalog locations configured in
   `app-config.yaml`. Replace the default demo catalog mount/location when the
   local assets are absent.
3. Supply UI discovery choices through
   [the discovery provider](../plugins/dependency-impact-backend/src/inputDiscovery.ts)
   and its [typed input manifest](../plugins/dependency-impact-backend/src/demoInputs.ts).
   Current discovery is a controlled manifest, rather than an Artifactory lookup.
4. Make binary/source artifacts available through the normal Maven repository
   or Maven settings. The service resolves the effective versions independently.

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

For Rust-only changes, rebuild/recreate only the relevant service:

```sh
sh local-dev/build-analysis.sh
podman compose -f local-dev/compose.yaml up -d --no-build --force-recreate analysis
sh local-dev/build-application-graph.sh
podman compose -f local-dev/application-graph-compose.yaml up -d --no-build --force-recreate application-graph
```

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
`rhdh` and `analysis`/`application-graph` logs with `logs --tail=60`.

- Missing exports: run the plugin build before Compose.
- Unavailable Podman socket: check the VM from your own terminal. These helpers
  must not start/stop it automatically.
- Port conflicts: use the table above and avoid changing the other running stack.
- Blank page or `PluginRoot` load error: use Chrome with a fresh profile/Incognito
  and the configured `localhost` URL. Safari is not the verified browser.
- A Compose provider requiring unavailable Docker credentials can be replaced
  per command with `PODMAN_COMPOSE_PROVIDER=podman-compose` when installed.
