# Proposed Rust data model for native dependence repair

**Status:** Design proposal, not an implemented API. Companion to the
[repair plan](06-native-dependence-repair.md). Existing canonical callable
identity and statement PDG IDs remain authoritative.

The distinction is between persistent per-function facts and derived query
results. Record syntax and local dependencies once; derive cross-function
bindings, contexts and possible targets as needed.

## 1. Compact IDs scoped to a function record

```rust
struct StatementId(u32);
struct ValueId(u32);
struct LocalSymbolId(u32);
struct CallSiteId(u32);
struct FormalId(u32);
struct ReturnId(u32);
struct SymbolRefId(u32); // index into the record's canonical symbol-reference pool

struct PoolRange {
    start: u32,
    len: u32,
}

struct ByteSpan {
    start: u32,
    end: u32, // exclusive, relative to the source file
}
```

IDs are assigned deterministically within one source/analysis revision. They
are not identities that survive arbitrary edits. Existing function UUID,
source hash and procedural schema version qualify the record; a query must
not join local IDs from incompatible records. Check u32 conversion/limits;
report an unsupported-size result instead of wrapping an index.

Keep the existing UUID-backed `PdgNodeId`. Add a mapping to the dense
`StatementId`; replacing the existing PDG ID is not required for this repair.

## 2. Native CFG additions

Add `id: StatementId` and `span: ByteSpan` to `Statement`. Add one field to
`ControlFlowGraph`:

```rust
struct ControlFlowGraph {
    // Existing blocks, edges, entry, exits and derived indices retained.
    operands: FunctionOperands,
}

struct FunctionOperands {
    symbols: Vec<LocalSymbol>,
    symbol_refs: Vec<uuid::Uuid>,
    values: Vec<ValueFact>,
    inputs: Vec<ValueId>,
    formals: Vec<FormalFact>,
    calls: Vec<CallSiteFact>,
    arguments: Vec<ArgumentFact>,
    returns: Vec<ReturnFact>,
    names: Vec<String>, // interned once per function; no copied name per operand
    coverage: OperandCoverage,
}

struct LocalSymbol {
    name: u32, // index into names
    scope: u32,
}

struct ValueFact {
    statement: StatementId,
    span: ByteSpan,
    kind: ValueKind,
    inputs: PoolRange, // range into FunctionOperands.inputs
}

enum ValueKind {
    Formal(FormalId),
    Read(LocalSymbolId),
    Definition(LocalSymbolId),
    Allocation { type_ref: Option<SymbolRefId> },
    SymbolReference(SymbolRefId),
    Copy,
    Projection,
    Computation,
    CallResult { call: CallSiteId, position: u32 },
    Constant,
    Unknown,
}

struct FormalFact {
    symbol: LocalSymbolId,
    value: ValueId, // explicit entry definition
    kind: FormalKind,
    default_value: Option<ValueId>,
}

enum FormalKind {
    Receiver,
    Positional { index: u32 },
    PositionalOnly { index: u32 },
    KeywordOnly,
    VariadicPositional,
    VariadicKeyword,
}

struct CallSiteFact {
    statement: StatementId,
    span: ByteSpan,
    callee: ValueId,
    receiver: Option<ValueId>,
    arguments: PoolRange, // range into arguments
    results: PoolRange,   // range into inputs; supports multiple return values
}

struct ArgumentFact {
    value: ValueId,
    kind: ArgumentKind,
}

enum ArgumentKind {
    Positional,
    Named { name: u32 },
    SpreadPositional,
    SpreadKeywords,
}

struct ReturnFact {
    statement: StatementId,
    span: ByteSpan,
    values: PoolRange, // range into inputs; empty for a valueless return
}
```

Supporting details to finalize during language lowering: an explicit method
receiver is not always an extra implicit parameter (for example, Python bound
versus unbound calls). The binder uses the invocation kind and language, rather
than shifting every argument index blindly. Preserve that invocation kind in
the production call record. Type annotations, projection members/indexes,
operation codes and unknown-reason metadata also need compact native records;
the enums above show the core shape, not a complete multi-language IR.

`OperandCoverage` must distinguish complete supported lowering from unsupported
constructs and failed lowering. Diagnostic details can live in a shared pool.
No canonical type resolution is required to retain `new X()`'s value identity;
an unresolved type reference remains explicit native evidence.

Allocations/calls have their own value records even when nested in one source
statement. `inputs` retains operand order. Operator-specific lowering/transfer
rules determine semantics: a computational dependency does not automatically
mean the output is an alias of every input.

## 3. Local PDG additions

```rust
struct ProgramDependenceGraph {
    // Existing statement nodes, data/control edges and indices retained.
    value_edges: Vec<ValueEdge>,
}

struct ValueEdge {
    from: ValueId,
    to: ValueId,
    kind: ValueEdgeKind,
}

enum ValueEdgeKind {
    ReachingDefinition,
    Copy,
    ComputationDependency,
    ProjectionDependency,
}
```

