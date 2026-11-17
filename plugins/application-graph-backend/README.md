# Application graph backend

A dynamically loadable Backstage backend plugin for onboarding existing Java
applications into the separate Rust graph service. Its plugin ID and API base
are `application-graph` and `/api/application-graph`.

The plugin reads authorized catalog entities and their standard source/API
fields. It submits immutable graph builds, scopes graph reads to visible
Components, and connects applications using live processed catalog API relations.
`GET /relationships` also returns component/resource `dependsOn`/`dependencyOf`
and component `partOf`/`hasPart`, with source paths where the stored Java/Maven
graph provides evidence. Hierarchy membership stays separate from calls.

The companion [catalog module](../catalog-backend-module-application-graph/README.md)
starts graph onboarding during ordinary catalog imports and enriches standard
dependency/API fields from the results.

```yaml
applicationGraph:
  service:
    baseUrl: http://application-graph:7071
    token: ${APPLICATION_GRAPH_SERVICE_TOKEN}
```

The token is backend-only. Export with
`yarn workspace @internal/backstage-plugin-application-graph-backend export-dynamic`.
The generated package exposes the standard new-backend-system plugin entrypoint.
Install it through RHDH's dynamic plugin configuration:

```yaml
plugins:
  - package: ./local-plugins/application-graph-backend/dist-dynamic
    disabled: false
```

See the [service and onboarding guide](../../analysis-server/APPLICATION-GRAPH.md)
for API endpoints, repository configuration, standard catalog fields, storage,
the real provider/consumer pair, and local verification.
