# Local development

Use a running Podman environment, Podman Compose, Node.js 22 or later, and Yarn
4 available on your PATH. On macOS, start the Podman machine from your own
terminal before running these commands. These scripts manage containers only.

From the repository root:

```sh
python3 local-dev/select-rgctl.py /path/to/rgctl-checkout
python3 local-dev/prepare-demos.py
sh local-dev/build-plugins.sh
sh local-dev/build-analysis.sh
podman compose -f local-dev/compose.yaml up -d --no-build
```

This also requires Python 3, Maven, JDK 17 or later, and the sibling `../rgctl`
checkout. The demo preparation installs controlled releases into your normal
Maven repository. Plugin builds export both dynamic packages; the analysis build
uses Podman directly and a compiler compatible with rgctl. Compose starts the
Rust service, installs the dynamic packages, and starts RHDH. Initial image
downloads and compilation can take several minutes.

Open [localhost:7007](http://localhost:7007) and choose **Enter** under **Guest**.
Open **Dependency Impact** in the sidebar to choose an application/scenario and
run a comparison. The form loads coordinates and version defaults from discovery.
The catalog contains eight Components:

| Component | Scenario |
| --- | --- |
| dependency-impact-checkout | Internal change behind a public function |
| dependency-impact-delivery | Public function change |
| dependency-impact-notification | Transitive dependency override |
| dependency-impact-file-import | Released patch upgrade |
| dependency-impact-no-impact | Unused library change; no application paths identified |
| dependency-impact-injected-policy | Constructor-injected interface reaches the changed policy |
| dependency-impact-caller-separation | Submission affected; preview excluded through a shared gateway |
| dependency-impact-stable-preview | Static field and lambda select an unchanged implementation |

Choose **All** in the catalog if the default ownership filter hides the demos.
Each demo has a dedicated **Dependency Impact** tab with the same comparison
workflow scoped to that application. The
catalog declarations are read directly from `demo-evaluation/`; no demo assets
are copied into plugin source directories.

Select **Run comparison** to see actual analysis stages, then inspect affected
application functions. Select a testing area to see a focused call graph. Click or keyboard-select a
node to open its full source block. Upstream callers provide context rather than
additional testing tasks. Unrelated dependency changes remain in the JSON report. **Download report** saves all
results and diagnostics. The `?comparison=...` URL reopens a persisted report;
source links include a fragment identifying its exact analyzed block.

Partial coverage and overload ambiguity remain visible. No identified impact is
not proof of upgrade safety. The Rust service and authenticated Backstage API are
documented in the
[service guide](../analysis-server/README.md) for requests and report semantics.
The backend also exposes [mock input discovery](../plugins/dependency-impact-backend/README.md)
at `/api/dependency-impact/inputs`, supplying the five demo coordinates and version
choices for the comparison form.

## Exported packages

After building the plugins, create distributable tarballs and integrity hashes:

```sh
sh local-dev/package-plugins.sh
```

The independently exported frontend/backend packages and `manifest.json` are in
`local-dev/.generated/packages/`. The manifest records each filename, version,
and SHA-512 integrity value. For another compatible RHDH host, install both
packages and use the route/tab/mount configuration from `dynamic-plugins.yaml`.
Set the backend-only analysis URL and token as in `app-config.yaml`.

To disable the plugin locally, set `disabled: true` for both entries in
`dynamic-plugins.yaml`, then run the reload command below. Restore `false` and
reload to enable it again. The analysis container runs separately.

## Rebuild and reload

After editing plugin source:

```sh
sh local-dev/build-plugins.sh
COREPACK_ENABLE_AUTO_PIN=0 yarn local:reload
```

Reload the browser after the host restarts. `local:reload` reinstalls the local
exports before restarting RHDH. The development plugin configuration always
refreshes local packages, including when their version number is unchanged.
Re-run `up -d --no-build` after changing Compose settings. For Rust source changes,
run `sh local-dev/build-analysis.sh` and then
`podman compose -f local-dev/compose.yaml up -d --no-build analysis`.

To stop the containers:

```sh
podman compose -f local-dev/compose.yaml down
```

The catalog database is in memory and reimports the five demos on startup. Named
volumes hold installed plugins and analysis jobs/reports and survive `down`.
Compose mounts your normal `~/.m2` settings/cache into the analysis container.

## Configuration and troubleshooting

`compose.yaml` uses RHDH 1.10.6. The exporter uses the compatible 1.10.x CLI and
Scalprum frontend format. `RHDH_IMAGE` can override the image, but keep the host
and exporter compatible. Toolchains are not pinned by this repository.

`app-config.yaml` supplies local guest authentication, an in-memory database,
and catalog locations. The host binds to localhost only. Its static development
token can be used to inspect backend/analysis availability:

```sh
curl -H 'Authorization: Bearer dependency-impact-local-development' \
  http://localhost:7007/api/dependency-impact/status
```

Expected response while both services are available:
`{"status":"ready","analysisAvailable":true}`.

For startup errors, read the appropriate service logs:

```sh
podman compose -f local-dev/compose.yaml logs --tail=60 install-plugins
podman compose -f local-dev/compose.yaml logs --tail=60 rhdh
podman compose -f local-dev/compose.yaml logs --tail=60 analysis
```

If exports are missing, run the build command before starting Compose. If
Podman cannot connect, check that the machine is running from your terminal.
If port 7007 is occupied, stop the conflicting service before starting this host.
Port 7070 is used by the analysis service. Use `localhost` when opening RHDH;
`127.0.0.1` is a different browser origin from its configured base URL. Chrome is
the verified browser for this setup; Safari's blank page remains unresolved.

The local `rgctl-source` symlink selects a single checkout for all imported crates,
build provenance, and the Podman named build context. It is ignored by version
control. The current checkout is the persistent `fix/module-qualified-symbols`
worktree at `/Users/shurley/repos/rgctl-app/rgctl-module-qualified-symbols`.
Rebuild the analysis image after changing the selection. No changes to the
selected checkout are made by the selection script.

If the default Compose provider fails because `docker-credential-desktop` is
missing, the installed Podman Compose provider can be selected for a command:

```sh
PODMAN_COMPOSE_PROVIDER=podman-compose podman compose -f local-dev/compose.yaml up -d --no-build
```

If Chrome fails to register `PluginRoot` and hides the plugin navigation, try
`http://localhost:7007/dependency-impact` in Incognito after Guest login. A fresh
profile was verified after the restart. The regular profile's failure has not
been attributed conclusively to cached state or extensions.

## Demo walkthrough

See [the demo runbook](DEMO-RUNBOOK.md) for the eight scenarios, expected behavior,
independent runtime checks, and authenticated HTTP verification. Remaining analysis
[bugs](../demo-evaluation/KNOWN-BUGS.md) are tracked separately from demo acceptance.
The analysis image build context is limited to `analysis-server/` plus the selected
rgctl source context; generated targets and unrelated repositories are excluded.