The CFG stores value-producing/use operations; PDG edges establish dependence
between them. Seed formal definitions in the CFG entry block and give them
corresponding PDG nodes. `StatementKind` therefore needs a formal-definition
variant. Operand-to-statement mapping connects detailed value paths to the
existing statement-level UI/slicing API without changing `PdgNodeId`.

Use the local dataflow solver for reaching-definition edges, including branch
joins and loops. Do not allocate a new edge merely because two names occur in
one statement. Taint may propagate over computation dependencies; object-origin
resolution must use operation semantics rather than treating every such edge
as object identity.

## 4. Derived interprocedural control links

```rust
struct FunctionRef(u32); // query-local index into canonical callable UUIDs
struct ContextId(u32);   // bounded, interned query context

struct CallRef {
    function: FunctionRef,
    site: CallSiteId,
}

struct ControlPoint {
    function: FunctionRef,
    block: BlockId,
    boundary: u32, // before statement N; statement count means block end
}

struct CallLink {
    call: CallRef,
    target: FunctionRef,
    entry: ControlPoint,
    normal_continuation: Option<ControlPoint>,
    exceptional_continuations: PoolRange,
    evidence: EvidenceId,
}
```

The query/view owns a shared pool of exceptional `ControlPoint`s. Some calls
have no normal continuation (known non-returning callees); no-handler exceptional
flow can exit the function. The implementation can split blocks at calls or
use statement boundaries, but must define ordering for nested invocations.
Call-site identity matches callee exits to the appropriate continuation.

`CallLink` belongs to a derived ICFG view, not to a repository-wide eagerly
cloned graph. Known and newly discovered targets populate that view. `EvidenceId`
references pooled extraction/dispatch/value-path evidence; it is not a repeated
description string on every edge.

## 5. Derived cross-function PDG transfers

```rust
struct ValueRef {
    function: FunctionRef,
    value: ValueId,
}

struct InterproceduralTransfer {
    from: ValueRef,
    to: ValueRef,
    call: CallRef,
    kind: TransferKind,
}

enum TransferKind {
    ActualToFormal,
    ReceiverToFormal,
    ReturnToResult,
    DefaultToFormal,
}

struct FlowState {
    value: ValueRef,
    context: ContextId,
}
```

The transfer view reuses the call binder's result; slicing, origin propagation
and taint must not implement separate argument binders. Matched/bounded context
belongs to traversal state rather than duplicating every persistent local fact
for each caller. Recursion is handled by fixed-point summary/state processing.

Field effects, captures, containers and exception values need additional typed
transfers/access paths in later milestones. They must not be approximated as
ordinary scalar returns without diagnostics.

## 6. Derived receiver targets and shared result status

```rust
struct PossibleCallTarget {
    call: CallRef,
    target: FunctionRef,
    context: ContextId,
    evidence: EvidenceId,
}

enum Completion {
    CompleteForModeledInputs,
    Incomplete,
}

struct AnalysisResult<T> {
    data: T,
    completion: Completion,
    diagnostics: Vec<AnalysisDiagnostic>,
}
```

Possible-origin sets, target sets, evidence and contexts use shared query pools.
Worklists propagate only newly discovered facts; records are loaded lazily.
Track source/schema/configuration provenance once in the query's record table.
`CompleteForModeledInputs` is a scope statement, not a blanket upgrade-safety
verdict. Unknown targets, unsupported constructs, missing records and limits
must affect coverage rather than disappearing from the result.

## 7. Taint changes

Keep local `TaintAnalyzer` and existing rule packs. Add an interprocedural
consumer whose state includes `FlowState`, taint category and value-specific
sanitization state. Interprocedural paths use function-qualified value/statement
references; a bare PDG-node ID or source variable name is insufficient.

Reuse `InterproceduralTransfer`; do not add taint-specific copies of argument,
receiver or return edges. Source/sink/sanitizer matches bind to operand roles.
Preserve unsanitized alternatives through joins and recursion. Never compare
local block IDs across functions to establish sanitizer dominance.

## 8. Storage and compatibility

- Persist `FunctionOperands` and local `value_edges` with the existing
  `CfgPdgRecord`'s CFG/PDG. Do not duplicate them in both structures.
- Bump the binary procedural schema and affected extraction cache versions;
  update custom CFG/PDG deserializers as well as serialization.
- Build adjacency indices from compact pools as needed; do not persist all
  redundant forward/backward maps by default.
- Derive call links, transfers, contexts and target results lazily. Persisting
  reusable summaries is a separate measured optimization, not an initial
  requirement.
- Measure Rust layout and serialized size separately. A new enum/field can add
  padding, and `Option<ValueId>` is not assumed to fit in four bytes. Consider
  checked `NonZeroU32`-backed IDs if layout measurements justify them.
- No new field on every repository symbol/edge is required. The detailed data
  lives in the optional procedural layer and query results.
