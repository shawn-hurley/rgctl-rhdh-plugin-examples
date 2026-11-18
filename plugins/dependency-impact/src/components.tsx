import React, { FormEvent, useEffect, useState } from "react";
import {
  Content,
  Header,
  InfoCard,
  DependencyGraph,
  DependencyGraphTypes,
  Page,
  Progress,
} from "@backstage/core-components";
import { useApi } from "@backstage/core-plugin-api";
import { useEntity } from "@backstage/plugin-catalog-react";
import { useSearchParams } from "react-router-dom";
import {
  Coordinate,
  ComparisonInput,
  Job,
  Report,
  SourceBlock,
  dependencyImpactApiRef,
} from "./api";
import "./styles.css";

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);
const coordinate = (d: Coordinate) =>
  "modulePath" in d ? d.modulePath : `${d.groupId}:${d.artifactId}`;
const stages: Record<string, string> = {
  resolving: "Resolving dependency versions",
  "acquiring-sources": "Acquiring dependency sources",
  indexing: "Indexing application and dependency code",
  comparing: "Comparing dependency functions",
  "tracing-impact": "Tracing application impact",
  "assembling-report": "Preparing source blocks and report",
};

function SourceViewer({ block }: { block?: SourceBlock }) {
  if (!block)
    return (
      <p className="di-notice">
        A reliable source block is unavailable for this function. Review the
        analysis limitations below.
      </p>
    );
  const identity = block.sourceIdentity;
  return (
    <section aria-label="Source context" className="di-source">
      <div className="di-source-heading">
        <strong>{block.file}</strong>
        <span>
          {block.snapshot} · lines {block.startLine}–{block.endLine}
        </span>
      </div>
      <p className="di-caption">
        {identity.artifact
          ? `${coordinate(identity.artifact)}${"modulePath" in identity.artifact ? "@" : ":"}${identity.artifact.version}`
          : `Application revision: ${identity.revision ?? "analyzed input"}`}
      </p>
      <pre tabIndex={0} aria-label={`${block.file} source code`}>
        <code>
          {block.text.split("\n").map((line, i) => (
            <span className="di-code-line" key={i}>
              <span className="di-line-number" aria-hidden="true">
                {block.startLine + i}
              </span>
              <span>{line || " "}</span>
              {"\n"}
            </span>
          ))}
        </code>
      </pre>
      <details className="di-caption">
        <summary>Source identity</summary>
        <p className="di-wrap">SHA-256: {identity.sha256}</p>
        <p>Full enclosing function from the analyzed {block.snapshot} input.</p>
      </details>
    </section>
  );
}

