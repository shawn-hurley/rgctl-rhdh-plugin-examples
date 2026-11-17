import { HttpError } from './catalog';
export interface Job {id: string; request: {entityRef: string}; state: string; graphVersion?: string}
export interface Application {applicationKey: string; entityRef: string; graphVersion: string}
export interface Connection {apiRef: string; consumer: string; provider: string}
export class GraphClient {
  constructor(private readonly baseUrl?: string, private readonly token?: string) {}
  async request<T>(path: string, body?: unknown): Promise<T> {
    if (!this.baseUrl || !this.token) throw new HttpError(503,'SERVICE_UNAVAILABLE','Application graph service is not configured');
    let response: Response;
    try {
      response = await fetch(`${this.baseUrl.replace(/\/$/,'')}${path}`, {
        method: body === undefined ? 'GET' : 'POST', headers: {Authorization: `Bearer ${this.token}`, 'Content-Type':'application/json'},
        body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(30000),
      });
    } catch { throw new HttpError(503,'SERVICE_UNAVAILABLE','Application graph service is unavailable'); }
    const result = await response.json();
    if (!response.ok) throw new HttpError(response.status,result.error?.code ?? 'SERVICE_ERROR',result.error?.message ?? 'Application graph request failed');
    return result as T;
  }
}
