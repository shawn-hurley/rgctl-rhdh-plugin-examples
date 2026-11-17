export interface Config {
  applicationGraph?: {
    service?: {
      /** URL of the separate Rust graph service. */
      baseUrl: string;
      /** @visibility secret */
      token: string;
    };
  };
}
