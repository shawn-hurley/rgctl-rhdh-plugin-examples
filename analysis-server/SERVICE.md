# Shared rgctl service

`rgctl-service` runs Dependency Impact and Application Graph in one Rust process.
Both Backstage backends use its single HTTP listener and backend-only bearer
token. The graph pipeline, Java/Go extraction, dependency acquisition and dispatch
analysis are shared library code importing the selected rgctl crates.

## Build and start

Select `fix/module-qualified-symbols` as described in the root README, then:

```sh
sh local-dev/build-service.sh
sh local-dev/start-service.sh
podman compose -f local-dev/compose.yaml up -d --no-build
podman compose -f local-dev/application-graph-compose.yaml up -d --no-build
```

Build dynamic exports first with `sh local-dev/build-plugins.sh`. Either RHDH host
can run independently. Both join the external `rgctl-services` network and use
`http://rgctl-service:7070`. Host ports 7070 and 17071 reach the same container
listener; 17071 preserves existing direct graph clients. The old build helper
names are compatibility aliases for the same image build.

For native development:

```sh
cargo build --manifest-path analysis-server/Cargo.toml --bin rgctl-service
RGCTL_SERVICE_CONFIG=local-dev/.generated/service-config.json \
RGCTL_SERVICE_TOKEN=replace-with-a-development-token \
analysis-server/target/debug/rgctl-service
```

Use native source/data paths in that config, with Git and the tools for the
selected ecosystem: Maven/JDK 17+ for Java, Go for module acquisition.
`RGCTL_SERVICE_CONFIG` defaults to `service-config.json` in the working directory;
`RGCTL_SERVICE_LISTEN` defaults to `127.0.0.1:7070`. The container listens on
`0.0.0.0:7070` and publishes loopback host ports only.

## Configuration and execution

`local-dev/service-config.json` contains:

```json
{
  "comparisons": {
    "applications": {},
    "dataDir": "/data/comparisons",
    "queueCapacity": 8
  },
  "applicationGraphs": {
    "dataDir": "/data/application-graphs",
    "allowedRepositoryHosts": ["github.com"],
    "allowedSourceRoots": [],
    "queueCapacity": 8
  },
  "maxConcurrentJobs": 1
}
```

Configure comparison inputs under `comparisons.applications`, using the same
entry shape as the previous analysis config. For Go requests, select a
`go.mod` application and pass `dependency.modulePath`; optional
`comparisons.goProxy` selects a module proxy. See [Go comparisons](GO.md). Configure repository permissions
under `applicationGraphs`. The default empty comparison list lets onboarding
start without local demo fixtures. Source mounts/config overrides stay local.

Each workflow retains its bounded queue and serial worker. A shared semaphore
limits heavy work across both workflows: the default permits one running job
total. Queued jobs remain queued until they obtain a permit. Raising
`maxConcurrentJobs` allows both workers to run together; it does not add workers
to either queue. Values from 1 through 16 are accepted. Request handlers and
saved-report/graph reads remain available while jobs run.

Both data directories must be distinct and non-nested. Startup rejects shared
or aliased roots so recovery cannot confuse the two job formats. Comparisons
retain their report/source layout; onboarding retains immutable graph versions
and manifests. No report or snapshot format migration is needed. Each workflow
marks its own interrupted jobs failed on restart, retaining successful results.
Wait for active jobs before recreating the container.

## HTTP contract

All `/v1` operations require the same `RGCTL_SERVICE_TOKEN` (at least 16
characters). `GET /health` is public and reports readiness for both features:

```json
{"status":"ready","features":["dependency-impact","application-graph"]}
```

Existing API paths are unchanged:

| Workflow | Routes | Contract |
| --- | --- | --- |
| Comparisons | `/v1/comparisons`, job/report operations | [OpenAPI](openapi.yaml), [comparison guide](README.md) |
| Onboarding/graphs | `/v1/onboard`, `/v1/jobs`, `/v1/applications`, `/v1/connections` | [Graph guide](APPLICATION-GRAPH.md) |

Backstage still authenticates callers and authorizes catalog access separately
for each plugin. Its development tokens differ from the shared Rust service
token. Service credentials remain in backend configuration, never browser code.

The legacy `dependency-impact-server` and `application-graph-server` entry points
remain for standalone compatibility. Supplied containers run `rgctl-service`.
Never run multiple writers against either persistent store.

## Migrate the previous two-container setup

1. Preserve manually registered catalog locations before recreating either RHDH
   host: the supplied hosts use in-memory catalogs. Keep existing ignored Compose
   overrides for catalog/demo mounts.
2. Copy the old comparison settings under `comparisons`, and onboarding settings
   under `applicationGraphs`, in a combined local config. Set their container
   data roots to `/data/comparisons` and `/data/application-graphs`. Carry over
   source mounts using an ignored override for `service-compose.yaml`.
3. Wait for both old queues to finish, then stop only the old `analysis` and
   `application-graph` Rust containers. Leave their volumes intact.
4. Build/start the shared service. The startup helper refuses existing legacy
   writers and reuses `dependency-impact-dev_analysis-data` and
   `application-graph-local_application-graphs`. It creates missing volumes for
   fresh installations. It never manages the Podman VM.
5. Recreate the RHDH hosts with the updated configs/network, retaining all their
   original override files, then restore catalog locations. Both backend configs
   must use the same shared URL and token.
6. Check both backend status APIs, a saved comparison report and immutable graph
   versions before removing stopped legacy containers. No volume copying or
   deletion is required.

Use `sh local-dev/start-service.sh -f <override-file>` for a shared-service
override. For subsequent recreations, include the same `-f` files. Stopping
either RHDH host leaves the Rust service available to the other host; stopping
the shared service makes both workflows temporarily unavailable.

## Verification

```sh
cargo test --manifest-path analysis-server/Cargo.toml
sh local-dev/test-catalog-import.sh
```

The shared-router tests exercise both APIs and bearer authentication, native Git
onboarding, store isolation/recovery, configuration rejection and cross-workflow
queue contention using the actual shared semaphore. Existing analysis and catalog
tests continue to exercise feature behavior.
