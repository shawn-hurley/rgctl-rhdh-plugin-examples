# Application graph catalog enrichment

This dynamic catalog backend module makes the normal **Import an existing Git
repository** flow start Java graph onboarding. An opted-in Component is imported
first; subsequent catalog processing adds graph-derived facts to its standard
`spec.dependsOn`, `spec.providesApis` and `spec.consumesApis` fields. Backstage
creates forward/inverse relations and displays them in its existing Dependencies
and API tabs. Repository descriptors are never rewritten.

Install alongside the application-graph backend and Rust service:

```yaml
plugins:
  - package: ./local-plugins/catalog-backend-module-application-graph/dist-dynamic
    disabled: false
```

Add this annotation to an otherwise ordinary Component descriptor:

```yaml
metadata:
  annotations:
    application-graph.io/enabled: 'true'
    backstage.io/source-location: url:https://github.com/example/application
    application-graph.io/maven-sources: 'true'
```

The Maven annotation opts into dependency resolution and source-JAR acquisition.
No application compilation or execution occurs during onboarding. Component
dependencies match direct Maven artifact coordinates to indexed catalog
Components. Resource dependencies match reachable native resource-use paths to
registered Resources' `application-graph.io/resource-uri` annotations. Importing
a dependency later allows earlier Components to acquire their relationship on
the next processing pass. Declared fields are preserved and merged with derived
facts. `subcomponentOf` remains an explicit organizational declaration.

HTTP providers receive a generated `<component-name>-api` API entity with an
OpenAPI operation summary extracted from their graph, plus `providesApis`.
The summary describes discovered routes and methods; it does not infer response
schemas. Provider endpoint identity is configured using
`application-graph.io/api-base-url`. Consumers acquire `consumesApis` and a
Component dependency only when the destination origin, route and method match.
If a graph contains relative request URLs, set
`application-graph.io/http-client-base-url` on the consumer; absent that binding,
relative paths are not used to infer cross-service identity.

Graph jobs are asynchronous and deduplicated while in flight. Processing adds
`application-graph.io/status` (`queued`, `running`, `indexed` or `failed`) and,
on success, `application-graph.io/graph-version`. Graph-service errors preserve
the imported descriptor and its manually declared relationships. Failed jobs
retry after 60 seconds; successful jobs are eligible for rebuilding after
`applicationGraph.catalog.refreshSeconds` (default 3600). Restarting the catalog
module clears its in-memory job tracker and submits a fresh build. The catalog's
own processing interval controls when job completion and new peers appear.

Export with:

```sh
yarn workspace @internal/backstage-plugin-catalog-backend-module-application-graph export-dynamic
```

The shared application-graph backend code is embedded in the dynamic artifact.
Run `sh local-dev/test-catalog-import.sh` for processor/enrichment tests and the
existing application-graph backend regression suite.
