import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

type NativeBinding = {
  scan_json(options: unknown): string;
  impact_json(options: unknown): string;
  explain_json(options: unknown): string;
  load_config_json(cwd?: string, config?: string): string;
  debug_resolve_text(options: unknown): string;
  debug_entrypoints_json(options: unknown): string;
};

function loadNative(): NativeBinding {
  const platform = `${process.platform}-${process.arch}`;
  const candidates = [
    "@oxgraph/binding",
    "../oxgraph.node",
    `../oxgraph.${platform}.node`,
    `../binding.${platform}.node`,
  ];

  for (const candidate of candidates) {
    try {
      return require(candidate) as NativeBinding;
    } catch {
      continue;
    }
  }

  throw new Error(
    "Failed to load the oxgraph native binding. Run the package build first.",
  );
}

export const native = loadNative();
