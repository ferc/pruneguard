import { Tree } from '@nx/devkit';
import { Schema } from './schema';

export default async function componentGenerator(tree: Tree, options: Schema) {
  const content = `export function ${options.name}() {}`;
  tree.write(`src/${options.name}.ts`, content);
}
