export interface Config {
  applicationGraph?: {
    service?: {baseUrl: string; /** @visibility secret */ token: string};
    catalog?: {
      /** Minimum seconds before rebuilding an automatically indexed application. */
      refreshSeconds?: number;
    };
  };
}
