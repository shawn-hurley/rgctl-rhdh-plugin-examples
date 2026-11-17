# Phase 2 — Local test environment

**Status:** Complete — local environment verified on November 9, 2026.

**Start:** Phase 1 is accepted and the four demo fixtures are available.

**End:** A local Podman Compose environment starts, exposes four separate demo
catalog entries, and loads minimal frontend/backend dynamic plugins. A short
setup guide explains how to use it.

## Local setup

- Use Podman Compose for the local Backstage/RHDH-compatible dynamic plugin host.
  No cluster is required.
- Register all four demos as separate catalog Components, reusing the existing
  `demo-evaluation/` assets without copying them into plugin implementation directories.
- Create minimal frontend/backend dynamic plugin skeletons and the configuration
  needed to export and load them without rebuilding the host. Placeholder content
  is sufficient; the impact UI belongs to Phase 4.
- Use compatible host and plugin dependencies without adding a toolchain-pinning
  project or isolated cache infrastructure. Normal local caches are sufficient.

## Documentation

Provide one brief setup guide covering prerequisites, startup/shutdown commands,
the local URL, catalog registration, plugin rebuild/reload, and a few common
troubleshooting steps. Include any locally required configuration examples.

## Phase boundary

No automated checks, CI workflows, cache isolation, or toolchain pinning are part
of Phase 2. Confirm the environment manually by opening the host, viewing the four
catalog entries, and seeing the placeholder plugin load.

The Rust analysis server, rgctl crate integration, service container, and custom
JSON HTTP contract belong to Phase 3. Phase 2 does not need an analysis service or
an additional crate-access/source-identity investigation. The accepted UI scope
is full enclosing function/block context.

## Execution result

- [Local setup guide](../local-dev/README.md), Compose configuration, and host
  configuration are available under `local-dev/`.
- Frontend and backend plugin skeletons are under `plugins/`. Both export and
  install dynamically into RHDH 1.10.6 without rebuilding the host image.
- The running host exposes all four demo catalog Components from the existing
  evaluation directory.
- Manual HTTP verification returned `{"status":"ready","analysisAvailable":false}`
  from the authenticated backend and the four expected Component names from
  the catalog API.
- Browser verification with guest authentication confirmed the global plugin
  page and each demo's Dependencies card display **Plugin backend connected.**
  The documented reinstall/restart command also passed, followed by another
  browser verification of the page and all four cards.

No toolchain pinning, isolated caches, automated check suite, or CI was added.
The placeholder reports connectivity only; dependency comparisons remain Phase 3.
