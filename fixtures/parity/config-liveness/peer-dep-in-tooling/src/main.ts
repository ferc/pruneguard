// This tooling package generates code that imports react,
// but react itself is a peer dependency, not a direct dependency.
export function generateComponent(name: string): string {
  return `import * as React from "react";\nexport const ${name} = () => <div />;`;
}
