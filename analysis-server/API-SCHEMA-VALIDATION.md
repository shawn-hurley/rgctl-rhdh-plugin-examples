# Generated API schema validation

The graph service now persists source-derived OpenAPI contracts alongside HTTP
operations. The catalog module places those contracts and reusable schemas into
the generated API entity, preserving existing provider/consumer relationships.
No application compilation, Java reflection or external semantic indexer is used.

## Automated checks

All **149 Rust tests** pass, including nine new schema/compatibility cases:

- DTO/record properties, field renaming/ignoring and validation constraints.
- Request bodies, path/query/header bindings, media types and response status.
- Generic substitution, collections/maps, enums and recursive references.
- Public getters, unresolved types and custom-serializer handling.
- Cross-file wildcard imports, nested types and primitive binary arrays.
- Source-available inheritance and read-only/write-only properties.
- Overloaded method annotations remain scoped to their actual declaration.
- Stored manifests lacking schema fields remain readable.
- Wildcard collection imports, enum parameter defaults and void-method exclusion.

All **24 backend/catalog tests** pass. Two new checks cover preserving operation
contracts/reusable schemas in the generated API and unique operation identities
for routes that support multiple HTTP methods. Type checking, formatting and
dynamic module export pass.

## Existing Spring demo

The existing Quoters repository was reindexed through the service. Its generated
API definition contains:

| Operation/model | Verified result |
| --- | --- |
| `GET /api` | Array response with items referencing `QuoteResource` |
| `GET /api/{id}` | Required `int64` path parameter and `QuoteResource` response |
| `GET /api/random` | `QuoteResource` response |
| `QuoteResource` | `value` references `Quote`; `type` is a string |
| `Quote` | `id` is `int64`; `quote` is a string |

All emitted schema references resolve. Backstage's Definition tab renders the
operations and both reusable models. The six existing Component imports retain
their dependency/injection controls and API links, including inverse provider
and consumer relationships. Local catalog registrations were preserved across
the backend reload.

External HATEOAS superclass fields are unresolved when their sources are absent;
the definition retains that uncertainty explicitly. Application-specific
serializer configuration, mix-ins, multipart/model binding and exception-handler
response inference are outside this pass. See the
[catalog module guide](../plugins/catalog-backend-module-application-graph/README.md#source-derived-api-schemas)
for supported inference and compatibility behavior.
