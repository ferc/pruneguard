interface Config {
  debug: boolean;
}

let currentConfig: Config = { debug: true };

export function configure(opts: Partial<Config>) {
  currentConfig = { ...currentConfig, ...opts };
}

export function getConfig(): Config {
  return currentConfig;
}
