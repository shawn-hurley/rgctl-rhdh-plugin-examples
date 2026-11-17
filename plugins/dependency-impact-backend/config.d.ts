export interface Config {
  dependencyImpact?: {
    service?: {
      /** Rust analysis service URL, reachable from the Backstage backend. */
      baseUrl: string;
      /** @visibility secret */
      token: string;
    };
  };
}
