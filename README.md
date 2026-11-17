# Dependency Impact for RHDH

A dynamically loaded Backstage/RHDH plugin for exploring how dependency changes
affect application source code.

- [Local development setup](local-dev/README.md)
- [Phased implementation plan](plans/README.md)
- [Application onboarding and persisted API-connected graphs](analysis-server/APPLICATION-GRAPH.md)
- [Demo evaluation and evidence](demo-evaluation/README.md)

The dynamic plugin provides a testing-focus form and clickable code-path graph,
connected to a Rust service importing rgctl crates.
The [comparison API](analysis-server/README.md) resolves Maven dependencies,
acquires source JARs, and returns impact paths and full source blocks. The full
workflow is available from the global Dependency Impact page and each demo's
Dependency Impact catalog tab.