function ReportViewer({ report }: { report: Report }) {
  const appId = report.application.sourceIdentity.id;
  const blocks = new Map(report.sourceBlocks.map((b) => [b.id, b]));
  const areaPaths = new Map<string, typeof report.paths>();
  for (const path of report.paths) {
    const boundary = path.symbols.findIndex(
      (symbol, i) =>
        symbol.sourceId === appId &&
        path.symbols[i + 1] &&
        path.symbols[i + 1].sourceId !== appId
    );
    if (boundary >= 0) {
      const id = path.symbols[boundary].id;
      areaPaths.set(id, [...(areaPaths.get(id) ?? []), path]);
    }
  }
  const areas = report.impacts
    .filter((i) => areaPaths.has(i.symbol.id))
    .sort(
      (a, b) =>
        a.symbol.file.localeCompare(b.symbol.file) ||
        (a.symbol.startLine ?? 0) - (b.symbol.startLine ?? 0)
    );
  const [areaId, setAreaId] = useState(areas[0]?.id);
  const [blockId, setBlockId] = useState(() => {
    try {
      const id = decodeURIComponent(
        window.location.hash.replace(/^#source-/, "")
      );
      if (blocks.has(id)) return id;
    } catch {
      /* Invalid fragment. */
    }
    return areas[0]?.sourceBlockId ?? undefined;
  });
  const area = areas.find((i) => i.id === areaId);
  const paths = area ? areaPaths.get(area.symbol.id) ?? [] : [];
  const symbols = new Map(
    paths.flatMap((path) =>
      path.symbols.map((symbol) => [symbol.id, symbol] as const)
    )
  );
  const edges = new Map(
    paths.flatMap((path) =>
      path.edges.map((edge) => [`${edge.from}:${edge.to}`, edge] as const)
    )
  );
  const changed = new Set(
    report.changes.flatMap((c) => (c.baseline ? [c.baseline.id] : []))
  );
  const uncertain =
    report.coverage.truncated ||
    report.diagnostics.some(
      (d) => ![
        "STATIC_CALL_COVERAGE_UNVERIFIED",
        "NATIVE_VALUE_ORIGIN_COVERAGE",
        "CONSERVATIVE_DISPATCH_POLICY",
      ].includes(d.code)
    );
  function download() {
    const url = URL.createObjectURL(
      new Blob([JSON.stringify(report, null, 2)], { type: "application/json" })
    );
    const a = document.createElement("a");
    a.href = url;
    a.download = `dependency-impact-${report.comparisonId}.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  function selectSource(id: string) {
    setBlockId(id);
    const url = new URL(window.location.href);
    url.hash = `source-${encodeURIComponent(id)}`;
    window.history.replaceState(window.history.state, "", url);
  }
  const request = report.resolution.requested;
  const baseline = report.resolution.baseline.find(
    (d) => coordinate(d) === coordinate(request.dependency)
  );
  return (
    <div className="di-report" data-testid="comparison-report">
      <div className="di-section-heading">
        <div>
          <p className="di-eyebrow">Testing focus for this update</p>
          <h2>
            {coordinate(request.dependency)} · {baseline?.version ?? "baseline"}{" "}
            → {request.targetVersion}
          </h2>
          <p>
            {report.application.entityRef} · {report.application.revision}
          </p>
        </div>
        <button className="di-button" onClick={download}>
          Download report
        </button>
      </div>
      {areas.length > 0 ? (
        <>
          <h3>Where to focus testing</h3>
          <p>
            {areas.length} application {areas.length === 1 ? "area" : "areas"}{" "}
            identified. Exercise these functions and the flows that use them.
          </p>
          {uncertain && (
            <p className="di-notice">
              Additional review is needed: some changes or source context could
              not be assessed reliably.
            </p>
          )}
          <div className="di-explorer">
            <nav className="di-function-list" aria-label="Testing areas">
              {areas.map((i) => (
                <button
                  key={i.id}
                  aria-pressed={areaId === i.id}
                  onClick={() => {
                    setAreaId(i.id);
                    setBlockId(i.sourceBlockId ?? undefined);
                  }}
                >
                  <strong>{i.symbol.name}</strong>
                  <span>{i.symbol.file}</span>
                </button>
              ))}
            </nav>
            <div className="di-context">
              <h3>Exercise {area?.symbol.name}</h3>
              <p>
                This application function may reach changed dependency code.
                Possible implementations are included across profiles and factory
                conditions. Upstream callers provide flow context.
              </p>
              <h3>Why this area?</h3>
              <p className="di-caption">
                Arrows include reported calls and possible implementation paths.
                Select a node for its full source block.
                Application · Dependency · Changed · Ambiguous
              </p>
              {paths.some((path) => path.edges.some((edge) => edge.type === "PossibleDispatch")) && (
                <div className="di-notice" role="status">
                  <strong>Possible implementation impact</strong>
                  <p>This area needs testing if a changed implementation is selected.</p>
                  <ul>
                    {[...new Set(paths.flatMap((path) => path.edges
                      .filter((edge) => edge.type === "PossibleDispatch")
                      .map((edge) => `${edge.binding?.declaredQualifiedName ?? "Declared method"} may dispatch to ${edge.binding?.implementationQualifiedName ?? "an implementation"}`)))].map((message) => (
                        <li key={message}>{message}</li>
                      ))}
                  </ul>
                </div>
              )}
              <div
                style={{ height: 340, position: "relative" }}
                aria-label="Code paths"
              >
                <DependencyGraph
                  nodes={[...symbols.values()].map((symbol) => ({
                    id: symbol.id,
                    symbol,
                  }))}
                  edges={[...edges.values()]}
                  direction={DependencyGraphTypes.Direction.LEFT_RIGHT}
                  fit="contain"
                  zoom="enable-on-click"
                  showArrowHeads
                  renderNode={({ node }) => {
                    const symbol = node.symbol;
                    const ambiguous =
                      symbol.resolution === "ambiguous-overload";
                    const label = changed.has(symbol.id)
                      ? "Changed dependency"
                      : symbol.sourceId === appId
                      ? "Application"
                      : "Dependency";
                    return (
                      <g
                        role="button"
                        tabIndex={0}
                        aria-label={`${symbol.name} · ${label}${
                          ambiguous ? " · ambiguous" : ""
                        }`}
                        onClick={() => selectSource(symbol.id)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            selectSource(symbol.id);
                          }
                        }}
                      >
                        <rect
                          width={220}
                          height={70}
                          rx={7}
                          fill={
                            changed.has(symbol.id)
                              ? "#fff0cf"
                              : symbol.sourceId === appId
                              ? "#e6f1fb"
                              : "#eeeeee"
                          }
                          stroke={blockId === symbol.id ? "#1765a0" : "#747474"}
                          strokeWidth={blockId === symbol.id ? 3 : 1}
                          strokeDasharray={ambiguous ? "5 3" : undefined}
                        />
                        <text x={12} y={24} fill="#182534">
                          {symbol.name.slice(0, 26)}
                        </text>
                        <text x={12} y={47} fill="#374859" fontSize={11}>
                          {label}
                          {ambiguous ? " · ambiguous" : ""}
                        </text>
                        <title>{symbol.qualifiedName ?? symbol.name}</title>
                      </g>
                    );
                  }}
                />
              </div>
              <details className="di-details">
                <summary>Accessible path nodes</summary>
                {[...symbols.values()].map((symbol) => (
                  <p key={symbol.id}>
                    <a
                      href={`?comparison=${
                        report.comparisonId
                      }#source-${encodeURIComponent(symbol.id)}`}
                      onClick={() => selectSource(symbol.id)}
                    >
                      {symbol.qualifiedName ?? symbol.name}
                    </a>
                    {symbol.resolution === "ambiguous-overload"
                      ? " · ambiguous overload"
                      : ""}
                  </p>
                ))}
              </details>
              <SourceViewer block={blockId ? blocks.get(blockId) : undefined} />
            </div>
          </div>
        </>
      ) : (
        <div className="di-notice" role="status">
          <h3>
            {report.impacts.length
              ? "Testing scope could not be determined"
              : "No affected application paths found"}
          </h3>
          <p>
            {report.impacts.length
              ? "Affected code was reported, but testing areas could not be assembled. Review the analysis details before deciding what to test."
              : uncertain
              ? "No affected paths were identified within the available analysis. Coverage gaps may leave paths undetected; review the analysis details alongside your regression tests."
              : report.changes.length
              ? "Dependency changes were detected, but no paths from application code to those changes were identified within the available analysis."
              : "No supported dependency function changes were detected within the available analysis."}
          </p>
        </div>
      )}
      <details className="di-details">
        <summary>Resolved versions and source identity</summary>
        <p>Application SHA-256: {report.application.sourceIdentity.sha256}</p>
        {report.resolution.baseline.map((d) => (
          <p key={coordinate(d)}>
            {coordinate(d)}: {d.version} →{" "}
            {report.resolution.candidate.find(
              (c) => coordinate(c) === coordinate(d)
            )?.version ?? "Absent"}
          </p>
        ))}
        <p>Source links remain within this exact analyzed comparison.</p>
      </details>
    </div>
  );
}

