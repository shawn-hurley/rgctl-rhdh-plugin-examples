import {
  createApiRef,
  DiscoveryApi,
  FetchApi,
} from "@backstage/core-plugin-api";

export interface Coordinate {
  groupId: string;
  artifactId: string;
  type: string;
  classifier?: string | null;
}
export interface ComparisonRequest {
  entityRef: string;
  applicationRevision: string;
  dependency: Coordinate;
  targetVersion: string;
  strategy: "direct-upgrade" | "transitive-override";
}
export interface ComparisonInput extends ComparisonRequest {
  id: string;
  label: string;
  description: string;
  baselineVersion: string;
  packageManager: string;
}
export interface Job {
  id: string;
  request: ComparisonRequest;
  state: "queued" | "running" | "succeeded" | "failed";
  stage: string | null;
  error: { code: string; message: string } | null;
}
export interface SymbolInfo {
  id: string;
  name: string;
  qualifiedName: string | null;
  signature: string | null;
  file: string;
  startLine: number | null;
  endLine: number | null;
  snapshot: string;
  sourceId: string;
  resolution?: string;
}
export interface SourceBlock {
  id: string;
  snapshot: string;
  sourceId: string;
  sourceIdentity: {
    kind: string;
    revision?: string;
    sha256: string;
    artifact?: Coordinate & { version: string };
  };
  file: string;
  startLine: number;
  endLine: number;
  text: string;
}
export interface Change {
  id: string;
  kind: string;
  baseline: SymbolInfo | null;
  candidate: SymbolInfo | null;
}
export interface Impact {
  id: string;
  symbol: SymbolInfo;
  sourceBlockId: string | null;
  changeIds: string[];
  pathIds: string[];
}
export interface CallPath {
  id: string;
  changeId: string;
  edges: {
    from: string;
    to: string;
    type: string;
    binding?: {
      declaredQualifiedName: string;
      implementationQualifiedName: string;
      reason: string;
    } | null;
  }[];
  symbols: SymbolInfo[];
  resolution?: string;
}
export interface Diagnostic {
  code: string;
  message?: string;
  qualifiedName?: string;
  file?: string;
  snapshot?: string;
}
export interface Report {
  schemaVersion: string;
  comparisonId: string;
  application: {
    entityRef: string;
    revision: string;
    sourceIdentity: { id: string; sha256: string };
  };
  resolution: {
    requested: ComparisonRequest;
    baseline: (Coordinate & { version: string })[];
    candidate: (Coordinate & { version: string })[];
    deltas: {
      coordinate: string;
      baseline: (Coordinate & { version: string }) | null;
      candidate: (Coordinate & { version: string }) | null;
    }[];
  };
  changes: Change[];
  impacts: Impact[];
  paths: CallPath[];
  sourceBlocks: SourceBlock[];
  coverage: { status: string; scope: string; truncated: boolean };
  diagnostics: Diagnostic[];
}
export interface DependencyImpactApi {
  getInputs(
    entityRef?: string,
    signal?: AbortSignal
  ): Promise<{ inputs: ComparisonInput[] }>;
  getStatus(signal?: AbortSignal): Promise<{ analysisAvailable: boolean }>;
  submit(request: ComparisonRequest, signal?: AbortSignal): Promise<Job>;
  getJob(id: string, signal?: AbortSignal): Promise<Job>;
  getReport(id: string, signal?: AbortSignal): Promise<Report>;
}
export const dependencyImpactApiRef = createApiRef<DependencyImpactApi>({
  id: "plugin.dependency-impact.api",
});

export class DependencyImpactClient implements DependencyImpactApi {
  constructor(
    private readonly discovery: DiscoveryApi,
    private readonly fetchApi: FetchApi
  ) {}
  private async request<T>(
    path: string,
    signal?: AbortSignal,
    body?: ComparisonRequest
  ): Promise<T> {
    const url = await this.discovery.getBaseUrl("dependency-impact");
    const response = await this.fetchApi.fetch(`${url}${path}`, {
      method: body ? "POST" : "GET",
      signal,
      headers: body ? { "Content-Type": "application/json" } : undefined,
      body: body ? JSON.stringify(body) : undefined,
    });
    const data = await response.json().catch(() => undefined);
    if (!response.ok) {
      const message =
        data?.error?.message || `Request failed (${response.status})`;
      const retry = response.headers.get("retry-after");
      throw new Error(
        `${message}${retry ? ` Retry in ${retry} seconds.` : ""}`
      );
    }
    if (!data)
      throw new Error(
        "The service returned an empty response. Retry the request."
      );
    return data as T;
  }
  getInputs(entityRef?: string, signal?: AbortSignal) {
    return this.request<{ inputs: ComparisonInput[] }>(
      `/inputs${
        entityRef ? `?entityRef=${encodeURIComponent(entityRef)}` : ""
      }`,
      signal
    );
  }
  getStatus(signal?: AbortSignal) {
    return this.request<{ analysisAvailable: boolean }>("/status", signal);
  }
  submit(request: ComparisonRequest, signal?: AbortSignal) {
    return this.request<Job>("/comparisons", signal, request);
  }
  getJob(id: string, signal?: AbortSignal) {
    return this.request<Job>(`/comparisons/${encodeURIComponent(id)}`, signal);
  }
  getReport(id: string, signal?: AbortSignal) {
    return this.request<Report>(
      `/comparisons/${encodeURIComponent(id)}/report`,
      signal
    );
  }
}
