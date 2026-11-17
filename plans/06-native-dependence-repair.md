# Native CFG/PDG and interprocedural repair plan

**Status:** Implementation in progress under the user's explicit authorization.

Concrete proposed Rust types and storage ownership are in the
[data-model companion](06-native-dependence-types.md).

The proposal is now [posted on issue #198](https://github.com/rgctl/rgctl/issues/198#issuecomment-6834308629).
Implementation has started with the independently tested
[boundary-traversal repair](../demo-evaluation/interprocedural-research/BOUNDARY-TRAVERSAL.md),
local commit `d001ea50`. The broader operand/binding/target/taint work remains.
The next self-contained commit, `964eeeb2`, repairs
[predicate def/use facts and procedural cache invalidation](../demo-evaluation/interprocedural-research/PREDICATE-REPAIR.md).
Commit `a56a0bf2` adds the
[structured operand foundation](../demo-evaluation/interprocedural-research/OPERAND-FOUNDATION.md),
including native language roles, persistence and source-position cache checks.
Commit `ae355193` adds [query-local invocation bindings](../demo-evaluation/interprocedural-research/CALL-BINDINGS.md)
for selected invocation/target pairs, including actual/formal, receiver and return
transfers. Local use/definition binding, target discovery and solver/consumer
integration remain.

Commit `578994fc` adds [typed formal entry definitions](../demo-evaluation/interprocedural-research/FORMAL-ENTRY.md)
and the formal-ID bridge into local statement PDGs. Precise expression-level
definition/use links, matched-context propagation and target discovery remain.

Commit `fb446a6a` adds [scoped local expression flow](../demo-evaluation/interprocedural-research/LOCAL-VALUE-FLOW.md)
and composes it with selected argument/return bindings in native regressions.
Receiver-origin propagation, target association/discovery, matched contexts and
adapter/taint consumption remain. The full adapter qualification is still the
separately recorded `578994fc` run; no score improvement is claimed from unit tests.

Commit `fe17294d` adds [bounded native possible-origin propagation](../demo-evaluation/interprocedural-research/VALUE-ORIGINS.md)
and the adapter now consumes it. The [full qualification](../demo-evaluation/value-flow/ORIGIN-PROPAGATION-QUALIFICATION.md)
finds 117/169 expected impacts: 14 recovered, no lost detections, 52 remaining
misses, zero execution failures. Negative warnings increased by five; the 19-case
Spring classifications are unchanged. Caller contexts deliberately merge; fields,
containers, callbacks, dynamic/deferred execution and shared taint consumption
remain. All coverage is partial; the broader milestone is not complete.

Seven additional [native model slices](../demo-evaluation/interprocedural-research/REMAINING-ORIGIN-MODELS.md)
are committed through `4db0d68a`. The [final qualification](../demo-evaluation/value-flow/REMAINING-MODELS-QUALIFICATION.md)
finds **155/169** impacts: 38 recovered since `fe17294d`, no lost detections,
14 remaining misses and zero execution failures. Spring and inputs are unchanged.
Partial coverage remains; module initialization, implicit/generated/captured
calls and state, partial arguments, reflection and shared taint consumers still
need dedicated work. This broader repair milestone is not complete.

The [native environment fix](../demo-evaluation/state-scope/ENVIRONMENT-FIX.md), `c87e8666`, now connects owning module,
lexical and captured bindings, source imports/package state, callable definition
values and Java members. Full qualification finds **162/169** impacts: seven
recovered from `4db0d68a`, no lost detections, seven remaining misses and zero
execution failures. Spring and fixture inputs are unchanged; 29 scope controls
match 58 runtime observations. All coverage remains partial. Implicit/generated
invocations, retained argument prefixes, reflection, missing Go anonymous graph
callables and shared taint consumption remain open.

**Start:** Maintainer agreement that the documented interprocedural dependence gaps are bugs; native reproducers and an implementation/history audit exist.

**End:** Native rgctl APIs expose and traverse the actual/formal, receiver and return dependencies needed to recover qualified dependency-impact paths; local and interprocedural taint use the same value-transfer facts where applicable. Language qualifications, unsupported forms and analysis limits are explicit.

This is an analysis repair workstream, independent of the product's original five delivery phases. It does not require redoing the existing UI, package acquisition, or local environment.

## Evidence and intended outcome

The [native audit](../demo-evaluation/interprocedural-research/README.md) reproduces six omissions on both fetched main `4d4d1b3f` and the identity-derived branch `3d798b31`. The local PDGs often already contain useful dependencies; interprocedural traversal drops them. Ordered argument metadata is read but not produced/consumed end to end; callee return transfer is absent. Formal-parameter PDG nodes described in the design documents do not exist.

Target example:

```javascript
class X { execute() { return 100; } }
function consume(value) { return value.execute(); }
function exercise() {
  const allocated = new X();
  const actual = allocated;
  return consume(actual);
}
```

An update to `X.execute` must be connected to `exercise` through native evidence:

```text
allocation X
  → local definition allocated
  → alias actual
  → actual argument 0 at this consume invocation
  → formal value in consume
  → receiver operand of this execute invocation
  → possible target X.execute
```

The native dependence edges explain the value path. A derived Calls edge explains possible invocation. Both are required; neither should substitute for the other. Source statement text is presentation, not the binding key.

## Ownership and representation

| Layer | Responsibility | Output |
|---|---|---|
| Native language lowering / CFG | Evaluation order, branch/control transfer, canonical formals, allocations, local definitions/uses, structured call/return operands | Per-function procedural facts using the existing parsed tree |
| Local PDG | Reaching definitions and control dependencies over those facts; explicit formal definitions and return/result dependencies | Scoped value/statement dependence edges |
| ICFG | Link each known call site to possible callee entries and normal/exceptional continuations | Call/return topology, retaining call-site identity and target uncertainty |
| Interprocedural dependence layer | Actual → formal, return → invocation result, receiver → implicit receiver, supported side effects | Lazy cross-function value-transfer edges and summaries |
| Receiver/target resolution | Follow possible value origins/types, apply hierarchy/member lookup, discover compatible targets | Derived possible Calls edges with provenance |
| Consumers | Slicing, blast radius, adapter explanation, taint and vulnerability evidence | Shared traversal facts with consumer-specific policies |

These are logical responsibilities, not a requirement for six new crates or a monolithic persisted graph. Prefer an on-demand dependence view joining per-function records by canonical identities. Keep the hot repository snapshot small and reuse existing lazy archives and workers.

Proposed internal edge roles are `ActualToFormal`, `ReceiverToThis`, `ReturnToResult`, and later side-effect/capture transfers. Final Rust types and serialization belong to implementation review; do not introduce ad-hoc string edges into the repository graph. Call/control-flow edges are distinct from data-transfer edges.

## Milestones

| Milestone | Start | End |
|---|---|---|
| 0. Failing regression contract | Audit accepted | Tests assert required origins, bindings and targets and fail against current code |
| 1. Native procedural operands | Regression contract agreed | Canonical formal/call/receiver/return facts exist and survive archive reload |
| 2. Correct interprocedural traversal | Structured operands available | Argument/return/receiver dependencies work in both directions across known calls |
| 3. Value origins and derived targets | Known-call transfers pass | Untyped `consume(new X())` resolves possible `X.execute` through a native fixed point |
| 4. Language and state coverage | Core target discovery passes | Qualification matrix covers required parameter/state/callback forms and reports unsupported forms |
| 5. Shared taint integration | Dependence edges stable | Taint follows the same cross-function data transfers with path-correct sanitizer handling |
| 6. Consumer integration and qualification | Native behavior qualified | Adapter and CLI/service consumers use native facts; results and docs reflect actual behavior |

### Milestone 0 — Turn the audit into failing regressions

**Start:** The audit and maintainer bug classification are accepted.

- Port the six controlled cases into native tests that assert the correct missing dependency statements, rather than asserting that today's omissions occur.
- Strengthen `test_interprocedural_slice_includes_caller`, the multi-hop test, and the argument-index test to check the actual caller definitions and full binding chain.
- Add end-to-end extraction variants so hand-constructed Calls/`arg_N` properties cannot hide production metadata gaps.
- Add `consume(new X())`, aliasing, factory returns, separate callers passing X/W, nested calls, repeated calls on one line, and same line numbers in different files.
- Separate test expectations for value-dependence edges, control-flow links, derived possible Calls, and final impact reachability. A successful call-graph traversal is not proof of argument binding.
- Record current native and adapter results before implementation. Preserve the existing 103/169 baseline and classify misses by missing fact versus missing propagation.

**End:** Current code fails precise positive regression assertions; local-flow and existing identity tests remain available as controls. No broad method-name matching or fixture expected labels are used to construct static edges.

### Milestone 1 — Add native formal and operand facts to CFG/PDG

**Start:** Required regression shapes are agreed.

- Seed each function entry with explicit formal definitions, including declared order, scoped variable identity and parameter kind. Resolve implicit `this`/`self`/Go receiver conventions separately from explicit argument numbering.
- Reuse canonical callable identity and byte spans. Add stable call-site/expression identities within that callable, including source revision/hash. Distinguish nested invocations and repeated invocations on one line.
- Lower call operands as structured facts: target expression, optional receiver, ordered/named actuals, spread/variadic markers, result value, and continuation. Lower return expressions and allocations into value-producing facts.
- Link each operand use to its reaching definitions. Preserve RHS-before-LHS semantics, lexical shadowing, evaluation order, branches and loops. A statement-wide set of names must not make every argument the receiver or every formal depend on every actual.
- Keep expression-role facts compact and queryable through the native crate API. Reuse the native parsed tree; the adapter must not parse source text to recover roles.
- Persist the facts needed after archive reload. Audit `local_types`, which is currently skipped by serialization; distinguish rebuildable hints from required persisted dependence facts.
- Version affected procedural archives/extraction caches and reject or rebuild incompatible records. Do not silently treat absent old-schema facts as an empty, complete result.

**End:** The same formal/operand identities and local dependence edges are available from fresh lowering and archive-backed queries for the qualified languages. Formal uses have real definitions; call roles require no statement-text heuristic.

### Milestone 2 — Repair ICFG and interprocedural PDG traversal

**Start:** Native operands/formal definitions are available.

- Retain each invocation's known possible targets in the ICFG, with call-site entry and return-continuation links. Match returns to the invocation that entered the callee; do not connect every callee exit to every caller continuation.
- Join actual values to formals by the language's binding rules. Join receiver values to implicit receivers. Join each normal return value to that invocation's result. Model exceptional continuations separately; do not treat a thrown value as a normal return.
- Resume local data/control dependence traversal in every visited function. Remove the parameter-name text fallback and line/name call-site matching as semantic binding mechanisms.
- Support backward and forward interprocedural queries over the same transfers. Backward slicing includes relevant control dependencies; value-origin and taint propagation use data-transfer edges, not arbitrary control reachability.
- Use bounded call-site context or equivalent matched summaries to prevent infeasible caller/return mixing. Cache summaries by callable/source/analysis configuration. Worklist state must include the value/operand and relevant context, not just a visited function.
- Handle recursive SCCs by monotone fixed-point iteration. Bound query work/context depth explicitly; report `incomplete` and reasons when limits are reached.
- Load missing records lazily when an archive is partially populated. Propagate native CFG-build errors and unavailable-source facts instead of dropping functions or returning a successful empty trace.
- Use file/function-qualified source locations in results and counts; bare cross-file line numbers cannot identify statements.

**End:** Caller-local, renamed multi-hop, multi-argument and return/result regressions pass with the correct bindings. Fresh and archive-backed paths agree. Recursion terminates, limits are visible, and ordinary local slices retain their behavior.

### Milestone 3 — Use value origins to discover missing receiver targets

**Start:** Cross-function transfers work for already known calls.

- Track conservative sets of possible allocation origins/types through local aliases, actual/formal transfers and returns. Declared types constrain evidence but are not required for propagating `new X()` into an untyped formal.
- At each invocation, query the receiver operand's possible origins. Reuse native hierarchy/member lookup, inherited implementations and canonical callable identities to select compatible methods.
- Derive possible Calls edges with the invocation identity, value path and resolution assumptions. Preserve original edges/evidence; do not relabel inferred targets as directly extracted certainty.
- Iterate origin propagation and newly discovered call targets to a fixed point. Seed direct/static calls, known constructions and available native dispatch facts so the algorithm does not require `consume → X.execute` to exist before it can discover that edge.
- Preserve all possible conditional factory/profile alternatives. When origins are unknown, retain explicit conservative possibilities/diagnostics; never narrow to a single arbitrary implementation.
- Expose the native facts/results to an external Rust qualification probe and the adapter. This preserves the agreed workflow of proving the joins externally without building a second parser or a duplicate private dataflow solver.

**End:** The five-language positive parameter/alias/factory examples reach their concrete implementation through native evidence, where language syntax applies. No parameter type annotation or global same-method-name fallback is needed. Unsupported dynamic forms are measured separately.

### Milestone 4 — Broaden state, callback and language semantics

**Start:** Ordinary parameter, receiver and return paths work.

Qualify Java, JavaScript, TypeScript, Python and Go separately. Implement incremental slices of this matrix, each with independently executed positive fixtures and W-only controls:

| Family | Required facts/transfers | Examples |
|---|---|---|
| Call binding | Language-aware formal/actual mapping | Positional, Python keyword/default arguments, JS spreads/rest, Java varargs, Go variadics, implicit receivers |
| Local value reuse | Reaching definitions, alias identity | Reassignment, shadowing, branch joins, loops, destructuring |
| Returned values | Return/result transfer and alternatives | Direct factories, conditional factories, chained calls, Go multiple returns |
| Instance state | Receiver/allocation-sensitive field writes and reads | Constructors, setters, holders, object properties, Go struct fields |
| Collection state | Conservative element access paths | Lists/maps/arrays, interface containers, indexed reads and writes |
| Higher-order calls | Callable origins, captures and invocation transfers | Lambdas, function parameters, bound methods, closures, callbacks |
| Shared/module state | Scoped global/module reads and writes | Registries, imported values, stored providers |
| Framework DI | Native registration/provider/binding facts feeding origin sets | Qualifiers, all possible profiles, conditional provider outputs |
| Deferred execution | Explicit callback/scheduler models where known | Async callbacks, promises/futures, goroutines and deferred calls |

Reuse native field-write facts, but do not infer a complete heap flow merely because writes to a field name are indexed. Track receiver/access-path provenance; unknown aliases or updates require conservative unions. Only use strong updates when the affected location is established.

For library/framework boundaries without bodies, use explicit versioned summaries/models or report unknown transfer. Do not claim automatic support for every scheduler, reflection mechanism, dynamic import, or generated function.

**End:** The matrix records proven forms, missed positives, missing native facts and explicit limits. Every currently supported form has no missed positives in its fixtures. False positives are recorded and allowed by the initial recall policy; an unsupported positive remains a miss even if a coverage warning is emitted.

### Milestone 5 — Apply the shared dependencies to taint

**Start:** Interprocedural data-transfer edges and their provenance are stable.

Research confirms this applies to taint. `TaintAnalyzer` currently takes a single PDG/CFG; its source-to-sink search follows that PDG's data adjacency. The sink-first CLI checks archive presence but does not perform the documented cross-function PDG proof. `vuln analyze` consumes structural path summaries rather than a shared interprocedural taint traversal.

The [taint probe](../demo-evaluation/interprocedural-research/native/src/bin/taint.rs) and [captured results](../demo-evaluation/interprocedural-research/evidence/taint-results.json) compare a local positive control with equivalent argument-boundary and return-boundary forms. This demonstrates the local API's limit, not invocation of a nonexistent interprocedural taint API.

Planned repair:

- Retain declarative source/sink/sanitizer rule packs. Bind matched sources, sink arguments and sanitizer inputs/outputs to structured operands, rather than tainting an entire statement indiscriminately.
- Add an interprocedural taint consumer of the shared data-transfer view, including parameter, receiver, return, supported field and callback transfers. Keep control dependence available for slicing; do not propagate taint merely because a statement is control-dependent.
- Track taint state per value/access path and call context. A source that reaches argument 0 must not automatically taint argument 1 or every receiver field.
- Treat sanitizers as transformations of specific values with sink-category semantics. Local block dominance does not prove cross-function sanitization; a sanitizer dominating a sink statement may still sanitize the wrong argument.
- Preserve unsanitized alternatives when a sanitizer is conditional, bypassed, applied to a different value, or followed by reassignment. The existing single-parent reachability path must not allow a sanitized representative path to hide another unsanitized route. Use joined taint states/summaries rather than enumerating all execution paths.
- Add argument/return/callback source-to-sink positives; clean sibling-argument controls; sanitizer wrappers; conditional sanitizer bypass; wrong-value sanitizers; recursion; missing dependency bodies; unknown targets; and bounded-analysis fixtures.
- Wire sink-first and vulnerability consumers to real dataflow evidence. Distinguish structural sink reachability, possible taint flow, and proven sanitization. Changing security verdict policy requires explicit tests and review; taint is not required to prove every kind of dependency impact.
- Return incomplete/unknown evidence for unsupported boundaries or limits. Empty paths alone must not certify safety or non-exploitability.

**End:** The boundary taint positives are found through the same native transfers as dependency impact; unsanitized alternatives cannot be erased by an unrelated/sanitized path. Local taint behavior remains tested. CLI/service output distinguishes evidence levels and coverage.

### Milestone 6 — Integrate, retest and correct the contract

**Start:** Qualified native dependencies, target discovery and taint are available.

- Expose versioned native query results containing function-qualified nodes, typed edge roles, call-site context, source provenance, unresolved facts and completion/limit status.
- Have local and interprocedural CLI/CPG/service paths call shared native entry points. Keep local mode explicit; do not silently change a local query into an unbounded repository analysis.
- Replace experimental adapter joins only after native results demonstrate equal or better recall. Retain explicit distinctions between established value bindings and conservative fallback candidates in explanations.
- Run all 254 existing qualification cases and the Spring DI set. Compare each changed classification and path to the independent runtime oracle. The initial criterion is no lost detections and recovery of the newly supported positive families; do not require these fixes to solve unrelated missing-extraction cases.
- Run focused upstream language, CFG/PDG, interprocedural, archive and taint tests, then the mandatory `./scripts/preflight-pr.sh --quick` for upstream code changes. Test cache invalidation and source/graph staleness.
- Check representative bounded-query memory/time and lazy-loading behavior before any scale claims. Preserve parallel extraction, parse reuse, streaming commits and small repository snapshots. No new CI or pinned toolchain/cache setup is part of this work.
- Correct design/API docs with executable examples and actual capability limits. Remove claims of argument/return or PDG-verified taint support until corresponding tests pass.

**End:** A published local qualification report identifies recovered paths, remaining misses, provenance and unsupported boundaries. Code and docs agree. GitHub issue edits, publication and commits remain separate review steps.

## Suggested commit boundaries

1. Regression fixtures and strengthened assertions.
2. Native formal/call operand lowering plus local PDG edges and archive versioning.
3. ICFG transfer view and corrected backward/forward interprocedural traversal.
4. Receiver-origin propagation and derived target fixed point.
5. State/callback/language families in small independently qualified commits.
6. Shared taint traversal, sanitizer-state tests and security consumer wiring.
7. Adapter/CLI/service migration, qualification report and documentation correction.

Keep the existing identity-derived worktree and canonical identity implementation. Rebase on the maintainer's accepted identity work when appropriate; do not redesign overload identity as part of these repairs.