function Workbench({ entityRef }: { entityRef?: string }) {
  const api = useApi(dependencyImpactApiRef);
  const [params, setParams] = useSearchParams();
  const jobId = params.get("comparison");
  const [inputs, setInputs] = useState<ComparisonInput[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [targetVersion, setTargetVersion] = useState("");
  const [loading, setLoading] = useState(true);
  const [available, setAvailable] = useState<boolean>();
  const [discoveryError, setDiscoveryError] = useState("");
  const [error, setError] = useState("");
  const [job, setJob] = useState<Job>();
  const [report, setReport] = useState<Report>();
  const [starting, setStarting] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [pollRetry, setPollRetry] = useState(0);
  const selected = inputs.find((input) => input.id === selectedId);
  const busy = starting || job?.state === "queued" || job?.state === "running";

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setDiscoveryError("");
    setAvailable(undefined);
    api
      .getStatus(controller.signal)
      .then((status) => setAvailable(status.analysisAvailable))
      .catch(() => {
        if (!controller.signal.aborted) setAvailable(false);
      });
    api
      .getInputs(entityRef, controller.signal)
      .then((result) => {
        if (controller.signal.aborted) return;
        setInputs(result.inputs);
        setSelectedId(result.inputs[0]?.id ?? "");
        setTargetVersion(result.inputs[0]?.targetVersion ?? "");
      })
      .catch((e) => {
        if (!controller.signal.aborted) setDiscoveryError(errorText(e));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [api, entityRef, refresh]);

  useEffect(() => {
    if (!jobId || loading || discoveryError) return undefined;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let initialized = false;
    setError("");
    setReport(undefined);
    async function poll() {
      try {
        const current = await api.getJob(jobId!, controller.signal);
        if (entityRef && current.request.entityRef !== entityRef)
          throw new Error(
            "This comparison belongs to another catalog entity. Open it from the global Dependency Impact page."
          );
        if (controller.signal.aborted) return;
        setJob(current);
        if (!initialized) {
          const input = inputs.find(
            (i) =>
              i.entityRef === current.request.entityRef &&
              coordinate(i.dependency) ===
                coordinate(current.request.dependency)
          );
          setSelectedId(input?.id ?? "");
          setTargetVersion(current.request.targetVersion);
          initialized = true;
        }
        if (current.state === "succeeded") {
          const result = await api.getReport(current.id, controller.signal);
          if (
            result.schemaVersion !== "1" ||
            result.comparisonId !== current.id ||
            result.application.entityRef !== current.request.entityRef
          )
            throw new Error(
              "The report identity or schema does not match this comparison."
            );
          if (!controller.signal.aborted) setReport(result);
        } else if (current.state === "queued" || current.state === "running") {
          timer = setTimeout(() => {
            void poll();
          }, 1500);
        }
      } catch (e) {
        if (!controller.signal.aborted) setError(errorText(e));
      }
    }
    void poll();
    return () => {
      controller.abort();
      if (timer) clearTimeout(timer);
    };
  }, [api, entityRef, jobId, inputs, loading, discoveryError, pollRetry]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!selected || starting) return;
    setStarting(true);
    setError("");
    setReport(undefined);
    setJob(undefined);
    setParams((previous) => {
      previous.delete("comparison");
      return previous;
    });
    try {
      const created = await api.submit({
        entityRef: selected.entityRef,
        applicationRevision: selected.applicationRevision,
        dependency: selected.dependency,
        targetVersion: targetVersion.trim(),
        strategy: selected.strategy,
      });
      setJob(created);
      setParams((previous) => {
        previous.set("comparison", created.id);
        return previous;
      });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setStarting(false);
    }
  }
  function select(id: string) {
    const input = inputs.find((i) => i.id === id);
    setSelectedId(id);
    setTargetVersion(input?.targetVersion ?? "");
    setJob(undefined);
    setReport(undefined);
    setError("");
    setParams((previous) => {
      previous.delete("comparison");
      return previous;
    });
  }
  return (
    <div className="dependency-impact">
      <InfoCard title="Compare dependency versions">
        <p>
          Explore where dependency changes could affect your application code.
        </p>
        {loading && (
          <>
            <Progress />
            <p role="status">Discovering available comparisons…</p>
          </>
        )}
        {discoveryError && (
          <div role="alert" className="di-error">
            Unable to discover comparison inputs: {discoveryError}{" "}
            <button
              className="di-button"
              onClick={() => setRefresh((n) => n + 1)}
            >
              Retry discovery
            </button>
          </div>
        )}
        {!loading && !discoveryError && inputs.length === 0 && (
          <p role="status">
            No comparison inputs are configured for this application, or none
            are accessible to you.
          </p>
        )}
        {!loading && !discoveryError && inputs.length > 0 && (
          <form onSubmit={submit}>
            <div className="di-form-grid">
              <label className="di-field">
                {entityRef ? "Comparison scenario" : "Application / scenario"}
                <select
                  aria-label={
                    entityRef ? "Comparison scenario" : "Application / scenario"
                  }
                  value={selectedId}
                  onChange={(e) => select(e.target.value)}
                  disabled={busy}
                  required
                >
                  {!selected && <option value="">Select a comparison</option>}
                  {inputs.map((input) => (
                    <option key={input.id} value={input.id}>
                      {input.label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="di-field">
                Source revision
                <select
                  aria-label="Source revision"
                  value={selected?.applicationRevision ?? ""}
                  disabled={busy || !selected}
                  onChange={(e) => {
                    const input = inputs.find(
                      (i) =>
                        i.entityRef === selected?.entityRef &&
                        i.applicationRevision === e.target.value
                    );
                    if (input) select(input.id);
                  }}
                >
                  {[
                    ...new Set(
                      inputs
                        .filter((i) => i.entityRef === selected?.entityRef)
                        .map((i) => i.applicationRevision)
                    ),
                  ].map((revision) => (
                    <option key={revision} value={revision}>
                      {revision}
                    </option>
                  ))}
                </select>
              </label>
              <label className="di-field">
                Dependency
                <select
                  aria-label="Dependency"
                  value={selected ? coordinate(selected.dependency) : ""}
                  disabled={busy || !selected}
                  onChange={(e) => {
                    const input = inputs.find(
                      (i) =>
                        i.entityRef === selected?.entityRef &&
                        i.applicationRevision ===
                          selected?.applicationRevision &&
                        coordinate(i.dependency) === e.target.value
                    );
                    if (input) select(input.id);
                  }}
                >
                  {[
                    ...new Set(
                      inputs
                        .filter(
                          (i) =>
                            i.entityRef === selected?.entityRef &&
                            i.applicationRevision ===
                              selected?.applicationRevision
                        )
                        .map((i) => coordinate(i.dependency))
                    ),
                  ].map((name) => (
                    <option key={name} value={name}>
                      {name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="di-field">
                Target version
                <input
                  aria-label="Target version"
                  value={targetVersion}
                  onChange={(e) => setTargetVersion(e.target.value)}
                  disabled={busy}
                  required
                  maxLength={199}
                  pattern="[A-Za-z0-9][A-Za-z0-9._\-]*"
                />
              </label>
            </div>
            {selected && (
              <>
                <p>{selected.description}</p>
                <p className="di-caption">
                  Baseline: <strong>{selected.baselineVersion}</strong>{" "}
                  (resolved again during analysis) ·{" "}
                  {selected.strategy === "transitive-override"
                    ? "Transitive override"
                    : "Direct upgrade"}
                </p>
              </>
            )}
            <div className="di-actions">
              <button
                className="di-button di-primary"
                type="submit"
                disabled={!selected || busy || available !== true}
              >
                {starting ? "Submitting…" : "Run comparison"}
              </button>
              <span role="status">
                {available === undefined
                  ? "Checking analysis service…"
                  : available
                  ? "Analysis service available"
                  : "Analysis service unavailable"}
              </span>
              {available === false && (
                <button
                  className="di-button"
                  type="button"
                  onClick={() => setRefresh((n) => n + 1)}
                >
                  Retry connection
                </button>
              )}
            </div>
            {available === false && (
              <div className="di-error" role="alert">
                The analysis service is unavailable. Start or reconnect the
                analysis service, then retry the connection.
              </div>
            )}
          </form>
        )}
      </InfoCard>
      {error && (
        <div className="di-error" role="alert">
          {error}{" "}
          {jobId && (
            <button
              className="di-button"
              onClick={() => setPollRetry((n) => n + 1)}
            >
              Retry status
            </button>
          )}
        </div>
      )}
      {job && (
        <section className="di-job" aria-label="Comparison status">
          <strong role="status">
            {job.state === "queued"
              ? "Queued — waiting for an analysis worker"
              : job.state === "running"
              ? stages[job.stage ?? ""] ?? "Analysis in progress"
              : job.state === "succeeded"
              ? "Comparison complete"
              : "Comparison failed"}
          </strong>
          <p className="di-caption">Comparison {job.id}</p>
          {(job.state === "running" || job.state === "queued") && !error && (
            <Progress />
          )}
          {job.state === "failed" && (
            <div role="alert" className="di-error">
              {job.error?.message ?? "Analysis could not complete."}
              <p>
                Check the selected version and available sources, then run the
                comparison again.
              </p>
            </div>
          )}
        </section>
      )}
      {report && <ReportViewer key={report.comparisonId} report={report} />}
    </div>
  );
}

export function DependencyImpactPage() {
  return (
    <Page themeId="tool">
      <Header
        title="Dependency impact"
        subtitle="Compare versions and explore affected source code"
      />
      <Content>
        <Workbench />
      </Content>
    </Page>
  );
}
export function EntityDependencyImpactContent() {
  const { entity } = useEntity();
  return (
    <Workbench
      entityRef={`${entity.kind.toLowerCase()}:${
        entity.metadata.namespace ?? "default"
      }/${entity.metadata.name}`}
    />
  );
}
