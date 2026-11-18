# rgctl RHDH plugin examples

Two workflows implemented as dynamic Backstage/Red Hat Developer Hub plugins,
backed by Rust services that import rgctl as crates.

| Workflow | What it does | Local UI |
| --- | --- | --- |
| Dependency Impact | Compare Maven dependency versions and identify application code to test, including transitive changes and injected implementations | [Dependency Impact](http://localhost:7007/dependency-impact) |
| Application Graph | Import existing Java applications, persist their graphs, and enrich catalog dependencies, APIs, schemas and inverse relationships | [Catalog import](http://localhost:17007/catalog-import) |

Both workflows are on `main`. Their supplied Compose stacks use separate RHDH
hosts, ports and volumes, so they can run alongside each other.

## Select rgctl and build

Use the combined fixes from
[shawn-hurley/rgctl, `fix/module-qualified-symbols`](https://github.com/shawn-hurley/rgctl/tree/fix/module-qualified-symbols).
The last verified revision is `72823ed2`; the branch is the source of updates.

Clone the examples, then select the rgctl source:

```sh
git clone git@github.com:shawn-hurley/rgctl-rhdh-plugin-examples.git
cd rgctl-rhdh-plugin-examples
git clone --branch fix/module-qualified-symbols --single-branch \
  git@github.com:shawn-hurley/rgctl.git ../rgctl-qualified-symbols
python3 local-dev/select-rgctl.py ../rgctl-qualified-symbols
git -C rgctl-source branch --show-current
git -C rgctl-source rev-parse HEAD
sh local-dev/build-plugins.sh
```

For an existing checkout, pass its path to `select-rgctl.py` instead of cloning.
The ignored `rgctl-source` symlink supplies one source tree for Cargo, build
provenance and Podman builds. No rgctl executable or SCIP indexer is required.

Prerequisites are Git, Python 3, Node.js 22+, Yarn 4, and a running Podman/Compose
environment. Native Rust builds need a compiler satisfying the selected rgctl
checkout's `rust-version`; Maven source acquisition/comparisons need Maven and
JDK 17+ when run outside the provided containers. This repository does not pin
toolchains or use separate dependency caches. Start the Podman VM yourself on
macOS before running the container commands.

## Run Application Graph

This stack can run from a fresh clone using the tracked public Spring catalog:

```sh
sh local-dev/build-application-graph.sh
podman compose -f local-dev/application-graph-compose.yaml up -d --no-build
python3 local-dev/verify-application-graph.py
```

Open [RHDH on port 17007](http://localhost:17007), enter as Guest, and use the
catalog's API and Dependencies tabs. The verification command onboards the public
Spring provider/client and checks their stored graphs and API connection.
For import-triggered onboarding, opt your Component descriptor in with
`application-graph.io/enabled: 'true'`; see the
[onboarding guide](analysis-server/APPLICATION-GRAPH.md) and
[catalog module guide](plugins/catalog-backend-module-application-graph/README.md).

## Run Dependency Impact

The configured comparison inputs use local demo applications that are deliberately
excluded from Git. In a workspace that already has those assets:

```sh
python3 local-dev/prepare-demos.py
sh local-dev/build-analysis.sh
podman compose -f local-dev/compose.yaml up -d --no-build
```

Open [RHDH on port 7007](http://localhost:7007), enter as Guest, and select
**Dependency Impact**. A fresh clone needs application inputs configured before
starting this stack; see [local setup](local-dev/README.md#dependency-impact-inputs)
for the catalog, service and discovery-provider configuration involved.

## Development

- [Run, configure, reload and troubleshoot both workflows](local-dev/README.md)
- [Agent instructions](AGENTS.md)
- [Comparison service](analysis-server/README.md)
- [Application graph service](analysis-server/APPLICATION-GRAPH.md)
- [Phased plans](plans/README.md)

Demo applications, qualification fixtures, walkthrough scripts and recordings stay
local and ignored. Building the tracked source exports all four dynamic packages;
`sh local-dev/package-plugins.sh` packages them for another compatible RHDH host.
