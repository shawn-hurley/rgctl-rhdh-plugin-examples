# Working on this repository

## Scope and architecture

This repository contains two dynamic RHDH/Backstage workflows:

- Dependency Impact: `plugins/dependency-impact`,
  `plugins/dependency-impact-backend`, and the comparison API.
- Application Graph: `plugins/application-graph-backend`,
  `plugins/catalog-backend-module-application-graph`, and the
  application onboarding/graph API.

Both APIs run in the single `rgctl-service` binary in `analysis-server/`.
Reuse its shared library, authentication and worker limiter. Import rgctl as
Rust crates; do not replace this integration with CLI subprocesses, SCIP, or external indexers.
Catalog enrichment belongs in the catalog module; authorized browser-facing
requests go through the Backstage backend, with Rust service credentials kept
backend-only. Preserve source/graph provenance and existing detected impacts.

## rgctl source selection

Use `fix/module-qualified-symbols` from `git@github.com:shawn-hurley/rgctl.git`
unless the user explicitly selects another branch. The last verified revision is
`72823ed2`. Do not silently substitute upstream `main` or a different checkout.

```sh
git clone --branch fix/module-qualified-symbols --single-branch \
  git@github.com:shawn-hurley/rgctl.git ../rgctl-qualified-symbols
python3 local-dev/select-rgctl.py ../rgctl-qualified-symbols
git -C rgctl-source branch --show-current
git -C rgctl-source rev-parse HEAD
```

Reuse an existing checkout when appropriate. The selector creates an ignored
symlink used by Cargo, provenance and named Podman build contexts. Inspect an
existing checkout before altering it; do not change its branch or discard work
just to build this repository. Rebuild the affected service image after updating
rgctl or changing the selected checkout.

## Build and run

Read `README.md` and `local-dev/README.md` before starting services. From the root:

```sh
sh local-dev/build-plugins.sh
sh local-dev/build-service.sh
sh local-dev/start-service.sh
```

The plugin helper exports all four dynamic packages. Use the service build
helper: it passes the selected rgctl source as a named build context. Start
Compose with `--no-build` after building the images.

- Shared Rust service: `local-dev/service-compose.yaml`, listener 7070, host
  ports 7070 and compatibility alias 17071. Configure `service-config.json`.
- Dependency Impact host: `local-dev/compose.yaml`, RHDH 7007.
- Application Graph host: `local-dev/application-graph-compose.yaml`, RHDH 17007.

Start the shared service before either host. Both backends use the same URL/token.
Its report and graph volumes must remain separate, and only one process may write
these stores. Stop legacy Rust writers before migrating; see `analysis-server/SERVICE.md`.

Inspect the currently running project/configuration before changing containers.
An ignored Compose override may provide local import demos; keep using the same
files when reloading that stack. Each RHDH catalog uses an in-memory database:
restarting it loses manually imported registrations. Preserve/restore those
registrations when a reload is necessary. Rust graph/report volumes persist.

Never start, stop or restart the Podman VM automatically. The user manages the
VM; operate only on the requested Compose project's containers. Do not disturb
other stacks or remove their volumes. Do not add pinned toolchains, isolated
Maven/Cargo/Yarn caches, or unsolicited CI.

## Verification

Use an installed Rust compiler that meets the selected rgctl `rust-version`:

```sh
cargo test --manifest-path analysis-server/Cargo.toml
sh local-dev/test-catalog-import.sh
COREPACK_ENABLE_AUTO_PIN=0 yarn tsc --noEmit
```

The catalog-import runner includes the application-graph backend tests. Run
appropriate checks for the files changed, and use
`local-dev/verify-application-graph.py` for the tracked public Spring setup when
live verification is needed. Do not run local-only demo scripts that are absent
from a fresh clone. Keep generated checks and artifacts outside tracked source.

## Git and local assets

Demo applications, qualification fixtures, catalog walkthrough assets, recordings,
`rgctl-source`, dependency installations and build output are intentionally
ignored. Keep them out of commits unless the user explicitly changes that scope.
Use self-contained commits for features that may become separate pull requests.

Commit messages, PR descriptions and PR comments must use repository-relative
paths or GitHub URLs. Do not include local filesystem paths, local file links,
or references to local worktrees or artifacts in those messages.
