export interface AnalysisJob {
  id: string;
  request: { entityRef: string };
  state: "queued" | "running" | "succeeded" | "failed";
}
export interface AnalysisResponse<T> {
  status: number;
  data: T;
  retryAfter: string | null;
}
export interface AnalysisError {
  error: { code: string; message: string };
}
export class AnalysisUnavailableError extends Error {}

/** Backend-only client; the service credential never enters frontend config. */
export class AnalysisClient {
  constructor(
    private readonly baseUrl?: string,
    private readonly token?: string
  ) {}
  private async request<T>(
    path: string,
    method = "GET",
    body?: unknown
  ): Promise<AnalysisResponse<T | AnalysisError>> {
    if (!this.baseUrl || !this.token)
      throw new AnalysisUnavailableError("Analysis service is not configured");
    try {
      const response = await fetch(
        `${this.baseUrl.replace(/\/$/, "")}${path}`,
        {
          method,
          headers: {
            Authorization: `Bearer ${this.token}`,
            "Content-Type": "application/json",
          },
          body: body === undefined ? undefined : JSON.stringify(body),
          signal: AbortSignal.timeout(30_000),
        }
      );
      return {
        status: response.status,
        data: (await response.json()) as T | AnalysisError,
        retryAfter: response.headers.get("retry-after"),
      };
    } catch {
      throw new AnalysisUnavailableError(
        "The analysis service is unavailable. Reconnect the service and retry."
      );
    }
  }
  async isAvailable(): Promise<boolean> {
    try {
      const result = await this.request<{ status: string }>("/health");
      return result.status === 200;
    } catch {
      return false;
    }
  }
  submit(body: unknown) {
    return this.request<AnalysisJob>("/v1/comparisons", "POST", body);
  }
  getJob(id: string) {
    return this.request<AnalysisJob>(
      `/v1/comparisons/${encodeURIComponent(id)}`
    );
  }
  getReport(id: string) {
    return this.request<Record<string, unknown>>(
      `/v1/comparisons/${encodeURIComponent(id)}/report`
    );
  }
}
