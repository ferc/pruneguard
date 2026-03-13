import type { NextConfig } from "next";

// Highly dynamic config: uses runtime function calls, environment variables,
// and conditional logic that cannot be statically analyzed.
const getConfig = async (): Promise<NextConfig> => {
  const plugins = await import("./plugins.config");
  const env = process.env.NODE_ENV;

  return {
    reactStrictMode: env === "production",
    experimental: {
      ...(process.env.ENABLE_TURBOPACK === "true" ? { turbo: {} } : {}),
    },
    rewrites: async () => {
      const rules = await fetch("https://api.example.com/rewrites").then((r) => r.json());
      return rules;
    },
    webpack: (config: any) => {
      plugins.default.forEach((plugin: any) => config.plugins.push(plugin));
      return config;
    },
  };
};

export default getConfig;
